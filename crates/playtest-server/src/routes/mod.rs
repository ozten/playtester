//! Axum route handlers and router assembly.
//!
//! Every handler returns an [`ApiResponse<T>`](playtest_api::ApiResponse)
//! envelope; errors map through [`ApiError`](playtest_api::ApiError) and
//! [`http_status`](playtest_api::http_status) so the HTTP status is
//! derived from the wire contract rather than invented locally.

use axum::Router;

use crate::state::AppState;

pub mod games;
pub mod health;
pub mod registry;
pub mod reports;
pub mod runs;

/// Build the full axum router for the playtester API.
pub fn build_router(state: AppState) -> Router {
    Router::new()
        .merge(health::router())
        .merge(registry::router())
        .merge(runs::router())
        .merge(games::router())
        .merge(reports::router())
        .with_state(state)
}

/// Convenience type alias for handlers that return a wrapped
/// `ApiResponse<T>` on success or an `(StatusCode, ApiResponse<()>)`
/// pair on failure.
pub type ApiResult<T> = Result<
    axum::Json<playtest_api::ApiResponse<T>>,
    (
        axum::http::StatusCode,
        axum::Json<playtest_api::ApiResponse<()>>,
    ),
>;

/// Wrap a single [`ApiError`](playtest_api::ApiError) in the standard
/// failure envelope plus its mapped HTTP status.
pub fn api_error(
    err: playtest_api::ApiError,
) -> (
    axum::http::StatusCode,
    axum::Json<playtest_api::ApiResponse<()>>,
) {
    let status = axum::http::StatusCode::from_u16(playtest_api::http_status(err.code))
        .unwrap_or(axum::http::StatusCode::INTERNAL_SERVER_ERROR);
    (
        status,
        axum::Json(playtest_api::ApiResponse::fail(vec![err])),
    )
}
