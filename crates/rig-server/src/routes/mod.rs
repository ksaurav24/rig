pub mod account;
mod adopt;
mod alerts;
pub mod apps;
pub mod auth;
mod databases;
pub mod devices;
mod events;
pub mod github;
pub mod hooks;
pub mod members;
mod metrics;
pub(crate) mod org;
mod projects;
mod snapshot;
pub mod system;
mod terminal;
mod uploads;

use std::net::SocketAddr;

use axum::{
    Router,
    extract::{ConnectInfo, Request, State},
    http::{HeaderValue, header},
    middleware::Next,
    response::{IntoResponse, Response},
    routing::get,
};
use bytes::Bytes;
use tower_http::{
    compression::CompressionLayer, limit::RequestBodyLimitLayer,
    set_header::SetResponseHeaderLayer, trace::TraceLayer,
};
use utoipa::OpenApi;
use utoipa_axum::router::OpenApiRouter;
use utoipa_scalar::Scalar;

use crate::{
    auth::{Auth, Viewer, is_https},
    state::AppState,
};

/// Pinned Scalar release used only when `--enable-api-docs` is explicitly turned on.
const SCALAR_HTML: &str = r#"<!doctype html>
<html>
<head>
  <title>Rig API Reference</title>
  <meta charset="utf-8" />
  <meta name="viewport" content="width=device-width, initial-scale=1" />
</head>
<body>
  <script id="api-reference" type="application/json">$spec</script>
  <script src="https://cdn.jsdelivr.net/npm/@scalar/api-reference@1.28.11/dist/browser/standalone.min.js" crossorigin="anonymous"></script>
</body>
</html>"#;

const DOCS_CSP: &str = "default-src 'self'; script-src 'self' https://cdn.jsdelivr.net; \
    style-src 'self' 'unsafe-inline' https://cdn.jsdelivr.net; \
    img-src 'self' data: https:; font-src 'self' data: https://cdn.jsdelivr.net; \
    connect-src 'self'; object-src 'none'; base-uri 'self'; frame-ancestors 'none'";

async fn hsts_middleware(State(state): State<AppState>, req: Request, next: Next) -> Response {
    let peer = req
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|c| c.0.ip());
    let https = is_https(&state, req.headers(), peer);
    let mut res = next.run(req).await;
    if https {
        res.headers_mut().entry(header::STRICT_TRANSPORT_SECURITY).or_insert(
            HeaderValue::from_static("max-age=31536000; includeSubDomains"),
        );
    }
    res
}

#[derive(OpenApi)]
#[openapi(
    components(schemas(rig_api::LogLine, rig_api::ServerEvent)),
    info(
        title = "Rig API",
        description = "Self-hosted deployment platform. Authenticate with the session cookie \
                   (plus `x-rig-csrf` on writes) or `Authorization: Bearer rig_...`."
    )
)]
struct ApiDoc;

fn api() -> OpenApiRouter<AppState> {
    OpenApiRouter::with_openapi(ApiDoc::openapi())
        .merge(auth::router())
        .merge(account::router())
        .merge(devices::router())
        .merge(events::router())
        .merge(org::router())
        .merge(projects::router())
        .merge(apps::router())
        .merge(databases::router())
        .merge(members::router())
        .merge(system::router())
        .merge(metrics::router())
        .merge(terminal::router())
        .merge(hooks::router())
        .merge(github::router())
        .merge(alerts::router())
        .merge(uploads::json_router())
        .merge(snapshot::json_router())
        .merge(adopt::router())
}

/// The OpenAPI document, without building a server. Used by `rig-server openapi`.
pub fn openapi() -> utoipa::openapi::OpenApi {
    api().into_openapi()
}

pub fn app(state: AppState) -> Router {
    let (mut router, spec) = api().split_for_parts();
    let spec_bytes = Bytes::from(
        serde_json::to_vec(&spec).expect("serializing OpenAPI spec"),
    );
    router = router.route(
        "/api/openapi.json",
        get(move |_auth: Auth<Viewer>| {
            let spec_bytes = spec_bytes.clone();
            async move {
                (
                    [(
                        header::CONTENT_TYPE,
                        HeaderValue::from_static("application/json"),
                    )],
                    spec_bytes,
                )
            }
        }),
    );
    if state.config.enable_api_docs {
        let html = Bytes::from(Scalar::new(spec).custom_html(SCALAR_HTML).to_html());
        router = router.route(
            "/api/docs",
            get(move |_auth: Auth<Viewer>| {
                let html = html.clone();
                async move {
                    (
                        [
                            (
                                header::CONTENT_TYPE,
                                HeaderValue::from_static("text/html; charset=utf-8"),
                            ),
                            (
                                header::CONTENT_SECURITY_POLICY,
                                HeaderValue::from_static(DOCS_CSP),
                            ),
                        ],
                        html,
                    )
                        .into_response()
                }
            }),
        );
    }
    let router = if state.config.mode == rig_api::ServerMode::Full {
        router.fallback(crate::dashboard::serve)
    } else {
        router
    };
    router
        .layer(RequestBodyLimitLayer::new(2 * 1024 * 1024))
        .merge(uploads::streaming_router())
        .merge(snapshot::streaming_router())
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            crate::access::guard,
        ))
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            hsts_middleware,
        ))
        .with_state(state)
        .layer(SetResponseHeaderLayer::if_not_present(
            header::X_CONTENT_TYPE_OPTIONS,
            HeaderValue::from_static("nosniff"),
        ))
        .layer(SetResponseHeaderLayer::if_not_present(
            header::X_FRAME_OPTIONS,
            HeaderValue::from_static("DENY"),
        ))
        .layer(SetResponseHeaderLayer::if_not_present(
            header::HeaderName::from_static("x-rig-version"),
            HeaderValue::from_static(env!("CARGO_PKG_VERSION")),
        ))
        .layer(SetResponseHeaderLayer::if_not_present(
            header::REFERRER_POLICY,
            HeaderValue::from_static("same-origin"),
        ))
        .layer(CompressionLayer::new())
        .layer(TraceLayer::new_for_http())
}
