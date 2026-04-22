//! Uniform error shape plus HTTP-status mapping.
//!
//! Every error the server returns uses [`ApiError`], so TypeScript
//! consumers have one error path to handle. The [`ApiErrorCode`] enum
//! is the stable, machine-readable taxonomy; [`http_status`] maps
//! each variant to its HTTP status code.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;

/// Machine-readable error taxonomy.
///
/// Any variant added here must also be assigned an HTTP status in
/// [`http_status`]. The compiler enforces exhaustiveness.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum ApiErrorCode {
    /// The `game` field in a request did not name a registered game.
    UnknownGame,

    /// An `agents[_]` entry did not name a registered agent kind.
    UnknownAgent,

    /// The `config` blob failed validation against the target game's
    /// config schema.
    InvalidConfig,

    /// The requested run id was not found (or has been evicted).
    RunNotFound,

    /// The requested game id / log file was not found.
    GameNotFound,

    /// An `offset` or `limit` query parameter was negative, zero where
    /// positive is required, or exceeds server-enforced ceilings.
    InvalidPaginationParams,

    /// An unexpected server-side failure. The `message` is safe to
    /// show users; `details` (if populated) is meant for operators.
    Internal,

    /// The endpoint is scaffolded but not yet implemented. Used for
    /// routes shipped as stubs ahead of their full implementation
    /// (e.g. `/api/reports` in Unit 17, proper implementation deferred
    /// to a later unit).
    NotImplemented,

    /// `POST .../actions` was submitted with a `prompt_id` that does
    /// not match the currently-pending prompt. The game advanced
    /// before the submission landed. Fetch the latest `turn_prompt`
    /// via the SSE stream and retry with the new `prompt_id`. Phase
    /// 2.5.
    StaleTick,

    /// `POST .../actions` supplied an `action_index` outside the range
    /// of the pending prompt's `legal_actions`. Phase 2.5.
    IllegalActionIndex,

    /// `POST .../actions` arrived with no prompt pending for this
    /// seat — either the game has not reached the seat yet or the
    /// last prompt was already satisfied. Phase 2.5.
    NotYourTurn,

    /// `POST .../actions` targeted a seat that is not backed by an
    /// `http-remote` agent. AI-only seats cannot be driven via HTTP.
    /// Phase 2.5.
    NoRemoteAgentAtSeat,

    /// A `POST /api/runs` payload named an agent kind that is CLI-only
    /// in the current phase — today `llm` and `stdio`. Server callers
    /// must use `playtest play --agents ...` from the command line.
    /// Phase 3.
    AgentKindNotAllowedHere,
}

/// Uniform error body, shared by every endpoint.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ApiError {
    /// Machine-readable code — clients branch on this, not on `message`.
    pub code: ApiErrorCode,

    /// Human-readable, user-facing explanation.
    pub message: String,

    /// Optional structured payload (field path, offending value, etc.).
    /// Shape is error-specific; carried as untyped JSON so the API
    /// crate does not need to enumerate every possibility.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub details: Option<JsonValue>,
}

impl ApiError {
    /// Build an error with no `details` payload.
    #[must_use]
    pub fn new(code: ApiErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            details: None,
        }
    }

    /// Build an error carrying a structured `details` payload.
    #[must_use]
    pub fn with_details(
        code: ApiErrorCode,
        message: impl Into<String>,
        details: JsonValue,
    ) -> Self {
        Self {
            code,
            message: message.into(),
            details: Some(details),
        }
    }
}

/// Map each [`ApiErrorCode`] to its HTTP status code.
///
/// Kept here (not in `playtest-server`) so the mapping is part of the
/// wire contract and cannot drift between the OpenAPI spec and the
/// server implementation.
#[must_use]
pub fn http_status(code: ApiErrorCode) -> u16 {
    match code {
        ApiErrorCode::UnknownGame
        | ApiErrorCode::UnknownAgent
        | ApiErrorCode::InvalidConfig
        | ApiErrorCode::InvalidPaginationParams
        | ApiErrorCode::StaleTick
        | ApiErrorCode::IllegalActionIndex
        | ApiErrorCode::NotYourTurn
        | ApiErrorCode::NoRemoteAgentAtSeat
        | ApiErrorCode::AgentKindNotAllowedHere => 400,
        ApiErrorCode::RunNotFound | ApiErrorCode::GameNotFound => 404,
        ApiErrorCode::Internal => 500,
        ApiErrorCode::NotImplemented => 501,
    }
}
