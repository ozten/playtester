//! Determinize tests for Great Gyre (Unit 5).
//!
//! The single correctness property, per `CLAUDE.md`:
//!
//! ```text
//! public_view(determinize(s, p, rng), p) == public_view(s, p)
//! ```
//!
//! Checked here across many seeds on mid-game states (2/3/4-player),
//! plus targeted scenario tests: the happy path at `initial_state`
//! (where nothing is hidden yet), observer-hand preservation, and a
//! "resampling actually scrambles" regression guard (an
//! accidentally-identity `determinize` would pass the invariant
//! property trivially).

use std::collections::HashSet;

use playtest_adapters::StubRng;
use playtest_core::{Actor, Game};
use playtest_greatgyre::{Face, GameState, GreatGyreConfig, GreatGyreGame};
use playtest_ports::Rng;

fn pick(rng: &mut StubRng, n: usize) -> usize {
    let upper = u64::try_from(n).expect("legal count fits in u64");
    let v = rng.gen_range(0..upper).unwrap();
    usize::try_from(v).unwrap()
}

/// Same driver as `tests/public_view.rs` — deterministic pseudo-random
/// play to a representative mid-game state.
fn mid_game_state(seed: u64, num_players: u8, num_events: usize) -> GameState {
    let game = GreatGyreGame::new();
    let cfg = GreatGyreConfig::new(num_players).expect("valid player count");
    let mut state = game.initial_state(seed, &cfg);
    let mut rng = StubRng::seeded(seed);
    for _ in 0..num_events {
        if game.game_over(&state).is_some() {
            break;
        }
        match game.next_actor(&state) {
            Actor::Chance => {
                let ev = game.resolve_chance(&state, &mut rng).expect("chance resolves");
                game.apply_event(&mut state, &ev);
            }
            Actor::Player(p) => {
                let legal = game.legal_actions(&state, p);
                if legal.is_empty() {
                    break;
                }
                let idx = pick(&mut rng, legal.len());
                let events = game
                    .apply_action(&state, p, &legal[idx])
                    .expect("legal action applies");
                for e in &events {
                    game.apply_event(&mut state, e);
                }
            }
        }
    }
    state
}

#[test]
fn determinize_at_initial_state_is_identity_no_hidden_info_yet() {
    // At Phase::SurvivorDraft (fresh initial_state), every card is
    // accounted for in `undrafted_survivors` / `pending_shuffle_pool` /
    // `pending_event_pool` / raft bases / the extension pile — all
    // public — so the hidden pool is empty and determinize can't do
    // anything but hand back an equivalent state.
    let game = GreatGyreGame::new();
    for n in 2..=4u8 {
        let cfg = GreatGyreConfig::new(n).unwrap();
        let state = game.initial_state(7, &cfg);
        for observer in 0..n {
            let mut rng = StubRng::seeded(u64::from(observer) + 1);
            let out = game.determinize(&state, observer, &mut rng);
            assert_eq!(
                game.public_view(&state, observer),
                game.public_view(&out, observer),
                "n={n} observer={observer}: public view changed at initial_state"
            );
        }
    }
}

#[test]
fn determinize_preserves_public_view_over_many_seeds() {
    let game = GreatGyreGame::new();
    for n in 2..=4u8 {
        let state = mid_game_state(42u64.wrapping_add(u64::from(n) * 1013), n, 120);
        for observer in 0..n {
            let expected = game.public_view(&state, observer);
            for seed in 0..300u64 {
                let mut rng = StubRng::seeded(seed.wrapping_add(u64::from(observer) * 9_000_001));
                let out = game.determinize(&state, observer, &mut rng);
                let got = game.public_view(&out, observer);
                assert_eq!(
                    got, expected,
                    "n={n} observer={observer} seed={seed}: public view diverged"
                );
            }
        }
    }
}

#[test]
fn determinize_preserves_observer_hand_and_face_up_current_exactly() {
    let game = GreatGyreGame::new();
    let state = mid_game_state(314, 3, 100);
    for observer in 0..3u8 {
        let observer_idx = observer as usize;
        let original_hand = state.players[observer_idx].hand.clone();
        let original_current = state.players[observer_idx].current.clone();
        for seed in 0..50u64 {
            let mut rng = StubRng::seeded(seed.wrapping_mul(97));
            let out = game.determinize(&state, observer, &mut rng);
            assert_eq!(
                out.players[observer_idx].hand, original_hand,
                "observer={observer} seed={seed}: own hand must not change"
            );
            // Own face-up Current cards keep their identity and
            // position; face-down slots may (and per the next test,
            // do) change identity, but never their face or position.
            assert_eq!(
                out.players[observer_idx].current.len(),
                original_current.len()
            );
            for (got, orig) in out.players[observer_idx]
                .current
                .iter()
                .zip(original_current.iter())
            {
                assert_eq!(got.face, orig.face, "observer={observer} seed={seed}: face flipped");
                if orig.face == Face::Up {
                    assert_eq!(got.card, orig.card, "observer={observer} seed={seed}: face-up card identity changed");
                }
            }
        }
    }
}

#[test]
fn determinize_resamples_opponent_hands_and_own_face_down_current_across_seeds() {
    // Regression guard: an accidentally-identity `determinize` (e.g.
    // `state.clone()`) would pass the invariant property test above
    // trivially. Confirm it actually scrambles hidden zones.
    let game = GreatGyreGame::new();
    let state = mid_game_state(271_828, 3, 100);
    let observer: u8 = 0;
    let opponent = 1usize;

    let original_opp_hand = state.players[opponent].hand.clone();
    let original_own_face_down: Vec<_> = state.players[0]
        .current
        .iter()
        .filter(|cc| cc.face == Face::Down)
        .map(|cc| cc.card)
        .collect();

    assert!(!original_opp_hand.is_empty(), "test fixture needs a non-empty opponent hand");
    assert!(
        !original_own_face_down.is_empty(),
        "test fixture needs at least one own face-down Current card"
    );

    let mut distinct_hands = 0usize;
    let mut distinct_own_face_down = 0usize;
    for seed in 0..80u64 {
        let mut rng = StubRng::seeded(seed.wrapping_mul(31337));
        let out = game.determinize(&state, observer, &mut rng);
        if out.players[opponent].hand != original_opp_hand {
            distinct_hands += 1;
        }
        let new_face_down: Vec<_> = out.players[0]
            .current
            .iter()
            .filter(|cc| cc.face == Face::Down)
            .map(|cc| cc.card)
            .collect();
        if new_face_down != original_own_face_down {
            distinct_own_face_down += 1;
        }
    }
    assert!(distinct_hands > 0, "opponent hand never changed across 80 seeds — resampling broken");
    assert!(
        distinct_own_face_down > 0,
        "observer's own face-down Current never changed across 80 seeds — resampling broken \
         (face-down identity must be hidden from the owner too)"
    );
}

#[test]
fn determinize_produces_a_full_valid_card_universe_with_no_duplicates() {
    // Every physical card must still be accounted for exactly once
    // after determinize — resampling must not drop or duplicate cards.
    let game = GreatGyreGame::new();
    for n in 2..=4u8 {
        let state = mid_game_state(555u64.wrapping_add(u64::from(n)), n, 90);
        for observer in 0..n {
            let mut rng = StubRng::seeded(u64::from(observer).wrapping_add(1234));
            let out = game.determinize(&state, observer, &mut rng);

            let mut ids: Vec<u32> = Vec::new();
            for p in &out.players {
                ids.push(p.raft_left.id.0);
                ids.push(p.raft_right.id.0);
                ids.extend(p.hand.iter().map(|c| c.id.0));
                ids.extend(p.current.iter().map(|cc| cc.card.id.0));
                ids.extend(p.built_extensions.iter().map(|c| c.id.0));
                ids.extend(p.placed.iter().map(|pc| pc.card.id.0));
                ids.extend(p.blocked_by_walrus.iter().map(|c| c.id.0));
            }
            ids.extend(out.deep_sea_deck.iter().map(|c| c.id.0));
            ids.extend(out.final_round_deck.iter().map(|c| c.id.0));
            ids.extend(out.event_deck.iter().map(|c| c.id.0));
            ids.extend(out.discard_pile.iter().map(|c| c.id.0));
            ids.extend(out.extension_pile.iter().map(|c| c.id.0));
            ids.extend(out.undrafted_survivors.iter().map(|c| c.id.0));
            ids.extend(out.pending_shuffle_pool.iter().map(|c| c.id.0));
            ids.extend(out.pending_event_pool.iter().map(|c| c.id.0));
            for pd in &out.pending_decisions {
                if let playtest_greatgyre::PendingDecisionKind::EventReaction {
                    held_card: Some(card),
                    ..
                } = &pd.kind
                {
                    ids.push(card.id.0);
                }
            }

            let expected = playtest_greatgyre::pool::total_catalog_size(n);
            assert_eq!(
                u32::try_from(ids.len()).unwrap(),
                expected,
                "n={n} observer={observer}: card count mismatch after determinize"
            );
            let unique: HashSet<_> = ids.iter().copied().collect();
            assert_eq!(
                unique.len(),
                ids.len(),
                "n={n} observer={observer}: duplicate CardInstanceId after determinize"
            );
        }
    }
}
