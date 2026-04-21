//! `/api/reports` — stubbed for this unit.
//!
//! The routes are scaffolded and mounted so the OpenAPI dump in
//! Unit 18 is complete and frontend contract compilers can see the
//! shape, but the handlers always return
//! [`ApiErrorCode::NotImplemented`] with HTTP 501.
//!
//! TODO(unit-later): implement report generation end-to-end. The plan
//! originally listed reports as part of Unit 17; deferring the real
//! implementation keeps Unit 17 shippable. Reports are Unit 27 scope
//! or later — see the Phase 2 plan.

use axum::{Router, extract::Path, routing::{get, post}};
use playtest_api::{ApiError, ApiErrorCode};

use crate::routes::{ApiResult, api_error};
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/reports", post(create_report))
        .route("/api/reports/{report_id}", get(get_report))
        .route("/api/reports/{report_id}/markdown", get(get_report_markdown))
}

// TODO(unit-later): real `POST /api/reports` — takes a `run_id`,
// builds a markdown report, returns `{report_id, status}`.
async fn create_report() -> ApiResult<serde_json::Value> {
    Err(api_error(ApiError::new(
        ApiErrorCode::NotImplemented,
        "POST /api/reports is not yet implemented; see Unit 27 in the Phase 2 plan",
    )))
}

// TODO(unit-later): real `GET /api/reports/:id` — return report metadata.
async fn get_report(Path(_report_id): Path<String>) -> ApiResult<serde_json::Value> {
    Err(api_error(ApiError::new(
        ApiErrorCode::NotImplemented,
        "GET /api/reports/:id is not yet implemented; see Unit 27 in the Phase 2 plan",
    )))
}

// TODO(unit-later): real `GET /api/reports/:id/markdown` — return raw markdown.
async fn get_report_markdown(Path(_report_id): Path<String>) -> ApiResult<serde_json::Value> {
    Err(api_error(ApiError::new(
        ApiErrorCode::NotImplemented,
        "GET /api/reports/:id/markdown is not yet implemented; see Unit 27 in the Phase 2 plan",
    )))
}
