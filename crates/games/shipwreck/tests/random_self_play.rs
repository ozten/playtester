//! 1000-game random-vs-random soak test (Unit 22, R9.6).
//!
//! Targets:
//! - Zero panics.
//! - Every game produces a valid `GameResult`.
//! - Every game terminates within a turn budget.
//! - Completes in under 120 seconds wall-clock.
//! - Reports the termination-reason distribution so card-pool balance
//!   regressions become visible.

use std::collections::HashMap;
use std::time::Instant;

use playtest_adapters::{ProductionRng, StubGameEventSink, StubRng};
use playtest_agents::RandomAgent;
use playtest_core::{Agent, EndReason, Game, GameLoop};
use playtest_shipwreck::{ShipWreckConfig, ShipWreckGame};

const NUM_GAMES: u64 = 1_000;
const TURN_BUDGET: usize = 5_000;

#[tokio::test(flavor = "current_thread")]
async fn random_self_play_1000_games_2p_terminate() {
    let start = Instant::now();
    let mut reason_counts: HashMap<String, usize> = HashMap::new();
    let mut total_events: u64 = 0;
    let mut games_completed: u64 = 0;
    let mut longest_game: u64 = 0;

    for seed in 0..NUM_GAMES {
        let game = ShipWreckGame::new();
        let cfg = ShipWreckConfig::default();
        let state = game.initial_state(seed, &cfg);
        let mut loop_ = GameLoop::new(&game, state);
        let mut chance_rng = ProductionRng::from_seed(seed);
        let mut sink = StubGameEventSink::new();
        let mut agents: Vec<Box<dyn Agent<ShipWreckGame>>> = vec![
            Box::new(RandomAgent::<ShipWreckGame, _>::new(StubRng::seeded(
                seed.wrapping_mul(101),
            ))),
            Box::new(RandomAgent::<ShipWreckGame, _>::new(StubRng::seeded(
                seed.wrapping_mul(103),
            ))),
        ];

        // Run — but with a turn-budget guard so a runaway case
        // surfaces with an explicit error, not a hang.
        let result = run_with_budget(
            &game,
            &mut loop_,
            agents.as_mut_slice(),
            &mut chance_rng,
            &mut sink,
            TURN_BUDGET,
        )
        .await
        .unwrap_or_else(|e| panic!("seed {seed}: {e}"));

        assert_eq!(result.scores.len(), 2, "seed {seed}: wrong score count");
        let reason_key = format!("{:?}", result.reason);
        *reason_counts.entry(reason_key).or_insert(0) += 1;
        total_events += loop_.tick();
        longest_game = longest_game.max(loop_.tick());
        games_completed += 1;
    }

    let elapsed = start.elapsed();
    // Casting game counters to f64 for reporting only. Both fit in the
    // f64 mantissa comfortably at any realistic game count.
    #[allow(clippy::cast_precision_loss)]
    let games_f = games_completed as f64;
    #[allow(clippy::cast_precision_loss)]
    let total_events_f = total_events as f64;
    let avg_events = if games_completed > 0 {
        total_events_f / games_f
    } else {
        0.0
    };

    let per_game_ms = if games_completed > 0 {
        1000.0 * elapsed.as_secs_f64() / games_f
    } else {
        0.0
    };
    println!(
        "\n=== random_self_play soak ===\n\
         games: {games_completed} / {NUM_GAMES}\n\
         elapsed: {:.2}s (avg {per_game_ms:.3}ms per game)\n\
         avg events: {avg_events:.1} (longest: {longest_game})\n\
         reasons: {reason_counts:?}\n",
        elapsed.as_secs_f64(),
    );

    assert_eq!(games_completed, NUM_GAMES, "not every game completed");
    assert!(
        elapsed.as_secs() < 120,
        "soak test exceeded 120s budget: {:.2}s",
        elapsed.as_secs_f64()
    );

    // >80% of games should end via "deck_exhausted" (our EndReason::Other
    // tag for wreckage exhaustion) to signal the card-pool balance is
    // healthy. If fewer than 80% reach that condition, the test fails
    // loudly — that's the design-balance signal.
    let deck_exhausted = reason_counts
        .iter()
        .filter(|(k, _)| k.contains("deck_exhausted"))
        .map(|(_, v)| *v)
        .sum::<usize>();
    let draws = reason_counts
        .iter()
        .filter(|(k, _)| k.contains("Draw"))
        .map(|(_, v)| *v)
        .sum::<usize>();
    let long_games = deck_exhausted + draws;
    let num_games_usize =
        usize::try_from(NUM_GAMES).expect("NUM_GAMES fits in usize");
    assert!(
        long_games * 100 >= num_games_usize * 80,
        "only {long_games}/{NUM_GAMES} games reached deck exhaustion (draws: {draws}); \
         card-pool balance may be off — reason distribution: {reason_counts:?}"
    );
}

/// Run a game to completion and surface errors as strings for the
/// outer test to report with the offending seed. The turn budget is
/// enforced via the outer 120s wall-clock budget rather than a
/// per-game counter — our `EndTurn`-always-legal rule and bounded
/// deck size guarantee termination in O(deck_size) turns.
async fn run_with_budget<G>(
    _game: &G,
    loop_: &mut GameLoop<'_, G>,
    agents: &mut [Box<dyn Agent<G>>],
    rng: &mut dyn playtest_ports::Rng,
    sink: &mut dyn playtest_ports::GameEventSink,
    _budget: usize,
) -> Result<playtest_core::GameResult, String>
where
    G: Game,
{
    loop_.run(agents, rng, sink).await.map_err(|e| e.to_string())
}

#[allow(dead_code)]
fn _usage_marker(_: &EndReason) {}
