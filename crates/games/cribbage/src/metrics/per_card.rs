//! Per-card design-insight metrics (R1.5).
//!
//! For a fixed-deck game the "per-card" stats from a CCG roadmap don't
//! map 1:1 — every card appears exactly once per deck, so a "pick rate"
//! is degenerate. The reframed questions that *do* have signal:
//!
//! - **Kept rate:** given a player was dealt rank R, how often did they
//!   keep it in their 4-card hand?
//! - **Discard rates:** as dealer (own crib) vs. non-dealer (opp crib),
//!   how often did they send rank R to the crib?
//! - **Win-rate when held:** conditional on a player holding rank R in
//!   their 4-card hand, what's their win rate?
//! - **Win-rate when in crib:** conditional on rank R landing in the
//!   crib, how often does the dealer (crib owner) win?
//!
//! These are per-game boolean/count facts; cross-game averaging happens
//! at query time in Unit 15. Emitting them as *per-rank tags* on a few
//! total-metric names keeps the SQLite schema (Unit 14) simple: one
//! `tag` column, one row per (game, metric, rank, player).

use playtest_core::PlayerId;
use playtest_metrics::{MetricDef, MetricKind, MetricScope, MetricValue, MetricValueKind};
use uuid::Uuid;

use crate::card::Rank;
use crate::metrics::accumulator::Accumulator;

/// "How many hands this game had player `p` dealt a card of rank R?" —
/// the denominator for `CARD_KEPT_RATE` aggregation in the reporter.
pub const CARD_DEALT_COUNT: &str = "card_dealt_count";
/// Numerator for kept-rate: how many hands did player `p` keep R in
/// their 4-card hand (given they were dealt it)?
pub const CARD_KEPT_COUNT: &str = "card_kept_count";
/// As dealer this hand, did player `p` send R to their own crib?
pub const CARD_DISCARDED_TO_OWN_CRIB_COUNT: &str = "card_discarded_to_own_crib_count";
/// As non-dealer this hand, did player `p` send R to the opponent's crib?
pub const CARD_DISCARDED_TO_OPP_CRIB_COUNT: &str = "card_discarded_to_opp_crib_count";
/// One per (player, rank) that the player held in their kept hand at
/// any point in the game *and* won the game. Denominator is
/// `CARD_KEPT_COUNT` across games — the reporter divides.
pub const WIN_WHEN_CARD_IN_HAND_COUNT: &str = "win_when_card_in_hand_count";
/// One per (player, rank) that the player had in their 4-card kept hand
/// at any point, winning or losing. Cross-game denominator.
pub const HAND_CONTAINED_CARD_COUNT: &str = "hand_contained_card_count";
/// One per (player, rank) where R ended up in the crib and the crib's
/// dealer was `player`. Numerator for `win_rate_when_card_in_crib`.
pub const CRIB_CONTAINED_CARD_COUNT: &str = "crib_contained_card_count";
/// Numerator for `win_rate_when_card_in_crib`: `CRIB_CONTAINED_CARD_COUNT`
/// filtered to games where `player` also won.
pub const WIN_WHEN_CARD_IN_CRIB_COUNT: &str = "win_when_card_in_crib_count";

/// Metric definitions owned by this module. Each base metric is
/// declared once; per-rank fan-out happens on the emitted
/// [`MetricValue`]s via the `tag` field (rank symbol: `"A"`, `"2"`,
/// …, `"K"`). The Unit 14 SQLite schema's `tag` column stores the
/// rank directly, so the aggregate table can be grouped by rank with
/// plain SQL.
#[must_use]
pub fn definitions() -> Vec<MetricDef> {
    vec![
        def(
            CARD_DEALT_COUNT,
            "How many hands this game had the player dealt a card of the given rank (tag = rank symbol).",
        ),
        def(
            CARD_KEPT_COUNT,
            "How many hands this game had the player keep a card of the given rank in their 4-card hand.",
        ),
        def(
            CARD_DISCARDED_TO_OWN_CRIB_COUNT,
            "How many hands as dealer the player sent a card of the given rank to their own crib.",
        ),
        def(
            CARD_DISCARDED_TO_OPP_CRIB_COUNT,
            "How many hands as non-dealer the player sent a card of the given rank to the opponent's crib.",
        ),
        def(
            HAND_CONTAINED_CARD_COUNT,
            "1 if the player ever held a card of the given rank in any 4-card hand this game, else 0.",
        ),
        def(
            WIN_WHEN_CARD_IN_HAND_COUNT,
            "1 if the player both held a card of the given rank and won this game, else 0.",
        ),
        def(
            CRIB_CONTAINED_CARD_COUNT,
            "1 if the player's crib contained a card of the given rank at any point this game, else 0.",
        ),
        def(
            WIN_WHEN_CARD_IN_CRIB_COUNT,
            "1 if the player's crib contained a card of the given rank AND the player won this game, else 0.",
        ),
    ]
}

/// Emit per-card values for both players across all 13 ranks. Each
/// metric is a `Count` scoped to `Player`; zero-valued entries are
/// still emitted so the reporter can compute rates without missing
/// denominators.
#[must_use]
pub fn extract(game_id: Uuid, acc: &Accumulator) -> Vec<MetricValue> {
    let mut out = Vec::with_capacity(13 * 8 * 2);
    for player in [0u8, 1u8] {
        extract_for_player(game_id, acc, player, &mut out);
    }
    out
}

/// Per-player accumulators over ranks. Indexed by `rank_ord() as usize`
/// (1..=13; index 0 is unused).
#[derive(Default)]
struct RankTotals {
    dealt: [u32; 14],
    kept: [u32; 14],
    own_crib: [u32; 14],
    opp_crib: [u32; 14],
    held_any: [bool; 14],
    crib_any: [bool; 14],
}

fn extract_for_player(game_id: Uuid, acc: &Accumulator, player: u8, out: &mut Vec<MetricValue>) {
    let totals = aggregate_ranks(acc, player);
    let won = acc.winner == Some(player);
    let pid: Option<PlayerId> = Some(player);
    for rank in Rank::ALL {
        let ri = usize::from(rank.rank_ord());
        let held = u32::from(totals.held_any[ri]);
        let in_crib = u32::from(totals.crib_any[ri]);
        let pairs: [(&str, u32); 8] = [
            (CARD_DEALT_COUNT, totals.dealt[ri]),
            (CARD_KEPT_COUNT, totals.kept[ri]),
            (CARD_DISCARDED_TO_OWN_CRIB_COUNT, totals.own_crib[ri]),
            (CARD_DISCARDED_TO_OPP_CRIB_COUNT, totals.opp_crib[ri]),
            (HAND_CONTAINED_CARD_COUNT, held),
            (CRIB_CONTAINED_CARD_COUNT, in_crib),
            (WIN_WHEN_CARD_IN_HAND_COUNT, if won { held } else { 0 }),
            (WIN_WHEN_CARD_IN_CRIB_COUNT, if won { in_crib } else { 0 }),
        ];
        for (base, count) in pairs {
            push_count(out, game_id, base, rank, pid, count);
        }
    }
}

fn aggregate_ranks(acc: &Accumulator, player: u8) -> RankTotals {
    let pid = usize::from(player);
    let mut t = RankTotals::default();
    for hand in &acc.hands {
        let is_dealer = hand.dealer == player;
        for rank in Rank::ALL {
            let ri = usize::from(rank.rank_ord());
            let dealt_this = hand.dealt[pid].iter().filter(|r| **r == rank).count();
            if dealt_this > 0 {
                t.dealt[ri] += u32::try_from(dealt_this).unwrap_or(u32::MAX);
            }
            let kept_this = hand.kept[pid].iter().filter(|r| **r == rank).count();
            if kept_this > 0 {
                t.kept[ri] += u32::try_from(kept_this).unwrap_or(u32::MAX);
                t.held_any[ri] = true;
            }
            let discarded_this = hand.discards[pid].iter().filter(|r| **r == rank).count();
            if discarded_this > 0 {
                let bucket = if is_dealer {
                    &mut t.own_crib[ri]
                } else {
                    &mut t.opp_crib[ri]
                };
                *bucket += u32::try_from(discarded_this).unwrap_or(u32::MAX);
            }
            // Crib belongs to this hand's dealer. Credit discards from
            // either player against the dealer's crib-contained set.
            if is_dealer {
                let from_own = hand.discards[pid].contains(&rank);
                let other = 1 - pid;
                let from_opp = hand.discards[other].contains(&rank);
                if from_own || from_opp {
                    t.crib_any[ri] = true;
                }
            }
        }
    }
    t
}

fn def(base: &str, description: &str) -> MetricDef {
    MetricDef {
        name: base.into(),
        kind: MetricKind::Count,
        scope: MetricScope::Player,
        description: description.into(),
    }
}

fn push_count(
    out: &mut Vec<MetricValue>,
    game_id: Uuid,
    base: &str,
    rank: Rank,
    player: Option<PlayerId>,
    count: u32,
) {
    out.push(MetricValue {
        game_id,
        metric_name: base.into(),
        player,
        tag: Some(rank.symbol().to_string()),
        value: MetricValueKind::Count(i64::from(count)),
    });
}
