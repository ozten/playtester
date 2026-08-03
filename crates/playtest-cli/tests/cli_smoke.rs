//! End-to-end smoke tests for the `playtest` binary via `assert_cmd`.

use std::fs;

use assert_cmd::Command;
use predicates::prelude::*;
use tempfile::TempDir;

fn bin() -> Command {
    Command::cargo_bin("playtest").expect("bin `playtest` builds")
}

/// Core args for `play --game cribbage` with a fixed clock so runs
/// are byte-identical across invocations.
fn play_args(dir: &TempDir, games: u32, seed: u64) -> Vec<String> {
    vec![
        "play".into(),
        "--game".into(),
        "cribbage".into(),
        "--agents".into(),
        "random,random".into(),
        "--games".into(),
        games.to_string(),
        "--seed".into(),
        seed.to_string(),
        "--out".into(),
        dir.path().to_string_lossy().into_owned(),
        "--fixed-time".into(),
        "0".into(),
    ]
}

#[test]
fn play_produces_the_requested_number_of_jsonl_files() {
    let dir = TempDir::new().unwrap();
    bin().args(play_args(&dir, 5, 42)).assert().success();

    let count = fs::read_dir(dir.path()).unwrap().count();
    assert_eq!(count, 5, "expected 5 game files");
    for i in 0..5u32 {
        let path = dir.path().join(format!("game-{i:04}.jsonl"));
        assert!(path.exists(), "missing {}", path.display());
    }
}

#[test]
fn zero_games_exits_zero_with_no_files_written() {
    let dir = TempDir::new().unwrap();
    bin().args(play_args(&dir, 0, 1)).assert().success();
    assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 0);
}

#[test]
fn unknown_game_exits_nonzero_with_clear_message() {
    let dir = TempDir::new().unwrap();
    let mut args = play_args(&dir, 1, 1);
    // Replace --game value.
    let pos = args.iter().position(|a| a == "--game").unwrap();
    args[pos + 1] = "not-a-real-game".into();
    bin()
        .args(args)
        .assert()
        .failure()
        .stderr(predicate::str::contains("unknown game").and(predicate::str::contains("cribbage")));
}

#[test]
fn unknown_agent_exits_nonzero() {
    let dir = TempDir::new().unwrap();
    let mut args = play_args(&dir, 1, 1);
    let pos = args.iter().position(|a| a == "--agents").unwrap();
    args[pos + 1] = "random,alpha-zero".into();
    bin()
        .args(args)
        .assert()
        .failure()
        .stderr(predicate::str::contains("unknown agent").and(predicate::str::contains("random")));
}

#[test]
fn same_seed_produces_byte_identical_output_across_two_runs() {
    let dir_a = TempDir::new().unwrap();
    let dir_b = TempDir::new().unwrap();

    bin().args(play_args(&dir_a, 3, 100)).assert().success();
    bin().args(play_args(&dir_b, 3, 100)).assert().success();

    for i in 0..3u32 {
        let a = fs::read(dir_a.path().join(format!("game-{i:04}.jsonl"))).unwrap();
        let b = fs::read(dir_b.path().join(format!("game-{i:04}.jsonl"))).unwrap();
        assert_eq!(a, b, "game-{i} differs across runs with same seed");
    }
}

#[test]
fn parallel_flag_produces_same_output_as_serial() {
    // Each game is independently seeded by `seed + idx`, so rayon's
    // non-determinism across *scheduling* can't affect per-file
    // contents. This locks in that property.
    let serial = TempDir::new().unwrap();
    let parallel = TempDir::new().unwrap();

    bin().args(play_args(&serial, 4, 7)).assert().success();

    let mut par_args = play_args(&parallel, 4, 7);
    par_args.push("--parallel".into());
    bin().args(par_args).assert().success();

    for i in 0..4u32 {
        let s = fs::read(serial.path().join(format!("game-{i:04}.jsonl"))).unwrap();
        let p = fs::read(parallel.path().join(format!("game-{i:04}.jsonl"))).unwrap();
        assert_eq!(s, p, "game-{i} differs between serial and parallel");
    }
}

#[test]
fn replay_prints_a_nonempty_state_dump_for_a_recorded_game() {
    let dir = TempDir::new().unwrap();
    bin().args(play_args(&dir, 1, 13)).assert().success();
    let log = dir.path().join("game-0000.jsonl");

    let out = bin()
        .args(["replay", log.to_string_lossy().as_ref()])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    assert!(!out.is_empty(), "replay produced no stdout");
    let text = std::str::from_utf8(&out).unwrap();
    assert!(text.contains("game: cribbage"), "output: {text}");
    assert!(text.contains("tick 1"), "output: {text}");
    assert!(text.contains("--- final ---"), "output: {text}");
}

#[test]
fn replay_with_tick_filter_prints_only_that_state() {
    let dir = TempDir::new().unwrap();
    bin().args(play_args(&dir, 1, 21)).assert().success();
    let log = dir.path().join("game-0000.jsonl");

    let out = bin()
        .args(["replay", log.to_string_lossy().as_ref(), "--tick", "3"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let text = std::str::from_utf8(&out).unwrap();
    assert!(text.contains("state at tick 3"), "output: {text}");
    assert!(
        !text.contains("--- tick 1 ---"),
        "tick 1 should not appear: {text}"
    );
}

#[test]
fn replay_with_out_of_range_tick_errors() {
    let dir = TempDir::new().unwrap();
    bin().args(play_args(&dir, 1, 33)).assert().success();
    let log = dir.path().join("game-0000.jsonl");

    bin()
        .args(["replay", log.to_string_lossy().as_ref(), "--tick", "999999"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("out of range"));
}

/// Core args for `play --game greatgyre` with a fixed clock, 4 random
/// agents.
fn play_greatgyre_args(dir: &TempDir, games: u32, seed: u64) -> Vec<String> {
    vec![
        "play".into(),
        "--game".into(),
        "greatgyre".into(),
        "--agents".into(),
        "random,random,random,random".into(),
        "--games".into(),
        games.to_string(),
        "--seed".into(),
        seed.to_string(),
        "--out".into(),
        dir.path().to_string_lossy().into_owned(),
        "--fixed-time".into(),
        "0".into(),
    ]
}

#[test]
fn replay_public_view_dump_has_one_line_per_event_and_parses() {
    let dir = TempDir::new().unwrap();
    bin()
        .args(play_greatgyre_args(&dir, 1, 77))
        .assert()
        .success();
    let log = dir.path().join("game-0000.jsonl");
    let out_dir = TempDir::new().unwrap();
    let view_out = out_dir.path().join("views.jsonl");

    bin()
        .args([
            "replay",
            log.to_string_lossy().as_ref(),
            "--public-view-observer",
            "0",
            "--public-view-out",
            view_out.to_string_lossy().as_ref(),
        ])
        .assert()
        .success();

    // Count `kind: event` lines in the source log.
    let log_text = fs::read_to_string(&log).unwrap();
    let event_count = log_text
        .lines()
        .filter(|l| {
            let v: serde_json::Value = serde_json::from_str(l).unwrap();
            v.get("kind").and_then(|k| k.as_str()) == Some("event")
        })
        .count();
    assert!(event_count > 0, "expected at least one event in the log");

    let view_text = fs::read_to_string(&view_out).unwrap();
    let view_lines: Vec<&str> = view_text.lines().collect();
    assert_eq!(
        view_lines.len(),
        event_count,
        "expected one view line per event"
    );

    for (i, line) in view_lines.iter().enumerate() {
        let v: serde_json::Value = serde_json::from_str(line)
            .unwrap_or_else(|e| panic!("view line {i} failed to parse: {e}\nline: {line}"));
        let tick = v.get("tick").and_then(serde_json::Value::as_u64).unwrap();
        assert_eq!(tick, (i as u64) + 1, "tick should be 1-based and in order");
        let view = v.get("view").expect("view line missing `view` field");
        assert_eq!(
            view.get("observer").and_then(serde_json::Value::as_u64),
            Some(0),
            "view should be for observer seat 0"
        );
        assert!(
            view.get("own").is_some(),
            "greatgyre view missing `own`: {view}"
        );
    }
}

#[test]
fn replay_public_view_flags_must_be_passed_together() {
    let dir = TempDir::new().unwrap();
    bin()
        .args(play_greatgyre_args(&dir, 1, 55))
        .assert()
        .success();
    let log = dir.path().join("game-0000.jsonl");

    bin()
        .args([
            "replay",
            log.to_string_lossy().as_ref(),
            "--public-view-observer",
            "0",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("must be passed together"));

    bin()
        .args([
            "replay",
            log.to_string_lossy().as_ref(),
            "--public-view-out",
            "/tmp/wont-be-written.jsonl",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("must be passed together"));
}

#[test]
fn help_text_is_readable() {
    let out = bin()
        .arg("--help")
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let text = std::str::from_utf8(&out).unwrap();
    assert!(text.contains("play"), "help missing play: {text}");
    assert!(text.contains("replay"), "help missing replay: {text}");

    let out = bin()
        .args(["play", "--help"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let text = std::str::from_utf8(&out).unwrap();
    assert!(text.contains("--game"));
    assert!(text.contains("--agents"));
    assert!(text.contains("--games"));
    assert!(text.contains("--seed"));
    assert!(text.contains("--out"));
    assert!(text.contains("--parallel"));
}
