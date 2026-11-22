//! Organization branding (name, logo, brand accent, environment badge) and
//! uploaded media serving.

use axum::{
    Json,
    body::Body,
    extract::{Path, State},
    http::{HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
};
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use rig_api::{ErrorBody, Organization, UpdateOrganizationRequest, UploadImageRequest};
use sqlx::Row;
use utoipa_axum::{router::OpenApiRouter, routes};

use super::auth::validate_name;
use crate::{
    auth::{Admin, Auth, Viewer},
    error::{ApiError, ApiResult},
    state::{AppState, new_id},
};

pub const MAX_IMAGE_BYTES: usize = 768 * 1024;
pub const MEDIA_PREFIX: &str = "/api/v1/media/";

pub const VALID_ACCENTS: &[&str] = &[
    "emerald", "indigo", "sky", "violet", "amber", "rose", "graphite",
];

pub const VALID_ENV_COLORS: &[&str] = &["emerald", "amber", "red", "blue", "purple", "gray"];

pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(update_org))
        .routes(routes!(upload_org_logo, delete_org_logo))
        .routes(routes!(get_media))
}

pub(crate) fn media_id_from_url(url: &str) -> Option<&str> {
    url.strip_prefix(MEDIA_PREFIX)
        .filter(|id| !id.is_empty() && !id.contains('/'))
}

pub(crate) fn validate_external_image_url(raw: &str) -> ApiResult<String> {
    let trimmed = raw.trim();
    if trimmed.len() > 2048 || trimmed.chars().any(char::is_control) {
        return Err(ApiError::validation(
            "image URL must be at most 2048 characters",
        ));
    }
    let parsed = url::Url::parse(trimmed)
        .map_err(|_| ApiError::validation("enter a valid http:// or https:// image URL"))?;
    if !matches!(parsed.scheme(), "http" | "https") || parsed.host_str().is_none() {
        return Err(ApiError::validation(
            "image URL must start with http:// or https://",
        ));
    }
    Ok(trimmed.to_string())
}

pub(crate) fn validate_avatar_preset(raw: &str) -> ApiResult<String> {
    let trimmed = raw.trim();
    let Some(slug) = trimmed.strip_prefix("preset:") else {
        return Err(ApiError::validation("invalid avatar preset"));
    };
    if slug.is_empty()
        || slug.len() > 32
        || !slug
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
    {
        return Err(ApiError::validation("invalid avatar preset"));
    }
    Ok(trimmed.to_string())
}

fn is_safe_svg(bytes: &[u8]) -> bool {
    let Ok(text) = std::str::from_utf8(bytes) else {
        return false;
    };
    let lower = text.to_ascii_lowercase();
    if !lower.contains("<svg") {
        return false;
    }
    for banned in [
        "<script",
        "javascript:",
        "<foreignobject",
        "<iframe",
        "<object",
        "<embed",
        "data:text/html",
    ] {
        if lower.contains(banned) {
            return false;
        }
    }
    // Reject inline event handlers like onload=, onerror=, onclick=, etc.
    for (idx, _) in lower.match_indices("on") {
        let prev = idx
            .checked_sub(1)
            .and_then(|i| lower.as_bytes().get(i).copied());
        if prev.is_some_and(|b| b.is_ascii_whitespace() || b == b'/' || b == b'"' || b == b'\'') {
            let rest = &lower[idx + 2..];
            let name_len = rest
                .bytes()
                .take_while(|b| b.is_ascii_alphabetic())
                .count();
            if name_len >= 3 {
                let after = rest[name_len..].trim_start();
                if after.starts_with('=') {
                    return false;
                }
            }
        }
    }
    true
}

pub(crate) fn validate_and_decode_image(req: &UploadImageRequest) -> ApiResult<(String, Vec<u8>)> {
    let ct = req.content_type.trim().to_ascii_lowercase();
    let canonical = match ct.as_str() {
        "image/png" => "image/png",
        "image/jpeg" | "image/jpg" => "image/jpeg",
        "image/webp" => "image/webp",
        "image/gif" => "image/gif",
        "image/svg+xml" => "image/svg+xml",
        _ => {
            return Err(ApiError::validation(
                "unsupported image type; use PNG, JPEG, WebP, GIF, or SVG",
            ));
        }
    };

    let b64 = req.data_base64.trim();
    if b64.is_empty() {
        return Err(ApiError::validation("image data is empty"));
    }
    // Reject overly large base64 strings before decoding.
    if b64.len() > MAX_IMAGE_BYTES * 2 {
        return Err(ApiError::validation("image must be 768 KB or smaller"));
    }
    let bytes = BASE64
        .decode(b64)
        .map_err(|_| ApiError::validation("invalid base64 image data"))?;
    if bytes.is_empty() || bytes.len() > MAX_IMAGE_BYTES {
        return Err(ApiError::validation("image must be 768 KB or smaller"));
    }

    let valid_magic = match canonical {
        "image/png" => bytes.starts_with(b"\x89PNG\r\n\x1a\n"),
        "image/jpeg" => bytes.starts_with(&[0xFF, 0xD8, 0xFF]),
        "image/webp" => bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP",
        "image/gif" => bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a"),
        "image/svg+xml" => is_safe_svg(&bytes),
        _ => false,
    };
    if !valid_magic {
        return Err(ApiError::validation(
            "image file does not match its content type or contains disallowed markup",
        ));
    }

    Ok((canonical.to_string(), bytes))
}

pub(crate) async fn store_media(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    org_id: &str,
    content_type: &str,
    data: &[u8],
    is_public: bool,
) -> ApiResult<String> {
    let id = new_id();
    sqlx::query(
        "INSERT INTO media (id, org_id, content_type, data, is_public) VALUES (?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(org_id)
    .bind(content_type)
    .bind(data)
    .bind(if is_public { 1i64 } else { 0i64 })
    .execute(&mut **tx)
    .await?;
    Ok(format!("{MEDIA_PREFIX}{id}"))
}

pub(crate) async fn delete_media_if_owned(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    org_id: &str,
    url: Option<&str>,
) -> ApiResult<()> {
    if let Some(id) = url.and_then(media_id_from_url) {
        sqlx::query("DELETE FROM media WHERE id = ? AND org_id = ?")
            .bind(id)
            .bind(org_id)
            .execute(&mut **tx)
            .await?;
    }
    Ok(())
}

pub(crate) async fn load_org(state: &AppState, org_id: &str) -> ApiResult<Organization> {
    let row = sqlx::query(
        "SELECT id, name, slug, logo_url, accent_color, env_label, env_color
         FROM organizations WHERE id = ?",
    )
    .bind(org_id)
    .fetch_one(&state.db)
    .await?;
    Ok(Organization {
        id: row.get("id"),
        name: row.get("name"),
        slug: row.get("slug"),
        logo_url: row.get("logo_url"),
        accent_color: row.get("accent_color"),
        env_label: row.get("env_label"),
        env_color: row.get("env_color"),
    })
}

/// Update organization name, logo URL, brand accent color, or environment badge.
#[utoipa::path(patch, path = "/api/v1/org", tag = "organization",
    request_body = UpdateOrganizationRequest,
    responses((status = 200, body = Organization), (status = 403, body = ErrorBody),
              (status = 422, body = ErrorBody)))]
async fn update_org(
    State(state): State<AppState>,
    auth: Auth<Admin>,
    Json(req): Json<UpdateOrganizationRequest>,
) -> ApiResult<Json<Organization>> {
    let current = load_org(&state, &auth.org_id).await?;

    let next_name = match req.name {
        Some(n) => validate_name("organization name", &n, 80)?,
        None => current.name.clone(),
    };

    let (next_logo, logo_changed) = match req.logo_url {
        None => (current.logo_url.clone(), false),
        Some(ref s) if s.trim().is_empty() => (None, true),
        Some(ref s) => (Some(validate_external_image_url(s)?), true),
    };

    let next_accent = match req.accent_color {
        None => current.accent_color.clone(),
        Some(ref c) => {
            let c = c.trim().to_ascii_lowercase();
            if !VALID_ACCENTS.contains(&c.as_str()) {
                return Err(ApiError::validation("unsupported accent color"));
            }
            c
        }
    };

    let next_env_label = match req.env_label {
        None => current.env_label.clone(),
        Some(ref s) if s.trim().is_empty() => None,
        Some(ref s) => Some(validate_name("environment label", s, 24)?),
    };

    let next_env_color = match req.env_color {
        None => current.env_color.clone(),
        Some(ref s) if s.trim().is_empty() => None,
        Some(ref s) => {
            let c = s.trim().to_ascii_lowercase();
            if !VALID_ENV_COLORS.contains(&c.as_str()) {
                return Err(ApiError::validation("unsupported environment badge color"));
            }
            Some(c)
        }
    };

    let mut tx = state.db.begin().await?;
    sqlx::query(
        "UPDATE organizations
         SET name = ?, logo_url = ?, accent_color = ?, env_label = ?, env_color = ?
         WHERE id = ?",
    )
    .bind(&next_name)
    .bind(&next_logo)
    .bind(&next_accent)
    .bind(&next_env_label)
    .bind(&next_env_color)
    .bind(&auth.org_id)
    .execute(&mut *tx)
    .await?;

    if logo_changed && current.logo_url != next_logo {
        delete_media_if_owned(&mut tx, &auth.org_id, current.logo_url.as_deref()).await?;
    }
    tx.commit().await?;

    auth.audit(&state, "org.update", Some(&next_name), None)
        .await?;
    Ok(Json(load_org(&state, &auth.org_id).await?))
}

/// Upload a custom organization logo image (PNG, JPEG, WebP, GIF, or SVG).
#[utoipa::path(post, path = "/api/v1/org/logo", tag = "organization",
    request_body = UploadImageRequest,
    responses((status = 200, body = Organization), (status = 403, body = ErrorBody),
              (status = 422, body = ErrorBody)))]
async fn upload_org_logo(
    State(state): State<AppState>,
    auth: Auth<Admin>,
    Json(req): Json<UploadImageRequest>,
) -> ApiResult<Json<Organization>> {
    let (content_type, bytes) = validate_and_decode_image(&req)?;
    let current = load_org(&state, &auth.org_id).await?;

    let mut tx = state.db.begin().await?;
    let url = store_media(&mut tx, &auth.org_id, &content_type, &bytes, true).await?;
    sqlx::query("UPDATE organizations SET logo_url = ? WHERE id = ?")
        .bind(&url)
        .bind(&auth.org_id)
        .execute(&mut *tx)
        .await?;
    delete_media_if_owned(&mut tx, &auth.org_id, current.logo_url.as_deref()).await?;
    tx.commit().await?;

    auth.audit(&state, "org.logo_upload", None, Some(&content_type))
        .await?;
    Ok(Json(load_org(&state, &auth.org_id).await?))
}

/// Remove the organization logo and fall back to the default mark.
#[utoipa::path(delete, path = "/api/v1/org/logo", tag = "organization",
    responses((status = 200, body = Organization), (status = 403, body = ErrorBody)))]
async fn delete_org_logo(
    State(state): State<AppState>,
    auth: Auth<Admin>,
) -> ApiResult<Json<Organization>> {
    let current = load_org(&state, &auth.org_id).await?;
    let mut tx = state.db.begin().await?;
    sqlx::query("UPDATE organizations SET logo_url = NULL WHERE id = ?")
        .bind(&auth.org_id)
        .execute(&mut *tx)
        .await?;
    delete_media_if_owned(&mut tx, &auth.org_id, current.logo_url.as_deref()).await?;
    tx.commit().await?;

    auth.audit(&state, "org.logo_remove", None, None).await?;
    Ok(Json(load_org(&state, &auth.org_id).await?))
}

/// Serve an uploaded organization logo or user avatar.
#[utoipa::path(get, path = "/api/v1/media/{id}", tag = "media",
    params(("id" = String, Path)),
    responses((status = 200, description = "Raw image bytes"), (status = 404, body = ErrorBody)))]
async fn get_media(
    State(state): State<AppState>,
    auth: Result<Auth<Viewer>, ApiError>,
    Path(id): Path<String>,
) -> ApiResult<Response> {
    let row = sqlx::query("SELECT org_id, content_type, data, is_public FROM media WHERE id = ?")
        .bind(&id)
        .fetch_optional(&state.db)
        .await?
        .ok_or(ApiError::NotFound("media"))?;

    let is_public: i64 = row.get("is_public");
    if is_public == 0 {
        let auth = auth?;
        let org_id: String = row.get("org_id");
        if org_id != auth.org_id {
            return Err(ApiError::NotFound("media"));
        }
    }

    let content_type: String = row.get("content_type");
    let data: Vec<u8> = row.get("data");
    let mut res = Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, &content_type)
        .header(header::CACHE_CONTROL, "public, max-age=31536000, immutable")
        .header(
            header::X_CONTENT_TYPE_OPTIONS,
            HeaderValue::from_static("nosniff"),
        );
    if content_type == "image/svg+xml" {
        res = res.header(
            header::CONTENT_SECURITY_POLICY,
            HeaderValue::from_static("default-src 'none'; style-src 'unsafe-inline'; sandbox"),
        );
    }
    Ok(res
        .body(Body::from(data))
        .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_external_urls() {
        assert!(validate_external_image_url("https://example.com/logo.png").is_ok());
        assert!(validate_external_image_url("http://cdn.local/avatar.webp").is_ok());
        assert!(validate_external_image_url("javascript:alert(1)").is_err());
        assert!(validate_external_image_url("data:image/png;base64,abc").is_err());
        assert!(validate_external_image_url("not a url").is_err());
    }

    #[test]
    fn rejects_unsafe_svgs() {
        assert!(is_safe_svg(b"<svg viewBox='0 0 10 10'><circle r='4'/></svg>"));
        assert!(!is_safe_svg(b"<svg><script>alert(1)</script></svg>"));
        assert!(!is_safe_svg(b"<svg onload=\"alert(1)\"></svg>"));
        assert!(!is_safe_svg(b"<svg><a href=\"javascript:alert(1)\"/></svg>"));
    }

    #[test]
    fn validates_presets() {
        assert!(validate_avatar_preset("preset:orb-emerald").is_ok());
        assert!(validate_avatar_preset("preset:").is_err());
        assert!(validate_avatar_preset("preset:BAD_NAME").is_err());
        assert!(validate_avatar_preset("other:foo").is_err());
    }
}
