//! Evaluation function for ShipWreck one-ply agents.
//!
//! Signature (per `playtest_agents::eval::EvalFn`):
//! `fn(view: &ShipWreckPublicView, player: PlayerId) -> f64`.
//!
//! Higher = better for `player`. Weights tuned to clear R2.2
//! ("heuristic beats random > 90% over 10K games").
//!
//! Inputs we consider:
//! - rescue points held on own raft — the primary scoring axis
//! - raft length (first tie-breaker in the spec)
//! - invention count (second tie-breaker)
//! - resource-inventory *diversity* — future-value for building
//!   equipment (which needs varied resources)
//! - opponent rescue-point pressure (small negative — don't enable
//!   easy mirror-plays)

use playtest_core::PlayerId;

use crate::public_view::ShipWreckPublicView;

// The heuristic's score differences between good and bad actions
// drive how well softmax sampling survives. Weights are scaled so the
// per-move delta between a "place a player card" (big rescue points)
// and an "end turn" (no progress) is on the order of 10+ at T=0.5 —
// keeping noise under 1% of the time. Cribbage's eval made the same
// tradeoff at a different scale.
const W_OWN_RESCUE: f64 = 10.0;
const W_OWN_RAFT_LEN: f64 = 2.0;
const W_OWN_INV_COUNT: f64 = 3.0;
const W_OWN_DIVERSITY: f64 = 1.0;
const W_OPP_MAX_RESCUE: f64 = -5.0;
const W_OWN_FOOD: f64 = 0.5;
const W_OWN_RESOURCE_SUM: f64 = 0.3;
/// Bonus per card still in hand — strongly reward "have more
/// options". Zeroed-out hands tend to mean the player stalled out.
const W_OWN_HAND_SIZE: f64 = 0.5;
/// Bonus for each placed player card (beyond rescue points) — placing
/// a card is a direct scoring commitment; penalize being slow to do
/// it via this lightweight counter.
const W_OWN_PLACED_COUNT: f64 = 2.0;

/// Score the ShipWreck public view from `player`'s perspective.
#[must_use]
pub fn shipwreck_eval(view: &ShipWreckPublicView, _player: PlayerId) -> f64 {
    // Own-seat signals.
    let own_rescue: u32 = view
        .own
        .played_players
        .iter()
        .map(|pp| u32::from(pp.card.rescue_points))
        .sum();
    let own_raft_len = view.own.raft.length();
    let own_inv_count = view.own.raft.invention_count();
    let own_diversity = inventory_diversity(view.own.inventory);
    let own_food = f64::from(view.own.food_counter.max(0));
    let own_resource_sum: u32 = view.own.inventory.iter().map(|x| u32::from(*x)).sum();

    // Opponent signals — only consider the *strongest* opponent.
    let mut max_opp_rescue: u32 = 0;
    for opp_slot in &view.opponents {
        let Some(opp) = opp_slot else { continue };
        let rp: u32 = opp
            .played_players
            .iter()
            .map(|pp| u32::from(pp.card.rescue_points))
            .sum();
        if rp > max_opp_rescue {
            max_opp_rescue = rp;
        }
    }

    let own_hand_size =
        f64::from(u32::try_from(view.own.hand.len()).unwrap_or(u32::MAX));
    let own_placed_count =
        f64::from(u32::try_from(view.own.played_players.len()).unwrap_or(u32::MAX));

    W_OWN_RESCUE * f64::from(own_rescue)
        + W_OWN_RAFT_LEN
            * f64::from(u32::try_from(own_raft_len).unwrap_or(u32::MAX))
        + W_OWN_INV_COUNT
            * f64::from(u32::try_from(own_inv_count).unwrap_or(u32::MAX))
        + W_OWN_DIVERSITY * own_diversity
        + W_OPP_MAX_RESCUE * f64::from(max_opp_rescue)
        + W_OWN_FOOD * own_food
        + W_OWN_RESOURCE_SUM * f64::from(own_resource_sum)
        + W_OWN_HAND_SIZE * own_hand_size
        + W_OWN_PLACED_COUNT * own_placed_count
}

/// Count of distinct resources we hold (0..=5). The plan calls out:
/// "prefer more different resources than high count of one". Multiply
/// by 2 to scale into [0, 10] and cap there.
fn inventory_diversity(inventory: [u8; 5]) -> f64 {
    let distinct = inventory.iter().filter(|&&c| c > 0).count();
    f64::from(u32::try_from(distinct).unwrap_or(0)) * 2.0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::card::{PlayerCard, PlayerCardId};
    use crate::phase::Phase;
    use crate::public_view::{OpponentView, OwnView};
    use crate::raft::{Raft, SlotId};
    use crate::state::PlacedPlayerCard;
    use crate::{BaseRaftCard, card::BaseRaftSide};

    fn new_raft() -> Raft {
        Raft::new(
            BaseRaftCard::new(BaseRaftSide::Left),
            BaseRaftCard::new(BaseRaftSide::Right),
        )
    }

    fn empty_own(player: PlayerId) -> OwnView {
        OwnView {
            player,
            hand: Vec::new(),
            raft: new_raft(),
            played_players: Vec::new(),
            food_counter: 0,
            inventory: [0; 5],
            face_up_pool: Vec::new(),
        }
    }

    fn view_with(own: OwnView) -> ShipWreckPublicView {
        ShipWreckPublicView {
            observer: own.player,
            own,
            opponents: vec![None, None],
            current_player: 0,
            phase: Phase::Play,
            current_equipment: None,
            equipment_deck_remaining: 0,
            event_resolution_stack: Vec::new(),
            wreckage_deck_size: 0,
            discarded_event_cards: Vec::new(),
        }
    }

    #[test]
    fn more_rescue_points_scores_higher() {
        let mut own = empty_own(0);
        let pc = PlayerCard::new(PlayerCardId::MovieStar, 5, 1, None);
        own.played_players
            .push(PlacedPlayerCard { card: pc, slot: SlotId::BaseLeft });
        let v_with = view_with(own);
        let v_without = view_with(empty_own(0));
        assert!(shipwreck_eval(&v_with, 0) > shipwreck_eval(&v_without, 0));
    }

    #[test]
    fn diverse_inventory_beats_concentrated_inventory() {
        let mut diverse_own = empty_own(0);
        diverse_own.inventory = [1, 1, 1, 1, 1];
        let mut concentrated_own = empty_own(0);
        concentrated_own.inventory = [5, 0, 0, 0, 0];
        let v_div = view_with(diverse_own);
        let v_conc = view_with(concentrated_own);
        // Diversity weighting should dominate at these modest scales.
        assert!(shipwreck_eval(&v_div, 0) > shipwreck_eval(&v_conc, 0));
    }

    #[test]
    fn opponent_with_rescue_points_lowers_my_score() {
        let my_view = view_with(empty_own(0));
        let mut op_view = my_view.clone();
        // Install an opponent with placed-player rescue points.
        let opp_card = PlayerCard::new(PlayerCardId::MovieStar, 5, 1, None);
        op_view.opponents[1] = Some(OpponentView {
            player: 1,
            hand_size: 0,
            raft: new_raft(),
            played_players: vec![PlacedPlayerCard {
                card: opp_card,
                slot: SlotId::BaseLeft,
            }],
            food_counter: 0,
            inventory: [0; 5],
            face_up_pool: Vec::new(),
        });
        assert!(shipwreck_eval(&my_view, 0) > shipwreck_eval(&op_view, 0));
    }
}
