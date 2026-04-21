//! Phase 0 exit-criterion R0.9: 10,000 self-play games complete in
//! under 60 seconds on one core.
//!
//! `#[ignore]` by default — kept out of the PR-blocking test suite.
//! Run explicitly via
//!
//! ```bash
//! cargo test --release -p playtest-cli --test soak_10k -- --ignored --nocapture
//! ```
//!
//! Always build in `--release`. Debug builds are 10–30× slower and
//! will miss the 60-second bar even with a correct engine.

use std::time::Instant;

use playtest_adapters::{ProductionRng, StubGameEventSink, StubRng};
use playtest_agents::RandomAgent;
use playtest_core::{Agent, EndReason, Game, GameLoop};
use playtest_cribbage::{CribbageConfig, CribbageGame};

const SOAK_GAMES: u32 = 10_000;
const SOAK_BUDGET_SECS: f64 = 60.0;

#[test]
#[ignore = "soak -- run with `--ignored --release`"]
fn ten_thousand_games_complete_in_under_sixty_seconds() {
    let game = CribbageGame::new();
    let cfg = CribbageConfig;

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("tokio runtime");

    let start = Instant::now();
    let mut wins = [0u32; 2];
    let mut draws: u32 = 0;

    for seed in 0..u64::from(SOAK_GAMES) {
        let mut agents: Vec<Box<dyn Agent<CribbageGame>>> = vec![
            Box::new(RandomAgent::<CribbageGame, _>::new(StubRng::seeded(
                seed.wrapping_mul(0x9E37_79B9_7F4A_7C15),
            ))),
            Box::new(RandomAgent::<CribbageGame, _>::new(StubRng::seeded(
                seed.wrapping_mul(0xBF58_476D_1CE4_E5B9),
            ))),
        ];
        let mut chance_rng = ProductionRng::from_seed(seed);
        let mut sink = StubGameEventSink::new();
        let mut loop_ = GameLoop::new(&game, game.initial_state(seed, &cfg));

        let result = rt
            .block_on(loop_.run(agents.as_mut_slice(), &mut chance_rng, &mut sink))
            .unwrap_or_else(|e| panic!("seed {seed}: game loop error {e}"));

        assert_eq!(
            result.reason,
            EndReason::Victory,
            "seed {seed}: non-Victory reason {:?}",
            result.reason
        );
        match result.winner {
            Some(0) => wins[0] += 1,
            Some(1) => wins[1] += 1,
            None => draws += 1,
            Some(other) => panic!("seed {seed}: impossible winner {other}"),
        }
    }

    let elapsed = start.elapsed();
    let secs = elapsed.as_secs_f64();
    let throughput = f64::from(SOAK_GAMES) / secs;

    println!(
        "soak_10k: {SOAK_GAMES} games in {secs:.2}s ({throughput:.0} games/sec) \
         wins=[{}, {}] draws={draws}",
        wins[0], wins[1]
    );

    assert_eq!(draws, 0, "random Cribbage should not draw");
    assert!(
        secs < SOAK_BUDGET_SECS,
        "R0.9 violated: {SOAK_GAMES} games took {secs:.2}s > budget {SOAK_BUDGET_SECS}s"
    );
}
