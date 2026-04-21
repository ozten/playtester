//! End-to-end Cribbage tests: `GameLoop` driving two `RandomAgent`s
//! through the full game, plus a log-replay round-trip.
//!
//! Plan verification:
//! - 100 Random-vs-Random games all terminate with a winner ≥ 121
//! - Replay reproduces the same final state

use playtest_adapters::{
    ProductionFileSystem, ProductionGameEventSink, ProductionRng, StubGameEventSink, StubRng,
};
use playtest_agents::RandomAgent;
use playtest_core::{Agent, EndReason, Game, GameLoop};
use playtest_cribbage::{CribbageConfig, CribbageGame, WINNING_SCORE};
use playtest_log::{
    EventLogWriter, LogHeader, LogRecord, SCHEMA_VERSION, compute_config_hash, replay,
};
use tempfile::tempdir;

/// Build a fresh game + agents + chance RNG + sink.
fn build_game(seed: u64) -> (CribbageGame, Vec<Box<dyn Agent<CribbageGame>>>) {
    let game = CribbageGame::new();
    let agents: Vec<Box<dyn Agent<CribbageGame>>> = vec![
        Box::new(RandomAgent::<CribbageGame, _>::new(StubRng::seeded(
            seed.wrapping_mul(101),
        ))),
        Box::new(RandomAgent::<CribbageGame, _>::new(StubRng::seeded(
            seed.wrapping_mul(103),
        ))),
    ];
    (game, agents)
}

#[tokio::test]
async fn one_hundred_random_games_terminate_with_a_valid_winner() {
    // The headline Phase-0 test: 100 full random-vs-random games all
    // reach a winner without panics, illegal actions, or hangs.
    for seed in 1..=100u64 {
        let (game, mut agents) = build_game(seed);
        let mut loop_ = GameLoop::new(&game, game.initial_state(seed, &CribbageConfig));
        let mut chance_rng = ProductionRng::from_seed(seed);
        let mut sink = StubGameEventSink::new();

        let result = loop_
            .run(agents.as_mut_slice(), &mut chance_rng, &mut sink)
            .await
            .unwrap_or_else(|e| panic!("seed {seed}: game loop error {e}"));

        assert_eq!(
            result.reason,
            EndReason::Victory,
            "seed {seed}: expected Victory, got {:?}",
            result.reason
        );
        let winner = result.winner.unwrap_or_else(|| {
            panic!("seed {seed}: expected Some(winner), got None");
        });
        let winning_score = result.scores[winner as usize];
        assert!(
            winning_score >= i32::from(WINNING_SCORE),
            "seed {seed}: winner score {winning_score} < {WINNING_SCORE}"
        );
    }
}

#[tokio::test]
async fn replay_reproduces_final_state_of_a_random_game() {
    let dir = tempdir().unwrap();
    let log_path = dir.path().join("game.jsonl");
    let seed: u64 = 424_242;

    // --- Live pass: play + log -----------------------------------------
    let live_result = {
        let (game, mut agents) = build_game(seed);
        let fs = ProductionFileSystem::new();
        let mut sink = ProductionGameEventSink::new(fs, &log_path);

        let cfg = CribbageConfig;
        let header = LogHeader {
            schema: SCHEMA_VERSION,
            game: CribbageGame::NAME.to_owned(),
            version: "0.1.0".into(),
            seed,
            agents: vec!["random".into(), "random".into()],
            started_at: 0,
            config_hash: compute_config_hash(&cfg).unwrap(),
        };
        {
            let mut writer: EventLogWriter<playtest_cribbage::Event> =
                EventLogWriter::new(&mut sink);
            writer.write_header(&header).unwrap();
        }

        let mut loop_ = GameLoop::new(&game, game.initial_state(seed, &cfg));
        let mut chance_rng = ProductionRng::from_seed(seed);
        let result = loop_
            .run(agents.as_mut_slice(), &mut chance_rng, &mut sink)
            .await
            .unwrap();

        // Emit Final record directly — see playtest-log roundtrip test
        // for the reasoning about sharing a sink across two writer
        // lifecycles.
        {
            use playtest_ports::GameEventSink;
            let final_line = serde_json::to_string(&LogRecord::<playtest_cribbage::Event>::Final {
                winner: result.winner,
                reason: result.reason.clone(),
                scores: result.scores.clone(),
            })
            .unwrap();
            sink.emit(&final_line).unwrap();
            sink.flush().unwrap();
        }

        result
    };

    // --- Replay pass ----------------------------------------------------
    let cfg = CribbageConfig;
    let game = CribbageGame::new();
    let replayed = replay::<CribbageGame>(&game, CribbageGame::NAME, &cfg, &log_path).unwrap();

    assert_eq!(replayed.header.seed, seed);
    assert_eq!(replayed.result.as_ref().unwrap().winner, live_result.winner);
    assert_eq!(replayed.result.as_ref().unwrap().scores, live_result.scores);
    // Board pins reconstructed from replay match the live final state.
    assert_eq!(
        i32::from(replayed.final_state.board.score(0)),
        live_result.scores[0]
    );
    assert_eq!(
        i32::from(replayed.final_state.board.score(1)),
        live_result.scores[1]
    );
}

#[tokio::test]
async fn single_game_finishes_quickly() {
    // Plan R0.9 implication: a single game should complete in well
    // under 0.5s. A typical Cribbage game is 8-12 hands, each with a
    // deal + discard + cut + pegging + show = tens of actions. With
    // RandomAgent the whole thing is essentially a straight-through
    // allocation run, so 0.5s is a very generous bound.
    use std::time::Instant;

    let (game, mut agents) = build_game(7);
    let mut loop_ = GameLoop::new(&game, game.initial_state(7, &CribbageConfig));
    let mut chance_rng = ProductionRng::from_seed(7);
    let mut sink = StubGameEventSink::new();

    let start = Instant::now();
    let _result = loop_
        .run(agents.as_mut_slice(), &mut chance_rng, &mut sink)
        .await
        .unwrap();
    let elapsed = start.elapsed();

    assert!(
        elapsed.as_millis() < 500,
        "single game took {}ms, should be well under 500ms",
        elapsed.as_millis()
    );
}
