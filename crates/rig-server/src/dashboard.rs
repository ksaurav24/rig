//! Serves the dashboard, which is pre-compressed and compiled into the binary.
//!
//! Files under `/assets/` carry a content hash in their name and are cached
//! forever. Everything else is revalidated with an ETag. Unknown paths get
//! `index.html` so client-side routes survive a reload.

use std::io::Read;
use std::sync::OnceLock;

use axum::{
    body::Body,
    http::{HeaderMap, HeaderValue, Method, StatusCode, Uri, header},
    response::{IntoResponse, Response},
};
use bytes::Bytes;
use flate2::read::GzDecoder;

enum AssetData {
    Raw(&'static [u8]),
    Compressed {
        br: &'static [u8],
        gz: &'static [u8],
    },
}

struct Asset {
    path: &'static str,
    mime: &'static str,
    etag: &'static str,
    is_html: bool,
    immutable: bool,
    data: AssetData,
    identity: OnceLock<Bytes>,
}

include!(concat!(env!("OUT_DIR"), "/dist_assets.rs"));

fn lookup(path: &str) -> Option<&'static Asset> {
    ASSETS
        .binary_search_by_key(&path, |a| a.path)
        .ok()
        .map(|i| &ASSETS[i])
}

/// Everything the dashboard needs comes from this origin. Inline styles are
/// allowed because UI libraries position popups with them; scripts are not.
/// Forms may also post to the GitHub host, for the App manifest flow.
static CSP: OnceLock<HeaderValue> = OnceLock::new();

/// Set the policy once at startup; `github_web` is an origin like `https://github.com`.
pub fn init_csp(github_web: &str) {
    let policy = format!(
        "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; \
         img-src 'self' data: blob: https:; font-src 'self' data:; connect-src 'self'; object-src 'none'; \
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
    if !path.is_empty()
        && let Some(asset) = lookup(path)
    {
        return respond(asset, &headers);
    }
    // A missing file with an extension is a real 404; anything else is a route.
    let last = path.rsplit('/').next().unwrap_or_default();
    if last.contains('.') {
        return StatusCode::NOT_FOUND.into_response();
    }
    match lookup("index.html") {
        Some(asset) => respond(asset, &headers),
        None => (StatusCode::OK, NOT_BUILT).into_response(),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PreferredEncoding {
    Brotli,
    Gzip,
    Identity,
}

fn negotiate_encoding(headers: &HeaderMap) -> PreferredEncoding {
    let mut br_q = 0.0f32;
    let mut gz_q = 0.0f32;
    for val in headers.get_all(header::ACCEPT_ENCODING) {
        let Ok(s) = val.to_str() else { continue };
        for part in s.split(',') {
            let mut segs = part.split(';');
            let Some(enc) = segs.next().map(str::trim) else {
                continue;
            };
            let mut q = 1.0f32;
            for param in segs {
                if let Some(qv) = param.trim().strip_prefix("q=")
                    && let Ok(parsed) = qv.trim().parse::<f32>()
                {
                    q = parsed;
                }
            }
            if enc.eq_ignore_ascii_case("br") {
                br_q = br_q.max(q);
            } else if enc.eq_ignore_ascii_case("gzip") {
                gz_q = gz_q.max(q);
            }
        }
    }
    if br_q > 0.0 && br_q >= gz_q {
        PreferredEncoding::Brotli
    } else if gz_q > 0.0 {
        PreferredEncoding::Gzip
    } else {
        PreferredEncoding::Identity
    }
}

fn inflate_gzip(gz: &'static [u8]) -> Bytes {
    let mut dec = GzDecoder::new(gz);
    let mut out = Vec::with_capacity(gz.len().saturating_mul(3));
    if dec.read_to_end(&mut out).is_ok() {
        Bytes::from(out)
    } else {
        Bytes::from_static(gz)
    }
}

fn respond(asset: &'static Asset, headers: &HeaderMap) -> Response {
    let mut res = Response::builder()
        .header(header::CONTENT_TYPE, HeaderValue::from_static(asset.mime))
        .header(header::ETAG, HeaderValue::from_static(asset.etag))
        .header(
            header::CACHE_CONTROL,
            HeaderValue::from_static(if asset.immutable {
                "public, max-age=31536000, immutable"
            } else {
                "no-cache"
            }),
        );
    if asset.is_html {
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
        .is_some_and(|v| v.split(',').any(|tag| tag.trim() == asset.etag));
    if fresh {
        if matches!(asset.data, AssetData::Compressed { .. }) {
            res = res.header(header::VARY, HeaderValue::from_static("accept-encoding"));
        }
        return res
            .status(StatusCode::NOT_MODIFIED)
            .body(Body::empty())
            .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response());
    }
    let body = match asset.data {
        AssetData::Raw(bytes) => Body::from(Bytes::from_static(bytes)),
        AssetData::Compressed { br, gz } => {
            res = res.header(header::VARY, HeaderValue::from_static("accept-encoding"));
            match negotiate_encoding(headers) {
                PreferredEncoding::Brotli => {
                    res = res.header(header::CONTENT_ENCODING, HeaderValue::from_static("br"));
                    Body::from(Bytes::from_static(br))
                }
                PreferredEncoding::Gzip => {
                    res = res.header(header::CONTENT_ENCODING, HeaderValue::from_static("gzip"));
                    Body::from(Bytes::from_static(gz))
                }
                PreferredEncoding::Identity => {
                    let raw = asset.identity.get_or_init(|| inflate_gzip(gz));
                    Body::from(raw.clone())
                }
            }
        }
    };
    res.body(body)
        .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::to_bytes;

    #[test]
    fn accept_encoding_negotiation_respects_q_values() {
        let mut h = HeaderMap::new();
        assert_eq!(negotiate_encoding(&h), PreferredEncoding::Identity);

        h.insert(header::ACCEPT_ENCODING, HeaderValue::from_static("gzip, deflate, br"));
        assert_eq!(negotiate_encoding(&h), PreferredEncoding::Brotli);

        h.insert(header::ACCEPT_ENCODING, HeaderValue::from_static("gzip, br;q=0"));
        assert_eq!(negotiate_encoding(&h), PreferredEncoding::Gzip);

        h.insert(header::ACCEPT_ENCODING, HeaderValue::from_static("gzip;q=0, br;q=0"));
        assert_eq!(negotiate_encoding(&h), PreferredEncoding::Identity);
    }

    #[tokio::test]
    async fn serves_precompressed_br_gz_and_inflated_identity_with_etag() {
        let Some(asset) = lookup("index.html") else {
            return;
        };
        let uri: Uri = "/".parse().unwrap();

        let mut br_hdr = HeaderMap::new();
        br_hdr.insert(header::ACCEPT_ENCODING, HeaderValue::from_static("gzip, br"));
        let br_res = serve(Method::GET, uri.clone(), br_hdr).await;
        assert_eq!(br_res.status(), StatusCode::OK);
        assert_eq!(
            br_res.headers().get(header::CONTENT_ENCODING).and_then(|v| v.to_str().ok()),
            Some("br")
        );
        let etag = br_res.headers().get(header::ETAG).unwrap().to_str().unwrap().to_owned();
        assert_eq!(etag, asset.etag);

        let mut gz_hdr = HeaderMap::new();
        gz_hdr.insert(header::ACCEPT_ENCODING, HeaderValue::from_static("gzip"));
        let gz_res = serve(Method::GET, uri.clone(), gz_hdr).await;
        assert_eq!(
            gz_res.headers().get(header::CONTENT_ENCODING).and_then(|v| v.to_str().ok()),
            Some("gzip")
        );

        let id_res = serve(Method::GET, uri.clone(), HeaderMap::new()).await;
        assert!(id_res.headers().get(header::CONTENT_ENCODING).is_none());
        let body = to_bytes(id_res.into_body(), 1024 * 1024).await.unwrap();
        assert!(String::from_utf8_lossy(&body).contains("<!doctype html>") || String::from_utf8_lossy(&body).contains("<html"));

        let mut inm = HeaderMap::new();
        inm.insert(header::IF_NONE_MATCH, HeaderValue::from_str(&etag).unwrap());
        let not_mod = serve(Method::GET, uri, inm).await;
        assert_eq!(not_mod.status(), StatusCode::NOT_MODIFIED);
    }
}
