//! R2.2 exit criterion for ShipWreck: `HeuristicAgent` beats
//! `RandomAgent` > 90% over 10K games.
//!
//! Two entry points:
//! - `heuristic_beats_random_10k` (tagged `#[ignore = "10K games — run with --ignored for the R2.2 exit criterion"]`) — the full bar.
//! - `heuristic_beats_random_smoke_200` (always-run) — catches
//!   catastrophic regressions in CI.

use playtest_adapters::{ProductionRng, StubGameEventSink};
use playtest_agents::{GreedyAgent, HeuristicAgent, RandomAgent};
use playtest_core::{Agent, Game, GameLoop};
use playtest_shipwreck::{ShipWreckConfig, ShipWreckGame, shipwreck_eval};

async fn play_one_game(
    seed: u64,
    heuristic_seat: u8,
    use_heuristic: bool,
) -> Option<u8> {
    let game = ShipWreckGame::new();
    let cfg = ShipWreckConfig::default();
    let mix = 0x9E37_79B9_7F4A_7C15u64;
    let h_seed = seed ^ mix;
    let r_seed = seed ^ mix.wrapping_mul(2);

    let heuristic_agent: Box<dyn Agent<ShipWreckGame>> = if use_heuristic {
        let rng = ProductionRng::from_seed(h_seed);
        Box::new(HeuristicAgent::<ShipWreckGame, _>::with_temperature(
            heuristic_seat,
            shipwreck_eval,
            rng,
            0.5,
        ))
    } else {
        Box::new(GreedyAgent::<ShipWreckGame>::new(
            heuristic_seat,
            shipwreck_eval,
        ))
    };
    let random_agent: Box<dyn Agent<ShipWreckGame>> =
        Box::new(RandomAgent::<ShipWreckGame, _>::new(
            ProductionRng::from_seed(r_seed),
        ));

    let mut agents: Vec<Box<dyn Agent<ShipWreckGame>>> = if heuristic_seat == 0 {
        vec![heuristic_agent, random_agent]
    } else {
        vec![random_agent, heuristic_agent]
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

async fn run_match(games: u32, use_heuristic: bool) -> (u32, u32, u32) {
    let mut heuristic_wins = 0u32;
    let mut random_wins = 0u32;
    let mut draws = 0u32;
    for i in 0..games {
        let seat = u8::try_from(i % 2).expect("0..=1 fits");
        let seed = u64::from(i).wrapping_mul(0xDEAD_BEEF) ^ 0x1234_5678;
        let winner = play_one_game(seed, seat, use_heuristic).await;
        match winner {
            Some(w) if w == seat => heuristic_wins += 1,
            Some(_) => random_wins += 1,
            None => draws += 1,
        }
    }
    (heuristic_wins, random_wins, draws)
}

#[tokio::test(flavor = "current_thread")]
async fn heuristic_beats_random_smoke_200() {
    let (hw, rw, draws) = run_match(200, true).await;
    let total = hw + rw + draws;
    let rate = f64::from(hw) / f64::from(total);
    println!(
        "heuristic-shipwreck smoke: wins={hw}, losses={rw}, draws={draws}, rate={rate:.3}"
    );
    assert!(
        rate >= 0.60,
        "heuristic-shipwreck smoke: wins={hw}, losses={rw}, draws={draws} — rate {rate:.3} < 0.60"
    );
}

#[tokio::test(flavor = "current_thread")]
#[ignore = "10K games — run with --ignored for the R2.2 exit criterion"]
async fn heuristic_beats_random_10k() {
    let (hw, rw, draws) = run_match(10_000, true).await;
    let total = hw + rw + draws;
    let rate = f64::from(hw) / f64::from(total);
    println!(
        "heuristic-shipwreck vs random over {total} games: wins={hw}, losses={rw}, draws={draws}, rate={rate:.4}"
    );
    assert!(
        rate >= 0.90,
        "R2.2 (shipwreck) not met: rate {rate:.4} < 0.90 (wins={hw}, losses={rw})"
    );
}

#[tokio::test(flavor = "current_thread")]
#[ignore = "10K games — run with --ignored for the R2.2 exit criterion"]
async fn greedy_beats_random_10k() {
    let (gw, rw, draws) = run_match(10_000, false).await;
    let total = gw + rw + draws;
    let rate = f64::from(gw) / f64::from(total);
    println!(
        "greedy-shipwreck vs random over {total} games: wins={gw}, losses={rw}, draws={draws}, rate={rate:.4}"
    );
    assert!(
        rate >= 0.90,
        "greedy-shipwreck not meeting 90% bar: rate {rate:.4} (wins={gw}, losses={rw})"
    );
}
