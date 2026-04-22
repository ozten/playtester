//! Wire frames for the stdio protocol.
//!
//! Two directions, both newline-delimited JSON:
//!
//! - **Agent -> child:** a single [`TurnFrame`] per turn, carrying the
//!   public view, the legal-action slice, the agent's [`ScratchBuffer`],
//!   plus `api_version` + `game` so version/game mismatches surface on
//!   first contact (no separate hello handshake).
//! - **Child -> agent:** a [`ReplyFrame`] — either an `action` frame
//!   containing the chosen index + optional scratch edits, or an
//!   `error` frame surfacing a child-side failure.
//!
//! Field names are stable — the Python reference client and any
//! user-authored subprocess depend on this shape.

use serde::{Deserialize, Serialize};

use crate::llm::ScratchBuffer;

/// API version stamped on every [`TurnFrame`].
///
/// Version mismatches are detected implicitly: a child that parses a
/// different `api_version` and replies accordingly is expected to emit
/// an `error` frame. No separate hello handshake — the turn frame is
/// the handshake.
pub const STDIO_API_VERSION: &str = "3.0.0";

/// Agent -> child: one frame emitted per turn.
///
/// The generic `V` and `A` match the game's `PublicView` and `Action`
/// types respectively; both must be `Serialize`.
#[derive(Debug, Clone, Serialize)]
pub struct TurnFrame<V, A>
where
    V: Serialize,
    A: Serialize,
{
    /// Frame discriminator — always the literal `"turn"`.
    pub kind: &'static str,
    /// Protocol version, always [`STDIO_API_VERSION`]. Present on every
    /// frame so child processes can reject on the first incompatible
    /// turn rather than after a hello round-trip.
    pub api_version: &'static str,
    /// Game name (e.g. `"cribbage"`, `"shipwreck"`) — lets the child
    /// dispatch rules/view schemas.
    pub game: String,
    /// Seat index the child is playing.
    pub seat: u8,
    /// Monotonically increasing per-agent id; child must echo it.
    pub prompt_id: u64,
    /// Redacted, player-visible snapshot.
    pub view: V,
    /// The enumerated legal actions for this turn. The child replies
    /// with an index into this slice.
    pub legal_actions: Vec<A>,
    /// The agent's persistent per-turn memory (plan / notes /
    /// rolling turn log).
    pub scratch: ScratchBuffer,
}

/// Child -> agent: a one-line JSON reply.
///
/// Tagged on `kind` with `"action"` or `"error"`.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ReplyFrame {
    /// Child chose a legal action.
    Action {
        /// Must equal the `prompt_id` the agent sent.
        prompt_id: u64,
        /// Index into the turn frame's `legal_actions`.
        action_index: usize,
        /// Optional scratch edits the agent should persist.
        #[serde(default)]
        scratch: ReplyScratch,
    },
    /// Child-side failure; agent maps to `AgentError::Other`.
    Error {
        /// Echoed prompt id for correlation.
        #[serde(default)]
        prompt_id: u64,
        /// Human-readable explanation.
        message: String,
    },
}

/// Scratch subset a child can update on the agent side.
///
/// The child only gets to push `plan` and `notes`; `turn_log` is
/// managed by the agent (the agent decides what counts as a turn).
#[derive(Debug, Clone, Default, Deserialize)]
pub struct ReplyScratch {
    #[serde(default)]
    pub plan: String,
    #[serde(default)]
    pub notes: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn turn_frame_serializes_with_expected_fields() {
        let frame = TurnFrame {
            kind: "turn",
            api_version: STDIO_API_VERSION,
            game: "cribbage".into(),
            seat: 1,
            prompt_id: 7,
            view: json!({"phase": "discard"}),
            legal_actions: vec![json!({"Discard": [0, 1]})],
            scratch: ScratchBuffer::default(),
        };
        let v = serde_json::to_value(&frame).unwrap();
        assert_eq!(v["kind"], "turn");
        assert_eq!(v["api_version"], "3.0.0");
        assert_eq!(v["game"], "cribbage");
        assert_eq!(v["seat"], 1);
        assert_eq!(v["prompt_id"], 7);
        assert!(v["view"].is_object());
        assert!(v["legal_actions"].is_array());
        assert!(v["scratch"].is_object());
    }

    #[test]
    fn reply_frame_action_deserializes() {
        let raw = r#"{"kind":"action","prompt_id":3,"action_index":2,"scratch":{"plan":"p","notes":"n"}}"#;
        let r: ReplyFrame = serde_json::from_str(raw).unwrap();
        match r {
            ReplyFrame::Action {
                prompt_id,
                action_index,
                scratch,
            } => {
                assert_eq!(prompt_id, 3);
                assert_eq!(action_index, 2);
                assert_eq!(scratch.plan, "p");
                assert_eq!(scratch.notes, "n");
            }
            ReplyFrame::Error { .. } => panic!("expected Action frame"),
        }
    }

    #[test]
    fn reply_frame_action_scratch_is_optional() {
        let raw = r#"{"kind":"action","prompt_id":3,"action_index":0}"#;
        let r: ReplyFrame = serde_json::from_str(raw).unwrap();
        match r {
            ReplyFrame::Action { scratch, .. } => {
                assert!(scratch.plan.is_empty());
                assert!(scratch.notes.is_empty());
            }
            ReplyFrame::Error { .. } => panic!("expected Action frame"),
        }
    }

    #[test]
    fn reply_frame_error_deserializes() {
        let raw = r#"{"kind":"error","prompt_id":5,"message":"boom"}"#;
        let r: ReplyFrame = serde_json::from_str(raw).unwrap();
        match r {
            ReplyFrame::Error { prompt_id, message } => {
                assert_eq!(prompt_id, 5);
                assert_eq!(message, "boom");
            }
            ReplyFrame::Action { .. } => panic!("expected Error frame"),
        }
    }
}
