//! "What did this game look like?" metrics: end phase, dealer-won,
//! nibs rate, lead changes, final-score margin.

use playtest_metrics::{MetricDef, MetricKind, MetricScope, MetricValue, MetricValueKind};
use uuid::Uuid;

use crate::metrics::accumulator::Accumulator;
use crate::phase::Phase;

pub const GAME_ENDED_IN_PHASE: &str = "game_ended_in_phase";
pub const GAME_WINNER_WAS_DEALER: &str = "game_winner_was_dealer";
pub const CUTS_PRODUCING_NIBS: &str = "cuts_producing_nibs";
pub const LEAD_CHANGES: &str = "lead_changes";
pub const FINAL_SCORE_MARGIN: &str = "final_score_margin";

/// Metric definitions owned by this module.
#[must_use]
pub fn definitions() -> Vec<MetricDef> {
    vec![
        MetricDef {
            name: GAME_ENDED_IN_PHASE.into(),
            kind: MetricKind::Tag,
            scope: MetricScope::Game,
            description: "Phase the EndGame event fired in: \"pegging\", \"show\", \"crib_count\", or \"unfinished\"."
                .into(),
        },
        MetricDef {
            name: GAME_WINNER_WAS_DEALER.into(),
            kind: MetricKind::Bool,
            scope: MetricScope::Game,
            description: "True if the winner was the dealer of the final hand; false otherwise. Absent when the log has no EndGame event."
                .into(),
        },
        MetricDef {
            name: CUTS_PRODUCING_NIBS.into(),
            kind: MetricKind::Count,
            scope: MetricScope::Game,
            description: "Number of hands in this game where the starter card was a Jack (nibs awarded).".into(),
        },
        MetricDef {
            name: LEAD_CHANGES.into(),
            kind: MetricKind::Count,
            scope: MetricScope::Game,
            description: "Number of times the leading player flipped during the game. Ties do not count as a flip.".into(),
        },
        MetricDef {
            name: FINAL_SCORE_MARGIN.into(),
            kind: MetricKind::Count,
            scope: MetricScope::Game,
            description: "winner_score - loser_score at game end. Absent when the log has no EndGame event.".into(),
        },
    ]
}

/// Emit all game-shape metric values for one accumulated game.
#[must_use]
pub fn extract(game_id: Uuid, acc: &Accumulator) -> Vec<MetricValue> {
    let mut out = Vec::new();

    out.push(MetricValue {
        game_id,
        metric_name: GAME_ENDED_IN_PHASE.into(),
        player: None,
        value: MetricValueKind::Tag(phase_tag(acc.end_phase).into()),
    });

    // cuts_producing_nibs: one per hand whose starter was a jack.
    let nibs_hands = acc.hands.iter().filter(|h| h.nibs_awarded).count();
    out.push(MetricValue {
        game_id,
        metric_name: CUTS_PRODUCING_NIBS.into(),
        player: None,
        value: MetricValueKind::Count(i64::try_from(nibs_hands).unwrap_or(i64::MAX)),
    });

    out.push(MetricValue {
        game_id,
        metric_name: LEAD_CHANGES.into(),
        player: None,
        value: MetricValueKind::Count(i64::from(acc.lead_changes)),
    });

    // winner-dependent metrics are absent when no EndGame.
    if let Some(winner) = acc.winner {
        // The dealer at end-of-game is whichever hand the EndGame fired
        // in. That's the last `HandRecord` in the accumulator.
        let final_dealer = acc.hands.last().map(|h| h.dealer);
        if let Some(dealer) = final_dealer {
            out.push(MetricValue {
                game_id,
                metric_name: GAME_WINNER_WAS_DEALER.into(),
                player: None,
                value: MetricValueKind::Bool(winner == dealer),
            });
        }

        let margin = winner_margin(acc, winner);
        out.push(MetricValue {
            game_id,
            metric_name: FINAL_SCORE_MARGIN.into(),
            player: None,
            value: MetricValueKind::Count(i64::from(margin)),
        });
    }

    out
}

fn phase_tag(phase: Option<Phase>) -> &'static str {
    match phase {
        Some(Phase::Pegging) => "pegging",
        Some(Phase::Show) => "show",
        // `ScoreCrib` is a future distinct phase; the live state
        // machine runs crib counting inside `Show` and emits the
        // final ShowScored(is_crib=true) before the EndGame event.
        // Map both to `crib_count` when EndGame fired on a crib score.
        Some(Phase::ScoreCrib) => "crib_count",
        Some(Phase::Finished) => "finished",
        Some(Phase::Deal | Phase::Discard | Phase::Cut) => "pre_pegging",
        None => "unfinished",
    }
}

fn winner_margin(acc: &Accumulator, winner: u8) -> u16 {
    let widx = usize::from(winner);
    let lidx = 1 - widx;
    let w = acc.final_scores.get(widx).copied().unwrap_or(0);
    let l = acc.final_scores.get(lidx).copied().unwrap_or(0);
    w.saturating_sub(l)
}
