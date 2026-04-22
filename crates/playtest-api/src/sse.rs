//! Server-Sent Events frame shape.
//!
//! The server streams frames over `text/event-stream`. Each frame's
//! JSON body is an [`SseFrame`] value — a tagged union where `kind`
//! identifies the variant and `data` (when present) carries the
//! pre-serialized JSON from the event log. This lets the server hand
//! the log's JSON straight through without re-serializing it.
//!
//! Phase 2.5 (api_version 1.1.0) adds a fifth variant: `TurnPrompt`.
//! It is **ephemeral** — not in the JSONL log, not resumable via
//! `Last-Event-ID`. The server re-emits it on reconnect by reading the
//! coordinator's pending-prompt state, not by replaying a log line.
//! See `docs/api-contract.md` for the wire semantics.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;

/// Payload carried by [`SseFrame::TurnPrompt`]. A remote agent at `seat`
/// is waiting for a submission; the client should POST an action_index
/// into the legal list. Echo `prompt_id` back so the server can reject
/// stale submissions.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct TurnPromptPayload {
    /// Zero-based seat index the prompt is addressed to.
    pub seat: u8,

    /// Per-game monotonic id; echo on submission so the server can
    /// reject stale prompts (e.g., the game advanced before the client
    /// submitted).
    pub prompt_id: u64,

    /// Legal actions indexed 0..N, each serialized as the game's
    /// `Action` type. The client picks one index and submits it.
    pub legal_actions: Vec<JsonValue>,
}

/// One frame in an SSE stream.
///
/// Serialized with `#[serde(tag = "kind", content = "data")]`, so
/// the JSON shape is `{"kind":"event","data":{...}}` /
/// `{"kind":"heartbeat"}` / etc.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", content = "data", rename_all = "snake_case")]
pub enum SseFrame {
    /// First frame on every stream. `data` is the JSON object for the
    /// log's header record.
    Header(JsonValue),

    /// An in-progress game event. `data` is the JSON object for one
    /// `Event` record from the log.
    Event(JsonValue),

    /// Last frame when a game ends cleanly. `data` is the JSON
    /// object for the log's `Final` record.
    Final(JsonValue),

    /// A remote agent is waiting for a submission. Ephemeral — not in
    /// the JSONL log, not resumable via `Last-Event-ID`. On reconnect
    /// the server re-emits this frame from the coordinator's pending
    /// state when the game is still waiting.
    TurnPrompt(TurnPromptPayload),

    /// Keep-alive tick so proxies do not time out idle streams.
    /// Carries no payload.
    Heartbeat,
}
