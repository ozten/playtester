//! End-to-end smoke tests for `playtest report`.
//!
//! Pipeline coverage: `play` writes logs, `report` ingests them and
//! emits markdown. These tests also pin the load-bearing properties
//! of the Unit 15 output — R1.5 per-card signal, expected section
//! headings, empty-directory robustness.

use std::fs;

use assert_cmd::Command;
use tempfile::TempDir;

fn bin() -> Command {
    Command::cargo_bin("playtest").expect("bin `playtest` builds")
}

fn play_n(games_dir: &TempDir, games: u32, seed: u64) {
    bin()
        .args([
            "play",
            "--game",
            "cribbage",
            "--agents",
            "random,random",
            "--games",
            &games.to_string(),
            "--seed",
            &seed.to_string(),
            "--out",
            games_dir.path().to_str().unwrap(),
            "--fixed-time",
            "0",
        ])
        .assert()
        .success();
}

fn report_to(games_dir: &TempDir, out_path: &std::path::Path) {
    bin()
        .args([
            "report",
            "--game",
            "cribbage",
            "--games",
            games_dir.path().to_str().unwrap(),
            "--out",
            out_path.to_str().unwrap(),
        ])
        .assert()
        .success();
}

#[test]
fn report_after_play_contains_all_expected_sections() {
    let games = TempDir::new().unwrap();
    let out = TempDir::new().unwrap();
    let md_path = out.path().join("report.md");
    play_n(&games, 20, 7);
    report_to(&games, &md_path);

    let md = fs::read_to_string(&md_path).unwrap();
    for section in [
        "# Playtest report — cribbage",
        "## Summary",
        "### Winner distribution",
        "### End-reason breakdown",
        "## Per-agent",
        "## Cribbage: game shape",
        "### Phase of game end",
        "## Cribbage: scoring breakdown",
        "## Cribbage: per-card design insight (R1.5)",
    ] {
        assert!(
            md.contains(section),
            "expected section `{section}` missing; report was:\n{md}"
        );
    }
    // Per-rank table has one row per rank.
    for rank in [
        "A", "2", "3", "4", "5", "6", "7", "8", "9", "T", "J", "Q", "K",
    ] {
        assert!(
            md.contains(&format!("| {rank}    |")) || md.contains(&format!("| {rank} ")),
            "per-card row for rank {rank} missing; report was:\n{md}"
        );
    }
}

#[test]
fn report_on_empty_directory_produces_no_games_message_and_exits_zero() {
    let games = TempDir::new().unwrap();
    let out = TempDir::new().unwrap();
    let md_path = out.path().join("report.md");
    report_to(&games, &md_path);

    let md = fs::read_to_string(&md_path).unwrap();
    assert!(md.contains("## Summary"));
    assert!(
        md.contains("No games ingested"),
        "empty-dir report should say so; got:\n{md}"
    );
}

#[test]
fn report_preserves_games_count_across_repeated_runs_when_db_is_persistent() {
    let games = TempDir::new().unwrap();
    let out = TempDir::new().unwrap();
    let md_path = out.path().join("report.md");
    let db_path = out.path().join("report.sqlite");

    play_n(&games, 10, 99);

    // First report: ingests + reports.
    bin()
        .args([
            "report",
            "--game",
            "cribbage",
            "--games",
            games.path().to_str().unwrap(),
            "--out",
            md_path.to_str().unwrap(),
            "--db",
            db_path.to_str().unwrap(),
        ])
        .assert()
        .success();
    let md_first = fs::read_to_string(&md_path).unwrap();
    assert!(md_first.contains("Total games: **10**"));

    // Second report over the same directory with the same persistent
    // db: idempotent ingestion — no duplicate rows, report unchanged.
    bin()
        .args([
            "report",
            "--game",
            "cribbage",
            "--games",
            games.path().to_str().unwrap(),
            "--out",
            md_path.to_str().unwrap(),
            "--db",
            db_path.to_str().unwrap(),
        ])
        .assert()
        .success();
    let md_second = fs::read_to_string(&md_path).unwrap();
    assert!(md_second.contains("Total games: **10**"));
}

#[test]
fn per_card_kept_rates_show_rank_to_rank_asymmetry() {
    // R1.5: even with random agents, the *deal* distribution produces
    // visible per-rank asymmetry. 50 games is enough to see it; if
    // this test gets flaky on a tighter threshold we can bump the
    // sample.
    let games = TempDir::new().unwrap();
    let out = TempDir::new().unwrap();
    let md_path = out.path().join("report.md");
    play_n(&games, 50, 1);
    report_to(&games, &md_path);

    let md = fs::read_to_string(&md_path).unwrap();
    // Find the per-card section and parse kept-rates into a vec.
    let section = md
        .split("per-card design insight")
        .nth(1)
        .expect("per-card section present");
    let mut kept_rates = Vec::new();
    for line in section.lines() {
        let line = line.trim();
        // Skip header + separator + empty.
        if !line.starts_with('|') || line.contains("---") || line.contains("rank") {
            continue;
        }
        // Columns: | rank | kept | ... |
        let cells: Vec<&str> = line.split('|').map(str::trim).collect();
        if cells.len() < 3 {
            continue;
        }
        let kept_cell = cells[2];
        if let Some(percent_str) = kept_cell.strip_suffix('%')
            && let Ok(v) = percent_str.trim().parse::<f64>()
        {
            kept_rates.push(v);
        }
    }
    assert_eq!(
        kept_rates.len(),
        13,
        "expected 13 rank rows, parsed {kept_rates:?} from section:\n{section}"
    );
    let min = kept_rates.iter().copied().fold(f64::INFINITY, f64::min);
    let max = kept_rates.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    assert!(
        max - min >= 2.0,
        "expected meaningful rank-to-rank variance in kept rates; got min={min} max={max}"
    );
}

#[test]
fn report_writes_ingestion_summary_line_to_stdout() {
    let games = TempDir::new().unwrap();
    let out = TempDir::new().unwrap();
    let md_path = out.path().join("report.md");
    play_n(&games, 3, 5);
    let output = bin()
        .args([
            "report",
            "--game",
            "cribbage",
            "--games",
            games.path().to_str().unwrap(),
            "--out",
            md_path.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "report exited non-zero: {output:?}"
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("ingested 3 games"),
        "stdout summary missing count; got: {stdout}"
    );
    assert!(
        stdout.contains(&md_path.display().to_string()),
        "stdout should mention output path; got: {stdout}"
    );
}
