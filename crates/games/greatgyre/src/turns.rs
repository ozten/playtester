//! Stat aggregation and small helpers shared by `rules.rs`.
//!
//! Unit 2 landed the *printed* per-card numbers (hope, food, hand-tab,
//! space rules, resource cost). Unit 3 (this unit) adds the *active*
//! per-turn budget bonuses — `add_bonus` / `draw_bonus` / `action_bonus`
//! — and the special Phase-2 draw sources' eligibility checks
//! (`has_survivor`, `adjacent_players`).

use playtest_core::PlayerId;

use crate::card::{Card, CardInstanceId, CardKind, ModificationKind, SurvivorId};
use crate::resource::{Resource, ResourceCost};
use crate::state::PlayerState;

/// Base max hand size before any survivor/modification adjustment.
pub const BASE_HAND_SIZE: i32 = 3;

/// Cost to build a raft extension: 1 Wood + 1 Rope + 1 Plastic.
#[must_use]
pub const fn extension_cost() -> ResourceCost {
    ResourceCost::new(1, 1, 1)
}

/// `3 + Σ survivor hand tabs − (Fishing Rod built ? 1 : 0) − (Telescope
/// built ? 1 : 0)`, per `docs/greatgyre.md` Phase 3. Clamped at 0 (a
/// negative hand size is meaningless).
#[must_use]
pub fn compute_hand_limit(player: &PlayerState) -> u32 {
    let mut limit = BASE_HAND_SIZE;
    for p in &player.placed {
        limit += match p.card.kind {
            CardKind::Survivor(s) => i32::from(s.hand_tab()),
            CardKind::Modification(m) => i32::from(m.hand_tab()),
            _ => 0,
        };
    }
    limit.max(0).unsigned_abs()
}

/// `Σ` food icons on face-up raft cards: Raft Right baseline `+1`,
/// every placed survivor/modification's printed food value (Hungry
/// survivors still count, per the spec's `[A]` ruling).
#[must_use]
pub fn compute_food(player: &PlayerState) -> i32 {
    let mut food = 1; // Raft Right.
    for p in &player.placed {
        food += match p.card.kind {
            CardKind::Survivor(s) => i32::from(s.food()),
            CardKind::Modification(m) => i32::from(m.food()),
            _ => 0,
        };
    }
    food
}

/// `(spaces used, total capacity)`. Capacity = 2 (base raft) + 2 per
/// built extension + 2 per built Quarterdeck (which also occupies 1 of
/// its own, netting +1). Usage counts every placed survivor except
/// Stowaway, every placed space-occupying modification, and every
/// Walrus currently blocking a space on this raft (each Walrus blocks
/// exactly 1 space — the plan's exception to the fungible-count
/// model).
#[must_use]
pub fn raft_capacity(player: &PlayerState) -> (u32, u32) {
    let mut used = u32::try_from(player.blocked_by_walrus.len()).unwrap_or(u32::MAX);
    let mut total = 2 + 2 * u32::try_from(player.built_extensions.len()).unwrap_or(u32::MAX);
    for p in &player.placed {
        let occupies = match p.card.kind {
            CardKind::Survivor(s) => s.occupies_space(),
            CardKind::Modification(m) => m.occupies_space(),
            _ => false,
        };
        if occupies {
            used += 1;
        }
        if let CardKind::Modification(ModificationKind::Quarterdeck) = p.card.kind {
            total += u32::from(ModificationKind::Quarterdeck.provides_capacity());
        }
    }
    (used, total)
}

/// Free raft spaces remaining.
#[must_use]
pub fn free_spaces(player: &PlayerState) -> u32 {
    let (used, total) = raft_capacity(player);
    total.saturating_sub(used)
}

/// True if `hand` holds enough of each resource to cover `cost`.
#[must_use]
pub fn can_afford(hand: &[Card], cost: ResourceCost) -> bool {
    for r in Resource::ALL {
        let have = u32::try_from(count_resource(hand, r)).unwrap_or(u32::MAX);
        if have < u32::from(cost.amount_of(r)) {
            return false;
        }
    }
    true
}

fn count_resource(hand: &[Card], resource: Resource) -> usize {
    hand.iter()
        .filter(|c| matches!(c.kind, CardKind::Resource(r) if r == resource))
        .count()
}

/// Pick which literal resource cards from `hand` pay `cost`. Resources
/// are fungible within a kind, so selection is deterministic
/// (hand-order) rather than agent-chosen. Panics if `hand` cannot
/// cover `cost` — callers must check [`can_afford`] first.
#[must_use]
pub fn select_payment(hand: &[Card], cost: ResourceCost) -> Vec<Card> {
    let mut out = Vec::with_capacity(usize::from(cost.total()));
    for r in Resource::ALL {
        let mut needed = cost.amount_of(r);
        if needed == 0 {
            continue;
        }
        for c in hand {
            if needed == 0 {
                break;
            }
            if matches!(c.kind, CardKind::Resource(res) if res == r) {
                out.push(*c);
                needed -= 1;
            }
        }
        assert_eq!(needed, 0, "select_payment: hand cannot cover {r:?} cost");
    }
    out
}

/// Find a card by instance id in `hand`.
#[must_use]
pub fn find_in_hand(hand: &[Card], id: CardInstanceId) -> Option<Card> {
    hand.iter().copied().find(|c| c.id == id)
}

/// True if `player` has `survivor` placed on their raft (hungry or
/// standing — per the spec's `[A]` ruling that Hungry survivors'
/// stats still count, extended here to abilities as well: nothing in
/// the spec says going Hungry switches an ability off, and the
/// alternative — silently losing your draw source mid-game — would be
/// a harsher, unstated penalty).
#[must_use]
pub fn has_survivor(player: &PlayerState, survivor: SurvivorId) -> bool {
    player
        .placed
        .iter()
        .any(|pc| matches!(pc.card.kind, CardKind::Survivor(s) if s == survivor))
}

fn count_modification(player: &PlayerState, kind: ModificationKind) -> u8 {
    let n = player
        .placed
        .iter()
        .filter(|pc| matches!(pc.card.kind, CardKind::Modification(k) if k == kind))
        .count();
    u8::try_from(n).unwrap_or(u8::MAX)
}

/// `add_bonus`: Sail +1 each (built), Millionaire +1 (placed). Added
/// to the base `1` for Phase 1's card-add count.
#[must_use]
pub fn add_bonus(player: &PlayerState) -> u8 {
    count_modification(player, ModificationKind::Sail)
        .saturating_add(u8::from(has_survivor(player, SurvivorId::Millionaire)))
}

/// `draw_bonus`: Net +1 each (built), Athlete +1 (placed). Added to
/// the base `1` for Phase 2's draw budget.
#[must_use]
pub fn draw_bonus(player: &PlayerState) -> u8 {
    count_modification(player, ModificationKind::Net)
        .saturating_add(u8::from(has_survivor(player, SurvivorId::Athlete)))
}

/// `action_bonus`: Toolkit +1 (built, single copy in the game), First
/// Mate +1 (placed). Added to the base `1` for Phase 3's action
/// budget. Work Day's "unlimited actions" (Unit 4) layers on top of
/// this rather than folding into it.
#[must_use]
pub fn action_bonus(player: &PlayerState) -> u8 {
    count_modification(player, ModificationKind::Toolkit)
        .saturating_add(u8::from(has_survivor(player, SurvivorId::FirstMate)))
}

/// The (deduplicated) seat(s) adjacent to `player` in a `num_players`-
/// seat game: left and right neighbors. For `num_players == 2` these
/// coincide (there's only one opponent), so the result has length 1;
/// otherwise length 2.
#[must_use]
pub fn adjacent_players(num_players: usize, player: PlayerId) -> Vec<PlayerId> {
    let n = num_players;
    let me = usize::from(player);
    let left = u8::try_from((me + n - 1) % n).expect("seat fits in u8");
    let right = u8::try_from((me + 1) % n).expect("seat fits in u8");
    if left == right {
        vec![left]
    } else {
        vec![left, right]
    }
}

// ---------- Unit 4: events & reactions -------------------------------------

/// True if `hand` holds at least one Dead Fish.
#[must_use]
pub fn has_dead_fish(hand: &[Card]) -> bool {
    hand.iter().any(|c| matches!(c.kind, CardKind::DeadFish))
}

/// True if `player` has a Fisher placed on their raft (hungry or
/// standing — see `has_survivor`'s doc comment).
#[must_use]
pub fn has_fisher(player: &PlayerState) -> bool {
    has_survivor(player, SurvivorId::Fisher)
}

/// Hand cards that are *not* event cards — the only cards Storm's
/// "discard 2 hand cards" route may pick from (event cards are exempt
/// from Max Hand Size and, per the spec's phrasing, from this discard
/// too).
#[must_use]
pub fn non_event_hand_count(hand: &[Card]) -> u8 {
    let n = hand
        .iter()
        .filter(|c| !matches!(c.kind, CardKind::Event(_)))
        .count();
    u8::try_from(n).unwrap_or(u8::MAX)
}

/// Number of survivors (hungry or standing) placed on `player`'s raft
/// — the eligibility count for Shark Attack / Love Boat targeting.
#[must_use]
pub fn survivor_count(player: &PlayerState) -> usize {
    player
        .placed
        .iter()
        .filter(|pc| matches!(pc.card.kind, CardKind::Survivor(_)))
        .count()
}

/// Storm's resolution order: starting with `caster`'s down-current
/// neighbor and proceeding around, excluding `caster`. "Down-current"
/// is ruled `[A]` to be the same "left" direction Phase 5 already
/// passes Currents in (`(seat + 1) % n`) — the spec fixes a single
/// physical direction and Phase 5's is the only one already pinned
/// down, so reusing it is the most consistent reading.
#[must_use]
pub fn storm_order(num_players: usize, caster: PlayerId) -> Vec<PlayerId> {
    let n = num_players;
    let start = usize::from(caster);
    (1..n)
        .map(|k| u8::try_from((start + k) % n).expect("seat fits in u8"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::card::{Card, CardInstanceId};
    use crate::state::PlacedCard;

    fn placed(id: u32, kind: CardKind) -> PlacedCard {
        PlacedCard {
            card: Card::new(CardInstanceId(id), kind),
            hungry: false,
        }
    }

    fn fresh_player() -> PlayerState {
        let raft = Card::new(CardInstanceId(9000), CardKind::RaftLeft);
        PlayerState::fresh(raft, raft)
    }

    #[test]
    fn add_bonus_counts_sails_and_millionaire() {
        let mut p = fresh_player();
        assert_eq!(add_bonus(&p), 0);
        p.placed.push(placed(1, CardKind::Modification(ModificationKind::Sail)));
        assert_eq!(add_bonus(&p), 1);
        p.placed.push(placed(2, CardKind::Modification(ModificationKind::Sail)));
        assert_eq!(add_bonus(&p), 2);
        p.placed.push(placed(3, CardKind::Survivor(SurvivorId::Millionaire)));
        assert_eq!(add_bonus(&p), 3);
    }

    #[test]
    fn draw_bonus_counts_nets_and_athlete() {
        let mut p = fresh_player();
        p.placed.push(placed(1, CardKind::Modification(ModificationKind::Net)));
        p.placed.push(placed(2, CardKind::Modification(ModificationKind::Net)));
        p.placed.push(placed(3, CardKind::Survivor(SurvivorId::Athlete)));
        assert_eq!(draw_bonus(&p), 3);
    }

    #[test]
    fn action_bonus_counts_toolkit_and_first_mate() {
        let mut p = fresh_player();
        p.placed.push(placed(1, CardKind::Modification(ModificationKind::Toolkit)));
        p.placed.push(placed(2, CardKind::Survivor(SurvivorId::FirstMate)));
        assert_eq!(action_bonus(&p), 2);
    }

    #[test]
    fn hungry_survivors_still_grant_abilities() {
        let mut p = fresh_player();
        p.placed.push(PlacedCard {
            card: Card::new(CardInstanceId(1), CardKind::Survivor(SurvivorId::Athlete)),
            hungry: true,
        });
        assert!(has_survivor(&p, SurvivorId::Athlete));
        assert_eq!(draw_bonus(&p), 1);
    }

    #[test]
    fn adjacent_players_dedupes_for_two() {
        assert_eq!(adjacent_players(2, 0), vec![1]);
        assert_eq!(adjacent_players(2, 1), vec![0]);
    }

    #[test]
    fn adjacent_players_distinct_for_three_or_more() {
        assert_eq!(adjacent_players(3, 0), vec![2, 1]);
        assert_eq!(adjacent_players(4, 1), vec![0, 2]);
    }
}
