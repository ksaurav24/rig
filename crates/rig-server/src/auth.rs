//! Authentication and authorization.
//!
//! Every protected handler takes an [`Auth<R>`] argument, where `R` names the
//! minimum role. Extraction resolves the principal (session cookie or API
//! token), enforces CSRF for cookie sessions, and rejects insufficient roles,
//! so a handler body can never run unauthenticated or under-privileged.

use std::{
    marker::PhantomData,
    net::{IpAddr, SocketAddr},
};

use axum::{
    extract::{ConnectInfo, FromRequestParts},
    http::{HeaderMap, Method, header, request::Parts},
};
use axum_extra::extract::{
    CookieJar,
    cookie::{Cookie, SameSite},
};
use rig_api::Role;
use sqlx::Row;

use crate::{
    crypto,
    error::{ApiError, ApiResult},
    state::{AppState, new_id, now, ts},
};

pub const SESSION_COOKIE: &str = "rig_session";
pub const CSRF_HEADER: &str = "x-rig-csrf";
pub const TOKEN_PREFIX: &str = "rig_";
const SESSION_DAYS: i64 = 14;

pub trait MinRole: Send + Sync + 'static {
    const ROLE: Role;
}

macro_rules! min_role {
    ($($name:ident),*) => {$(
        // `Owner` has no route yet; member management will use it.
        #[allow(dead_code)]
        #[derive(Debug)]
        pub struct $name;
        impl MinRole for $name { const ROLE: Role = Role::$name; }
    )*};
}
min_role!(Viewer, Developer, Admin, Owner);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Via {
    Session,
    ApiToken,
}

/// The authenticated principal, scoped to one organization.
#[derive(Debug)]
pub struct Auth<R: MinRole = Viewer> {
    pub user_id: String,
    pub org_id: String,
    pub email: String,
    pub role: Role,
    pub via: Via,
    pub ip: Option<String>,
    /// Set for cookie sessions only.
    pub csrf_token: Option<String>,
    _min: PhantomData<R>,
}

impl<R: MinRole> FromRequestParts<AppState> for Auth<R> {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> ApiResult<Self> {
        let ip = client_ip(parts);
        let mut auth = match bearer(&parts.headers) {
            Some(token) => from_api_token(state, token).await?,
            None => from_session(state, parts).await?,
        };
        if auth.role < R::ROLE {
            return Err(ApiError::Forbidden("your role does not allow this action"));
        }
        auth.ip = ip;
        Ok(Auth {
            user_id: auth.user_id,
            org_id: auth.org_id,
            email: auth.email,
            role: auth.role,
            via: auth.via,
            ip: auth.ip,
            csrf_token: auth.csrf_token,
            _min: PhantomData,
        })
    }
}

fn bearer(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(header::AUTHORIZATION)?
        .to_str()
        .ok()?
        .strip_prefix("Bearer ")
        .map(str::trim)
        .filter(|t| !t.is_empty())
}

async fn from_api_token(state: &AppState, token: &str) -> ApiResult<Auth<Viewer>> {
    let hash = crypto::hash_token(token);
    let row = sqlx::query(
        "SELECT t.id, t.user_id, t.org_id, t.role AS token_role, m.role AS member_role, u.email
         FROM api_tokens t
         JOIN users u ON u.id = t.user_id AND u.disabled = 0
         JOIN memberships m ON m.org_id = t.org_id AND m.user_id = t.user_id
         WHERE t.token_hash = ? AND (t.expires_at IS NULL OR t.expires_at > ?)",
    )
    .bind(&hash)
    .bind(now())
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::Unauthorized)?;

    // A token can never exceed its owner's current role.
    let token_role = parse_role(row.get("token_role"))?;
    let member_role = parse_role(row.get("member_role"))?;
    let id: String = row.get("id");

    // Coarse last-used tracking: at most one write per token per minute.
    let cutoff = ts(chrono::Utc::now() - chrono::Duration::minutes(1));
    sqlx::query(
        "UPDATE api_tokens SET last_used_at = ?
         WHERE id = ? AND (last_used_at IS NULL OR last_used_at < ?)",
    )
    .bind(now())
    .bind(&id)
    .bind(cutoff)
    .execute(&state.db)
    .await?;

    Ok(Auth {
        user_id: row.get("user_id"),
        org_id: row.get("org_id"),
        email: row.get("email"),
        role: token_role.min(member_role),
        via: Via::ApiToken,
        ip: None,
        csrf_token: None,
        _min: PhantomData,
    })
}

async fn from_session(state: &AppState, parts: &Parts) -> ApiResult<Auth<Viewer>> {
    let jar = CookieJar::from_headers(&parts.headers);
    let token = jar
        .get(SESSION_COOKIE)
        .map(Cookie::value)
        .ok_or(ApiError::Unauthorized)?;
    let row = sqlx::query(
        "SELECT s.user_id, s.org_id, s.csrf_token, m.role, u.email
         FROM sessions s
         JOIN users u ON u.id = s.user_id AND u.disabled = 0
         JOIN memberships m ON m.org_id = s.org_id AND m.user_id = s.user_id
         WHERE s.token_hash = ? AND s.expires_at > ?",
    )
    .bind(crypto::hash_token(token))
    .bind(now())
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::Unauthorized)?;

    let csrf: String = row.get("csrf_token");
    if !is_safe(&parts.method) {
        check_origin(&parts.headers)?;
        let sent = parts
            .headers
            .get(CSRF_HEADER)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        if !crypto::constant_time_eq(sent, &csrf) {
            return Err(ApiError::Forbidden("missing or invalid CSRF token"));
        }
    }

    Ok(Auth {
        user_id: row.get("user_id"),
        org_id: row.get("org_id"),
        email: row.get("email"),
        role: parse_role(row.get("role"))?,
        via: Via::Session,
        ip: None,
        csrf_token: Some(csrf),
        _min: PhantomData,
    })
}

fn parse_role(s: &str) -> ApiResult<Role> {
    Role::parse(s)
        .ok_or_else(|| ApiError::Internal(anyhow::anyhow!("unknown role {s:?} in database")))
}

fn is_safe(method: &Method) -> bool {
    matches!(*method, Method::GET | Method::HEAD | Method::OPTIONS)
}

/// Rejects cross-origin browser requests. Browsers always send `Origin` on
/// cross-site unsafe requests, so a present-but-mismatched value is an attack
/// or a misconfigured proxy. Used for cookie sessions, the unauthenticated
/// auth endpoints and WebSocket upgrades.
pub fn check_origin(headers: &HeaderMap) -> ApiResult<()> {
    let Some(origin) = headers.get(header::ORIGIN).and_then(|v| v.to_str().ok()) else {
        return Ok(());
    };
    let host = headers
        .get("x-forwarded-host")
        .or_else(|| headers.get(header::HOST))
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    let origin_host = origin.split_once("://").map(|(_, h)| h).unwrap_or(origin);
    if !host.is_empty() && origin_host.eq_ignore_ascii_case(host) {
        Ok(())
    } else {
        Err(ApiError::Forbidden("cross-origin request blocked"))
    }
}

/// The client address. `X-Forwarded-For` is trusted only when the direct
/// peer is loopback, i.e. the request came through the local proxy.
pub fn client_ip(parts: &Parts) -> Option<String> {
    let peer = parts
        .extensions
        .get::<ConnectInfo<SocketAddr>>()
        .map(|c| c.0.ip())?;
    Some(real_ip(&parts.headers, peer).to_string())
}

/// The address a request really came from. Caddy runs on this machine and
/// reaches Rig over loopback, so only then is the last `X-Forwarded-For`
/// entry (the one Caddy itself appended) believed; anything else could be
/// forged by the client.
pub fn real_ip(headers: &HeaderMap, peer: IpAddr) -> IpAddr {
    if peer.is_loopback()
        && let Some(fwd) = headers.get("x-forwarded-for").and_then(|v| v.to_str().ok())
        && let Some(ip) = fwd
            .rsplit(',')
            .next()
            .and_then(|s| s.trim().parse::<IpAddr>().ok())
    {
        return ip;
    }
    peer
}

/// Whether the browser reached Rig over HTTPS (through the proxy), so the
/// session cookie must be marked Secure.
pub fn is_https(state: &AppState, headers: &HeaderMap) -> bool {
    state.config.secure_cookies
        || headers
            .get("x-forwarded-proto")
            .and_then(|v| v.to_str().ok())
            .is_some_and(|p| p.eq_ignore_ascii_case("https"))
}

/// Creates a session and returns the cookie to set plus the CSRF token.
pub async fn create_session(
    state: &AppState,
    user_id: &str,
    org_id: &str,
    user_agent: Option<&str>,
    ip: Option<&str>,
    secure: bool,
) -> ApiResult<(Cookie<'static>, String)> {
    let token = crypto::random_token(32);
    let csrf = crypto::random_token(24);
    let expires = chrono::Utc::now() + chrono::Duration::days(SESSION_DAYS);
    sqlx::query(
        "INSERT INTO sessions (token_hash, user_id, org_id, csrf_token, user_agent, ip, expires_at)
         VALUES (?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(crypto::hash_token(&token))
    .bind(user_id)
    .bind(org_id)
    .bind(&csrf)
    .bind(user_agent.map(|ua| ua.chars().take(256).collect::<String>()))
    .bind(ip)
    .bind(ts(expires))
    .execute(&state.db)
    .await?;
    Ok((session_cookie(token, SESSION_DAYS, secure), csrf))
}

pub fn clear_session_cookie(secure: bool) -> Cookie<'static> {
    session_cookie(String::new(), 0, secure)
}

fn session_cookie(value: String, days: i64, secure: bool) -> Cookie<'static> {
    Cookie::build((SESSION_COOKIE, value))
        .path("/")
        .http_only(true)
        .same_site(SameSite::Strict)
        .secure(secure)
        .max_age(time_duration(days))
        .build()
}

fn time_duration(days: i64) -> time::Duration {
    time::Duration::days(days)
}

pub async fn delete_session(state: &AppState, token: &str) -> ApiResult<()> {
    sqlx::query("DELETE FROM sessions WHERE token_hash = ?")
        .bind(crypto::hash_token(token))
        .execute(&state.db)
        .await?;
    Ok(())
}

// ------------------------------------------------------------ rate limiting

const WINDOW_SECS: i64 = 15 * 60;

/// Fails once `key` has `max` failures recorded inside the window.
pub async fn check_rate(state: &AppState, key: &str, max: i64) -> ApiResult<()> {
    let since = chrono::Utc::now().timestamp() - WINDOW_SECS;
    let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM login_attempts WHERE key = ? AND ts > ?")
        .bind(key)
        .bind(since)
        .fetch_one(&state.db)
        .await?;
    if n >= max {
        Err(ApiError::RateLimited)
    } else {
        Ok(())
    }
}

pub async fn record_failure(state: &AppState, key: &str) -> ApiResult<()> {
    sqlx::query("INSERT INTO login_attempts (key, ts) VALUES (?, ?)")
        .bind(key)
        .bind(chrono::Utc::now().timestamp())
        .execute(&state.db)
        .await?;
    Ok(())
}

pub async fn clear_failures(state: &AppState, key: &str) -> ApiResult<()> {
    sqlx::query("DELETE FROM login_attempts WHERE key = ?")
        .bind(key)
        .execute(&state.db)
        .await?;
    Ok(())
}

// ------------------------------------------------------------------- audit

pub struct AuditEvent<'a> {
    pub org_id: Option<&'a str>,
    pub user_id: Option<&'a str>,
    pub actor: &'a str,
    pub action: &'a str,
    pub target: Option<&'a str>,
    pub detail: Option<&'a str>,
    pub ip: Option<&'a str>,
}

pub async fn audit(state: &AppState, e: AuditEvent<'_>) -> ApiResult<()> {
    sqlx::query(
        "INSERT INTO audit_log (id, org_id, user_id, actor, action, target, detail, ip)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(new_id())
    .bind(e.org_id)
    .bind(e.user_id)
    .bind(e.actor)
    .bind(e.action)
    .bind(e.target)
    .bind(e.detail)
    .bind(e.ip)
    .execute(&state.db)
    .await?;
    Ok(())
}

impl<R: MinRole> Auth<R> {
    /// Records an action by this principal in the organization's audit log.
    pub async fn audit(
        &self,
        state: &AppState,
        action: &str,
        target: Option<&str>,
        detail: Option<&str>,
    ) -> ApiResult<()> {
        audit(
            state,
            AuditEvent {
                org_id: Some(&self.org_id),
                user_id: Some(&self.user_id),
                actor: &self.email,
                action,
                target,
                detail,
                ip: self.ip.as_deref(),
            },
        )
        .await
    }
}
