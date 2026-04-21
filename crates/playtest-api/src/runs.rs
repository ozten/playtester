//! Types for the `/runs` endpoints: creating, listing, and polling
//! self-play runs.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;

/// Request body for `POST /runs`.
///
/// `config` is carried as untyped JSON so this crate stays
/// game-agnostic; the server parses it against the named game's
/// schema before accepting the run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CreateRunRequest {
    /// Registered game id (e.g. `"cribbage"`).
    pub game: String,

    /// Ordered list of agent kind ids, one per seat. Length must
    /// match the game's required player count.
    pub agents: Vec<String>,

    /// How many games to play in this run. Must be >= 1.
    pub games_count: u32,

    /// Optional base seed. `None` means "pick one at run start".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seed: Option<u64>,

    /// Game-specific configuration blob. Shape is defined by the
    /// target game's `config_schema`
    /// (see [`crate::registry::GameRegistryEntry`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub config: Option<JsonValue>,
}

/// Lifecycle state of a run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum RunStatus {
    /// Accepted by the server but not yet started.
    Pending,

    /// Currently executing games.
    Running,

    /// All games finished successfully.
    Completed,

    /// Terminated early due to an error.
    Failed,
}

/// Summary row returned by `GET /runs` and `GET /runs/:id`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RunSummary {
    /// Server-assigned run id.
    pub id: String,

    /// Registered game id this run is playing.
    pub game: String,

    /// Agent kind ids, one per seat.
    pub agents: Vec<String>,

    /// Total games requested.
    pub games_count: u32,

    /// Games that have finished (successfully or not) so far.
    pub games_completed: u32,

    /// Base seed chosen for the run.
    pub seed: u64,

    /// Current lifecycle state.
    pub status: RunStatus,

    /// Wall-clock time the run was created, Unix epoch milliseconds.
    pub created_at: u64,

    /// Wall-clock time the run finished, Unix epoch milliseconds.
    /// `None` while `status` is `Pending` or `Running`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<u64>,
}
