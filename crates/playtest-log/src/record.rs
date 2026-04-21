//! Tagged union for log lines: header, event, final.
//!
//! Wire format (JSONL, one record per line):
//!
//! ```text
//! {"kind":"header","schema":2,"game":"cribbage","version":"0.1.0", ...}
//! {"kind":"event","tick":0,"payload":{"DealCard":{"player":0,"card":"AH"}}}
//! ...
//! {"kind":"final","winner":0,"reason":"Victory","scores":[121,98],"finished_at":1712345678901}
//! ```

use playtest_core::{EndReason, GameResult, PlayerId};
use playtest_ports::UnixMillis;
use serde::{Deserialize, Serialize};

use crate::header::LogHeader;

/// One parsed line of an event log, generic over the game's event type.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LogRecord<E> {
    /// First line of every log. Present exactly once.
    Header(LogHeader),

    /// An atomic game event. `tick` is a monotonically increasing index
    /// starting at 0; together with the header's `seed` it's enough to
    /// replay the log deterministically.
    Event { tick: u64, payload: E },

    /// Last line of every completed log. Present at most once; missing
    /// if the game crashed mid-play (the log can still be replayed up
    /// to the last committed event).
    ///
    /// `finished_at` is the wall-clock time the game ended, in Unix
    /// epoch milliseconds (from the [`Clock`](playtest_ports::Clock)
    /// port). Paired with [`LogHeader::started_at`] it yields the
    /// `wall_clock_ms` built-in metric. Defaults to `0` when missing
    /// (e.g. v1 logs written before the field existed).
    Final {
        winner: Option<PlayerId>,
        reason: EndReason,
        scores: Vec<i32>,
        #[serde(default)]
        finished_at: UnixMillis,
    },
}

impl<E> LogRecord<E> {
    /// Build a `Final` record from a `GameResult` and the wall-clock
    /// timestamp captured when the loop finished.
    #[must_use]
    pub fn final_from_result(result: &GameResult, finished_at: UnixMillis) -> Self {
        Self::Final {
            winner: result.winner,
            reason: result.reason.clone(),
            scores: result.scores.clone(),
            finished_at,
        }
    }

    /// Inverse of [`Self::final_from_result`] — pull a `GameResult` out
    /// of a `Final` record. Discards `finished_at`; callers that need it
    /// should destructure the enum directly.
    #[must_use]
    pub fn as_result(&self) -> Option<GameResult> {
        match self {
            Self::Final {
                winner,
                reason,
                scores,
                ..
            } => Some(GameResult {
                winner: *winner,
                reason: reason.clone(),
                scores: scores.clone(),
            }),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::{Deserialize, Serialize};

    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
    struct Ping {
        n: u32,
    }

    #[test]
    fn event_record_roundtrips_via_json() {
        let rec: LogRecord<Ping> = LogRecord::Event {
            tick: 7,
            payload: Ping { n: 42 },
        };
        let line = serde_json::to_string(&rec).unwrap();
        assert!(line.starts_with("{\"kind\":\"event\""), "got: {line}");
        assert!(line.contains("\"tick\":7"));
        let back: LogRecord<Ping> = serde_json::from_str(&line).unwrap();
        assert_eq!(rec, back);
    }

    #[test]
    fn header_record_serializes_with_kind_field() {
        let rec: LogRecord<Ping> = LogRecord::Header(LogHeader {
            schema: crate::header::SCHEMA_VERSION,
            game: "tally".into(),
            version: "0.0.0".into(),
            seed: 1,
            agents: vec!["random".into(), "random".into()],
            started_at: 1_700_000_000_000,
            config_hash: "deadbeef".repeat(8),
        });
        let line = serde_json::to_string(&rec).unwrap();
        assert!(line.starts_with("{\"kind\":\"header\""), "got: {line}");
        let back: LogRecord<Ping> = serde_json::from_str(&line).unwrap();
        assert_eq!(rec, back);
    }

    #[test]
    fn final_record_can_be_built_from_and_inverted_to_game_result() {
        let orig = GameResult {
            winner: Some(1),
            reason: EndReason::Victory,
            scores: vec![98, 121],
        };
        let rec: LogRecord<Ping> = LogRecord::final_from_result(&orig, 1_700_000_001_234);
        let back = rec.as_result().unwrap();
        assert_eq!(back, orig);
        // `finished_at` survives a JSON round-trip so metrics can read it.
        let line = serde_json::to_string(&rec).unwrap();
        assert!(line.contains("\"finished_at\":1700000001234"));
        let back_rec: LogRecord<Ping> = serde_json::from_str(&line).unwrap();
        match back_rec {
            LogRecord::Final { finished_at, .. } => assert_eq!(finished_at, 1_700_000_001_234),
            _ => panic!("expected Final"),
        }
    }

    #[test]
    fn final_record_without_finished_at_defaults_to_zero() {
        // Backward-compat: a Final line written by v1 code (no
        // `finished_at` field) must still parse — wall_clock_ms for
        // those logs will be absent rather than poisoned.
        let v1_line = r#"{"kind":"final","winner":0,"reason":"Victory","scores":[121,98]}"#;
        let rec: LogRecord<Ping> = serde_json::from_str(v1_line).unwrap();
        match rec {
            LogRecord::Final { finished_at, .. } => assert_eq!(finished_at, 0),
            _ => panic!("expected Final"),
        }
    }
}
