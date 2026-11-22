use axum::{
    Json,
    extract::{Path, Query, State},
};
use rig_api::{ErrorBody, MetricRange, MetricSeries};
use serde::Deserialize;
use utoipa::IntoParams;
use utoipa_axum::{router::OpenApiRouter, routes};

use crate::{
    auth::{Auth, Viewer},
    error::{ApiError, ApiResult},
    metrics,
    state::AppState,
};

pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(host_metrics))
        .routes(routes!(app_metrics))
        .routes(routes!(database_metrics))
}

#[derive(Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
struct RangeQuery {
    /// How far back to look. Defaults to one hour.
    #[serde(default)]
    range: MetricRange,
}

/// CPU and memory use of the server over time.
#[utoipa::path(get, path = "/api/v1/system/metrics", tag = "system", params(RangeQuery),
    responses((status = 200, body = MetricSeries)))]
async fn host_metrics(
    State(state): State<AppState>,
    _auth: Auth<Viewer>,
    Query(q): Query<RangeQuery>,
) -> ApiResult<Json<MetricSeries>> {
    Ok(Json(metrics::series(&state, metrics::HOST, q.range).await?))
}

/// CPU and memory use of an app's container over time.
#[utoipa::path(get, path = "/api/v1/apps/{id}/metrics", tag = "apps",
    params(("id" = String, Path), RangeQuery),
    responses((status = 200, body = MetricSeries), (status = 404, body = ErrorBody)))]
async fn app_metrics(
    State(state): State<AppState>,
    auth: Auth<Viewer>,
    Path(id): Path<String>,
    Query(q): Query<RangeQuery>,
) -> ApiResult<Json<MetricSeries>> {
    owned(
        &state,
        "SELECT 1 FROM apps WHERE id = ? AND org_id = ?",
        &id,
        &auth.org_id,
        "app",
    )
    .await?;
    Ok(Json(
        metrics::series(&state, &metrics::app_scope(&id), q.range).await?,
    ))
}

/// CPU and memory use of a database's container over time.
#[utoipa::path(get, path = "/api/v1/databases/{id}/metrics", tag = "databases",
    params(("id" = String, Path), RangeQuery),
    responses((status = 200, body = MetricSeries), (status = 404, body = ErrorBody)))]
async fn database_metrics(
    State(state): State<AppState>,
    auth: Auth<Viewer>,
    Path(id): Path<String>,
    Query(q): Query<RangeQuery>,
) -> ApiResult<Json<MetricSeries>> {
    owned(
        &state,
        "SELECT 1 FROM databases WHERE id = ? AND org_id = ?",
        &id,
        &auth.org_id,
        "database",
    )
    .await?;
    Ok(Json(
        metrics::series(&state, &metrics::database_scope(&id), q.range).await?,
    ))
}

/// Metric scopes are keyed by id alone, so check the id belongs to the caller's
/// organization before reading them.
async fn owned(
    state: &AppState,
    sql: &'static str,
    id: &str,
    org_id: &str,
    what: &'static str,
) -> ApiResult<()> {
    sqlx::query_scalar::<_, i64>(sql)
        .bind(id)
        .bind(org_id)
        .fetch_optional(&state.db)
        .await?
        .map(|_| ())
        .ok_or(ApiError::NotFound(what))
}
