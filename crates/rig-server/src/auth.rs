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
use rig_api::{Role, TokenKind};
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
const SESSION_IDLE_DAYS: i64 = 7;

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

/// Snapshot of the credentials that opened a long-lived stream (WebSocket or SSE),
/// used to periodically re-verify that the session/token is still valid and the
/// user still holds the required role.
#[derive(Debug, Clone)]
pub struct AuthSnapshot {
    pub user_id: String,
    pub org_id: String,
    pub via: Via,
    pub credential_hash: String,
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
    /// SHA-256 hash of the active session cookie or API token.
    pub credential_hash: String,
    /// Set for API tokens only.
    pub token_id: Option<String>,
    /// Set for API tokens only (`Api` vs `Device`).
    pub token_kind: Option<TokenKind>,
    /// True when the account must change its temporary password before using Rig.
    pub must_change_password: bool,
    _min: PhantomData<R>,
}

impl<R: MinRole> Auth<R> {
    /// Rejects API tokens on interactive account/identity/network endpoints.
    pub fn require_session(&self, msg: &'static str) -> ApiResult<()> {
        match self.via {
            Via::Session => Ok(()),
            Via::ApiToken => Err(ApiError::Forbidden(msg)),
        }
    }

    /// Allows browser sessions and paired desktop/mobile device tokens, while
    /// still blocking non-interactive API/CI tokens (`kind = 'api'`).
    pub fn require_interactive(&self, msg: &'static str) -> ApiResult<()> {
        match (self.via, self.token_kind) {
            (Via::Session, _) | (Via::ApiToken, Some(TokenKind::Device)) => Ok(()),
            _ => Err(ApiError::Forbidden(msg)),
        }
    }

    pub fn snapshot(&self) -> AuthSnapshot {
        AuthSnapshot {
            user_id: self.user_id.clone(),
            org_id: self.org_id.clone(),
            via: self.via,
            credential_hash: self.credential_hash.clone(),
        }
    }
}

/// Paths a user with a temporary password (`must_change_password = 1`) is
/// still allowed to call so they can see who they are, sign out, or set a
/// permanent password.
fn allowed_during_password_reset(path: &str) -> bool {
    matches!(
        path,
        "/api/v1/auth/me" | "/api/v1/auth/logout" | "/api/v1/auth/password"
    )
}

impl<R: MinRole> FromRequestParts<AppState> for Auth<R> {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> ApiResult<Self> {
        let ip = client_ip(parts);
        let mut auth = match bearer(&parts.headers) {
            Some(token) => from_api_token(state, token, ip.as_deref()).await?,
            None => from_session(state, parts).await?,
        };
        if auth.must_change_password
            && (auth.via == Via::ApiToken || !allowed_during_password_reset(parts.uri.path()))
        {
            return Err(ApiError::Forbidden(
                "you must change your temporary password before continuing",
            ));
        }
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
            credential_hash: auth.credential_hash,
            token_id: auth.token_id,
            token_kind: auth.token_kind,
            must_change_password: auth.must_change_password,
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

/// Non-failing authentication check for endpoints that enrich their response
/// when the caller presents valid credentials (e.g. `GET /api/v1/meta`).
pub async fn is_authenticated_request(state: &AppState, headers: &HeaderMap) -> bool {
    if let Some(token) = bearer(headers) {
        return from_api_token(state, token, None).await.is_ok();
    }
    let jar = CookieJar::from_headers(headers);
    let Some(token) = jar.get(SESSION_COOKIE).map(Cookie::value) else {
        return false;
    };
    let hash = crypto::hash_token(token);
    let current_now = now();
    let idle_cutoff = ts(chrono::Utc::now() - chrono::Duration::days(SESSION_IDLE_DAYS));
    sqlx::query_scalar::<_, i64>(
        "SELECT 1 FROM sessions s
         JOIN users u ON u.id = s.user_id AND u.disabled = 0
         WHERE s.token_hash = ? AND s.expires_at > ?
           AND (s.last_active_at IS NULL OR s.last_active_at > ?)",
    )
    .bind(&hash)
    .bind(&current_now)
    .bind(&idle_cutoff)
    .fetch_optional(&state.db)
    .await
    .ok()
    .flatten()
    .is_some()
}

async fn from_api_token(
    state: &AppState,
    token: &str,
    ip: Option<&str>,
) -> ApiResult<Auth<Viewer>> {
    let hash = crypto::hash_token(token);
    let current_now = now();
    let row = sqlx::query(
        "SELECT t.id, t.user_id, t.org_id, t.role AS token_role, t.kind AS token_kind, t.last_used_at,
                m.role AS member_role, u.email, u.must_change_password
         FROM api_tokens t
         JOIN users u ON u.id = t.user_id AND u.disabled = 0
         JOIN memberships m ON m.org_id = t.org_id AND m.user_id = t.user_id
         WHERE t.token_hash = ? AND (t.expires_at IS NULL OR t.expires_at > ?)",
    )
    .bind(&hash)
    .bind(&current_now)
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::Unauthorized)?;

    // A token can never exceed its owner's current role.
    let token_role = parse_role(row.get("token_role"))?;
    let token_kind = TokenKind::parse(row.get("token_kind"));
    let member_role = parse_role(row.get("member_role"))?;
    let id: String = row.get("id");

    // Coarse last-used + last-ip tracking: at most one write per token per minute.
    let cutoff = ts(chrono::Utc::now() - chrono::Duration::minutes(1));
    let last_used: Option<String> = row.get("last_used_at");
    if last_used.as_deref().is_none_or(|t| t < cutoff.as_str()) {
        let _ = sqlx::query(
            "UPDATE api_tokens SET last_used_at = ?, last_ip = COALESCE(?, last_ip)
             WHERE id = ? AND (last_used_at IS NULL OR last_used_at < ?)",
        )
        .bind(&current_now)
        .bind(ip)
        .bind(&id)
        .bind(&cutoff)
        .execute(&state.db)
        .await;
    }

    Ok(Auth {
        user_id: row.get("user_id"),
        org_id: row.get("org_id"),
        email: row.get("email"),
        role: token_role.min(member_role),
        via: Via::ApiToken,
        ip: None,
        csrf_token: None,
        credential_hash: hash,
        token_id: Some(id),
        token_kind: Some(token_kind),
        must_change_password: row.get::<i64, _>("must_change_password") != 0,
        _min: PhantomData,
    })
}

async fn from_session(state: &AppState, parts: &Parts) -> ApiResult<Auth<Viewer>> {
    let jar = CookieJar::from_headers(&parts.headers);
    let token = jar
        .get(SESSION_COOKIE)
        .map(Cookie::value)
        .ok_or(ApiError::Unauthorized)?;
    let hash = crypto::hash_token(token);
    let current_now = now();
    let idle_cutoff = ts(chrono::Utc::now() - chrono::Duration::days(SESSION_IDLE_DAYS));
    let row = sqlx::query(
        "SELECT s.user_id, s.org_id, s.csrf_token, s.last_active_at, m.role,
                u.email, u.must_change_password
         FROM sessions s
         JOIN users u ON u.id = s.user_id AND u.disabled = 0
         JOIN memberships m ON m.org_id = s.org_id AND m.user_id = s.user_id
         WHERE s.token_hash = ? AND s.expires_at > ?
           AND (s.last_active_at IS NULL OR s.last_active_at > ?)",
    )
    .bind(&hash)
    .bind(&current_now)
    .bind(&idle_cutoff)
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::Unauthorized)?;

    let csrf: String = row.get("csrf_token");
    let peer = direct_peer(parts);
    if !is_safe(&parts.method) {
        check_origin(state, &parts.headers, peer)?;
        let sent = parts
            .headers
            .get(CSRF_HEADER)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        if !crypto::constant_time_eq(sent, &csrf) {
            return Err(ApiError::Forbidden("missing or invalid CSRF token"));
        }
    }

    // Coarse last-active tracking: at most one write per session every 5 minutes.
    let touch_cutoff = ts(chrono::Utc::now() - chrono::Duration::minutes(5));
    let last_active: Option<String> = row.get("last_active_at");
    if last_active.as_deref().is_none_or(|t| t < touch_cutoff.as_str()) {
        let _ = sqlx::query("UPDATE sessions SET last_active_at = ? WHERE token_hash = ?")
            .bind(&current_now)
            .bind(&hash)
            .execute(&state.db)
            .await;
    }

    Ok(Auth {
        user_id: row.get("user_id"),
        org_id: row.get("org_id"),
        email: row.get("email"),
        role: parse_role(row.get("role"))?,
        via: Via::Session,
        ip: None,
        csrf_token: Some(csrf),
        credential_hash: hash,
        token_id: None,
        token_kind: None,
        must_change_password: row.get::<i64, _>("must_change_password") != 0,
        _min: PhantomData,
    })
}

/// Re-verify that a session or API token that opened a long-lived stream
/// (WebSocket shell or SSE log tail) is still active and still holds `min_role`.
pub async fn verify_still_valid(state: &AppState, snap: &AuthSnapshot, min_role: Role) -> bool {
    let current_now = now();
    match snap.via {
        Via::Session => {
            let idle_cutoff = ts(chrono::Utc::now() - chrono::Duration::days(SESSION_IDLE_DAYS));
            let role: Option<String> = sqlx::query_scalar(
                "SELECT m.role
                 FROM sessions s
                 JOIN users u ON u.id = s.user_id AND u.disabled = 0 AND u.must_change_password = 0
                 JOIN memberships m ON m.org_id = s.org_id AND m.user_id = s.user_id
                 WHERE s.token_hash = ? AND s.user_id = ? AND s.org_id = ?
                   AND s.expires_at > ?
                   AND (s.last_active_at IS NULL OR s.last_active_at > ?)",
            )
            .bind(&snap.credential_hash)
            .bind(&snap.user_id)
            .bind(&snap.org_id)
            .bind(&current_now)
            .bind(&idle_cutoff)
            .fetch_optional(&state.db)
            .await
            .ok()
            .flatten();
            role.and_then(|r| Role::parse(&r))
                .is_some_and(|r| r >= min_role)
        }
        Via::ApiToken => {
            let row: Option<(String, String)> = sqlx::query_as(
                "SELECT t.role, m.role
                 FROM api_tokens t
                 JOIN users u ON u.id = t.user_id AND u.disabled = 0 AND u.must_change_password = 0
                 JOIN memberships m ON m.org_id = t.org_id AND m.user_id = t.user_id
                 WHERE t.token_hash = ? AND t.user_id = ? AND t.org_id = ?
                   AND (t.expires_at IS NULL OR t.expires_at > ?)",
            )
            .bind(&snap.credential_hash)
            .bind(&snap.user_id)
            .bind(&snap.org_id)
            .bind(&current_now)
            .fetch_optional(&state.db)
            .await
            .ok()
            .flatten();
            row.and_then(|(tr, mr)| Some(Role::parse(&tr)?.min(Role::parse(&mr)?)))
                .is_some_and(|r| r >= min_role)
        }
    }
}

fn parse_role(s: &str) -> ApiResult<Role> {
    Role::parse(s)
        .ok_or_else(|| ApiError::Internal(anyhow::anyhow!("unknown role {s:?} in database")))
}

fn is_safe(method: &Method) -> bool {
    matches!(*method, Method::GET | Method::HEAD | Method::OPTIONS)
}

fn direct_peer(parts: &Parts) -> Option<IpAddr> {
    parts
        .extensions
        .get::<ConnectInfo<SocketAddr>>()
        .map(|c| c.0.ip())
}

/// Rejects cross-origin browser requests. Browsers always send `Origin` on
/// cross-site unsafe requests and WebSocket upgrades, so a present-but-mismatched
/// scheme or host is blocked. `X-Forwarded-Host` and `X-Forwarded-Proto` are
/// trusted only when the direct TCP peer is loopback (the local Caddy proxy).
pub fn check_origin(
    state: &AppState,
    headers: &HeaderMap,
    peer: Option<IpAddr>,
) -> ApiResult<()> {
    check_origin_for(state.config.secure_cookies, headers, peer)
}

fn check_origin_for(
    secure_cookies: bool,
    headers: &HeaderMap,
    peer: Option<IpAddr>,
) -> ApiResult<()> {
    let Some(origin) = headers.get(header::ORIGIN).and_then(|v| v.to_str().ok()) else {
        return Ok(());
    };
    let trusted_proxy = peer.is_some_and(|p| p.is_loopback());
    let host = if trusted_proxy {
        headers
            .get("x-forwarded-host")
            .or_else(|| headers.get(header::HOST))
    } else {
        headers.get(header::HOST)
    }
    .and_then(|v| v.to_str().ok())
    .unwrap_or("");
    let expected_scheme = if is_https_for(secure_cookies, headers, peer) {
        "https"
    } else {
        "http"
    };
    let Some((origin_scheme, origin_host)) = origin.split_once("://") else {
        return Err(ApiError::Forbidden("cross-origin request blocked"));
    };
    if !host.is_empty()
        && origin_scheme.eq_ignore_ascii_case(expected_scheme)
        && origin_host.eq_ignore_ascii_case(host)
    {
        Ok(())
    } else {
        Err(ApiError::Forbidden("cross-origin request blocked"))
    }
}

/// The client address. `X-Forwarded-For` is trusted only when the direct
/// peer is loopback, i.e. the request came through the local proxy.
pub fn client_ip(parts: &Parts) -> Option<String> {
    let peer = direct_peer(parts)?;
    Some(real_ip(&parts.headers, peer).to_string())
}

/// The address a request really came from. Caddy runs on this machine and
/// reaches Rig over loopback, so only then is the last `X-Forwarded-For`
/// entry across all `X-Forwarded-For` headers (the one Caddy itself appended)
/// believed; anything else could be forged by the client.
pub fn real_ip(headers: &HeaderMap, peer: IpAddr) -> IpAddr {
    if peer.is_loopback() {
        let last_ip = headers
            .get_all("x-forwarded-for")
            .iter()
            .filter_map(|v| v.to_str().ok())
            .flat_map(|s| s.split(','))
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .filter_map(|s| s.parse::<IpAddr>().ok())
            .next_back();
        if let Some(ip) = last_ip {
            return ip;
        }
    }
    peer
}

/// Whether the browser reached Rig over HTTPS (through the local proxy), so
/// the session cookie must be marked Secure and HSTS can be sent.
pub fn is_https(state: &AppState, headers: &HeaderMap, peer: Option<IpAddr>) -> bool {
    is_https_for(state.config.secure_cookies, headers, peer)
}

fn is_https_for(secure_cookies: bool, headers: &HeaderMap, peer: Option<IpAddr>) -> bool {
    if secure_cookies {
        return true;
    }
    if peer.is_some_and(|p| p.is_loopback()) {
        return headers
            .get("x-forwarded-proto")
            .and_then(|v| v.to_str().ok())
            .is_some_and(|p| p.eq_ignore_ascii_case("https"));
    }
    false
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
    let current = now();
    let expires = chrono::Utc::now() + chrono::Duration::days(SESSION_DAYS);
    sqlx::query(
        "INSERT INTO sessions (token_hash, user_id, org_id, csrf_token, user_agent, ip, last_active_at, expires_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(crypto::hash_token(&token))
    .bind(user_id)
    .bind(org_id)
    .bind(&csrf)
    .bind(user_agent.map(|ua| ua.chars().take(256).collect::<String>()))
    .bind(ip)
    .bind(&current)
    .bind(ts(expires))
    .execute(&state.db)
    .await?;
    Ok((session_cookie(token, SESSION_DAYS, secure), csrf))
}

/// Revokes every existing browser session for `user_id` and mints a fresh
/// rotated session cookie + CSRF token (used after password and 2FA changes).
pub async fn rotate_sessions(
    state: &AppState,
    user_id: &str,
    org_id: &str,
    user_agent: Option<&str>,
    ip: Option<&str>,
    secure: bool,
) -> ApiResult<(Cookie<'static>, String)> {
    let token = crypto::random_token(32);
    let csrf = crypto::random_token(24);
    let current = now();
    let expires = chrono::Utc::now() + chrono::Duration::days(SESSION_DAYS);
    let mut tx = state.db.begin().await?;
    sqlx::query("DELETE FROM sessions WHERE user_id = ?")
        .bind(user_id)
        .execute(&mut *tx)
        .await?;
    sqlx::query(
        "INSERT INTO sessions (token_hash, user_id, org_id, csrf_token, user_agent, ip, last_active_at, expires_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(crypto::hash_token(&token))
    .bind(user_id)
    .bind(org_id)
    .bind(&csrf)
    .bind(user_agent.map(|ua| ua.chars().take(256).collect::<String>()))
    .bind(ip)
    .bind(&current)
    .bind(ts(expires))
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
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

/// Removes expired browser sessions, stale rate-limit rows, and expired 2FA
/// challenges so unauthenticated traffic cannot grow `rig.db` unboundedly.
pub async fn prune_expired(db: &sqlx::SqlitePool) {
    let current = now();
    let idle_cutoff = ts(chrono::Utc::now() - chrono::Duration::days(SESSION_IDLE_DAYS));
    let attempts_cutoff = chrono::Utc::now().timestamp() - WINDOW_SECS;
    if let Err(e) = sqlx::query(
        "DELETE FROM sessions WHERE expires_at <= ? OR (last_active_at IS NOT NULL AND last_active_at <= ?)",
    )
    .bind(&current)
    .bind(&idle_cutoff)
    .execute(db)
    .await
    {
        tracing::warn!("pruning expired sessions: {e}");
    }
    if let Err(e) = sqlx::query("DELETE FROM login_attempts WHERE ts < ?")
        .bind(attempts_cutoff)
        .execute(db)
        .await
    {
        tracing::warn!("pruning old login attempts: {e}");
    }
    if let Err(e) = sqlx::query("DELETE FROM pending_2fa WHERE expires_at <= ?")
        .bind(&current)
        .execute(db)
        .await
    {
        tracing::warn!("pruning expired 2fa challenges: {e}");
    }
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

#[cfg(test)]
mod tests {
    use std::net::Ipv4Addr;

    use super::*;

    #[test]
    fn real_ip_reads_last_entry_across_multiple_xff_headers_only_from_loopback() {
        let mut h = HeaderMap::new();
        h.append("x-forwarded-for", "1.1.1.1, 2.2.2.2".parse().unwrap());
        h.append("x-forwarded-for", "3.3.3.3, 4.4.4.4".parse().unwrap());

        let loopback = IpAddr::V4(Ipv4Addr::LOCALHOST);
        assert_eq!(real_ip(&h, loopback), "4.4.4.4".parse::<IpAddr>().unwrap());

        let remote = "203.0.113.9".parse::<IpAddr>().unwrap();
        assert_eq!(real_ip(&h, remote), remote);
    }

    #[test]
    fn origin_and_https_enforce_scheme_and_loopback_proxy_trust() {
        let loopback = Some(IpAddr::V4(Ipv4Addr::LOCALHOST));
        let remote = Some("203.0.113.9".parse::<IpAddr>().unwrap());

        let mut h = HeaderMap::new();
        h.insert(header::HOST, "rig.example.com".parse().unwrap());
        h.insert("x-forwarded-proto", "https".parse().unwrap());
        h.insert(header::ORIGIN, "http://rig.example.com".parse().unwrap());
        // From loopback proxy, X-Forwarded-Proto=https is trusted, so http:// Origin MUST fail.
        assert!(is_https_for(false, &h, loopback));
        assert!(check_origin_for(false, &h, loopback).is_err());

        h.insert(header::ORIGIN, "https://rig.example.com".parse().unwrap());
        assert!(check_origin_for(false, &h, loopback).is_ok());

        // From non-loopback peer, X-Forwarded-Proto and X-Forwarded-Host are NOT trusted.
        h.insert("x-forwarded-host", "evil.example.com".parse().unwrap());
        h.insert(header::ORIGIN, "https://evil.example.com".parse().unwrap());
        assert!(!is_https_for(false, &h, remote));
        assert!(check_origin_for(false, &h, remote).is_err());

        h.insert(header::ORIGIN, "http://rig.example.com".parse().unwrap());
        assert!(check_origin_for(false, &h, remote).is_ok());

        // With secure_cookies=true, HTTPS is always required even from remote peers.
        assert!(is_https_for(true, &h, remote));
        assert!(check_origin_for(true, &h, remote).is_err());
    }
}
