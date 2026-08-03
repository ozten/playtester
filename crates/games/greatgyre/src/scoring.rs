//! Hope scoring and the end-game winner determination.
//!
//! **Unit 2 scope**: per the implementation task, scoring is "hope sum
//! of face-up raft cards (survivors + hope-bearing mods + raft cards
//! if any hope — raft base cards have no hope)". The spec's full
//! formula also adds `+15` if a player holds Land Sighting at game end
//! and subtracts `5` per opponent's Influencer — both are *survivor/
//! event* special-cases layered on top of the base hope sum, and both
//! are explicitly out of scope until events (Unit 4) and abilities
//! (Unit 3) land. Only the base hope sum is implemented here; Unit 4
//! extends [`score_player`] with the Land Sighting/Influencer terms.
//!
//! Ties: **all tied players win** (per the plan's design decision).
//! `GameResult::winner` can only hold a single `PlayerId`, so it's set
//! to the lowest-seat tied winner — the authoritative multi-winner
//! list lives on `Event::EndGame::winners`.

use playtest_core::{EndReason, GameResult, PlayerId};

use crate::event::ScoreRow;
use crate::state::GameState;

/// Sum of hope icons on every face-up card on `player`'s raft:
/// placed survivors + placed modifications. Raft Left/Right and built
/// extensions contribute 0 (no hope printed on those cards).
#[must_use]
pub fn score_player(state: &GameState, player: PlayerId) -> u32 {
    state.players[player as usize]
        .placed
        .iter()
        .map(|p| p.card.kind.hope())
        .sum()
}

/// Score every seat, in seat order.
#[must_use]
pub fn score_all(state: &GameState) -> Vec<ScoreRow> {
    (0..state.players.len())
        .map(|i| {
            let player = u8::try_from(i).expect("seat count fits in u8");
            ScoreRow {
                player,
                hope: score_player(state, player),
            }
        })
        .collect()
}

/// Every seat tied for the highest hope total. Ties: all tied players win.
#[must_use]
pub fn winners(scores: &[ScoreRow]) -> Vec<PlayerId> {
    let Some(max) = scores.iter().map(|s| s.hope).max() else {
        return Vec::new();
    };
    scores
        .iter()
        .filter(|s| s.hope == max)
        .map(|s| s.player)
        .collect()
}

/// Build the `(GameResult, winners, scores)` triple used by both
/// `game_over` and the `Event::EndGame` builder.
#[must_use]
pub fn build_game_result(state: &GameState) -> (GameResult, Vec<PlayerId>, Vec<ScoreRow>) {
    let scores = score_all(state);
    let win = winners(&scores);
    let reason = if win.len() <= 1 {
        EndReason::Victory
    } else {
        EndReason::Other("tie_all_win".into())
    };
    let score_vec: Vec<i32> = scores
        .iter()
        .map(|s| i32::try_from(s.hope).unwrap_or(i32::MAX))
        .collect();
    let result = GameResult {
        winner: win.iter().min().copied(),
        reason,
        scores: score_vec,
    };
    (result, win, scores)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(player: PlayerId, hope: u32) -> ScoreRow {
        ScoreRow { player, hope }
    }

    #[test]
    fn single_max_wins_alone() {
        let scores = vec![row(0, 20), row(1, 10)];
        assert_eq!(winners(&scores), vec![0]);
    }

    #[test]
    fn tied_max_all_win() {
        let scores = vec![row(0, 20), row(1, 20), row(2, 5)];
        assert_eq!(winners(&scores), vec![0, 1]);
    }

    #[test]
    fn empty_scores_no_winners() {
        assert!(winners(&[]).is_empty());
    }
}
