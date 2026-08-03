//! Stat aggregation and small helpers shared by `rules.rs`.
//!
//! Unit 2 scope: only *printed* per-card numbers (hope, food, hand-tab,
//! space rules, resource cost) are aggregated here — never an *active*
//! ability (extra actions/draws/adds, alternate draw sources,
//! reactions). Those land in Unit 3. This split is why
//! `compute_hand_limit` and `compute_food` are fully implemented (the
//! spec states their formulas as plain per-card numbers) while the
//! Phase 1/2/3 budgets stay fixed at `1` regardless of which
//! survivors/modifications are on the raft.

use crate::card::{Card, CardInstanceId, CardKind, ModificationKind};
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
/// Stowaway, plus every placed space-occupying modification.
#[must_use]
pub fn raft_capacity(player: &PlayerState) -> (u32, u32) {
    let mut used = 0u32;
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
