//! `heuristic-greatgyre` vs `random` sanity + exit-criterion tests
//! (Unit 5), mirroring ShipWreck's `heuristic_beats_random.rs`.
//!
//! Two entry points:
//! - `heuristic_beats_random_smoke_200` (always-run) — 200 two-player
//!   games, alternating seats; catches catastrophic regressions in CI.
//!   Bar: >= 70% win share, per the plan's sanity benchmark.
//! - `heuristic_beats_random_10k` (`#[ignore]`) — the full 10K-game
//!   version of the same match, run on demand for a tighter estimate.

use playtest_adapters::{ProductionRng, StubGameEventSink};
use playtest_agents::{HeuristicAgent, RandomAgent};
use playtest_core::{Agent, Game, GameLoop};
use playtest_greatgyre::{GreatGyreConfig, GreatGyreGame, greatgyre_eval};

async fn play_one_game(seed: u64, heuristic_seat: u8) -> Option<u8> {
    let game = GreatGyreGame::new();
    let cfg = GreatGyreConfig::new(2).expect("2 players always valid");
    let mix = 0x9E37_79B9_7F4A_7C15u64;
    let h_seed = seed ^ mix;
    let r_seed = seed ^ mix.wrapping_mul(2);

    let heuristic_agent: Box<dyn Agent<GreatGyreGame>> = Box::new(HeuristicAgent::<
        GreatGyreGame,
        _,
    >::with_temperature(
        heuristic_seat,
        greatgyre_eval,
        ProductionRng::from_seed(h_seed),
        0.5,
    ));
    let random_agent: Box<dyn Agent<GreatGyreGame>> =
        Box::new(RandomAgent::<GreatGyreGame, _>::new(ProductionRng::from_seed(r_seed)));

    let mut agents: Vec<Box<dyn Agent<GreatGyreGame>>> = if heuristic_seat == 0 {
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

async fn run_match(games: u32) -> (u32, u32, u32) {
    let mut heuristic_wins = 0u32;
    let mut random_wins = 0u32;
    let mut draws = 0u32;
    for i in 0..games {
        let seat = u8::try_from(i % 2).expect("0..=1 fits");
        let seed = u64::from(i).wrapping_mul(0xDEAD_BEEF) ^ 0x1234_5678;
        let winner = play_one_game(seed, seat).await;
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
    let (hw, rw, draws) = run_match(200).await;
    let total = hw + rw + draws;
    let rate = f64::from(hw) / f64::from(total);
    println!(
        "heuristic-greatgyre vs random over {total} games: wins={hw}, losses={rw}, draws={draws}, rate={rate:.3}"
    );
    assert!(
        rate >= 0.70,
        "heuristic-greatgyre sanity: wins={hw}, losses={rw}, draws={draws} — rate {rate:.3} < 0.70"
    );
}

#[tokio::test(flavor = "current_thread")]
#[ignore = "10K games — run with --ignored for a tighter win-rate estimate"]
async fn heuristic_beats_random_10k() {
    let (hw, rw, draws) = run_match(10_000).await;
    let total = hw + rw + draws;
    let rate = f64::from(hw) / f64::from(total);
    println!(
        "heuristic-greatgyre vs random over {total} games: wins={hw}, losses={rw}, draws={draws}, rate={rate:.4}"
    );
    assert!(
        rate >= 0.70,
        "heuristic-greatgyre 10K: rate {rate:.4} < 0.70 (wins={hw}, losses={rw})"
    );
}
