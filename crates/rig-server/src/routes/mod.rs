pub mod account;
mod alerts;
pub mod apps;
pub mod auth;
mod databases;
pub mod github;
pub mod hooks;
mod members;
mod metrics;
mod org;
mod projects;
pub mod system;
mod terminal;

use axum::{
    Json, Router,
    http::{HeaderValue, header},
    routing::get,
};
use tower_http::{
    compression::CompressionLayer, limit::RequestBodyLimitLayer,
    set_header::SetResponseHeaderLayer, trace::TraceLayer,
};
use utoipa::OpenApi;
use utoipa_axum::router::OpenApiRouter;
use utoipa_scalar::{Scalar, Servable};

use crate::state::AppState;

#[derive(OpenApi)]
#[openapi(
    components(schemas(rig_api::LogLine)),
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
}

/// The OpenAPI document, without building a server. Used by `rig-server openapi`.
pub fn openapi() -> utoipa::openapi::OpenApi {
    api().into_openapi()
}

pub fn app(state: AppState) -> Router {
    let (router, spec) = api().split_for_parts();
    let spec_json = spec.clone();
    router
        .route(
            "/api/openapi.json",
            get(move || async move { Json(spec_json) }),
        )
        .merge(Scalar::with_url("/api/docs", spec))
        .fallback(crate::dashboard::serve)
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            crate::access::guard,
        ))
        .with_state(state)
        .layer(RequestBodyLimitLayer::new(2 * 1024 * 1024))
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
