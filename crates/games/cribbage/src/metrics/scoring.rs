//! Scoring-breakdown metrics: where did points come from?
//!
//! Each game contributes one sample per player; a reporter built on
//! Unit 14's SQLite schema averages across games later. The underlying
//! values here are totals for *this game*, not multi-game averages —
//! averaging is a query-time concern.

use playtest_core::PlayerId;
use playtest_metrics::{MetricDef, MetricKind, MetricScope, MetricValue, MetricValueKind};
use uuid::Uuid;

use crate::metrics::accumulator::Accumulator;

pub const HAND_SCORE_TOTAL: &str = "hand_score_total";
pub const CRIB_SCORE_TOTAL: &str = "crib_score_total";
pub const PEGGING_SCORE_TOTAL: &str = "pegging_score_total";
pub const PEGGING_SHARE_OF_TOTAL: &str = "pegging_share_of_total";
pub const NIBS_CONTRIBUTION: &str = "nibs_contribution";
pub const DECISIONS_PER_PLAYER: &str = "decisions_per_player";

/// Metric definitions owned by this module.
#[must_use]
pub fn definitions() -> Vec<MetricDef> {
    vec![
        MetricDef {
            name: HAND_SCORE_TOTAL.into(),
            kind: MetricKind::Count,
            scope: MetricScope::Player,
            description: "Total show-phase hand points this player scored across all hands in this game.".into(),
        },
        MetricDef {
            name: CRIB_SCORE_TOTAL.into(),
            kind: MetricKind::Count,
            scope: MetricScope::Player,
            description: "Total crib points credited to this player while they were dealer.".into(),
        },
        MetricDef {
            name: PEGGING_SCORE_TOTAL.into(),
            kind: MetricKind::Count,
            scope: MetricScope::Player,
            description: "Total pegging-phase points (15/31/pair/run/last-card) this player scored this game.".into(),
        },
        MetricDef {
            name: PEGGING_SHARE_OF_TOTAL.into(),
            kind: MetricKind::Scalar,
            scope: MetricScope::Player,
            description: "pegging_score_total / final_score for this player. Absent when final_score is 0.".into(),
        },
        MetricDef {
            name: NIBS_CONTRIBUTION.into(),
            kind: MetricKind::Count,
            scope: MetricScope::Player,
            description: "Nibs (his-heels) points this player received as dealer. Typically 0 or 2 per hand.".into(),
        },
        MetricDef {
            name: DECISIONS_PER_PLAYER.into(),
            kind: MetricKind::Count,
            scope: MetricScope::Player,
            description: "Number of agent decisions this player made (one per DiscardToCrib, PegPlayed, or Go).".into(),
        },
    ]
}

/// Emit scoring-breakdown values for both players.
#[must_use]
pub fn extract(game_id: Uuid, acc: &Accumulator) -> Vec<MetricValue> {
    let mut out = Vec::with_capacity(12);
    for (idx, player) in [0u8, 1u8].into_iter().enumerate() {
        let pid: Option<PlayerId> = Some(player);
        let hand = acc.show_hand_points[idx];
        let crib = acc.crib_points[idx];
        let peg = acc.pegging_points[idx];
        let nibs = acc.nibs_points[idx];
        let decisions = acc.decisions[idx];

        out.push(MetricValue {
            game_id,
            metric_name: HAND_SCORE_TOTAL.into(),
            player: pid,
            tag: None,
            value: MetricValueKind::Count(i64::from(hand)),
        });
        out.push(MetricValue {
            game_id,
            metric_name: CRIB_SCORE_TOTAL.into(),
            player: pid,
            tag: None,
            value: MetricValueKind::Count(i64::from(crib)),
        });
        out.push(MetricValue {
            game_id,
            metric_name: PEGGING_SCORE_TOTAL.into(),
            player: pid,
            tag: None,
            value: MetricValueKind::Count(i64::from(peg)),
        });
        out.push(MetricValue {
            game_id,
            metric_name: NIBS_CONTRIBUTION.into(),
            player: pid,
            tag: None,
            value: MetricValueKind::Count(i64::from(nibs)),
        });
        out.push(MetricValue {
            game_id,
            metric_name: DECISIONS_PER_PLAYER.into(),
            player: pid,
            tag: None,
            value: MetricValueKind::Count(i64::from(decisions)),
        });

        // pegging_share_of_total = peg / (hand + crib + peg + nibs).
        // If the denominator is 0 the share is undefined — omit rather
        // than report 0.0.
        let total = u64::from(hand) + u64::from(crib) + u64::from(peg) + u64::from(nibs);
        if total > 0 {
            #[allow(
                clippy::cast_precision_loss,
                reason = "scorecard values fit in f64 trivially"
            )]
            let share = f64::from(peg) / (total as f64);
            out.push(MetricValue {
                game_id,
                metric_name: PEGGING_SHARE_OF_TOTAL.into(),
                player: pid,
                tag: None,
                value: MetricValueKind::Scalar(share),
            });
        }
    }
    out
}
