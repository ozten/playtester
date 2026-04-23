//! Invariant: the main JSONL event log never contains coordination
//! frames from stdio protocol or LLM traffic.
//!
//! Two runs, two assertions:
//!
//! 1. A full stdio game (Python subprocess driving seat 0). Main log
//!    must have no line whose top-level `"kind"` is `"turn"`, `"hello"`,
//!    `"ready"`, or `"llm_call"` — the stdio frame shapes and LLM
//!    sidecar record type.
//! 2. A full LLM-stubbed game (two `LlmAgent`s). Main log must have
//!    no `"llm_call"` or `"sidecar_header"` lines.
//!
//! This uses JSON-level parsing (not substring matching) because the
//! Cribbage event payload itself may legitimately contain strings
//! like `"action"`. The `kind` field is checked at the top-level of
//! every log record — and only `"header"`, `"event"`, and `"final"`
//! are legal there. Anything else is a coordination leak.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use assert_cmd::Command;
use async_trait::async_trait;
use playtest_adapters::{ProductionFileSystem, ProductionGameEventSink};
use playtest_agents::{LlmSidecar, SidecarHeader, sha256_hex};
use playtest_ports::{FileSystem, LlmClient, LlmError, LlmRequest, LlmResponse};
use playtest_registry::game_registry::lookup as lookup_game;
use playtest_registry::play::{LlmCliDeps, RunExtras, run_single_game_into_sink_with_extras};
use tempfile::TempDir;
use tokio::sync::Mutex as TokioMutex;

const RULES_TEXT: &str = include_str!("../../games/cribbage/rules_for_llm.md");

/// Forbidden top-level `kind` values. Any of these in the main log is
/// a coordination-leak regression.
const FORBIDDEN_KINDS: &[&str] = &[
    // Stdio protocol frames (agent -> child).
    "turn",
    // Legacy/future handshake frames — reject proactively.
    "hello",
    "ready",
    // LLM sidecar records — they belong only in `<gid>.llm.jsonl`.
    "llm_call",
    "sidecar_header",
    // HTTP-remote coordination frames from Phase 2.5 (same policy).
    "turn_prompt",
];

/// The `kind` values the main log schema v2 legitimately uses.
const ALLOWED_KINDS: &[&str] = &["header", "event", "final"];

/// Parse every line of a JSONL log; fail the test if any line has a
/// forbidden `kind` at the top level, or an unexpected `kind`.
fn assert_log_has_no_coordination_frames(log_path: &Path) {
    let text = fs::read_to_string(log_path).expect("read log");
    for (i, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let v: serde_json::Value =
            serde_json::from_str(line).unwrap_or_else(|e| panic!("line {i} not JSON: {e}\n{line}"));
        let kind = v
            .get("kind")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_else(|| panic!("line {i} has no top-level `kind`: {line}"));
        assert!(
            !FORBIDDEN_KINDS.contains(&kind),
            "main log line {i} leaked coordination frame `kind: {kind}`:\n{line}"
        );
        assert!(
            ALLOWED_KINDS.contains(&kind),
            "main log line {i} has unexpected `kind: {kind}` (not in allow list):\n{line}"
        );
    }
}

// ---------------------------------------------------------------------
// Stdio: run a full game via the CLI, inspect the produced main log.
// ---------------------------------------------------------------------

const ALWAYS_FIRST_PY: &str = r#"
import sys, json
for line in sys.stdin:
    line = line.strip()
    if not line:
        continue
    req = json.loads(line)
    reply = {
        "kind": "action",
        "prompt_id": req["prompt_id"],
        "action_index": 0,
        "scratch": {"plan": "", "notes": ""},
    }
    sys.stdout.write(json.dumps(reply) + "\n")
    sys.stdout.flush()
"#;

fn bin() -> Command {
    Command::cargo_bin("playtest").expect("bin `playtest` builds")
}

fn write_script(dir: &TempDir, name: &str, body: &str) -> PathBuf {
    let path = dir.path().join(name);
    fs::write(&path, body).unwrap();
    path
}

#[test]
fn stdio_game_main_log_has_no_protocol_frames() {
    let script_dir = TempDir::new().unwrap();
    let out_dir = TempDir::new().unwrap();
    let script = write_script(&script_dir, "always_first.py", ALWAYS_FIRST_PY);

    bin()
        .args([
            "play",
            "--game",
            "cribbage",
            "--agents",
            "stdio,random",
            "--stdio-cmd",
            "/usr/bin/python3",
            "--stdio-arg",
            script.to_str().unwrap(),
            "--seed",
            "42",
            "--games",
            "1",
            "--out",
            out_dir.path().to_str().unwrap(),
            "--fixed-time",
            "0",
        ])
        .assert()
        .success();

    let log_path = out_dir.path().join("game-0000.jsonl");
    assert_log_has_no_coordination_frames(&log_path);
}

// ---------------------------------------------------------------------
// LLM: in-process run via the registry, inspect the produced main log.
// ---------------------------------------------------------------------

struct AlwaysFirstStub;

#[async_trait]
impl LlmClient for AlwaysFirstStub {
    async fn complete(&self, _req: LlmRequest) -> Result<LlmResponse, LlmError> {
        Ok(LlmResponse {
            text: "{\"action_index\": 0, \"plan\": \"\", \"notes\": \"\"}".into(),
            input_tokens: 50,
            output_tokens: 10,
            cache_read_input_tokens: 40,
            cache_creation_input_tokens: 0,
        })
    }
}

#[test]
fn llm_game_main_log_has_no_sidecar_records_or_envelopes() {
    let out_dir = TempDir::new().unwrap();
    let log_path = out_dir.path().join("game-0000.jsonl");
    let sidecar_path = out_dir.path().join("game-0000.llm.jsonl");
    let game = lookup_game("cribbage").unwrap();

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
            SidecarHeader::new(
                "cribbage",
                42,
                sha256_hex(RULES_TEXT.as_bytes()),
                sha256_hex(b""),
            ),
        ))
        .unwrap();
    drop(rt);

    let client: Arc<dyn LlmClient> = Arc::new(AlwaysFirstStub);
    let llm_deps = LlmCliDeps {
        client,
        sidecar: Some(Arc::new(sidecar)),
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
        .unwrap();

    // Main log — no sidecar kinds, no coordination frames.
    assert_log_has_no_coordination_frames(&log_path);

    // Sidecar has its own kinds — make sure they actually live *there*,
    // not in the main log. This anchors the invariant from both sides.
    let sidecar_text = fs::read_to_string(&sidecar_path).unwrap();
    assert!(sidecar_text.contains("\"kind\":\"sidecar_header\""));
    assert!(sidecar_text.contains("\"kind\":\"llm_call\""));
}
