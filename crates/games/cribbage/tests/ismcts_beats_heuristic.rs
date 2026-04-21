//! R2.3 exit criterion for Cribbage: `ISMCTSAgent` beats `HeuristicAgent`
//! >= 65% over 10K games.
//!
//! Mirrors the `heuristic_beats_random` test shape:
//! - `ismcts_beats_heuristic_smoke_200` (always-run) — catches
//!   catastrophic regressions in CI at a reduced 500-iter budget. The
//!   bar is set at 50% (simple majority): this smoke proves ISMCTS is
//!   *at least on par* with heuristic without paying for the full R2.3
//!   benchmark budget. Tuned this way because Cribbage is tactical
//!   enough that ISMCTS at 200 iterations sits right at parity with
//!   heuristic — a 50%-bar-at-200-iter would be flaky.
//! - `ismcts_beats_heuristic_10k` (`#[ignore]`d) — the R2.3 bar at the
//!   production budget. Run with
//!   `cargo test --release -p playtest-cribbage --test ismcts_beats_heuristic -- --ignored`.

use playtest_adapters::{ProductionRng, StubGameEventSink};
use playtest_agents::{HeuristicAgent, ISMCTSAgent, ISMCTSConfig};
use playtest_core::{Agent, Game, GameLoop};
use playtest_cribbage::{CribbageConfig, CribbageGame, cribbage_eval};

async fn play_one_game(
    seed: u64,
    ismcts_seat: u8,
    ismcts_iterations: u32,
) -> Option<u8> {
    let game = CribbageGame::new();
    let mix = 0x9E37_79B9_7F4A_7C15u64;
    let i_seed = seed ^ mix;
    let h_seed = seed ^ mix.wrapping_mul(2);

    let ismcts_cfg = ISMCTSConfig {
        iterations: ismcts_iterations,
        exploration_c: std::f64::consts::SQRT_2,
        rollout_depth: 80,
        seed: i_seed,
    };
    let ismcts_agent: Box<dyn Agent<CribbageGame>> = Box::new(
        ISMCTSAgent::<CribbageGame>::with_eval(ismcts_cfg, ismcts_seat, cribbage_eval),
    );
    let heuristic_seat = 1 - ismcts_seat;
    let heuristic_agent: Box<dyn Agent<CribbageGame>> =
        Box::new(HeuristicAgent::<CribbageGame, _>::with_temperature(
            heuristic_seat,
            cribbage_eval,
            ProductionRng::from_seed(h_seed),
            0.5,
        ));

    let mut agents: Vec<Box<dyn Agent<CribbageGame>>> = if ismcts_seat == 0 {
        vec![ismcts_agent, heuristic_agent]
    } else {
        vec![heuristic_agent, ismcts_agent]
    };

    let mut loop_ = GameLoop::new(&game, game.initial_state(seed, &CribbageConfig));
    let mut chance_rng = ProductionRng::from_seed(seed);
    let mut sink = StubGameEventSink::new();

    let result = loop_
        .run(agents.as_mut_slice(), &mut chance_rng, &mut sink)
        .await
        .unwrap_or_else(|e| panic!("seed {seed}: game-loop error {e}"));
    result.winner
}

async fn run_match(games: u32, ismcts_iterations: u32) -> (u32, u32, u32) {
    let mut ismcts_wins = 0u32;
    let mut heuristic_wins = 0u32;
    let mut draws = 0u32;
    for i in 0..games {
        // Alternate seats so dealer advantage doesn't bias.
        let seat = u8::try_from(i % 2).expect("0..=1 fits");
        let seed = u64::from(i).wrapping_mul(0xDEAD_BEEF) ^ 0x1234_5678;
        let winner = play_one_game(seed, seat, ismcts_iterations).await;
        match winner {
            Some(w) if w == seat => ismcts_wins += 1,
            Some(_) => heuristic_wins += 1,
            None => draws += 1,
        }
    }
    (ismcts_wins, heuristic_wins, draws)
}

#[tokio::test]
async fn ismcts_beats_heuristic_smoke_200() {
    // 200 games at a 500-iteration budget — fast enough for CI while
    // giving ISMCTS enough planning to clear the 50% bar reliably.
    let (iw, hw, draws) = run_match(200, 500).await;
    let total = iw + hw + draws;
    let rate = f64::from(iw) / f64::from(total);
    println!(
        "ismcts-cribbage (iter=500) vs heuristic-cribbage smoke: wins={iw}, losses={hw}, draws={draws}, rate={rate:.3}"
    );
    assert!(
        rate >= 0.50,
        "ismcts-cribbage smoke: rate {rate:.3} < 0.50 (wins={iw}, losses={hw}, draws={draws})"
    );
}

#[tokio::test]
#[ignore = "10K games — run with --ignored for the R2.3 exit criterion"]
async fn ismcts_beats_heuristic_10k() {
    // The R2.3 bar: >= 65% at the registry's default iteration budget.
    let (iw, hw, draws) = run_match(10_000, 1000).await;
    let total = iw + hw + draws;
    let rate = f64::from(iw) / f64::from(total);
    println!(
        "ismcts-cribbage (iter=1000) vs heuristic-cribbage over {total} games: wins={iw}, losses={hw}, draws={draws}, rate={rate:.4}"
    );
    assert!(
        rate >= 0.65,
        "R2.3 (cribbage) not met: rate {rate:.4} < 0.65 (wins={iw}, losses={hw})"
    );
}
