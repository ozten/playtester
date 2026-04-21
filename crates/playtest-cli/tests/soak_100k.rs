//! Phase 0 exit-criteria R0.10 and R0.11:
//! - R0.10: every game produces a complete, replayable log.
//! - R0.11: zero panics in a 100,000-game soak.
//!
//! `#[ignore]` by default — this test takes several minutes even in
//! `--release` and is not meant to run per PR. Run explicitly via
//!
//! ```bash
//! cargo test --release -p playtest-cli --test soak_100k -- --ignored --nocapture
//! ```
//!
//! **Panic handling.** A panic inside `GameLoop::run` bubbles up and
//! fails the test immediately, which is exactly what we want — the
//! test printing surrounding context (seed, game index) gives us the
//! minimal reproducer. We do *not* wrap each game in
//! `catch_unwind`; the whole point of R0.11 is that no panics
//! happen, so the test failing loudly on one is the right signal.
//!
//! **Replay sampling.** Writing 100K JSONL files to disk would cost
//! ~5 GB; instead we write-and-replay a 1 000-game sample through
//! real files and verify the final state matches. The other 99 000
//! games prove zero-panic via in-memory `StubGameEventSink` only.

use std::time::Instant;

use playtest_adapters::{
    ProductionFileSystem, ProductionGameEventSink, ProductionRng, StubGameEventSink, StubRng,
};
use playtest_agents::RandomAgent;
use playtest_core::{Agent, EndReason, Game, GameLoop};
use playtest_cribbage::{CribbageConfig, CribbageGame, Event as CribbageEvent};
use playtest_log::{
    EventLogWriter, LogHeader, LogRecord, SCHEMA_VERSION, compute_config_hash, replay,
};
use playtest_ports::GameEventSink;
use tempfile::TempDir;

const SOAK_GAMES: u64 = 100_000;
const REPLAY_SAMPLE_EVERY: u64 = 100; // 100K / 100 = 1 000 sampled games

#[test]
#[ignore = "soak -- run with `--ignored --release`"]
#[allow(
    clippy::too_many_lines,
    reason = "top-level soak body -- splitting it adds indirection without clarity"
)]
fn one_hundred_thousand_games_panic_free_and_sample_logs_replay_cleanly() {
    let game = CribbageGame::new();
    let cfg = CribbageConfig;

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("tokio runtime");

    // Sample sink dir for the games we disk-write + replay.
    let dir = TempDir::new().expect("tempdir");

    let start = Instant::now();
    let mut completed: u64 = 0;
    let mut wins = [0u64; 2];
    let mut replay_sampled: u64 = 0;
    let mut events_emitted: u64 = 0;

    for seed in 0..SOAK_GAMES {
        let mut agents: Vec<Box<dyn Agent<CribbageGame>>> = vec![
            Box::new(RandomAgent::<CribbageGame, _>::new(StubRng::seeded(
                seed.wrapping_mul(0x9E37_79B9_7F4A_7C15),
            ))),
            Box::new(RandomAgent::<CribbageGame, _>::new(StubRng::seeded(
                seed.wrapping_mul(0xBF58_476D_1CE4_E5B9),
            ))),
        ];
        let mut chance_rng = ProductionRng::from_seed(seed);
        let mut loop_ = GameLoop::new(&game, game.initial_state(seed, &cfg));

        let sample_this_game = seed % REPLAY_SAMPLE_EVERY == 0;

        let result = if sample_this_game {
            // Full disk-write + replay round-trip.
            let out_path = dir.path().join(format!("soak-{seed}.jsonl"));
            let fs = ProductionFileSystem::new();
            let mut sink = ProductionGameEventSink::new(fs, &out_path);

            let header = LogHeader {
                schema: SCHEMA_VERSION,
                game: CribbageGame::NAME.to_owned(),
                version: env!("CARGO_PKG_VERSION").to_owned(),
                seed,
                agents: vec!["random".into(), "random".into()],
                started_at: 0,
                config_hash: compute_config_hash(&cfg).expect("config serializes"),
            };
            {
                let mut writer: EventLogWriter<CribbageEvent> = EventLogWriter::new(&mut sink);
                writer.write_header(&header).expect("write header");
            }

            let result = rt
                .block_on(loop_.run(agents.as_mut_slice(), &mut chance_rng, &mut sink))
                .unwrap_or_else(|e| panic!("seed {seed}: game loop error {e}"));

            let final_line = serde_json::to_string(&LogRecord::<CribbageEvent>::Final {
                winner: result.winner,
                reason: result.reason.clone(),
                scores: result.scores.clone(),
                // Soak uses `started_at: 0`; keep finished_at fixed for
                // byte-stable logs. Real wall-clock time would make the
                // sampled logs non-deterministic across runs.
                finished_at: 0,
            })
            .expect("serialize final");
            sink.emit(&final_line).expect("emit final");
            sink.flush().expect("flush");

            // Replay and assert final state matches.
            let replayed = replay::<CribbageGame>(&game, CribbageGame::NAME, &cfg, &out_path)
                .unwrap_or_else(|e| panic!("seed {seed}: replay error {e}"));
            assert_eq!(
                replayed.result.as_ref().expect("final record").winner,
                result.winner,
                "seed {seed}: replay winner differs"
            );
            assert_eq!(
                replayed.result.as_ref().expect("final record").scores,
                result.scores,
                "seed {seed}: replay scores differ"
            );
            // Drop the file now -- we don't need 1 000 log files on disk.
            std::fs::remove_file(&out_path).expect("remove sample log");
            replay_sampled += 1;
            result
        } else {
            let mut sink = StubGameEventSink::new();
            let r = rt
                .block_on(loop_.run(agents.as_mut_slice(), &mut chance_rng, &mut sink))
                .unwrap_or_else(|e| panic!("seed {seed}: game loop error {e}"));
            events_emitted = events_emitted.saturating_add(sink.lines().len() as u64);
            r
        };

        assert_eq!(
            result.reason,
            EndReason::Victory,
            "seed {seed}: non-Victory result {:?}",
            result.reason
        );
        match result.winner {
            Some(0) => wins[0] += 1,
            Some(1) => wins[1] += 1,
            Some(other) => panic!("seed {seed}: impossible winner id {other}"),
            None => panic!("seed {seed}: Victory with no winner"),
        }
        completed += 1;

        if seed.is_multiple_of(10_000) && seed > 0 {
            let secs = start.elapsed().as_secs_f64();
            println!(
                "soak_100k: {seed}/{SOAK_GAMES} complete in {secs:.1}s \
                 ({:.0} games/sec)",
                f64::from(u32::try_from(seed).unwrap_or(u32::MAX)) / secs,
            );
        }
    }

    let elapsed = start.elapsed().as_secs_f64();
    println!(
        "soak_100k: {completed} games, zero panics, {elapsed:.1}s \
         wins=[{}, {}] replay-sampled={replay_sampled} events-emitted={events_emitted}",
        wins[0], wins[1]
    );

    assert_eq!(completed, SOAK_GAMES, "not all games completed");
    assert!(
        replay_sampled >= SOAK_GAMES / REPLAY_SAMPLE_EVERY,
        "replay sample undercount: {replay_sampled}"
    );
}
