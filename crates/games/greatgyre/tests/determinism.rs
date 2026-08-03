//! Determinism byte-identity test (Unit 5): the same seed produces
//! byte-identical event-log output across two independent runs,
//! mirroring how the CLI's smoke test proves this for Cribbage
//! (`crates/playtest-cli/tests/cli_smoke.rs::
//! same_seed_produces_byte_identical_output_across_two_runs`) — here
//! exercised directly at the engine layer (no CLI process spawn
//! needed) across every supported player count.

use playtest_adapters::{ProductionRng, StubGameEventSink, StubRng};
use playtest_agents::RandomAgent;
use playtest_core::{Agent, Game, GameLoop};
use playtest_greatgyre::{GreatGyreConfig, GreatGyreGame};

/// Play one full game under `seed`/`n` and return every JSONL line the
/// loop emitted, in order.
async fn play_and_collect_lines(seed: u64, n: u8) -> Vec<String> {
    let game = GreatGyreGame::new();
    let cfg = GreatGyreConfig::new(n).expect("valid player count");
    let state = game.initial_state(seed, &cfg);
    let mut loop_ = GameLoop::new(&game, state);
    let mut chance_rng = ProductionRng::from_seed(seed);
    let mut sink = StubGameEventSink::new();
    let mut agents: Vec<Box<dyn Agent<GreatGyreGame>>> = (0..n)
        .map(|p| {
            Box::new(RandomAgent::<GreatGyreGame, _>::new(StubRng::seeded(
                seed.wrapping_mul(101).wrapping_add(u64::from(p)),
            ))) as Box<dyn Agent<GreatGyreGame>>
        })
        .collect();

    loop_
        .run(agents.as_mut_slice(), &mut chance_rng, &mut sink)
        .await
        .unwrap_or_else(|e| panic!("seed {seed} n={n}: game loop errored: {e}"));

    sink.lines().to_vec()
}

#[tokio::test(flavor = "current_thread")]
async fn same_seed_produces_byte_identical_event_log_across_two_runs() {
    for n in 2..=4u8 {
        for seed in [1u64, 42, 7_777] {
            let a = play_and_collect_lines(seed, n).await;
            let b = play_and_collect_lines(seed, n).await;
            assert_eq!(
                a.len(),
                b.len(),
                "seed={seed} n={n}: event count differs across identical-seed runs"
            );
            for (i, (la, lb)) in a.iter().zip(b.iter()).enumerate() {
                assert_eq!(
                    la, lb,
                    "seed={seed} n={n}: line {i} differs across identical-seed runs"
                );
            }
        }
    }
}

#[tokio::test(flavor = "current_thread")]
async fn different_seeds_usually_produce_different_event_logs() {
    // Sanity guard against a `determinism` test that would pass
    // vacuously if the engine ignored the seed entirely.
    let a = play_and_collect_lines(1, 3).await;
    let b = play_and_collect_lines(2, 3).await;
    assert_ne!(a, b, "different seeds produced identical event logs — seed is not wired through");
}
