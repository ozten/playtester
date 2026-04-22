//! Types for the `/games` endpoints: browsing saved game event logs.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;

/// Short summary row returned by `GET /games`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct GameSummary {
    /// Stable game id (usually derived from log filename).
    pub id: String,

    /// Run id this game belongs to, if known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,

    /// Registered game id (e.g. `"cribbage"`).
    pub game: String,

    /// Wall-clock time the game started, Unix epoch milliseconds.
    pub started_at: u64,

    /// Wall-clock time the game finished, Unix epoch milliseconds.
    /// `None` if the log has no `Final` record (crashed mid-play).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<u64>,

    /// Winning player seat, if the game ended cleanly.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub winner: Option<u32>,
}

/// Full metadata row returned by `GET /games/:id`. Mirrors the JSON
/// contents of a log's header plus a few server-side additions.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct GameMetadata {
    /// Summary row fields.
    #[serde(flatten)]
    pub summary: GameSummary,

    /// Log schema version (from `LogHeader.schema`).
    pub schema: u32,

    /// Engine/build version stamped at write time.
    pub version: String,

    /// Base seed the run was started with.
    pub seed: u64,

    /// Hex-encoded hash of the game config.
    pub config_hash: String,

    /// Agent kind ids, one per seat, from the header.
    pub agents: Vec<String>,

    /// Final scores per seat, if the game ended cleanly.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scores: Option<Vec<i32>>,
}

/// One record from an event log, carried to the client as the
/// already-serialized JSON object. Using `JsonValue` keeps the API
/// crate game-agnostic — the exact shape of `event.payload` is
/// defined by the game crate that produced the log.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct LogLineDto {
    /// Kind discriminant as it appears on the JSONL line
    /// (`"header"`, `"event"`, or `"final"`).
    pub kind: String,

    /// The full JSON object for this line. Callers parse this
    /// against the game's event type to recover typed values.
    pub line: JsonValue,
}

/// Paginated page of log lines returned by `GET /games/:id/events`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct EventPage {
    /// Zero-based offset of the first record in `events`.
    pub offset: u64,

    /// Maximum page size that was requested.
    pub limit: u32,

    /// Total number of records in the log (header + events + final).
    pub total: u64,

    /// The page's records, in on-disk order.
    pub events: Vec<LogLineDto>,
}

/// Body of `POST /api/runs/{run_id}/games/{game_id}/actions` — the
/// inbound path Phase 2.5 introduces for browser-driven play.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SubmitActionBody {
    /// Zero-based seat index the action is for. Must match the seat
    /// of the currently-pending prompt.
    pub seat: u8,

    /// The `prompt_id` from the `turn_prompt` frame being answered.
    /// Prevents stale submissions after the game has advanced.
    pub prompt_id: u64,

    /// Zero-based index into the prompt's `legal_actions` list.
    pub action_index: u32,
}

/// Response body of a successful action submission. Minimal by design
/// — the next `event` frame on the SSE stream is the meaningful
/// signal that the submission was applied.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SubmitActionResponse {
    /// Always `true` on a 200 response; kept so the shape is a JSON
    /// object rather than a bare `null`.
    pub accepted: bool,
}

impl SubmitActionResponse {
    /// The standard success body for a submitted action.
    #[must_use]
    pub fn ok() -> Self {
        Self { accepted: true }
    }
}
