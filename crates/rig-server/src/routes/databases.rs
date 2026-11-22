use axum::{
    Json,
    body::Body,
    extract::{Path, Query, State},
    http::{StatusCode, header},
    response::{IntoResponse, Response},
};
use rig_api::{
    Backup, BackupStatus, CreateDatabaseRequest, Database, DatabaseCredentials, DatabaseEngineInfo,
    ErrorBody,
};
use serde::Deserialize;
use sqlx::{Row, sqlite::SqliteRow};
use tokio_util::io::ReaderStream;
use utoipa::IntoParams;
use utoipa_axum::{router::OpenApiRouter, routes};

use crate::{
    auth::{Admin, Auth, Developer, Viewer},
    backups,
    databases::{self, ENGINES, engine_spec},
    error::{ApiError, ApiResult},
    routes::auth::{slugify, validate_name},
    state::{AppState, new_id, now},
};

pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(list_databases, create_database))
        .routes(routes!(list_engines))
        .routes(routes!(get_database, delete_database))
        .routes(routes!(start_database))
        .routes(routes!(stop_database))
        .routes(routes!(reveal_database))
        .routes(routes!(list_backups, create_backup))
        .routes(routes!(delete_backup))
        .routes(routes!(download_backup))
        .routes(routes!(restore_backup))
}

/// The shared column list, spliced into literal queries at compile time.
macro_rules! select {
    ($tail:literal) => {
        concat!(
            "SELECT d.id, d.project_id, p.name AS project_name, d.name, d.slug, d.engine,
                    d.version, d.status, d.error, d.username, d.db_name, d.host_port, d.created_at
               FROM databases d JOIN projects p ON p.id = d.project_id ",
            $tail
        )
    };
}

fn stored<T: serde::de::DeserializeOwned>(value: String) -> ApiResult<T> {
    serde_json::from_value(serde_json::Value::String(value))
        .map_err(|e| ApiError::Internal(anyhow::anyhow!("unexpected stored value: {e}")))
}

fn to_database(r: &SqliteRow) -> ApiResult<Database> {
    let engine = stored(r.get("engine"))?;
    Ok(Database {
        id: r.get("id"),
        project_id: r.get("project_id"),
        project_name: r.get("project_name"),
        name: r.get("name"),
        engine,
        version: r.get("version"),
        status: stored(r.get("status"))?,
        error: r.get("error"),
        host: r.get("slug"),
        port: engine_spec(engine).port,
        username: r.get("username"),
        db_name: r.get("db_name"),
        host_port: r
            .get::<Option<i64>, _>("host_port")
            .and_then(|p| u16::try_from(p).ok()),
        created_at: r.get("created_at"),
    })
}

async fn load(state: &AppState, org_id: &str, id: &str) -> ApiResult<Database> {
    let row = sqlx::query(select!("WHERE d.id = ? AND d.org_id = ?"))
        .bind(id)
        .bind(org_id)
        .fetch_optional(&state.db)
        .await?;
    match row {
        Some(r) => to_database(&r),
        None => Err(ApiError::NotFound("database")),
    }
}

#[derive(Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
struct ListQuery {
    /// Only databases in this project.
    project_id: Option<String>,
}

/// List databases.
#[utoipa::path(get, path = "/api/v1/databases", tag = "databases", params(ListQuery),
    responses((status = 200, body = Vec<Database>)))]
async fn list_databases(
    State(state): State<AppState>,
    auth: Auth<Viewer>,
    Query(q): Query<ListQuery>,
) -> ApiResult<Json<Vec<Database>>> {
    let rows = sqlx::query(select!(
        "WHERE d.org_id = ?1 AND (?2 IS NULL OR d.project_id = ?2) ORDER BY d.created_at DESC"
    ))
    .bind(&auth.org_id)
    .bind(&q.project_id)
    .fetch_all(&state.db)
    .await?;
    rows.iter()
        .map(to_database)
        .collect::<ApiResult<Vec<_>>>()
        .map(Json)
}

/// Engines and versions Rig can run.
#[utoipa::path(get, path = "/api/v1/databases/engines", tag = "databases",
    responses((status = 200, body = Vec<DatabaseEngineInfo>)))]
async fn list_engines(_auth: Auth<Viewer>) -> Json<Vec<DatabaseEngineInfo>> {
    Json(
        ENGINES
            .iter()
            .map(|e| DatabaseEngineInfo {
                engine: e.engine,
                label: e.label.to_string(),
                versions: e.versions.iter().map(|v| v.to_string()).collect(),
                port: e.port,
            })
            .collect(),
    )
}

/// Create a database and start it. Poll the database until it is `running`.
#[utoipa::path(post, path = "/api/v1/databases", tag = "databases", request_body = CreateDatabaseRequest,
    responses((status = 202, body = Database), (status = 409, body = ErrorBody), (status = 422, body = ErrorBody)))]
async fn create_database(
    State(state): State<AppState>,
    auth: Auth<Developer>,
    Json(req): Json<CreateDatabaseRequest>,
) -> ApiResult<(StatusCode, Json<Database>)> {
    let name = validate_name("database name", &req.name, 64)?;
    let slug = slugify(&name);
    let spec = engine_spec(req.engine);
    // Only offered versions: the value becomes part of an image reference.
    let version = match req
        .version
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
    {
        None => spec.versions[0],
        Some(v) => spec
            .versions
            .iter()
            .copied()
            .find(|offered| *offered == v)
            .ok_or_else(|| {
                ApiError::validation(format!(
                    "{} {v} is not offered. Choose one of: {}",
                    spec.label,
                    spec.versions.join(", ")
                ))
            })?,
    };

    let project: Option<String> =
        sqlx::query_scalar("SELECT id FROM projects WHERE id = ? AND org_id = ?")
            .bind(&req.project_id)
            .bind(&auth.org_id)
            .fetch_optional(&state.db)
            .await?;
    if project.is_none() {
        return Err(ApiError::NotFound("project"));
    }
    // Apps and databases share the project's DNS names.
    let taken: i64 = sqlx::query_scalar(
        "SELECT (SELECT COUNT(*) FROM databases WHERE project_id = ?1 AND (slug = ?2 OR name = ?3))
              + (SELECT COUNT(*) FROM apps WHERE project_id = ?1 AND slug = ?2)",
    )
    .bind(&req.project_id)
    .bind(&slug)
    .bind(&name)
    .fetch_one(&state.db)
    .await?;
    if taken > 0 {
        return Err(ApiError::Conflict(format!(
            "this project already has an app or database called {name}. Choose another name"
        )));
    }

    let id = new_id();
    let (username, db_name) = databases::identifiers(req.engine, &slug);
    let password_enc = state.vault.encrypt(&databases::new_password())?;
    sqlx::query(
        "INSERT INTO databases (id, org_id, project_id, name, slug, engine, version, volume, db_name,
                                username, password_enc, status, created_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, 'creating', ?)",
    )
    .bind(&id)
    .bind(&auth.org_id)
    .bind(&req.project_id)
    .bind(&name)
    .bind(&slug)
    .bind(req.engine.as_str())
    .bind(version)
    .bind(databases::volume_name(&id))
    .bind(&db_name)
    .bind(&username)
    .bind(&password_enc)
    .bind(now())
    .execute(&state.db)
    .await?;
    auth.audit(&state, "database.create", Some(&id), Some(&name))
        .await?;
    tokio::spawn(databases::provision(state.clone(), id.clone()));
    Ok((
        StatusCode::ACCEPTED,
        Json(load(&state, &auth.org_id, &id).await?),
    ))
}

/// Get one database.
#[utoipa::path(get, path = "/api/v1/databases/{id}", tag = "databases", params(("id" = String, Path)),
    responses((status = 200, body = Database), (status = 404, body = ErrorBody)))]
async fn get_database(
    State(state): State<AppState>,
    auth: Auth<Viewer>,
    Path(id): Path<String>,
) -> ApiResult<Json<Database>> {
    load(&state, &auth.org_id, &id).await.map(Json)
}

/// Start a stopped or failed database on its existing data.
#[utoipa::path(post, path = "/api/v1/databases/{id}/start", tag = "databases", params(("id" = String, Path)),
    responses((status = 202, body = Database), (status = 409, body = ErrorBody)))]
async fn start_database(
    State(state): State<AppState>,
    auth: Auth<Developer>,
    Path(id): Path<String>,
) -> ApiResult<(StatusCode, Json<Database>)> {
    let db = load(&state, &auth.org_id, &id).await?;
    // The status flip is the lock: only one caller moves it to `creating`,
    // and never while a delete or restore holds `busy_op`.
    let claimed = sqlx::query(
        "UPDATE databases SET status = 'creating', error = NULL
          WHERE id = ? AND status IN ('stopped', 'failed') AND busy_op IS NULL",
    )
    .bind(&id)
    .execute(&state.db)
    .await?
    .rows_affected();
    if claimed == 0 {
        return Err(ApiError::Conflict(format!(
            "{} is already running, starting, or busy",
            db.name
        )));
    }
    auth.audit(&state, "database.start", Some(&id), Some(&db.name))
        .await?;
    tokio::spawn(databases::provision(state.clone(), id.clone()));
    Ok((
        StatusCode::ACCEPTED,
        Json(load(&state, &auth.org_id, &id).await?),
    ))
}

/// Stop a running database. Its data is kept.
#[utoipa::path(post, path = "/api/v1/databases/{id}/stop", tag = "databases", params(("id" = String, Path)),
    responses((status = 200, body = Database), (status = 409, body = ErrorBody), (status = 503, body = ErrorBody)))]
async fn stop_database(
    State(state): State<AppState>,
    auth: Auth<Developer>,
    Path(id): Path<String>,
) -> ApiResult<Json<Database>> {
    let db = load(&state, &auth.org_id, &id).await?;
    let claimed = sqlx::query(
        "UPDATE databases SET status = 'stopped', error = NULL, container_id = NULL, host_port = NULL
          WHERE id = ? AND status IN ('running', 'failed') AND busy_op IS NULL",
    )
    .bind(&id)
    .execute(&state.db)
    .await?
    .rows_affected();
    if claimed == 0 {
        return Err(ApiError::Conflict(format!(
            "{} is not running or is busy. Wait for the current operation to finish",
            db.name
        )));
    }
    if let Err(e) = databases::remove_containers(&state, &id).await {
        let message = format!("could not stop the database container ({e:#})");
        sqlx::query("UPDATE databases SET status = 'failed', error = ? WHERE id = ?")
            .bind(&message)
            .bind(&id)
            .execute(&state.db)
            .await?;
        return Err(ApiError::Unavailable(message));
    }
    auth.audit(&state, "database.stop", Some(&id), Some(&db.name))
        .await?;
    load(&state, &auth.org_id, &id).await.map(Json)
}

/// Delete a database and all of its data.
#[utoipa::path(delete, path = "/api/v1/databases/{id}", tag = "databases", params(("id" = String, Path)),
    responses((status = 204), (status = 409, body = ErrorBody), (status = 503, body = ErrorBody)))]
async fn delete_database(
    State(state): State<AppState>,
    auth: Auth<Admin>,
    Path(id): Path<String>,
) -> ApiResult<StatusCode> {
    let db = load(&state, &auth.org_id, &id).await?;
    // Atomically claim the row for deletion so concurrent start/stop/restore/backup
    // calls cannot race and leave orphaned containers.
    let claimed = sqlx::query(
        "UPDATE databases SET busy_op = 'deleting'
          WHERE id = ? AND org_id = ? AND status != 'creating' AND busy_op IS NULL
            AND NOT EXISTS (SELECT 1 FROM backups WHERE database_id = ? AND status = 'running')",
    )
    .bind(&id)
    .bind(&auth.org_id)
    .bind(&id)
    .execute(&state.db)
    .await?
    .rows_affected();
    if claimed == 0 {
        return Err(ApiError::Conflict(format!(
            "{} is still starting, backing up, or busy. Wait for it to finish, then delete",
            db.name
        )));
    }
    let volume: String = sqlx::query_scalar("SELECT volume FROM databases WHERE id = ?")
        .bind(&id)
        .fetch_one(&state.db)
        .await?;
    // The engine goes first: once the row is gone nothing remembers the volume.
    if let Err(e) = databases::destroy(&state, &id, &volume).await {
        let _ = sqlx::query("UPDATE databases SET busy_op = NULL WHERE id = ?")
            .bind(&id)
            .execute(&state.db)
            .await;
        return Err(ApiError::Unavailable(format!(
            "could not remove the database ({e:#}). Nothing was deleted"
        )));
    }
    backups::remove_all(&state, &id).await;
    sqlx::query("DELETE FROM databases WHERE id = ? AND org_id = ?")
        .bind(&id)
        .bind(&auth.org_id)
        .execute(&state.db)
        .await?;
    auth.audit(&state, "database.delete", Some(&id), Some(&db.name))
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Reveal the password and connection URL. Recorded in the audit log.
#[utoipa::path(post, path = "/api/v1/databases/{id}/reveal", tag = "databases", params(("id" = String, Path)),
    responses((status = 200, body = DatabaseCredentials), (status = 404, body = ErrorBody)))]
async fn reveal_database(
    State(state): State<AppState>,
    auth: Auth<Developer>,
    Path(id): Path<String>,
) -> ApiResult<Json<DatabaseCredentials>> {
    let db = load(&state, &auth.org_id, &id).await?;
    // Recorded before the secret leaves the vault.
    auth.audit(&state, "database.reveal", Some(&id), Some(&db.name))
        .await?;
    let enc: String = sqlx::query_scalar("SELECT password_enc FROM databases WHERE id = ?")
        .bind(&id)
        .fetch_one(&state.db)
        .await?;
    let password = state.vault.decrypt(&enc)?;
    let url = engine_spec(db.engine).url(&db.username, &password, &db.host, db.port, &db.db_name);
    Ok(Json(DatabaseCredentials { password, url }))
}

// ------------------------------------------------------------------ backups

macro_rules! select_backups {
    ($tail:literal) => {
        concat!(
            "SELECT b.id, b.database_id, b.status, b.size_bytes, b.error, b.created_at,
                    b.finished_at, b.path, d.engine, d.name AS database_name
               FROM backups b JOIN databases d ON d.id = b.database_id ",
            $tail
        )
    };
}

fn to_backup(r: &SqliteRow) -> ApiResult<Backup> {
    let status = match r.get::<&str, _>("status") {
        "running" => BackupStatus::Running,
        "ok" => BackupStatus::Ok,
        _ => BackupStatus::Failed,
    };
    let engine: String = r.get("engine");
    let engine = serde_json::from_value(serde_json::Value::String(engine))
        .map_err(|e| ApiError::Internal(e.into()))?;
    Ok(Backup {
        id: r.get("id"),
        database_id: r.get("database_id"),
        status,
        size_bytes: r.get("size_bytes"),
        error: r.get("error"),
        restorable: status == BackupStatus::Ok && backups::restorable(engine),
        created_at: r.get("created_at"),
        finished_at: r.get("finished_at"),
    })
}

/// A backup in the caller's org, with its file and database name.
struct BackupFile {
    backup: Backup,
    path: Option<String>,
    database_name: String,
}

async fn load_backup(state: &AppState, org_id: &str, id: &str) -> ApiResult<BackupFile> {
    let r = sqlx::query(select_backups!("WHERE b.id = ? AND b.org_id = ?"))
        .bind(id)
        .bind(org_id)
        .fetch_optional(&state.db)
        .await?
        .ok_or(ApiError::NotFound("backup"))?;
    Ok(BackupFile {
        backup: to_backup(&r)?,
        path: r.get("path"),
        database_name: r.get("database_name"),
    })
}

/// Backups of a database, newest first.
#[utoipa::path(get, path = "/api/v1/databases/{id}/backups", tag = "databases", params(("id" = String, Path)),
    responses((status = 200, body = Vec<Backup>), (status = 404, body = ErrorBody)))]
async fn list_backups(
    State(state): State<AppState>,
    auth: Auth<Viewer>,
    Path(id): Path<String>,
) -> ApiResult<Json<Vec<Backup>>> {
    load(&state, &auth.org_id, &id).await?;
    let rows = sqlx::query(select_backups!(
        "WHERE b.database_id = ? ORDER BY b.created_at DESC LIMIT 100"
    ))
    .bind(&id)
    .fetch_all(&state.db)
    .await?;
    rows.iter()
        .map(to_backup)
        .collect::<ApiResult<_>>()
        .map(Json)
}

/// Take a backup now. The dump runs in the background; poll the list.
#[utoipa::path(post, path = "/api/v1/databases/{id}/backups", tag = "databases", params(("id" = String, Path)),
    responses((status = 202, body = Backup), (status = 409, body = ErrorBody)))]
async fn create_backup(
    State(state): State<AppState>,
    auth: Auth<Developer>,
    Path(id): Path<String>,
) -> ApiResult<(StatusCode, Json<Backup>)> {
    let db = load(&state, &auth.org_id, &id).await?;
    if db.status != rig_api::DatabaseStatus::Running {
        return Err(ApiError::Conflict(format!(
            "{} is not running. Start it to take a backup",
            db.name
        )));
    }
    crate::deploy::ensure_disk_space(&state)
        .await
        .map_err(|e| ApiError::Unavailable(e.to_string()))?;
    let backup_id = new_id();
    // The partial check and insert run as one statement, so two clicks
    // cannot start two dumps or race with a restore/delete.
    let inserted = sqlx::query(
        "INSERT INTO backups (id, org_id, database_id, created_at)
         SELECT ?, ?, ?, ?
          WHERE NOT EXISTS (SELECT 1 FROM backups WHERE database_id = ? AND status = 'running')
            AND EXISTS (SELECT 1 FROM databases WHERE id = ? AND status = 'running' AND busy_op IS NULL)",
    )
    .bind(&backup_id)
    .bind(&auth.org_id)
    .bind(&id)
    .bind(now())
    .bind(&id)
    .bind(&id)
    .execute(&state.db)
    .await?
    .rows_affected();
    if inserted == 0 {
        return Err(ApiError::Conflict(format!(
            "a backup or restore of {} is already running",
            db.name
        )));
    }
    auth.audit(&state, "database.backup", Some(&id), Some(&db.name))
        .await?;
    tokio::spawn(backups::run(state.clone(), backup_id.clone(), id));
    let backup = load_backup(&state, &auth.org_id, &backup_id).await?.backup;
    Ok((StatusCode::ACCEPTED, Json(backup)))
}

/// Delete a backup and its file.
#[utoipa::path(delete, path = "/api/v1/backups/{id}", tag = "databases", params(("id" = String, Path)),
    responses((status = 204), (status = 409, body = ErrorBody)))]
async fn delete_backup(
    State(state): State<AppState>,
    auth: Auth<Admin>,
    Path(id): Path<String>,
) -> ApiResult<StatusCode> {
    let file = load_backup(&state, &auth.org_id, &id).await?;
    if file.backup.status == BackupStatus::Running {
        return Err(ApiError::Conflict("this backup is still running".into()));
    }
    if let Some(path) = &file.path
        && let Err(e) = tokio::fs::remove_file(path).await
        && e.kind() != std::io::ErrorKind::NotFound
    {
        return Err(ApiError::Internal(
            anyhow::Error::from(e).context("removing the backup"),
        ));
    }
    sqlx::query("DELETE FROM backups WHERE id = ?")
        .bind(&id)
        .execute(&state.db)
        .await?;
    auth.audit(
        &state,
        "backup.delete",
        Some(&id),
        Some(&file.database_name),
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Download a backup file. It holds the database's data, so this is audited.
#[utoipa::path(get, path = "/api/v1/backups/{id}/download", tag = "databases", params(("id" = String, Path)),
    responses((status = 200, content_type = "application/octet-stream"), (status = 404, body = ErrorBody)))]
async fn download_backup(
    State(state): State<AppState>,
    auth: Auth<Admin>,
    Path(id): Path<String>,
) -> ApiResult<Response> {
    let file = load_backup(&state, &auth.org_id, &id).await?;
    let Some(path) = file.path.filter(|_| file.backup.status == BackupStatus::Ok) else {
        return Err(ApiError::NotFound("backup file"));
    };
    let reader = backups::open_decrypted(state.vault.clone(), std::path::Path::new(&path))
        .await
        .map_err(|_| ApiError::NotFound("backup file"))?;
    let len = file.backup.size_bytes.and_then(|s| u64::try_from(s).ok());
    auth.audit(
        &state,
        "backup.download",
        Some(&id),
        Some(&file.database_name),
    )
    .await?;

    let extension = std::path::Path::new(&path)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("bin");
    // Only slug characters reach the header, so it needs no quoting rules.
    let stamp: String = file
        .backup
        .created_at
        .chars()
        .take(19)
        .filter(char::is_ascii_digit)
        .collect();
    let filename = format!("{}-{stamp}.{extension}", slugify(&file.database_name));
    let mut response = Body::from_stream(ReaderStream::new(reader)).into_response();
    let headers = response.headers_mut();
    headers.insert(
        header::CONTENT_TYPE,
        "application/octet-stream".parse().expect("static"),
    );
    if let Ok(v) = format!("attachment; filename=\"{filename}\"").parse() {
        headers.insert(header::CONTENT_DISPOSITION, v);
    }
    if let Some(len) = len {
        headers.insert(header::CONTENT_LENGTH, len.into());
    }
    headers.insert(header::CACHE_CONTROL, "no-store".parse().expect("static"));
    Ok(response)
}

/// Restore a backup into its database, replacing the current data.
#[utoipa::path(post, path = "/api/v1/backups/{id}/restore", tag = "databases", params(("id" = String, Path)),
    responses((status = 204), (status = 409, body = ErrorBody), (status = 422, body = ErrorBody)))]
async fn restore_backup(
    State(state): State<AppState>,
    auth: Auth<Admin>,
    Path(id): Path<String>,
) -> ApiResult<StatusCode> {
    let file = load_backup(&state, &auth.org_id, &id).await?;
    if !file.backup.restorable {
        return Err(ApiError::Validation(
            "this backup cannot be restored".into(),
        ));
    }
    let Some(path) = file.path else {
        return Err(ApiError::NotFound("backup file"));
    };
    let claimed = sqlx::query(
        "UPDATE databases SET busy_op = 'restoring'
          WHERE id = ? AND status = 'running' AND busy_op IS NULL
            AND NOT EXISTS (SELECT 1 FROM backups WHERE database_id = ? AND status = 'running')",
    )
    .bind(&file.backup.database_id)
    .bind(&file.backup.database_id)
    .execute(&state.db)
    .await?
    .rows_affected();
    if claimed == 0 {
        return Err(ApiError::Conflict(format!(
            "{} is not running or another backup/restore is already in progress",
            file.database_name
        )));
    }
    // Recorded before the data changes, so a failed restore is still on file.
    let audit_res = auth
        .audit(
            &state,
            "backup.restore",
            Some(&id),
            Some(&file.database_name),
        )
        .await;
    let restore_res = if let Err(e) = audit_res {
        Err(e)
    } else {
        backups::restore(
            &state,
            &file.backup.database_id,
            std::path::Path::new(&path),
        )
        .await
        .map_err(|e| ApiError::Conflict(format!("{e:#}")))
    };
    let _ = sqlx::query("UPDATE databases SET busy_op = NULL WHERE id = ?")
        .bind(&file.backup.database_id)
        .execute(&state.db)
        .await;
    restore_res?;
    Ok(StatusCode::NO_CONTENT)
}
