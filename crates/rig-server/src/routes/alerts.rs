//! Alert channels: where Rig sends word when something breaks. Admin only,
//! since a channel decides where operational details leave the server.

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use rig_api::{
    AlertChannel, AlertEvent, AlertTestResult, CreateAlertChannelRequest, CreatedAlertChannel,
    ErrorBody, UpdateAlertChannelRequest,
};
use sqlx::{Row, sqlite::SqliteRow};
use utoipa_axum::{router::OpenApiRouter, routes};

use crate::{
    alerts::{self, Alert, event_key, parse_event},
    auth::{Admin, Auth},
    crypto::random_token,
    error::{ApiError, ApiResult},
    routes::auth::validate_name,
    state::{AppState, new_id},
};

const MAX_CHANNELS: i64 = 20;

pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(list_channels, create_channel))
        .routes(routes!(update_channel, delete_channel))
        .routes(routes!(test_channel))
}

fn channel_from_row(r: &SqliteRow) -> AlertChannel {
    AlertChannel {
        id: r.get("id"),
        name: r.get("name"),
        url_host: r.get("url_host"),
        events: r
            .get::<String, _>("events")
            .split(',')
            .filter_map(parse_event)
            .collect(),
        enabled: r.get::<i64, _>("enabled") != 0,
        last_sent_at: r.get("last_sent_at"),
        last_error: r.get("last_error"),
        created_at: r.get("created_at"),
    }
}

fn encode_events(events: &[AlertEvent]) -> ApiResult<String> {
    if events.is_empty() {
        return Err(ApiError::Validation("pick at least one event".into()));
    }
    let mut keys: Vec<&str> = events.iter().map(|e| event_key(*e)).collect();
    keys.sort_unstable();
    keys.dedup();
    Ok(keys.join(","))
}

async fn load(state: &AppState, org_id: &str, id: &str) -> ApiResult<SqliteRow> {
    sqlx::query(
        "SELECT id, name, url_host, events, enabled, last_sent_at, last_error, created_at,
                url_enc, signing_secret
           FROM alert_channels WHERE id = ? AND org_id = ?",
    )
    .bind(id)
    .bind(org_id)
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::NotFound("alert channel not found"))
}

/// Alert channels in this organization.
#[utoipa::path(get, path = "/api/v1/alerts/channels", tag = "alerts",
    responses((status = 200, body = Vec<AlertChannel>)))]
async fn list_channels(
    State(state): State<AppState>,
    auth: Auth<Admin>,
) -> ApiResult<Json<Vec<AlertChannel>>> {
    let rows = sqlx::query(
        "SELECT id, name, url_host, events, enabled, last_sent_at, last_error, created_at
           FROM alert_channels WHERE org_id = ? ORDER BY created_at",
    )
    .bind(&auth.org_id)
    .fetch_all(&state.db)
    .await?;
    Ok(Json(rows.iter().map(channel_from_row).collect()))
}

/// Add a channel. The signing secret is returned only here.
#[utoipa::path(post, path = "/api/v1/alerts/channels", tag = "alerts",
    request_body = CreateAlertChannelRequest,
    responses((status = 201, body = CreatedAlertChannel), (status = 422, body = ErrorBody)))]
async fn create_channel(
    State(state): State<AppState>,
    auth: Auth<Admin>,
    Json(req): Json<CreateAlertChannelRequest>,
) -> ApiResult<(StatusCode, Json<CreatedAlertChannel>)> {
    let name = validate_name("channel name", &req.name, 60)?;
    let url = alerts::validate_url(&req.url, state.config.allow_private_alerts)
        .map_err(|e| ApiError::Validation(e.to_string()))?;
    let events = encode_events(&req.events)?;
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM alert_channels WHERE org_id = ?")
        .bind(&auth.org_id)
        .fetch_one(&state.db)
        .await?;
    if count >= MAX_CHANNELS {
        return Err(ApiError::Validation(format!(
            "an organization can have at most {MAX_CHANNELS} alert channels"
        )));
    }
    let id = new_id();
    let secret = random_token(24);
    let secret_enc = state.vault.encrypt(&secret)?;
    let host = url.host_str().unwrap_or_default().to_owned();
    sqlx::query(
        "INSERT INTO alert_channels (id, org_id, name, url_enc, url_host, signing_secret, events)
         VALUES (?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(&auth.org_id)
    .bind(&name)
    .bind(state.vault.encrypt(url.as_str())?)
    .bind(&host)
    .bind(&secret_enc)
    .bind(&events)
    .execute(&state.db)
    .await?;
    auth.audit(&state, "alert.channel.create", Some(&id), Some(&host))
        .await?;
    let row = load(&state, &auth.org_id, &id).await?;
    Ok((
        StatusCode::CREATED,
        Json(CreatedAlertChannel {
            channel: channel_from_row(&row),
            signing_secret: secret,
        }),
    ))
}

/// Rename a channel, change its events, or pause it.
#[utoipa::path(patch, path = "/api/v1/alerts/channels/{id}", tag = "alerts",
    params(("id" = String, Path)), request_body = UpdateAlertChannelRequest,
    responses((status = 200, body = AlertChannel), (status = 404, body = ErrorBody)))]
async fn update_channel(
    State(state): State<AppState>,
    auth: Auth<Admin>,
    Path(id): Path<String>,
    Json(req): Json<UpdateAlertChannelRequest>,
) -> ApiResult<Json<AlertChannel>> {
    load(&state, &auth.org_id, &id).await?;
    let name = req
        .name
        .as_deref()
        .map(|n| validate_name("channel name", n, 60))
        .transpose()?;
    let events = req.events.as_deref().map(encode_events).transpose()?;
    sqlx::query(
        "UPDATE alert_channels
            SET name = COALESCE(?, name), events = COALESCE(?, events), enabled = COALESCE(?, enabled)
          WHERE id = ? AND org_id = ?",
    )
    .bind(name)
    .bind(events)
    .bind(req.enabled.map(i64::from))
    .bind(&id)
    .bind(&auth.org_id)
    .execute(&state.db)
    .await?;
    auth.audit(&state, "alert.channel.update", Some(&id), None)
        .await?;
    Ok(Json(channel_from_row(
        &load(&state, &auth.org_id, &id).await?,
    )))
}

/// Remove a channel.
#[utoipa::path(delete, path = "/api/v1/alerts/channels/{id}", tag = "alerts",
    params(("id" = String, Path)),
    responses((status = 204), (status = 404, body = ErrorBody)))]
async fn delete_channel(
    State(state): State<AppState>,
    auth: Auth<Admin>,
    Path(id): Path<String>,
) -> ApiResult<StatusCode> {
    let done = sqlx::query("DELETE FROM alert_channels WHERE id = ? AND org_id = ?")
        .bind(&id)
        .bind(&auth.org_id)
        .execute(&state.db)
        .await?;
    if done.rows_affected() == 0 {
        return Err(ApiError::NotFound("alert channel not found"));
    }
    auth.audit(&state, "alert.channel.delete", Some(&id), None)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Send a test alert now and report what happened.
#[utoipa::path(post, path = "/api/v1/alerts/channels/{id}/test", tag = "alerts",
    params(("id" = String, Path)),
    responses((status = 200, body = AlertTestResult), (status = 404, body = ErrorBody)))]
async fn test_channel(
    State(state): State<AppState>,
    auth: Auth<Admin>,
    Path(id): Path<String>,
) -> ApiResult<Json<AlertTestResult>> {
    let row = load(&state, &auth.org_id, &id).await?;
    let alert = Alert {
        event: AlertEvent::DeployFailed,
        title: "Test alert from Rig".into(),
        detail: "This channel is set up correctly.".into(),
        target: None,
    };
    let outcome = alerts::deliver(
        &state,
        &row.get::<String, _>("url_enc"),
        &row.get::<String, _>("signing_secret"),
        &alert,
    )
    .await;
    alerts::record(&state, &id, outcome.as_ref().err()).await;
    Ok(Json(AlertTestResult {
        ok: outcome.is_ok(),
        error: outcome.err().map(|e| format!("{e:#}")),
    }))
}
