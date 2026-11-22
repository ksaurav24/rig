//! Serves the dashboard, which is compiled into the binary.
//!
//! Files under `/assets/` carry a content hash in their name and are cached
//! forever. Everything else is revalidated with an ETag. Unknown paths get
//! `index.html` so client-side routes survive a reload.

use axum::{
    body::Body,
    http::{HeaderMap, HeaderValue, Method, StatusCode, Uri, header},
    response::{IntoResponse, Response},
};
use rust_embed::RustEmbed;

#[derive(RustEmbed)]
#[folder = "../../web/dist"]
struct Dist;

/// Everything the dashboard needs comes from this origin. Inline styles are
/// allowed because UI libraries position popups with them; scripts are not.
/// Forms may also post to the GitHub host, for the App manifest flow.
static CSP: std::sync::OnceLock<HeaderValue> = std::sync::OnceLock::new();

/// Set the policy once at startup; `github_web` is an origin like `https://github.com`.
pub fn init_csp(github_web: &str) {
    let policy = format!(
        "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; \
         img-src 'self' data:; font-src 'self' data:; connect-src 'self'; object-src 'none'; \
         base-uri 'self'; form-action 'self' {github_web}; frame-ancestors 'none'"
    );
    let _ = CSP.set(
        HeaderValue::from_str(&policy).unwrap_or(HeaderValue::from_static(
            "default-src 'self'; form-action 'self'; frame-ancestors 'none'",
        )),
    );
}

const NOT_BUILT: &str = "Rig is running, but this binary was built without the dashboard.\n\
    Build it with `pnpm --dir web build` and rebuild the server, or use the API at /api/docs.\n";

pub async fn serve(method: Method, uri: Uri, headers: HeaderMap) -> Response {
    if method != Method::GET && method != Method::HEAD {
        return StatusCode::METHOD_NOT_ALLOWED.into_response();
    }
    let path = uri.path().trim_start_matches('/');
    // API typos should be a JSON-ish 404, not the dashboard's HTML.
    if path == "api" || path.starts_with("api/") {
        return StatusCode::NOT_FOUND.into_response();
    }
    if let Some(file) = Dist::get(path).filter(|_| !path.is_empty()) {
        return respond(path, file, &headers);
    }
    // A missing file with an extension is a real 404; anything else is a route.
    let last = path.rsplit('/').next().unwrap_or_default();
    if last.contains('.') {
        return StatusCode::NOT_FOUND.into_response();
    }
    match Dist::get("index.html") {
        Some(file) => respond("index.html", file, &headers),
        None => (StatusCode::OK, NOT_BUILT).into_response(),
    }
}

fn respond(path: &str, file: rust_embed::EmbeddedFile, headers: &HeaderMap) -> Response {
    let etag = format!("\"{}\"", hex::encode(&file.metadata.sha256_hash()[..12]));
    let immutable = path.starts_with("assets/");
    let mime = mime_guess::from_path(path).first_or_octet_stream();

    let mut res = Response::builder()
        .header(header::CONTENT_TYPE, mime.as_ref())
        .header(header::ETAG, &etag)
        .header(
            header::CACHE_CONTROL,
            if immutable {
                "public, max-age=31536000, immutable"
            } else {
                "no-cache"
            },
        );
    if mime == mime_guess::mime::TEXT_HTML {
        res = res.header(
            header::CONTENT_SECURITY_POLICY,
            CSP.get().cloned().unwrap_or(HeaderValue::from_static(
                "default-src 'self'; frame-ancestors 'none'",
            )),
        );
    }
    let fresh = headers
        .get(header::IF_NONE_MATCH)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.split(',').any(|tag| tag.trim() == etag));
    let res = if fresh {
        res.status(StatusCode::NOT_MODIFIED).body(Body::empty())
    } else {
        res.body(Body::from(file.data))
    };
    res.unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
}
