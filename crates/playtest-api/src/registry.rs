//! Types for the `/registry` endpoints: telling the frontend which
//! games and agent kinds the server knows about.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;

/// One registered game, returned by `GET /registry/games`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct GameRegistryEntry {
    /// Stable id used in [`crate::runs::CreateRunRequest::game`]
    /// (e.g. `"cribbage"`).
    pub id: String,

    /// Human-readable label for UI menus.
    pub display_name: String,

    /// JSON Schema describing the shape of this game's `config`
    /// blob. Carried as raw JSON so the API crate does not have to
    /// depend on any particular schema library at runtime.
    pub config_schema: JsonValue,
}

/// One registered agent kind, returned by `GET /registry/agents`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct AgentRegistryEntry {
    /// Stable id used in [`crate::runs::CreateRunRequest::agents`]
    /// (e.g. `"random"`).
    pub id: String,

    /// Human-readable label for UI menus.
    pub display_name: String,

    /// Registered game ids this agent kind supports. Empty means
    /// "game-agnostic" (usable with any game).
    #[serde(default)]
    pub supported_games: Vec<String>,
}
