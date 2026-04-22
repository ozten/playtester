//! `ScratchBuffer`: the LLM agent's persistent per-turn memory.
//!
//! Three slots, all owned by the agent (see `Scratch buffer lives in the
//! agent, not the engine` in the Phase 3 plan's Key Technical Decisions):
//!
//! - `plan` — free-form strategic intent for the current hand.
//! - `notes` — free-form tactical notes.
//! - `turn_log` — a bounded rolling window of one-line records of what
//!   the agent has done this game. Cap of `MAX_TURN_LOG` keeps the
//!   per-turn prompt bounded regardless of game length.
//!
//! The buffer is `Serialize` so it can be dropped into the user-message
//! JSON as-is.

use serde::{Deserialize, Serialize};

/// Maximum number of entries retained in `turn_log`. Older entries are
/// dropped from the front on overflow.
pub const MAX_TURN_LOG: usize = 64;

/// Per-agent persistent memory passed to the LLM on every turn.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ScratchBuffer {
    pub plan: String,
    pub notes: String,
    pub turn_log: Vec<String>,
}

impl ScratchBuffer {
    /// Construct an empty buffer.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Push a line onto the turn log, dropping the oldest entries so the
    /// log never exceeds [`MAX_TURN_LOG`].
    pub fn push_turn_log(&mut self, line: String) {
        self.turn_log.push(line);
        while self.turn_log.len() > MAX_TURN_LOG {
            self.turn_log.remove(0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_is_empty() {
        let s = ScratchBuffer::new();
        assert!(s.plan.is_empty());
        assert!(s.notes.is_empty());
        assert!(s.turn_log.is_empty());
    }

    #[test]
    fn push_turn_log_caps_at_max() {
        let mut s = ScratchBuffer::new();
        for i in 0..(MAX_TURN_LOG + 20) {
            s.push_turn_log(format!("line {i}"));
        }
        assert_eq!(s.turn_log.len(), MAX_TURN_LOG);
        // The oldest entries were dropped — the 20th line through the
        // last line remain.
        assert_eq!(s.turn_log.first().unwrap(), "line 20");
        assert_eq!(
            s.turn_log.last().unwrap(),
            &format!("line {}", MAX_TURN_LOG + 19)
        );
    }

    #[test]
    fn serializes_as_a_single_object() {
        let mut s = ScratchBuffer::new();
        s.plan = "keep aces".into();
        s.notes = "opponent pegs aggressively".into();
        s.push_turn_log("tick=0 seat=0 chose index=1".into());
        let json = serde_json::to_value(&s).unwrap();
        assert_eq!(json["plan"], "keep aces");
        assert_eq!(json["notes"], "opponent pegs aggressively");
        assert_eq!(json["turn_log"].as_array().unwrap().len(), 1);
    }
}
