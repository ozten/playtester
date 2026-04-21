//! Cribbage determinization: resample hidden information from the
//! unknown pool so a search algorithm (ISMCTS) can simulate forward
//! from the observer's epistemic position.
//!
//! The invariant — `public_view(determinize(s, p, rng), p) ==
//! public_view(s, p)` — is enforced by the property test in
//! `tests/determinize.rs`.
//!
//! Determinization preserves:
//! - observer's own hand (known to them)
//! - every card played during pegging (`played[0] ∪ played[1]`)
//! - the starter card (if cut)
//! - the dealer's crib (always known to the dealer; known to the
//!   non-dealer only once show-phase crib scoring has fired)
//! - the pegging stack, running total, board, phase, and turn-order
//!   fields (all in the public view)
//!
//! Resampled (uniformly from the unknown pool):
//! - the opponent's current hand
//! - the crib contents (when hidden from the observer)
//! - the remaining undealt deck

use std::collections::HashSet;

use playtest_ports::Rng;

use crate::card::Card;
use crate::deck;
use crate::hand::Hand;
use crate::phase::Phase;
use crate::state::GameState;
use playtest_core::PlayerId;

/// Return a determinized clone of `state` from `observer`'s point of
/// view. See module docs for the invariant.
pub(crate) fn determinize(
    state: &GameState,
    observer: PlayerId,
    rng: &mut dyn Rng,
) -> GameState {
    let observer_idx = observer as usize;
    let opponent = 1 - observer;
    let opponent_idx = opponent as usize;

    let observer_knows_crib = observer_knows_crib(state, observer);

    // 1. Gather every card the observer has seen or holds.
    let known = collect_known(state, observer_idx, observer_knows_crib);

    // 2. The unknown pool = 52-card deck − known.
    let unknown_pool = unknown_pool(&known);

    // 3. Targets to fill from the unknown pool.
    let opp_hand_size = state.hands[opponent_idx].len();
    let crib_resample_size = if observer_knows_crib {
        0
    } else {
        state.crib.len()
    };

    debug_assert!(
        unknown_pool.len() >= opp_hand_size + crib_resample_size,
        "unknown pool ({}) smaller than targets ({} + {})",
        unknown_pool.len(),
        opp_hand_size,
        crib_resample_size,
    );

    // 4. Shuffle and slice.
    let mut pool = unknown_pool;
    fisher_yates(&mut pool, rng);

    let (new_opp_hand, rest) = pool.split_at(opp_hand_size);
    let (new_crib, leftover) = if crib_resample_size > 0 {
        let (c, r) = rest.split_at(crib_resample_size);
        (Some(c.to_vec()), r.to_vec())
    } else {
        (None, rest.to_vec())
    };

    // 5. Splice the resampled slots into a clone of state.
    let mut out = state.clone();
    out.hands[opponent_idx] = Hand::new(new_opp_hand.to_vec());
    if let Some(c) = new_crib {
        out.crib = c;
    }
    // The deck is everything unknown that wasn't assigned to a slot.
    // Order doesn't matter for public_view equality, but must be
    // deterministic per rng.
    out.deck = leftover;

    out
}

/// Does `observer` know the current crib's contents?
///
/// The dealer always knows their own crib. The non-dealer sees it once
/// crib scoring has fired — i.e. `phase == Show && show_step >= 3`.
fn observer_knows_crib(state: &GameState, observer: PlayerId) -> bool {
    if observer == state.dealer {
        return true;
    }
    state.phase == Phase::Show && state.show_step >= 3
}

/// Every card the observer has seen plus their own hand.
fn collect_known(
    state: &GameState,
    observer_idx: usize,
    observer_knows_crib: bool,
) -> HashSet<Card> {
    let mut known: HashSet<Card> = HashSet::with_capacity(52);

    // Observer's hand.
    for &c in state.hands[observer_idx].cards() {
        known.insert(c);
    }
    // Every card played during pegging (both players).
    for per_player in &state.played {
        for &c in per_player {
            known.insert(c);
        }
    }
    // Pegging stack is a subset of `played`, but include defensively
    // in case a future refactor decouples them.
    for &c in &state.pegging_stack {
        known.insert(c);
    }
    // Starter.
    if let Some(c) = state.starter {
        known.insert(c);
    }
    // Crib, when known to the observer.
    if observer_knows_crib {
        for &c in &state.crib {
            known.insert(c);
        }
    }
    known
}

/// Cards in the full 52-card deck that aren't in `known`. Ordered by
/// `deck::fresh()` so the output is deterministic before shuffling.
fn unknown_pool(known: &HashSet<Card>) -> Vec<Card> {
    deck::fresh()
        .iter()
        .copied()
        .filter(|c| !known.contains(c))
        .collect()
}

/// Fisher-Yates via `Rng::gen_range`. `Rng::shuffle` has a `Self: Sized`
/// bound that makes it unavailable through `&mut dyn Rng`, so we
/// implement the loop here (same pattern as `deck::shuffle`).
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
