//! `GET /api/health` — liveness probe plus wire-contract version.

use axum::{Json, Router, routing::get};
use playtest_api::{API_VERSION, ApiResponse};
use serde::{Deserialize, Serialize};

use crate::state::AppState;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HealthBody {
    pub status: &'static str,
    pub api_version: &'static str,
}

pub fn router() -> Router<AppState> {
    Router::new().route("/api/health", get(health))
}

async fn health() -> Json<ApiResponse<HealthBody>> {
    Json(ApiResponse::ok(HealthBody {
        status: "ok",
        api_version: API_VERSION,
    }))
}
