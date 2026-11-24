//! HTTP routes for local folder/binary deploys (`rig up`):
//! - Content-addressed blob cache check & zstd streaming uploads
//! - Local OCI image deploy (reassemble from cached+uploaded layers -> `Runtime::load_image`)
//! - Remote delta source deploy (materialize context -> build Dockerfile/Containerfile/Railpack)
//! - Direct Linux binary deploy (zero-Dockerfile scratch/distroless wrap)

use std::io::Write;

use axum::{
    Json, Router,
    body::Body,
    extract::{DefaultBodyLimit, Path, State},
    http::{HeaderMap, StatusCode, header},
    routing::{post, put},
};
use futures_util::StreamExt;
use rig_api::{
    BinaryDeployRequest, CheckBlobsRequest, CheckBlobsResponse, Deployment, ErrorBody,
    LocalImageDeployRequest, LocalSourceDeployRequest, StoredBlob, StoredBlobBatch,
};
use tokio::sync::mpsc;
use tower_http::limit::RequestBodyLimitLayer;
use utoipa_axum::{router::OpenApiRouter, routes};

use crate::{
    auth::{Auth, Developer},
    deploy::validate_relative_path,
    error::{ApiError, ApiResult},
    routes::apps::{load_app, require_app, start_deployment_with},
    runtime::LogSink,
    state::{AppState, new_id, now},
    uploads::{self, MAX_BLOB_BYTES},
};

const MAX_UPLOAD_BODY_BYTES: usize = 512 * 1024 * 1024;

/// JSON endpoints for `rig up` (covered by the standard 2 MiB JSON limit).
pub fn json_router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(check_blobs))
        .routes(routes!(deploy_local_image))
        .routes(routes!(deploy_local_source))
        .routes(routes!(deploy_binary))
}

/// Large streaming upload endpoints (up to 512 MiB compressed body).
pub fn streaming_router() -> Router<AppState> {
    Router::new()
        .route("/api/v1/uploads/blobs/{sha256}", put(upload_blob))
        .route("/api/v1/uploads/blobs/batch", post(upload_blob_batch))
        .layer(DefaultBodyLimit::max(MAX_UPLOAD_BODY_BYTES))
        .layer(RequestBodyLimitLayer::new(MAX_UPLOAD_BODY_BYTES))
}

/// Check which SHA-256 content blobs are already cached on the server.
#[utoipa::path(
    post,
    path = "/api/v1/uploads/blobs/check",
    tag = "uploads",
    request_body = CheckBlobsRequest,
    responses((status = 200, body = CheckBlobsResponse), (status = 422, body = ErrorBody))
)]
async fn check_blobs(
    State(state): State<AppState>,
    _auth: Auth<Developer>,
    Json(req): Json<CheckBlobsRequest>,
) -> ApiResult<Json<CheckBlobsResponse>> {
    if req.hashes.len() > uploads::MAX_CONTEXT_FILES {
        return Err(ApiError::validation("too many hashes in one check request"));
    }
    let data_dir = state.config.data_dir.clone();
    let hashes = req.hashes;
    let (present, missing) =
        tokio::task::spawn_blocking(move || uploads::check_blobs(&data_dir, &hashes))
            .await
            .map_err(|e| ApiError::Internal(e.into()))?
            .map_err(|e| ApiError::validation(format!("{e:#}")))?;
    Ok(Json(CheckBlobsResponse { present, missing }))
}

/// Stream a single blob (optionally `zstd`-compressed via `Content-Type: application/zstd`
/// or `Content-Encoding: zstd`) into the server's content-addressed blob cache.
async fn upload_blob(
    State(state): State<AppState>,
    _auth: Auth<Developer>,
    Path(sha256): Path<String>,
    headers: HeaderMap,
    body: Body,
) -> ApiResult<(StatusCode, Json<StoredBlob>)> {
    uploads::validate_sha256(&sha256).map_err(|e| ApiError::validation(format!("{e:#}")))?;
    let is_zstd = is_zstd_request(&headers);

    // Stream incoming request body to a temp file first so blocking zstd+sha256 runs off the reactor.
    let tmp_dir = uploads::blobs_dir(&state.config.data_dir).join(".incoming");
    tokio::fs::create_dir_all(&tmp_dir)
        .await
        .map_err(|e| ApiError::Internal(e.into()))?;
    let tmp = tempfile::NamedTempFile::new_in(&tmp_dir).map_err(|e| ApiError::Internal(e.into()))?;
    let tmp_path = tmp.into_temp_path();
    stream_body_to_file(body, &tmp_path, MAX_BLOB_BYTES).await?;

    let data_dir = state.config.data_dir.clone();
    let sha_clone = sha256.clone();
    let size = tokio::task::spawn_blocking(move || {
        let file = std::fs::File::open(&tmp_path)?;
        uploads::store_blob_from_reader(&data_dir, &sha_clone, file, is_zstd)
    })
    .await
    .map_err(|e| ApiError::Internal(e.into()))?
    .map_err(|e| ApiError::validation(format!("{e:#}")))?;

    Ok((StatusCode::CREATED, Json(StoredBlob { sha256, size })))
}

/// Stream a `zstd`-compressed delta tarball whose entries are named `<sha256>`.
async fn upload_blob_batch(
    State(state): State<AppState>,
    _auth: Auth<Developer>,
    body: Body,
) -> ApiResult<(StatusCode, Json<StoredBlobBatch>)> {
    let tmp_dir = uploads::blobs_dir(&state.config.data_dir).join(".incoming");
    tokio::fs::create_dir_all(&tmp_dir)
        .await
        .map_err(|e| ApiError::Internal(e.into()))?;
    let tmp = tempfile::NamedTempFile::new_in(&tmp_dir).map_err(|e| ApiError::Internal(e.into()))?;
    let tmp_path = tmp.into_temp_path();
    stream_body_to_file(body, &tmp_path, uploads::MAX_BATCH_BYTES).await?;

    let data_dir = state.config.data_dir.clone();
    let (stored, bytes) = tokio::task::spawn_blocking(move || {
        let file = std::fs::File::open(&tmp_path)?;
        uploads::store_blob_batch_zstd(&data_dir, file)
    })
    .await
    .map_err(|e| ApiError::Internal(e.into()))?
    .map_err(|e| ApiError::validation(format!("{e:#}")))?;

    Ok((
        StatusCode::CREATED,
        Json(StoredBlobBatch { stored, bytes }),
    ))
}

/// Deploy a locally built OCI/Docker image by reassembling cached + uploaded layers,
/// loading it into the server's container engine, and triggering a zero-downtime swap.
#[utoipa::path(
    post,
    path = "/api/v1/apps/{id}/deploy/local-image",
    tag = "uploads",
    params(("id" = String, Path)),
    request_body = LocalImageDeployRequest,
    responses(
        (status = 202, body = Deployment),
        (status = 404, body = ErrorBody),
        (status = 409, body = ErrorBody),
        (status = 422, body = ErrorBody)
    )
)]
async fn deploy_local_image(
    State(state): State<AppState>,
    auth: Auth<Developer>,
    Path(id): Path<String>,
    Json(req): Json<LocalImageDeployRequest>,
) -> ApiResult<(StatusCode, Json<Deployment>)> {
    require_app(&state, &auth.org_id, &id).await?;
    let image_ref = req.image_ref.trim();
    if image_ref.is_empty() || image_ref.len() > 255 {
        return Err(ApiError::validation("image_ref is required"));
    }

    let (tx, mut rx) = mpsc::channel(64);
    let drain = tokio::spawn(async move { while rx.recv().await.is_some() {} });
    let log = LogSink::new(tx);
    let loaded = uploads::load_reassembled_image(
        &state.config.data_dir,
        state.runtime.as_ref(),
        &req.entries,
        &log,
    )
    .await;
    drop(log);
    drain.abort();
    loaded.map_err(|e| ApiError::validation(format!("loading uploaded image: {e:#}")))?;

    let image_id = state
        .runtime
        .image_id(image_ref)
        .await
        .map_err(|e| ApiError::Unavailable(format!("inspecting loaded image: {e:#}")))?;

    let msg = req
        .label
        .as_deref()
        .map(|l| format!("Local image deploy ({l})"))
        .unwrap_or_else(|| "Local image deploy".into());
    let dep = start_deployment_with(
        &state,
        &auth,
        &id,
        "local",
        Some(image_ref),
        image_id.as_deref(),
        None,
        Some(&msg),
    )
    .await?;
    auth.audit(&state, "app.deploy.local_image", Some(&id), Some(&dep.id))
        .await?;
    Ok((StatusCode::ACCEPTED, Json(dep)))
}

/// Materialize a local project directory from uploaded blobs and queue a remote build.
#[utoipa::path(
    post,
    path = "/api/v1/apps/{id}/deploy/local-source",
    tag = "uploads",
    params(("id" = String, Path)),
    request_body = LocalSourceDeployRequest,
    responses(
        (status = 202, body = Deployment),
        (status = 404, body = ErrorBody),
        (status = 409, body = ErrorBody),
        (status = 422, body = ErrorBody)
    )
)]
async fn deploy_local_source(
    State(state): State<AppState>,
    auth: Auth<Developer>,
    Path(id): Path<String>,
    Json(req): Json<LocalSourceDeployRequest>,
) -> ApiResult<(StatusCode, Json<Deployment>)> {
    require_app(&state, &auth.org_id, &id).await?;
    if let Some(df) = req.dockerfile.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        validate_relative_path(df).map_err(ApiError::validation)?;
        sqlx::query("UPDATE apps SET dockerfile_path = ?, updated_at = ? WHERE id = ? AND org_id = ?")
            .bind(df)
            .bind(now())
            .bind(&id)
            .bind(&auth.org_id)
            .execute(&state.db)
            .await?;
    }

    let staging_id = new_id();
    let data_dir = state.config.data_dir.clone();
    let files = req.files;
    let staged = tokio::task::spawn_blocking(move || {
        uploads::materialize_context(&data_dir, &staging_id, &files)
    })
    .await
    .map_err(|e| ApiError::Internal(e.into()))?
    .map_err(|e| ApiError::validation(format!("{e:#}")))?;

    let staged_str = staged.to_string_lossy().into_owned();
    let msg = req
        .label
        .as_deref()
        .map(|l| format!("Local source deploy ({l})"))
        .unwrap_or_else(|| "Local source deploy".into());
    let dep = match start_deployment_with(
        &state,
        &auth,
        &id,
        "local",
        None,
        None,
        Some(&staged_str),
        Some(&msg),
    )
    .await
    {
        Ok(d) => d,
        Err(e) => {
            let _ = tokio::fs::remove_dir_all(&staged).await;
            return Err(e);
        }
    };
    auth.audit(&state, "app.deploy.local_source", Some(&id), Some(&dep.id))
        .await?;
    Ok((StatusCode::ACCEPTED, Json(dep)))
}

/// Wrap an uploaded Linux ELF binary in a minimal `scratch` or `distroless` OCI image
/// and queue a zero-downtime deployment — zero Dockerfile required.
#[utoipa::path(
    post,
    path = "/api/v1/apps/{id}/deploy/binary",
    tag = "uploads",
    params(("id" = String, Path)),
    request_body = BinaryDeployRequest,
    responses(
        (status = 202, body = Deployment),
        (status = 404, body = ErrorBody),
        (status = 409, body = ErrorBody),
        (status = 422, body = ErrorBody)
    )
)]
async fn deploy_binary(
    State(state): State<AppState>,
    auth: Auth<Developer>,
    Path(id): Path<String>,
    Json(req): Json<BinaryDeployRequest>,
) -> ApiResult<(StatusCode, Json<Deployment>)> {
    let app = load_app(&state, &auth.org_id, &id).await?;
    uploads::validate_sha256(&req.sha256).map_err(|e| ApiError::validation(format!("{e:#}")))?;
    if let Some(base) = req.base_image.as_deref() {
        let base = base.trim();
        let valid = !base.is_empty()
            && base.len() <= 255
            && base
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || "._-/:@".contains(c));
        if !valid {
            return Err(ApiError::validation("invalid base_image reference"));
        }
    }
    let port = match req.port {
        Some(0) => return Err(ApiError::validation("port must be between 1 and 65535")),
        Some(p) => {
            sqlx::query("UPDATE apps SET port = ?, updated_at = ? WHERE id = ? AND org_id = ?")
                .bind(i64::from(p))
                .bind(now())
                .bind(&id)
                .bind(&auth.org_id)
                .execute(&state.db)
                .await?;
            p
        }
        None => app.port,
    };

    let tag = format!("rig/{}:bin-{}", app.slug, &req.sha256[..12]);
    let (tx, mut rx) = mpsc::channel(64);
    let drain = tokio::spawn(async move { while rx.recv().await.is_some() {} });
    let log = LogSink::new(tx);
    let elf = uploads::wrap_binary_into_image(
        &state.config.data_dir,
        state.runtime.as_ref(),
        &app.id,
        &req.sha256,
        &tag,
        port,
        &req.args,
        req.base_image.as_deref(),
        &log,
    )
    .await;
    drop(log);
    drain.abort();
    let elf = elf.map_err(|e| ApiError::validation(format!("{e:#}")))?;

    let image_id = state
        .runtime
        .image_id(&tag)
        .await
        .map_err(|e| ApiError::Unavailable(format!("inspecting binary image: {e:#}")))?;

    let msg = format!(
        "Direct binary deploy: {} (linux/{}, {})",
        req.filename,
        elf.arch,
        if elf.is_static { "scratch" } else { "distroless" }
    );
    let dep = start_deployment_with(
        &state,
        &auth,
        &id,
        "local",
        Some(&tag),
        image_id.as_deref(),
        None,
        Some(&msg),
    )
    .await?;
    auth.audit(&state, "app.deploy.binary", Some(&id), Some(&req.filename))
        .await?;
    Ok((StatusCode::ACCEPTED, Json(dep)))
}

fn is_zstd_request(headers: &HeaderMap) -> bool {
    let ct = headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    let ce = headers
        .get(header::CONTENT_ENCODING)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    ct.contains("zstd") || ce.eq_ignore_ascii_case("zstd")
}

async fn stream_body_to_file(body: Body, path: &std::path::Path, max_bytes: u64) -> ApiResult<()> {
    let mut stream = body.into_data_stream();
    let mut file = std::fs::File::create(path).map_err(|e| ApiError::Internal(e.into()))?;
    let mut written = 0u64;
    while let Some(chunk) = stream.next().await {
        let bytes = chunk.map_err(|e| ApiError::validation(format!("reading upload body: {e}")))?;
        written = written.saturating_add(bytes.len() as u64);
        if written > max_bytes {
            return Err(ApiError::validation(format!(
                "upload exceeds limit of {} MiB",
                max_bytes / (1024 * 1024)
            )));
        }
        file.write_all(&bytes)
            .map_err(|e| ApiError::Internal(e.into()))?;
    }
    file.flush().map_err(|e| ApiError::Internal(e.into()))?;
    Ok(())
}
