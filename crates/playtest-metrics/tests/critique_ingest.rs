//! Integration tests for Phase 5 critique-sidecar ingest.
//!
//! Reuses the synthetic-game approach from `ingest.rs`: the metrics
//! crate must not depend on any specific game. Here we hand-write
//! both a main log and a companion `<basename>.critique.jsonl`, then
//! assert the two critique tables got populated idempotently.

use std::fs::{self, File};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};

use playtest_core::{Actor, EndReason, Game, GameError, GameResult, PlayerId};
use playtest_log::{LogHeader, LogRecord, SCHEMA_VERSION};
use playtest_metrics::{
    MetricDef, MetricRegistry, MetricValue, ingest_directory,
};
use playtest_ports::Rng;
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use tempfile::tempdir;
use uuid::Uuid;

// ---------- Synthetic game ----------------------------------------------

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

// ---------- Log helpers --------------------------------------------------

fn write_main_log(dir: &Path, name: &str, seed: u64) -> PathBuf {
    let path = dir.join(name);
    let file = File::create(&path).unwrap();
    let mut w = BufWriter::new(file);
    let header = LogHeader {
        schema: SCHEMA_VERSION,
        game: "testgame".into(),
        version: "0.0.0".into(),
        seed,
        agents: vec!["llm".into(), "llm".into()],
        started_at: 1,
        config_hash: "0".repeat(64),
    };
    let hdr = serde_json::to_string(&LogRecord::<TestEvent>::Header(header)).unwrap();
    writeln!(w, "{hdr}").unwrap();
    let final_rec = LogRecord::<TestEvent>::Final {
        winner: Some(0),
        reason: EndReason::Victory,
        scores: vec![121, 97],
        finished_at: 2,
    };
    writeln!(w, "{}", serde_json::to_string(&final_rec).unwrap()).unwrap();
    path
}

type QuestionnaireRow<'a> = (u8, &'a [(&'a str, u8)], &'a [(&'a str, &'a str)]);
type CodedTagRow<'a> = (u8, &'a [(&'a str, u8, Option<&'a str>)]);

fn write_critique_sidecar(
    main_log: &Path,
    responses: &[QuestionnaireRow<'_>],
    coded_tags: &[CodedTagRow<'_>],
) -> PathBuf {
    // Derive sidecar path from the main log path.
    let sidecar = PathBuf::from(format!(
        "{}.critique.jsonl",
        main_log.to_string_lossy().trim_end_matches(".jsonl")
    ));
    let mut contents = String::new();
    contents.push_str(r#"{"kind":"critique_sidecar_header","game":"testgame","seed":42,"questionnaire_spec_sha256":"abc","rules_text_sha256":"def"}"#);
    contents.push('\n');
    for (seat, likert, open_ended) in responses {
        let mut likert_map = serde_json::Map::new();
        for (k, v) in *likert {
            likert_map.insert((*k).into(), serde_json::json!(v));
        }
        let mut open_map = serde_json::Map::new();
        for (k, v) in *open_ended {
            open_map.insert((*k).into(), serde_json::json!(v));
        }
        let rec = serde_json::json!({
            "kind": "questionnaire_response",
            "seat": seat,
            "spec_version": 1,
            "likert": likert_map,
            "open_ended": open_map,
        });
        contents.push_str(&rec.to_string());
        contents.push('\n');
    }
    for (seat, tags) in coded_tags {
        let tag_arr: Vec<serde_json::Value> = tags
            .iter()
            .map(|(tag, severity, ref_card)| {
                serde_json::json!({
                    "tag": tag,
                    "severity": severity,
                    "ref_card": ref_card,
                })
            })
            .collect();
        let rec = serde_json::json!({
            "kind": "coded_tag",
            "seat": seat,
            "tags": tag_arr,
        });
        contents.push_str(&rec.to_string());
        contents.push('\n');
    }
    fs::write(&sidecar, contents).unwrap();
    sidecar
}

// ---------- Tests -------------------------------------------------------

#[test]
fn ingest_populates_likert_and_tags_from_sidecar() {
    let dir = tempdir().unwrap();
    let main = write_main_log(dir.path(), "g-0000.jsonl", 42);
    write_critique_sidecar(
        &main,
        &[
            (
                0,
                &[("agency", 4), ("fairness", 5), ("tension", 3)],
                &[("worst_moment", "typhoon")],
            ),
            (
                1,
                &[("agency", 2), ("fairness", 3), ("tension", 4)],
                &[("worst_moment", "lost my cordage")],
            ),
        ],
        &[
            (
                0,
                &[
                    ("forced_sacrifice", 3, Some("typhoon")),
                    ("lack_of_agency", 2, None),
                ],
            ),
            (1, &[("snowball_loss", 4, None)]),
        ],
    );

    let mut conn = Connection::open_in_memory().unwrap();
    let report = ingest_directory::<TestGame, _>(
        &mut conn,
        dir.path(),
        "testgame",
        &NoopRegistry,
    )
    .unwrap();
    assert_eq!(report.games_ingested, 1);
    // 3 Likert keys × 2 seats = 6 rows.
    assert_eq!(report.critique_likert_rows, 6);
    // 2 + 1 = 3 tag rows.
    assert_eq!(report.critique_tag_rows, 3);

    let likert_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM critique_likert", [], |r| r.get(0))
        .unwrap();
    assert_eq!(likert_count, 6);
    let tags_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM critique_tags", [], |r| r.get(0))
        .unwrap();
    assert_eq!(tags_count, 3);

    // Spot-check: seat 0's agency score is 4.
    let score: i64 = conn
        .query_row(
            "SELECT score FROM critique_likert WHERE seat = 0 AND question = 'agency'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(score, 4);

    // Spot-check: the ref_card for forced_sacrifice is "typhoon".
    let ref_card: String = conn
        .query_row(
            "SELECT ref_card FROM critique_tags \
             WHERE seat = 0 AND tag = 'forced_sacrifice'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(ref_card, "typhoon");

    // lack_of_agency has ref_card = '' (the null-sentinel).
    let empty: String = conn
        .query_row(
            "SELECT ref_card FROM critique_tags WHERE seat = 0 AND tag = 'lack_of_agency'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(empty, "");
}

#[test]
fn ingest_without_critique_sidecar_leaves_tables_empty() {
    let dir = tempdir().unwrap();
    let _main = write_main_log(dir.path(), "g-0000.jsonl", 42);

    let mut conn = Connection::open_in_memory().unwrap();
    let report = ingest_directory::<TestGame, _>(
        &mut conn,
        dir.path(),
        "testgame",
        &NoopRegistry,
    )
    .unwrap();
    assert_eq!(report.games_ingested, 1);
    assert_eq!(report.critique_likert_rows, 0);
    assert_eq!(report.critique_tag_rows, 0);

    let likert_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM critique_likert", [], |r| r.get(0))
        .unwrap();
    assert_eq!(likert_count, 0);
}

#[test]
fn reingest_is_idempotent_for_critique_rows() {
    let dir = tempdir().unwrap();
    let main = write_main_log(dir.path(), "g-0000.jsonl", 42);
    write_critique_sidecar(
        &main,
        &[(0, &[("agency", 4)], &[("worst_moment", "x")])],
        &[(0, &[("forced_sacrifice", 3, Some("typhoon"))])],
    );

    let mut conn = Connection::open_in_memory().unwrap();
    let _ = ingest_directory::<TestGame, _>(
        &mut conn,
        dir.path(),
        "testgame",
        &NoopRegistry,
    )
    .unwrap();
    let _ = ingest_directory::<TestGame, _>(
        &mut conn,
        dir.path(),
        "testgame",
        &NoopRegistry,
    )
    .unwrap();

    // Re-ingestion deletes + reinserts — row count stays at 1 for
    // each table, not duplicated.
    let likert_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM critique_likert", [], |r| r.get(0))
        .unwrap();
    assert_eq!(likert_count, 1);
    let tag_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM critique_tags", [], |r| r.get(0))
        .unwrap();
    assert_eq!(tag_count, 1);
}

#[test]
fn multiple_coded_tag_records_for_same_seat_last_wins() {
    let dir = tempdir().unwrap();
    let main = write_main_log(dir.path(), "g-0000.jsonl", 42);
    write_critique_sidecar(
        &main,
        &[(0, &[("agency", 4)], &[("worst_moment", "x")])],
        &[
            // Earlier coding — should be overwritten.
            (0, &[("forced_sacrifice", 1, Some("typhoon"))]),
            // Later coding — should win.
            (0, &[("lack_of_agency", 5, None)]),
        ],
    );

    let mut conn = Connection::open_in_memory().unwrap();
    let _ = ingest_directory::<TestGame, _>(
        &mut conn,
        dir.path(),
        "testgame",
        &NoopRegistry,
    )
    .unwrap();

    // Only the later record survived.
    let rows: Vec<(String, i64)> = conn
        .prepare("SELECT tag, severity FROM critique_tags WHERE seat = 0")
        .unwrap()
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].0, "lack_of_agency");
    assert_eq!(rows[0].1, 5);
}

#[test]
fn score_outside_1_to_5_is_silently_dropped() {
    // An upstream producer bug could emit score=7; the CHECK
    // constraint would reject the INSERT, aborting the transaction.
    // Our code filters those out pre-insert and just skips the row.
    let dir = tempdir().unwrap();
    let main = write_main_log(dir.path(), "g-0000.jsonl", 42);
    write_critique_sidecar(
        &main,
        &[(
            0,
            &[("agency", 7), ("fairness", 4)],
            &[("worst_moment", "x")],
        )],
        &[],
    );

    let mut conn = Connection::open_in_memory().unwrap();
    let report = ingest_directory::<TestGame, _>(
        &mut conn,
        dir.path(),
        "testgame",
        &NoopRegistry,
    )
    .unwrap();
    // Only `fairness` survived — `agency: 7` was dropped.
    assert_eq!(report.critique_likert_rows, 1);
    let question: String = conn
        .query_row("SELECT question FROM critique_likert", [], |r| r.get(0))
        .unwrap();
    assert_eq!(question, "fairness");
}
