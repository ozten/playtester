//! Metric *definitions* — the schema a game or harness declares at
//! startup. Separate from [`crate::MetricValue`] (the per-game emitted
//! values) so reports can list every known metric even if zero games
//! have emitted values yet.

use serde::{Deserialize, Serialize};

/// Static description of a metric the harness knows how to persist.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MetricDef {
    /// Unique identifier, e.g. `game_length_ticks`. No two metrics in
    /// a single registry may share a name.
    pub name: String,

    /// Value shape ([`MetricKind::Scalar`] / `Count` / `Tag` / `Bool`).
    /// The emitted [`crate::MetricValueKind`] must match.
    pub kind: MetricKind,

    /// Whether this metric is emitted once per game
    /// ([`MetricScope::Game`]) or once per (game, player)
    /// ([`MetricScope::Player`]).
    pub scope: MetricScope,

    /// Human-readable one-liner. Shown in reports.
    pub description: String,
}

/// The shape of values a metric emits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MetricKind {
    /// A real-valued scalar. Good for rates, averages, entropies.
    Scalar,
    /// A signed integer count. Good for "how many times did X happen".
    Count,
    /// A string tag. Good for categorical outcomes ("Victory", "Draw").
    Tag,
    /// A boolean. Good for per-game flags ("dealer won").
    Bool,
}

/// Whether a metric attaches to the whole game or to a specific player.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MetricScope {
    /// Exactly one value per game; [`crate::MetricValue::player`] is `None`.
    Game,
    /// One value per (game, player); [`crate::MetricValue::player`] is `Some`.
    Player,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metric_def_serde_roundtrip() {
        let def = MetricDef {
            name: "game_length_ticks".into(),
            kind: MetricKind::Count,
            scope: MetricScope::Game,
            description: "Total number of events in the game log.".into(),
        };
        let json = serde_json::to_string(&def).unwrap();
        let back: MetricDef = serde_json::from_str(&json).unwrap();
        assert_eq!(def, back);
    }

    #[test]
    fn metric_kind_serializes_as_snake_case() {
        assert_eq!(
            serde_json::to_string(&MetricKind::Scalar).unwrap(),
            "\"scalar\""
        );
        assert_eq!(
            serde_json::to_string(&MetricKind::Count).unwrap(),
            "\"count\""
        );
        assert_eq!(serde_json::to_string(&MetricKind::Tag).unwrap(), "\"tag\"");
        assert_eq!(
            serde_json::to_string(&MetricKind::Bool).unwrap(),
            "\"bool\""
        );
    }

    #[test]
    fn metric_scope_serializes_as_snake_case() {
        assert_eq!(
            serde_json::to_string(&MetricScope::Game).unwrap(),
            "\"game\""
        );
        assert_eq!(
            serde_json::to_string(&MetricScope::Player).unwrap(),
            "\"player\""
        );
    }
}
