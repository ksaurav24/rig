//! The signed-in user's own second factor.

use axum::{Json, extract::State, http::StatusCode};
use rig_api::{
    ErrorBody, PasswordConfirm, RecoveryCodes, TotpCode, TotpDisable, TotpEnrollment, TotpStatus,
};
use sqlx::Row;
use utoipa_axum::{router::OpenApiRouter, routes};

use crate::{
    auth::{self, Auth, Via, Viewer},
    crypto,
    error::{ApiError, ApiResult},
    state::{AppState, now},
    totp,
};

pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(totp_status))
        .routes(routes!(totp_setup))
        .routes(routes!(totp_enable))
        .routes(routes!(totp_disable))
        .routes(routes!(totp_regenerate))
}

const MAX_CODE_FAILURES: i64 = 8;

/// Second-factor changes need a real browser session: an API token that
/// leaked must not be able to turn the account's protection off.
fn session_only(auth: &Auth<Viewer>) -> ApiResult<()> {
    match auth.via {
        Via::Session => Ok(()),
        Via::ApiToken => Err(ApiError::Forbidden(
            "sign in to the dashboard to change two-factor settings",
        )),
    }
}

fn recovery_hash(user_id: &str, code: &str) -> String {
    crypto::hash_token(&format!("{user_id}:{}", totp::normalize_recovery(code)))
}

async fn verify_password(state: &AppState, user_id: &str, password: String) -> ApiResult<()> {
    let stored: String = sqlx::query_scalar("SELECT password_hash FROM users WHERE id = ?")
        .bind(user_id)
        .fetch_one(&state.db)
        .await?;
    let ok = tokio::task::spawn_blocking(move || crypto::verify_password(&password, &stored))
        .await
        .map_err(|e| ApiError::Internal(e.into()))?;
    if ok {
        Ok(())
    } else {
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
    if totp::normalize_recovery(code).len() == 8 {
        let used = sqlx::query(
            "UPDATE recovery_codes SET used_at = ?
             WHERE user_id = ? AND code_hash = ? AND used_at IS NULL",
        )
        .bind(now())
        .bind(user_id)
        .bind(recovery_hash(user_id, code))
        .execute(&state.db)
        .await?
        .rows_affected();
        return Ok(used == 1);
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
            .bind(recovery_hash(user_id, code))
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    Ok(codes)
}

/// Signs out every other session of the user, keeping the current one.
async fn revoke_other_sessions(state: &AppState, auth: &Auth<Viewer>) -> ApiResult<()> {
    sqlx::query("DELETE FROM sessions WHERE user_id = ? AND csrf_token != ?")
        .bind(&auth.user_id)
        .bind(auth.csrf_token.as_deref().unwrap_or_default())
        .execute(&state.db)
        .await?;
    Ok(())
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
/// nothing changes until it is confirmed with a code.
#[utoipa::path(post, path = "/api/v1/account/totp/setup", tag = "account",
    request_body = PasswordConfirm,
    responses((status = 200, body = TotpEnrollment), (status = 409, body = ErrorBody),
              (status = 422, body = ErrorBody)))]
async fn totp_setup(
    State(state): State<AppState>,
    auth: Auth<Viewer>,
    Json(req): Json<PasswordConfirm>,
) -> ApiResult<Json<TotpEnrollment>> {
    session_only(&auth)?;
    verify_password(&state, &auth.user_id, req.password).await?;
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
    sqlx::query("UPDATE users SET totp_pending = ? WHERE id = ?")
        .bind(state.vault.encrypt(&secret)?)
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
/// signs out your other sessions and returns ten recovery codes, once.
#[utoipa::path(post, path = "/api/v1/account/totp/enable", tag = "account",
    request_body = TotpCode,
    responses((status = 200, body = RecoveryCodes), (status = 422, body = ErrorBody),
              (status = 429, body = ErrorBody)))]
async fn totp_enable(
    State(state): State<AppState>,
    auth: Auth<Viewer>,
    Json(req): Json<TotpCode>,
) -> ApiResult<Json<RecoveryCodes>> {
    session_only(&auth)?;
    let key = format!("totp:user:{}", auth.user_id);
    auth::check_rate(&state, &key, MAX_CODE_FAILURES).await?;
    let pending: Option<String> = sqlx::query_scalar("SELECT totp_pending FROM users WHERE id = ?")
        .bind(&auth.user_id)
        .fetch_one(&state.db)
        .await?;
    let pending = pending.ok_or_else(|| ApiError::validation("start the setup first"))?;
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
    revoke_other_sessions(&state, &auth).await?;
    auth.audit(&state, "auth.totp_enabled", None, None).await?;
    Ok(Json(RecoveryCodes { codes }))
}

/// Turn two-factor sign-in off. Needs your password and a current code
/// (or a recovery code), and signs out your other sessions.
#[utoipa::path(post, path = "/api/v1/account/totp/disable", tag = "account",
    request_body = TotpDisable,
    responses((status = 204), (status = 422, body = ErrorBody), (status = 429, body = ErrorBody)))]
async fn totp_disable(
    State(state): State<AppState>,
    auth: Auth<Viewer>,
    Json(req): Json<TotpDisable>,
) -> ApiResult<StatusCode> {
    session_only(&auth)?;
    verify_password(&state, &auth.user_id, req.password).await?;
    let secret: Option<String> = sqlx::query_scalar("SELECT totp_secret FROM users WHERE id = ?")
        .bind(&auth.user_id)
        .fetch_one(&state.db)
        .await?;
    let secret = secret.ok_or_else(|| ApiError::validation("two-factor sign-in is not on"))?;
    if !guarded_check(&state, &auth.user_id, &secret, &req.code).await? {
        return Err(ApiError::validation("that code did not match"));
    }
    disable_for_user(&state, &auth.user_id).await?;
    revoke_other_sessions(&state, &auth).await?;
    auth.audit(&state, "auth.totp_disabled", None, None).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Replace your recovery codes. The old ones stop working.
#[utoipa::path(post, path = "/api/v1/account/totp/recovery-codes", tag = "account",
    request_body = TotpDisable,
    responses((status = 200, body = RecoveryCodes), (status = 422, body = ErrorBody)))]
async fn totp_regenerate(
    State(state): State<AppState>,
    auth: Auth<Viewer>,
    Json(req): Json<TotpDisable>,
) -> ApiResult<Json<RecoveryCodes>> {
    session_only(&auth)?;
    verify_password(&state, &auth.user_id, req.password).await?;
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
