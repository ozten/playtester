//! "What did this game look like?" metrics for ShipWreck: turn count,
//! winner's raft / invention counts, tie-breaker flag, event-card
//! plays.

use playtest_metrics::{MetricDef, MetricKind, MetricScope, MetricValue, MetricValueKind};
use uuid::Uuid;

use crate::event::TieBreakerUsed;
use crate::metrics::accumulator::Accumulator;

pub const GAME_LENGTH_TURNS: &str = "game_length_turns";
pub const WINNER_RAFT_LENGTH: &str = "winner_raft_length";
pub const WINNER_EQUIPMENT_COUNT: &str = "winner_equipment_count";
pub const TIE_BREAKER_USED: &str = "tie_breaker_used";
pub const EVENT_CARDS_PLAYED: &str = "event_cards_played";

/// Metric definitions owned by this module.
#[must_use]
pub fn definitions() -> Vec<MetricDef> {
    vec![
        MetricDef {
            name: GAME_LENGTH_TURNS.into(),
            kind: MetricKind::Count,
            scope: MetricScope::Game,
            description:
                "Total number of EndTurn events in the game log — how long the game ran in turns."
                    .into(),
        },
        MetricDef {
            name: WINNER_RAFT_LENGTH.into(),
            kind: MetricKind::Count,
            scope: MetricScope::Game,
            description:
                "Length of the winning player's raft (base + extensions) at game end. Absent on a tie."
                    .into(),
        },
        MetricDef {
            name: WINNER_EQUIPMENT_COUNT.into(),
            kind: MetricKind::Count,
            scope: MetricScope::Game,
            description:
                "Number of equipment upgrades installed on the winning player's raft at game end. Absent on a tie."
                    .into(),
        },
        MetricDef {
            name: TIE_BREAKER_USED.into(),
            kind: MetricKind::Tag,
            scope: MetricScope::Game,
            description:
                "Which tie-breaker decided the game: \"none\" (rescue points alone), \"raft_length\", \"invention_count\", or \"tie\"."
                    .into(),
        },
        MetricDef {
            name: EVENT_CARDS_PLAYED.into(),
            kind: MetricKind::Count,
            scope: MetricScope::Game,
            description: "Count of EventCardPlayed records — how many event cards fired this game."
                .into(),
        },
    ]
}

/// Emit all game-shape metric values for one accumulated game.
#[must_use]
pub fn extract(game_id: Uuid, acc: &Accumulator) -> Vec<MetricValue> {
    let mut out = Vec::new();

    out.push(MetricValue {
        game_id,
        metric_name: GAME_LENGTH_TURNS.into(),
        player: None,
        tag: None,
        value: MetricValueKind::Count(i64::try_from(acc.end_turn_count).unwrap_or(i64::MAX)),
    });

    out.push(MetricValue {
        game_id,
        metric_name: EVENT_CARDS_PLAYED.into(),
        player: None,
        tag: None,
        value: MetricValueKind::Count(i64::try_from(acc.event_cards_played).unwrap_or(i64::MAX)),
    });

    // Winner-dependent metrics are only emitted when a winner exists
    // and we have their score row — keeps the reporter from computing
    // averages over sentinel zeros.
    if let Some(w) = acc.winner
        && let Some(row) = acc.final_scores.iter().find(|s| s.player == w)
    {
        out.push(MetricValue {
            game_id,
            metric_name: WINNER_RAFT_LENGTH.into(),
            player: None,
            tag: None,
            value: MetricValueKind::Count(i64::from(row.raft_length)),
        });
        out.push(MetricValue {
            game_id,
            metric_name: WINNER_EQUIPMENT_COUNT.into(),
            player: None,
            tag: None,
            value: MetricValueKind::Count(i64::from(row.invention_count)),
        });
    }

    // Tie-breaker is always emitted when we have an EndGame — the
    // metric is a single tag, easy to aggregate in SQL.
    if let Some(tb) = acc.tie_breaker {
        out.push(MetricValue {
            game_id,
            metric_name: TIE_BREAKER_USED.into(),
            player: None,
            tag: None,
            value: MetricValueKind::Tag(tie_breaker_tag(tb).into()),
        });
    }

    out
}

/// Canonical string labels for [`TieBreakerUsed`]. Kept here so the
/// reporter, tests, and any dashboards all agree on the exact value.
#[must_use]
pub const fn tie_breaker_tag(tb: TieBreakerUsed) -> &'static str {
    match tb {
        TieBreakerUsed::None => "none",
        TieBreakerUsed::RaftLength => "raft_length",
        TieBreakerUsed::InventionCount => "invention_count",
        TieBreakerUsed::Tie => "tie",
    }
}
