//! Rescue-point scoring and tie-breaker chain for ShipWreck.
//!
//! Per `docs/shipwreck.md`:
//! > "Once all of the wreckage cards are gone, the game ends. Rescue
//! > points are totaled. The most rescue points determines the winner.
//! > Tie breaker is settled first by length of raft and finally by
//! > total number of inventions. Lastly a tie is a tie."
//!
//! Rescue points for a seat are the sum of `rescue_points` over every
//! *placed* player card (card sitting on a raft slot via `PlacePlayerCard`).
//! Player cards still in hand score nothing — placing is the
//! scoring-eligibility action. Dropped (starved) cards are likewise
//! ignored — `apply_event` removes them from `played_players` before
//! scoring runs.
//!
//! The tie-breaker chain is encoded as [`TieBreakerUsed`] so metrics
//! can report *which* step decided the game (R9.7 — "winner decided
//! on rescue points vs. via a tie-breaker").

use playtest_core::{EndReason, GameResult, PlayerId};

use crate::event::{PlayerScore, TieBreakerUsed};
use crate::state::{GameState, PlayerState};

/// Sum of `rescue_points` across every placed player card for one seat.
///
/// Player cards still in `hand` score nothing; that matches the spec's
/// "place a player card in hand onto a raft slot" phrasing.
#[must_use]
pub fn score_player(p: &PlayerState) -> u16 {
    let total: u32 = p
        .played_players
        .iter()
        .map(|pp| u32::from(pp.card.rescue_points))
        .sum();
    u16::try_from(total).unwrap_or(u16::MAX)
}

/// Build a [`PlayerScore`] row for one seat. Captures the primary
/// score and both tie-breakers so the caller doesn't have to re-walk
/// the player's state.
fn score_row(player: PlayerId, p: &PlayerState) -> PlayerScore {
    let rescue_points = score_player(p);
    let raft_length = u16::try_from(p.raft.length()).unwrap_or(u16::MAX);
    let invention_count = u16::try_from(p.raft.invention_count()).unwrap_or(u16::MAX);
    PlayerScore {
        player,
        rescue_points,
        raft_length,
        invention_count,
    }
}

/// Score every seat — one row per player, in seat order.
#[must_use]
pub fn score_all(state: &GameState) -> Vec<PlayerScore> {
    state
        .players
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let seat = u8::try_from(i).expect("seat fits in u8");
            score_row(seat, p)
        })
        .collect()
}

/// Decide the winner from a slice of per-seat [`PlayerScore`] rows.
///
/// Returns `(winner, tie_breaker, scores_in_input_order)`:
///
/// - `winner` is `Some(seat)` when the tie-breaker chain produced a
///   unique leader, `None` on a true tie.
/// - `tie_breaker` is the step that decided it — `None` when rescue
///   points alone were enough, `RaftLength` / `InventionCount` when a
///   fallback broke the tie, `Tie` when every step came up equal.
/// - `scores_in_input_order` echoes `scores` so callers that passed a
///   reference (e.g. the end-of-game event builder) don't have to
///   clone separately.
#[must_use]
pub fn determine_winner(
    scores: &[PlayerScore],
) -> (Option<PlayerId>, TieBreakerUsed, Vec<PlayerScore>) {
    if scores.is_empty() {
        return (None, TieBreakerUsed::Tie, Vec::new());
    }

    // Sort a *copy* of the rows by the tie-breaker chain. Keep the
    // original order untouched so the returned score vector still lines
    // up with seat indices — the engine wires it into `GameResult` that
    // way.
    let mut ranked: Vec<&PlayerScore> = scores.iter().collect();
    ranked.sort_by(|a, b| {
        b.rescue_points
            .cmp(&a.rescue_points)
            .then(b.raft_length.cmp(&a.raft_length))
            .then(b.invention_count.cmp(&a.invention_count))
    });

    let top = ranked[0];
    // When there is only one player the "top" is trivially the winner
    // without any tie-breaker — that case gets `None`.
    let tie_breaker = if ranked.len() < 2 {
        TieBreakerUsed::None
    } else {
        let second = ranked[1];
        if top.rescue_points > second.rescue_points {
            TieBreakerUsed::None
        } else if top.raft_length > second.raft_length {
            TieBreakerUsed::RaftLength
        } else if top.invention_count > second.invention_count {
            TieBreakerUsed::InventionCount
        } else {
            TieBreakerUsed::Tie
        }
    };

    let winner = if matches!(tie_breaker, TieBreakerUsed::Tie) {
        None
    } else {
        Some(top.player)
    };

    (winner, tie_breaker, scores.to_vec())
}

/// Build the full [`GameResult`] from a `GameState`. Used by the engine's
/// `game_over` impl and the `EndGame` event builder.
#[must_use]
pub fn build_game_result(state: &GameState) -> (GameResult, TieBreakerUsed, Vec<PlayerScore>) {
    let scores = score_all(state);
    let (winner, tie_breaker, _) = determine_winner(&scores);
    let reason = match winner {
        Some(_) => EndReason::Other("deck_exhausted".into()),
        None => EndReason::Draw,
    };
    let score_vec: Vec<i32> = scores
        .iter()
        .map(|s| i32::from(s.rescue_points))
        .collect();
    let result = GameResult {
        winner,
        reason,
        scores: score_vec,
    };
    (result, tie_breaker, scores)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn score(player: PlayerId, rp: u16, raft: u16, inv: u16) -> PlayerScore {
        PlayerScore {
            player,
            rescue_points: rp,
            raft_length: raft,
            invention_count: inv,
        }
    }

    #[test]
    fn rescue_points_alone_picks_winner() {
        let scores = vec![score(0, 10, 2, 0), score(1, 5, 3, 1)];
        let (w, tb, _) = determine_winner(&scores);
        assert_eq!(w, Some(0));
        assert_eq!(tb, TieBreakerUsed::None);
    }

    #[test]
    fn raft_length_breaks_rescue_tie() {
        let scores = vec![score(0, 7, 3, 0), score(1, 7, 2, 0)];
        let (w, tb, _) = determine_winner(&scores);
        assert_eq!(w, Some(0));
        assert_eq!(tb, TieBreakerUsed::RaftLength);
    }

    #[test]
    fn invention_count_breaks_rescue_plus_raft_tie() {
        let scores = vec![score(0, 7, 3, 1), score(1, 7, 3, 2)];
        let (w, tb, _) = determine_winner(&scores);
        assert_eq!(w, Some(1));
        assert_eq!(tb, TieBreakerUsed::InventionCount);
    }

    #[test]
    fn fully_tied_returns_none_and_tie_marker() {
        let scores = vec![score(0, 7, 3, 1), score(1, 7, 3, 1)];
        let (w, tb, _) = determine_winner(&scores);
        assert!(w.is_none());
        assert_eq!(tb, TieBreakerUsed::Tie);
    }

    #[test]
    fn four_way_tie_is_a_tie() {
        let scores = vec![
            score(0, 5, 2, 0),
            score(1, 5, 2, 0),
            score(2, 5, 2, 0),
            score(3, 5, 2, 0),
        ];
        let (w, tb, _) = determine_winner(&scores);
        assert!(w.is_none());
        assert_eq!(tb, TieBreakerUsed::Tie);
    }

    #[test]
    fn empty_input_is_a_tie() {
        let (w, tb, out) = determine_winner(&[]);
        assert!(w.is_none());
        assert_eq!(tb, TieBreakerUsed::Tie);
        assert!(out.is_empty());
    }
}
