//! Initial deal for a fresh ShipWreck game.
//!
//! Per `docs/shipwreck.md`:
//!
//! 1. Shuffle the seven player cards; deal one to each player.
//! 2. Mix the remaining player cards into the wreckage deck.
//! 3. Shuffle the full wreckage deck.
//! 4. Deal 6 wreckage cards into each player's hand.
//! 5. Distribute remaining wreckage face-up, round-robin one per
//!    player per pass, until the deck is empty.
//!
//! Setup is deterministic given `(seed, cfg)`: all randomness flows
//! through the `Rng` port via Fisher-Yates shuffles.
//!
//! The return value is a [`Setup`] struct carrying both the final
//! `GameState` *and* the ordered `Vec<Event>` describing the deal
//! step-by-step. Unit 22's `initial_state` / `apply_event` machinery
//! will forward these events to the log so replay yields the same
//! state this function returns directly.

use playtest_ports::Rng;

use crate::card::Card;
use crate::config::ShipWreckConfig;
use crate::event::Event;
use crate::phase::Phase;
use crate::pool::{all_equipment, all_player_cards, all_wreckage_cards};
use crate::state::{GameState, STARTING_FOOD_COUNTER};

/// Number of wreckage cards dealt face-down into each player's hand
/// during setup (`docs/shipwreck.md`: "each player has 6 wreckage
/// cards").
pub const WRECKAGE_HAND_SIZE: usize = 6;

/// Result of the setup flow. The state is ready for `Phase::Play`;
/// `events` is the ordered chance-event stream.
///
/// This type is `#[doc(hidden)]` alongside its module — it is an
/// implementation detail of the setup flow, exposed only so the
/// integration tests and Unit 22's `initial_state` can consume it.
#[derive(Debug)]
pub struct Setup {
    pub state: GameState,
    pub events: Vec<Event>,
}

/// Run the full setup deal.
///
/// `seed` is carried here only for traceability — actual randomness
/// comes from `rng`, which the caller is expected to have seeded
/// identically on every run for the same `seed`.
#[allow(clippy::needless_pass_by_value)]
pub fn build_initial_state(
    _seed: u64,
    cfg: &ShipWreckConfig,
    rng: &mut dyn Rng,
) -> Setup {
    let n = cfg.num_players as usize;
    let mut state = GameState::empty_for(*cfg);
    let mut events: Vec<Event> = Vec::new();

    // Seed the equipment pile + starting food. Equipment is shuffled so
    // the "top card" a player sees is random-but-deterministic. Food
    // reserves start at a small positive number so played player cards
    // don't immediately starve (see `STARTING_FOOD_COUNTER` rationale).
    let mut equipment = all_equipment();
    shuffle_via_port(&mut equipment, rng);
    state.equipment_deck = equipment;
    for seat in 0..n {
        state.players[seat].food_counter = STARTING_FOOD_COUNTER;
    }

    // --- 1. Shuffle & deal one player card per seat -----------------
    let mut player_deck: Vec<_> = all_player_cards();
    shuffle_via_port(&mut player_deck, rng);

    // Deal the first `n` player cards, one per seat. Any remainder
    // mixes into the wreckage deck in step 2.
    #[allow(clippy::needless_range_loop)]
    for seat in 0..n {
        let pc = player_deck[seat];
        state.players[seat].hand.push(Card::Player(pc));
        events.push(Event::DealPlayerCard {
            player: u8::try_from(seat).expect("seat < 4 fits in u8"),
            card: pc.id,
        });
    }

    // --- 2 + 3. Build wreckage deck (leftover player cards + rest) ---
    let mut wreckage: Vec<Card> = all_wreckage_cards();
    // Phase 5 + 6: event-card toggles. `events_enabled` is the master
    // switch; per-card flags (`shark_enabled`, `typhoon_enabled`,
    // `flying_fish_enabled`) ablate individual cards for R6 restricted-
    // play cohorts. `event_card_active` composes both layers with AND.
    wreckage.retain(|c| match c {
        Card::Event(ec) => cfg.event_card_active(*ec),
        _ => true,
    });
    for leftover in player_deck.into_iter().skip(n) {
        wreckage.push(Card::Player(leftover));
    }
    shuffle_via_port(&mut wreckage, rng);

    // --- 4. Deal 6 wreckage cards to each player's hand ------------
    for seat in 0..n {
        let mut dealt: Vec<Card> = Vec::with_capacity(WRECKAGE_HAND_SIZE);
        for _ in 0..WRECKAGE_HAND_SIZE {
            let c = wreckage
                .pop()
                .expect("wreckage deck has >= 6 cards per seat after setup step 3");
            dealt.push(c);
        }
        let player_id = u8::try_from(seat).expect("seat < 4 fits in u8");
        // Record-then-commit: push the hand into state, then log it.
        for &c in &dealt {
            state.players[seat].hand.push(c);
        }
        events.push(Event::DealWreckageHand {
            player: player_id,
            cards: dealt,
        });
    }

    // --- 5. Round-robin face-up distribution ------------------------
    // Pop from the end of the deck; one card per seat per pass until
    // the deck is empty. Seat 0 always gets the first card of each
    // pass, so uneven remainders favor lower seats.
    let mut seat_cursor = 0usize;
    while let Some(c) = wreckage.pop() {
        state.face_up_pools[seat_cursor].push(c);
        events.push(Event::DealWreckageFaceUp {
            player: u8::try_from(seat_cursor).expect("seat < 4 fits in u8"),
            card: c,
        });
        seat_cursor = (seat_cursor + 1) % n;
    }

    // --- finalize ---------------------------------------------------
    state.wreckage_deck = wreckage; // empty by construction
    state.phase = Phase::Play;
    state.current_player = 0;

    Setup { state, events }
}

/// Fisher-Yates over `slice` using the [`Rng`] port. Inlined (rather
/// than the trait's default `shuffle`) so it works through
/// `&mut dyn Rng` — the trait method has a `Self: Sized` bound.
fn shuffle_via_port<T>(slice: &mut [T], rng: &mut dyn Rng) {
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
        let j = usize::try_from(j_u64).expect("j < upper fits in usize");
        slice.swap(i, j);
        i -= 1;
    }
}
