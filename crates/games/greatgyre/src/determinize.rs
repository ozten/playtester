//! Determinization for Great Gyre.
//!
//! The invariant this module exists to satisfy (per `CLAUDE.md`):
//!
//! ```text
//! public_view(determinize(s, p, rng), p) == public_view(s, p)
//! ```
//!
//! ## Hidden information in Great Gyre
//!
//! `CardInstanceId` gives every physical card a stable identity
//! assigned once, up front, by [`crate::pool::build_catalog`] — the
//! exact same function `setup::initial_state` calls. That means the
//! "universe" of cards for a `num_players`-seat game can be
//! reconstructed byte-for-byte (same ids, same kinds) just by calling
//! `build_catalog(num_players, seed)` again with the *same* `seed`
//! `initial_state` used — no need to walk an event log. `seed` here is
//! `GameState::id_permutation_seed`, not re-derived: `build_catalog`
//! permutes ids per `(num_players, seed)` (see `crate::pool`'s doc
//! comment), so calling it with a different or default seed would
//! silently rebuild a *different* id→kind mapping than the one the
//! real game state was built from.
//!
//! From `public_view`'s redaction rules (see that module's doc
//! comment), exactly three kinds of zone are hidden from an observer
//! `P`:
//! 1. every *other* player's hand,
//! 2. every player's face-down Current cards — **including `P`'s
//!    own** (face-down identity is hidden from everyone until drawn),
//! 3. the three face-down decks (Deep Sea, Final Round, Event).
//!
//! Everything else — `P`'s own hand, every face-up Current card,
//! every raft (base cards, extensions, placed survivors/
//! modifications, Walrus blocks), the Discard Pile, the
//! undrafted-survivor pool, and the pre-shuffle pools
//! (`pending_shuffle_pool` / `pending_event_pool`, both empty after
//! the post-draft shuffle and, before it, fully derivable from public
//! catalog knowledge since nothing has been dealt from them yet) — is
//! public and left untouched.
//!
//! One more wrinkle: `PendingDecisionKind::EventReaction`'s
//! `held_card` holds a Walrus card *in transit* (already publicly
//! played via `EventCardPlayed`, not yet placed or discarded) — it's
//! not sitting in anyone's `PlayerState` or in `discard_pile`, so it
//! must be excluded from the resample pool explicitly or it would get
//! shuffled into some hidden zone *and* stay referenced from the
//! pending decision, duplicating a physical card.
//!
//! ## Algorithm
//!
//! 1. Rebuild the full card universe via `build_catalog`.
//! 2. Mark every card whose location is publicly known (`seen`) —
//!    raft bases, face-up Current cards, placed cards, built
//!    extensions, Walrus blocks, the discard pile, the undrafted-
//!    survivor pool, the pending pre-shuffle pools, any
//!    `EventReaction` held card, and (for `P`'s own hand only) the
//!    observer's hand.
//! 3. Everything else is `hidden`: shuffle it, then deal it back out
//!    to match the *sizes* the state already commits to — opponent
//!    hand sizes, per-seat face-down Current slot counts, and the
//!    three deck sizes. Sizes are exactly what stays fixed by
//!    `public_view`, so this preserves the invariant by construction.

use std::collections::HashSet;

use playtest_core::PlayerId;
use playtest_ports::Rng;

use crate::card::{Card, CardInstanceId};
use crate::state::{Face, GameState, PendingDecisionKind};

pub(crate) fn determinize(state: &GameState, observer: PlayerId, rng: &mut dyn Rng) -> GameState {
    let mut out = state.clone();
    let observer_idx = observer as usize;

    let num_players = u8::try_from(state.players.len()).expect("player count fits in u8");
    let universe = full_universe(num_players, state.id_permutation_seed);
    let seen = publicly_known_ids(state, observer_idx);

    // --- Derive the hidden pool + shuffle it ------------------------
    let mut hidden: Vec<Card> = universe.into_iter().filter(|c| !seen.contains(&c.id)).collect();
    fisher_yates(&mut hidden, rng);

    deal_hidden_pool(state, &mut out, observer_idx, &hidden);

    out
}

/// Every physical card in a `num_players`-seat game, rebuilt via the
/// same catalog function `setup::initial_state` uses, with the *same*
/// `seed` (`GameState::id_permutation_seed`) — same `CardInstanceId`s
/// mapped to the same kinds, guaranteed.
fn full_universe(num_players: u8, seed: u64) -> Vec<Card> {
    let catalog = crate::pool::build_catalog(num_players, seed);
    let mut universe: Vec<Card> = Vec::new();
    for (left, right) in &catalog.raft_pairs {
        universe.push(*left);
        universe.push(*right);
    }
    universe.extend(catalog.survivors.iter().copied());
    universe.extend(catalog.shuffle_pool.iter().copied());
    universe.extend(catalog.events.iter().copied());
    universe.extend(catalog.extensions.iter().copied());
    universe
}

/// Ids whose current location `observer` already knows — see the
/// module doc's "seen" list.
fn publicly_known_ids(state: &GameState, observer_idx: usize) -> HashSet<CardInstanceId> {
    let mut seen: HashSet<CardInstanceId> = HashSet::new();
    let mut mark = |id: CardInstanceId| {
        seen.insert(id);
    };

    for p in &state.players {
        mark(p.raft_left.id);
        mark(p.raft_right.id);
        for cc in &p.current {
            if cc.face == Face::Up {
                mark(cc.card.id);
            }
        }
        for pc in &p.placed {
            mark(pc.card.id);
        }
        for c in &p.built_extensions {
            mark(c.id);
        }
        for c in &p.blocked_by_walrus {
            mark(c.id);
        }
    }
    for c in &state.discard_pile {
        mark(c.id);
    }
    for c in &state.undrafted_survivors {
        mark(c.id);
    }
    for c in &state.pending_shuffle_pool {
        mark(c.id);
    }
    for c in &state.pending_event_pool {
        mark(c.id);
    }
    for c in &state.extension_pile {
        mark(c.id);
    }
    for pd in &state.pending_decisions {
        if let PendingDecisionKind::EventReaction {
            held_card: Some(card),
            ..
        } = &pd.kind
        {
            mark(card.id);
        }
    }
    // The observer's own hand is public to them.
    for c in &state.players[observer_idx].hand {
        mark(c.id);
    }

    seen
}

/// Deal the shuffled `hidden` pool back into every hidden zone: every
/// non-observer hand, every face-down Current slot (including the
/// observer's own), and the three face-down decks — in that order.
fn deal_hidden_pool(
    state: &GameState,
    out: &mut GameState,
    observer_idx: usize,
    hidden: &[Card],
) {
    let mut cursor = 0usize;
    for (i, p) in out.players.iter_mut().enumerate() {
        if i != observer_idx {
            let n = p.hand.len();
            debug_assert!(
                cursor + n <= hidden.len(),
                "determinize: hidden pool exhausted while resampling seat {i}'s hand"
            );
            p.hand = hidden[cursor..cursor + n].to_vec();
            cursor += n;
        }
        for cc in &mut p.current {
            if cc.face == Face::Down {
                debug_assert!(
                    cursor < hidden.len(),
                    "determinize: hidden pool exhausted while resampling seat {i}'s Current"
                );
                cc.card = hidden[cursor];
                cursor += 1;
            }
        }
    }

    let dsd_len = state.deep_sea_deck.len();
    out.deep_sea_deck = hidden[cursor..cursor + dsd_len].to_vec();
    cursor += dsd_len;

    let frd_len = state.final_round_deck.len();
    out.final_round_deck = hidden[cursor..cursor + frd_len].to_vec();
    cursor += frd_len;

    let ed_len = state.event_deck.len();
    out.event_deck = hidden[cursor..cursor + ed_len].to_vec();
    cursor += ed_len;

    debug_assert_eq!(
        cursor,
        hidden.len(),
        "determinize: hidden pool size didn't match the sum of hidden-zone sizes"
    );
}

/// Fisher-Yates via `Rng::gen_range` — same shape as `setup.rs` (other
/// games) and `cribbage/determinize.rs`.
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
