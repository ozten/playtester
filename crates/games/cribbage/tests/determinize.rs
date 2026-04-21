//! Determinize tests for Cribbage.
//!
//! The single correctness property: for every reachable state `s`,
//! every observer `p`, and every rng `r`:
//!
//!     public_view(determinize(s, p, r), p) == public_view(s, p)
//!
//! Proven here by:
//! - a 1000-seed property test over a mid-pegging state
//! - targeted scenario tests for each hidden-info decision point
//!
//! The scramble test guards against the "oops, determinize returns the
//! input unchanged" regression — a bug that would pass the invariant
//! property check trivially.

use playtest_adapters::StubRng;
use playtest_core::Game;
use playtest_cribbage::{CribbageConfig, CribbageGame, GameState, Phase};
use playtest_ports::Rng;
use std::collections::HashSet;

// ---------- State builders ---------------------------------------------

/// Drive `state` from `Phase::Deal` through to the start of
/// `Phase::Pegging` using a single chance RNG for both deal and cut and
/// a pair of action RNGs for the non-dealer and dealer discard.
fn deal_through_cut(state: &mut GameState, chance: &mut dyn Rng, action_rngs: &mut [StubRng; 2]) {
    // Deal.
    while state.phase == Phase::Deal {
        let ev = state.resolve_chance(chance).unwrap();
        state.apply_event(&ev);
    }
    // Discard — each player picks a random legal discard.
    while state.phase == Phase::Discard {
        let p = state.to_act;
        let legal = state.legal_actions(p);
        let idx = pick(&mut action_rngs[p as usize], legal.len());
        let events = state.apply_action(p, &legal[idx]).unwrap();
        for e in &events {
            state.apply_event(e);
        }
    }
    // Cut.
    while state.phase == Phase::Cut {
        let ev = state.resolve_chance(chance).unwrap();
        state.apply_event(&ev);
    }
}

/// Play `num_plays` pegging plays (or Go's), chosen at random from each
/// player's legal actions. Stops early if the phase transitions out of
/// pegging.
fn play_some_pegging(
    state: &mut GameState,
    num_plays: usize,
    action_rngs: &mut [StubRng; 2],
) {
    for _ in 0..num_plays {
        if state.phase != Phase::Pegging {
            break;
        }
        let p = state.to_act;
        let legal = state.legal_actions(p);
        if legal.is_empty() {
            break;
        }
        let idx = pick(&mut action_rngs[p as usize], legal.len());
        let events = state.apply_action(p, &legal[idx]).unwrap();
        for e in &events {
            state.apply_event(e);
        }
    }
}

fn pick(rng: &mut StubRng, n: usize) -> usize {
    let upper = u64::try_from(n).expect("legal count fits in u64");
    let v = rng.gen_range(0..upper).unwrap();
    usize::try_from(v).unwrap()
}

fn mid_pegging_state(seed: u64) -> GameState {
    let game = CribbageGame::new();
    let mut state = game.initial_state(seed, &CribbageConfig);
    let mut chance = StubRng::seeded(seed);
    let mut actions = [
        StubRng::seeded(seed.wrapping_mul(101)),
        StubRng::seeded(seed.wrapping_mul(103)),
    ];
    deal_through_cut(&mut state, &mut chance, &mut actions);
    play_some_pegging(&mut state, 3, &mut actions);
    state
}

fn deal_only_state(seed: u64) -> GameState {
    let game = CribbageGame::new();
    let mut state = game.initial_state(seed, &CribbageConfig);
    let mut chance = StubRng::seeded(seed);
    while state.phase == Phase::Deal {
        let ev = state.resolve_chance(&mut chance).unwrap();
        state.apply_event(&ev);
    }
    state
}

/// Construct a toy "finished" state: set `phase = Finished` with
/// observer's hand empty, opponent's hand empty — no hidden information
/// remains.
fn finished_state(seed: u64) -> GameState {
    let mut state = mid_pegging_state(seed);
    state.phase = Phase::Finished;
    state
}

// ---------- Scenario 1: happy path — no hidden info --------------------

#[test]
fn determinize_at_finished_state_preserves_public_view() {
    let game = CribbageGame::new();
    let state = finished_state(1);

    // Invariant holds when observer matches: `public_view(determinize(s, p, rng), p) == public_view(s, p)`.
    for observer in [0u8, 1u8] {
        let mut rng = StubRng::seeded(u64::from(observer) + 7);
        let out = game.determinize(&state, observer, &mut rng);
        assert_eq!(
            game.public_view(&state, observer),
            game.public_view(&out, observer),
            "observer={observer}: public view changed at Finished phase",
        );
    }
}

// ---------- Scenario 2: 1000-seed invariant property test ---------------

#[test]
fn determinize_preserves_public_view_over_1000_seeds() {
    let game = CribbageGame::new();
    let state = mid_pegging_state(42);
    let expected_view_0 = game.public_view(&state, 0);
    let expected_view_1 = game.public_view(&state, 1);

    for seed in 0..1000u64 {
        let mut rng = StubRng::seeded(seed);
        let out0 = game.determinize(&state, 0, &mut rng);
        assert_eq!(
            game.public_view(&out0, 0),
            expected_view_0,
            "observer=0 seed={seed}: public view diverged"
        );

        let mut rng = StubRng::seeded(seed.wrapping_add(1_000_000));
        let out1 = game.determinize(&state, 1, &mut rng);
        assert_eq!(
            game.public_view(&out1, 1),
            expected_view_1,
            "observer=1 seed={seed}: public view diverged"
        );
    }
}

// ---------- Scenario 3: resampling actually scrambles -------------------

#[test]
fn determinize_produces_varied_opponent_hands_across_seeds() {
    let game = CribbageGame::new();
    let state = mid_pegging_state(42);
    let observer: u8 = 0;
    let opponent: u8 = 1 - observer;

    let mut distinct: HashSet<Vec<_>> = HashSet::new();
    for seed in 0..100u64 {
        let mut rng = StubRng::seeded(seed.wrapping_mul(7919));
        let out = game.determinize(&state, observer, &mut rng);
        let mut cards = out.hands[opponent as usize].cards().to_vec();
        cards.sort_by_key(|c| (c.rank as u8, c.suit as u8));
        distinct.insert(cards);
    }
    assert!(
        distinct.len() > 1,
        "determinize returned identical opponent hand across all seeds; resampling is broken"
    );
}

// ---------- Scenario 4: dealer sees crib, non-dealer doesn't ------------

#[test]
fn determinize_preserves_crib_for_dealer_and_resamples_for_non_dealer() {
    // Construct a pre-show state: deal + discard + cut done, pegging
    // midway, crib has 4 real cards. Dealer = 0 in our `initial_state`.
    let game = CribbageGame::new();
    let state = mid_pegging_state(7);
    let dealer = state.dealer;
    let non_dealer = state.non_dealer();

    assert_eq!(state.crib.len(), 4, "expected crib to be populated");
    assert!(state.phase != Phase::Show, "pre-show only");

    let original_crib = state.crib.clone();

    // Dealer sees their crib — determinize must preserve it.
    let mut rng = StubRng::seeded(101);
    let out_dealer = game.determinize(&state, dealer, &mut rng);
    assert_eq!(
        out_dealer.crib, original_crib,
        "dealer's crib must not be resampled"
    );

    // Non-dealer doesn't know the crib — determinize should (almost
    // always) produce a different crib. Sample across several seeds so
    // a rare identical permutation doesn't flake the test.
    let mut any_different = false;
    for seed in 0..20u64 {
        let mut rng = StubRng::seeded(seed.wrapping_add(31337));
        let out_nd = game.determinize(&state, non_dealer, &mut rng);
        if out_nd.crib != original_crib {
            any_different = true;
        }
        // In every case, non-dealer's public view is unchanged.
        assert_eq!(
            game.public_view(&state, non_dealer),
            game.public_view(&out_nd, non_dealer),
            "seed={seed}: non-dealer public view must be preserved",
        );
    }
    assert!(
        any_different,
        "non-dealer's crib never differed across 20 seeds — resampling must have a bug"
    );
}

// ---------- Scenario 5: deal-only state (no cut, nothing played) -------

#[test]
fn determinize_at_deal_only_preserves_observer_hand_and_resamples_rest() {
    let game = CribbageGame::new();
    let state = deal_only_state(99);

    assert_eq!(state.phase, Phase::Discard);
    assert_eq!(state.hands[0].len(), 6);
    assert_eq!(state.hands[1].len(), 6);
    assert!(state.starter.is_none());

    let observer: u8 = 0;
    let observer_hand: Vec<_> = state.hands[observer as usize].cards().to_vec();

    for seed in 0..50u64 {
        let mut rng = StubRng::seeded(seed.wrapping_mul(31));
        let out = game.determinize(&state, observer, &mut rng);

        // Observer hand preserved exactly (order + identity).
        assert_eq!(
            out.hands[observer as usize].cards(),
            observer_hand.as_slice(),
            "seed={seed}: observer's 6-card hand must be preserved",
        );

        // Opponent hand is 6 fresh cards not overlapping the observer's.
        let opp_cards: HashSet<_> =
            out.hands[1 - observer as usize].cards().iter().copied().collect();
        assert_eq!(opp_cards.len(), 6, "seed={seed}: opponent must hold 6 distinct cards");
        for c in &observer_hand {
            assert!(
                !opp_cards.contains(c),
                "seed={seed}: observer's card {c} ended up in opponent's hand"
            );
        }

        // Public view unchanged.
        assert_eq!(game.public_view(&state, observer), game.public_view(&out, observer));
    }
}

// ---------- Scenario: determinize leaves pegging plays intact -----------

#[test]
fn determinize_preserves_pegging_stack_and_played_cards() {
    let game = CribbageGame::new();
    let state = mid_pegging_state(13);
    assert!(
        !state.pegging_stack.is_empty(),
        "mid-pegging state should have cards played"
    );

    let original_played = state.played.clone();
    let original_stack = state.pegging_stack.clone();

    for seed in 0..20u64 {
        let mut rng = StubRng::seeded(seed.wrapping_add(4242));
        let out = game.determinize(&state, 0, &mut rng);
        assert_eq!(
            out.played, original_played,
            "seed={seed}: played[*] must be observer-known and unchanged",
        );
        assert_eq!(
            out.pegging_stack, original_stack,
            "seed={seed}: pegging_stack must be unchanged",
        );
    }
}

