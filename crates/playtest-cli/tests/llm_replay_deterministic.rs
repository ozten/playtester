//! Replay-determinism invariant (R3.6): after recording a full Cribbage
//! game whose seat 0 is driven by `LlmAgent` (backed by a canned stub),
//! the resulting JSONL log replays byte-for-byte from `seed + events`
//! without re-contacting any LLM.
//!
//! The LLM port is *not* consulted during `playtest_log::replay` — the
//! replay folds `apply_event` over the recorded event stream. The
//! canonical agent never runs. This is the trivially-true determinism
//! story; the non-trivial tape-level variant lives in
//! `llm_tape_replay_deterministic.rs`.

use std::fs;
use std::sync::Arc;

use async_trait::async_trait;
use playtest_adapters::{ProductionFileSystem, ProductionGameEventSink};
use playtest_cribbage::{CribbageConfig, CribbageGame};
use playtest_core::Game;
use playtest_log::replay;
use playtest_ports::{LlmClient, LlmError, LlmRequest, LlmResponse};
use playtest_registry::game_registry::lookup as lookup_game;
use playtest_registry::play::{LlmCliDeps, RunExtras, run_single_game_into_sink_with_extras};
use tempfile::TempDir;

/// Stub LLM that always returns action_index 0.
struct AlwaysFirstStub;

#[async_trait]
impl LlmClient for AlwaysFirstStub {
    async fn complete(&self, _req: LlmRequest) -> Result<LlmResponse, LlmError> {
        Ok(LlmResponse {
            text: "{\"action_index\": 0, \"plan\": \"\", \"notes\": \"\"}".into(),
            input_tokens: 10,
            output_tokens: 5,
            cache_read_input_tokens: 0,
            cache_creation_input_tokens: 0,
        })
    }
}

#[test]
fn llm_recorded_game_replays_from_seed_without_llm_contact() {
    let out_dir = TempDir::new().unwrap();
    let log_path = out_dir.path().join("game-0000.jsonl");
    let game = lookup_game("cribbage").unwrap();

    let client: Arc<dyn LlmClient> = Arc::new(AlwaysFirstStub);
    let llm_deps = LlmCliDeps {
        client,
        sidecar: None,
        model: "claude-stub".into(),
        max_tokens: Some(256),
    critique_sidecar: None,
    critique_spec: None,
    };
    let extras = RunExtras::new().with_llm_deps(&llm_deps);
    let agent_names = vec!["llm".to_owned(), "random".to_owned()];

    let fs = ProductionFileSystem::new();
    let mut sink = ProductionGameEventSink::new(fs, &log_path);
    run_single_game_into_sink_with_extras(&game, &agent_names, 42, Some(0), &extras, &mut sink)
        .expect("game runs");

    // The log exists.
    assert!(log_path.exists());
    let text_before = fs::read_to_string(&log_path).unwrap();

    // Replay the log using the public `playtest_log::replay` API. Note
    // there is no `LlmClient` passed in — replay does not consult any
    // agent; it folds events into state.
    let cribbage = CribbageGame::new();
    let rep = replay::<CribbageGame>(&cribbage, "cribbage", &CribbageConfig, &log_path)
        .expect("replay reconstructs state");

    // Replay produced a snapshot per event.
    assert!(
        !rep.snapshots.is_empty(),
        "expected at least one snapshot from replay"
    );
    // The game terminated — replay's `result` is Some.
    assert!(rep.result.is_some(), "replay lost the final record");

    // The header matches the run's seed.
    assert_eq!(rep.header.seed, 42);
    assert_eq!(rep.header.game, "cribbage");

    // The log is untouched after replay — replay is read-only.
    let text_after = fs::read_to_string(&log_path).unwrap();
    assert_eq!(text_before, text_after);

    // Replay public view at tick N should match what an online
    // observer would see at tick N. We can't call public_view on the
    // final state alone and expect it to equal anything specific, but
    // we can check shape: every recorded tick has a snapshot.
    let event_count = text_before
        .lines()
        .filter(|l| l.contains("\"kind\":\"event\""))
        .count();
    assert_eq!(
        rep.snapshots.len(),
        event_count,
        "one snapshot per event line: {} vs {}",
        rep.snapshots.len(),
        event_count
    );

    // Seal the invariant: replay constructed a final state. Spot-check
    // by asking for a public view at the end.
    let _ = cribbage.public_view(&rep.final_state, 0);
}
