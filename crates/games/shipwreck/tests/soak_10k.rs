//! 10,000-game random-vs-random soak test (Unit 24, R9.6).
//!
//! Guarded by `#[ignore]` so it doesn't run in the default test pass;
//! execute explicitly via:
//!
//! ```text
//! cargo test -p playtest-shipwreck --test soak_10k -- --ignored
//! ```
//!
//! Targets:
//! - Zero panics, zero engine errors.
//! - Every game produces a valid `GameResult`.
//! - ≥95% of games terminate via `EndReason::Other("deck_exhausted")`.
//! - Wall-clock completes in under 120 seconds on the reference machine.

use std::collections::HashMap;
use std::time::Instant;

use playtest_adapters::{ProductionRng, StubGameEventSink, StubRng};
use playtest_agents::RandomAgent;
use playtest_core::{Agent, Game, GameLoop};
use playtest_shipwreck::{ShipWreckConfig, ShipWreckGame};

const NUM_GAMES: u64 = 10_000;

#[tokio::test(flavor = "current_thread")]
#[ignore = "slow: 10K-game soak; run with --ignored"]
async fn random_self_play_10k_games_2p_terminate() {
    let start = Instant::now();
    let mut reason_counts: HashMap<String, usize> = HashMap::new();
    let mut total_events: u64 = 0;
    let mut games_completed: u64 = 0;
    let mut total_raft_length_winner: u64 = 0;
    let mut winners_counted: u64 = 0;

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

        let result = loop_
            .run(agents.as_mut_slice(), &mut chance_rng, &mut sink)
            .await
            .unwrap_or_else(|e| panic!("seed {seed}: game loop errored: {e}"));

        assert_eq!(result.scores.len(), 2, "seed {seed}: score count");

        let reason_key = format!("{:?}", result.reason);
        *reason_counts.entry(reason_key).or_insert(0) += 1;

        total_events += loop_.tick();
        games_completed += 1;

        // Peek the last line from the sink to recover the winner's raft
        // length for the aggregate stats — the `EndGame` event carries
        // it on `final_scores`. Each sink line is the wire format
        // `{"kind":"event","tick":N,"payload":{...event body...}}`, so
        // we reach through `payload` to the inner event variant tag.
        if let (Some(w), Some(last_line)) = (result.winner, sink.lines().last())
            && let Ok(outer) = serde_json::from_str::<serde_json::Value>(last_line)
            && let Some(val) = outer.get("payload")
            && val.get("kind").and_then(|k| k.as_str()) == Some("end_game")
            && let Some(scores) = val.get("final_scores").and_then(|s| s.as_array())
        {
            for row in scores {
                let pid = row
                    .get("player")
                    .and_then(serde_json::Value::as_u64)
                    .unwrap_or(u64::MAX);
                if pid == u64::from(w)
                    && let Some(r) = row.get("raft_length").and_then(serde_json::Value::as_u64)
                {
                    total_raft_length_winner += r;
                    winners_counted += 1;
                    break;
                }
            }
        }
    }

    let elapsed = start.elapsed();

    // Precision-loss cast: counters fit well within f64 mantissa at
    // 10K games.
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
    #[allow(clippy::cast_precision_loss)]
    let avg_winner_raft = if winners_counted > 0 {
        total_raft_length_winner as f64 / winners_counted as f64
    } else {
        0.0
    };

    println!(
        "\n=== random_self_play_10k soak ===\n\
         games: {games_completed} / {NUM_GAMES}\n\
         elapsed: {:.2}s (avg {per_game_ms:.3}ms per game)\n\
         avg events: {avg_events:.1}\n\
         avg winner raft_length: {avg_winner_raft:.2} (over {winners_counted} decided games)\n\
         reasons: {reason_counts:?}\n",
        elapsed.as_secs_f64(),
    );

    assert_eq!(games_completed, NUM_GAMES, "not every game completed");
    assert!(
        elapsed.as_secs() < 120,
        "soak test exceeded 120s budget: {:.2}s",
        elapsed.as_secs_f64()
    );

    // R9.6: ≥95% of games must terminate via deck_exhausted. The rest
    // may be draws on (rescue_points, raft_length, invention_count)
    // equality — small tails acceptable.
    let deck_exhausted = reason_counts
        .iter()
        .filter(|(k, _)| k.contains("deck_exhausted"))
        .map(|(_, v)| *v)
        .sum::<usize>();
    let num_games_usize = usize::try_from(NUM_GAMES).expect("fits in usize");
    assert!(
        deck_exhausted * 100 >= num_games_usize * 95,
        "only {deck_exhausted}/{NUM_GAMES} games ended via deck_exhausted — \
         reason distribution: {reason_counts:?}",
    );
}
