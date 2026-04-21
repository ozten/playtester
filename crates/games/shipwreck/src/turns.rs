//! Legal-action enumeration helpers for ShipWreck.
//!
//! Split out of `rules.rs` so `legal_actions` reads as a short
//! concatenation of sub-enumerators. Every helper returns a concrete
//! `Vec<Action>` rather than an iterator — the legal-actions slice is
//! the public surface the agent indexes into, so its ordering is part
//! of the contract and a `Vec` makes it impossible to accidentally
//! re-order.
//!
//! **Unit 22 scope**: no `PlayEventCard`, no `ResolveEvent`. Those
//! arrive in Unit 23. `PlacePlayerCard`, `PickWreckage`,
//! `BuildEquipment`, `ExtendRaft`, `EndTurn` are the only variants
//! enumerated here.

use playtest_core::PlayerId;

use crate::action::Action;
use crate::card::{Card, EquipmentKind, PlayerCard};
use crate::raft::{Raft, SlotId};
use crate::resource::ResourceCost;
use crate::state::PlayerState;

/// All raft-extension insert positions that would succeed on this
/// raft. `BaseRight` is excluded (spec forbids inserting past the
/// right base). Output order: `BaseLeft`, then `Extension(0..n)` —
/// stable so agents indexing into the legal-actions slice see a
/// predictable ordering.
#[must_use]
pub(crate) fn legal_extends(raft: &Raft) -> Vec<Action> {
    let mut out: Vec<Action> = Vec::with_capacity(1 + raft.extensions.len());
    out.push(Action::ExtendRaft {
        insert_after: SlotId::BaseLeft,
    });
    for i in 0..raft.extensions.len() {
        let idx = u16::try_from(i).expect("extension index fits in u16");
        out.push(Action::ExtendRaft {
            insert_after: SlotId::Extension(idx),
        });
    }
    out
}

/// Cartesian product of (player cards in `hand`) × (slots on `raft`
/// that don't already have a player card).
#[must_use]
pub(crate) fn legal_player_placements(
    hand: &[Card],
    raft: &Raft,
    played_players: &[crate::state::PlacedPlayerCard],
) -> Vec<Action> {
    // Collect unique player-card ids in hand (multiple copies of the
    // same id can't exist — each `PlayerCardId` is unique in the
    // spec — so one pass is enough).
    let mut out = Vec::new();
    let mut open_slots: Vec<SlotId> = Vec::with_capacity(2 + raft.extensions.len());
    open_slots.push(SlotId::BaseLeft);
    for i in 0..raft.extensions.len() {
        let idx = u16::try_from(i).expect("extension index fits in u16");
        open_slots.push(SlotId::Extension(idx));
    }
    open_slots.push(SlotId::BaseRight);

    for c in hand {
        if let Card::Player(pc) = c {
            let id = pc.id;
            for &slot in &open_slots {
                if !played_players.iter().any(|pp| pp.slot == slot) {
                    out.push(Action::PlacePlayerCard { card: id, slot });
                }
            }
        }
    }
    out
}

/// Pick actions against every player's face-up pool. Reach rules:
/// - always legal: the current player's own pool
/// - with a `Telescope` upgrade installed somewhere on the current
///   player's raft: also the immediate seat-left and seat-right
///   neighbors (wrap-around — 2-player means one opponent, the two
///   reaches coincide)
///
/// `from_pool` carries the pool owner's seat index; `card_index` is
/// the position within that pool.
#[must_use]
pub(crate) fn legal_wreckage_picks(
    current_player: PlayerId,
    face_up_pools: &[Vec<Card>],
    has_telescope: bool,
) -> Vec<Action> {
    let n = face_up_pools.len();
    let me = usize::from(current_player);
    let mut reachable: Vec<usize> = vec![me];
    if has_telescope && n >= 2 {
        let left = (me + n - 1) % n;
        let right = (me + 1) % n;
        if left != me && !reachable.contains(&left) {
            reachable.push(left);
        }
        if right != me && !reachable.contains(&right) {
            reachable.push(right);
        }
    }

    let mut out = Vec::new();
    for seat in reachable {
        let pool = &face_up_pools[seat];
        for i in 0..pool.len() {
            let idx = u16::try_from(i).expect("pool index fits in u16");
            out.push(Action::PickWreckage {
                from_pool: u8::try_from(seat).expect("seat fits in u8"),
                card_index: idx,
            });
        }
    }
    out
}

/// `BuildEquipment` is legal iff the current equipment top card exists,
/// the player can afford its (possibly Professor-discounted) cost, and
/// there is at least one raft slot without an upgrade on it.
///
/// We emit one action per `(equipment_kind, empty_slot)` pair — the
/// engine chooses which slot to place on, not the build cost. In Unit
/// 22 there is exactly one `current_equipment` kind at any time, so
/// this is effectively `legal_empty_slots` copies of one kind.
#[must_use]
#[allow(clippy::trivially_copy_pass_by_ref)]
pub(crate) fn legal_builds(
    inventory: &[u8; 5],
    raft: &Raft,
    current_equipment: Option<EquipmentKind>,
    professor_skill: bool,
) -> Vec<Action> {
    let Some(kind) = current_equipment else {
        return Vec::new();
    };
    let cost = effective_cost(kind, professor_skill);
    if !cost.can_pay(inventory) {
        return Vec::new();
    }
    let mut out = Vec::new();
    for slot in iter_raft_slots(raft) {
        if raft.upgrade_at(slot).is_none() {
            out.push(Action::BuildEquipment {
                equipment_kind: kind,
                slot,
            });
        }
    }
    out
}

/// Effective cost for `kind` given whether a Professor is on the
/// current player's raft. Professor: "can construct with 1 fewer
/// resource of any type." We apply the discount to the most-expensive
/// non-zero resource — greedy, deterministic, and always the best
/// use of a -1.
#[must_use]
pub(crate) fn effective_cost(kind: EquipmentKind, professor_skill: bool) -> ResourceCost {
    let mut amounts: [u8; 5] = *kind.cost().amounts();
    if professor_skill {
        // Find the highest amount; drop it by one.
        let (mut best_idx, mut best_amt) = (usize::MAX, 0u8);
        for (i, &a) in amounts.iter().enumerate() {
            if a > best_amt {
                best_amt = a;
                best_idx = i;
            }
        }
        if best_idx != usize::MAX && amounts[best_idx] > 0 {
            amounts[best_idx] -= 1;
        }
    }
    ResourceCost::new(amounts)
}

/// True if the current player has a `PlayerCardId::Professor` on their
/// raft (via `played_players`). Drives the
/// `ConstructWithOneFewerResource` discount.
#[must_use]
pub(crate) fn has_professor(player: &PlayerState) -> bool {
    player
        .played_players
        .iter()
        .any(|pp| pp.card.id == crate::card::PlayerCardId::Professor)
}

/// True if the current player has a Telescope upgrade installed on
/// any slot of their raft. Drives the reach-extension for wreckage
/// picks.
#[must_use]
pub(crate) fn has_telescope(player: &PlayerState) -> bool {
    player
        .raft
        .upgrades
        .values()
        .any(|eq| eq.kind == crate::card::EquipmentKind::Telescope)
}

/// True if `hand` contains at least one player card with this id.
#[must_use]
pub(crate) fn player_card_in_hand(hand: &[Card], id: crate::card::PlayerCardId) -> Option<PlayerCard> {
    for c in hand {
        if let Card::Player(pc) = c
            && pc.id == id
        {
            return Some(*pc);
        }
    }
    None
}

/// Every slot `SlotId` on this raft, in BaseLeft → Extension(0..) →
/// BaseRight order.
pub(crate) fn iter_raft_slots(raft: &Raft) -> impl Iterator<Item = SlotId> + '_ {
    let ext_count = raft.extensions.len();
    std::iter::once(SlotId::BaseLeft)
        .chain((0..ext_count).map(|i| {
            SlotId::Extension(u16::try_from(i).expect("extension index fits in u16"))
        }))
        .chain(std::iter::once(SlotId::BaseRight))
}

/// First extension-card held in this hand (if any). Returned along
/// with its index in `hand` so callers can remove it cleanly.
#[must_use]
pub(crate) fn first_extension_in_hand(
    hand: &[Card],
) -> Option<(usize, crate::card::RaftExtensionCard)> {
    for (i, c) in hand.iter().enumerate() {
        if let Card::RaftExtension(ext) = c {
            return Some((i, *ext));
        }
    }
    None
}
