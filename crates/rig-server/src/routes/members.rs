//! Team members and their roles, and changing your own password.
//!
//! Rig sends no email, so adding a member returns a temporary password for
//! the admin to hand over. Rules that keep the role ladder honest:
//! - you cannot grant a role above your own;
//! - only an owner can change or remove an admin or owner;
//! - you cannot change or remove yourself, so an organization always keeps
//!   at least one owner.

use std::net::SocketAddr;

use axum::{
    Json,
    extract::{ConnectInfo, Path, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
};
use axum_extra::extract::CookieJar;
use rig_api::{
    AddMemberRequest, ChangePasswordRequest, ErrorBody, Member, MemberWithPassword, Role,
    UpdateMemberRequest,
};
use sqlx::Row;
use utoipa_axum::{router::OpenApiRouter, routes};

use super::auth::{validate_email, validate_name, validate_password};
use crate::{
    auth::{self, Admin, Auth, CSRF_HEADER, Viewer},
    crypto,
    error::{ApiError, ApiResult},
    state::{AppState, new_id, now, ts},
};

pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(list_members, add_member))
        .routes(routes!(update_member, remove_member))
        .routes(routes!(reset_password))
        .routes(routes!(change_password))
}

const MAX_PASSWORD_FAILURES: i64 = 8;
pub const TEMP_PASSWORD_TTL_DAYS: i64 = 7;

fn member_from_row(r: &sqlx::sqlite::SqliteRow) -> Member {
    Member {
        user_id: r.get("user_id"),
        name: r.get("name"),
        email: r.get("email"),
        avatar_url: r.get("avatar_url"),
        role: Role::parse(r.get("role")).unwrap_or(Role::Viewer),
        joined_at: r.get("joined_at"),
        last_login_at: r.get("last_login_at"),
    }
}

/// Members of an organization, or just one of them.
async fn members(state: &AppState, org_id: &str, user_id: Option<&str>) -> ApiResult<Vec<Member>> {
    let rows = sqlx::query(
        "SELECT u.id AS user_id, u.name, u.email, u.avatar_url, m.role, m.created_at AS joined_at, u.last_login_at
           FROM memberships m JOIN users u ON u.id = m.user_id
          WHERE m.org_id = ?1 AND (?2 IS NULL OR m.user_id = ?2)
          ORDER BY m.created_at",
    )
    .bind(org_id)
    .bind(user_id)
    .fetch_all(&state.db)
    .await?;
    Ok(rows.iter().map(member_from_row).collect())
}

async fn load_member(state: &AppState, org_id: &str, user_id: &str) -> ApiResult<Member> {
    members(state, org_id, Some(user_id))
        .await?
        .pop()
        .ok_or(ApiError::NotFound("member"))
}

/// Whether `actor` may change, reset or remove `target`.
fn check_can_manage(actor: Role, actor_id: &str, target: &Member) -> ApiResult<()> {
    if target.user_id == actor_id {
        return Err(ApiError::Forbidden(
            "you cannot change your own membership. Ask another owner or admin",
        ));
    }
    if actor != Role::Owner && target.role >= Role::Admin {
        return Err(ApiError::Forbidden(
            "only an owner can change admins and owners",
        ));
    }
    Ok(())
}

fn check_can_grant(actor: Role, role: Role) -> ApiResult<()> {
    if role > actor {
        return Err(ApiError::Forbidden(
            "you cannot grant a role higher than your own",
        ));
    }
    Ok(())
}

async fn hash(password: String) -> ApiResult<String> {
    Ok(
        tokio::task::spawn_blocking(move || crypto::hash_password(&password))
            .await
            .map_err(|e| ApiError::Internal(e.into()))??,
    )
}

/// Everyone in this organization.
#[utoipa::path(get, path = "/api/v1/members", tag = "members",
    responses((status = 200, body = Vec<Member>)))]
async fn list_members(
    State(state): State<AppState>,
    auth: Auth<Admin>,
) -> ApiResult<Json<Vec<Member>>> {
    Ok(Json(members(&state, &auth.org_id, None).await?))
}

async fn load_member_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    org_id: &str,
    user_id: &str,
) -> ApiResult<Member> {
    let row = sqlx::query(
        "SELECT u.id AS user_id, u.name, u.email, u.avatar_url, m.role, m.created_at AS joined_at, u.last_login_at
           FROM memberships m JOIN users u ON u.id = m.user_id
          WHERE m.org_id = ? AND m.user_id = ?",
    )
    .bind(org_id)
    .bind(user_id)
    .fetch_optional(&mut **tx)
    .await?
    .ok_or(ApiError::NotFound("member"))?;
    Ok(member_from_row(&row))
}

async fn actor_role_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    org_id: &str,
    user_id: &str,
) -> ApiResult<Role> {
    let role: Option<String> =
        sqlx::query_scalar("SELECT role FROM memberships WHERE org_id = ? AND user_id = ?")
            .bind(org_id)
            .bind(user_id)
            .fetch_optional(&mut **tx)
            .await?;
    role.and_then(|r| Role::parse(&r))
        .ok_or(ApiError::Forbidden("your role does not allow this action"))
}

/// Add a person. Returns a temporary password (valid 7 days, must be changed on first sign-in) once.
#[utoipa::path(post, path = "/api/v1/members", tag = "members", request_body = AddMemberRequest,
    responses((status = 200, body = MemberWithPassword), (status = 403, body = ErrorBody),
              (status = 409, body = ErrorBody), (status = 422, body = ErrorBody)))]
async fn add_member(
    State(state): State<AppState>,
    auth: Auth<Admin>,
    Json(req): Json<AddMemberRequest>,
) -> ApiResult<Json<MemberWithPassword>> {
    auth.require_interactive("sign in to the dashboard or Rig Desktop to manage team members")?;
    let name = validate_name("name", &req.name, 80)?;
    let email = validate_email(&req.email)?;
    check_can_grant(auth.role, req.role)?;

    let temporary_password = crypto::random_token(15);
    let password_hash = hash(temporary_password.clone()).await?;
    let (user_id, created_at) = (new_id(), now());
    let temp_expires = ts(chrono::Utc::now() + chrono::Duration::days(TEMP_PASSWORD_TTL_DAYS));

    let mut tx = state.db.begin_with("BEGIN IMMEDIATE").await?;
    let actor_role = actor_role_tx(&mut tx, &auth.org_id, &auth.user_id).await?;
    check_can_grant(actor_role, req.role)?;
    let inserted = sqlx::query(
        "INSERT INTO users (id, email, name, password_hash, must_change_password, temp_password_expires_at, created_at)
         VALUES (?, ?, ?, ?, 1, ?, ?)
         ON CONFLICT (email) DO NOTHING",
    )
    .bind(&user_id)
    .bind(&email)
    .bind(&name)
    .bind(&password_hash)
    .bind(&temp_expires)
    .bind(&created_at)
    .execute(&mut *tx)
    .await?;
    if inserted.rows_affected() == 0 {
        return Err(ApiError::Conflict(format!(
            "{email} already has an account"
        )));
    }
    sqlx::query("INSERT INTO memberships (org_id, user_id, role, created_at) VALUES (?, ?, ?, ?)")
        .bind(&auth.org_id)
        .bind(&user_id)
        .bind(req.role.as_str())
        .bind(&created_at)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;

    auth.audit(
        &state,
        "member.add",
        Some(&user_id),
        Some(&format!("{email} as {}", req.role.as_str())),
    )
    .await?;
    let member = load_member(&state, &auth.org_id, &user_id).await?;
    Ok(Json(MemberWithPassword {
        member,
        temporary_password,
    }))
}

/// Change a member's role.
#[utoipa::path(patch, path = "/api/v1/members/{user_id}", tag = "members", params(("user_id" = String, Path)), request_body = UpdateMemberRequest,
    responses((status = 200, body = Member), (status = 403, body = ErrorBody), (status = 404, body = ErrorBody)))]
async fn update_member(
    State(state): State<AppState>,
    auth: Auth<Admin>,
    Path(user_id): Path<String>,
    Json(req): Json<UpdateMemberRequest>,
) -> ApiResult<Json<Member>> {
    auth.require_interactive("sign in to the dashboard or Rig Desktop to manage team members")?;

    let mut tx = state.db.begin_with("BEGIN IMMEDIATE").await?;
    let actor_role = actor_role_tx(&mut tx, &auth.org_id, &auth.user_id).await?;
    if actor_role < Role::Admin {
        return Err(ApiError::Forbidden("your role does not allow this action"));
    }
    let target = load_member_tx(&mut tx, &auth.org_id, &user_id).await?;
    check_can_manage(actor_role, &auth.user_id, &target)?;
    check_can_grant(actor_role, req.role)?;

    sqlx::query("UPDATE memberships SET role = ? WHERE org_id = ? AND user_id = ?")
        .bind(req.role.as_str())
        .bind(&auth.org_id)
        .bind(&user_id)
        .execute(&mut *tx)
        .await?;
    // A token never outranks its owner at request time, but lower stored
    // roles keep the token list truthful after a demotion.
    for higher in [Role::Owner, Role::Admin, Role::Developer] {
        if higher > req.role {
            sqlx::query(
                "UPDATE api_tokens SET role = ? WHERE org_id = ? AND user_id = ? AND role = ?",
            )
            .bind(req.role.as_str())
            .bind(&auth.org_id)
            .bind(&user_id)
            .bind(higher.as_str())
            .execute(&mut *tx)
            .await?;
        }
    }
    ensure_owner_remains(&mut tx, &auth.org_id).await?;
    tx.commit().await?;

    auth.audit(
        &state,
        "member.role",
        Some(&user_id),
        Some(&format!(
            "{}: {} to {}",
            target.email,
            target.role.as_str(),
            req.role.as_str()
        )),
    )
    .await?;
    Ok(Json(load_member(&state, &auth.org_id, &user_id).await?))
}

/// Guards against two owners demoting or removing each other at once.
async fn ensure_owner_remains(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    org_id: &str,
) -> ApiResult<()> {
    let owners: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM memberships WHERE org_id = ? AND role = 'owner'")
            .bind(org_id)
            .fetch_one(&mut **tx)
            .await?;
    if owners == 0 {
        return Err(ApiError::Conflict(
            "an organization needs at least one owner".into(),
        ));
    }
    Ok(())
}

/// Remove a member. Their sessions and API tokens stop working at once.
#[utoipa::path(delete, path = "/api/v1/members/{user_id}", tag = "members", params(("user_id" = String, Path)),
    responses((status = 204), (status = 403, body = ErrorBody), (status = 404, body = ErrorBody)))]
async fn remove_member(
    State(state): State<AppState>,
    auth: Auth<Admin>,
    Path(user_id): Path<String>,
) -> ApiResult<StatusCode> {
    auth.require_interactive("sign in to the dashboard or Rig Desktop to manage team members")?;

    let mut tx = state.db.begin_with("BEGIN IMMEDIATE").await?;
    let actor_role = actor_role_tx(&mut tx, &auth.org_id, &auth.user_id).await?;
    if actor_role < Role::Admin {
        return Err(ApiError::Forbidden("your role does not allow this action"));
    }
    let target = load_member_tx(&mut tx, &auth.org_id, &user_id).await?;
    check_can_manage(actor_role, &auth.user_id, &target)?;

    sqlx::query("DELETE FROM memberships WHERE org_id = ? AND user_id = ?")
        .bind(&auth.org_id)
        .bind(&user_id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM sessions WHERE org_id = ? AND user_id = ?")
        .bind(&auth.org_id)
        .bind(&user_id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM api_tokens WHERE org_id = ? AND user_id = ?")
        .bind(&auth.org_id)
        .bind(&user_id)
        .execute(&mut *tx)
        .await?;
    // An account with no organization left cannot sign in to anything, and
    // would block re-adding the same email.
    sqlx::query(
        "DELETE FROM users WHERE id = ? AND NOT EXISTS (SELECT 1 FROM memberships WHERE user_id = ?)",
    )
    .bind(&user_id)
    .bind(&user_id)
    .execute(&mut *tx)
    .await?;
    ensure_owner_remains(&mut tx, &auth.org_id).await?;
    tx.commit().await?;

    auth.audit(&state, "member.remove", Some(&user_id), Some(&target.email))
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Give a member a new temporary password, sign them out everywhere, and revoke their API tokens.
#[utoipa::path(post, path = "/api/v1/members/{user_id}/reset-password", tag = "members", params(("user_id" = String, Path)),
    responses((status = 200, body = MemberWithPassword), (status = 403, body = ErrorBody), (status = 404, body = ErrorBody)))]
async fn reset_password(
    State(state): State<AppState>,
    auth: Auth<Admin>,
    Path(user_id): Path<String>,
) -> ApiResult<Json<MemberWithPassword>> {
    auth.require_interactive("sign in to the dashboard or Rig Desktop to reset member passwords")?;

    let temporary_password = crypto::random_token(15);
    let password_hash = hash(temporary_password.clone()).await?;
    let temp_expires = ts(chrono::Utc::now() + chrono::Duration::days(TEMP_PASSWORD_TTL_DAYS));

    let mut tx = state.db.begin_with("BEGIN IMMEDIATE").await?;
    let actor_role = actor_role_tx(&mut tx, &auth.org_id, &auth.user_id).await?;
    if actor_role < Role::Admin {
        return Err(ApiError::Forbidden("your role does not allow this action"));
    }
    let member = load_member_tx(&mut tx, &auth.org_id, &user_id).await?;
    check_can_manage(actor_role, &auth.user_id, &member)?;

    sqlx::query(
        "UPDATE users SET password_hash = ?, must_change_password = 1, temp_password_expires_at = ?
         WHERE id = ?",
    )
    .bind(&password_hash)
    .bind(&temp_expires)
    .bind(&user_id)
    .execute(&mut *tx)
    .await?;
    sqlx::query("DELETE FROM sessions WHERE user_id = ?")
        .bind(&user_id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM api_tokens WHERE user_id = ?")
        .bind(&user_id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM pending_2fa WHERE user_id = ?")
        .bind(&user_id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;

    auth.audit(
        &state,
        "member.reset_password",
        Some(&user_id),
        Some(&member.email),
    )
    .await?;
    Ok(Json(MemberWithPassword {
        member,
        temporary_password,
    }))
}

/// Change your own password. Rotates your current session and signs out your other sessions.
#[utoipa::path(post, path = "/api/v1/auth/password", tag = "auth", request_body = ChangePasswordRequest,
    responses((status = 204), (status = 403, body = ErrorBody), (status = 422, body = ErrorBody)))]
async fn change_password(
    State(state): State<AppState>,
    auth: Auth<Viewer>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    jar: CookieJar,
    Json(req): Json<ChangePasswordRequest>,
) -> ApiResult<(CookieJar, HeaderMap, StatusCode)> {
    // A leaked API token must not be able to take over the account.
    auth.require_session("sign in to the dashboard to change your password")?;
    validate_password(&req.new_password)?;
    if req.new_password == req.current_password {
        return Err(ApiError::validation(
            "the new password must differ from the current one",
        ));
    }

    let rate_key = format!("password:{}", auth.user_id);
    auth::check_rate(&state, &rate_key, MAX_PASSWORD_FAILURES).await?;
    let stored: String = sqlx::query_scalar("SELECT password_hash FROM users WHERE id = ?")
        .bind(&auth.user_id)
        .fetch_one(&state.db)
        .await?;
    let current = req.current_password;
    let ok = tokio::task::spawn_blocking(move || crypto::verify_password(&current, &stored))
        .await
        .map_err(|e| ApiError::Internal(e.into()))?;
    if !ok {
        auth::record_failure(&state, &rate_key).await?;
        auth.audit(&state, "auth.password_change_failed", Some(&auth.user_id), None)
            .await?;
        return Err(ApiError::validation("the current password is not right"));
    }
    auth::clear_failures(&state, &rate_key).await?;

    let password_hash = hash(req.new_password).await?;
    sqlx::query(
        "UPDATE users SET password_hash = ?, must_change_password = 0, temp_password_expires_at = NULL
         WHERE id = ?",
    )
    .bind(&password_hash)
    .bind(&auth.user_id)
    .execute(&state.db)
    .await?;

    let ua = headers
        .get(header::USER_AGENT)
        .and_then(|v| v.to_str().ok());
    let (cookie, csrf) = auth::rotate_sessions(
        &state,
        &auth.user_id,
        &auth.org_id,
        ua,
        auth.ip.as_deref(),
        auth::is_https(&state, &headers, Some(addr.ip())),
    )
    .await?;

    auth.audit(&state, "auth.password_change", Some(&auth.user_id), None)
        .await?;
    let mut out_headers = HeaderMap::new();
    if let Ok(v) = HeaderValue::from_str(&csrf) {
        out_headers.insert(CSRF_HEADER, v);
    }
    Ok((jar.add(cookie), out_headers, StatusCode::NO_CONTENT))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn member(id: &str, role: Role) -> Member {
        Member {
            user_id: id.into(),
            name: "N".into(),
            email: "n@example.com".into(),
            avatar_url: None,
            role,
            joined_at: String::new(),
            last_login_at: None,
        }
    }

    #[test]
    fn role_ladder() {
        // Nobody manages themselves.
        assert!(check_can_manage(Role::Owner, "a", &member("a", Role::Owner)).is_err());
        // Admins manage developers and viewers only.
        assert!(check_can_manage(Role::Admin, "a", &member("b", Role::Developer)).is_ok());
        assert!(check_can_manage(Role::Admin, "a", &member("b", Role::Viewer)).is_ok());
        assert!(check_can_manage(Role::Admin, "a", &member("b", Role::Admin)).is_err());
        assert!(check_can_manage(Role::Admin, "a", &member("b", Role::Owner)).is_err());
        // Owners manage everyone else.
        assert!(check_can_manage(Role::Owner, "a", &member("b", Role::Owner)).is_ok());
        // Granting stops at your own role.
        assert!(check_can_grant(Role::Admin, Role::Admin).is_ok());
        assert!(check_can_grant(Role::Admin, Role::Owner).is_err());
        assert!(check_can_grant(Role::Owner, Role::Owner).is_ok());
    }
}
