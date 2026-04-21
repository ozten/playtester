//! Per-player ShipWreck metrics: final rescue points, raft length,
//! and starvation-event counts.
//!
//! Emitted as `MetricScope::Player` values — the reporter aggregates
//! across the batch (average rescue points for seat 0, seat 1, …).
//! SQL rollups key on `(metric_name, player)` so we do *not* fan out
//! via the metric *name* (e.g. `rescue_points_player_0`), we fan out
//! via the `player` column.

use playtest_metrics::{MetricDef, MetricKind, MetricScope, MetricValue, MetricValueKind};
use uuid::Uuid;

use crate::metrics::accumulator::Accumulator;

pub const PLAYER_RESCUE_POINTS: &str = "player_rescue_points";
pub const PLAYER_RAFT_LENGTH: &str = "player_raft_length";
pub const PLAYER_INVENTION_COUNT: &str = "player_invention_count";
pub const PLAYER_FOOD_STARVATION_EVENTS: &str = "player_food_starvation_events";

/// Metric definitions owned by this module.
#[must_use]
pub fn definitions() -> Vec<MetricDef> {
    vec![
        MetricDef {
            name: PLAYER_RESCUE_POINTS.into(),
            kind: MetricKind::Scalar,
            scope: MetricScope::Player,
            description:
                "Final rescue points for this player, summed across placed player cards. Emitted once per (game, player)."
                    .into(),
        },
        MetricDef {
            name: PLAYER_RAFT_LENGTH.into(),
            kind: MetricKind::Count,
            scope: MetricScope::Player,
            description:
                "Final raft length (base cards + extensions) for this player at game end."
                    .into(),
        },
        MetricDef {
            name: PLAYER_INVENTION_COUNT.into(),
            kind: MetricKind::Count,
            scope: MetricScope::Player,
            description:
                "Final count of equipment upgrades installed on this player's raft at game end."
                    .into(),
        },
        MetricDef {
            name: PLAYER_FOOD_STARVATION_EVENTS.into(),
            kind: MetricKind::Count,
            scope: MetricScope::Player,
            description:
                "Count of FoodConsumed{starved:true} events for this player (cards that fell off the raft)."
                    .into(),
        },
    ]
}

/// Emit all per-player metric values for one accumulated game.
#[must_use]
pub fn extract(game_id: Uuid, acc: &Accumulator) -> Vec<MetricValue> {
    let mut out = Vec::new();

    // Starvation counts are always available — even on a log that
    // never reached EndGame, the counts still make sense.
    for (i, count) in acc.starvation_per_player.iter().enumerate() {
        let player = u8::try_from(i).unwrap_or(u8::MAX);
        out.push(MetricValue {
            game_id,
            metric_name: PLAYER_FOOD_STARVATION_EVENTS.into(),
            player: Some(player),
            tag: None,
            value: MetricValueKind::Count(i64::try_from(*count).unwrap_or(i64::MAX)),
        });
    }

    // Final score rows — rescue points, raft length, invention count.
    // Only present when the game reached EndGame (or similar) and the
    // `final_scores` were recorded.
    for row in &acc.final_scores {
        out.push(MetricValue {
            game_id,
            metric_name: PLAYER_RESCUE_POINTS.into(),
            player: Some(row.player),
            tag: None,
            value: MetricValueKind::Scalar(f64::from(row.rescue_points)),
        });
        out.push(MetricValue {
            game_id,
            metric_name: PLAYER_RAFT_LENGTH.into(),
            player: Some(row.player),
            tag: None,
            value: MetricValueKind::Count(i64::from(row.raft_length)),
        });
        out.push(MetricValue {
            game_id,
            metric_name: PLAYER_INVENTION_COUNT.into(),
            player: Some(row.player),
            tag: None,
            value: MetricValueKind::Count(i64::from(row.invention_count)),
        });
    }

    out
}
