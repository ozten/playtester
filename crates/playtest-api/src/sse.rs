//! Server-Sent Events frame shape.
//!
//! The server streams frames over `text/event-stream`. Each frame's
//! JSON body is an [`SseFrame`] value — a tagged union where `kind`
//! identifies the variant and `data` (when present) carries the
//! pre-serialized JSON from the event log. This lets the server hand
//! the log's JSON straight through without re-serializing it.
//!
//! Only four variants exist by design: `Lagged` and `Shutdown`
//! (discussed during planning) were omitted because the localhost-only
//! scope makes the broadcaster buffer ample, and graceful shutdown is
//! communicated via connection close instead of an in-band frame.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;

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

    /// Keep-alive tick so proxies do not time out idle streams.
    /// Carries no payload.
    Heartbeat,
}
