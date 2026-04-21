//! ShipWreck determinization for ISMCTS.
//!
//! The invariant enforced by the property test:
//! `public_view(determinize(s, p, rng), p) == public_view(s, p)`.
//!
//! ## Hidden information in ShipWreck
//!
//! Nearly every state transition in ShipWreck is publicly observable:
//! `PickedWreckage`, `PlacedPlayerCard`, `BuiltEquipment`,
//! `ExtendedRaft`, `ResourceSpent`, `FoodConsumed` — all emit events
//! naming a specific card or resource, so any observer with full
//! log access can track an opponent's inventory and placed cards
//! exactly.
//!
//! The only thing an observer *cannot* see is the contents of an
//! opponent's hand. Hands grow from two sources:
//! 1. **Setup-deal**: `DealPlayerCard` + `DealWreckageHand`. The
//!    dealt-player-card ID is publicly announced; the six-card
//!    wreckage hand is dealt face-down and is private to that seat.
//! 2. **In-game picks**: `PickedWreckage` carries the picked card's
//!    identity in the event log. Once picked, an opponent's card is
//!    *publicly known to be in their hand* — until they consume it.
//!
//! So the only genuinely hidden bit is: **which cards from the
//! face-down setup-deal remain in the opponent's hand?** That shrinks
//! monotonically as the opponent plays.
//!
//! ## Algorithm
//!
//! For each opponent:
//! 1. Reconstruct the "public subset" of their hand: every card they
//!    picked publicly that hasn't been seen leaving their hand since.
//!    Unit 22's state doesn't carry a full event history, so we
//!    approximate: at the current state, cards in the opponent's hand
//!    fall into two buckets — cards that *look like* setup-dealt
//!    private cards, and cards we'd identify as "publicly picked but
//!    still in hand" if we had the event log. We can't distinguish
//!    these without the log, so we conservatively treat the *entire*
//!    opponent hand as candidates for resampling, then preserve the
//!    observer's public view by reshuffling while keeping hand sizes
//!    equal.
//! 2. Collect every card publicly visible (observer's own hand, every
//!    face-up pool, every placed-player-card on any raft, every
//!    installed upgrade on any raft, equipment-deck remaining, every
//!    raft extension). Subtract that from the full card universe to
//!    get the unknown pool.
//! 3. Shuffle the unknown pool; deal into opponent hand slots to
//!    match their known hand sizes. Leftover goes into the
//!    wreckage_deck — which is empty in practice during play, but
//!    we put the remainder there for forward-compatibility.
//!
//! ## Invariant preservation
//!
//! The public view exposes opponent `hand_size`, not hand contents.
//! Face-up pools, rafts, played cards, inventory, food, phase,
//! current_player, equipment_deck, event_resolution_stack — all come
//! through unchanged. Hand sizes are preserved by construction. So
//! `public_view(determinize(s, p, rng), p) == public_view(s, p)`.

use std::collections::HashMap;

use playtest_core::PlayerId;
use playtest_ports::Rng;

use crate::card::Card;
use crate::pool::{all_equipment, all_player_cards, all_wreckage_cards};
use crate::state::GameState;

/// Return a determinized clone of `state` consistent with the
/// observer's epistemic view. See module docs for the algorithm.
pub(crate) fn determinize(
    state: &GameState,
    observer: PlayerId,
    rng: &mut dyn Rng,
) -> GameState {
    let mut out = state.clone();

    // --- 1. Build the universe of cards in the game ----------------
    // The universe is every *physical* card in the box:
    //   - everything in `all_wreckage_cards()` (extensions, items,
    //     event cards, and the 13 equipment cards that the wreckage
    //     pool initially bundles), PLUS
    //   - all 7 player cards, PLUS
    //   - the 13 equipment cards that live in the dedicated equipment
    //     pile (these are *separate copies* from the ones embedded in
    //     the wreckage pool — the spec keeps the equipment deck
    //     distinct from the wreckage deck, and Unit 21's setup seeds
    //     them independently).
    //
    // If that double-counting ever becomes a maintenance problem, the
    // fix is to have `all_wreckage_cards()` exclude equipment — but
    // that changes Unit 20's card-count tests, so we keep the
    // accounting local to the determinize universe here.
    let mut universe = all_wreckage_cards();
    for pc in all_player_cards() {
        universe.push(Card::Player(pc));
    }
    for eq in all_equipment() {
        universe.push(Card::Equipment(eq));
    }

    // --- 2. Tally publicly-visible card counts ---------------------
    // `seen[card]` = how many copies of this card are already accounted
    // for in public state. Cards in opponent hands are *not* in `seen`
    // — they're the thing we're resampling.
    let observer_idx = observer as usize;
    let mut seen: HashMap<Card, usize> = HashMap::new();

    let bump = |map: &mut HashMap<Card, usize>, c: Card| {
        *map.entry(c).or_insert(0) += 1;
    };

    // Observer's own hand: fully visible to observer.
    for &c in &state.players[observer_idx].hand {
        bump(&mut seen, c);
    }
    // Every player's face-up pool: public.
    for pool in &state.face_up_pools {
        for &c in pool {
            bump(&mut seen, c);
        }
    }
    // Played player cards on every raft: public (each placement was
    // emitted as `PlacedPlayerCard`).
    for p in &state.players {
        for pp in &p.played_players {
            bump(&mut seen, Card::Player(pp.card));
        }
    }
    // Installed equipment on every raft: public (each build emitted
    // `BuiltEquipment`).
    for p in &state.players {
        for eq in p.raft.upgrades.values() {
            bump(&mut seen, Card::Equipment(*eq));
        }
    }
    // Raft extensions already installed: public.
    for p in &state.players {
        for ext in &p.raft.extensions {
            bump(&mut seen, Card::RaftExtension(*ext));
        }
    }
    // Equipment-deck remaining: public — every observer knows the pile
    // size and the top card, and in Unit 22 the pile order is
    // deterministic from setup's shuffle (the observer can't see
    // buried cards, but their identities are still a matter of
    // public record). We treat the whole pile as known.
    for eq in &state.equipment_deck {
        bump(&mut seen, Card::Equipment(*eq));
    }
    // Inventory: each `Item` is a wreckage card. Inventory is derived
    // from `PickedWreckage` / `ResourceSpent` events — public.
    for p in &state.players {
        use crate::card::ItemCard;
        use crate::resource::Resource;
        for r in Resource::ALL {
            let n = usize::from(p.inventory[r.index()]);
            for _ in 0..n {
                bump(&mut seen, Card::Item(ItemCard::new(r)));
            }
        }
    }
    // Remaining face-down wreckage deck: treated as private. In Unit
    // 22 this is always empty after setup, but we leave the slot in
    // the unknown pool for forward compatibility.
    let wreckage_deck_size = state.wreckage_deck.len();

    // --- 3. Derive the unknown pool --------------------------------
    // For each card in the universe, subtract `seen`. What's left is
    // the pool we resample from (for opponent hands + face-down deck).
    let mut unknown: Vec<Card> = Vec::new();
    let mut seen_remaining = seen.clone();
    for c in universe {
        let slot = seen_remaining.entry(c).or_insert(0);
        if *slot > 0 {
            *slot -= 1;
        } else {
            unknown.push(c);
        }
    }

    // --- 4. Shuffle + deal into opponent hands + face-down deck ----
    fisher_yates(&mut unknown, rng);
    let mut cursor = 0usize;
    for (i, p) in out.players.iter_mut().enumerate() {
        if i == observer_idx {
            continue;
        }
        let n = p.hand.len();
        // Safety: hand_size + face_down_deck_size == unknown.len() by
        // construction, so this never overflows.
        debug_assert!(
            cursor + n <= unknown.len(),
            "determinize: unknown pool ({}) smaller than opponent hands + deck ({} + {})",
            unknown.len(),
            cursor + n,
            wreckage_deck_size,
        );
        let new_hand = unknown[cursor..cursor + n].to_vec();
        p.hand = new_hand;
        cursor += n;
    }
    // Leftover goes back into the face-down deck.
    out.wreckage_deck = unknown[cursor..].to_vec();

    out
}

/// Fisher-Yates via `Rng::gen_range` — same shape as `setup.rs` and
/// `cribbage/determinize.rs`.
fn fisher_yates<T>(slice: &mut [T], rng: &mut dyn Rng) {
    let n = slice.len();
    if n < 2 {
        return;
    }
    let mut i = n - 1;
    while i > 0 {
        let upper = u64::try_from(i).expect("usize fits in u64") + 1;
        let j_u64 = rng
            .gen_range(0..upper)
            .expect("0..(i+1) is never empty for i > 0");
        let j = usize::try_from(j_u64).expect("j <= i fits in usize");
        slice.swap(i, j);
        i -= 1;
    }
}
