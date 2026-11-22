//! First-time setup, sign-in, API tokens and the audit log.

use axum::{
    Json,
    extract::{ConnectInfo, Path, State},
    http::{HeaderMap, StatusCode, header},
};
use axum_extra::extract::CookieJar;
use rig_api::{
    ApiToken, AuditEntry, CreateApiTokenRequest, CreatedApiToken, ErrorBody, LoginRequest, Me,
    Organization, PublicBranding, Role, SetupRequest, SetupStatus, User,
};
use sqlx::Row;
use std::net::SocketAddr;
use utoipa_axum::{router::OpenApiRouter, routes};

use crate::{
    auth::{self, Admin, AuditEvent, Auth, MinRole, SESSION_COOKIE, TOKEN_PREFIX, Viewer},
    crypto,
    error::{ApiError, ApiResult},
    setup,
    state::{AppState, new_id, now, ts},
};

pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(setup_status))
        .routes(routes!(complete_setup))
        .routes(routes!(login))
        .routes(routes!(logout))
        .routes(routes!(me))
        .routes(routes!(list_tokens, create_token))
        .routes(routes!(delete_token))
        .routes(routes!(list_audit))
}

const MAX_LOGIN_FAILURES_PER_IP: i64 = 20;
const MAX_LOGIN_FAILURES_PER_EMAIL: i64 = 8;
const MAX_SETUP_FAILURES_PER_IP: i64 = 10;

fn peer_ip(headers: &HeaderMap, addr: SocketAddr) -> String {
    auth::real_ip(headers, addr.ip()).to_string()
}

fn user_agent(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(header::USER_AGENT)
        .and_then(|v| v.to_str().ok())
}

pub(crate) fn validate_email(email: &str) -> ApiResult<String> {
    let email = email.trim().to_lowercase();
    let ok = email.len() <= 254
        && email.split_once('@').is_some_and(|(local, domain)| {
            !local.is_empty()
                && domain.contains('.')
                && !domain.starts_with('.')
                && !domain.ends_with('.')
        })
        && !email.chars().any(char::is_whitespace);
    if ok {
        Ok(email)
    } else {
        Err(ApiError::validation("enter a valid email address"))
    }
}

pub(crate) fn validate_password(password: &str) -> ApiResult<()> {
    if password.chars().count() < 10 {
        return Err(ApiError::validation(
            "password must be at least 10 characters",
        ));
    }
    if password.len() > 256 {
        return Err(ApiError::validation("password must be at most 256 bytes"));
    }
    Ok(())
}

pub fn validate_name(what: &str, name: &str, max: usize) -> ApiResult<String> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > max || name.chars().any(char::is_control) {
        return Err(ApiError::Validation(format!(
            "{what} must be 1 to {max} characters"
        )));
    }
    Ok(name.to_string())
}

/// Lowercase letters, digits and single dashes; safe in URLs and container names.
pub fn slugify(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    for c in name.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.ends_with('-') && !out.is_empty() {
            out.push('-');
        }
    }
    let out = out.trim_end_matches('-');
    let out: String = out.chars().take(48).collect();
    if out.is_empty() {
        "item".into()
    } else {
        out.trim_end_matches('-').to_string()
    }
}

/// Whether the instance still needs its first owner account.
#[utoipa::path(get, path = "/api/v1/setup", tag = "auth",
    responses((status = 200, body = SetupStatus)))]
async fn setup_status(State(state): State<AppState>) -> ApiResult<Json<SetupStatus>> {
    let setup_complete = setup::is_complete(&state).await?;
    let branding = if setup_complete {
        sqlx::query(
            "SELECT name, logo_url, accent_color, env_label, env_color
             FROM organizations ORDER BY created_at LIMIT 1",
        )
        .fetch_optional(&state.db)
        .await?
        .map(|row| PublicBranding {
            org_name: row.get("name"),
            logo_url: row.get("logo_url"),
            accent_color: row.get("accent_color"),
            env_label: row.get("env_label"),
            env_color: row.get("env_color"),
        })
    } else {
        None
    };
    Ok(Json(SetupStatus {
        setup_complete,
        version: env!("CARGO_PKG_VERSION").into(),
        branding,
    }))
}

/// Create the first owner and organization using the one-time setup token.
#[utoipa::path(post, path = "/api/v1/setup", tag = "auth", request_body = SetupRequest,
    responses((status = 200, body = Me), (status = 403, body = ErrorBody),
              (status = 409, body = ErrorBody), (status = 422, body = ErrorBody)))]
async fn complete_setup(
    State(state): State<AppState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    jar: CookieJar,
    Json(req): Json<SetupRequest>,
) -> ApiResult<(CookieJar, Json<Me>)> {
    auth::check_origin(&headers)?;
    let ip = peer_ip(&headers, addr);
    let rate_key = format!("setup:{ip}");
    auth::check_rate(&state, &rate_key, MAX_SETUP_FAILURES_PER_IP).await?;

    let email = validate_email(&req.email)?;
    validate_password(&req.password)?;
    let name = validate_name("name", &req.name, 80)?;
    let org_name = validate_name("organization name", &req.org_name, 80)?;

    if setup::is_complete(&state).await? {
        return Err(ApiError::Conflict(
            "setup has already been completed".into(),
        ));
    }
    if !setup::token_matches(&state, req.token.trim()).await? {
        auth::record_failure(&state, &rate_key).await?;
        return Err(ApiError::Forbidden("setup token is invalid or has expired"));
    }

    let password = req.password;
    let hash = tokio::task::spawn_blocking(move || crypto::hash_password(&password))
        .await
        .map_err(|e| ApiError::Internal(e.into()))??;

    let (user_id, org_id, server_id) = (new_id(), new_id(), new_id());
    let slug = slugify(&org_name);
    let mut tx = state.db.begin().await?;
    // Consume the token inside the transaction: of two racing requests,
    // exactly one deletes the row and proceeds.
    let consumed = sqlx::query("DELETE FROM settings WHERE key = 'setup_token_hash'")
        .execute(&mut *tx)
        .await?
        .rows_affected();
    if consumed != 1 {
        return Err(ApiError::Conflict(
            "setup has already been completed".into(),
        ));
    }
    sqlx::query("DELETE FROM settings WHERE key = 'setup_token_expires'")
        .execute(&mut *tx)
        .await?;
    sqlx::query("INSERT INTO organizations (id, name, slug) VALUES (?, ?, ?)")
        .bind(&org_id)
        .bind(&org_name)
        .bind(&slug)
        .execute(&mut *tx)
        .await?;
    sqlx::query("INSERT INTO users (id, email, name, password_hash) VALUES (?, ?, ?, ?)")
        .bind(&user_id)
        .bind(&email)
        .bind(&name)
        .bind(&hash)
        .execute(&mut *tx)
        .await?;
    sqlx::query("INSERT INTO memberships (org_id, user_id, role) VALUES (?, ?, 'owner')")
        .bind(&org_id)
        .bind(&user_id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("INSERT INTO servers (id, org_id, name, kind) VALUES (?, ?, 'local', 'local')")
        .bind(&server_id)
        .bind(&org_id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("INSERT INTO settings (key, value) VALUES ('setup_complete', '1')")
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;

    auth::audit(
        &state,
        AuditEvent {
            org_id: Some(&org_id),
            user_id: Some(&user_id),
            actor: &email,
            action: "setup.complete",
            target: None,
            detail: None,
            ip: Some(&ip),
        },
    )
    .await?;
    tracing::info!(%email, "setup complete");

    let (cookie, csrf_token) =
        auth::create_session(&state, &user_id, &org_id, user_agent(&headers), Some(&ip), auth::is_https(&state, &headers)).await?;
    Ok((
        jar.add(cookie),
        Json(Me {
            user: User {
                id: user_id,
                email,
                name,
                avatar_url: None,
            },
            org: Organization {
                id: org_id,
                name: org_name,
                slug,
                logo_url: None,
                accent_color: "emerald".into(),
                env_label: None,
                env_color: None,
            },
            role: Role::Owner,
            csrf_token,
        }),
    ))
}

/// Sign in with email and password. Sets the session cookie.
#[utoipa::path(post, path = "/api/v1/auth/login", tag = "auth", request_body = LoginRequest,
    responses((status = 200, body = Me), (status = 401, body = ErrorBody),
              (status = 429, body = ErrorBody)))]
async fn login(
    State(state): State<AppState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    jar: CookieJar,
    Json(req): Json<LoginRequest>,
) -> ApiResult<(CookieJar, Json<Me>)> {
    auth::check_origin(&headers)?;
    let ip = peer_ip(&headers, addr);
    let email = req.email.trim().to_lowercase();
    let (ip_key, email_key) = (format!("login:ip:{ip}"), format!("login:email:{email}"));
    auth::check_rate(&state, &ip_key, MAX_LOGIN_FAILURES_PER_IP).await?;
    auth::check_rate(&state, &email_key, MAX_LOGIN_FAILURES_PER_EMAIL).await?;

    let row = sqlx::query(
        "SELECT u.id, u.name, u.avatar_url, u.password_hash, u.disabled, u.totp_secret
         FROM users u WHERE u.email = ?",
    )
    .bind(&email)
    .fetch_optional(&state.db)
    .await?;

    // Always run one Argon2 verification, so response time does not reveal
    // whether the account exists.
    let (stored, found) = match &row {
        Some(r) if r.get::<i64, _>("disabled") == 0 => (r.get::<String, _>("password_hash"), true),
        _ => (setup::dummy_hash().to_string(), false),
    };
    let password = req.password;
    let ok = tokio::task::spawn_blocking(move || crypto::verify_password(&password, &stored))
        .await
        .map_err(|e| ApiError::Internal(e.into()))?;

    let Some(row) = row.filter(|_| ok && found) else {
        auth::record_failure(&state, &ip_key).await?;
        auth::record_failure(&state, &email_key).await?;
        auth::audit(
            &state,
            AuditEvent {
                org_id: None,
                user_id: None,
                actor: &email,
                action: "auth.login_failed",
                target: None,
                detail: None,
                ip: Some(&ip),
            },
        )
        .await?;
        return Err(ApiError::Unauthorized);
    };
    let user_id: String = row.get("id");

    // The password was right. With a second factor on, a code is needed too;
    // a missing one is not a failure, a wrong one is.
    if let Some(secret) = row.get::<Option<String>, _>("totp_secret") {
        let Some(code) = req.code.as_deref().map(str::trim).filter(|c| !c.is_empty()) else {
            return Err(ApiError::SecondFactorRequired);
        };
        let totp_key = format!("totp:user:{user_id}");
        auth::check_rate(&state, &totp_key, MAX_LOGIN_FAILURES_PER_EMAIL).await?;
        if !super::account::check_code(&state, &user_id, &secret, code).await? {
            auth::record_failure(&state, &ip_key).await?;
            auth::record_failure(&state, &totp_key).await?;
            auth::audit(
                &state,
                AuditEvent {
                    org_id: None,
                    user_id: Some(&user_id),
                    actor: &email,
                    action: "auth.login_failed",
                    target: None,
                    detail: Some("wrong second-factor code"),
                    ip: Some(&ip),
                },
            )
            .await?;
            return Err(ApiError::Unauthorized);
        }
        auth::clear_failures(&state, &totp_key).await?;
    }
    auth::clear_failures(&state, &email_key).await?;
    let m = sqlx::query(
        "SELECT o.id, o.name, o.slug, o.logo_url, o.accent_color, o.env_label, o.env_color, m.role FROM memberships m
         JOIN organizations o ON o.id = m.org_id
         WHERE m.user_id = ? ORDER BY m.created_at LIMIT 1",
    )
    .bind(&user_id)
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::Forbidden(
        "this account does not belong to an organization",
    ))?;
    let org = Organization {
        id: m.get("id"),
        name: m.get("name"),
        slug: m.get("slug"),
        logo_url: m.get("logo_url"),
        accent_color: m.get("accent_color"),
        env_label: m.get("env_label"),
        env_color: m.get("env_color"),
    };
    let role = Role::parse(m.get("role"))
        .ok_or_else(|| ApiError::Internal(anyhow::anyhow!("bad role")))?;

    sqlx::query("UPDATE users SET last_login_at = ? WHERE id = ?")
        .bind(now())
        .bind(&user_id)
        .execute(&state.db)
        .await?;
    auth::audit(
        &state,
        AuditEvent {
            org_id: Some(&org.id),
            user_id: Some(&user_id),
            actor: &email,
            action: "auth.login",
            target: None,
            detail: None,
            ip: Some(&ip),
        },
    )
    .await?;

    let (cookie, csrf_token) =
        auth::create_session(&state, &user_id, &org.id, user_agent(&headers), Some(&ip), auth::is_https(&state, &headers)).await?;
    Ok((
        jar.add(cookie),
        Json(Me {
            user: User {
                id: user_id,
                email,
                name: row.get("name"),
                avatar_url: row.get("avatar_url"),
            },
            org,
            role,
            csrf_token,
        }),
    ))
}

/// End the current session.
#[utoipa::path(post, path = "/api/v1/auth/logout", tag = "auth",
    responses((status = 204), (status = 401, body = ErrorBody)))]
async fn logout(
    State(state): State<AppState>,
    auth: Auth<Viewer>,
    headers: HeaderMap,
    jar: CookieJar,
) -> ApiResult<(CookieJar, StatusCode)> {
    if let Some(c) = jar.get(SESSION_COOKIE) {
        auth::delete_session(&state, c.value()).await?;
    }
    auth.audit(&state, "auth.logout", None, None).await?;
    Ok((
        jar.add(auth::clear_session_cookie(auth::is_https(&state, &headers))),
        StatusCode::NO_CONTENT,
    ))
}

/// The signed-in user, their organization and role.
#[utoipa::path(get, path = "/api/v1/auth/me", tag = "auth",
    responses((status = 200, body = Me), (status = 401, body = ErrorBody)))]
async fn me(State(state): State<AppState>, auth: Auth<Viewer>) -> ApiResult<Json<Me>> {
    let row = sqlx::query(
        "SELECT u.name AS user_name, u.avatar_url,
                o.name AS org_name, o.slug, o.logo_url, o.accent_color, o.env_label, o.env_color
         FROM users u, organizations o WHERE u.id = ? AND o.id = ?",
    )
    .bind(&auth.user_id)
    .bind(&auth.org_id)
    .fetch_one(&state.db)
    .await?;
    Ok(Json(Me {
        user: User {
            id: auth.user_id,
            email: auth.email,
            name: row.get("user_name"),
            avatar_url: row.get("avatar_url"),
        },
        org: Organization {
            id: auth.org_id,
            name: row.get("org_name"),
            slug: row.get("slug"),
            logo_url: row.get("logo_url"),
            accent_color: row.get("accent_color"),
            env_label: row.get("env_label"),
            env_color: row.get("env_color"),
        },
        role: auth.role,
        csrf_token: auth.csrf_token.unwrap_or_default(),
    }))
}

fn token_from_row(r: &sqlx::sqlite::SqliteRow) -> ApiToken {
    ApiToken {
        id: r.get("id"),
        name: r.get("name"),
        prefix: r.get("prefix"),
        role: Role::parse(r.get("role")).unwrap_or(Role::Viewer),
        last_used_at: r.get("last_used_at"),
        expires_at: r.get("expires_at"),
        created_at: r.get("created_at"),
    }
}

/// Your API tokens in this organization.
#[utoipa::path(get, path = "/api/v1/tokens", tag = "tokens",
    responses((status = 200, body = Vec<ApiToken>)))]
async fn list_tokens(
    State(state): State<AppState>,
    auth: Auth<Viewer>,
) -> ApiResult<Json<Vec<ApiToken>>> {
    let rows = sqlx::query(
        "SELECT id, name, prefix, role, last_used_at, expires_at, created_at
         FROM api_tokens WHERE org_id = ? AND user_id = ? ORDER BY created_at DESC",
    )
    .bind(&auth.org_id)
    .bind(&auth.user_id)
    .fetch_all(&state.db)
    .await?;
    Ok(Json(rows.iter().map(token_from_row).collect()))
}

/// Create an API token for the CLI or MCP. The secret is returned once.
#[utoipa::path(post, path = "/api/v1/tokens", tag = "tokens", request_body = CreateApiTokenRequest,
    responses((status = 200, body = CreatedApiToken), (status = 422, body = ErrorBody)))]
async fn create_token(
    State(state): State<AppState>,
    auth: Auth<Viewer>,
    Json(req): Json<CreateApiTokenRequest>,
) -> ApiResult<Json<CreatedApiToken>> {
    let name = validate_name("token name", &req.name, 60)?;
    if req.role > auth.role {
        return Err(ApiError::Forbidden(
            "a token cannot have a higher role than your own",
        ));
    }
    let expires_at = match req.expires_in_days {
        Some(d) if !(1..=3650).contains(&d) => {
            return Err(ApiError::validation(
                "expiry must be between 1 and 3650 days",
            ));
        }
        Some(d) => Some(ts(chrono::Utc::now() + chrono::Duration::days(d.into()))),
        None => None,
    };
    let secret = format!("{TOKEN_PREFIX}{}", crypto::random_token(32));
    let prefix: String = secret.chars().take(TOKEN_PREFIX.len() + 6).collect();
    let (id, created_at) = (new_id(), now());
    sqlx::query(
        "INSERT INTO api_tokens (id, org_id, user_id, name, token_hash, prefix, role, expires_at, created_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&id).bind(&auth.org_id).bind(&auth.user_id).bind(&name)
    .bind(crypto::hash_token(&secret)).bind(&prefix).bind(req.role.as_str())
    .bind(&expires_at).bind(&created_at)
    .execute(&state.db)
    .await?;
    auth.audit(&state, "token.create", Some(&name), Some(req.role.as_str()))
        .await?;
    Ok(Json(CreatedApiToken {
        token: ApiToken {
            id,
            name,
            prefix,
            role: req.role,
            last_used_at: None,
            expires_at,
            created_at,
        },
        secret,
    }))
}

/// Revoke one of your API tokens.
#[utoipa::path(delete, path = "/api/v1/tokens/{id}", tag = "tokens",
    params(("id" = String, Path)),
    responses((status = 204), (status = 404, body = ErrorBody)))]
async fn delete_token(
    State(state): State<AppState>,
    auth: Auth<Viewer>,
    Path(id): Path<String>,
) -> ApiResult<StatusCode> {
    let n = sqlx::query("DELETE FROM api_tokens WHERE id = ? AND org_id = ? AND user_id = ?")
        .bind(&id)
        .bind(&auth.org_id)
        .bind(&auth.user_id)
        .execute(&state.db)
        .await?
        .rows_affected();
    if n == 0 {
        return Err(ApiError::NotFound("token"));
    }
    auth.audit(&state, "token.revoke", Some(&id), None).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// The most recent audit log entries for the organization.
#[utoipa::path(get, path = "/api/v1/audit", tag = "audit",
    responses((status = 200, body = Vec<AuditEntry>), (status = 403, body = ErrorBody)))]
async fn list_audit(
    State(state): State<AppState>,
    auth: Auth<Admin>,
) -> ApiResult<Json<Vec<AuditEntry>>> {
    debug_assert!(Admin::ROLE >= Role::Admin);
    let rows = sqlx::query(
        "SELECT id, actor, action, target, detail, ip, created_at
         FROM audit_log WHERE org_id = ? ORDER BY created_at DESC, id DESC LIMIT 200",
    )
    .bind(&auth.org_id)
    .fetch_all(&state.db)
    .await?;
    Ok(Json(
        rows.iter()
            .map(|r| AuditEntry {
                id: r.get("id"),
                actor: r.get("actor"),
                action: r.get("action"),
                target: r.get("target"),
                detail: r.get("detail"),
                ip: r.get("ip"),
                created_at: r.get("created_at"),
            })
            .collect(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugs_are_url_safe() {
        assert_eq!(slugify("My Cool App!"), "my-cool-app");
        assert_eq!(slugify("  --a__b  "), "a-b");
        assert_eq!(slugify("日本"), "item");
        assert!(slugify(&"x".repeat(200)).len() <= 48);
    }

    #[test]
    fn email_and_password_rules() {
        assert_eq!(validate_email(" A@B.co ").unwrap(), "a@b.co");
        for bad in ["", "a", "a@b", "@b.co", "a b@c.co", "a@.co"] {
            assert!(validate_email(bad).is_err(), "{bad}");
        }
        assert!(validate_password("short").is_err());
        assert!(validate_password("long enough pw").is_ok());
    }
}
