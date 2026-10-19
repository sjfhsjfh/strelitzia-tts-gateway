use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

use crate::state::AppState;

#[utoipa::path(
    get,
    path = "/healthz",
    responses((status = 200, description = "Service is alive", body = String)),
    tag = "health",
    security(())
)]
pub async fn healthz() -> &'static str {
    "ok"
}

#[utoipa::path(
    get,
    path = "/readyz",
    responses(
        (status = 200, description = "Database is reachable", body = String),
        (status = 503, description = "Database is unavailable", body = String)
    ),
    tag = "health",
    security(())
)]
pub async fn readyz(State(state): State<AppState>) -> Response {
    match state.db().channels().list(true).await {
        Ok(_) => (StatusCode::OK, "ok").into_response(),
        Err(error) => {
            tracing::warn!(%error, "readiness probe failed");
            (StatusCode::SERVICE_UNAVAILABLE, "unavailable").into_response()
        }
    }
}

#[utoipa::path(
    get,
    path = "/openapi.json",
    responses((status = 200, description = "OpenAPI document", body = inline(serde_json::Value))),
    tag = "health",
    security(())
)]
pub async fn openapi_json() -> Json<utoipa::openapi::OpenApi> {
    Json(crate::openapi::openapi())
}
