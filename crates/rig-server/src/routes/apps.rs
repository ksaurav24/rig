//! Apps and everything that hangs off one: deployments, logs, environment
//! variables and domains.

use std::{convert::Infallible, time::Duration};

use axum::{
    Json,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::{
        IntoResponse, Response,
        sse::{Event, KeepAlive, Sse},
    },
};
use futures_util::{Stream, StreamExt};
use rig_api::{
    Access, AddDomainRequest, App, AppStatus, AppWebhook, BulkSetEnvRequest, CreateAppRequest,
    Deployment, DeploymentEndEvent, DeploymentStatus, Domain, EnvVar, ErrorBody, LogLine,
    RevealedEnvVar, SetEnvVarRequest, SourceKind, TlsMode, UpdateAppRequest,
};
use serde::Deserialize;
use sqlx::Row;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use utoipa_axum::{router::OpenApiRouter, routes};

use crate::{
    auth::{self, Admin, Auth, Developer, MinRole, Viewer},
    crypto::random_token,
    deploy::{self, NewDeployment, validate_git_url_with, validate_relative_path},
    error::{ApiError, ApiResult},
    proxy,
    routes::auth::{slugify, validate_name},
    runtime::LogOptions,
    state::{AppState, new_id, now},
};

pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(list_apps, create_app))
        .routes(routes!(resolve_app))
        .routes(routes!(get_app, update_app, delete_app))
        .routes(routes!(deploy_app))
        .routes(routes!(get_webhook))
        .routes(routes!(rotate_webhook))
        .routes(routes!(start_app))
        .routes(routes!(restart_app))
        .routes(routes!(stop_app))
        .routes(routes!(app_logs))
        .routes(routes!(list_deployments))
        .routes(routes!(get_deployment))
        .routes(routes!(rollback))
        .routes(routes!(cancel_deployment))
        .routes(routes!(deployment_logs))
        .routes(routes!(list_env, set_env))
        .routes(routes!(set_env_bulk))
        .routes(routes!(delete_env))
        .routes(routes!(reveal_env))
        .routes(routes!(list_domains, add_domain))
        .routes(routes!(delete_domain))
}

// ---------------------------------------------------------------- validation

const MAX_ENV_VALUE: usize = 32 * 1024;

fn stored_enum<T>(parsed: Option<T>, kind: &str, raw: &str) -> ApiResult<T> {
    parsed.ok_or_else(|| ApiError::Internal(anyhow::anyhow!("unexpected stored {kind}: {raw:?}")))
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
        (SELECT group_concat(h.host, char(10)) FROM
           (SELECT host FROM domains WHERE app_id = a.id ORDER BY host) h) AS domains,
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

fn app_from_row(r: &sqlx::sqlite::SqliteRow) -> ApiResult<App> {
    let raw_status: &str = r.get("status");
    let status = stored_enum(AppStatus::parse(raw_status), "app status", raw_status)?;
    let raw_source: &str = r.get("source_kind");
    let source_kind = stored_enum(SourceKind::parse(raw_source), "source_kind", raw_source)?;
    Ok(App {
        id: r.get("id"),
        project_id: r.get("project_id"),
        project_name: r.get("project_name"),
        name: r.get("name"),
        slug: r.get("slug"),
        source_kind,
        image: r.get("image"),
        git_url: r.get("git_url"),
        git_branch: r.get("git_branch"),
        dockerfile_path: r.get("dockerfile_path"),
        port: u16::try_from(r.get::<i64, _>("port")).unwrap_or(80),
        memory_mb: opt_u32(r, "memory_mb"),
        cpu_millis: opt_u32(r, "cpu_millis"),
        health_path: r.get("health_path"),
        host_port: (status == AppStatus::Running)
            .then(|| {
                r.get::<Option<i64>, _>("host_port")
                    .and_then(|p| u16::try_from(p).ok())
            })
            .flatten(),
        status,
        current_deployment_id: r.get("current_deployment_id"),
        domains: r
            .get::<Option<String>, _>("domains")
            .map(|d| d.lines().map(str::to_owned).collect())
            .unwrap_or_default(),
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

pub(crate) async fn load_app(state: &AppState, org_id: &str, id: &str) -> ApiResult<App> {
    let row = sqlx::query(app_select!("WHERE a.id = ? AND a.org_id = ?"))
        .bind(id)
        .bind(org_id)
        .fetch_optional(&state.db)
        .await?
        .ok_or(ApiError::NotFound("app"))?;
    app_from_row(&row)
}

/// Fail with 404 unless the app belongs to the caller's organization.
pub(crate) async fn require_app(state: &AppState, org_id: &str, id: &str) -> ApiResult<()> {
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
    let rows = match q.project_id.as_deref() {
        Some(project_id) => {
            sqlx::query(app_select!(
                "WHERE a.org_id = ?1 AND a.project_id = ?2
                  ORDER BY p.name COLLATE NOCASE, a.name COLLATE NOCASE"
            ))
            .bind(&auth.org_id)
            .bind(project_id)
            .fetch_all(&state.db)
            .await?
        }
        None => {
            sqlx::query(app_select!(
                "WHERE a.org_id = ?1
                  ORDER BY p.name COLLATE NOCASE, a.name COLLATE NOCASE"
            ))
            .bind(&auth.org_id)
            .fetch_all(&state.db)
            .await?
        }
    };
    let apps = rows
        .iter()
        .map(app_from_row)
        .collect::<ApiResult<Vec<_>>>()?;
    Ok(Json(apps))
}

#[derive(Deserialize, utoipa::IntoParams)]
#[into_params(parameter_in = Query)]
struct ResolveAppQuery {
    /// App id, name, slug, or `project/app`.
    #[serde(rename = "ref")]
    r#ref: String,
}

/// Resolve an app by id, name, slug, or `project/app`.
#[utoipa::path(get, path = "/api/v1/apps/resolve", tag = "apps", params(ResolveAppQuery),
    responses((status = 200, body = App), (status = 404, body = ErrorBody), (status = 409, body = ErrorBody)))]
async fn resolve_app(
    State(state): State<AppState>,
    auth: Auth<Viewer>,
    Query(q): Query<ResolveAppQuery>,
) -> ApiResult<Json<App>> {
    let wanted = q.r#ref.trim();
    if wanted.is_empty() {
        return Err(ApiError::validation("ref cannot be empty"));
    }
    let (project_filter, name_filter) = match wanted.split_once('/') {
        Some((p, n)) => (Some(p.trim()), n.trim()),
        None => {
            // Fast path: exact id lookup (skipped when `/` is present, since ids never contain `/`).
            if let Ok(app) = load_app(&state, &auth.org_id, wanted).await {
                return Ok(Json(app));
            }
            (None, wanted)
        }
    };
    let rows = match project_filter {
        Some(project) => {
            sqlx::query(app_select!(
                "WHERE a.org_id = ?1
                   AND (a.name = ?2 COLLATE NOCASE OR a.slug = ?2 COLLATE NOCASE)
                   AND (p.id = ?3 OR p.name = ?3 COLLATE NOCASE OR p.slug = ?3 COLLATE NOCASE)
                 ORDER BY p.name COLLATE NOCASE, a.name COLLATE NOCASE"
            ))
            .bind(&auth.org_id)
            .bind(name_filter)
            .bind(project)
            .fetch_all(&state.db)
            .await?
        }
        None => {
            sqlx::query(app_select!(
                "WHERE a.org_id = ?1
                   AND (a.name = ?2 COLLATE NOCASE OR a.slug = ?2 COLLATE NOCASE)
                 ORDER BY p.name COLLATE NOCASE, a.name COLLATE NOCASE"
            ))
            .bind(&auth.org_id)
            .bind(name_filter)
            .fetch_all(&state.db)
            .await?
        }
    };

    match rows.len() {
        0 => Err(ApiError::NotFound("app")),
        1 => Ok(Json(app_from_row(&rows[0])?)),
        _ => {
            let names: Vec<String> = rows
                .iter()
                .map(|r| {
                    format!(
                        "{}/{}",
                        r.get::<String, _>("project_name"),
                        r.get::<String, _>("name")
                    )
                })
                .collect();
            Err(ApiError::Conflict(format!(
                "{wanted} matches several apps: {}. Use project/app",
                names.join(", ")
            )))
        }
    }
}

/// The source fields of an app after validation.
struct Source {
    image: Option<String>,
    git_url: Option<String>,
    git_branch: Option<String>,
    dockerfile_path: String,
}

#[cfg(test)]
fn validate_source(
    kind: SourceKind,
    image: Option<&str>,
    git_url: Option<&str>,
    git_branch: Option<&str>,
    dockerfile_path: Option<&str>,
) -> ApiResult<Source> {
    validate_source_with(kind, image, git_url, git_branch, dockerfile_path, false)
}

fn validate_source_with(
    kind: SourceKind,
    image: Option<&str>,
    git_url: Option<&str>,
    git_branch: Option<&str>,
    dockerfile_path: Option<&str>,
    allow_private_git: bool,
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
        SourceKind::Local => Ok(Source {
            image: None,
            git_url: None,
            git_branch: None,
            dockerfile_path: dockerfile_path.to_string(),
        }),
        SourceKind::Git => {
            let url = git_url.unwrap_or_default().trim();
            validate_git_url_with(url, allow_private_git).map_err(ApiError::validation)?;
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
    let source = validate_source_with(
        req.source_kind,
        req.image.as_deref(),
        req.git_url.as_deref(),
        req.git_branch.as_deref(),
        req.dockerfile_path.as_deref(),
        state.config.allow_private_git,
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
    let webhook_secret_enc = state.vault.encrypt(&random_token(24))?;
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
    .bind(&webhook_secret_enc)
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
    let source = validate_source_with(
        app.source_kind,
        req.image.as_deref().or(app.image.as_deref()),
        req.git_url.as_deref().or(app.git_url.as_deref()),
        // An empty branch clears it, which means the repository's default.
        req.git_branch.as_deref().or(app.git_branch.as_deref()),
        req.dockerfile_path
            .as_deref()
            .or(Some(&app.dockerfile_path)),
        state.config.allow_private_git,
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
    let next_volumes = req.volumes.as_deref().map(validate_volumes).transpose()?;

    // The slug is kept: it names containers and images that already exist.
    let at = now();
    let mut tx = state.db.begin().await?;
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
    .bind(&at)
    .bind(&id)
    .bind(&auth.org_id)
    .execute(&mut *tx)
    .await?;

    if let Some(paths) = next_volumes {
        let existing: Vec<(String, String)> = sqlx::query_as(
            "SELECT id, mount_path FROM app_volumes WHERE app_id = ? AND kind = 'volume'",
        )
        .bind(&id)
        .fetch_all(&mut *tx)
        .await?;
        for (vol_id, mount_path) in &existing {
            if !paths.iter().any(|p| p == mount_path) {
                sqlx::query("DELETE FROM app_volumes WHERE id = ?")
                    .bind(vol_id)
                    .execute(&mut *tx)
                    .await?;
            }
        }
        for path in &paths {
            if !existing.iter().any(|(_, mp)| mp == path) {
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
        }
    }

    tx.commit().await?;
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
    let raw_status: &str = r.get("status");
    Ok(Deployment {
        id: r.get("id"),
        app_id: r.get("app_id"),
        status: stored_enum(
            DeploymentStatus::parse(raw_status),
            "deployment status",
            raw_status,
        )?,
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

pub(crate) async fn start_deployment<R: MinRole>(
    state: &AppState,
    auth: &Auth<R>,
    app_id: &str,
    trigger: &str,
    image_ref: Option<&str>,
    image_id: Option<&str>,
) -> ApiResult<Deployment> {
    start_deployment_with(state, auth, app_id, trigger, image_ref, image_id, None, None).await
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn start_deployment_with<R: MinRole>(
    state: &AppState,
    auth: &Auth<R>,
    app_id: &str,
    trigger: &str,
    image_ref: Option<&str>,
    image_id: Option<&str>,
    source_dir: Option<&str>,
    commit_msg: Option<&str>,
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
            source_dir,
            commit_msg,
        },
    )
    .await
    .map_err(|e| {
        if deploy::is_active_deploy_conflict(&e) {
            ApiError::Conflict(
                "a deployment is already in progress for this app. Wait for it to finish".into(),
            )
        } else {
            e.into()
        }
    })?;
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
    let secret_enc: Option<String> =
        sqlx::query_scalar("SELECT webhook_secret FROM apps WHERE id = ? AND org_id = ?")
            .bind(&id)
            .bind(&auth.org_id)
            .fetch_optional(&state.db)
            .await?;
    let secret_enc = secret_enc.ok_or(ApiError::NotFound("app not found"))?;
    let secret = state.vault.decrypt(&secret_enc)?;
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
    let secret_enc = state.vault.encrypt(&secret)?;
    let done = sqlx::query(
        "UPDATE apps SET webhook_secret = ?, updated_at = ? WHERE id = ? AND org_id = ?",
    )
    .bind(&secret_enc)
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

/// Start a stopped or failed app, reusing its most recent built image when available.
#[utoipa::path(post, path = "/api/v1/apps/{id}/start", tag = "apps", params(("id" = String, Path)),
    responses((status = 202, body = Deployment), (status = 404, body = ErrorBody), (status = 409, body = ErrorBody)))]
async fn start_app(
    State(state): State<AppState>,
    auth: Auth<Developer>,
    Path(id): Path<String>,
) -> ApiResult<(StatusCode, Json<Deployment>)> {
    let app = load_app(&state, &auth.org_id, &id).await?;
    if app.status == rig_api::AppStatus::Running {
        return Err(ApiError::Conflict(format!("{} is already running", app.name)));
    }
    let last_image: Option<(Option<String>, Option<String>)> = sqlx::query_as(
        "SELECT image_ref, image_id FROM deployments
          WHERE app_id = ? AND org_id = ? AND image_ref IS NOT NULL
          ORDER BY created_at DESC, id DESC LIMIT 1",
    )
    .bind(&id)
    .bind(&auth.org_id)
    .fetch_optional(&state.db)
    .await?;
    let (image_ref, image_id) = last_image.unwrap_or((None, None));
    let trigger = if auth.via == crate::auth::Via::ApiToken {
        "api"
    } else {
        "manual"
    };
    let d = start_deployment(
        &state,
        &auth,
        &id,
        trigger,
        image_ref.as_deref(),
        image_id.as_deref(),
    )
    .await?;
    auth.audit(&state, "app.start", Some(&id), Some(&d.id))
        .await?;
    Ok((StatusCode::ACCEPTED, Json(d)))
}

/// Restart an app with a zero-downtime swap reusing its current image.
#[utoipa::path(post, path = "/api/v1/apps/{id}/restart", tag = "apps", params(("id" = String, Path)),
    responses((status = 202, body = Deployment), (status = 404, body = ErrorBody), (status = 409, body = ErrorBody)))]
async fn restart_app(
    State(state): State<AppState>,
    auth: Auth<Developer>,
    Path(id): Path<String>,
) -> ApiResult<(StatusCode, Json<Deployment>)> {
    let app = load_app(&state, &auth.org_id, &id).await?;
    let source_dep_id = app.current_deployment_id.clone();
    let image_pair: Option<(Option<String>, Option<String>)> = match source_dep_id {
        Some(dep_id) => {
            sqlx::query_as("SELECT image_ref, image_id FROM deployments WHERE id = ? AND org_id = ?")
                .bind(dep_id)
                .bind(&auth.org_id)
                .fetch_optional(&state.db)
                .await?
        }
        None => {
            sqlx::query_as(
                "SELECT image_ref, image_id FROM deployments
                  WHERE app_id = ? AND org_id = ? AND image_ref IS NOT NULL
                  ORDER BY created_at DESC, id DESC LIMIT 1",
            )
            .bind(&id)
            .bind(&auth.org_id)
            .fetch_optional(&state.db)
            .await?
        }
    };
    let Some((Some(image_ref), image_id)) = image_pair else {
        return Err(ApiError::Conflict(format!(
            "{} has not been deployed yet. Deploy it first",
            app.name
        )));
    };
    let trigger = if auth.via == crate::auth::Via::ApiToken {
        "api"
    } else {
        "manual"
    };
    let d = start_deployment(
        &state,
        &auth,
        &id,
        trigger,
        Some(&image_ref),
        image_id.as_deref(),
    )
    .await?;
    auth.audit(&state, "app.restart", Some(&id), Some(&d.id))
        .await?;
    Ok((StatusCode::ACCEPTED, Json(d)))
}

/// Cancel a queued or in-progress deployment.
#[utoipa::path(post, path = "/api/v1/deployments/{id}/cancel", tag = "deployments",
    params(("id" = String, Path)),
    responses((status = 200, body = Deployment), (status = 404, body = ErrorBody), (status = 409, body = ErrorBody)))]
async fn cancel_deployment(
    State(state): State<AppState>,
    auth: Auth<Developer>,
    Path(id): Path<String>,
) -> ApiResult<Json<Deployment>> {
    let dep = load_deployment(&state, &auth.org_id, &id).await?;
    if dep.status.is_finished() {
        return Err(ApiError::Conflict(
            "this deployment has already finished".into(),
        ));
    }
    let finished_at = now();
    let updated = sqlx::query(
        "UPDATE deployments SET status = 'cancelled', error = 'Cancelled by user', finished_at = ?
          WHERE id = ? AND org_id = ? AND status IN ('queued', 'building', 'deploying')",
    )
    .bind(&finished_at)
    .bind(&id)
    .bind(&auth.org_id)
    .execute(&state.db)
    .await?
    .rows_affected();
    if updated == 0 {
        return Err(ApiError::Conflict(
            "this deployment has already finished".into(),
        ));
    }
    state.deploys.signal_cancel(&id);
    sqlx::query(
        "UPDATE apps SET updated_at = ?,
                status = CASE WHEN current_deployment_id IS NULL THEN 'idle' ELSE 'running' END
          WHERE id = ? AND status = 'deploying'",
    )
    .bind(&finished_at)
    .bind(&dep.app_id)
    .execute(&state.db)
    .await?;
    state.deploys.touch_deployment(&id);
    auth.audit(&state, "deployment.cancel", Some(&id), Some(&dep.app_id))
        .await?;
    Ok(Json(load_deployment(&state, &auth.org_id, &id).await?))
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

#[derive(Deserialize, utoipa::IntoParams)]
#[into_params(parameter_in = Query)]
struct DeploymentListQuery {
    /// Maximum number of deployments to return (1–200, default 50).
    limit: Option<u32>,
    /// Return only deployments created before this deployment id.
    before: Option<String>,
}

#[utoipa::path(get, path = "/api/v1/apps/{id}/deployments", tag = "deployments",
    params(("id" = String, Path), DeploymentListQuery),
    responses((status = 200, body = Vec<Deployment>), (status = 404, body = ErrorBody)))]
async fn list_deployments(
    State(state): State<AppState>,
    auth: Auth<Viewer>,
    Path(id): Path<String>,
    Query(q): Query<DeploymentListQuery>,
) -> ApiResult<Json<Vec<Deployment>>> {
    require_app(&state, &auth.org_id, &id).await?;
    let limit = i64::from(q.limit.unwrap_or(50).clamp(1, 200));
    let rows = match q.before.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        Some(before_id) => {
            let cursor_created: Option<String> = sqlx::query_scalar(
                "SELECT created_at FROM deployments WHERE id = ? AND app_id = ? AND org_id = ?",
            )
            .bind(before_id)
            .bind(&id)
            .bind(&auth.org_id)
            .fetch_optional(&state.db)
            .await?;
            match cursor_created {
                Some(created_at) => {
                    sqlx::query(deployment_select!(
                        "WHERE app_id = ?1 AND org_id = ?2
                           AND (created_at < ?3 OR (created_at = ?3 AND id < ?4))
                         ORDER BY created_at DESC, id DESC LIMIT ?5"
                    ))
                    .bind(&id)
                    .bind(&auth.org_id)
                    .bind(&created_at)
                    .bind(before_id)
                    .bind(limit)
                    .fetch_all(&state.db)
                    .await?
                }
                None => {
                    sqlx::query(deployment_select!(
                        "WHERE app_id = ?1 AND org_id = ?2 AND id < ?3
                         ORDER BY created_at DESC, id DESC LIMIT ?4"
                    ))
                    .bind(&id)
                    .bind(&auth.org_id)
                    .bind(before_id)
                    .bind(limit)
                    .fetch_all(&state.db)
                    .await?
                }
            }
        }
        None => {
            sqlx::query(deployment_select!(
                "WHERE app_id = ?1 AND org_id = ?2 ORDER BY created_at DESC, id DESC LIMIT ?3"
            ))
            .bind(&id)
            .bind(&auth.org_id)
            .bind(limit)
            .fetch_all(&state.db)
            .await?
        }
    };
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
    /// When `after` is omitted, replay only the last `tail` lines (1–10000).
    tail: Option<u32>,
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
    let explicit_after = q
        .after
        .or_else(|| headers.get("last-event-id")?.to_str().ok()?.parse().ok());
    let mut after = match (explicit_after, q.tail) {
        (Some(a), _) => a,
        (None, Some(n)) => {
            let tail = i64::from(n.clamp(1, 10_000));
            let max_seq: i64 = sqlx::query_scalar(
                "SELECT COALESCE(MAX(seq), 0) FROM deployment_logs WHERE deployment_id = ?",
            )
            .bind(&id)
            .fetch_one(&state.db)
            .await?;
            (max_seq - tail).max(0)
        }
        (None, None) => 0,
    };

    let snap = auth.snapshot();
    let (tx, rx) = mpsc::channel(256);
    tokio::spawn(async move {
        // Subscribe before reading so a line written between the read and
        // the wait still wakes this task.
        let mut activity = state.deploys.subscribe_deployment(&id);
        let mut auth_tick = tokio::time::interval(Duration::from_secs(30));
        auth_tick.tick().await;
        loop {
            if !auth::verify_still_valid(&state, &snap, rig_api::Role::Viewer).await {
                return;
            }
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
                let final_status = status.unwrap_or_else(|| "deleted".into());
                let domains: Vec<String> = sqlx::query_scalar(
                    "SELECT host FROM domains
                      WHERE app_id = (SELECT app_id FROM deployments WHERE id = ?)
                      ORDER BY host",
                )
                .bind(&id)
                .fetch_all(&state.db)
                .await
                .unwrap_or_default();
                let payload = serde_json::to_string(&DeploymentEndEvent {
                    status: final_status,
                    domains,
                })
                .unwrap_or_default();
                let _ = tx.send(Ok(Event::default().event("end").data(payload))).await;
                return;
            }
            tokio::select! {
                changed = activity.changed() => if changed.is_err() { return },
                _ = auth_tick.tick() => {}
                () = tx.closed() => return,
            }
        }
    });
    Ok(sse(rx))
}

#[derive(Deserialize, utoipa::IntoParams)]
#[into_params(parameter_in = Query)]
struct AppLogQuery {
    /// Number of trailing lines to replay (1–10000, default 200).
    tail: Option<u32>,
    /// Keep streaming live lines after the initial replay (default true).
    follow: Option<bool>,
    /// Only return lines since this time (RFC 3339, Unix seconds, or relative like `10m`, `2h`, `30s`, `1d`).
    since: Option<String>,
    /// Request original container timestamps on each line.
    timestamps: Option<bool>,
    /// Case-insensitive substring filter applied to each log line.
    grep: Option<String>,
}

/// Parse a `since` query parameter into Unix seconds.
fn parse_since(raw: &str) -> ApiResult<i64> {
    let raw = raw.trim();
    if let Ok(secs) = raw.parse::<i64>() {
        return Ok(secs);
    }
    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(raw) {
        return Ok(dt.timestamp());
    }
    if raw.len() >= 2 {
        let (num, unit) = raw.split_at(raw.len() - 1);
        if let Ok(n) = num.parse::<i64>()
            && n > 0
        {
            let secs = match unit {
                "s" | "S" => Some(n),
                "m" | "M" => n.checked_mul(60),
                "h" | "H" => n.checked_mul(3600),
                "d" | "D" => n.checked_mul(86400),
                _ => None,
            };
            if let Some(delta) = secs {
                return Ok((chrono::Utc::now().timestamp() - delta).max(0));
            }
        }
    }
    Err(ApiError::validation(
        "invalid `since` value; use a duration like 10m or 2h, Unix seconds, or RFC 3339",
    ))
}

/// The running container's output as server-sent events: the last `tail` lines,
/// then live output when `follow=true`. Each `log` event's data is a `LogLine`.
#[utoipa::path(get, path = "/api/v1/apps/{id}/logs", tag = "apps",
    params(("id" = String, Path), AppLogQuery),
    responses((status = 200, content_type = "text/event-stream", body = String),
        (status = 404, body = ErrorBody), (status = 409, body = ErrorBody), (status = 422, body = ErrorBody)))]
async fn app_logs(
    State(state): State<AppState>,
    auth: Auth<Viewer>,
    Path(id): Path<String>,
    Query(q): Query<AppLogQuery>,
) -> ApiResult<Sse<impl Stream<Item = Result<Event, Infallible>>>> {
    let since = q
        .since
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(parse_since)
        .transpose()?;
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

    let opts = LogOptions {
        tail: q.tail.unwrap_or(200).clamp(1, 10_000),
        follow: q.follow.unwrap_or(true),
        since,
        timestamps: q.timestamps.unwrap_or(false),
    };
    let grep = q
        .grep
        .map(|g| g.to_lowercase())
        .filter(|g| !g.is_empty());

    let snap = auth.snapshot();
    let (tx, rx) = mpsc::channel(256);
    tokio::spawn(async move {
        let mut raw = state.runtime.logs_with(&container, opts);
        let mut seq = 0i64;
        let mut auth_tick = tokio::time::interval(Duration::from_secs(30));
        auth_tick.tick().await;
        loop {
            tokio::select! {
                item = raw.next() => match item {
                    Some(Ok(msg)) => {
                        if let Some(needle) = &grep
                            && !msg.line.to_lowercase().contains(needle)
                        {
                            continue;
                        }
                        seq += 1;
                        let ev = log_event(&LogLine {
                            seq,
                            ts: msg.ts.unwrap_or_else(now),
                            stream: msg.stream.to_string(),
                            line: msg.line,
                        });
                        if tx.send(Ok(ev)).await.is_err() {
                            return;
                        }
                    }
                    Some(Err(_)) | None => return,
                },
                _ = auth_tick.tick() => {
                    if !auth::verify_still_valid(&state, &snap, rig_api::Role::Viewer).await {
                        return;
                    }
                }
                () = tx.closed() => return,
            }
        }
    });
    Ok(sse(rx))
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

const MAX_BULK_ENV: usize = 256;

async fn apply_bulk_env(
    state: &AppState,
    auth: &Auth<Developer>,
    id: &str,
    req: BulkSetEnvRequest,
) -> ApiResult<Vec<EnvVar>> {
    require_app(state, &auth.org_id, id).await?;
    if req.vars.len() > MAX_BULK_ENV {
        return Err(ApiError::validation(format!(
            "set at most {MAX_BULK_ENV} variables in one request"
        )));
    }
    let mut prepared = Vec::with_capacity(req.vars.len());
    for var in &req.vars {
        validate_env_key(&var.key)?;
        validate_env_value(&var.value)?;
        if prepared
            .iter()
            .any(|(k, _, _, _): &(&str, String, bool, &str)| *k == var.key)
        {
            return Err(ApiError::validation(format!(
                "the variable {} is set twice",
                var.key
            )));
        }
        prepared.push((
            var.key.as_str(),
            state.vault.encrypt(&var.value)?,
            var.is_secret,
            var.value.as_str(),
        ));
    }

    let at = now();
    let mut tx = state.db.begin().await?;
    if req.replace {
        sqlx::query("DELETE FROM env_vars WHERE app_id = ? AND org_id = ?")
            .bind(id)
            .bind(&auth.org_id)
            .execute(&mut *tx)
            .await?;
    }
    let mut saved = Vec::with_capacity(prepared.len());
    for (key, value_enc, is_secret, plain_value) in prepared {
        sqlx::query(
            "INSERT INTO env_vars (id, org_id, app_id, key, value_enc, is_secret, updated_at)
             VALUES (?, ?, ?, ?, ?, ?, ?)
             ON CONFLICT (app_id, key) DO UPDATE SET
                value_enc = excluded.value_enc, is_secret = excluded.is_secret, updated_at = excluded.updated_at",
        )
        .bind(new_id())
        .bind(&auth.org_id)
        .bind(id)
        .bind(key)
        .bind(&value_enc)
        .bind(is_secret)
        .bind(&at)
        .execute(&mut *tx)
        .await?;
        saved.push(EnvVar {
            key: key.to_string(),
            value: (!is_secret).then(|| plain_value.to_string()),
            is_secret,
            updated_at: at.clone(),
        });
    }
    tx.commit().await?;

    let detail = format!("{} variable(s)", saved.len());
    auth.audit(state, "env.set_bulk", Some(id), Some(&detail))
        .await?;
    Ok(saved)
}

/// Create or replace one variable, or atomically update a batch (`{"vars": [...]}`).
/// Takes effect on the next deployment.
#[utoipa::path(put, path = "/api/v1/apps/{id}/env", tag = "env", params(("id" = String, Path)),
    request_body = SetEnvVarRequest,
    responses((status = 200, body = EnvVar), (status = 404, body = ErrorBody), (status = 422, body = ErrorBody)))]
async fn set_env(
    State(state): State<AppState>,
    auth: Auth<Developer>,
    Path(id): Path<String>,
    Json(raw): Json<serde_json::Value>,
) -> ApiResult<Response> {
    if raw.get("vars").is_some() {
        let req: BulkSetEnvRequest = serde_json::from_value(raw)
            .map_err(|e| ApiError::validation(format!("invalid bulk env payload: {e}")))?;
        let saved = apply_bulk_env(&state, &auth, &id, req).await?;
        return Ok(Json(saved).into_response());
    }
    let req: SetEnvVarRequest = serde_json::from_value(raw)
        .map_err(|e| ApiError::validation(format!("invalid env payload: {e}")))?;
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
    })
    .into_response())
}

/// Atomically create or replace multiple variables in one transaction.
#[utoipa::path(put, path = "/api/v1/apps/{id}/env/bulk", tag = "env", params(("id" = String, Path)),
    request_body = BulkSetEnvRequest,
    responses((status = 200, body = Vec<EnvVar>), (status = 404, body = ErrorBody), (status = 422, body = ErrorBody)))]
async fn set_env_bulk(
    State(state): State<AppState>,
    auth: Auth<Developer>,
    Path(id): Path<String>,
    Json(req): Json<BulkSetEnvRequest>,
) -> ApiResult<Json<Vec<EnvVar>>> {
    Ok(Json(apply_bulk_env(&state, &auth, &id, req).await?))
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
            let raw_tls: &str = r.get("tls_mode");
            let raw_access: &str = r.get("access");
            Ok(Domain {
                id: r.get("id"),
                host: r.get("host"),
                tls_mode: stored_enum(TlsMode::parse(raw_tls), "tls_mode", raw_tls)?,
                access: stored_enum(Access::parse(raw_access), "access", raw_access)?,
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

    #[test]
    fn parses_since_values() {
        assert_eq!(parse_since("1700000000").unwrap(), 1_700_000_000);
        assert!(parse_since("2026-11-22T10:00:00Z").unwrap() > 1_700_000_000);
        assert!(parse_since("10m").unwrap() > 0);
        assert!(parse_since("2h").unwrap() > 0);
        assert!(parse_since("1d").unwrap() > 0);
        assert!(parse_since("0m").is_err());
        assert!(parse_since("bogus").is_err());
    }

    #[tokio::test]
    async fn phase2_enablers_end_to_end() {
        use std::sync::Arc;

        struct StubRuntime;

        #[async_trait::async_trait]
        impl crate::runtime::Runtime for StubRuntime {
            async fn info(&self) -> anyhow::Result<crate::runtime::EngineInfo> {
                Ok(crate::runtime::EngineInfo {
                    name: "stub".into(),
                    version: "1.0".into(),
                })
            }
            async fn pull(&self, _: &str, _: &crate::runtime::LogSink) -> anyhow::Result<()> {
                Ok(())
            }
            async fn build(
                &self,
                _: crate::runtime::BuildSpec,
                _: &crate::runtime::LogSink,
            ) -> anyhow::Result<()> {
                Ok(())
            }
            async fn build_plan(
                &self,
                _: crate::runtime::PlanSpec,
                _: &crate::runtime::LogSink,
            ) -> anyhow::Result<()> {
                Ok(())
            }
            async fn run(
                &self,
                _: &crate::runtime::ContainerSpec,
            ) -> anyhow::Result<crate::runtime::Started> {
                anyhow::bail!("unused")
            }
            async fn state(&self, _: &str) -> anyhow::Result<crate::runtime::ContainerState> {
                Ok(crate::runtime::ContainerState::Running)
            }
            async fn health(&self, _: &str) -> anyhow::Result<Option<crate::runtime::Health>> {
                Ok(None)
            }
            async fn start(&self, _: &str) -> anyhow::Result<()> {
                Ok(())
            }
            async fn stop(&self, _: &str, _: u32) -> anyhow::Result<()> {
                Ok(())
            }
            async fn remove(&self, _: &str) -> anyhow::Result<()> {
                Ok(())
            }
            fn logs_with(
                &self,
                _: &str,
                opts: crate::runtime::LogOptions,
            ) -> crate::runtime::LogStream {
                let items = vec![
                    Ok(crate::runtime::LogMsg {
                        stream: "stdout",
                        line: "boot ok".into(),
                        ts: opts
                            .timestamps
                            .then(|| "2026-11-22T10:00:00Z".into()),
                    }),
                    Ok(crate::runtime::LogMsg {
                        stream: "stderr",
                        line: "ERROR: disk slow".into(),
                        ts: opts
                            .timestamps
                            .then(|| "2026-11-22T10:00:01Z".into()),
                    }),
                ];
                Box::pin(futures_util::stream::iter(items))
            }
            async fn list_by_label(&self, _: &str, _: &str) -> anyhow::Result<Vec<String>> {
                Ok(vec![])
            }
            async fn remove_volume(&self, _: &str) -> anyhow::Result<()> {
                Ok(())
            }
            async fn remove_network(&self, _: &str) -> anyhow::Result<()> {
                Ok(())
            }
            async fn shell(&self, _: &str, _: u16, _: u16) -> anyhow::Result<crate::runtime::Shell> {
                anyhow::bail!("unused")
            }
            async fn image_id(&self, _: &str) -> anyhow::Result<Option<String>> {
                Ok(None)
            }
            async fn usage(&self, _: &str) -> anyhow::Result<Option<crate::runtime::Usage>> {
                Ok(None)
            }
            async fn resize_shell(&self, _: &str, _: u16, _: u16) -> anyhow::Result<()> {
                Ok(())
            }
            async fn exec(
                &self,
                _: &str,
                _: Vec<String>,
                _: Vec<String>,
                _: Option<crate::runtime::ByteReader>,
                _: crate::runtime::ByteWriter,
            ) -> anyhow::Result<crate::runtime::ExecOutcome> {
                anyhow::bail!("unused")
            }
        }

        struct StubProxy;

        #[async_trait::async_trait]
        impl crate::proxy::Proxy for StubProxy {
            async fn ready(&self) -> bool {
                true
            }
            async fn apply(
                &self,
                _: &[crate::proxy::Route],
                _: Option<&rig_api::DashboardDomain>,
                _: &[String],
            ) -> anyhow::Result<()> {
                Ok(())
            }
            fn ports(&self) -> (u16, u16) {
                (80, 443)
            }
        }

        let dir = tempfile::tempdir().unwrap();
        let db = crate::state::open_db(&format!("sqlite://{}", dir.path().join("rig.db").display()))
            .await
            .unwrap();
        let vault = crate::crypto::Vault::open(dir.path()).unwrap();
        let state = AppState {
            db: db.clone(),
            vault,
            config: Arc::new(crate::state::Config {
                data_dir: dir.path().to_path_buf(),
                secure_cookies: false,
                github_api: "http://127.0.0.1".into(),
                github_web: "http://127.0.0.1".into(),
                poll_interval: 0,
                allow_private_alerts: false,
                allow_private_git: false,
                enable_api_docs: false,
                mode: rig_api::ServerMode::Full,
            }),
            runtime: Arc::new(StubRuntime),
            proxy: Arc::new(StubProxy),
            deploys: Arc::new(crate::deploy::Deployer::new().unwrap()),
            access: crate::access::DashboardAccess::default(),
            caches: Arc::new(crate::state::RuntimeCaches::default()),
            events: Arc::new(crate::events::EventBus::new()),
        };

        // Seed org, user, token, two projects, and apps.
        let token = "rig_test_enablers_secret";
        sqlx::query("INSERT INTO organizations (id, name, slug) VALUES ('org1', 'Acme', 'acme')")
            .execute(&db)
            .await
            .unwrap();
        sqlx::query("INSERT INTO users (id, email, name, password_hash) VALUES ('u1', 'a@b.c', 'A', 'h')")
            .execute(&db)
            .await
            .unwrap();
        sqlx::query("INSERT INTO memberships (org_id, user_id, role) VALUES ('org1', 'u1', 'owner')")
            .execute(&db)
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO api_tokens (id, org_id, user_id, name, token_hash, prefix, role, created_at)
             VALUES ('t1', 'org1', 'u1', 'ci', ?, 'rig_te', 'owner', '2026-11-22T00:00:00Z')",
        )
        .bind(crate::crypto::hash_token(token))
        .execute(&db)
        .await
        .unwrap();
        sqlx::query("INSERT INTO projects (id, org_id, name, slug, description, created_at) VALUES ('p1', 'org1', 'Alpha', 'alpha', '', '2026-11-22T00:00:00Z')")
            .execute(&db)
            .await
            .unwrap();
        sqlx::query("INSERT INTO projects (id, org_id, name, slug, description, created_at) VALUES ('p2', 'org1', 'Beta', 'beta', '', '2026-11-22T00:00:00Z')")
            .execute(&db)
            .await
            .unwrap();
        for (id, pid, name, status) in [
            ("a1", "p1", "web", "stopped"),
            ("a2", "p2", "web", "running"),
            ("a3", "p1", "api", "running"),
        ] {
            sqlx::query(
                "INSERT INTO apps (id, org_id, project_id, name, slug, source_kind, image, dockerfile_path, port, status, webhook_secret, created_at, updated_at)
                 VALUES (?, 'org1', ?, ?, ?, 'image', 'nginx:alpine', 'Dockerfile', 80, ?, 'sec', '2026-11-22T00:00:00Z', '2026-11-22T00:00:00Z')",
            )
            .bind(id)
            .bind(pid)
            .bind(name)
            .bind(name)
            .bind(status)
            .execute(&db)
            .await
            .unwrap();
        }
        sqlx::query(
            "INSERT INTO domains (id, org_id, app_id, host, tls_mode, access, created_at)
             VALUES ('dom1', 'org1', 'a3', 'api.example.com', 'auto', 'public', '2026-11-22T00:00:00Z')",
        )
        .execute(&db)
        .await
        .unwrap();
        // Seed deployments for pagination & SSE end & start/restart/cancel tests.
        for (did, app_id, status, img, cid, ts) in [
            ("d1", "a3", "superseded", Some("nginx:1"), None, "2026-11-22T00:01:00.000Z"),
            ("d2", "a3", "superseded", Some("nginx:2"), None, "2026-11-22T00:02:00.000Z"),
            ("d3", "a3", "running", Some("nginx:3"), Some("cid-3"), "2026-11-22T00:03:00.000Z"),
            ("d4", "a1", "superseded", Some("nginx:old"), None, "2026-11-22T00:01:00.000Z"),
        ] {
            sqlx::query(
                "INSERT INTO deployments (id, org_id, app_id, status, trigger, image_ref, container_id, created_at)
                 VALUES (?, 'org1', ?, ?, 'manual', ?, ?, ?)",
            )
            .bind(did)
            .bind(app_id)
            .bind(status)
            .bind(img)
            .bind(cid)
            .bind(ts)
            .execute(&db)
            .await
            .unwrap();
        }
        sqlx::query("UPDATE apps SET current_deployment_id = 'd3' WHERE id = 'a3'")
            .execute(&db)
            .await
            .unwrap();

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server_handle = tokio::spawn(async move {
            axum::serve(listener, crate::routes::app(state))
                .await
                .unwrap();
        });
        let base = format!("http://{addr}");
        let http = reqwest::Client::new();

        // 1. GET /api/v1/apps/resolve?ref= + x-rig-version header
        let res = http
            .get(format!("{base}/api/v1/apps/resolve?ref=api"))
            .bearer_auth(token)
            .send()
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        assert_eq!(
            res.headers()
                .get("x-rig-version")
                .and_then(|v| v.to_str().ok()),
            Some(env!("CARGO_PKG_VERSION"))
        );
        let resolved: App = res.json().await.unwrap();
        assert_eq!(resolved.id, "a3");

        // Ambiguous ref returns 409.
        let res = http
            .get(format!("{base}/api/v1/apps/resolve?ref=web"))
            .bearer_auth(token)
            .send()
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::CONFLICT);

        // Qualified project/app disambiguates cleanly.
        let res = http
            .get(format!("{base}/api/v1/apps/resolve?ref=beta%2Fweb"))
            .bearer_auth(token)
            .send()
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let resolved: App = res.json().await.unwrap();
        assert_eq!(resolved.id, "a2");

        // 2. Deployment pagination (?limit=&before=)
        let page1: Vec<Deployment> = http
            .get(format!("{base}/api/v1/apps/a3/deployments?limit=2"))
            .bearer_auth(token)
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(page1.len(), 2);
        assert_eq!(page1[0].id, "d3");
        assert_eq!(page1[1].id, "d2");

        let page2: Vec<Deployment> = http
            .get(format!("{base}/api/v1/apps/a3/deployments?limit=2&before=d2"))
            .bearer_auth(token)
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(page2.len(), 1);
        assert_eq!(page2[0].id, "d1");

        // 3. Deployment logs SSE `end` event includes domains
        let body_text = http
            .get(format!("{base}/api/v1/deployments/d3/logs"))
            .bearer_auth(token)
            .send()
            .await
            .unwrap()
            .text()
            .await
            .unwrap();
        assert!(body_text.contains("event: end"), "{body_text}");
        assert!(body_text.contains("api.example.com"), "{body_text}");

        // 4. App logs query params (follow=false, timestamps=true, grep=error)
        let log_body = http
            .get(format!(
                "{base}/api/v1/apps/a3/logs?tail=50&follow=false&timestamps=true&grep=error"
            ))
            .bearer_auth(token)
            .send()
            .await
            .unwrap()
            .text()
            .await
            .unwrap();
        assert!(log_body.contains("ERROR: disk slow"), "{log_body}");
        assert!(!log_body.contains("boot ok"), "{log_body}");
        assert!(log_body.contains("2026-11-22T10:00:01Z"), "{log_body}");

        // 5. Atomic bulk env PUT /api/v1/apps/{id}/env (and rollback on invalid key)
        let bad_bulk = serde_json::json!({
            "vars": [
                { "key": "GOOD_KEY", "value": "1", "is_secret": false },
                { "key": "1BAD", "value": "2", "is_secret": false }
            ]
        });
        let res = http
            .put(format!("{base}/api/v1/apps/a3/env"))
            .bearer_auth(token)
            .json(&bad_bulk)
            .send()
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY);

        // Verify atomicity: GOOD_KEY was NOT persisted.
        let vars: Vec<EnvVar> = http
            .get(format!("{base}/api/v1/apps/a3/env"))
            .bearer_auth(token)
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert!(vars.is_empty());

        // Valid bulk set
        let good_bulk = serde_json::json!({
            "vars": [
                { "key": "A", "value": "1", "is_secret": false },
                { "key": "B", "value": "secret", "is_secret": true }
            ]
        });
        let res = http
            .put(format!("{base}/api/v1/apps/a3/env"))
            .bearer_auth(token)
            .json(&good_bulk)
            .send()
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let saved: Vec<EnvVar> = res.json().await.unwrap();
        assert_eq!(saved.len(), 2);

        // Replace mode drops A and B when only C is provided.
        let replace_bulk = serde_json::json!({
            "vars": [{ "key": "C", "value": "3", "is_secret": false }],
            "replace": true
        });
        let res = http
            .put(format!("{base}/api/v1/apps/a3/env/bulk"))
            .bearer_auth(token)
            .json(&replace_bulk)
            .send()
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let vars: Vec<EnvVar> = http
            .get(format!("{base}/api/v1/apps/a3/env"))
            .bearer_auth(token)
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(vars.len(), 1);
        assert_eq!(vars[0].key, "C");

        // 6. Start stopped app a1 -> 202 queued with previous image, then cancel it -> 200 cancelled
        let res = http
            .post(format!("{base}/api/v1/apps/a1/start"))
            .bearer_auth(token)
            .send()
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::ACCEPTED);
        let started_dep: Deployment = res.json().await.unwrap();
        assert_eq!(started_dep.trigger, "api");
        assert_eq!(started_dep.image_ref.as_deref(), Some("nginx:old"));

        let res = http
            .post(format!(
                "{base}/api/v1/deployments/{}/cancel",
                started_dep.id
            ))
            .bearer_auth(token)
            .send()
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let cancelled_dep: Deployment = res.json().await.unwrap();
        assert_eq!(cancelled_dep.status, rig_api::DeploymentStatus::Cancelled);

        // 7. Restart running app a3 -> 202 queued reusing current image
        let res = http
            .post(format!("{base}/api/v1/apps/a3/restart"))
            .bearer_auth(token)
            .send()
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::ACCEPTED);
        let restarted_dep: Deployment = res.json().await.unwrap();
        assert_eq!(restarted_dep.trigger, "api");
        assert_eq!(restarted_dep.image_ref.as_deref(), Some("nginx:3"));

        server_handle.abort();
    }

    #[tokio::test]
    async fn security_audit_regressions_end_to_end() {
        use std::net::SocketAddr;
        use std::sync::Arc;

        struct StubRuntime;

        #[async_trait::async_trait]
        impl crate::runtime::Runtime for StubRuntime {
            async fn info(&self) -> anyhow::Result<crate::runtime::EngineInfo> {
                Ok(crate::runtime::EngineInfo {
                    name: "stub".into(),
                    version: "1.0".into(),
                })
            }
            async fn pull(&self, _: &str, _: &crate::runtime::LogSink) -> anyhow::Result<()> {
                Ok(())
            }
            async fn build(
                &self,
                _: crate::runtime::BuildSpec,
                _: &crate::runtime::LogSink,
            ) -> anyhow::Result<()> {
                Ok(())
            }
            async fn build_plan(
                &self,
                _: crate::runtime::PlanSpec,
                _: &crate::runtime::LogSink,
            ) -> anyhow::Result<()> {
                Ok(())
            }
            async fn run(
                &self,
                _: &crate::runtime::ContainerSpec,
            ) -> anyhow::Result<crate::runtime::Started> {
                anyhow::bail!("unused")
            }
            async fn state(&self, _: &str) -> anyhow::Result<crate::runtime::ContainerState> {
                Ok(crate::runtime::ContainerState::Running)
            }
            async fn health(&self, _: &str) -> anyhow::Result<Option<crate::runtime::Health>> {
                Ok(None)
            }
            async fn start(&self, _: &str) -> anyhow::Result<()> {
                Ok(())
            }
            async fn stop(&self, _: &str, _: u32) -> anyhow::Result<()> {
                Ok(())
            }
            async fn remove(&self, _: &str) -> anyhow::Result<()> {
                Ok(())
            }
            fn logs_with(
                &self,
                _: &str,
                _: crate::runtime::LogOptions,
            ) -> crate::runtime::LogStream {
                Box::pin(futures_util::stream::empty())
            }
            async fn list_by_label(&self, _: &str, _: &str) -> anyhow::Result<Vec<String>> {
                Ok(vec![])
            }
            async fn remove_volume(&self, _: &str) -> anyhow::Result<()> {
                panic!("delete_project must NOT remove volumes before checking creating databases");
            }
            async fn remove_network(&self, _: &str) -> anyhow::Result<()> {
                Ok(())
            }
            async fn shell(
                &self,
                _: &str,
                _: u16,
                _: u16,
            ) -> anyhow::Result<crate::runtime::Shell> {
                anyhow::bail!("unused")
            }
            async fn image_id(&self, _: &str) -> anyhow::Result<Option<String>> {
                Ok(None)
            }
            async fn usage(&self, _: &str) -> anyhow::Result<Option<crate::runtime::Usage>> {
                Ok(None)
            }
            async fn resize_shell(&self, _: &str, _: u16, _: u16) -> anyhow::Result<()> {
                Ok(())
            }
            async fn exec(
                &self,
                _: &str,
                _: Vec<String>,
                _: Vec<String>,
                _: Option<crate::runtime::ByteReader>,
                _: crate::runtime::ByteWriter,
            ) -> anyhow::Result<crate::runtime::ExecOutcome> {
                anyhow::bail!("unused")
            }
        }

        struct StubProxy;

        #[async_trait::async_trait]
        impl crate::proxy::Proxy for StubProxy {
            async fn ready(&self) -> bool {
                true
            }
            async fn apply(
                &self,
                _: &[crate::proxy::Route],
                _: Option<&rig_api::DashboardDomain>,
                _: &[String],
            ) -> anyhow::Result<()> {
                Ok(())
            }
            fn ports(&self) -> (u16, u16) {
                (80, 443)
            }
        }

        let dir = tempfile::tempdir().unwrap();
        let db = crate::state::open_db(&format!("sqlite://{}", dir.path().join("rig.db").display()))
            .await
            .unwrap();
        let vault = crate::crypto::Vault::open(dir.path()).unwrap();
        let state = AppState {
            db: db.clone(),
            vault: vault.clone(),
            config: Arc::new(crate::state::Config {
                data_dir: dir.path().to_path_buf(),
                secure_cookies: false,
                github_api: "http://127.0.0.1".into(),
                github_web: "http://127.0.0.1".into(),
                poll_interval: 0,
                allow_private_alerts: false,
                allow_private_git: false,
                enable_api_docs: false,
                mode: rig_api::ServerMode::Full,
            }),
            runtime: Arc::new(StubRuntime),
            proxy: Arc::new(StubProxy),
            deploys: Arc::new(crate::deploy::Deployer::new().unwrap()),
            access: crate::access::DashboardAccess::default(),
            caches: Arc::new(crate::state::RuntimeCaches::default()),
            events: Arc::new(crate::events::EventBus::new()),
        };

        let owner_pw = "owner-super-secret-pw";
        let owner_hash = crate::crypto::hash_password(owner_pw).unwrap();
        let api_token = "rig_sec_test_owner_token";
        let other_token = "rig_sec_test_other_token";

        sqlx::query("INSERT INTO organizations (id, name, slug) VALUES ('org1', 'Acme', 'acme')")
            .execute(&db)
            .await
            .unwrap();
        sqlx::query("INSERT INTO settings (key, value) VALUES ('setup_complete', '1')")
            .execute(&db)
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO users (id, email, name, password_hash) VALUES ('u1', 'owner@example.com', 'Owner', ?)",
        )
        .bind(&owner_hash)
        .execute(&db)
        .await
        .unwrap();
        sqlx::query("INSERT INTO memberships (org_id, user_id, role) VALUES ('org1', 'u1', 'owner')")
            .execute(&db)
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO api_tokens (id, org_id, user_id, name, token_hash, prefix, role, created_at)
             VALUES ('tok1', 'org1', 'u1', 'cli', ?, 'rig_se', 'owner', '2026-11-22T00:00:00Z'),
                    ('tok2', 'org1', 'u1', 'other', ?, 'rig_ot', 'owner', '2026-11-22T00:00:00Z')",
        )
        .bind(crate::crypto::hash_token(api_token))
        .bind(crate::crypto::hash_token(other_token))
        .execute(&db)
        .await
        .unwrap();
        // Private media item (is_public = 0)
        sqlx::query(
            "INSERT INTO media (id, org_id, content_type, data, is_public)
             VALUES ('priv1', 'org1', 'image/png', X'89504E470D0A1A0A', 0)",
        )
        .execute(&db)
        .await
        .unwrap();
        // Project with an app + a creating database (for RIG-SEC-09d test)
        sqlx::query("INSERT INTO projects (id, org_id, name, slug, description, created_at) VALUES ('p1', 'org1', 'P1', 'p1', '', '2026-11-22T00:00:00Z')")
            .execute(&db)
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO apps (id, org_id, project_id, name, slug, source_kind, image, dockerfile_path, port, status, webhook_secret, created_at, updated_at)
             VALUES ('a1', 'org1', 'p1', 'app1', 'app1', 'image', 'nginx:alpine', 'Dockerfile', 80, 'stopped', ?, '2026-11-22T00:00:00Z', '2026-11-22T00:00:00Z')",
        )
        .bind(vault.encrypt("whsec").unwrap())
        .execute(&db)
        .await
        .unwrap();
        sqlx::query("INSERT INTO app_volumes (id, app_id, name, mount_path, created_at) VALUES ('v1', 'a1', 'rig-app-v1', '/data', '2026-11-22T00:00:00Z')")
            .execute(&db)
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO databases (id, org_id, project_id, name, slug, engine, version, volume, db_name, username, password_enc, status, created_at)
             VALUES ('db1', 'org1', 'p1', 'pg', 'pg', 'postgres', '17', 'rig-db-1', 'pg', 'pg', ?, 'creating', '2026-11-22T00:00:00Z')",
        )
        .bind(vault.encrypt("dbpw").unwrap())
        .execute(&db)
        .await
        .unwrap();

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server_handle = tokio::spawn(async move {
            axum::serve(
                listener,
                crate::routes::app(state).into_make_service_with_connect_info::<SocketAddr>(),
            )
            .await
            .unwrap();
        });
        let base = format!("http://{addr}");
        let http = reqwest::Client::new();

        // RIG-SEC-06: /api/openapi.json requires auth; /api/docs disabled by default.
        assert_eq!(
            http.get(format!("{base}/api/openapi.json")).send().await.unwrap().status(),
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            http.get(format!("{base}/api/openapi.json"))
                .bearer_auth(api_token)
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::OK
        );
        assert_eq!(
            http.get(format!("{base}/api/docs"))
                .bearer_auth(api_token)
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::NOT_FOUND
        );

        // RIG-SEC-17.4 & RIG-SEC-15: /api/v1/setup omits version; HSTS sent when X-Forwarded-Proto: https from loopback.
        let setup_res = http
            .get(format!("{base}/api/v1/setup"))
            .header("x-forwarded-proto", "https")
            .send()
            .await
            .unwrap();
        assert_eq!(setup_res.status(), StatusCode::OK);
        assert!(setup_res.headers().contains_key("strict-transport-security"));
        let setup_body: rig_api::SetupStatus = setup_res.json().await.unwrap();
        assert!(setup_body.version.is_none());

        // RIG-SEC-15: Private media returns 404 (not 401) unauthenticated, and Cache-Control: private authenticated.
        assert_eq!(
            http.get(format!("{base}/api/v1/media/priv1")).send().await.unwrap().status(),
            StatusCode::NOT_FOUND
        );
        let priv_res = http
            .get(format!("{base}/api/v1/media/priv1"))
            .bearer_auth(api_token)
            .send()
            .await
            .unwrap();
        assert_eq!(priv_res.status(), StatusCode::OK);
        let cc = priv_res
            .headers()
            .get("cache-control")
            .unwrap()
            .to_str()
            .unwrap();
        assert!(cc.starts_with("private,"), "expected private cache-control, got {cc}");

        // RIG-SEC-01 & 05: Alert SSRF blocked; valid channel encrypts signing_secret at rest.
        assert_eq!(
            http.post(format!("{base}/api/v1/alerts/channels"))
                .bearer_auth(api_token)
                .json(&serde_json::json!({
                    "name": "ssrf",
                    "url": "http://127.0.0.1:2019/stop",
                    "events": ["deploy_failed"]
                }))
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::UNPROCESSABLE_ENTITY
        );
        let ch_res = http
            .post(format!("{base}/api/v1/alerts/channels"))
            .bearer_auth(api_token)
            .json(&serde_json::json!({
                "name": "ok-hook",
                "url": "https://hooks.slack.com/services/T/B/X",
                "events": ["deploy_failed"]
            }))
            .send()
            .await
            .unwrap();
        assert_eq!(ch_res.status(), StatusCode::CREATED);
        let created_ch: rig_api::CreatedAlertChannel = ch_res.json().await.unwrap();
        let stored_sig: String =
            sqlx::query_scalar("SELECT signing_secret FROM alert_channels WHERE id = ?")
                .bind(&created_ch.channel.id)
                .fetch_one(&db)
                .await
                .unwrap();
        assert_ne!(stored_sig, created_ch.signing_secret);
        assert_eq!(vault.decrypt(&stored_sig).unwrap(), created_ch.signing_secret);

        // RIG-SEC-03: API token cannot mint new tokens, revoke OTHER tokens, manage members, or change dashboard access/domain.
        assert_eq!(
            http.post(format!("{base}/api/v1/tokens"))
                .bearer_auth(api_token)
                .json(&serde_json::json!({ "name": "backdoor", "role": "owner", "expires_in_days": null }))
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            http.delete(format!("{base}/api/v1/tokens/tok2"))
                .bearer_auth(api_token)
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            http.post(format!("{base}/api/v1/members"))
                .bearer_auth(api_token)
                .json(&serde_json::json!({ "name": "Evil", "email": "evil@example.com", "role": "owner" }))
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            http.put(format!("{base}/api/v1/system/dashboard-access"))
                .bearer_auth(api_token)
                .json(&serde_json::json!({ "access": "public" }))
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            http.put(format!("{base}/api/v1/system/dashboard-domain"))
                .bearer_auth(api_token)
                .json(&serde_json::json!({ "host": null }))
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::FORBIDDEN
        );

        // RIG-SEC-09d: delete_project rejects creating DB BEFORE touching app volumes (StubRuntime::remove_volume would panic!).
        assert_eq!(
            http.delete(format!("{base}/api/v1/projects/p1?databases=delete"))
                .bearer_auth(api_token)
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::CONFLICT
        );

        // RIG-SEC-13: 8 bad logins from IP A lock (email, IP A) with 429, while IP B can still log in!
        for _ in 0..8 {
            let r = http
                .post(format!("{base}/api/v1/auth/login"))
                .header("x-forwarded-for", "198.51.100.10")
                .json(&serde_json::json!({ "email": "owner@example.com", "password": "wrong-password-1" }))
                .send()
                .await
                .unwrap();
            assert_eq!(r.status(), StatusCode::UNAUTHORIZED);
        }
        assert_eq!(
            http.post(format!("{base}/api/v1/auth/login"))
                .header("x-forwarded-for", "198.51.100.10")
                .json(&serde_json::json!({ "email": "owner@example.com", "password": owner_pw }))
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::TOO_MANY_REQUESTS
        );
        let login_res = http
            .post(format!("{base}/api/v1/auth/login"))
            .header("x-forwarded-for", "203.0.113.55")
            .json(&serde_json::json!({ "email": "owner@example.com", "password": owner_pw }))
            .send()
            .await
            .unwrap();
        assert_eq!(login_res.status(), StatusCode::OK);
        let session_cookie = login_res
            .headers()
            .get("set-cookie")
            .unwrap()
            .to_str()
            .unwrap()
            .split(';')
            .next()
            .unwrap()
            .to_string();
        let me: rig_api::Me = login_res.json().await.unwrap();
        assert!(!me.must_change_password);

        // RIG-SEC-04: verify_password on /api/v1/account/totp/setup rate-limits after 8 failures.
        for _ in 0..8 {
            let r = http
                .post(format!("{base}/api/v1/account/totp/setup"))
                .header("cookie", &session_cookie)
                .header("x-rig-csrf", &me.csrf_token)
                .json(&serde_json::json!({ "password": "wrong-password" }))
                .send()
                .await
                .unwrap();
            assert_eq!(r.status(), StatusCode::UNPROCESSABLE_ENTITY);
        }
        assert_eq!(
            http.post(format!("{base}/api/v1/account/totp/setup"))
                .header("cookie", &session_cookie)
                .header("x-rig-csrf", &me.csrf_token)
                .json(&serde_json::json!({ "password": owner_pw }))
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::TOO_MANY_REQUESTS
        );

        // RIG-SEC-09b & 09c: add_member issues a temporary password that forces password change & rotates session/CSRF.
        let add_res = http
            .post(format!("{base}/api/v1/members"))
            .header("cookie", &session_cookie)
            .header("x-rig-csrf", &me.csrf_token)
            .json(&serde_json::json!({
                "name": "Dev",
                "email": "dev@example.com",
                "role": "developer"
            }))
            .send()
            .await
            .unwrap();
        assert_eq!(add_res.status(), StatusCode::OK);
        let added: rig_api::MemberWithPassword = add_res.json().await.unwrap();

        let dev_login = http
            .post(format!("{base}/api/v1/auth/login"))
            .header("x-forwarded-for", "203.0.113.56")
            .json(&serde_json::json!({
                "email": "dev@example.com",
                "password": added.temporary_password
            }))
            .send()
            .await
            .unwrap();
        assert_eq!(dev_login.status(), StatusCode::OK);
        let dev_cookie = dev_login
            .headers()
            .get("set-cookie")
            .unwrap()
            .to_str()
            .unwrap()
            .split(';')
            .next()
            .unwrap()
            .to_string();
        let dev_me: rig_api::Me = dev_login.json().await.unwrap();
        assert!(dev_me.must_change_password);
        // Blocked from regular APIs until password is changed.
        assert_eq!(
            http.get(format!("{base}/api/v1/projects"))
                .header("cookie", &dev_cookie)
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::FORBIDDEN
        );
        // Changing password succeeds, rotates cookie & returns rotated x-rig-csrf header!
        let pw_change = http
            .post(format!("{base}/api/v1/auth/password"))
            .header("cookie", &dev_cookie)
            .header("x-rig-csrf", &dev_me.csrf_token)
            .json(&serde_json::json!({
                "current_password": added.temporary_password,
                "new_password": "new-permanent-password-123"
            }))
            .send()
            .await
            .unwrap();
        assert_eq!(pw_change.status(), StatusCode::NO_CONTENT);
        let new_dev_cookie = pw_change
            .headers()
            .get("set-cookie")
            .unwrap()
            .to_str()
            .unwrap()
            .split(';')
            .next()
            .unwrap()
            .to_string();
        let new_csrf = pw_change
            .headers()
            .get("x-rig-csrf")
            .unwrap()
            .to_str()
            .unwrap()
            .to_string();
        assert_ne!(new_dev_cookie, dev_cookie);
        assert_ne!(new_csrf, dev_me.csrf_token);
        // Old session cookie is now invalidated.
        assert_eq!(
            http.get(format!("{base}/api/v1/projects"))
                .header("cookie", &dev_cookie)
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::UNAUTHORIZED
        );
        // New rotated session works immediately.
        assert_eq!(
            http.get(format!("{base}/api/v1/projects"))
                .header("cookie", &new_dev_cookie)
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::OK
        );

        // RIG-SEC-09a & 05: 2FA challenge token login + Vault-keyed v2 recovery code.
        let secret = crate::totp::new_secret();
        sqlx::query("UPDATE users SET totp_secret = ?, totp_enabled_at = '2026-11-22T00:00:00Z' WHERE id = ?")
            .bind(vault.encrypt(&secret).unwrap())
            .bind(&added.member.user_id)
            .execute(&db)
            .await
            .unwrap();
        let step1 = http
            .post(format!("{base}/api/v1/auth/login"))
            .header("x-forwarded-for", "203.0.113.57")
            .json(&serde_json::json!({
                "email": "dev@example.com",
                "password": "new-permanent-password-123"
            }))
            .send()
            .await
            .unwrap();
        assert_eq!(step1.status(), StatusCode::UNAUTHORIZED);
        let step1_err: ErrorBody = step1.json().await.unwrap();
        assert_eq!(step1_err.error.code, "second_factor_required");
        let ch_tok = step1_err
            .error
            .challenge_token
            .expect("step 1 must return a server-side challenge_token");
        // Audit log recorded step-1 password verification.
        let chal_audit: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM audit_log WHERE user_id = ? AND action = 'auth.login_challenge'",
        )
        .bind(&added.member.user_id)
        .fetch_one(&db)
        .await
        .unwrap();
        assert_eq!(chal_audit, 1);

        // Step 2 completes without re-sending password.
        let raw_key = crate::totp::base32_decode(&secret).unwrap();
        let totp_code = format!(
            "{:06}",
            crate::totp::code_at(&raw_key, crate::totp::step_of(chrono::Utc::now().timestamp()))
        );
        let step2 = http
            .post(format!("{base}/api/v1/auth/login"))
            .header("x-forwarded-for", "203.0.113.57")
            .json(&serde_json::json!({
                "challenge_token": ch_tok,
                "code": totp_code
            }))
            .send()
            .await
            .unwrap();
        assert_eq!(step2.status(), StatusCode::OK);
        // Replaying the single-use challenge token fails.
        assert_eq!(
            http.post(format!("{base}/api/v1/auth/login"))
                .header("x-forwarded-for", "203.0.113.57")
                .json(&serde_json::json!({
                    "challenge_token": ch_tok,
                    "code": totp_code
                }))
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::UNAUTHORIZED
        );

        // Performance fixes verification: deployment_logs ?tail=N and media ETag/304.
        sqlx::query(
            "INSERT INTO projects (id, org_id, name, slug, description, created_at)
             VALUES ('perf-p1', 'org1', 'Perf', 'perf', '', '2026-11-22T00:00:00Z')",
        )
        .execute(&db)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO apps (id, org_id, project_id, name, slug, source_kind, image, dockerfile_path, port, status, webhook_secret, created_at, updated_at)
             VALUES ('perf-a1', 'org1', 'perf-p1', 'PerfApp', 'perfapp', 'image', 'nginx:alpine', 'Dockerfile', 80, 'running', 'sec', '2026-11-22T00:00:00Z', '2026-11-22T00:00:00Z')",
        )
        .execute(&db)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO deployments (id, org_id, app_id, status, trigger, created_at)
             VALUES ('perf-d1', 'org1', 'perf-a1', 'running', 'manual', '2026-11-22T00:00:00Z')",
        )
        .execute(&db)
        .await
        .unwrap();
        for seq in 1..=20i64 {
            sqlx::query(
                "INSERT INTO deployment_logs (deployment_id, seq, ts, stream, line)
                 VALUES ('perf-d1', ?, '2026-11-22T00:00:00Z', 'build', ?)",
            )
            .bind(seq)
            .bind(format!("line-{seq}"))
            .execute(&db)
            .await
            .unwrap();
        }
        let tail_sse = http
            .get(format!("{base}/api/v1/deployments/perf-d1/logs?tail=3"))
            .bearer_auth(api_token)
            .send()
            .await
            .unwrap();
        assert_eq!(tail_sse.status(), StatusCode::OK);
        let tail_body = tail_sse.text().await.unwrap();
        assert!(!tail_body.contains("\"line-17\""));
        assert!(tail_body.contains("\"line-18\""));
        assert!(tail_body.contains("\"line-19\""));
        assert!(tail_body.contains("\"line-20\""));

        sqlx::query(
            "INSERT INTO media (id, org_id, content_type, data, is_public)
             VALUES ('med-etag-1', 'org1', 'image/png', X'89504E470D0A1A0A', 1)",
        )
        .execute(&db)
        .await
        .unwrap();
        let med_res = http
            .get(format!("{base}/api/v1/media/med-etag-1"))
            .send()
            .await
            .unwrap();
        assert_eq!(med_res.status(), StatusCode::OK);
        let med_etag = med_res
            .headers()
            .get("etag")
            .unwrap()
            .to_str()
            .unwrap()
            .to_owned();
        assert_eq!(med_etag, "\"med-etag-1\"");
        let med_304 = http
            .get(format!("{base}/api/v1/media/med-etag-1"))
            .header("if-none-match", &med_etag)
            .send()
            .await
            .unwrap();
        assert_eq!(med_304.status(), StatusCode::NOT_MODIFIED);

        // RIG-SEC-03 self-revoke: API token CAN revoke itself (rig logout --revoke).
        assert_eq!(
            http.delete(format!("{base}/api/v1/tokens/tok1"))
                .bearer_auth(api_token)
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::NO_CONTENT
        );

        server_handle.abort();
    }
}
