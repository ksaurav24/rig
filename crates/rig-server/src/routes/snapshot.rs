//! HTTP routes for full portable server snapshot export, dry-run inspection, and restore.

use std::io::Write;

use axum::{
    Json, Router,
    body::Body,
    extract::{DefaultBodyLimit, State},
    http::{HeaderMap, HeaderValue, header},
    response::{IntoResponse, Response},
    routing::post,
};
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use futures_util::StreamExt;
use rig_api::{
    ErrorBody, SnapshotExportRequest, SnapshotInspectResult, SnapshotRestoreOptions,
    SnapshotRestoreReport,
};
use tokio_util::io::ReaderStream;
use tower_http::limit::RequestBodyLimitLayer;
use utoipa_axum::{router::OpenApiRouter, routes};

use crate::{
    auth::{Admin, Auth, Owner},
    error::{ApiError, ApiResult},
    snapshot,
    state::AppState,
};

const MAX_SNAPSHOT_UPLOAD_BYTES: usize = 512 * 1024 * 1024;
pub const HEADER_SNAPSHOT_PASSPHRASE: &str = "x-rig-snapshot-passphrase";
pub const HEADER_SNAPSHOT_OPTIONS: &str = "x-rig-snapshot-options";

pub fn json_router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new().routes(routes!(export_snapshot))
}

pub fn streaming_router() -> Router<AppState> {
    Router::new()
        .route("/api/v1/system/snapshot/inspect", post(inspect_snapshot))
        .route("/api/v1/system/snapshot/restore", post(restore_snapshot))
        .layer(DefaultBodyLimit::max(MAX_SNAPSHOT_UPLOAD_BYTES))
        .layer(RequestBodyLimitLayer::new(MAX_SNAPSHOT_UPLOAD_BYTES))
}

/// Export a full portable snapshot of this Rig server's organization, projects,
/// apps, decrypted-for-portability secrets, domains, volumes, managed databases
/// (with live dumps), media, and alert channels.
#[utoipa::path(
    post,
    path = "/api/v1/system/snapshot/export",
    tag = "system",
    request_body = SnapshotExportRequest,
    responses(
        (status = 200, content_type = "application/octet-stream"),
        (status = 403, body = ErrorBody)
    )
)]
async fn export_snapshot(
    State(state): State<AppState>,
    auth: Auth<Owner>,
    Json(req): Json<SnapshotExportRequest>,
) -> ApiResult<Response> {
    auth.audit(
        &state,
        "system.snapshot_export",
        Some(&auth.org_id),
        Some(if req.passphrase.as_deref().is_some_and(|p| !p.trim().is_empty()) {
            "encrypted"
        } else {
            "unencrypted"
        }),
    )
    .await?;

    let exported = snapshot::export_snapshot(&state, &auth.org_id, &req)
        .await
        .map_err(|e| ApiError::Internal(e.context("exporting server snapshot")))?;

    let file = tokio::fs::File::open(&exported.archive_path)
        .await
        .map_err(|e| ApiError::Internal(e.into()))?;
    let size = exported.size_bytes;
    let filename = exported.filename.clone();
    let encrypted = exported.encrypted;
    // Keep the TempDir alive until the response stream finishes reading the file.
    let temp_guard = exported._temp_dir;
    let stream = ReaderStream::new(file).map(move |chunk| {
        let _keep = &temp_guard;
        chunk
    });

    let mut res = Body::from_stream(stream).into_response();
    let headers = res.headers_mut();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/octet-stream"),
    );
    headers.insert(header::CONTENT_LENGTH, size.into());
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    headers.insert(
        header::HeaderName::from_static("x-rig-snapshot-encrypted"),
        HeaderValue::from_static(if encrypted { "true" } else { "false" }),
    );
    if let Ok(v) = format!("attachment; filename=\"{filename}\"").parse() {
        headers.insert(header::CONTENT_DISPOSITION, v);
    }
    Ok(res)
}

fn header_passphrase(headers: &HeaderMap) -> Option<String> {
    let raw = headers.get(HEADER_SNAPSHOT_PASSPHRASE)?.to_str().ok()?;
    let decoded = BASE64
        .decode(raw.trim())
        .ok()
        .and_then(|b| String::from_utf8(b).ok())
        .unwrap_or_else(|| raw.to_owned());
    let trimmed = decoded.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_owned())
}

fn header_restore_options(headers: &HeaderMap) -> ApiResult<SnapshotRestoreOptions> {
    let Some(raw) = headers.get(HEADER_SNAPSHOT_OPTIONS).and_then(|v| v.to_str().ok()) else {
        return Ok(SnapshotRestoreOptions::default());
    };
    let bytes = BASE64
        .decode(raw.trim())
        .map_err(|_| ApiError::validation("invalid base64 in X-Rig-Snapshot-Options"))?;
    serde_json::from_slice(&bytes)
        .map_err(|e| ApiError::validation(format!("invalid snapshot restore options: {e}")))
}

async fn stream_body_to_temp(body: Body, dir: &std::path::Path) -> ApiResult<std::path::PathBuf> {
    let path = dir.join("incoming.snapshot");
    let mut file = std::fs::File::create(&path).map_err(|e| ApiError::Internal(e.into()))?;
    let mut stream = body.into_data_stream();
    let mut written = 0u64;
    while let Some(chunk) = stream.next().await {
        let bytes = chunk.map_err(|e| ApiError::validation(format!("reading snapshot body: {e}")))?;
        written = written.saturating_add(bytes.len() as u64);
        if written > MAX_SNAPSHOT_UPLOAD_BYTES as u64 {
            return Err(ApiError::validation("snapshot exceeds 512 MiB upload limit"));
        }
        file.write_all(&bytes)
            .map_err(|e| ApiError::Internal(e.into()))?;
    }
    file.flush().map_err(|e| ApiError::Internal(e.into()))?;
    Ok(path)
}

/// Dry-run inspect a snapshot archive against this server without modifying anything.
async fn inspect_snapshot(
    State(state): State<AppState>,
    auth: Auth<Admin>,
    headers: HeaderMap,
    body: Body,
) -> ApiResult<Json<SnapshotInspectResult>> {
    let tmp = tempfile::tempdir().map_err(|e| ApiError::Internal(e.into()))?;
    let path = stream_body_to_temp(body, tmp.path()).await?;
    let pass = header_passphrase(&headers);
    let preview = snapshot::inspect_snapshot(&state, &auth.org_id, &path, pass.as_deref())
        .await
        .map_err(|e| ApiError::validation(format!("{e:#}")))?;
    Ok(Json(preview))
}

/// Restore a portable snapshot archive onto this server.
async fn restore_snapshot(
    State(state): State<AppState>,
    auth: Auth<Owner>,
    headers: HeaderMap,
    body: Body,
) -> ApiResult<Json<SnapshotRestoreReport>> {
    let opts = header_restore_options(&headers)?;
    let pass = header_passphrase(&headers);
    let tmp = tempfile::tempdir().map_err(|e| ApiError::Internal(e.into()))?;
    let path = stream_body_to_temp(body, tmp.path()).await?;

    auth.audit(
        &state,
        "system.snapshot_restore",
        Some(&auth.org_id),
        Some(match opts.mode {
            rig_api::SnapshotRestoreMode::Merge => "merge",
            rig_api::SnapshotRestoreMode::Replace => "replace",
        }),
    )
    .await?;

    let report = snapshot::restore_snapshot(
        &state,
        &auth.org_id,
        &auth.user_id,
        &path,
        pass.as_deref(),
        &opts,
    )
    .await
    .map_err(|e| ApiError::validation(format!("{e:#}")))?;
    Ok(Json(report))
}
