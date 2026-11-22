//! First-run setup token: single use, short lived, only its hash stored.

use std::sync::OnceLock;

use anyhow::Result;

use crate::{
    crypto,
    error::ApiResult,
    state::{AppState, now, ts},
};

const TOKEN_MINUTES: i64 = 30;

pub async fn is_complete(state: &AppState) -> ApiResult<bool> {
    let v: Option<String> =
        sqlx::query_scalar("SELECT value FROM settings WHERE key = 'setup_complete'")
            .fetch_optional(&state.db)
            .await?;
    Ok(v.is_some())
}

/// On an instance with no owner yet, mints a fresh setup token (replacing any
/// earlier one) and returns it so the caller can show it on the console.
pub async fn issue_token_if_needed(db: &sqlx::SqlitePool) -> Result<Option<String>> {
    let done: Option<String> =
        sqlx::query_scalar("SELECT value FROM settings WHERE key = 'setup_complete'")
            .fetch_optional(db)
            .await?;
    if done.is_some() {
        return Ok(None);
    }
    let token = crypto::random_token(24);
    let expires = ts(chrono::Utc::now() + chrono::Duration::minutes(TOKEN_MINUTES));
    let mut tx = db.begin().await?;
    for (k, v) in [
        ("setup_token_hash", crypto::hash_token(&token)),
        ("setup_token_expires", expires),
    ] {
        sqlx::query(
            "INSERT INTO settings (key, value) VALUES (?, ?)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        )
        .bind(k)
        .bind(v)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(Some(token))
}

pub async fn token_matches(state: &AppState, candidate: &str) -> ApiResult<bool> {
    let hash: Option<String> =
        sqlx::query_scalar("SELECT value FROM settings WHERE key = 'setup_token_hash'")
            .fetch_optional(&state.db)
            .await?;
    let expires: Option<String> =
        sqlx::query_scalar("SELECT value FROM settings WHERE key = 'setup_token_expires'")
            .fetch_optional(&state.db)
            .await?;
    let (Some(hash), Some(expires)) = (hash, expires) else {
        return Ok(false);
    };
    Ok(expires > now() && crypto::constant_time_eq(&crypto::hash_token(candidate), &hash))
}

/// A real Argon2 hash of a random password, verified against when the
/// account does not exist so both login paths cost the same.
pub fn dummy_hash() -> &'static str {
    static HASH: OnceLock<String> = OnceLock::new();
    HASH.get_or_init(|| crypto::hash_password(&crypto::random_token(16)).expect("argon2 hash"))
}
