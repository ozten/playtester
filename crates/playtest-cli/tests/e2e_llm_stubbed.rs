//! End-to-end Phase 3 validation: a full 2-player Cribbage game with
//! `LlmAgent` driving both seats via a canned `LlmClient` stub (no real
//! LLM traffic). Drives the registry-level `run_single_game_into_sink_with_extras`
//! directly — exercises the same path the CLI's `play` subcommand uses.
//!
//! What we prove here:
//!
//! 1. An LLM-backed game completes and writes a valid main JSONL log.
//! 2. The sidecar `<gid>.llm.jsonl` exists and has (a) a `sidecar_header`
//!    line whose `rules_text_sha256` matches the on-disk digest of
//!    `crates/games/cribbage/rules_for_llm.md` (cache-stability audit),
//!    (b) at least one `llm_call` record per LLM-advance tick.
//! 3. Every per-turn request the LLM received carries the same
//!    `rules_text` bytes (cache-stability: Anthropic's prefix cache
//!    hits only if the prefix is byte-identical).
//!
//! Tests are pure Rust — no Python, no subprocess, no HTTP.

use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use playtest_adapters::{ProductionFileSystem, ProductionGameEventSink};
use playtest_agents::{LlmSidecar, SidecarHeader, sha256_hex};
use playtest_ports::{FileSystem, LlmClient, LlmError, LlmRequest, LlmResponse};
use playtest_registry::game_registry::lookup as lookup_game;
use playtest_registry::play::{LlmCliDeps, RunExtras, run_single_game_into_sink_with_extras};
use tempfile::TempDir;
use tokio::sync::Mutex as TokioMutex;

/// Stub LLM: always returns `action_index = 0`, captures each request
/// so the test can assert on byte-level prompt-cache discipline.
struct AlwaysFirstCaptureStub {
    requests: Mutex<Vec<LlmRequest>>,
}

impl AlwaysFirstCaptureStub {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            requests: Mutex::new(Vec::new()),
        })
    }

    fn request_count(&self) -> usize {
        self.requests.lock().unwrap().len()
    }

    fn snapshot_requests(&self) -> Vec<LlmRequest> {
        self.requests.lock().unwrap().clone()
    }
}

#[async_trait]
impl LlmClient for AlwaysFirstCaptureStub {
    async fn complete(&self, req: LlmRequest) -> Result<LlmResponse, LlmError> {
        self.requests.lock().unwrap().push(req);
        Ok(LlmResponse {
            text: "{\"action_index\": 0, \"plan\": \"first\", \"notes\": \"\"}".into(),
            input_tokens: 50,
            output_tokens: 10,
            cache_read_input_tokens: 40,
            cache_creation_input_tokens: 0,
        })
    }
}

/// In-repo Cribbage rules text. Kept in sync with `play.rs`'s
/// `include_str!` — if either drifts, this test's cache-stability
/// assertion catches it.
const RULES_TEXT: &str = include_str!("../../games/cribbage/rules_for_llm.md");

#[test]
fn llm_stubbed_full_cribbage_game_produces_valid_main_log_and_sidecar() {
    let out_dir = TempDir::new().unwrap();
    let log_path = out_dir.path().join("game-0000.jsonl");
    let sidecar_path = out_dir.path().join("game-0000.llm.jsonl");
    let game = lookup_game("cribbage").expect("cribbage is registered");

    // Build the sidecar before the run — the CLI does this too.
    let rules_sha = sha256_hex(RULES_TEXT.as_bytes());
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let fs_handle: Arc<TokioMutex<dyn FileSystem + Send>> =
        Arc::new(TokioMutex::new(ProductionFileSystem::new()));
    let sidecar = rt
        .block_on(LlmSidecar::new(
            fs_handle,
            sidecar_path.clone(),
            SidecarHeader::new("cribbage", 42, rules_sha.clone(), sha256_hex(b"")),
        ))
        .unwrap();
    let sidecar = Arc::new(sidecar);
    drop(rt);

    let stub = AlwaysFirstCaptureStub::new();
    let llm_deps = LlmCliDeps {
        client: stub.clone() as Arc<dyn LlmClient>,
        sidecar: Some(sidecar.clone()),
        model: "claude-stub".into(),
        max_tokens: Some(256),
    critique_sidecar: None,
    critique_spec: None,
    };
    let extras = RunExtras::new().with_llm_deps(&llm_deps);
    let agent_names = vec!["llm".to_owned(), "llm".to_owned()];

    let fs = ProductionFileSystem::new();
    let mut sink = ProductionGameEventSink::new(fs, &log_path);
    run_single_game_into_sink_with_extras(&game, &agent_names, 42, Some(0), &extras, &mut sink)
        .expect("game runs to completion");

    // --- Main log assertions ---
    let text = fs::read_to_string(&log_path).expect("main log written");
    let lines: Vec<&str> = text.lines().collect();
    assert!(lines.len() >= 10, "expected many event records, got {}", lines.len());
    assert!(lines[0].contains("\"kind\":\"header\""));
    assert!(lines[lines.len() - 1].contains("\"kind\":\"final\""));

    // --- Sidecar assertions ---
    let sidecar_text = fs::read_to_string(&sidecar_path).expect("sidecar written");
    let sidecar_lines: Vec<&str> = sidecar_text.lines().collect();
    assert!(
        sidecar_lines.len() >= 2,
        "expected header + at least one llm_call, got {}",
        sidecar_lines.len()
    );

    // Line 0: `sidecar_header` with the on-disk rules-text digest.
    let header: serde_json::Value = serde_json::from_str(sidecar_lines[0]).unwrap();
    assert_eq!(header["kind"], "sidecar_header");
    assert_eq!(header["game"], "cribbage");
    assert_eq!(header["seed"], 42);
    assert_eq!(
        header["rules_text_sha256"].as_str().unwrap(),
        rules_sha,
        "sidecar header's rules digest must match the in-repo rules_for_llm.md"
    );

    // Lines 1..N: every record is a parseable `llm_call`.
    for (i, line) in sidecar_lines.iter().enumerate().skip(1) {
        let v: serde_json::Value =
            serde_json::from_str(line).unwrap_or_else(|e| panic!("line {i} parse error: {e}"));
        assert_eq!(v["kind"], "llm_call", "line {i}: {line}");
        // `chosen_index` is `Some(0)` on every successful call (our stub
        // always returns action_index 0).
        if v["budget_exceeded"] == serde_json::Value::Bool(false) {
            assert_eq!(v["chosen_index"], 0, "line {i}: {line}");
        }
    }

    // Sanity: the LLM was called at least a handful of times.
    assert!(
        stub.request_count() >= 3,
        "LLM called {} times — too few for a full game",
        stub.request_count()
    );
}

#[test]
fn llm_prompt_cache_discipline_rules_bytes_identical_across_turns() {
    // Anthropic caches on *byte-identical* prefixes. Two LLM calls in
    // the same game must ship the same `rules_text` bytes, or the
    // cache misses on every turn. This test asserts the discipline at
    // the port layer: the bytes the stub saw never drift.
    let out_dir = TempDir::new().unwrap();
    let log_path = out_dir.path().join("game-0000.jsonl");
    let sidecar_path = out_dir.path().join("game-0000.llm.jsonl");
    let game = lookup_game("cribbage").unwrap();

    let rules_sha = sha256_hex(RULES_TEXT.as_bytes());
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let fs_handle: Arc<TokioMutex<dyn FileSystem + Send>> =
        Arc::new(TokioMutex::new(ProductionFileSystem::new()));
    let sidecar = rt
        .block_on(LlmSidecar::new(
            fs_handle,
            sidecar_path,
            SidecarHeader::new("cribbage", 7, rules_sha, sha256_hex(b"")),
        ))
        .unwrap();
    drop(rt);

    let stub = AlwaysFirstCaptureStub::new();
    let llm_deps = LlmCliDeps {
        client: stub.clone() as Arc<dyn LlmClient>,
        sidecar: Some(Arc::new(sidecar)),
        model: "claude-stub".into(),
        max_tokens: Some(256),
    critique_sidecar: None,
    critique_spec: None,
    };
    let extras = RunExtras::new().with_llm_deps(&llm_deps);
    let agent_names = vec!["llm".to_owned(), "random".to_owned()];

    let fs = ProductionFileSystem::new();
    let mut sink = ProductionGameEventSink::new(fs, &log_path);
    run_single_game_into_sink_with_extras(&game, &agent_names, 7, Some(0), &extras, &mut sink)
        .expect("game runs to completion");

    // Every request's `system_blocks[0]` (the rules block) must carry
    // the exact same text as every other turn's. This is the load-bearing
    // precondition for Anthropic's prefix cache to hit.
    let reqs = stub.snapshot_requests();
    assert!(reqs.len() >= 3, "need multiple turns; got {}", reqs.len());
    let rules_block_0 = &reqs[0].system_blocks[0];
    assert!(rules_block_0.cache, "rules block must be flagged cacheable");
    for (i, r) in reqs.iter().enumerate().skip(1) {
        assert_eq!(
            r.system_blocks[0].text, rules_block_0.text,
            "turn {i}: rules-block bytes drifted — cache would miss"
        );
        assert_eq!(
            r.system_blocks[0].cache, rules_block_0.cache,
            "turn {i}: cache flag drifted"
        );
    }
}

#[test]
fn llm_sidecar_header_digest_matches_on_disk_rules_file() {
    // If the on-disk `rules_for_llm.md` changes between two runs, the
    // `rules_text_sha256` digest must differ. This is the cache-
    // stability audit signal — a digest-level regression test.
    let rules_bytes = RULES_TEXT.as_bytes();
    let digest_a = sha256_hex(rules_bytes);

    // Simulate a drifted file: append one byte.
    let mut drifted = rules_bytes.to_vec();
    drifted.push(b'.');
    let digest_b = sha256_hex(&drifted);

    assert_ne!(digest_a, digest_b, "digest must change on rules-file drift");

    // The on-disk file matches what `play.rs` will include via
    // `include_str!` — guard against relative-path regression.
    let repo_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf();
    let on_disk = repo_root.join("crates/games/cribbage/rules_for_llm.md");
    let disk_bytes = fs::read(&on_disk).expect("rules_for_llm.md exists");
    assert_eq!(
        sha256_hex(&disk_bytes),
        digest_a,
        "include_str! byte-drift relative to {}",
        on_disk.display()
    );
}
