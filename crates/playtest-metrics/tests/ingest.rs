//! Integration tests for [`ingest_directory`].
//!
//! Uses a synthetic `TestGame` rather than depending on a game crate —
//! the metrics crate must not depend on any specific game (see Unit
//! 14's verification note). The test writes JSONL logs to a tempdir
//! by hand, then ingests them.

use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};

use playtest_core::{Actor, EndReason, Game, GameError, GameResult, PlayerId};
use playtest_log::{LogHeader, LogRecord, SCHEMA_VERSION};
use playtest_metrics::{
    BuiltInMetrics, MetricDef, MetricRegistry, MetricValue, MetricValueKind, ingest_directory,
    query,
};
use playtest_ports::Rng;
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use tempfile::tempdir;
use uuid::Uuid;

// ---------- Synthetic game -----------------------------------------------

#[derive(Debug)]
struct TestGame;

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
struct Noop;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct TestEvent {
    tag: String,
}

impl Game for TestGame {
    type State = ();
    type Action = Noop;
    type Event = TestEvent;
    type PublicView = ();
    type Config = ();

    fn initial_state(&self, _seed: u64, (): &()) {}
    fn next_actor(&self, (): &()) -> Actor {
        Actor::Player(0)
    }
    fn legal_actions(&self, (): &(), _p: PlayerId) -> Vec<Noop> {
        Vec::new()
    }
    fn apply_action(&self, (): &(), _p: PlayerId, _a: &Noop) -> Result<Vec<TestEvent>, GameError> {
        unreachable!()
    }
    fn resolve_chance(&self, (): &(), _rng: &mut dyn Rng) -> Result<TestEvent, GameError> {
        unreachable!()
    }
    fn apply_event(&self, (): &mut (), _e: &TestEvent) {}
    fn public_view(&self, (): &(), _p: PlayerId) {}
    fn determinize(&self, (): &(), _observer: PlayerId, _rng: &mut dyn Rng) {}
    fn game_over(&self, (): &()) -> Option<GameResult> {
        None
    }
}

// ---------- Noop registry (BuiltInMetrics-only mode) ---------------------

#[derive(Default)]
struct NoopRegistry;

impl MetricRegistry<TestGame> for NoopRegistry {
    fn metric_definitions(&self) -> Vec<MetricDef> {
        Vec::new()
    }
    fn extract(
        &self,
        _game_id: Uuid,
        _log: &playtest_metrics::GameLog<TestGame>,
    ) -> Vec<MetricValue> {
        Vec::new()
    }
}

// ---------- A registry that emits a tagged metric ------------------------

#[derive(Default)]
struct TaggedRegistry;

const TAGGED_METRIC: &str = "event_count_by_tag";

impl MetricRegistry<TestGame> for TaggedRegistry {
    fn metric_definitions(&self) -> Vec<MetricDef> {
        vec![MetricDef {
            name: TAGGED_METRIC.into(),
            kind: playtest_metrics::MetricKind::Count,
            scope: playtest_metrics::MetricScope::Game,
            description: "Synthetic per-tag count for ingestion tests.".into(),
        }]
    }

    fn extract(
        &self,
        game_id: Uuid,
        log: &playtest_metrics::GameLog<TestGame>,
    ) -> Vec<MetricValue> {
        use std::collections::BTreeMap;
        let mut counts: BTreeMap<String, i64> = BTreeMap::new();
        for ev in &log.events {
            *counts.entry(ev.tag.clone()).or_default() += 1;
        }
        counts
            .into_iter()
            .map(|(tag, n)| MetricValue {
                game_id,
                metric_name: TAGGED_METRIC.into(),
                player: None,
                tag: Some(tag),
                value: MetricValueKind::Count(n),
            })
            .collect()
    }
}

// ---------- Log-file helpers --------------------------------------------

fn header(seed: u64, started_at: u64, agents: &[&str]) -> LogHeader {
    LogHeader {
        schema: SCHEMA_VERSION,
        game: "testgame".into(),
        version: "0.0.0".into(),
        seed,
        agents: agents.iter().map(|&s| s.into()).collect(),
        started_at,
        config_hash: "0".repeat(64),
    }
}

fn write_log(
    dir: &Path,
    name: &str,
    header: &LogHeader,
    events: &[TestEvent],
    result: Option<GameResult>,
    finished_at: u64,
) -> PathBuf {
    let path = dir.join(name);
    let file = File::create(&path).unwrap();
    let mut w = BufWriter::new(file);
    let hdr_line = serde_json::to_string(&LogRecord::<TestEvent>::Header(header.clone())).unwrap();
    writeln!(w, "{hdr_line}").unwrap();
    for (tick, payload) in events.iter().enumerate() {
        let line = serde_json::to_string(&LogRecord::Event {
            tick: tick as u64,
            payload: payload.clone(),
        })
        .unwrap();
        writeln!(w, "{line}").unwrap();
    }
    if let Some(r) = result {
        let fin: LogRecord<TestEvent> = LogRecord::Final {
            winner: r.winner,
            reason: r.reason,
            scores: r.scores,
            finished_at,
        };
        writeln!(w, "{}", serde_json::to_string(&fin).unwrap()).unwrap();
    }
    w.flush().unwrap();
    path
}

fn event(tag: &str) -> TestEvent {
    TestEvent { tag: tag.into() }
}

// ---------- Scenarios ----------------------------------------------------

#[test]
fn ingests_ten_files_and_fills_games_table() {
    let dir = tempdir().unwrap();
    for i in 0..10u64 {
        let h = header(i, i * 1000, &["random", "random"]);
        write_log(
            dir.path(),
            &format!("game-{i:02}.jsonl"),
            &h,
            &[event("a"), event("b")],
            Some(GameResult {
                winner: Some(u8::try_from(i % 2).unwrap()),
                reason: EndReason::Victory,
                scores: vec![121, 97],
            }),
            i * 1000 + 500,
        );
    }

    let mut conn = Connection::open_in_memory().unwrap();
    let report =
        ingest_directory::<TestGame, _>(&mut conn, dir.path(), "testgame", &NoopRegistry).unwrap();
    assert_eq!(report.games_ingested, 10);
    assert_eq!(report.agent_rows_written, 20);
    assert!(report.errors.is_empty());
    assert_eq!(query::games_count(&conn).unwrap(), 10);
}

#[test]
fn reingest_is_idempotent_over_the_same_directory() {
    let dir = tempdir().unwrap();
    for i in 0..10u64 {
        let h = header(i, 1_000, &["random", "random"]);
        write_log(
            dir.path(),
            &format!("g-{i:02}.jsonl"),
            &h,
            &[event("x")],
            Some(GameResult {
                winner: Some(0),
                reason: EndReason::Victory,
                scores: vec![121, 10],
            }),
            2_000,
        );
    }

    let mut conn = Connection::open_in_memory().unwrap();
    ingest_directory::<TestGame, _>(&mut conn, dir.path(), "testgame", &NoopRegistry).unwrap();
    let first = query::games_count(&conn).unwrap();
    ingest_directory::<TestGame, _>(&mut conn, dir.path(), "testgame", &NoopRegistry).unwrap();
    let second = query::games_count(&conn).unwrap();
    assert_eq!(first, second, "re-ingest should not duplicate rows");
    assert_eq!(first, 10);
}

#[test]
fn custom_registry_metric_lands_in_game_metrics_with_tags() {
    let dir = tempdir().unwrap();
    let h = header(7, 500, &["random", "random"]);
    write_log(
        dir.path(),
        "g.jsonl",
        &h,
        &[event("red"), event("red"), event("blue")],
        Some(GameResult {
            winner: Some(1),
            reason: EndReason::Victory,
            scores: vec![50, 121],
        }),
        1_500,
    );

    let mut conn = Connection::open_in_memory().unwrap();
    let report =
        ingest_directory::<TestGame, _>(&mut conn, dir.path(), "testgame", &TaggedRegistry)
            .unwrap();
    assert_eq!(report.games_ingested, 1);
    assert!(report.metrics_written > 0);

    // Per-tag metric rows are queryable by (name, tag).
    let red_count: f64 = conn
        .query_row(
            "SELECT value_numeric FROM game_metrics \
             WHERE metric_name = ?1 AND player = -1 AND tag = ?2",
            rusqlite::params![TAGGED_METRIC, "red"],
            |r| r.get(0),
        )
        .unwrap();
    assert!((red_count - 2.0).abs() < 1e-9);
    let blue_count: f64 = conn
        .query_row(
            "SELECT value_numeric FROM game_metrics \
             WHERE metric_name = ?1 AND player = -1 AND tag = ?2",
            rusqlite::params![TAGGED_METRIC, "blue"],
            |r| r.get(0),
        )
        .unwrap();
    assert!((blue_count - 1.0).abs() < 1e-9);
}

#[test]
fn malformed_file_is_reported_and_batch_continues() {
    let dir = tempdir().unwrap();
    // Two valid logs...
    for i in 0..2u64 {
        let h = header(i, 1_000, &["random", "random"]);
        write_log(
            dir.path(),
            &format!("good-{i}.jsonl"),
            &h,
            &[event("a")],
            Some(GameResult {
                winner: Some(0),
                reason: EndReason::Victory,
                scores: vec![121, 0],
            }),
            2_000,
        );
    }
    // ...plus one malformed file.
    std::fs::write(dir.path().join("broken.jsonl"), "not-json-at-all\n").unwrap();

    let mut conn = Connection::open_in_memory().unwrap();
    let report =
        ingest_directory::<TestGame, _>(&mut conn, dir.path(), "testgame", &NoopRegistry).unwrap();
    assert_eq!(report.games_ingested, 2, "good logs still ingested");
    assert_eq!(report.errors.len(), 1, "one file-level error reported");
    assert!(
        report.errors[0].path.ends_with("broken.jsonl"),
        "expected broken.jsonl in error list: {:?}",
        report.errors
    );
}

#[test]
fn wrong_game_name_is_counted_and_skipped_not_errored() {
    let dir = tempdir().unwrap();
    let mut h = header(0, 1_000, &["random", "random"]);
    h.game = "some_other_game".into();
    write_log(
        dir.path(),
        "foreign.jsonl",
        &h,
        &[event("x")],
        Some(GameResult {
            winner: Some(0),
            reason: EndReason::Victory,
            scores: vec![121, 0],
        }),
        2_000,
    );

    let mut conn = Connection::open_in_memory().unwrap();
    let report =
        ingest_directory::<TestGame, _>(&mut conn, dir.path(), "testgame", &NoopRegistry).unwrap();
    assert_eq!(report.games_ingested, 0);
    assert_eq!(report.games_skipped_wrong_game, 1);
    assert!(
        report.errors.is_empty(),
        "wrong-game is a skip, not an error"
    );
}

#[test]
fn schema_mismatch_is_counted_and_skipped() {
    let dir = tempdir().unwrap();
    let mut h = header(0, 1_000, &["random", "random"]);
    h.schema = 99; // pretend future version
    write_log(
        dir.path(),
        "future.jsonl",
        &h,
        &[event("x")],
        Some(GameResult {
            winner: Some(0),
            reason: EndReason::Victory,
            scores: vec![121, 0],
        }),
        2_000,
    );

    let mut conn = Connection::open_in_memory().unwrap();
    let report =
        ingest_directory::<TestGame, _>(&mut conn, dir.path(), "testgame", &NoopRegistry).unwrap();
    assert_eq!(report.games_ingested, 0);
    assert_eq!(report.games_skipped_schema_mismatch, 1);
    assert!(report.errors.is_empty());
}

#[test]
fn builtin_metrics_are_populated_by_ingestion() {
    let dir = tempdir().unwrap();
    let h = header(42, 1_000, &["random", "scripted"]);
    write_log(
        dir.path(),
        "g.jsonl",
        &h,
        &[event("a"), event("b")],
        Some(GameResult {
            winner: Some(0),
            reason: EndReason::Victory,
            scores: vec![121, 55],
        }),
        2_500,
    );

    let mut conn = Connection::open_in_memory().unwrap();
    ingest_directory::<TestGame, _>(&mut conn, dir.path(), "testgame", &NoopRegistry).unwrap();

    // Built-ins landed.
    let winner_tag: String = conn
        .query_row(
            "SELECT value_text FROM game_metrics \
             WHERE metric_name = ?1 AND player = -1",
            rusqlite::params![BuiltInMetrics::WINNER],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(winner_tag, "player_0");

    let wall: f64 = conn
        .query_row(
            "SELECT value_numeric FROM game_metrics \
             WHERE metric_name = ?1 AND player = -1",
            rusqlite::params![BuiltInMetrics::WALL_CLOCK_MS],
            |r| r.get(0),
        )
        .unwrap();
    assert!((wall - 1500.0).abs() < 1e-9);

    // agent_summaries query shows two agents, one of whom won once.
    let sums = query::agent_summaries(&conn).unwrap();
    assert_eq!(sums.len(), 2);
    let random = sums.iter().find(|a| a.agent_name == "random").unwrap();
    assert_eq!(random.games_played, 1);
    assert_eq!(random.wins, 1);
}

#[test]
fn games_table_source_path_and_event_count_are_populated() {
    let dir = tempdir().unwrap();
    let h = header(11, 0, &["a", "b"]);
    let events: Vec<TestEvent> = (0..7).map(|_| event("x")).collect();
    let path = write_log(
        dir.path(),
        "specific-name.jsonl",
        &h,
        &events,
        Some(GameResult {
            winner: None,
            reason: EndReason::Draw,
            scores: vec![60, 60],
        }),
        0,
    );

    let mut conn = Connection::open_in_memory().unwrap();
    ingest_directory::<TestGame, _>(&mut conn, dir.path(), "testgame", &NoopRegistry).unwrap();

    let (src, events, end_reason): (String, i64, String) = conn
        .query_row(
            "SELECT source_path, event_count, end_reason FROM games",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    assert_eq!(src, path.to_string_lossy());
    assert_eq!(events, 7);
    assert_eq!(end_reason, "draw");
}

#[test]
fn init_schema_is_idempotent_across_repeated_calls() {
    let conn = Connection::open_in_memory().unwrap();
    playtest_metrics::init_schema(&conn).unwrap();
    playtest_metrics::init_schema(&conn).unwrap();
    playtest_metrics::init_schema(&conn).unwrap();
    // Tables still exist and are empty.
    assert_eq!(query::games_count(&conn).unwrap(), 0);
}

#[test]
fn untagged_metric_row_stores_empty_string_in_tag_column() {
    let dir = tempdir().unwrap();
    let h = header(0, 1_000, &["random", "random"]);
    write_log(
        dir.path(),
        "g.jsonl",
        &h,
        &[],
        Some(GameResult {
            winner: Some(0),
            reason: EndReason::Victory,
            scores: vec![121, 60],
        }),
        2_000,
    );

    let mut conn = Connection::open_in_memory().unwrap();
    ingest_directory::<TestGame, _>(&mut conn, dir.path(), "testgame", &NoopRegistry).unwrap();

    let tag: String = conn
        .query_row(
            "SELECT tag FROM game_metrics WHERE metric_name = ?1 AND player = -1",
            rusqlite::params![BuiltInMetrics::WINNER],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        tag, "",
        "untagged values must land as empty string, not NULL"
    );
}
