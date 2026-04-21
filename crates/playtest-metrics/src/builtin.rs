//! Always-on, game-agnostic metrics. Every registered game picks
//! these up automatically — no game crate needs to redeclare them.
//!
//! These come from pieces of the log that every game emits: the
//! header (seed, agent names), the event stream's length, and the
//! final record (winner, scores). Metrics that *would* be nice to
//! include but require game-specific or schema-extending information
//! are deliberately deferred:
//!
//! - `wall_clock_ms` — needs a `finished_at` field in the Final record,
//!   or per-event timestamps. Neither exists yet.
//! - `decisions_per_player` — needs to distinguish player-action
//!   events from engine events, which is game-specific (Cribbage's
//!   `PegPlayed` / `DiscardToCrib` / `Go` vs `DealCard` / `PegScored`).
//!   [`CribbageMetrics`] in Unit 13 implements it.

use playtest_core::{Game, PlayerId};
use serde::de::DeserializeOwned;
use uuid::Uuid;

use crate::definition::{MetricDef, MetricKind, MetricScope};
use crate::log::GameLog;
use crate::registry::MetricRegistry;
use crate::value::{MetricValue, MetricValueKind};

/// Game-agnostic metrics that apply to every game's log.
#[derive(Debug, Default, Clone, Copy)]
pub struct BuiltInMetrics;

impl BuiltInMetrics {
    pub const GAME_LENGTH_TICKS: &'static str = "game_length_ticks";
    pub const WINNER: &'static str = "winner";
    pub const END_REASON: &'static str = "end_reason";
    pub const SCORE_MARGIN: &'static str = "score_margin";
    pub const AGENT_NAME: &'static str = "agent_name";
    pub const FINAL_SCORE: &'static str = "final_score";
}

impl<G: Game> MetricRegistry<G> for BuiltInMetrics
where
    G::Event: DeserializeOwned,
{
    fn metric_definitions(&self) -> Vec<MetricDef> {
        vec![
            MetricDef {
                name: BuiltInMetrics::GAME_LENGTH_TICKS.into(),
                kind: MetricKind::Count,
                scope: MetricScope::Game,
                description: "Total number of events in the game log.".into(),
            },
            MetricDef {
                name: BuiltInMetrics::WINNER.into(),
                kind: MetricKind::Tag,
                scope: MetricScope::Game,
                description:
                    "Winning player id (\"player_0\" / \"player_1\" / ... / \"draw\") or \"unfinished\" if the log lacks a Final record."
                        .into(),
            },
            MetricDef {
                name: BuiltInMetrics::END_REASON.into(),
                kind: MetricKind::Tag,
                scope: MetricScope::Game,
                description:
                    "How the game ended (\"victory\" / \"draw\" / \"stalemate\" / custom), or \"unfinished\" if missing."
                        .into(),
            },
            MetricDef {
                name: BuiltInMetrics::SCORE_MARGIN.into(),
                kind: MetricKind::Count,
                scope: MetricScope::Game,
                description: "abs(max_score - min_score) at game end; 0 if no final record.".into(),
            },
            MetricDef {
                name: BuiltInMetrics::AGENT_NAME.into(),
                kind: MetricKind::Tag,
                scope: MetricScope::Player,
                description: "Name of the agent controlling this player, from the log header.".into(),
            },
            MetricDef {
                name: BuiltInMetrics::FINAL_SCORE.into(),
                kind: MetricKind::Count,
                scope: MetricScope::Player,
                description:
                    "Final score for this player (signed; games may allow negative scores). 0 if no final record."
                        .into(),
            },
        ]
    }

    fn extract(&self, game_id: Uuid, log: &GameLog<G>) -> Vec<MetricValue> {
        let mut out = Vec::new();

        // game_length_ticks
        out.push(MetricValue {
            game_id,
            metric_name: BuiltInMetrics::GAME_LENGTH_TICKS.into(),
            player: None,
            value: MetricValueKind::Count(i64::try_from(log.events.len()).unwrap_or(i64::MAX)),
        });

        // winner + end_reason (+ score margin) from final record
        let (winner_tag, end_reason_tag, margin) = match &log.final_result {
            Some(res) => {
                let winner = match res.winner {
                    Some(p) => format!("player_{p}"),
                    None => "draw".into(),
                };
                let reason = reason_tag(&res.reason);
                let margin = score_margin(&res.scores);
                (winner, reason, margin)
            }
            None => ("unfinished".into(), "unfinished".into(), 0),
        };
        out.push(MetricValue {
            game_id,
            metric_name: BuiltInMetrics::WINNER.into(),
            player: None,
            value: MetricValueKind::Tag(winner_tag),
        });
        out.push(MetricValue {
            game_id,
            metric_name: BuiltInMetrics::END_REASON.into(),
            player: None,
            value: MetricValueKind::Tag(end_reason_tag),
        });
        out.push(MetricValue {
            game_id,
            metric_name: BuiltInMetrics::SCORE_MARGIN.into(),
            player: None,
            value: MetricValueKind::Count(margin),
        });

        // Per-player: agent_name (from header) and final_score (from Final record)
        for (idx, name) in log.header.agents.iter().enumerate() {
            let player = player_id_from_index(idx);
            out.push(MetricValue {
                game_id,
                metric_name: BuiltInMetrics::AGENT_NAME.into(),
                player: Some(player),
                value: MetricValueKind::Tag(name.clone()),
            });
        }

        if let Some(res) = &log.final_result {
            for (idx, &score) in res.scores.iter().enumerate() {
                let player = player_id_from_index(idx);
                out.push(MetricValue {
                    game_id,
                    metric_name: BuiltInMetrics::FINAL_SCORE.into(),
                    player: Some(player),
                    value: MetricValueKind::Count(i64::from(score)),
                });
            }
        }

        out
    }
}

fn reason_tag(reason: &playtest_core::EndReason) -> String {
    match reason {
        playtest_core::EndReason::Victory => "victory".into(),
        playtest_core::EndReason::Draw => "draw".into(),
        playtest_core::EndReason::Stalemate => "stalemate".into(),
        playtest_core::EndReason::Other(s) => format!("other:{s}"),
    }
}

fn score_margin(scores: &[i32]) -> i64 {
    let Some(&first) = scores.first() else {
        return 0;
    };
    let (mut lo, mut hi) = (first, first);
    for &s in scores {
        if s < lo {
            lo = s;
        }
        if s > hi {
            hi = s;
        }
    }
    i64::from(hi.saturating_sub(lo))
}

fn player_id_from_index(idx: usize) -> PlayerId {
    PlayerId::try_from(idx).unwrap_or(PlayerId::MAX)
}
