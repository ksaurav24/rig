//! Apps and everything that hangs off one: deployments, logs, environment
//! variables and domains.

use std::{collections::HashMap, convert::Infallible, time::Duration};

use axum::{
    Json,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::sse::{Event, KeepAlive, Sse},
};
use futures_util::{Stream, StreamExt};
use rig_api::{
    AddDomainRequest, App, AppWebhook, CreateAppRequest, Deployment, Domain, EnvVar, ErrorBody,
    LogLine, RevealedEnvVar, SetEnvVarRequest, SourceKind, TlsMode, UpdateAppRequest,
};
use serde::{Deserialize, de::DeserializeOwned};
use sqlx::Row;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use utoipa_axum::{router::OpenApiRouter, routes};

use crate::{
    auth::{Admin, Auth, Developer, MinRole, Viewer},
    crypto::random_token,
    deploy::{self, NewDeployment, validate_git_url, validate_relative_path},
    error::{ApiError, ApiResult},
    proxy,
    routes::auth::{slugify, validate_name},
    state::{AppState, new_id, now},
};

pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(list_apps, create_app))
        .routes(routes!(get_app, update_app, delete_app))
        .routes(routes!(deploy_app))
        .routes(routes!(get_webhook))
        .routes(routes!(rotate_webhook))
        .routes(routes!(stop_app))
        .routes(routes!(app_logs))
        .routes(routes!(list_deployments))
        .routes(routes!(get_deployment))
        .routes(routes!(rollback))
        .routes(routes!(deployment_logs))
        .routes(routes!(list_env, set_env))
        .routes(routes!(delete_env))
        .routes(routes!(reveal_env))
        .routes(routes!(list_domains, add_domain))
        .routes(routes!(delete_domain))
}

// ---------------------------------------------------------------- validation

const MAX_ENV_VALUE: usize = 32 * 1024;

/// Read a lowercase enum stored as text.
fn parse_enum<T: DeserializeOwned>(value: String) -> ApiResult<T> {
    serde_json::from_value(serde_json::Value::String(value))
        .map_err(|e| ApiError::Internal(anyhow::anyhow!("unexpected stored value: {e}")))
}

fn validate_image(image: &str) -> ApiResult<String> {
    let image = image.trim();
    let ok = !image.is_empty()
        && image.len() <= 255
        && image.starts_with(|c: char| c.is_ascii_alphanumeric())
        && image
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "._-/:@".contains(c));
    if ok {
        Ok(image.to_string())
    } else {
        Err(ApiError::validation(
            "enter an image reference such as nginx:1.27 or ghcr.io/acme/api:latest",
        ))
    }
}

fn validate_branch(branch: &str) -> ApiResult<String> {
    let branch = branch.trim();
    let ok = !branch.is_empty()
        && branch.len() <= 200
        && !branch.starts_with('-')
        && !branch.contains("..")
        && branch
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "._-/".contains(c));
    if ok {
        Ok(branch.to_string())
    } else {
        Err(ApiError::validation(
            "the branch name can use letters, digits and . _ - /",
        ))
    }
}

fn validate_port(port: u16) -> ApiResult<u16> {
    if port == 0 {
        return Err(ApiError::validation("the port must be between 1 and 65535"));
    }
    Ok(port)
}

fn validate_health_path(path: &str) -> ApiResult<String> {
    let ok =
        path.starts_with('/') && path.len() <= 200 && path.chars().all(|c| c.is_ascii_graphic());
    if ok {
        Ok(path.to_string())
    } else {
        Err(ApiError::validation(
            "the health check path must start with / and contain no spaces",
        ))
    }
}

fn validate_memory(mb: u32) -> ApiResult<u32> {
    if (16..=1_048_576).contains(&mb) {
        Ok(mb)
    } else {
        Err(ApiError::validation(
            "the memory limit must be at least 16 MB",
        ))
    }
}

fn validate_cpu(millis: u32) -> ApiResult<u32> {
    if (10..=256_000).contains(&millis) {
        Ok(millis)
    } else {
        Err(ApiError::validation(
            "the CPU limit must be at least 10 millicores (0.01 CPU)",
        ))
    }
}

fn validate_env_key(key: &str) -> ApiResult<()> {
    let mut chars = key.chars();
    let ok = key.len() <= 128
        && chars
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_');
    if ok {
        Ok(())
    } else {
        Err(ApiError::validation(
            "variable names use letters, digits and underscores, and cannot start with a digit",
        ))
    }
}

fn validate_env_value(value: &str) -> ApiResult<()> {
    if value.len() > MAX_ENV_VALUE || value.contains('\0') {
        return Err(ApiError::validation(
            "the value must be 32 KB or smaller and contain no NUL bytes",
        ));
    }
    Ok(())
}

const MAX_CREATE_ENV: usize = 64;
const MAX_VOLUMES: usize = 8;

/// Absolute, normalised container paths. System directories and the engine
/// socket are refused so a volume can't shadow or expose them.
fn validate_volumes(paths: &[String]) -> ApiResult<Vec<String>> {
    const FORBIDDEN: &[&str] = &[
        "/", "/bin", "/boot", "/dev", "/etc", "/lib", "/lib64", "/proc", "/run", "/sbin", "/sys",
        "/usr", "/var/run",
    ];
    if paths.len() > MAX_VOLUMES {
        return Err(ApiError::validation(format!(
            "an app can have at most {MAX_VOLUMES} volumes"
        )));
    }
    let mut out: Vec<String> = Vec::with_capacity(paths.len());
    for raw in paths {
        let path = raw.trim().trim_end_matches('/');
        let ok = path.starts_with('/')
            && path.len() <= 255
            && path
                .split('/')
                .skip(1)
                .all(|seg| !seg.is_empty() && seg != "." && seg != "..")
            && path.chars().all(|c| {
                c.is_ascii_alphanumeric() || matches!(c, '/' | '-' | '_' | '.' | '@' | '+')
            });
        if !ok {
            return Err(ApiError::validation(format!(
                "{raw} is not a valid volume path. Use an absolute path such as /data"
            )));
        }
        if FORBIDDEN.contains(&path)
            || path.ends_with(".sock")
            || path.starts_with("/proc/")
            || path.starts_with("/sys/")
        {
            return Err(ApiError::validation(format!("{path} cannot be a volume")));
        }
        if out.iter().any(|p| {
            p == path || path.starts_with(&format!("{p}/")) || p.starts_with(&format!("{path}/"))
        }) {
            return Err(ApiError::validation(format!(
                "{path} overlaps another volume"
            )));
        }
        out.push(path.to_owned());
    }
    Ok(out)
}

/// A lowercase DNS name. Rejects anything Caddy could read as a pattern.
pub(crate) fn validate_host(host: &str) -> ApiResult<String> {
    let host = host.trim().trim_end_matches('.').to_ascii_lowercase();
    let label_ok = |l: &str| {
        !l.is_empty()
            && l.len() <= 63
            && !l.starts_with('-')
            && !l.ends_with('-')
            && l.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
    };
    if host.len() <= 253 && host.split('.').all(label_ok) {
        Ok(host)
    } else {
        Err(ApiError::validation(
            "enter a hostname such as app.example.com, without a scheme, port or path",
        ))
    }
}

// ---------------------------------------------------------------------- apps

// SQL is assembled with `concat!` so every query stays a compile-time literal.
macro_rules! app_select {
    ($tail:literal) => {
        concat!(
            "SELECT a.id, a.project_id, p.name AS project_name, a.name, a.slug, a.source_kind,
        a.image, a.git_url, a.git_branch, a.dockerfile_path, a.port, a.memory_mb, a.cpu_millis,
        a.health_path, a.status, a.current_deployment_id, a.created_at, a.updated_at,
        a.restart_count, a.last_crash_at, a.last_crash_reason, a.auto_deploy,
        d.host_port,
        (SELECT group_concat(v.mount_path, char(10)) FROM
           (SELECT mount_path FROM app_volumes WHERE app_id = a.id ORDER BY mount_path) v) AS volumes
   FROM apps a
   JOIN projects p ON p.id = a.project_id
   LEFT JOIN deployments d ON d.id = a.current_deployment_id ",
            $tail
        )
    };
}

fn opt_u32(r: &sqlx::sqlite::SqliteRow, col: &str) -> Option<u32> {
    r.get::<Option<i64>, _>(col)
        .and_then(|v| u32::try_from(v).ok())
}

fn app_from_row(r: &sqlx::sqlite::SqliteRow, domains: Vec<String>) -> ApiResult<App> {
    let status: rig_api::AppStatus = parse_enum(r.get("status"))?;
    Ok(App {
        id: r.get("id"),
        project_id: r.get("project_id"),
        project_name: r.get("project_name"),
        name: r.get("name"),
        slug: r.get("slug"),
        source_kind: parse_enum(r.get("source_kind"))?,
        image: r.get("image"),
        git_url: r.get("git_url"),
        git_branch: r.get("git_branch"),
        dockerfile_path: r.get("dockerfile_path"),
        port: u16::try_from(r.get::<i64, _>("port")).unwrap_or(80),
        memory_mb: opt_u32(r, "memory_mb"),
        cpu_millis: opt_u32(r, "cpu_millis"),
        health_path: r.get("health_path"),
        host_port: (status == rig_api::AppStatus::Running)
            .then(|| {
                r.get::<Option<i64>, _>("host_port")
                    .and_then(|p| u16::try_from(p).ok())
            })
            .flatten(),
        status,
        current_deployment_id: r.get("current_deployment_id"),
        domains,
        volumes: r
            .get::<Option<String>, _>("volumes")
            .map(|v| v.lines().map(str::to_owned).collect())
            .unwrap_or_default(),
        last_crash: match (
            r.get::<Option<String>, _>("last_crash_at"),
            r.get::<Option<String>, _>("last_crash_reason"),
        ) {
            (Some(at), Some(reason)) => Some(rig_api::AppCrash {
                at,
                reason,
                restarts: u32::try_from(r.get::<i64, _>("restart_count")).unwrap_or(0),
            }),
            _ => None,
        },
        auto_deploy: r.get::<i64, _>("auto_deploy") != 0,
        created_at: r.get("created_at"),
        updated_at: r.get("updated_at"),
    })
}

async fn load_app(state: &AppState, org_id: &str, id: &str) -> ApiResult<App> {
    let row = sqlx::query(app_select!("WHERE a.id = ? AND a.org_id = ?"))
        .bind(id)
        .bind(org_id)
        .fetch_optional(&state.db)
        .await?
        .ok_or(ApiError::NotFound("app"))?;
    let domains = sqlx::query_scalar("SELECT host FROM domains WHERE app_id = ? ORDER BY host")
        .bind(id)
        .fetch_all(&state.db)
        .await?;
    app_from_row(&row, domains)
}

/// Fail with 404 unless the app belongs to the caller's organization.
async fn require_app(state: &AppState, org_id: &str, id: &str) -> ApiResult<()> {
    let found: Option<i64> = sqlx::query_scalar("SELECT 1 FROM apps WHERE id = ? AND org_id = ?")
        .bind(id)
        .bind(org_id)
        .fetch_optional(&state.db)
        .await?;
    found.map(|_| ()).ok_or(ApiError::NotFound("app"))
}

async fn deployment_in_progress(state: &AppState, app_id: &str) -> ApiResult<bool> {
    let n: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM deployments WHERE app_id = ? AND status IN ('queued', 'building', 'deploying')",
    )
    .bind(app_id)
    .fetch_one(&state.db)
    .await?;
    Ok(n > 0)
}

async fn sync_proxy(state: &AppState, why: &str) {
    if let Err(e) = proxy::sync(state).await {
        tracing::warn!(error = %format!("{e:#}"), "proxy not updated after {why}");
    }
}

#[derive(Deserialize, utoipa::IntoParams)]
#[into_params(parameter_in = Query)]
struct AppListQuery {
    /// Only apps in this project.
    project_id: Option<String>,
}

#[utoipa::path(get, path = "/api/v1/apps", tag = "apps", params(AppListQuery),
    responses((status = 200, body = Vec<App>)))]
async fn list_apps(
    State(state): State<AppState>,
    auth: Auth<Viewer>,
    Query(q): Query<AppListQuery>,
) -> ApiResult<Json<Vec<App>>> {
    let rows = sqlx::query(app_select!(
        "WHERE a.org_id = ?1 AND (?2 IS NULL OR a.project_id = ?2)
          ORDER BY p.name COLLATE NOCASE, a.name COLLATE NOCASE"
    ))
    .bind(&auth.org_id)
    .bind(&q.project_id)
    .fetch_all(&state.db)
    .await?;
    let mut domains: HashMap<String, Vec<String>> = HashMap::new();
    for r in sqlx::query("SELECT app_id, host FROM domains WHERE org_id = ? ORDER BY host")
        .bind(&auth.org_id)
        .fetch_all(&state.db)
        .await?
    {
        domains
            .entry(r.get("app_id"))
            .or_default()
            .push(r.get("host"));
    }
    let apps = rows
        .iter()
        .map(|r| {
            let id: String = r.get("id");
            app_from_row(r, domains.remove(&id).unwrap_or_default())
        })
        .collect::<ApiResult<Vec<_>>>()?;
    Ok(Json(apps))
}

/// The source fields of an app after validation.
struct Source {
    image: Option<String>,
    git_url: Option<String>,
    git_branch: Option<String>,
    dockerfile_path: String,
}

fn validate_source(
    kind: SourceKind,
    image: Option<&str>,
    git_url: Option<&str>,
    git_branch: Option<&str>,
    dockerfile_path: Option<&str>,
) -> ApiResult<Source> {
    let dockerfile_path = dockerfile_path
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .unwrap_or(crate::deploy::DEFAULT_DOCKERFILE);
    validate_relative_path(dockerfile_path).map_err(ApiError::validation)?;
    match kind {
        SourceKind::Image => Ok(Source {
            image: Some(validate_image(image.unwrap_or_default())?),
            git_url: None,
            git_branch: None,
            dockerfile_path: dockerfile_path.to_string(),
        }),
        SourceKind::Git => {
            let url = git_url.unwrap_or_default().trim();
            validate_git_url(url).map_err(ApiError::validation)?;
            let branch = match git_branch.map(str::trim).filter(|b| !b.is_empty()) {
                Some(b) => Some(validate_branch(b)?),
                None => None,
            };
            Ok(Source {
                image: None,
                git_url: Some(url.to_string()),
                git_branch: branch,
                dockerfile_path: dockerfile_path.to_string(),
            })
        }
    }
}

#[utoipa::path(post, path = "/api/v1/apps", tag = "apps", request_body = CreateAppRequest,
    responses((status = 201, body = App), (status = 404, body = ErrorBody),
        (status = 409, body = ErrorBody), (status = 422, body = ErrorBody)))]
async fn create_app(
    State(state): State<AppState>,
    auth: Auth<Developer>,
    Json(req): Json<CreateAppRequest>,
) -> ApiResult<(StatusCode, Json<App>)> {
    let name = validate_name("app name", &req.name, 64)?;
    let source = validate_source(
        req.source_kind,
        req.image.as_deref(),
        req.git_url.as_deref(),
        req.git_branch.as_deref(),
        req.dockerfile_path.as_deref(),
    )?;
    let port = validate_port(req.port)?;
    let memory_mb = req
        .memory_mb
        .filter(|v| *v != 0)
        .map(validate_memory)
        .transpose()?;
    let cpu_millis = req
        .cpu_millis
        .filter(|v| *v != 0)
        .map(validate_cpu)
        .transpose()?;
    let health_path = req
        .health_path
        .as_deref()
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .map(validate_health_path)
        .transpose()?;

    let volumes = validate_volumes(&req.volumes)?;
    if req.env.len() > MAX_CREATE_ENV {
        return Err(ApiError::validation(format!(
            "set at most {MAX_CREATE_ENV} variables when creating an app"
        )));
    }
    let mut env = Vec::with_capacity(req.env.len());
    for var in &req.env {
        validate_env_key(&var.key)?;
        validate_env_value(&var.value)?;
        if env
            .iter()
            .any(|(k, _, _): &(&str, String, bool)| *k == var.key)
        {
            return Err(ApiError::validation(format!(
                "the variable {} is set twice",
                var.key
            )));
        }
        env.push((
            var.key.as_str(),
            state.vault.encrypt(&var.value)?,
            var.is_secret,
        ));
    }

    let project: Option<i64> =
        sqlx::query_scalar("SELECT 1 FROM projects WHERE id = ? AND org_id = ?")
            .bind(&req.project_id)
            .bind(&auth.org_id)
            .fetch_optional(&state.db)
            .await?;
    if project.is_none() {
        return Err(ApiError::NotFound("project"));
    }
    // Apps and databases share the project's DNS names.
    let slug = slugify(&name);
    let taken: i64 = sqlx::query_scalar(
        "SELECT (SELECT COUNT(*) FROM apps WHERE project_id = ?1 AND (slug = ?2 OR name = ?3))
              + (SELECT COUNT(*) FROM databases WHERE project_id = ?1 AND slug = ?2)",
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
    let server_id: Option<String> =
        sqlx::query_scalar("SELECT id FROM servers WHERE org_id = ? AND kind = 'local' LIMIT 1")
            .bind(&auth.org_id)
            .fetch_optional(&state.db)
            .await?;

    let id = new_id();
    let at = now();
    let mut tx = state.db.begin().await?;
    sqlx::query(
        "INSERT INTO apps (id, org_id, project_id, server_id, name, slug, source_kind, image, git_url,
                           git_branch, dockerfile_path, port, memory_mb, cpu_millis, health_path,
                           webhook_secret, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(&auth.org_id)
    .bind(&req.project_id)
    .bind(server_id)
    .bind(&name)
    .bind(&slug)
    .bind(req.source_kind.as_str())
    .bind(&source.image)
    .bind(&source.git_url)
    .bind(&source.git_branch)
    .bind(&source.dockerfile_path)
    .bind(i64::from(port))
    .bind(memory_mb.map(i64::from))
    .bind(cpu_millis.map(i64::from))
    .bind(&health_path)
    .bind(random_token(24))
    .bind(&at)
    .bind(&at)
    .execute(&mut *tx)
    .await?;
    for path in &volumes {
        let volume_id = new_id();
        sqlx::query(
            "INSERT INTO app_volumes (id, app_id, name, mount_path, created_at) VALUES (?, ?, ?, ?, ?)",
        )
        .bind(&volume_id)
        .bind(&id)
        .bind(format!("rig-app-{volume_id}"))
        .bind(path)
        .bind(&at)
        .execute(&mut *tx)
        .await?;
    }
    for (key, value_enc, is_secret) in &env {
        sqlx::query(
            "INSERT INTO env_vars (id, org_id, app_id, key, value_enc, is_secret, updated_at)
             VALUES (?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(new_id())
        .bind(&auth.org_id)
        .bind(&id)
        .bind(key)
        .bind(value_enc)
        .bind(is_secret)
        .bind(&at)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    auth.audit(&state, "app.create", Some(&id), Some(&name))
        .await?;
    Ok((
        StatusCode::CREATED,
        Json(load_app(&state, &auth.org_id, &id).await?),
    ))
}

#[utoipa::path(get, path = "/api/v1/apps/{id}", tag = "apps", params(("id" = String, Path)),
    responses((status = 200, body = App), (status = 404, body = ErrorBody)))]
async fn get_app(
    State(state): State<AppState>,
    auth: Auth<Viewer>,
    Path(id): Path<String>,
) -> ApiResult<Json<App>> {
    Ok(Json(load_app(&state, &auth.org_id, &id).await?))
}

/// Change an app's settings. They take effect on the next deployment.
#[utoipa::path(patch, path = "/api/v1/apps/{id}", tag = "apps", params(("id" = String, Path)),
    request_body = UpdateAppRequest,
    responses((status = 200, body = App), (status = 404, body = ErrorBody), (status = 422, body = ErrorBody)))]
async fn update_app(
    State(state): State<AppState>,
    auth: Auth<Developer>,
    Path(id): Path<String>,
    Json(req): Json<UpdateAppRequest>,
) -> ApiResult<Json<App>> {
    let app = load_app(&state, &auth.org_id, &id).await?;
    let name = match &req.name {
        Some(n) => validate_name("app name", n, 64)?,
        None => app.name.clone(),
    };
    let source = validate_source(
        app.source_kind,
        req.image.as_deref().or(app.image.as_deref()),
        req.git_url.as_deref().or(app.git_url.as_deref()),
        // An empty branch clears it, which means the repository's default.
        req.git_branch.as_deref().or(app.git_branch.as_deref()),
        req.dockerfile_path
            .as_deref()
            .or(Some(&app.dockerfile_path)),
    )?;
    let port = validate_port(req.port.unwrap_or(app.port))?;
    let memory_mb = match req.memory_mb {
        Some(0) => None,
        Some(v) => Some(validate_memory(v)?),
        None => app.memory_mb,
    };
    let cpu_millis = match req.cpu_millis {
        Some(0) => None,
        Some(v) => Some(validate_cpu(v)?),
        None => app.cpu_millis,
    };
    let health_path = match req.health_path.as_deref().map(str::trim) {
        Some("") => None,
        Some(p) => Some(validate_health_path(p)?),
        None => app.health_path.clone(),
    };
    let auto_deploy = match req.auto_deploy {
        Some(true) if app.source_kind != rig_api::SourceKind::Git => {
            return Err(ApiError::validation(
                "deploy on push needs an app built from a Git repository",
            ));
        }
        Some(v) => v,
        None => app.auto_deploy,
    };

    // The slug is kept: it names containers and images that already exist.
    sqlx::query(
        "UPDATE apps SET name = ?, image = ?, git_url = ?, git_branch = ?, dockerfile_path = ?, port = ?,
                memory_mb = ?, cpu_millis = ?, health_path = ?, auto_deploy = ?, updated_at = ?
          WHERE id = ? AND org_id = ?",
    )
    .bind(&name)
    .bind(&source.image)
    .bind(&source.git_url)
    .bind(&source.git_branch)
    .bind(&source.dockerfile_path)
    .bind(i64::from(port))
    .bind(memory_mb.map(i64::from))
    .bind(cpu_millis.map(i64::from))
    .bind(&health_path)
    .bind(auto_deploy)
    .bind(now())
    .bind(&id)
    .bind(&auth.org_id)
    .execute(&state.db)
    .await?;
    auth.audit(&state, "app.update", Some(&id), Some(&name))
        .await?;
    Ok(Json(load_app(&state, &auth.org_id, &id).await?))
}

/// Delete an app, its containers, deployments, variables and domains.
#[utoipa::path(delete, path = "/api/v1/apps/{id}", tag = "apps", params(("id" = String, Path)),
    responses((status = 204), (status = 404, body = ErrorBody), (status = 409, body = ErrorBody)))]
async fn delete_app(
    State(state): State<AppState>,
    auth: Auth<Admin>,
    Path(id): Path<String>,
) -> ApiResult<StatusCode> {
    let app = load_app(&state, &auth.org_id, &id).await?;
    if deployment_in_progress(&state, &id).await? {
        return Err(ApiError::Conflict(
            "a deployment is still in progress. Wait for it to finish, then delete the app".into(),
        ));
    }
    // Containers go first: once the row is gone nothing would remember them.
    deploy::remove_app_containers(&state, &id)
        .await
        .map_err(|e| {
            ApiError::Unavailable(format!(
                "could not remove the app's containers ({e:#}). Nothing was deleted"
            ))
        })?;
    deploy::remove_app_volumes(&state, &id).await.map_err(|e| {
        ApiError::Unavailable(format!(
            "could not remove the app's volumes ({e:#}). Nothing was deleted"
        ))
    })?;
    sqlx::query("DELETE FROM apps WHERE id = ? AND org_id = ?")
        .bind(&id)
        .bind(&auth.org_id)
        .execute(&state.db)
        .await?;
    sync_proxy(&state, "deleting an app").await;
    auth.audit(&state, "app.delete", Some(&id), Some(&app.name))
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

// --------------------------------------------------------------- deployments

fn deployment_from_row(r: &sqlx::sqlite::SqliteRow) -> ApiResult<Deployment> {
    Ok(Deployment {
        id: r.get("id"),
        app_id: r.get("app_id"),
        status: parse_enum(r.get("status"))?,
        trigger: r.get("trigger"),
        image_ref: r.get("image_ref"),
        commit_sha: r.get("commit_sha"),
        commit_msg: r.get("commit_msg"),
        error: r.get("error"),
        created_at: r.get("created_at"),
        started_at: r.get("started_at"),
        finished_at: r.get("finished_at"),
    })
}

macro_rules! deployment_select {
    ($tail:literal) => {
        concat!(
            "SELECT id, app_id, status, trigger, image_ref, commit_sha, commit_msg,
        error, created_at, started_at, finished_at FROM deployments ",
            $tail
        )
    };
}

async fn load_deployment(state: &AppState, org_id: &str, id: &str) -> ApiResult<Deployment> {
    let row = sqlx::query(deployment_select!("WHERE id = ? AND org_id = ?"))
        .bind(id)
        .bind(org_id)
        .fetch_optional(&state.db)
        .await?
        .ok_or(ApiError::NotFound("deployment"))?;
    deployment_from_row(&row)
}

async fn start_deployment<R: MinRole>(
    state: &AppState,
    auth: &Auth<R>,
    app_id: &str,
    trigger: &str,
    image_ref: Option<&str>,
    image_id: Option<&str>,
) -> ApiResult<Deployment> {
    if deployment_in_progress(state, app_id).await? {
        return Err(ApiError::Conflict(
            "a deployment is already in progress for this app. Wait for it to finish".into(),
        ));
    }
    let id = deploy::enqueue(
        state,
        NewDeployment {
            org_id: &auth.org_id,
            app_id,
            trigger,
            created_by: Some(&auth.user_id),
            image_ref,
            image_id,
        },
    )
    .await?;
    sqlx::query("UPDATE apps SET status = 'deploying', updated_at = ? WHERE id = ?")
        .bind(now())
        .bind(app_id)
        .execute(&state.db)
        .await?;
    load_deployment(state, &auth.org_id, &id).await
}

fn webhook(id: &str, secret: String) -> AppWebhook {
    AppWebhook {
        path: format!("/api/v1/hooks/apps/{id}"),
        secret,
    }
}

/// The app's push-to-deploy webhook. Reading the secret is audited.
#[utoipa::path(get, path = "/api/v1/apps/{id}/webhook", tag = "deployments", params(("id" = String, Path)),
    responses((status = 200, body = AppWebhook), (status = 404, body = ErrorBody)))]
async fn get_webhook(
    State(state): State<AppState>,
    auth: Auth<Developer>,
    Path(id): Path<String>,
) -> ApiResult<Json<AppWebhook>> {
    let secret: Option<String> =
        sqlx::query_scalar("SELECT webhook_secret FROM apps WHERE id = ? AND org_id = ?")
            .bind(&id)
            .bind(&auth.org_id)
            .fetch_optional(&state.db)
            .await?;
    let secret = secret.ok_or(ApiError::NotFound("app not found"))?;
    auth.audit(&state, "app.webhook.reveal", Some(&id), None)
        .await?;
    Ok(Json(webhook(&id, secret)))
}

/// Replace the webhook secret. Hooks signed with the old one stop working.
#[utoipa::path(post, path = "/api/v1/apps/{id}/webhook/rotate", tag = "deployments", params(("id" = String, Path)),
    responses((status = 200, body = AppWebhook), (status = 404, body = ErrorBody)))]
async fn rotate_webhook(
    State(state): State<AppState>,
    auth: Auth<Developer>,
    Path(id): Path<String>,
) -> ApiResult<Json<AppWebhook>> {
    let secret = random_token(24);
    let done = sqlx::query(
        "UPDATE apps SET webhook_secret = ?, updated_at = ? WHERE id = ? AND org_id = ?",
    )
    .bind(&secret)
    .bind(now())
    .bind(&id)
    .bind(&auth.org_id)
    .execute(&state.db)
    .await?;
    if done.rows_affected() == 0 {
        return Err(ApiError::NotFound("app not found"));
    }
    auth.audit(&state, "app.webhook.rotate", Some(&id), None)
        .await?;
    Ok(Json(webhook(&id, secret)))
}

/// Queue a deployment of the app's current source.
#[utoipa::path(post, path = "/api/v1/apps/{id}/deploy", tag = "deployments", params(("id" = String, Path)),
    responses((status = 202, body = Deployment), (status = 404, body = ErrorBody), (status = 409, body = ErrorBody)))]
async fn deploy_app(
    State(state): State<AppState>,
    auth: Auth<Developer>,
    Path(id): Path<String>,
) -> ApiResult<(StatusCode, Json<Deployment>)> {
    require_app(&state, &auth.org_id, &id).await?;
    let trigger = if auth.via == crate::auth::Via::ApiToken {
        "api"
    } else {
        "manual"
    };
    let d = start_deployment(&state, &auth, &id, trigger, None, None).await?;
    auth.audit(&state, "app.deploy", Some(&id), Some(&d.id))
        .await?;
    Ok((StatusCode::ACCEPTED, Json(d)))
}

/// Deploy the image of an earlier deployment again.
#[utoipa::path(post, path = "/api/v1/deployments/{id}/rollback", tag = "deployments",
    params(("id" = String, Path)),
    responses((status = 202, body = Deployment), (status = 404, body = ErrorBody),
        (status = 409, body = ErrorBody), (status = 422, body = ErrorBody)))]
async fn rollback(
    State(state): State<AppState>,
    auth: Auth<Developer>,
    Path(id): Path<String>,
) -> ApiResult<(StatusCode, Json<Deployment>)> {
    let target = load_deployment(&state, &auth.org_id, &id).await?;
    let image = match (&target.image_ref, target.status) {
        (Some(image), rig_api::DeploymentStatus::Superseded) => image.clone(),
        (_, rig_api::DeploymentStatus::Running) => {
            return Err(ApiError::validation(
                "that deployment is already the live one",
            ));
        }
        _ => {
            return Err(ApiError::validation(
                "only a deployment that was live before can be rolled back to",
            ));
        }
    };
    let image_id: Option<String> =
        sqlx::query_scalar("SELECT image_id FROM deployments WHERE id = ?")
            .bind(&id)
            .fetch_one(&state.db)
            .await?;
    let d = start_deployment(
        &state,
        &auth,
        &target.app_id,
        "rollback",
        Some(&image),
        image_id.as_deref(),
    )
    .await?;
    auth.audit(&state, "app.rollback", Some(&target.app_id), Some(&id))
        .await?;
    Ok((StatusCode::ACCEPTED, Json(d)))
}

/// Stop an app's containers and take it off the proxy.
#[utoipa::path(post, path = "/api/v1/apps/{id}/stop", tag = "apps", params(("id" = String, Path)),
    responses((status = 200, body = App), (status = 404, body = ErrorBody), (status = 409, body = ErrorBody)))]
async fn stop_app(
    State(state): State<AppState>,
    auth: Auth<Developer>,
    Path(id): Path<String>,
) -> ApiResult<Json<App>> {
    require_app(&state, &auth.org_id, &id).await?;
    if deployment_in_progress(&state, &id).await? {
        return Err(ApiError::Conflict(
            "a deployment is in progress. Wait for it to finish, then stop the app".into(),
        ));
    }
    // Take the app off the proxy before its container goes away, so the
    // proxy never points at a closed port.
    let mut tx = state.db.begin().await?;
    sqlx::query(
        "UPDATE deployments SET status = 'superseded'
          WHERE id = (SELECT current_deployment_id FROM apps WHERE id = ?) AND status = 'running'",
    )
    .bind(&id)
    .execute(&mut *tx)
    .await?;
    sqlx::query("UPDATE apps SET status = 'stopped', current_deployment_id = NULL, updated_at = ? WHERE id = ?")
        .bind(now())
        .bind(&id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    sync_proxy(&state, "stopping an app").await;
    deploy::remove_app_containers(&state, &id).await.map_err(|e| {
        ApiError::Unavailable(format!(
            "the app is off the proxy, but its container could not be stopped ({e:#}). Try again"
        ))
    })?;
    auth.audit(&state, "app.stop", Some(&id), None).await?;
    Ok(Json(load_app(&state, &auth.org_id, &id).await?))
}

#[utoipa::path(get, path = "/api/v1/apps/{id}/deployments", tag = "deployments", params(("id" = String, Path)),
    responses((status = 200, body = Vec<Deployment>), (status = 404, body = ErrorBody)))]
async fn list_deployments(
    State(state): State<AppState>,
    auth: Auth<Viewer>,
    Path(id): Path<String>,
) -> ApiResult<Json<Vec<Deployment>>> {
    require_app(&state, &auth.org_id, &id).await?;
    let rows = sqlx::query(deployment_select!(
        "WHERE app_id = ? AND org_id = ? ORDER BY created_at DESC, id DESC LIMIT 50"
    ))
    .bind(&id)
    .bind(&auth.org_id)
    .fetch_all(&state.db)
    .await?;
    Ok(Json(
        rows.iter()
            .map(deployment_from_row)
            .collect::<ApiResult<_>>()?,
    ))
}

#[utoipa::path(get, path = "/api/v1/deployments/{id}", tag = "deployments", params(("id" = String, Path)),
    responses((status = 200, body = Deployment), (status = 404, body = ErrorBody)))]
async fn get_deployment(
    State(state): State<AppState>,
    auth: Auth<Viewer>,
    Path(id): Path<String>,
) -> ApiResult<Json<Deployment>> {
    Ok(Json(load_deployment(&state, &auth.org_id, &id).await?))
}

// ---------------------------------------------------------------------- logs

fn sse(
    rx: mpsc::Receiver<Result<Event, Infallible>>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    Sse::new(ReceiverStream::new(rx)).keep_alive(KeepAlive::new().interval(Duration::from_secs(15)))
}

fn log_event(line: &LogLine) -> Event {
    Event::default()
        .event("log")
        .id(line.seq.to_string())
        .data(serde_json::to_string(line).unwrap_or_default())
}

#[derive(Deserialize, utoipa::IntoParams)]
#[into_params(parameter_in = Query)]
struct LogQuery {
    /// Send only lines after this sequence number.
    after: Option<i64>,
}

/// A deployment's build and deploy output as server-sent events.
///
/// Sends a `log` event per line (data is a `LogLine`), then one `end` event
/// carrying the final status once the deployment finishes. Reconnecting with
/// `Last-Event-ID` or `?after=` resumes without repeats.
#[utoipa::path(get, path = "/api/v1/deployments/{id}/logs", tag = "deployments",
    params(("id" = String, Path), LogQuery),
    responses((status = 200, content_type = "text/event-stream", body = String),
        (status = 404, body = ErrorBody)))]
async fn deployment_logs(
    State(state): State<AppState>,
    auth: Auth<Viewer>,
    Path(id): Path<String>,
    Query(q): Query<LogQuery>,
    headers: HeaderMap,
) -> ApiResult<Sse<impl Stream<Item = Result<Event, Infallible>>>> {
    load_deployment(&state, &auth.org_id, &id).await?;
    let mut after = q
        .after
        .or_else(|| headers.get("last-event-id")?.to_str().ok()?.parse().ok())
        .unwrap_or(0);

    let (tx, rx) = mpsc::channel(256);
    tokio::spawn(async move {
        // Subscribe before reading so a line written between the read and
        // the wait still wakes this task.
        let mut activity = state.deploys.subscribe();
        loop {
            let status: Option<String> =
                sqlx::query_scalar("SELECT status FROM deployments WHERE id = ?")
                    .bind(&id)
                    .fetch_optional(&state.db)
                    .await
                    .ok()
                    .flatten();
            let rows = sqlx::query(
                "SELECT seq, ts, stream, line FROM deployment_logs
                  WHERE deployment_id = ? AND seq > ? ORDER BY seq LIMIT 500",
            )
            .bind(&id)
            .bind(after)
            .fetch_all(&state.db)
            .await
            .unwrap_or_default();
            let drained = rows.len() < 500;
            for r in &rows {
                let line = LogLine {
                    seq: r.get("seq"),
                    ts: r.get("ts"),
                    stream: r.get("stream"),
                    line: r.get("line"),
                };
                after = line.seq;
                if tx.send(Ok(log_event(&line))).await.is_err() {
                    return; // the client went away
                }
            }
            if !drained {
                continue;
            }
            // The status was read before the lines, so a finished status
            // means every line has been sent.
            let finished = match status.as_deref() {
                None => true, // deleted
                Some(s) => !matches!(s, "queued" | "building" | "deploying"),
            };
            if finished {
                let _ = tx
                    .send(Ok(Event::default()
                        .event("end")
                        .data(status.unwrap_or_else(|| "deleted".into()))))
                    .await;
                return;
            }
            tokio::select! {
                changed = activity.changed() => if changed.is_err() { return },
                () = tx.closed() => return,
            }
        }
    });
    Ok(sse(rx))
}

/// The running container's output as server-sent events: the last 200 lines,
/// then live output. Each `log` event's data is a `LogLine`.
#[utoipa::path(get, path = "/api/v1/apps/{id}/logs", tag = "apps", params(("id" = String, Path)),
    responses((status = 200, content_type = "text/event-stream", body = String),
        (status = 404, body = ErrorBody), (status = 409, body = ErrorBody)))]
async fn app_logs(
    State(state): State<AppState>,
    auth: Auth<Viewer>,
    Path(id): Path<String>,
) -> ApiResult<Sse<impl Stream<Item = Result<Event, Infallible>>>> {
    let container: Option<Option<String>> = sqlx::query_scalar(
        "SELECT d.container_id FROM apps a
           LEFT JOIN deployments d ON d.id = a.current_deployment_id
          WHERE a.id = ? AND a.org_id = ?",
    )
    .bind(&id)
    .bind(&auth.org_id)
    .fetch_optional(&state.db)
    .await?;
    let container = container.ok_or(ApiError::NotFound("app"))?.ok_or_else(|| {
        ApiError::Conflict("the app is not running, so it has no logs yet. Deploy it first".into())
    })?;

    let mut seq = 0i64;
    let stream = state
        .runtime
        .logs(&container, 200, true)
        // An engine error ends the stream; the dashboard reconnects.
        .take_while(|item| std::future::ready(item.is_ok()))
        .filter_map(move |item| {
            let msg = item.ok();
            seq += 1;
            let event = msg.map(|m| {
                Ok(log_event(&LogLine {
                    seq,
                    ts: now(),
                    stream: m.stream.to_string(),
                    line: m.line,
                }))
            });
            std::future::ready(event)
        });
    Ok(Sse::new(stream).keep_alive(KeepAlive::new().interval(Duration::from_secs(15))))
}

// ------------------------------------------------------------------ env vars

/// An app's variables. Secret values are never included; reveal them one at
/// a time.
#[utoipa::path(get, path = "/api/v1/apps/{id}/env", tag = "env", params(("id" = String, Path)),
    responses((status = 200, body = Vec<EnvVar>), (status = 404, body = ErrorBody)))]
async fn list_env(
    State(state): State<AppState>,
    auth: Auth<Viewer>,
    Path(id): Path<String>,
) -> ApiResult<Json<Vec<EnvVar>>> {
    require_app(&state, &auth.org_id, &id).await?;
    let rows = sqlx::query(
        "SELECT key, value_enc, is_secret, updated_at FROM env_vars WHERE app_id = ? ORDER BY key",
    )
    .bind(&id)
    .fetch_all(&state.db)
    .await?;
    let vars = rows
        .iter()
        .map(|r| {
            let is_secret: bool = r.get("is_secret");
            let value = if is_secret {
                None
            } else {
                Some(state.vault.decrypt(&r.get::<String, _>("value_enc"))?)
            };
            Ok(EnvVar {
                key: r.get("key"),
                value,
                is_secret,
                updated_at: r.get("updated_at"),
            })
        })
        .collect::<ApiResult<Vec<_>>>()?;
    Ok(Json(vars))
}

/// Create or replace one variable. Takes effect on the next deployment.
#[utoipa::path(put, path = "/api/v1/apps/{id}/env", tag = "env", params(("id" = String, Path)),
    request_body = SetEnvVarRequest,
    responses((status = 200, body = EnvVar), (status = 404, body = ErrorBody), (status = 422, body = ErrorBody)))]
async fn set_env(
    State(state): State<AppState>,
    auth: Auth<Developer>,
    Path(id): Path<String>,
    Json(req): Json<SetEnvVarRequest>,
) -> ApiResult<Json<EnvVar>> {
    require_app(&state, &auth.org_id, &id).await?;
    validate_env_key(&req.key)?;
    validate_env_value(&req.value)?;
    let at = now();
    sqlx::query(
        "INSERT INTO env_vars (id, org_id, app_id, key, value_enc, is_secret, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?)
         ON CONFLICT (app_id, key) DO UPDATE SET
            value_enc = excluded.value_enc, is_secret = excluded.is_secret, updated_at = excluded.updated_at",
    )
    .bind(new_id())
    .bind(&auth.org_id)
    .bind(&id)
    .bind(&req.key)
    .bind(state.vault.encrypt(&req.value)?)
    .bind(req.is_secret)
    .bind(&at)
    .execute(&state.db)
    .await?;
    // The key is recorded, never the value.
    auth.audit(&state, "env.set", Some(&id), Some(&req.key))
        .await?;
    Ok(Json(EnvVar {
        value: (!req.is_secret).then_some(req.value),
        key: req.key,
        is_secret: req.is_secret,
        updated_at: at,
    }))
}

#[utoipa::path(delete, path = "/api/v1/apps/{id}/env/{key}", tag = "env",
    params(("id" = String, Path), ("key" = String, Path)),
    responses((status = 204), (status = 404, body = ErrorBody)))]
async fn delete_env(
    State(state): State<AppState>,
    auth: Auth<Developer>,
    Path((id, key)): Path<(String, String)>,
) -> ApiResult<StatusCode> {
    let n = sqlx::query("DELETE FROM env_vars WHERE app_id = ? AND org_id = ? AND key = ?")
        .bind(&id)
        .bind(&auth.org_id)
        .bind(&key)
        .execute(&state.db)
        .await?
        .rows_affected();
    if n == 0 {
        return Err(ApiError::NotFound("variable"));
    }
    auth.audit(&state, "env.delete", Some(&id), Some(&key))
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Read one secret's value. Every reveal is written to the audit log.
#[utoipa::path(post, path = "/api/v1/apps/{id}/env/{key}/reveal", tag = "env",
    params(("id" = String, Path), ("key" = String, Path)),
    responses((status = 200, body = RevealedEnvVar), (status = 404, body = ErrorBody)))]
async fn reveal_env(
    State(state): State<AppState>,
    auth: Auth<Developer>,
    Path((id, key)): Path<(String, String)>,
) -> ApiResult<Json<RevealedEnvVar>> {
    let enc: Option<String> = sqlx::query_scalar(
        "SELECT value_enc FROM env_vars WHERE app_id = ? AND org_id = ? AND key = ?",
    )
    .bind(&id)
    .bind(&auth.org_id)
    .bind(&key)
    .fetch_optional(&state.db)
    .await?;
    let enc = enc.ok_or(ApiError::NotFound("variable"))?;
    // Audit first: a reveal that cannot be recorded does not happen.
    auth.audit(&state, "env.reveal", Some(&id), Some(&key))
        .await?;
    Ok(Json(RevealedEnvVar {
        value: state.vault.decrypt(&enc)?,
        key,
    }))
}

// ------------------------------------------------------------------- domains

#[utoipa::path(get, path = "/api/v1/apps/{id}/domains", tag = "domains", params(("id" = String, Path)),
    responses((status = 200, body = Vec<Domain>), (status = 404, body = ErrorBody)))]
async fn list_domains(
    State(state): State<AppState>,
    auth: Auth<Viewer>,
    Path(id): Path<String>,
) -> ApiResult<Json<Vec<Domain>>> {
    require_app(&state, &auth.org_id, &id).await?;
    let rows = sqlx::query(
        "SELECT id, host, tls_mode, access, port, created_at FROM domains
         WHERE app_id = ? ORDER BY host, port",
    )
    .bind(&id)
    .fetch_all(&state.db)
    .await?;
    let domains = rows
        .iter()
        .map(|r| {
            Ok(Domain {
                id: r.get("id"),
                host: r.get("host"),
                tls_mode: parse_enum(r.get("tls_mode"))?,
                access: parse_enum(r.get("access"))?,
                port: r
                    .get::<Option<i64>, _>("port")
                    .and_then(|p| u16::try_from(p).ok()),
                created_at: r.get("created_at"),
            })
        })
        .collect::<ApiResult<Vec<_>>>()?;
    Ok(Json(domains))
}

/// Route a hostname to the app. Applied to the proxy at once if the app is running.
#[utoipa::path(post, path = "/api/v1/apps/{id}/domains", tag = "domains", params(("id" = String, Path)),
    request_body = AddDomainRequest,
    responses((status = 201, body = Domain), (status = 404, body = ErrorBody),
        (status = 409, body = ErrorBody), (status = 422, body = ErrorBody)))]
async fn add_domain(
    State(state): State<AppState>,
    auth: Auth<Developer>,
    Path(id): Path<String>,
    Json(req): Json<AddDomainRequest>,
) -> ApiResult<(StatusCode, Json<Domain>)> {
    require_app(&state, &auth.org_id, &id).await?;
    let domain = Domain {
        id: new_id(),
        host: validate_host(&req.host)?,
        tls_mode: req.tls_mode,
        access: req.access,
        port: req.port,
        created_at: now(),
    };
    check_domain(&state, &domain).await?;
    let inserted = sqlx::query(
        "INSERT INTO domains (id, org_id, app_id, host, tls_mode, access, port, created_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&domain.id)
    .bind(&auth.org_id)
    .bind(&id)
    .bind(&domain.host)
    .bind(domain.tls_mode.as_str())
    .bind(domain.access.as_str())
    .bind(domain.port.map(i64::from))
    .bind(&domain.created_at)
    .execute(&state.db)
    .await;
    match inserted.map_err(ApiError::from) {
        Ok(_) => {}
        Err(ApiError::Conflict(_)) => {
            let at = domain.port.map(|p| format!(":{p}")).unwrap_or_default();
            return Err(ApiError::Conflict(format!(
                "{}{at} is already routed to an app",
                domain.host
            )));
        }
        Err(e) => return Err(e),
    }
    sync_proxy(&state, "adding a domain").await;
    auth.audit(&state, "domain.add", Some(&id), Some(&domain.host))
        .await?;
    Ok((StatusCode::CREATED, Json(domain)))
}

/// Rules a domain must meet beyond a valid host name.
async fn check_domain(state: &AppState, d: &Domain) -> ApiResult<()> {
    let bad = |m: &str| Err(ApiError::Validation(m.to_owned()));
    if d.port.is_none()
        && proxy::dashboard_domain(&state.db)
            .await?
            .is_some_and(|dash| dash.host == d.host)
    {
        return bad("this name serves the Rig dashboard; pick another");
    }
    if d.tls_mode == TlsMode::Tailscale && !d.host.ends_with(".ts.net") {
        return bad("Tailscale certificates only cover this machine's .ts.net name");
    }
    let Some(port) = d.port else {
        return Ok(());
    };
    if port < 1024 {
        return bad("Pick a port from 1024 to 65535");
    }
    let (http, https) = state.proxy.ports();
    if port == http || port == https {
        return bad("That port is the proxy's own; leave the port empty to use it");
    }
    if d.tls_mode == TlsMode::Auto {
        return bad("Public certificates need the standard HTTPS port; leave the port empty");
    }
    // One port is either all HTTPS or all plain HTTP.
    let modes: Vec<String> = sqlx::query_scalar("SELECT tls_mode FROM domains WHERE port = ?")
        .bind(i64::from(port))
        .fetch_all(&state.db)
        .await?;
    let plain = d.tls_mode == TlsMode::Off;
    if modes.iter().any(|m| (m == "off") != plain) {
        return bad("That port already serves the other kind of traffic (HTTP vs HTTPS)");
    }
    Ok(())
}

#[utoipa::path(delete, path = "/api/v1/apps/{id}/domains/{domain_id}", tag = "domains",
    params(("id" = String, Path), ("domain_id" = String, Path)),
    responses((status = 204), (status = 404, body = ErrorBody)))]
async fn delete_domain(
    State(state): State<AppState>,
    auth: Auth<Developer>,
    Path((id, domain_id)): Path<(String, String)>,
) -> ApiResult<StatusCode> {
    let host: Option<String> = sqlx::query_scalar(
        "DELETE FROM domains WHERE id = ? AND app_id = ? AND org_id = ? RETURNING host",
    )
    .bind(&domain_id)
    .bind(&id)
    .bind(&auth.org_id)
    .fetch_optional(&state.db)
    .await?;
    let host = host.ok_or(ApiError::NotFound("domain"))?;
    sync_proxy(&state, "removing a domain").await;
    auth.audit(&state, "domain.remove", Some(&id), Some(&host))
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn volume_paths() {
        let v =
            |p: &[&str]| validate_volumes(&p.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>());
        assert_eq!(
            v(&["/data/", " /app/uploads"]).unwrap(),
            ["/data", "/app/uploads"]
        );
        for bad in [
            &["data"][..],
            &["/"],
            &["/etc"],
            &["/var/run/docker.sock"],
            &["/proc/1"],
            &["/a/../etc"],
            &["/data:/etc"],
            &["/data", "/data/sub"],
            &["/a", "/a"],
        ] {
            assert!(v(bad).is_err(), "{bad:?}");
        }
        assert!(v(&["/d"; MAX_VOLUMES + 1]).is_err());
    }

    #[test]
    fn hosts() {
        assert_eq!(
            validate_host(" App.Example.com. ").unwrap(),
            "app.example.com"
        );
        assert!(validate_host("localhost").is_ok());
        for bad in [
            "",
            "*.example.com",
            "a..b",
            "-a.com",
            "a.com/x",
            "a.com:80",
            "http://a.com",
            "a b.com",
        ] {
            assert!(validate_host(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn images_and_env_keys() {
        for ok in [
            "nginx",
            "nginx:1.27",
            "ghcr.io/acme/api:latest",
            "redis@sha256:abc",
        ] {
            assert!(validate_image(ok).is_ok(), "{ok}");
        }
        for bad in ["", "-v /:/host", "nginx; rm -rf", "a b"] {
            assert!(validate_image(bad).is_err(), "{bad}");
        }
        assert!(validate_env_key("DATABASE_URL").is_ok());
        assert!(validate_env_key("_x1").is_ok());
        for bad in ["", "1A", "A-B", "A=B", "A B"] {
            assert!(validate_env_key(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn source_rules() {
        assert!(validate_source(SourceKind::Image, None, None, None, None).is_err());
        let s = validate_source(
            SourceKind::Git,
            None,
            Some("https://github.com/a/b"),
            Some(""),
            None,
        )
        .unwrap();
        assert_eq!(s.dockerfile_path, "Dockerfile");
        assert_eq!(s.git_branch, None);
        assert!(validate_source(SourceKind::Git, None, Some("file:///etc"), None, None).is_err());
        assert!(
            validate_source(
                SourceKind::Git,
                None,
                Some("https://github.com/a/b"),
                Some("--upload-pack=x"),
                None
            )
            .is_err()
        );
    }
}
