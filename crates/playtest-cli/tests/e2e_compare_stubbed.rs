//! End-to-end Phase 6 validation: two hand-written log directories
//! fed through the same code path `playtest compare` runs.
//!
//! The CLI crate is bin-only, so this test invokes the library-level
//! functions (`ingest_directory` into in-memory SQLite, `run_compare`,
//! `write_compare_report`) that `commands/compare.rs` orchestrates.
//! The CLI wrapper is a thin dispatch layer over these calls.

use std::fs;
use std::path::{Path, PathBuf};

use playtest_core::EndReason;
use playtest_cribbage::{CribbageGame, CribbageMetrics, Event as CribbageEvent};
use playtest_log::{LogHeader, LogRecord, SCHEMA_VERSION};
use playtest_metrics::{
    CompareOpts, Correction, MarkdownBuilder, ingest_directory, init_schema, run_compare,
    write_compare_report,
};
use rusqlite::Connection;
use tempfile::TempDir;

fn write_log(dir: &Path, id: &str, seed: u64, event_count: usize, winner: Option<u8>) -> PathBuf {
    let path = dir.join(format!("game-{id}.jsonl"));
    let mut lines: Vec<String> = Vec::new();
    let header = LogHeader {
        schema: SCHEMA_VERSION,
        game: "cribbage".into(),
        version: "0.0.0".into(),
        seed,
        agents: vec!["random".into(), "random".into()],
        started_at: 0,
        config_hash: "0".repeat(64),
    };
    lines.push(serde_json::to_string(&LogRecord::<CribbageEvent>::Header(header)).unwrap());
    for tick in 0..event_count {
        let event = CribbageEvent::DealCard {
            player: 0,
            card: playtest_cribbage::Card::new(
                playtest_cribbage::Rank::Ace,
                playtest_cribbage::Suit::Spades,
            ),
        };
        lines.push(
            serde_json::to_string(&LogRecord::Event {
                tick: tick as u64,
                payload: event,
            })
            .unwrap(),
        );
    }
    lines.push(
        serde_json::to_string(&LogRecord::<CribbageEvent>::Final {
            winner,
            reason: EndReason::Victory,
            scores: vec![121, 95],
            finished_at: 100,
        })
        .unwrap(),
    );
    fs::write(&path, lines.join("\n") + "\n").unwrap();
    path
}

/// Runs the same compare pipeline the CLI's `compare::run` does.
/// Returns the rendered markdown string.
fn run_compare_e2e(
    baseline_dir: &Path,
    variant_dir: &Path,
    opts: CompareOpts,
) -> String {
    let mut baseline_conn = Connection::open_in_memory().unwrap();
    let mut variant_conn = Connection::open_in_memory().unwrap();
    init_schema(&baseline_conn).unwrap();
    init_schema(&variant_conn).unwrap();
    ingest_directory::<CribbageGame, _>(
        &mut baseline_conn,
        baseline_dir,
        CribbageGame::NAME,
        &CribbageMetrics,
    )
    .unwrap();
    ingest_directory::<CribbageGame, _>(
        &mut variant_conn,
        variant_dir,
        CribbageGame::NAME,
        &CribbageMetrics,
    )
    .unwrap();
    let result = run_compare(&baseline_conn, &variant_conn, &opts).unwrap();
    let mut md = MarkdownBuilder::new();
    write_compare_report(&mut md, &result);
    md.into_string()
}

// -----------------------------------------------------------------

#[test]
fn two_log_dirs_produce_a_compare_markdown_with_expected_headings() {
    let root = TempDir::new().unwrap();
    let baseline_dir = root.path().join("baseline");
    let variant_dir = root.path().join("variant");
    fs::create_dir_all(&baseline_dir).unwrap();
    fs::create_dir_all(&variant_dir).unwrap();
    for i in 0..5_u8 {
        write_log(&baseline_dir, &format!("b{i}"), u64::from(i), 20, Some(0));
        write_log(&variant_dir, &format!("v{i}"), u64::from(i) + 100, 50, Some(0));
    }

    let markdown = run_compare_e2e(&baseline_dir, &variant_dir, CompareOpts::default());
    assert!(markdown.contains("## Compare"));
    assert!(markdown.contains("baseline = **5**"));
    assert!(markdown.contains("variant = **5**"));
    assert!(markdown.contains("**BH**"));
}

#[test]
fn bonferroni_correction_renders_in_header() {
    let root = TempDir::new().unwrap();
    let baseline_dir = root.path().join("baseline");
    let variant_dir = root.path().join("variant");
    fs::create_dir_all(&baseline_dir).unwrap();
    fs::create_dir_all(&variant_dir).unwrap();
    for i in 0..5_u8 {
        write_log(&baseline_dir, &format!("b{i}"), u64::from(i), 20, Some(0));
        write_log(&variant_dir, &format!("v{i}"), u64::from(i) + 100, 20, Some(0));
    }
    let opts = CompareOpts {
        alpha: 0.01,
        correction: Correction::Bonferroni,
    };
    let markdown = run_compare_e2e(&baseline_dir, &variant_dir, opts);
    assert!(markdown.contains("**Bonferroni**"));
    assert!(markdown.contains("α = **0.01**"));
}

#[test]
fn empty_log_dirs_produce_report_with_no_data_note() {
    let root = TempDir::new().unwrap();
    let baseline_dir = root.path().join("baseline");
    let variant_dir = root.path().join("variant");
    fs::create_dir_all(&baseline_dir).unwrap();
    fs::create_dir_all(&variant_dir).unwrap();
    let markdown = run_compare_e2e(&baseline_dir, &variant_dir, CompareOpts::default());
    assert!(markdown.contains("## Compare"));
    assert!(markdown.contains("*No comparable data.*"));
}
