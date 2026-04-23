//! Tape-level determinism (the non-trivial replay claim): an
//! `LlmAgent`-backed game recorded via [`RecordLlmClient`] reproduces
//! the same action sequence when re-run with [`PlaybackLlmClient`]
//! feeding back the recorded tape.
//!
//! The event-log replay test (`llm_replay_deterministic.rs`) only
//! proves that replay doesn't consult the LLM — which is trivially
//! true, since replay folds events. This test proves something
//! stronger: *the agent*, given identical inputs, produces identical
//! outputs because the `LlmClient` tape is a deterministic function
//! of the request bytes.
//!
//! Mechanism:
//!
//! 1. Run game A with `Record<Stub>` wrapping an "always first" stub.
//!    Every `LlmClient::complete` call is teed to a tape.
//! 2. Run game B with the same seed, same agents, but
//!    `PlaybackLlmClient` reading the tape from step 1.
//! 3. Assert both main `.jsonl` logs are byte-identical under
//!    `--fixed-time` (header + events + final), which means both runs
//!    chose the same action at every tick.

use std::fs;
use std::sync::Arc;

use async_trait::async_trait;
use playtest_adapters::{
    PlaybackLlmClient, ProductionFileSystem, ProductionGameEventSink, RecordLlmClient,
};
use playtest_ports::{LlmClient, LlmError, LlmRequest, LlmResponse};
use playtest_registry::game_registry::lookup as lookup_game;
use playtest_registry::play::{LlmCliDeps, RunExtras, run_single_game_into_sink_with_extras};
use tempfile::TempDir;

struct AlwaysFirstStub;

#[async_trait]
impl LlmClient for AlwaysFirstStub {
    async fn complete(&self, _req: LlmRequest) -> Result<LlmResponse, LlmError> {
        Ok(LlmResponse {
            text: "{\"action_index\": 0, \"plan\": \"p\", \"notes\": \"n\"}".into(),
            input_tokens: 50,
            output_tokens: 10,
            cache_read_input_tokens: 40,
            cache_creation_input_tokens: 0,
        })
    }
}

#[test]
fn recorded_llm_game_replays_to_same_action_sequence_under_playback() {
    let work = TempDir::new().unwrap();
    let tape_path = work.path().join("calls.tape.jsonl");
    let log_a = work.path().join("game-a.jsonl");
    let log_b = work.path().join("game-b.jsonl");

    let game = lookup_game("cribbage").unwrap();
    let agent_names = vec!["llm".to_owned(), "random".to_owned()];

    // ---- Run A: record every LLM call against a deterministic stub.
    {
        let recorder = Arc::new(
            RecordLlmClient::create(AlwaysFirstStub, &tape_path)
                .expect("RecordLlmClient opens tape"),
        );
        let client: Arc<dyn LlmClient> = recorder.clone();
        let llm_deps = LlmCliDeps {
            client,
            sidecar: None,
            model: "claude-stub".into(),
            max_tokens: Some(256),
            critique_sidecar: None,
            critique_spec: None,
        };
        let extras = RunExtras::new().with_llm_deps(&llm_deps);
        let fs = ProductionFileSystem::new();
        let mut sink = ProductionGameEventSink::new(fs, &log_a);
        run_single_game_into_sink_with_extras(&game, &agent_names, 42, Some(0), &extras, &mut sink)
            .expect("record run completes");
    }
    // Tape exists and is non-empty after the recorder drops.
    let tape_meta = fs::metadata(&tape_path).expect("tape file exists");
    assert!(tape_meta.len() > 0, "tape file should not be empty");

    // ---- Run B: replay the exact same game, but feed the recorded tape
    //      back through PlaybackLlmClient. Agent never consults the
    //      real stub; determinism is enforced at the port layer.
    {
        let playback =
            PlaybackLlmClient::open(&tape_path).expect("PlaybackLlmClient opens tape");
        let client: Arc<dyn LlmClient> = Arc::new(playback);
        let llm_deps = LlmCliDeps {
            client,
            sidecar: None,
            model: "claude-stub".into(),
            max_tokens: Some(256),
            critique_sidecar: None,
            critique_spec: None,
        };
        let extras = RunExtras::new().with_llm_deps(&llm_deps);
        let fs = ProductionFileSystem::new();
        let mut sink = ProductionGameEventSink::new(fs, &log_b);
        run_single_game_into_sink_with_extras(&game, &agent_names, 42, Some(0), &extras, &mut sink)
            .expect("playback run completes");
    }

    // Both runs must produce byte-identical main logs. If they differ,
    // the LLM-driven determinism story is broken — the same tape did
    // not produce the same action sequence.
    let a = fs::read(&log_a).unwrap();
    let b = fs::read(&log_b).unwrap();
    assert_eq!(
        a, b,
        "record and playback of the same tape must yield byte-identical main logs"
    );
}
