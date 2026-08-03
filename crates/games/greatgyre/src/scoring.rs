//! Final scoring and the end-game winner determination.
//!
//! `score = Σ hope icons on face-up raft cards (survivors + modifications;
//! raft base cards and extensions carry no hope) + 15 if Land Sighting is
//! in hand at game end − 5 × (each other player's Influencer on their
//! raft)`, per `docs/greatgyre.md`'s End of Game section. Hungry (sideways)
//! survivors still count their hope, per the spec's `[A]` ruling.
//!
//! Ties: **all tied players win** (per the plan's design decision).
//! `GameResult::winner` can only hold a single `PlayerId`, so it's set
//! to the lowest-seat tied winner — the authoritative multi-winner
//! list lives on `Event::EndGame::winners`.

use playtest_core::{EndReason, GameResult, PlayerId};

use crate::card::{CardKind, EventKind, SurvivorId};
use crate::event::ScoreRow;
use crate::state::GameState;

/// `player`'s full score: hope sum of their placed survivors/
/// modifications, `+15` if they hold Land Sighting, `-5` per *other*
/// player's placed Influencer.
#[must_use]
pub fn score_player(state: &GameState, player: PlayerId) -> i32 {
    let idx = player as usize;
    let hope_sum: u32 = state.players[idx].placed.iter().map(|p| p.card.kind.hope()).sum();
    let mut score = i32::try_from(hope_sum).unwrap_or(i32::MAX);

    let holds_land_sighting = state.players[idx]
        .hand
        .iter()
        .any(|c| matches!(c.kind, CardKind::Event(EventKind::LandSighting)));
    if holds_land_sighting {
        score += 15;
    }

    let influencer_count = state
        .players
        .iter()
        .enumerate()
        .filter(|&(i, _)| i != idx)
        .filter(|(_, p)| has_influencer(p))
        .count();
    let influencer_count = i32::try_from(influencer_count).unwrap_or(i32::MAX);
    score -= 5 * influencer_count;

    score
}

fn has_influencer(player: &crate::state::PlayerState) -> bool {
    player
        .placed
        .iter()
        .any(|pc| matches!(pc.card.kind, CardKind::Survivor(SurvivorId::Influencer)))
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

/// Every seat tied for the highest score. Ties: all tied players win.
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
    let score_vec: Vec<i32> = scores.iter().map(|s| s.hope).collect();
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
    use crate::card::{Card, CardInstanceId, ModificationKind};
    use crate::config::GreatGyreConfig;
    use crate::state::PlacedCard;

    fn row(player: PlayerId, hope: i32) -> ScoreRow {
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

    fn state_for_scoring(n: u8) -> GameState {
        let cfg = GreatGyreConfig::new(n).unwrap();
        crate::setup::initial_state(1, &cfg)
    }

    #[test]
    fn land_sighting_in_hand_adds_fifteen() {
        let mut state = state_for_scoring(2);
        state.players[0].hand.push(Card::new(
            CardInstanceId(9001),
            CardKind::Event(EventKind::LandSighting),
        ));
        assert_eq!(score_player(&state, 0), 15);
        assert_eq!(score_player(&state, 1), 0);
    }

    #[test]
    fn opponent_influencer_subtracts_five_per_player() {
        let mut state = state_for_scoring(3);
        state.players[1].placed.push(PlacedCard {
            card: Card::new(CardInstanceId(9002), CardKind::Survivor(SurvivorId::Influencer)),
            hungry: false,
        });
        // Seat 0 sees one opponent Influencer: -5. Seat 1 (the
        // Influencer's own owner) is unaffected by their own card.
        // Seat 2 also sees the same opponent Influencer: -5.
        assert_eq!(score_player(&state, 0), -5);
        assert_eq!(score_player(&state, 1), 0);
        assert_eq!(score_player(&state, 2), -5);
    }

    #[test]
    fn own_hope_and_penalties_combine() {
        let mut state = state_for_scoring(2);
        state.players[0].placed.push(PlacedCard {
            card: Card::new(CardInstanceId(9003), CardKind::Modification(ModificationKind::Sail)),
            hungry: false,
        }); // +10 hope
        state.players[1].placed.push(PlacedCard {
            card: Card::new(CardInstanceId(9004), CardKind::Survivor(SurvivorId::Influencer)),
            hungry: false,
        });
        assert_eq!(score_player(&state, 0), 10 - 5);
    }
}
