//! End-to-end smoke for `playtest matchup`.
//!
//! Runs the 100-games-per-pair matchup the spec calls for on a small
//! cribbage pool (random, heuristic-cribbage) and verifies the matrix
//! shape plus that heuristic-cribbage clearly dominates random.

use std::fs;

use assert_cmd::Command;
use predicates::prelude::*;
use tempfile::TempDir;

fn bin() -> Command {
    Command::cargo_bin("playtest").expect("bin `playtest` builds")
}

#[test]
fn matchup_cribbage_random_vs_heuristic_100_per_pair() {
    let dir = TempDir::new().unwrap();
    let out = dir.path().join("matrix.md");

    bin()
        .args([
            "matchup",
            "--game",
            "cribbage",
            "--agents",
            "random,heuristic-cribbage",
            "--games-per-pair",
            "100",
            "--seed",
            "7",
            "--out",
            out.to_string_lossy().as_ref(),
        ])
        .assert()
        .success();

    let md = fs::read_to_string(&out).expect("matrix.md written");
    // Shape: header, both agents in header row and leading column.
    assert!(md.contains("# Matchup matrix — cribbage"), "md: {md}");
    assert!(md.contains("random"), "missing random row/col: {md}");
    assert!(md.contains("heuristic-cribbage"), "missing heuristic row/col: {md}");
    // Spot-check the dominance signal: heuristic row vs random column must
    // be well above 50%. The row is the line starting with "| heuristic-cribbage".
    let hrow = md
        .lines()
        .find(|l| l.starts_with("| heuristic-cribbage |"))
        .expect("heuristic row");
    // Extract percentages in the order: random, heuristic-cribbage.
    let cells: Vec<&str> = hrow.split('|').map(str::trim).collect();
    // cells[0] is "" (leading pipe), cells[1] is agent name, cells[2] is vs random.
    let vs_random = cells[2].trim_end_matches('%').trim_end_matches(')').split_whitespace().next().unwrap();
    let pct: f64 = vs_random.trim_end_matches('%').parse().expect("percentage parses");
    assert!(
        pct >= 80.0,
        "heuristic-cribbage as seat 0 vs random should crush (>= 80%), got {pct}% — row: {hrow}"
    );
}

#[test]
fn matchup_unknown_agent_fails() {
    let dir = TempDir::new().unwrap();
    let out = dir.path().join("matrix.md");
    bin()
        .args([
            "matchup",
            "--game",
            "cribbage",
            "--agents",
            "random,alpha-zero",
            "--games-per-pair",
            "10",
            "--out",
            out.to_string_lossy().as_ref(),
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("unknown agent"));
    assert!(!out.exists(), "matrix.md should not be written on validation failure");
}

#[test]
fn matchup_needs_at_least_two_agents() {
    let dir = TempDir::new().unwrap();
    let out = dir.path().join("matrix.md");
    bin()
        .args([
            "matchup",
            "--game",
            "cribbage",
            "--agents",
            "random",
            "--games-per-pair",
            "10",
            "--out",
            out.to_string_lossy().as_ref(),
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("at least 2"));
}

#[test]
fn matchup_zero_games_per_pair_errors() {
    let dir = TempDir::new().unwrap();
    let out = dir.path().join("matrix.md");
    bin()
        .args([
            "matchup",
            "--game",
            "cribbage",
            "--agents",
            "random,heuristic-cribbage",
            "--games-per-pair",
            "0",
            "--out",
            out.to_string_lossy().as_ref(),
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("must be positive"));
}
