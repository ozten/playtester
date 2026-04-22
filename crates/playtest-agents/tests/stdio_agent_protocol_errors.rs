//! Protocol-error paths for `StdioAgent` — wrong prompt_id, out-of-range
//! action_index, too many garbage lines, child error frames.
//!
//! Uses python subprocess scripts the same way `stdio_agent_happy_path`
//! does; CI runs on Linux with `python3` on PATH.

use std::path::{Path, PathBuf};

use playtest_adapters::ProductionRng;
use playtest_agents::{StdioAgent, StdioAgentConfig};
use playtest_core::{Actor, Agent, Game};
use playtest_cribbage::{CribbageConfig, CribbageGame};
use tempfile::TempDir;

fn discard_phase_state(seed: u64) -> <CribbageGame as Game>::State {
    let game = CribbageGame::new();
    let mut state = game.initial_state(seed, &CribbageConfig);
    let mut rng = ProductionRng::from_seed(seed);
    while matches!(game.next_actor(&state), Actor::Chance) {
        let event = game.resolve_chance(&state, &mut rng).unwrap();
        game.apply_event(&mut state, &event);
    }
    state
}

fn write_py(dir: &TempDir, name: &str, body: &str) -> PathBuf {
    let path = dir.path().join(name);
    std::fs::write(&path, body).unwrap();
    path
}

fn cfg_py(script: &Path) -> StdioAgentConfig {
    StdioAgentConfig {
        command: PathBuf::from("/usr/bin/python3"),
        args: vec![script.to_string_lossy().to_string()],
    }
}

// Script variants. All read one line per turn; each one injects a
// different protocol violation.

const BAD_PROMPT_ID_PY: &str = r#"
import sys, json
line = sys.stdin.readline().strip()
req = json.loads(line)
reply = {
    "kind": "action",
    "prompt_id": req["prompt_id"] + 999,
    "action_index": 0,
    "scratch": {"plan": "", "notes": ""},
}
sys.stdout.write(json.dumps(reply) + "\n")
sys.stdout.flush()
"#;

const OUT_OF_RANGE_PY: &str = r#"
import sys, json
line = sys.stdin.readline().strip()
req = json.loads(line)
reply = {
    "kind": "action",
    "prompt_id": req["prompt_id"],
    "action_index": 999,
    "scratch": {"plan": "", "notes": ""},
}
sys.stdout.write(json.dumps(reply) + "\n")
sys.stdout.flush()
"#;

const GARBAGE_THEN_VALID_PY: &str = r#"
import sys, json
# Emit 17 non-JSON lines, then a valid reply. The agent caps at 16,
# so this never reaches the valid reply.
line = sys.stdin.readline().strip()
req = json.loads(line)
for i in range(17):
    sys.stdout.write(f"garbage line {i}\n")
sys.stdout.flush()
reply = {
    "kind": "action",
    "prompt_id": req["prompt_id"],
    "action_index": 0,
    "scratch": {"plan": "", "notes": ""},
}
sys.stdout.write(json.dumps(reply) + "\n")
sys.stdout.flush()
"#;

const ERROR_FRAME_PY: &str = r#"
import sys, json
line = sys.stdin.readline().strip()
req = json.loads(line)
reply = {
    "kind": "error",
    "prompt_id": req["prompt_id"],
    "message": "child encountered an unrecoverable condition",
}
sys.stdout.write(json.dumps(reply) + "\n")
sys.stdout.flush()
"#;

const TWO_GARBAGE_THEN_VALID_PY: &str = r#"
import sys, json
line = sys.stdin.readline().strip()
req = json.loads(line)
# A couple of warning lines ahead of the real reply — the agent should
# swallow them (cap is 16).
sys.stdout.write("WARN: some debug output from the child\n")
sys.stdout.write("\n")  # blank line counts as garbage too
reply = {
    "kind": "action",
    "prompt_id": req["prompt_id"],
    "action_index": 0,
    "scratch": {"plan": "ok", "notes": ""},
}
sys.stdout.write(json.dumps(reply) + "\n")
sys.stdout.flush()
"#;

#[tokio::test]
async fn wrong_prompt_id_surfaces_as_mismatch_error() {
    let dir = TempDir::new().unwrap();
    let script = write_py(&dir, "bad_prompt.py", BAD_PROMPT_ID_PY);
    let mut agent: StdioAgent<CribbageGame> =
        StdioAgent::new(0, "cribbage", cfg_py(&script)).unwrap();
    let game = CribbageGame::new();
    let state = discard_phase_state(42);
    let view = game.public_view(&state, state.to_act);
    let legal = game.legal_actions(&state, state.to_act);
    let err = agent.choose(&view, &legal, &state).await.unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("prompt_id"), "unexpected msg: {msg}");
}

#[tokio::test]
async fn out_of_range_action_index_is_rejected() {
    let dir = TempDir::new().unwrap();
    let script = write_py(&dir, "out_of_range.py", OUT_OF_RANGE_PY);
    let mut agent: StdioAgent<CribbageGame> =
        StdioAgent::new(0, "cribbage", cfg_py(&script)).unwrap();
    let game = CribbageGame::new();
    let state = discard_phase_state(42);
    let view = game.public_view(&state, state.to_act);
    let legal = game.legal_actions(&state, state.to_act);
    assert!(legal.len() < 999);
    let err = agent.choose(&view, &legal, &state).await.unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("999") && (msg.contains("range") || msg.contains("legal")),
        "unexpected msg: {msg}"
    );
}

#[tokio::test]
async fn too_many_garbage_lines_caps_at_16() {
    let dir = TempDir::new().unwrap();
    let script = write_py(&dir, "garbage.py", GARBAGE_THEN_VALID_PY);
    let mut agent: StdioAgent<CribbageGame> =
        StdioAgent::new(0, "cribbage", cfg_py(&script)).unwrap();
    let game = CribbageGame::new();
    let state = discard_phase_state(42);
    let view = game.public_view(&state, state.to_act);
    let legal = game.legal_actions(&state, state.to_act);
    let err = agent.choose(&view, &legal, &state).await.unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("16") || msg.to_lowercase().contains("garbage"),
        "unexpected msg: {msg}"
    );
}

#[tokio::test]
async fn error_frame_from_child_surfaces_as_child_error() {
    let dir = TempDir::new().unwrap();
    let script = write_py(&dir, "error_frame.py", ERROR_FRAME_PY);
    let mut agent: StdioAgent<CribbageGame> =
        StdioAgent::new(0, "cribbage", cfg_py(&script)).unwrap();
    let game = CribbageGame::new();
    let state = discard_phase_state(42);
    let view = game.public_view(&state, state.to_act);
    let legal = game.legal_actions(&state, state.to_act);
    let err = agent.choose(&view, &legal, &state).await.unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("unrecoverable") || msg.contains("child"),
        "unexpected msg: {msg}"
    );
}

#[tokio::test]
async fn a_few_garbage_lines_are_tolerated() {
    // The agent is explicitly human-friendly for debug prints — it
    // discards up to 16 non-JSON lines before erroring. Make sure the
    // happy path under that cap still works.
    let dir = TempDir::new().unwrap();
    let script = write_py(&dir, "two_garbage.py", TWO_GARBAGE_THEN_VALID_PY);
    let mut agent: StdioAgent<CribbageGame> =
        StdioAgent::new(0, "cribbage", cfg_py(&script)).unwrap();
    let game = CribbageGame::new();
    let state = discard_phase_state(42);
    let view = game.public_view(&state, state.to_act);
    let legal = game.legal_actions(&state, state.to_act);
    let idx = agent.choose(&view, &legal, &state).await.unwrap();
    assert_eq!(idx, 0);
    assert_eq!(agent.scratch().plan, "ok");
}
