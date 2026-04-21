//! Determinize invariant property test (Unit 22).
//!
//! The single correctness condition for `Game::determinize` is:
//!
//!     public_view(determinize(s, p, rng), p) == public_view(s, p)
//!
//! Checked here across 1000 seeds on a mid-game state for each
//! observer seat, for both 2-player and 3-player games.

use playtest_adapters::StubRng;
use playtest_agents::RandomAgent;
use playtest_core::{Actor, Agent, Game, GameLoop, PlayerId};
use playtest_ports::GameEventSink;
use playtest_shipwreck::{ShipWreckConfig, ShipWreckGame};

/// Drive a random-vs-random game for up to `max_turns` steps, then
/// stop and return whatever state we ended up in. This gives us a
/// realistic mid-game state (non-empty opponent hands, partially
/// consumed face-up pools, some placements / builds).
async fn mid_game_state(
    seed: u64,
    num_players: u8,
    max_turns: usize,
) -> playtest_shipwreck::GameState {
    let game = ShipWreckGame::new();
    let cfg = ShipWreckConfig::new(num_players).unwrap();
    let state = game.initial_state(seed, &cfg);
    let mut loop_ = GameLoop::new(&game, state);
    let mut sink = playtest_adapters::StubGameEventSink::new();
    let mut agents: Vec<Box<dyn Agent<ShipWreckGame>>> = (0..num_players)
        .map(|i| {
            let a: Box<dyn Agent<ShipWreckGame>> = Box::new(
                RandomAgent::<ShipWreckGame, _>::new(StubRng::seeded(
                    seed.wrapping_mul(101 + u64::from(i) * 2),
                )),
            );
            a
        })
        .collect();
    // Bounded loop — we snapshot whenever we hit max_turns by just
    // letting the game run until that many events happen and then
    // cloning the loop's state.
    //
    // Simpler: run to completion and then rewind? No — we want a
    // mid-game state. Drive step-by-step manually.
    let mut turns = 0usize;
    while turns < max_turns {
        if game.game_over(loop_.state()).is_some() {
            break;
        }
        let actor = game.next_actor(loop_.state());
        match actor {
            Actor::Chance => {
                // ShipWreck has no in-play chance events (all setup
                // chance is baked into initial_state). If we ever land
                // here, fall through to the full run.
                break;
            }
            Actor::Player(p) => {
                let legal = game.legal_actions(loop_.state(), p);
                if legal.is_empty() {
                    break;
                }
                let view = game.public_view(loop_.state(), p);
                let choice = agents[p as usize].choose(&view, &legal).await.unwrap();
                let events = game.apply_action(loop_.state(), p, &legal[choice]).unwrap();
                for e in &events {
                    // emit via sink (ignored contents) + fold into state.
                    let line = serde_json::to_string(e).unwrap();
                    sink.emit(&line).unwrap();
                }
                // Rebuild a new GameLoop-like state by mutating through
                // `into_state + new`. Simpler: apply each event via
                // game.apply_event on a mutable clone.
                let mut st = loop_.state().clone();
                for e in &events {
                    game.apply_event(&mut st, e);
                }
                loop_ = GameLoop::new(&game, st);
                turns += 1;
            }
        }
    }
    loop_.into_state()
}

#[tokio::test]
async fn determinize_preserves_public_view_2p_over_1000_seeds() {
    let game = ShipWreckGame::new();
    // Use one fixed mid-game state; exercise determinize with 1000
    // different RNG seeds.
    let state = mid_game_state(12_345, 2, 10).await;

    for observer in [0u8, 1u8] {
        let expected = game.public_view(&state, observer);
        for seed in 0..1000u64 {
            let mut rng = StubRng::seeded(seed.wrapping_add(u64::from(observer) * 9_000_001));
            let out = game.determinize(&state, observer, &mut rng);
            let got = game.public_view(&out, observer);
            assert_eq!(
                got, expected,
                "observer={observer} seed={seed}: public view diverged"
            );
        }
    }
}

#[tokio::test]
async fn determinize_preserves_public_view_3p_per_observer() {
    let game = ShipWreckGame::new();
    let state = mid_game_state(999, 3, 6).await;
    for observer in 0..3u8 {
        let expected = game.public_view(&state, observer);
        for seed in 0..200u64 {
            let mut rng = StubRng::seeded(seed.wrapping_mul(7919) ^ u64::from(observer));
            let out = game.determinize(&state, observer, &mut rng);
            let got = game.public_view(&out, observer);
            assert_eq!(
                got, expected,
                "3p observer={observer} seed={seed}: public view diverged"
            );
        }
    }
}

#[tokio::test]
async fn determinize_at_initial_state_preserves_public_view() {
    // At initial_state the face-up pools are full, hands are 7 cards
    // each, inventory is zero — a stressful case for the algorithm.
    let game = ShipWreckGame::new();
    let cfg = ShipWreckConfig::default();
    let state = game.initial_state(42, &cfg);
    for observer in [0u8, 1u8] {
        let expected = game.public_view(&state, observer);
        for seed in 0..100u64 {
            let mut rng = StubRng::seeded(seed);
            let out = game.determinize(&state, observer, &mut rng);
            assert_eq!(
                game.public_view(&out, observer),
                expected,
                "observer={observer} seed={seed}"
            );
        }
    }
}

#[tokio::test]
async fn determinize_preserves_observer_hand_exactly() {
    let game = ShipWreckGame::new();
    let state = mid_game_state(77, 2, 8).await;
    for observer in [0u8, 1u8] {
        let original: Vec<_> = state.players[observer as usize].hand.clone();
        for seed in 0..50u64 {
            let mut rng = StubRng::seeded(seed);
            let out = game.determinize(&state, observer, &mut rng);
            assert_eq!(
                out.players[observer as usize].hand, original,
                "observer={observer} seed={seed}: own hand must not change"
            );
        }
    }
}

#[tokio::test]
async fn determinize_resamples_opponent_hand_across_seeds() {
    // A cheap sanity check that determinize actually shuffles — without
    // this, an implementation that returns state.clone() would pass the
    // invariant property test.
    let game = ShipWreckGame::new();
    let state = mid_game_state(314, 2, 8).await;
    let observer: PlayerId = 0;
    let opp_seat = 1usize;
    let original_opp_hand = state.players[opp_seat].hand.clone();
    if original_opp_hand.is_empty() {
        // Opponent has no cards to resample — can't test. Not a failure.
        return;
    }
    let mut distinct = 0usize;
    for seed in 0..50u64 {
        let mut rng = StubRng::seeded(seed.wrapping_mul(31337));
        let out = game.determinize(&state, observer, &mut rng);
        if out.players[opp_seat].hand != original_opp_hand {
            distinct += 1;
        }
    }
    assert!(
        distinct > 0,
        "determinize produced identical opponent hand across 50 seeds; resampling broken"
    );
}
