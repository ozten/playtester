//! R2.3 exit criterion for ShipWreck: `ISMCTSAgent` beats
//! `HeuristicAgent` >= 65% over 10K games.
//!
//! Mirrors the `heuristic_beats_random` test shape:
//! - `ismcts_beats_heuristic_smoke` (always-run) — catches catastrophic
//!   regressions in CI. Uses a reduced 100 games × 200 iterations budget
//!   with a 40% floor because ShipWreck games are long (~150 plies) and
//!   the full 200-games × 500-iter budget used in the cribbage smoke
//!   runs too slowly for CI here. The smoke's job is to detect broken
//!   determinize / inverted rewards / compile breakage, not to reproduce
//!   R2.3 at a fractional budget.
//! - `ismcts_beats_heuristic_10k` (`#[ignore]`d) — the R2.3 bar at the
//!   production budget. Run with
//!   `cargo test --release -p playtest-shipwreck --test ismcts_beats_heuristic -- --ignored`.

use playtest_adapters::{ProductionRng, StubGameEventSink};
use playtest_agents::{HeuristicAgent, ISMCTSAgent, ISMCTSConfig};
use playtest_core::{Agent, Game, GameLoop};
use playtest_shipwreck::{ShipWreckConfig, ShipWreckGame, shipwreck_eval};

async fn play_one_game(
    seed: u64,
    ismcts_seat: u8,
    ismcts_iterations: u32,
) -> Option<u8> {
    let game = ShipWreckGame::new();
    let cfg = ShipWreckConfig::default();
    let mix = 0x9E37_79B9_7F4A_7C15u64;
    let i_seed = seed ^ mix;
    let h_seed = seed ^ mix.wrapping_mul(2);

    let ismcts_cfg = ISMCTSConfig {
        iterations: ismcts_iterations,
        exploration_c: std::f64::consts::SQRT_2,
        rollout_depth: 50,
        seed: i_seed,
    };
    let ismcts_agent: Box<dyn Agent<ShipWreckGame>> = Box::new(
        ISMCTSAgent::<ShipWreckGame>::with_eval(ismcts_cfg, ismcts_seat, shipwreck_eval),
    );
    let heuristic_seat = 1 - ismcts_seat;
    let heuristic_agent: Box<dyn Agent<ShipWreckGame>> =
        Box::new(HeuristicAgent::<ShipWreckGame, _>::with_temperature(
            heuristic_seat,
            shipwreck_eval,
            ProductionRng::from_seed(h_seed),
            0.5,
        ));

    let mut agents: Vec<Box<dyn Agent<ShipWreckGame>>> = if ismcts_seat == 0 {
        vec![ismcts_agent, heuristic_agent]
    } else {
        vec![heuristic_agent, ismcts_agent]
    };

    let mut loop_ = GameLoop::new(&game, game.initial_state(seed, &cfg));
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

#[tokio::test(flavor = "current_thread")]
async fn ismcts_beats_heuristic_smoke() {
    // 100 games × 200-iter budget, 40% floor — catastrophic-regression
    // detector only. See module doc for why this is looser than cribbage.
    let (iw, hw, draws) = run_match(100, 200).await;
    let total = iw + hw + draws;
    let rate = f64::from(iw) / f64::from(total);
    println!(
        "ismcts-shipwreck (iter=200) vs heuristic-shipwreck smoke: wins={iw}, losses={hw}, draws={draws}, rate={rate:.3}"
    );
    assert!(
        rate >= 0.40,
        "ismcts-shipwreck smoke: rate {rate:.3} < 0.40 (wins={iw}, losses={hw}, draws={draws})"
    );
}

fn run_match_parallel(games: u32, ismcts_iterations: u32) -> (u32, u32, u32) {
    use rayon::prelude::*;
    let results: Vec<(Option<u8>, u8)> = (0..games)
        .into_par_iter()
        .map(|i| {
            let seat = u8::try_from(i % 2).expect("0..=1 fits");
            let seed = u64::from(i).wrapping_mul(0xDEAD_BEEF) ^ 0x1234_5678;
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("tokio runtime");
            (rt.block_on(play_one_game(seed, seat, ismcts_iterations)), seat)
        })
        .collect();
    let mut iw = 0u32;
    let mut hw = 0u32;
    let mut dr = 0u32;
    for (winner, seat) in results {
        match winner {
            Some(w) if w == seat => iw += 1,
            Some(_) => hw += 1,
            None => dr += 1,
        }
    }
    (iw, hw, dr)
}

#[test]
#[ignore = "10K games — run with --ignored for the R2.3 exit criterion"]
fn ismcts_beats_heuristic_10k() {
    let (iw, hw, draws) = run_match_parallel(10_000, 1000);
    let total = iw + hw + draws;
    let rate = f64::from(iw) / f64::from(total);
    println!(
        "ismcts-shipwreck (iter=1000) vs heuristic-shipwreck over {total} games: wins={iw}, losses={hw}, draws={draws}, rate={rate:.4}"
    );
    assert!(
        rate >= 0.65,
        "R2.3 (shipwreck) not met: rate {rate:.4} < 0.65 (wins={iw}, losses={hw})"
    );
}
