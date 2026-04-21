//! Helpers for converting raw JSONL log lines into SSE frames.
//!
//! The per-game SSE stream carries three kinds of payload (header,
//! event, final) plus a keep-alive heartbeat. The log's wire format is
//! the same tagged JSONL the writer produces, so this module just
//! peeks at the `kind` discriminator and re-packages the line as the
//! matching [`SseFrame`] variant without re-serialising the inner
//! payload.

use playtest_api::SseFrame;
use serde_json::Value as JsonValue;

/// Parse a JSONL line and return `(tick_id, SseFrame)`.
///
/// `tick_id` is the `id:` value used on the SSE wire. For
/// `Event { tick }` lines it's the tick; for the header it's `0`, and
/// for `Final` it's the next tick after the last event (approximated
/// by the caller — we don't have a reader position here, so we use
/// `None` and let the caller supply a monotonic id).
///
/// # Errors
/// Returns `None` if the line is not valid JSON or lacks a recognised
/// `kind` field — callers drop such lines with a warning.
#[must_use]
pub fn line_to_sse_frame(line: &str) -> Option<(Option<u64>, SseFrame)> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return None;
    }
    let value: JsonValue = serde_json::from_str(trimmed).ok()?;
    let kind = value.get("kind")?.as_str()?;
    match kind {
        "header" => Some((Some(0), SseFrame::Header(value))),
        "event" => {
            let tick = value.get("tick").and_then(JsonValue::as_u64);
            Some((tick, SseFrame::Event(value)))
        }
        "final" => Some((None, SseFrame::Final(value))),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_header_with_id_zero() {
        let line = r#"{"kind":"header","schema":2,"game":"x","seed":7,"agents":[],"started_at":0,"version":"0.0.0","config_hash":""}"#;
        let (id, frame) = line_to_sse_frame(line).unwrap();
        assert_eq!(id, Some(0));
        assert!(matches!(frame, SseFrame::Header(_)));
    }

    #[test]
    fn parses_event_with_tick_id() {
        let line = r#"{"kind":"event","tick":42,"payload":{"A":1}}"#;
        let (id, frame) = line_to_sse_frame(line).unwrap();
        assert_eq!(id, Some(42));
        assert!(matches!(frame, SseFrame::Event(_)));
    }

    #[test]
    fn parses_final_with_no_id_hint() {
        let line = r#"{"kind":"final","winner":0,"reason":"Victory","scores":[121,98]}"#;
        let (id, frame) = line_to_sse_frame(line).unwrap();
        assert_eq!(id, None);
        assert!(matches!(frame, SseFrame::Final(_)));
    }

    #[test]
    fn unknown_or_malformed_line_returns_none() {
        assert!(line_to_sse_frame("").is_none());
        assert!(line_to_sse_frame("not json").is_none());
        assert!(line_to_sse_frame(r#"{"kind":"mystery"}"#).is_none());
        assert!(line_to_sse_frame(r#"{"foo":"bar"}"#).is_none());
    }
}
