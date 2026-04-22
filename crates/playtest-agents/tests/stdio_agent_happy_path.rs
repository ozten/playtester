//! `StdioAgent` happy-path tests — subprocess spawning, frame shape,
//! scratch updates, full-game integration, and drop-reap lifecycle.
//!
//! Test subprocesses are written as Python scripts into a tempfile and
//! invoked via `python3`. CI on Linux has `python3` on PATH; this is
//! the same assumption every `tokio::process` test in the tree already
//! relies on.

use std::path::{Path, PathBuf};

use playtest_adapters::{ProductionRng, StubGameEventSink};
use playtest_agents::{StdioAgent, StdioAgentConfig};
use playtest_core::{Actor, Agent, EndReason, Game, GameLoop};
use playtest_cribbage::{CribbageConfig, CribbageGame};
use tempfile::TempDir;

/// Reach a state whose `to_act` player has many legal actions (so the
/// agent does not short-circuit on a single legal choice).
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

/// Write a Python script to a tempdir and return its path. The caller
/// is responsible for keeping the `TempDir` alive until the subprocess
/// has exited.
fn write_py_script(dir: &TempDir, name: &str, body: &str) -> PathBuf {
    let path = dir.path().join(name);
    std::fs::write(&path, body).expect("write python script");
    // `python3` doesn't require the script to be executable (it's an
    // argument to the interpreter), so we don't chmod here.
    path
}

/// Script: reads one JSON line per turn from stdin, replies with
/// `{"kind":"action","prompt_id":<id>,"action_index":0,"scratch":{"plan":"","notes":""}}`.
const ALWAYS_ZERO_PY: &str = r#"
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
        "scratch": {"plan": "p", "notes": "n"},
    }
    sys.stdout.write(json.dumps(reply) + "\n")
    sys.stdout.flush()
"#;

/// Script that echoes the first turn frame it sees to a debug file
/// (path passed via `sys.argv[1]`) and then replies with action 0.
const CAPTURE_FIRST_PY: &str = r#"
import sys, json
debug_path = sys.argv[1]
captured = False
for line in sys.stdin:
    line = line.strip()
    if not line:
        continue
    req = json.loads(line)
    if not captured:
        with open(debug_path, "w") as f:
            f.write(line)
        captured = True
    reply = {
        "kind": "action",
        "prompt_id": req["prompt_id"],
        "action_index": 0,
        "scratch": {"plan": "", "notes": ""},
    }
    sys.stdout.write(json.dumps(reply) + "\n")
    sys.stdout.flush()
"#;

/// Script that exits cleanly after the first reply.
const ONE_SHOT_PY: &str = r#"
import sys, json
line = sys.stdin.readline().strip()
req = json.loads(line)
reply = {
    "kind": "action",
    "prompt_id": req["prompt_id"],
    "action_index": 0,
    "scratch": {"plan": "", "notes": ""},
}
sys.stdout.write(json.dumps(reply) + "\n")
sys.stdout.flush()
sys.exit(0)
"#;

/// Script that ignores stdin and sleeps forever — used to test
/// drop-reaps-cleanly. Writes its PID to `sys.argv[1]` on startup.
const HANG_FOREVER_PY: &str = r#"
import sys, os, time
with open(sys.argv[1], "w") as f:
    f.write(str(os.getpid()))
time.sleep(3600)
"#;

fn cfg_py(script: &Path, extra_args: Vec<String>) -> StdioAgentConfig {
    let mut args = vec![script.to_string_lossy().to_string()];
    args.extend(extra_args);
    StdioAgentConfig {
        command: PathBuf::from("/usr/bin/python3"),
        args,
    }
}

// ---------------------------------------------------------------------
// Happy path tests
// ---------------------------------------------------------------------

#[tokio::test]
async fn spawns_lazily_and_returns_action_index() {
    let dir = TempDir::new().unwrap();
    let script = write_py_script(&dir, "always_zero.py", ALWAYS_ZERO_PY);

    let mut agent: StdioAgent<CribbageGame> =
        StdioAgent::new(0, "cribbage", cfg_py(&script, vec![])).unwrap();

    // New agent has not spawned the subprocess yet.
    assert!(!agent.is_spawned());

    let game = CribbageGame::new();
    let state = discard_phase_state(42);
    let view = game.public_view(&state, state.to_act);
    let legal = game.legal_actions(&state, state.to_act);
    assert!(legal.len() >= 2, "expected discard phase with many legals");

    let idx = agent.choose(&view, &legal, &state).await.unwrap();
    assert_eq!(idx, 0);
    assert!(agent.is_spawned(), "child must be spawned after first choose");
    assert_eq!(agent.scratch().plan, "p");
    assert_eq!(agent.scratch().notes, "n");
    assert_eq!(agent.scratch().turn_log.len(), 1);
    assert!(agent.scratch().turn_log[0].contains("stdio_chose index=0"));
}

#[tokio::test]
async fn three_turns_reuse_the_same_child() {
    let dir = TempDir::new().unwrap();
    let script = write_py_script(&dir, "always_zero.py", ALWAYS_ZERO_PY);
    let mut agent: StdioAgent<CribbageGame> =
        StdioAgent::new(0, "cribbage", cfg_py(&script, vec![])).unwrap();

    let game = CribbageGame::new();
    let state = discard_phase_state(42);
    let view = game.public_view(&state, state.to_act);
    let legal = game.legal_actions(&state, state.to_act);
    for _ in 0..3 {
        let idx = agent.choose(&view, &legal, &state).await.unwrap();
        assert_eq!(idx, 0);
    }
    // turn_log should have three entries; prompt_ids are 0, 1, 2.
    assert_eq!(agent.scratch().turn_log.len(), 3);
    assert!(agent.scratch().turn_log[0].contains("tick=0"));
    assert!(agent.scratch().turn_log[2].contains("tick=2"));
}

#[tokio::test]
async fn first_turn_frame_carries_api_version_game_seat_and_prompt_id() {
    let dir = TempDir::new().unwrap();
    let script = write_py_script(&dir, "capture_first.py", CAPTURE_FIRST_PY);
    let debug_path = dir.path().join("first_frame.json");

    let mut agent: StdioAgent<CribbageGame> = StdioAgent::new(
        3,
        "cribbage",
        cfg_py(&script, vec![debug_path.to_string_lossy().into_owned()]),
    )
    .unwrap();

    let game = CribbageGame::new();
    let state = discard_phase_state(42);
    let view = game.public_view(&state, state.to_act);
    let legal = game.legal_actions(&state, state.to_act);
    let _ = agent.choose(&view, &legal, &state).await.unwrap();

    // Give the child a moment to flush its file, then parse the
    // captured frame.
    for _ in 0..50 {
        if debug_path.exists() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    let contents = std::fs::read_to_string(&debug_path).expect("frame captured");
    let v: serde_json::Value = serde_json::from_str(&contents).expect("valid JSON");
    assert_eq!(v["kind"], "turn");
    assert_eq!(v["api_version"], "3.0.0");
    assert_eq!(v["game"], "cribbage");
    assert_eq!(v["seat"], 3);
    assert_eq!(v["prompt_id"], 0);
    assert!(v["view"].is_object());
    assert!(v["legal_actions"].is_array());
    assert!(v["scratch"].is_object(), "scratch key present");
    assert!(v["scratch"]["plan"].is_string());
    assert!(v["scratch"]["notes"].is_string());
    assert!(v["scratch"]["turn_log"].is_array());
}

#[tokio::test]
async fn full_cribbage_game_with_stdio_agents_terminates() {
    // Integration: two StdioAgents drive a Cribbage game end-to-end
    // via a python subprocess that always picks index 0.
    let dir = TempDir::new().unwrap();
    let script = write_py_script(&dir, "always_zero.py", ALWAYS_ZERO_PY);

    let game = CribbageGame::new();
    let mut agents: Vec<Box<dyn Agent<CribbageGame>>> = vec![
        Box::new(
            StdioAgent::<CribbageGame>::new(0, "cribbage", cfg_py(&script, vec![])).unwrap(),
        ),
        Box::new(
            StdioAgent::<CribbageGame>::new(1, "cribbage", cfg_py(&script, vec![])).unwrap(),
        ),
    ];

    let mut loop_ = GameLoop::new(&game, game.initial_state(42, &CribbageConfig));
    let mut chance_rng = ProductionRng::from_seed(42);
    let mut sink = StubGameEventSink::new();

    let result = loop_
        .run(agents.as_mut_slice(), &mut chance_rng, &mut sink)
        .await
        .unwrap_or_else(|e| panic!("StdioAgent game loop error: {e}"));

    assert_eq!(result.reason, EndReason::Victory);
    assert!(result.winner.is_some());
}

// ---------------------------------------------------------------------
// Security: env scrubbing
// ---------------------------------------------------------------------
//
// Fully exercising env scrubbing would require mutating the parent's
// environment before spawn — but `std::env::set_var` is `unsafe` and
// `unsafe_code = forbid` at workspace scope. Instead we exploit
// `std::process::Command::env` on a probe command we run *outside*
// the agent to confirm the baseline (child inherits parent env), then
// run the agent's spawn with the same env present in the parent-ish
// context using a shell wrapper as the agent's target. The wrapper
// unconditionally sets the keys in its own environment *before*
// execing python, so the only way python sees MISSING is if
// `env_remove` on the agent's Command actually stripped them before
// fork + exec. That cannot happen: `env_remove` affects only the
// *inherited* env, not the wrapper's own `export`. So this test
// actually proves a different property: that env_remove does NOT
// affect vars set downstream, which is the correct semantics.
//
// The direct test — "set in parent, verify absent in child" — is
// skipped deliberately because it would require `unsafe { set_var }`.

const ENV_PROBE_PY: &str = r#"
import sys, json, os
debug_path = sys.argv[1]
with open(debug_path, "w") as f:
    f.write(os.environ.get("ANTHROPIC_API_KEY", "MISSING"))
    f.write("|")
    f.write(os.environ.get("PLAYTEST_OPENAI_COMPAT_KEY", "MISSING"))
line = sys.stdin.readline().strip()
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

#[tokio::test]
async fn agent_spawned_child_does_not_have_scrubbed_keys_in_its_env() {
    // Weaker claim than the full "set-in-parent, missing-in-child"
    // test (which would need `unsafe { set_var }`). This confirms:
    // if the test-binary's env does not carry the scrubbed keys, the
    // agent's child does not acquire them on its own. That still
    // exercises the `env_remove` call path — it's a no-op here but
    // present — and anchors future breakage against a known-good
    // baseline.
    let dir = TempDir::new().unwrap();
    let script = write_py_script(&dir, "env_probe.py", ENV_PROBE_PY);
    let debug_path = dir.path().join("env.txt");

    let mut agent: StdioAgent<CribbageGame> = StdioAgent::new(
        0,
        "cribbage",
        cfg_py(&script, vec![debug_path.to_string_lossy().into_owned()]),
    )
    .unwrap();

    let game = CribbageGame::new();
    let state = discard_phase_state(42);
    let view = game.public_view(&state, state.to_act);
    let legal = game.legal_actions(&state, state.to_act);
    let _ = agent.choose(&view, &legal, &state).await.unwrap();

    for _ in 0..50 {
        if debug_path.exists() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    let captured = std::fs::read_to_string(&debug_path).expect("env captured");
    assert_eq!(
        captured, "MISSING|MISSING",
        "child must not see LLM credentials: got {captured:?}"
    );
}

// ---------------------------------------------------------------------
// Lifecycle: drop reaps
// ---------------------------------------------------------------------

#[tokio::test]
async fn drop_reaps_a_hanging_child() {
    let dir = TempDir::new().unwrap();
    let script = write_py_script(&dir, "hang.py", HANG_FOREVER_PY);
    let pid_path = dir.path().join("pid.txt");

    {
        let mut agent: StdioAgent<CribbageGame> = StdioAgent::new(
            0,
            "cribbage",
            cfg_py(&script, vec![pid_path.to_string_lossy().into_owned()]),
        )
        .unwrap();
        // Force the spawn; the child never replies, so we cannot call
        // `choose`. Instead drive the lazy spawn directly via a tiny
        // helper — the legal-slice short-circuit masks real `choose`
        // semantics here, so we instead spawn via an internal hook:
        // call `choose` with a vector whose len > 1, but drop the
        // future before it ever completes.
        let game = CribbageGame::new();
        let state = discard_phase_state(1);
        let view = game.public_view(&state, state.to_act);
        let legal = game.legal_actions(&state, state.to_act);
        // Poll the future once (so `spawn_lazy` runs) then drop it.
        // tokio::select! with a 500ms timeout lets the spawn + write
        // complete but the read_line hang gets interrupted.
        let fut = agent.choose(&view, &legal, &state);
        tokio::select! {
            _ = fut => panic!("hang.py unexpectedly replied"),
            () = tokio::time::sleep(std::time::Duration::from_millis(500)) => {}
        }
        // Give the script a beat to write its pid file.
        for _ in 0..50 {
            if pid_path.exists() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        assert!(pid_path.exists(), "python subprocess never wrote its pid");
        // Agent drops here → stdin closes, kill_on_drop fires.
    }

    let pid: i32 = std::fs::read_to_string(&pid_path)
        .unwrap()
        .trim()
        .parse()
        .expect("pid int");

    // Poll `/proc/<pid>` — Linux CI. Give the reaper up to 5s.
    let proc_path = format!("/proc/{pid}");
    let mut reaped = false;
    for _ in 0..50 {
        if !std::path::Path::new(&proc_path).exists() {
            reaped = true;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    assert!(reaped, "child pid {pid} was not reaped within 5s");
}

// ---------------------------------------------------------------------
// Error plumbing: CommandNotFound at build time
// ---------------------------------------------------------------------

#[test]
fn new_rejects_missing_command_path() {
    let err = StdioAgent::<CribbageGame>::new(
        0,
        "cribbage",
        StdioAgentConfig {
            command: PathBuf::from("/no/such/binary/xyz_playtest_sentinel"),
            args: vec![],
        },
    )
    .unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("not found"), "unexpected msg: {msg}");
    assert!(
        msg.contains("xyz_playtest_sentinel"),
        "should include the missing path: {msg}"
    );
}

// ---------------------------------------------------------------------
// Lifecycle: child-exit detection on next turn
// ---------------------------------------------------------------------

#[tokio::test]
async fn child_exits_after_first_reply_then_next_choose_errors() {
    let dir = TempDir::new().unwrap();
    let script = write_py_script(&dir, "one_shot.py", ONE_SHOT_PY);
    let mut agent: StdioAgent<CribbageGame> =
        StdioAgent::new(0, "cribbage", cfg_py(&script, vec![])).unwrap();

    let game = CribbageGame::new();
    let state = discard_phase_state(42);
    let view = game.public_view(&state, state.to_act);
    let legal = game.legal_actions(&state, state.to_act);

    // First turn succeeds.
    let idx = agent.choose(&view, &legal, &state).await.unwrap();
    assert_eq!(idx, 0);

    // Give the child a beat to exit.
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;

    // Second turn: the child is gone, so write or read will fail.
    let err = agent.choose(&view, &legal, &state).await.unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.to_lowercase().contains("child") && msg.to_lowercase().contains("exit"),
        "expected child-exited message, got: {msg}"
    );
}
