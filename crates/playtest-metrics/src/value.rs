//! Per-game emitted metric values. Registries produce these by
//! extracting from a [`crate::GameLog`]; ingestion persists them into
//! SQLite (Unit 14).

use playtest_core::PlayerId;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// One emitted metric value for a single game.
///
/// `game_id` is supplied by the caller at extraction time — the event
/// log does not carry a UUID (that would break determinism invariants
/// on re-play). The ingestion pipeline generates or looks up a stable
/// game id when it reads each log file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MetricValue {
    pub game_id: Uuid,

    /// Must match an entry in the emitting registry's
    /// [`crate::MetricDef::name`].
    pub metric_name: String,

    /// `Some(pid)` for [`crate::MetricScope::Player`] metrics;
    /// `None` for game-level metrics.
    pub player: Option<PlayerId>,

    /// Optional categorical sub-key. Used when a single metric name is
    /// fanned out across discrete categories — Cribbage's per-card
    /// metrics emit one `MetricValue` per `(metric_name, rank)` pair
    /// using the rank symbol (`"A"`, `"2"`, …, `"K"`) as the tag. The
    /// Unit 14 SQLite schema has a matching `tag` column, so tagged
    /// values land with no transformation.
    ///
    /// `None` when the metric is not tagged.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tag: Option<String>,

    pub value: MetricValueKind,
}

/// The value shape of a single [`MetricValue`]. Must match the
/// emitting metric's [`crate::MetricKind`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum MetricValueKind {
    Scalar(f64),
    Count(i64),
    Tag(String),
    Bool(bool),
}

impl MetricValueKind {
    /// Discriminator matching [`crate::MetricKind`], for cross-checking
    /// against a [`crate::MetricDef`].
    #[must_use]
    pub const fn discriminator(&self) -> crate::MetricKind {
        match self {
            Self::Scalar(_) => crate::MetricKind::Scalar,
            Self::Count(_) => crate::MetricKind::Count,
            Self::Tag(_) => crate::MetricKind::Tag,
            Self::Bool(_) => crate::MetricKind::Bool,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metric_value_serde_roundtrip_each_kind() {
        let g = Uuid::new_v4();
        let cases = [
            MetricValue {
                game_id: g,
                metric_name: "game_length_ticks".into(),
                player: None,
                tag: None,
                value: MetricValueKind::Count(142),
            },
            MetricValue {
                game_id: g,
                metric_name: "winner".into(),
                player: None,
                tag: None,
                value: MetricValueKind::Tag("player_0".into()),
            },
            MetricValue {
                game_id: g,
                metric_name: "dealer_won".into(),
                player: None,
                tag: None,
                value: MetricValueKind::Bool(true),
            },
            MetricValue {
                game_id: g,
                metric_name: "pegging_efficiency".into(),
                player: Some(0),
                tag: None,
                value: MetricValueKind::Scalar(0.375),
            },
            MetricValue {
                game_id: g,
                metric_name: "card_kept_count".into(),
                player: Some(1),
                tag: Some("5".into()),
                value: MetricValueKind::Count(3),
            },
        ];
        for mv in cases {
            let json = serde_json::to_string(&mv).unwrap();
            let back: MetricValue = serde_json::from_str(&json).unwrap();
            assert_eq!(mv, back);
        }
    }

    #[test]
    fn untagged_metric_value_omits_tag_from_json() {
        let v = MetricValue {
            game_id: Uuid::nil(),
            metric_name: "x".into(),
            player: None,
            tag: None,
            value: MetricValueKind::Count(1),
        };
        let json = serde_json::to_string(&v).unwrap();
        assert!(
            !json.contains("\"tag\""),
            "expected tag to be omitted, got: {json}"
        );
    }

    #[test]
    fn discriminator_matches_kind() {
        use crate::MetricKind;
        assert_eq!(
            MetricValueKind::Scalar(1.0).discriminator(),
            MetricKind::Scalar
        );
        assert_eq!(MetricValueKind::Count(1).discriminator(), MetricKind::Count);
        assert_eq!(
            MetricValueKind::Tag("x".into()).discriminator(),
            MetricKind::Tag
        );
        assert_eq!(
            MetricValueKind::Bool(false).discriminator(),
            MetricKind::Bool
        );
    }
}
