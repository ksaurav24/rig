//! The signed-in user's own second factor.

use std::net::SocketAddr;

use axum::{
    Json,
    extract::{ConnectInfo, Path, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
};
use axum_extra::extract::CookieJar;
use rig_api::{
    ErrorBody, PasswordConfirm, RecoveryCodes, SessionInfo, TotpCode, TotpDisable, TotpEnrollment,
    TotpStatus, UpdateProfileRequest, UploadImageRequest, User,
};
use sqlx::Row;
use utoipa_axum::{router::OpenApiRouter, routes};

use super::{
    auth::validate_name,
    org::{
        delete_media_if_owned, store_media, validate_and_decode_image, validate_avatar_preset,
        validate_external_image_url,
    },
};
use crate::{
    auth::{self, Auth, CSRF_HEADER, Viewer},
    crypto,
    error::{ApiError, ApiResult},
    state::{AppState, now, ts},
    totp,
};

pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(update_profile))
        .routes(routes!(upload_avatar, delete_avatar))
        .routes(routes!(list_sessions))
        .routes(routes!(revoke_session))
        .routes(routes!(totp_status))
        .routes(routes!(totp_setup))
        .routes(routes!(totp_enable))
        .routes(routes!(totp_disable))
        .routes(routes!(totp_regenerate))
}

async fn load_user(state: &AppState, user_id: &str) -> ApiResult<User> {
    let row = sqlx::query("SELECT id, email, name, avatar_url FROM users WHERE id = ?")
        .bind(user_id)
        .fetch_one(&state.db)
        .await?;
    Ok(User {
        id: row.get("id"),
        email: row.get("email"),
        name: row.get("name"),
        avatar_url: row.get("avatar_url"),
    })
}

/// Update your own display name or avatar (built-in preset, image URL, or empty to reset).
#[utoipa::path(patch, path = "/api/v1/account/profile", tag = "account",
    request_body = UpdateProfileRequest,
    responses((status = 200, body = User), (status = 422, body = ErrorBody)))]
async fn update_profile(
    State(state): State<AppState>,
    auth: Auth<Viewer>,
    Json(req): Json<UpdateProfileRequest>,
) -> ApiResult<Json<User>> {
    let current = load_user(&state, &auth.user_id).await?;

    let next_name = match req.name {
        Some(n) => validate_name("name", &n, 80)?,
        None => current.name.clone(),
    };

    let (next_avatar, avatar_changed) = match req.avatar_url {
        None => (current.avatar_url.clone(), false),
        Some(ref s) if s.trim().is_empty() => (None, true),
        Some(ref s) if s.trim().starts_with("preset:") => {
            (Some(validate_avatar_preset(s)?), true)
        }
        Some(ref s) => (Some(validate_external_image_url(s)?), true),
    };

    let mut tx = state.db.begin().await?;
    sqlx::query("UPDATE users SET name = ?, avatar_url = ? WHERE id = ?")
        .bind(&next_name)
        .bind(&next_avatar)
        .bind(&auth.user_id)
        .execute(&mut *tx)
        .await?;
    if avatar_changed && current.avatar_url != next_avatar {
        delete_media_if_owned(&mut tx, &auth.org_id, current.avatar_url.as_deref()).await?;
    }
    tx.commit().await?;

    auth.audit(&state, "account.profile_update", None, None)
        .await?;
    Ok(Json(load_user(&state, &auth.user_id).await?))
}

/// Upload a custom avatar image for your account.
#[utoipa::path(post, path = "/api/v1/account/avatar", tag = "account",
    request_body = UploadImageRequest,
    responses((status = 200, body = User), (status = 422, body = ErrorBody)))]
async fn upload_avatar(
    State(state): State<AppState>,
    auth: Auth<Viewer>,
    Json(req): Json<UploadImageRequest>,
) -> ApiResult<Json<User>> {
    let (content_type, bytes) = validate_and_decode_image(&req)?;
    let current = load_user(&state, &auth.user_id).await?;

    let mut tx = state.db.begin().await?;
    let url = store_media(&mut tx, &auth.org_id, &content_type, &bytes, false).await?;
    sqlx::query("UPDATE users SET avatar_url = ? WHERE id = ?")
        .bind(&url)
        .bind(&auth.user_id)
        .execute(&mut *tx)
        .await?;
    delete_media_if_owned(&mut tx, &auth.org_id, current.avatar_url.as_deref()).await?;
    tx.commit().await?;

    auth.audit(&state, "account.avatar_upload", None, Some(&content_type))
        .await?;
    Ok(Json(load_user(&state, &auth.user_id).await?))
}

/// Remove your custom avatar and reset to initials.
#[utoipa::path(delete, path = "/api/v1/account/avatar", tag = "account",
    responses((status = 200, body = User)))]
async fn delete_avatar(
    State(state): State<AppState>,
    auth: Auth<Viewer>,
) -> ApiResult<Json<User>> {
    let current = load_user(&state, &auth.user_id).await?;
    let mut tx = state.db.begin().await?;
    sqlx::query("UPDATE users SET avatar_url = NULL WHERE id = ?")
        .bind(&auth.user_id)
        .execute(&mut *tx)
        .await?;
    delete_media_if_owned(&mut tx, &auth.org_id, current.avatar_url.as_deref()).await?;
    tx.commit().await?;

    auth.audit(&state, "account.avatar_remove", None, None)
        .await?;
    Ok(Json(load_user(&state, &auth.user_id).await?))
}

const MAX_CODE_FAILURES: i64 = 8;
const MAX_PASSWORD_FAILURES: i64 = 8;
const TOTP_PENDING_TTL_MINS: i64 = 15;

/// Second-factor changes need an interactive browser session or paired desktop/mobile
/// device token (plus password/code confirmation); non-interactive API/CI tokens are blocked.
fn interactive_only(auth: &Auth<Viewer>) -> ApiResult<()> {
    auth.require_interactive("sign in to the dashboard or Rig Desktop to change two-factor settings")
}

/// Vault-keyed HMAC-SHA256 hash of a recovery code (`v2:<hex>`), defeating
/// offline brute-force if only `rig.db` leaks without `master.key`.
fn recovery_hash_v2(state: &AppState, user_id: &str, code: &str) -> String {
    let payload = format!("{user_id}:{}", totp::normalize_recovery(code));
    format!(
        "v2:{}",
        state.vault.hmac_sha256("totp-recovery", payload.as_bytes())
    )
}

/// Legacy unsalted SHA-256 format kept only to verify codes generated prior
/// to the v2 upgrade.
fn recovery_hash_legacy(user_id: &str, code: &str) -> String {
    crypto::hash_token(&format!("{user_id}:{}", totp::normalize_recovery(code)))
}

async fn verify_password(state: &AppState, auth: &Auth<Viewer>, password: String) -> ApiResult<()> {
    let rate_key = format!("password:{}", auth.user_id);
    auth::check_rate(state, &rate_key, MAX_PASSWORD_FAILURES).await?;
    let stored: String = sqlx::query_scalar("SELECT password_hash FROM users WHERE id = ?")
        .bind(&auth.user_id)
        .fetch_one(&state.db)
        .await?;
    let ok = tokio::task::spawn_blocking(move || crypto::verify_password(&password, &stored))
        .await
        .map_err(|e| ApiError::Internal(e.into()))?;
    if ok {
        auth::clear_failures(state, &rate_key).await?;
        Ok(())
    } else {
        auth::record_failure(state, &rate_key).await?;
        auth.audit(state, "auth.password_confirm_failed", None, None)
            .await?;
        Err(ApiError::validation("that password is not correct"))
    }
}

/// Checks an authenticator or recovery code for a user who has TOTP on.
/// A TOTP code is accepted once per time step; a recovery code once ever.
pub(crate) async fn check_code(
    state: &AppState,
    user_id: &str,
    encrypted_secret: &str,
    code: &str,
) -> ApiResult<bool> {
    let secret = state.vault.decrypt(encrypted_secret)?;
    let last: Option<i64> = sqlx::query_scalar("SELECT totp_last_step FROM users WHERE id = ?")
        .bind(user_id)
        .fetch_one(&state.db)
        .await?;
    if let Some(step) = totp::verify(&secret, code, chrono::Utc::now().timestamp(), last) {
        // The condition makes two concurrent logins with the same code race
        // for one row update; only one wins.
        let won = sqlx::query(
            "UPDATE users SET totp_last_step = ?
             WHERE id = ? AND (totp_last_step IS NULL OR totp_last_step < ?)",
        )
        .bind(step)
        .bind(user_id)
        .bind(step)
        .execute(&state.db)
        .await?
        .rows_affected();
        return Ok(won == 1);
    }
    let normalized = totp::normalize_recovery(code);
    if normalized.len() == 16 || normalized.len() == 8 {
        let used_v2 = sqlx::query(
            "UPDATE recovery_codes SET used_at = ?
             WHERE user_id = ? AND code_hash = ? AND used_at IS NULL",
        )
        .bind(now())
        .bind(user_id)
        .bind(recovery_hash_v2(state, user_id, code))
        .execute(&state.db)
        .await?
        .rows_affected();
        if used_v2 == 1 {
            return Ok(true);
        }
        if normalized.len() == 8 {
            let used_legacy = sqlx::query(
                "UPDATE recovery_codes SET used_at = ?
                 WHERE user_id = ? AND code_hash = ? AND used_at IS NULL",
            )
            .bind(now())
            .bind(user_id)
            .bind(recovery_hash_legacy(user_id, code))
            .execute(&state.db)
            .await?
            .rows_affected();
            return Ok(used_legacy == 1);
        }
    }
    Ok(false)
}

/// Rate-limits code guesses per account, separately from password ones.
async fn guarded_check(
    state: &AppState,
    user_id: &str,
    encrypted_secret: &str,
    code: &str,
) -> ApiResult<bool> {
    let key = format!("totp:user:{user_id}");
    auth::check_rate(state, &key, MAX_CODE_FAILURES).await?;
    let ok = check_code(state, user_id, encrypted_secret, code).await?;
    if ok {
        auth::clear_failures(state, &key).await?;
    } else {
        auth::record_failure(state, &key).await?;
    }
    Ok(ok)
}

async fn store_recovery_codes(state: &AppState, user_id: &str) -> ApiResult<Vec<String>> {
    let codes = totp::recovery_codes();
    let mut tx = state.db.begin().await?;
    sqlx::query("DELETE FROM recovery_codes WHERE user_id = ?")
        .bind(user_id)
        .execute(&mut *tx)
        .await?;
    for code in &codes {
        sqlx::query("INSERT INTO recovery_codes (user_id, code_hash) VALUES (?, ?)")
            .bind(user_id)
            .bind(recovery_hash_v2(state, user_id, code))
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    Ok(codes)
}

fn csrf_headers(csrf: &str) -> HeaderMap {
    let mut out = HeaderMap::new();
    if let Ok(v) = HeaderValue::from_str(csrf) {
        out.insert(CSRF_HEADER, v);
    }
    out
}

fn user_agent(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(header::USER_AGENT)
        .and_then(|v| v.to_str().ok())
}

/// Active browser sessions for your account.
#[utoipa::path(get, path = "/api/v1/account/sessions", tag = "account",
    responses((status = 200, body = Vec<SessionInfo>), (status = 403, body = ErrorBody)))]
async fn list_sessions(
    State(state): State<AppState>,
    auth: Auth<Viewer>,
) -> ApiResult<Json<Vec<SessionInfo>>> {
    auth.require_session("sign in to the dashboard to view active sessions")?;
    let rows = sqlx::query(
        "SELECT token_hash, user_agent, ip, created_at, last_active_at, expires_at
         FROM sessions
         WHERE user_id = ? AND expires_at > ?
         ORDER BY created_at DESC",
    )
    .bind(&auth.user_id)
    .bind(now())
    .fetch_all(&state.db)
    .await?;
    Ok(Json(
        rows.iter()
            .map(|r| {
                let hash: String = r.get("token_hash");
                SessionInfo {
                    id: hash.chars().take(16).collect(),
                    user_agent: r.get("user_agent"),
                    ip: r.get("ip"),
                    created_at: r.get("created_at"),
                    last_active_at: r.get("last_active_at"),
                    expires_at: r.get("expires_at"),
                    current: hash == auth.credential_hash,
                }
            })
            .collect(),
    ))
}

/// Revoke one of your browser sessions by its short id (cannot revoke the current session; use logout).
#[utoipa::path(delete, path = "/api/v1/account/sessions/{id}", tag = "account",
    params(("id" = String, Path)),
    responses((status = 204), (status = 403, body = ErrorBody), (status = 404, body = ErrorBody)))]
async fn revoke_session(
    State(state): State<AppState>,
    auth: Auth<Viewer>,
    Path(id): Path<String>,
) -> ApiResult<StatusCode> {
    auth.require_session("sign in to the dashboard to revoke sessions")?;
    if id.len() != 16 || !id.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(ApiError::NotFound("session"));
    }
    let prefix = format!("{id}%");
    let deleted = sqlx::query(
        "DELETE FROM sessions WHERE user_id = ? AND token_hash LIKE ? AND token_hash != ?",
    )
    .bind(&auth.user_id)
    .bind(&prefix)
    .bind(&auth.credential_hash)
    .execute(&state.db)
    .await?
    .rows_affected();
    if deleted == 0 {
        return Err(ApiError::NotFound("session"));
    }
    auth.audit(&state, "auth.session_revoked", Some(&id), None)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Whether two-factor sign-in is on for you.
#[utoipa::path(get, path = "/api/v1/account/totp", tag = "account",
    responses((status = 200, body = TotpStatus)))]
async fn totp_status(
    State(state): State<AppState>,
    auth: Auth<Viewer>,
) -> ApiResult<Json<TotpStatus>> {
    let row = sqlx::query(
        "SELECT totp_enabled_at,
                (SELECT COUNT(*) FROM recovery_codes r
                  WHERE r.user_id = u.id AND r.used_at IS NULL) AS left_count
         FROM users u WHERE id = ?",
    )
    .bind(&auth.user_id)
    .fetch_one(&state.db)
    .await?;
    let enabled_at: Option<String> = row.get("totp_enabled_at");
    Ok(Json(TotpStatus {
        enabled: enabled_at.is_some(),
        enabled_at,
        recovery_codes_left: row.get("left_count"),
    }))
}

/// Start turning on two-factor sign-in. Returns a secret and its QR code;
/// nothing changes until it is confirmed with a code within 15 minutes.
#[utoipa::path(post, path = "/api/v1/account/totp/setup", tag = "account",
    request_body = PasswordConfirm,
    responses((status = 200, body = TotpEnrollment), (status = 409, body = ErrorBody),
              (status = 422, body = ErrorBody), (status = 429, body = ErrorBody)))]
async fn totp_setup(
    State(state): State<AppState>,
    auth: Auth<Viewer>,
    Json(req): Json<PasswordConfirm>,
) -> ApiResult<Json<TotpEnrollment>> {
    interactive_only(&auth)?;
    verify_password(&state, &auth, req.password).await?;
    let enabled: Option<String> =
        sqlx::query_scalar("SELECT totp_enabled_at FROM users WHERE id = ?")
            .bind(&auth.user_id)
            .fetch_one(&state.db)
            .await?;
    if enabled.is_some() {
        return Err(ApiError::Conflict(
            "two-factor sign-in is already on; turn it off first to change authenticators".into(),
        ));
    }
    let secret = totp::new_secret();
    let expires = ts(chrono::Utc::now() + chrono::Duration::minutes(TOTP_PENDING_TTL_MINS));
    sqlx::query("UPDATE users SET totp_pending = ?, totp_pending_expires_at = ? WHERE id = ?")
        .bind(state.vault.encrypt(&secret)?)
        .bind(&expires)
        .bind(&auth.user_id)
        .execute(&state.db)
        .await?;
    let uri = totp::otpauth_uri("Rig", &auth.email, &secret);
    let qr_svg = totp::qr_svg(&uri)?;
    Ok(Json(TotpEnrollment {
        secret,
        otpauth_uri: uri,
        qr_svg,
    }))
}

/// Confirm the new authenticator with a code. Turns two-factor sign-in on,
/// rotates the current session, signs out other sessions and returns ten recovery codes, once.
#[utoipa::path(post, path = "/api/v1/account/totp/enable", tag = "account",
    request_body = TotpCode,
    responses((status = 200, body = RecoveryCodes), (status = 422, body = ErrorBody),
              (status = 429, body = ErrorBody)))]
async fn totp_enable(
    State(state): State<AppState>,
    auth: Auth<Viewer>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    jar: CookieJar,
    Json(req): Json<TotpCode>,
) -> ApiResult<(CookieJar, HeaderMap, Json<RecoveryCodes>)> {
    interactive_only(&auth)?;
    let key = format!("totp:user:{}", auth.user_id);
    auth::check_rate(&state, &key, MAX_CODE_FAILURES).await?;
    let row = sqlx::query(
        "SELECT totp_pending, totp_pending_expires_at FROM users WHERE id = ?",
    )
    .bind(&auth.user_id)
    .fetch_one(&state.db)
    .await?;
    let pending: Option<String> = row.get("totp_pending");
    let expires: Option<String> = row.get("totp_pending_expires_at");
    let pending = pending.ok_or_else(|| ApiError::validation("start the setup first"))?;
    if expires.as_deref().is_some_and(|exp| exp <= now().as_str()) {
        let _ = sqlx::query(
            "UPDATE users SET totp_pending = NULL, totp_pending_expires_at = NULL WHERE id = ?",
        )
        .bind(&auth.user_id)
        .execute(&state.db)
        .await;
        return Err(ApiError::validation(
            "the setup expired; start two-factor setup again",
        ));
    }
    let secret = state.vault.decrypt(&pending)?;
    let Some(step) = totp::verify(&secret, &req.code, chrono::Utc::now().timestamp(), None) else {
        auth::record_failure(&state, &key).await?;
        return Err(ApiError::validation(
            "that code did not match; check the time on your phone and try the next code",
        ));
    };
    auth::clear_failures(&state, &key).await?;
    let done = sqlx::query(
        "UPDATE users SET totp_secret = totp_pending, totp_pending = NULL,
                          totp_pending_expires_at = NULL,
                          totp_enabled_at = ?, totp_last_step = ?
         WHERE id = ? AND totp_pending = ? AND totp_enabled_at IS NULL",
    )
    .bind(now())
    .bind(step)
    .bind(&auth.user_id)
    .bind(&pending)
    .execute(&state.db)
    .await?
    .rows_affected();
    if done != 1 {
        return Err(ApiError::Conflict("the setup changed; start again".into()));
    }
    let codes = store_recovery_codes(&state, &auth.user_id).await?;
    let (jar, out_headers) = if auth.via == auth::Via::Session {
        let (cookie, csrf) = auth::rotate_sessions(
            &state,
            &auth.user_id,
            &auth.org_id,
            user_agent(&headers),
            auth.ip.as_deref(),
            auth::is_https(&state, &headers, Some(addr.ip())),
        )
        .await?;
        (jar.add(cookie), csrf_headers(&csrf))
    } else {
        sqlx::query("DELETE FROM sessions WHERE user_id = ?")
            .bind(&auth.user_id)
            .execute(&state.db)
            .await?;
        (jar, HeaderMap::new())
    };
    auth.audit(&state, "auth.totp_enabled", None, None).await?;
    Ok((jar, out_headers, Json(RecoveryCodes { codes })))
}

/// Turn two-factor sign-in off. Needs your password and a current code
/// (or a recovery code), rotates the current session and signs out other sessions.
#[utoipa::path(post, path = "/api/v1/account/totp/disable", tag = "account",
    request_body = TotpDisable,
    responses((status = 204), (status = 422, body = ErrorBody), (status = 429, body = ErrorBody)))]
async fn totp_disable(
    State(state): State<AppState>,
    auth: Auth<Viewer>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    jar: CookieJar,
    Json(req): Json<TotpDisable>,
) -> ApiResult<(CookieJar, HeaderMap, StatusCode)> {
    interactive_only(&auth)?;
    verify_password(&state, &auth, req.password).await?;
    let secret: Option<String> = sqlx::query_scalar("SELECT totp_secret FROM users WHERE id = ?")
        .bind(&auth.user_id)
        .fetch_one(&state.db)
        .await?;
    let secret = secret.ok_or_else(|| ApiError::validation("two-factor sign-in is not on"))?;
    if !guarded_check(&state, &auth.user_id, &secret, &req.code).await? {
        return Err(ApiError::validation("that code did not match"));
    }
    disable_for_user(&state, &auth.user_id).await?;
    let (jar, out_headers) = if auth.via == auth::Via::Session {
        let (cookie, csrf) = auth::rotate_sessions(
            &state,
            &auth.user_id,
            &auth.org_id,
            user_agent(&headers),
            auth.ip.as_deref(),
            auth::is_https(&state, &headers, Some(addr.ip())),
        )
        .await?;
        (jar.add(cookie), csrf_headers(&csrf))
    } else {
        sqlx::query("DELETE FROM sessions WHERE user_id = ?")
            .bind(&auth.user_id)
            .execute(&state.db)
            .await?;
        (jar, HeaderMap::new())
    };
    auth.audit(&state, "auth.totp_disabled", None, None).await?;
    Ok((jar, out_headers, StatusCode::NO_CONTENT))
}

/// Replace your recovery codes. The old ones stop working.
#[utoipa::path(post, path = "/api/v1/account/totp/recovery-codes", tag = "account",
    request_body = TotpDisable,
    responses((status = 200, body = RecoveryCodes), (status = 422, body = ErrorBody), (status = 429, body = ErrorBody)))]
async fn totp_regenerate(
    State(state): State<AppState>,
    auth: Auth<Viewer>,
    Json(req): Json<TotpDisable>,
) -> ApiResult<Json<RecoveryCodes>> {
    interactive_only(&auth)?;
    verify_password(&state, &auth, req.password).await?;
    let secret: Option<String> = sqlx::query_scalar("SELECT totp_secret FROM users WHERE id = ?")
        .bind(&auth.user_id)
        .fetch_one(&state.db)
        .await?;
    let secret = secret.ok_or_else(|| ApiError::validation("two-factor sign-in is not on"))?;
    if !guarded_check(&state, &auth.user_id, &secret, &req.code).await? {
        return Err(ApiError::validation("that code did not match"));
    }
    let codes = store_recovery_codes(&state, &auth.user_id).await?;
    auth.audit(&state, "auth.recovery_codes_regenerated", None, None)
        .await?;
    Ok(Json(RecoveryCodes { codes }))
}

pub(crate) async fn disable_for_user(state: &AppState, user_id: &str) -> ApiResult<()> {
    let mut tx = state.db.begin().await?;
    sqlx::query(
        "UPDATE users SET totp_secret = NULL, totp_pending = NULL,
                          totp_pending_expires_at = NULL,
                          totp_enabled_at = NULL, totp_last_step = NULL WHERE id = ?",
    )
    .bind(user_id)
    .execute(&mut *tx)
    .await?;
    sqlx::query("DELETE FROM recovery_codes WHERE user_id = ?")
        .bind(user_id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(())
}
