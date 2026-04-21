//! Registry endpoints — list the games and agents the server can dispatch.
//!
//! Backed by `playtest_cli::game_registry::KNOWN_GAMES` and
//! `playtest_cli::agent_registry::KNOWN_AGENTS` so the HTTP surface
//! and the CLI stay in lockstep.

use axum::{Json, Router, routing::get};
use playtest_api::{AgentRegistryEntry, ApiResponse, GameRegistryEntry};
use serde_json::json;

use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/games-registry", get(games_registry))
        .route("/api/agents-registry", get(agents_registry))
}

async fn games_registry() -> Json<ApiResponse<Vec<GameRegistryEntry>>> {
    let entries: Vec<GameRegistryEntry> = playtest_registry::game_registry::KNOWN_GAMES
        .iter()
        .map(|id| GameRegistryEntry {
            id: (*id).to_owned(),
            display_name: (*id).to_owned(),
            // The `Config` schemas are not yet exposed by the engine
            // crates — ship an empty schema for now. Frontend should
            // treat `{}` as "no schema available".
            config_schema: json!({}),
        })
        .collect();
    Json(ApiResponse::ok(entries))
}

async fn agents_registry() -> Json<ApiResponse<Vec<AgentRegistryEntry>>> {
    let entries: Vec<AgentRegistryEntry> = playtest_registry::agent_registry::KNOWN_AGENTS
        .iter()
        .map(|id| AgentRegistryEntry {
            id: (*id).to_owned(),
            display_name: (*id).to_owned(),
            // Empty = game-agnostic, which is accurate for `random`.
            supported_games: Vec::new(),
        })
        .collect();
    Json(ApiResponse::ok(entries))
}
