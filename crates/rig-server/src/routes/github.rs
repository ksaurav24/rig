//! The organization's GitHub App: create it through GitHub's manifest flow,
//! list the repositories it can read, and receive its push events.

use std::net::SocketAddr;

use axum::{
    Json,
    body::Bytes,
    extract::{ConnectInfo, Path, State},
    http::{HeaderMap, StatusCode, header},
};
use rig_api::{
    ErrorBody, GithubAppInfo, GithubBranch, GithubManifest, GithubManifestComplete,
    GithubManifestRequest, GithubPushResult, GithubRepo, GithubStatus, TlsMode,
};
use sqlx::Row;
use utoipa_axum::{router::OpenApiRouter, routes};

use crate::{
    auth::{self, Admin, Auth, Developer, Viewer},
    crypto::random_token,
    error::{ApiError, ApiResult},
    github, proxy,
    routes::hooks::verify_hmac,
    state::{AppState, now},
};

pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(status, disconnect))
        .routes(routes!(manifest))
        .routes(routes!(complete))
        .routes(routes!(repos))
        .routes(routes!(branches))
        .routes(routes!(push_hook))
}

fn hook_path(org_id: &str) -> String {
    format!("/api/v1/hooks/github/{org_id}")
}

/// The GitHub App connected to this organization, if any.
#[utoipa::path(get, path = "/api/v1/github", tag = "github",
    responses((status = 200, body = GithubStatus)))]
async fn status(
    State(state): State<AppState>,
    auth: Auth<Viewer>,
) -> ApiResult<Json<GithubStatus>> {
    load_status(&state, &auth.org_id).await.map(Json)
}

async fn load_status(state: &AppState, org_id: &str) -> ApiResult<GithubStatus> {
    let app = sqlx::query(
        "SELECT name, slug, html_url, owner, created_at FROM github_apps WHERE org_id = ?",
    )
    .bind(org_id)
    .fetch_optional(&state.db)
    .await?
    .map(|r| {
        let html_url: String = r.get("html_url");
        GithubAppInfo {
            name: r.get("name"),
            slug: r.get("slug"),
            install_url: format!("{html_url}/installations/new"),
            html_url,
            owner: r.get("owner"),
            created_at: r.get("created_at"),
        }
    });
    Ok(GithubStatus {
        app,
        poll_interval: state.config.poll_interval,
        webhook_url: hook_path(org_id),
    })
}

/// Forget the GitHub App. The App itself stays on GitHub until you delete it there.
#[utoipa::path(delete, path = "/api/v1/github", tag = "github",
    responses((status = 204), (status = 404, body = ErrorBody)))]
async fn disconnect(State(state): State<AppState>, auth: Auth<Admin>) -> ApiResult<StatusCode> {
    auth.require_interactive("sign in to the dashboard or Rig Desktop to disconnect the GitHub App")?;
    let done = sqlx::query("DELETE FROM github_apps WHERE org_id = ?")
        .bind(&auth.org_id)
        .execute(&state.db)
        .await?;
    if done.rows_affected() == 0 {
        return Err(ApiError::NotFound("no GitHub App is connected"));
    }
    github::forget_tokens(&auth.org_id);
    auth.audit(&state, "github.disconnected", None, None)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

fn valid_github_login(s: &str) -> bool {
    (1..=39).contains(&s.len())
        && !s.starts_with('-')
        && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
}

/// Validates an optional native-client loopback redirect URL for GitHub's manifest callback.
fn validate_loopback_redirect_url(raw: &str) -> ApiResult<String> {
    let trimmed = raw.trim();
    let parsed = url::Url::parse(trimmed)
        .map_err(|_| ApiError::Validation("redirect_url is not a valid URL".into()))?;
    if parsed.scheme() != "http"
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.port().is_none()
        || !matches!(parsed.host_str(), Some("127.0.0.1" | "localhost"))
    {
        return Err(ApiError::Validation(
            "redirect_url must be a local loopback URL (http://127.0.0.1:<port>/callback)".into(),
        ));
    }
    Ok(trimmed.to_string())
}

async fn resolve_server_origin(
    state: &AppState,
    headers: &HeaderMap,
    peer: std::net::IpAddr,
) -> ApiResult<String> {
    if let Some(origin) = headers
        .get(header::ORIGIN)
        .and_then(|v| v.to_str().ok())
        .filter(|o| o.starts_with("https://") || o.starts_with("http://"))
    {
        return Ok(origin.trim_end_matches('/').to_string());
    }
    if let Ok(Some(domain)) = proxy::dashboard_domain(&state.db).await {
        let scheme = if domain.tls_mode == TlsMode::Off {
            "http"
        } else {
            "https"
        };
        return Ok(format!("{scheme}://{}", domain.host));
    }
    let host = if peer.is_loopback() {
        headers
            .get("x-forwarded-host")
            .or_else(|| headers.get(header::HOST))
    } else {
        headers.get(header::HOST)
    }
    .and_then(|v| v.to_str().ok())
    .map(str::trim)
    .filter(|h| !h.is_empty());
    let Some(host) = host else {
        return Err(ApiError::Validation(
            "could not determine server origin for GitHub App manifest".into(),
        ));
    };
    let scheme = if auth::is_https(state, headers, Some(peer)) {
        "https"
    } else {
        "http"
    };
    Ok(format!("{scheme}://{host}"))
}

/// Start creating a GitHub App. The browser posts `manifest` to `post_url`;
/// GitHub then redirects back to `/settings/github/callback` (or the validated
/// loopback `redirect_url` for Rig Desktop).
#[utoipa::path(post, path = "/api/v1/github/manifest", tag = "github",
    request_body = GithubManifestRequest,
    responses((status = 200, body = GithubManifest), (status = 409, body = ErrorBody),
        (status = 422, body = ErrorBody)))]
async fn manifest(
    State(state): State<AppState>,
    auth: Auth<Admin>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(req): Json<GithubManifestRequest>,
) -> ApiResult<Json<GithubManifest>> {
    auth.require_interactive("sign in to the dashboard or Rig Desktop to connect a GitHub App")?;
    let exists: Option<i64> = sqlx::query_scalar("SELECT 1 FROM github_apps WHERE org_id = ?")
        .bind(&auth.org_id)
        .fetch_optional(&state.db)
        .await?;
    if exists.is_some() {
        return Err(ApiError::Conflict(
            "a GitHub App is already connected; disconnect it first".into(),
        ));
    }
    let origin = resolve_server_origin(&state, &headers, addr.ip()).await?;
    let redirect_url = match req
        .redirect_url
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        Some(custom) => validate_loopback_redirect_url(custom)?,
        None => format!("{origin}/settings/github/callback"),
    };
    let organization = req
        .organization
        .map(|o| o.trim().to_string())
        .filter(|o| !o.is_empty());
    if let Some(o) = &organization
        && !valid_github_login(o)
    {
        return Err(ApiError::Validation(
            "that is not a GitHub organization name".into(),
        ));
    }
    let manifest = serde_json::json!({
        "name": format!("Rig {}", &random_token(4)[..6]),
        "url": origin,
        "hook_attributes": { "url": format!("{origin}{}", hook_path(&auth.org_id)), "active": true },
        "redirect_url": redirect_url,
        "public": false,
        "default_permissions": { "contents": "read", "metadata": "read" },
        "default_events": ["push"],
    });
    let flow = github::issue_state(&auth.org_id, &auth.user_id);
    let web = &state.config.github_web;
    let post_url = match &organization {
        Some(o) => format!("{web}/organizations/{o}/settings/apps/new?state={flow}"),
        None => format!("{web}/settings/apps/new?state={flow}"),
    };
    Ok(Json(GithubManifest {
        post_url,
        manifest: manifest.to_string(),
    }))
}

/// Finish creating the GitHub App with the code GitHub redirected back with.
#[utoipa::path(post, path = "/api/v1/github/manifest/complete", tag = "github",
    request_body = GithubManifestComplete,
    responses((status = 200, body = GithubStatus), (status = 400, body = ErrorBody),
        (status = 409, body = ErrorBody), (status = 503, body = ErrorBody)))]
async fn complete(
    State(state): State<AppState>,
    auth: Auth<Admin>,
    Json(req): Json<GithubManifestComplete>,
) -> ApiResult<Json<GithubStatus>> {
    auth.require_interactive("sign in to the dashboard or Rig Desktop to connect a GitHub App")?;
    if !github::take_state(&req.state, &auth.org_id, &auth.user_id) {
        return Err(ApiError::Validation(
            "this GitHub link expired or was not started by you; start again".into(),
        ));
    }
    let conv = github::convert_manifest(&state, &req.code)
        .await
        .map_err(|e| ApiError::Unavailable(format!("{e:#}")))?;
    let webhook_secret = conv
        .webhook_secret
        .ok_or_else(|| ApiError::Unavailable("GitHub did not return a webhook secret".into()))?;
    let done = sqlx::query(
        "INSERT OR IGNORE INTO github_apps
           (org_id, app_id, slug, name, html_url, owner, client_id, client_secret,
            webhook_secret, private_key, created_by, created_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&auth.org_id)
    .bind(conv.id)
    .bind(&conv.slug)
    .bind(&conv.name)
    .bind(&conv.html_url)
    .bind(&conv.owner.login)
    .bind(&conv.client_id)
    .bind(state.vault.encrypt(&conv.client_secret)?)
    .bind(state.vault.encrypt(&webhook_secret)?)
    .bind(state.vault.encrypt(&conv.pem)?)
    .bind(&auth.user_id)
    .bind(now())
    .execute(&state.db)
    .await?;
    if done.rows_affected() == 0 {
        return Err(ApiError::Conflict(
            "a GitHub App is already connected".into(),
        ));
    }
    auth.audit(&state, "github.connected", Some(&conv.slug), None)
        .await?;
    load_status(&state, &auth.org_id).await.map(Json)
}

/// Repositories the GitHub App is installed on.
#[utoipa::path(get, path = "/api/v1/github/repos", tag = "github",
    responses((status = 200, body = Vec<GithubRepo>), (status = 503, body = ErrorBody)))]
async fn repos(
    State(state): State<AppState>,
    auth: Auth<Developer>,
) -> ApiResult<Json<Vec<GithubRepo>>> {
    github::installed_repos(&state, &auth.org_id)
        .await
        .map(Json)
        .map_err(|e| ApiError::Unavailable(format!("{e:#}")))
}

/// Branches of one repository the GitHub App is installed on, default first.
#[utoipa::path(get, path = "/api/v1/github/repos/{owner}/{repo}/branches", tag = "github",
    params(("owner" = String, Path), ("repo" = String, Path)),
    responses((status = 200, body = Vec<GithubBranch>), (status = 404, body = ErrorBody),
        (status = 503, body = ErrorBody)))]
async fn branches(
    State(state): State<AppState>,
    auth: Auth<Developer>,
    Path((owner, repo)): Path<(String, String)>,
) -> ApiResult<Json<Vec<GithubBranch>>> {
    let ok = |s: &str| {
        !s.is_empty()
            && !s.starts_with('.')
            && s.chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
    };
    if !ok(&owner) || !ok(&repo) {
        return Err(ApiError::validation("not a GitHub repository name"));
    }
    match github::branches(&state, &auth.org_id, &owner, &repo).await {
        Ok(Some(list)) => Ok(Json(list)),
        Ok(None) => Err(ApiError::NotFound("repository")),
        Err(e) => Err(ApiError::Unavailable(format!("{e:#}"))),
    }
}

/// Push events from the organization's GitHub App.
#[utoipa::path(post, path = "/api/v1/hooks/github/{org_id}", tag = "github",
    params(("org_id" = String, Path)),
    request_body(content = String, description = "A GitHub webhook delivery."),
    responses((status = 202, body = GithubPushResult), (status = 200, body = GithubPushResult),
        (status = 401, body = ErrorBody)))]
async fn push_hook(
    State(state): State<AppState>,
    Path(org_id): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> ApiResult<(StatusCode, Json<GithubPushResult>)> {
    let creds = github::creds(&state, &org_id).await?;
    // Same answer for an unknown organization and a bad signature.
    // GitHub App webhooks always use HMAC-SHA256; never accept plain X-Gitlab-Token.
    if !creds.is_some_and(|c| verify_hmac(&c.webhook_secret, &headers, &body)) {
        return Err(ApiError::Unauthorized);
    };
    let skip = |why: &str| {
        Ok((
            StatusCode::OK,
            Json(GithubPushResult {
                deployment_ids: Vec::new(),
                skipped: Some(why.into()),
            }),
        ))
    };
    let header = |n: &str| headers.get(n).and_then(|v| v.to_str().ok());
    if let Some(id) = header("x-github-delivery")
        && !github::first_delivery(&state, &id.chars().take(64).collect::<String>()).await?
    {
        return skip("this delivery was already handled");
    }
    if header("x-github-event") != Some("push") {
        return skip("not a push event");
    }
    let Ok(ev) = serde_json::from_slice::<serde_json::Value>(&body) else {
        return Err(ApiError::Validation("the body is not JSON".into()));
    };
    if ev["deleted"].as_bool() == Some(true) {
        return skip("the branch was deleted");
    }
    let Some(branch) = ev["ref"]
        .as_str()
        .and_then(|r| r.strip_prefix("refs/heads/"))
    else {
        return skip("not a branch push");
    };
    let sha = ev["after"].as_str().filter(|s| s.len() == 40);
    let full_name = ev["repository"]["full_name"].as_str().unwrap_or_default();
    let default_branch = ev["repository"]["default_branch"]
        .as_str()
        .unwrap_or("main");
    let apps = github::apps_for_push(&state, &org_id, full_name, branch, default_branch).await?;
    if apps.is_empty() {
        return skip("no app deploys this branch on push");
    }
    let mut ids = Vec::new();
    let mut why = None;
    for app_id in apps {
        match github::deploy_commit(&state, &org_id, &app_id, sha, "github").await? {
            Ok(id) => ids.push(id),
            Err(w) => why = Some(w),
        }
    }
    let code = if ids.is_empty() {
        StatusCode::OK
    } else {
        StatusCode::ACCEPTED
    };
    Ok((
        code,
        Json(GithubPushResult {
            skipped: if ids.is_empty() { why } else { None },
            deployment_ids: ids,
        }),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loopback_redirect_url_validation() {
        assert!(validate_loopback_redirect_url("http://127.0.0.1:45123/callback").is_ok());
        assert!(validate_loopback_redirect_url("http://localhost:8080/callback").is_ok());
        assert!(validate_loopback_redirect_url("https://evil.example.com/callback").is_err());
        assert!(validate_loopback_redirect_url("http://127.0.0.1/callback").is_err());
        assert!(validate_loopback_redirect_url("http://user:pass@127.0.0.1:8080/cb").is_err());
        assert!(validate_loopback_redirect_url("https://127.0.0.1:8080/cb").is_err());
    }
}
