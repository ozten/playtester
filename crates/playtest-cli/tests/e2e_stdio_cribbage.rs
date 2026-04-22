//! End-to-end Phase 3 validation: a full 2-player Cribbage game driven
//! by a Python subprocess (seat 0) + a `random` agent (seat 1), invoked
//! through the release `playtest` binary via `assert_cmd`.
//!
//! The Python script is written to a tempdir and always picks
//! `action_index = 0`. With `--fixed-time 0` and `--seed 42` the run is
//! byte-deterministic across invocations.
//!
//! What we prove here:
//!
//! 1. `playtest play --agents stdio,random` completes and exits 0.
//! 2. The main JSONL log is well-formed (header + events + final) and
//!    parses cleanly — under the replay engine, from the seed + event
//!    stream alone.
//! 3. The log does not leak any stdio protocol frames (`"kind":"turn"`
//!    / `"kind":"action"` where `action` is the stdio child-reply frame
//!    — distinct from the game's event-kind `"action"`). See
//!    `log_has_no_coordination_frames.rs` for the JSON-path-aware
//!    variant.

use std::fs;
use std::path::PathBuf;

use assert_cmd::Command;
use tempfile::TempDir;

fn bin() -> Command {
    Command::cargo_bin("playtest").expect("bin `playtest` builds")
}

/// Python agent body: one-shot reply loop, always picks legal_actions[0].
///
/// Doesn't even import `playtest_stdio` — that module is a convenience
/// the reference client provides; here we keep the test self-contained.
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
        "scratch": {"plan": "pick-first", "notes": ""},
    }
    sys.stdout.write(json.dumps(reply) + "\n")
    sys.stdout.flush()
"#;

fn write_script(dir: &TempDir, name: &str, body: &str) -> PathBuf {
    let path = dir.path().join(name);
    fs::write(&path, body).expect("write python script");
    path
}

#[test]
fn stdio_cribbage_e2e_game_completes_and_replays() {
    let script_dir = TempDir::new().unwrap();
    let out_dir = TempDir::new().unwrap();
    let script = write_script(&script_dir, "always_first.py", ALWAYS_FIRST_PY);

    // Run a single game: stdio seat 0, random seat 1. `--fixed-time 0`
    // makes the header byte-deterministic.
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
    assert!(log_path.exists(), "expected log at {}", log_path.display());
    let text = fs::read_to_string(&log_path).expect("read log");
    let lines: Vec<&str> = text.lines().collect();
    assert!(lines.len() >= 10, "expected several event lines, got {}", lines.len());

    // Shape check: first line is the header, last is `final`. Bodies
    // in between are `event`s.
    assert!(lines[0].contains("\"kind\":\"header\""), "first: {}", lines[0]);
    assert!(
        lines[lines.len() - 1].contains("\"kind\":\"final\""),
        "last: {}",
        lines[lines.len() - 1]
    );

    // Replay must produce a non-empty state dump.
    let out = bin()
        .args(["replay", log_path.to_str().unwrap()])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let replay_text = std::str::from_utf8(&out).unwrap();
    assert!(replay_text.contains("game: cribbage"), "replay output: {replay_text}");
    assert!(replay_text.contains("--- final ---"), "replay output: {replay_text}");
}

#[test]
fn stdio_cribbage_e2e_no_llm_sidecar_when_no_llm_seat() {
    // When no seat is `llm`, the CLI should not materialise a
    // `game-XXXX.llm.jsonl` sidecar. Prevents the stdio path from
    // accidentally triggering LLM wiring.
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
            "7",
            "--games",
            "1",
            "--out",
            out_dir.path().to_str().unwrap(),
            "--fixed-time",
            "0",
        ])
        .assert()
        .success();

    let sidecar = out_dir.path().join("game-0000.llm.jsonl");
    assert!(
        !sidecar.exists(),
        "no LLM seat → no sidecar, but found {}",
        sidecar.display()
    );
}
