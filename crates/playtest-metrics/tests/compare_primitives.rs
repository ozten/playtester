//! Integration tests for the Phase 6 cross-DB compare primitives.
//!
//! Seeds two in-memory SQLite DBs by hand (mirroring what the ingest
//! pipeline produces) and verifies enumerate / fetch functions return
//! the expected paired vectors.

use playtest_metrics::{
    MetricKey, PairedMetrics, enumerate_paired_metrics, fetch_agent_outcomes, fetch_games_count,
    fetch_likert_questions, fetch_likert_samples, fetch_numeric_samples, fetch_tag_totals,
    fetch_total_critique_responses, init_schema,
};
use rusqlite::Connection;

fn open() -> Connection {
    let conn = Connection::open_in_memory().unwrap();
    init_schema(&conn).unwrap();
    conn
}

fn insert_game(conn: &Connection, id: &str, seed: i64) {
    conn.execute(
        "INSERT INTO games (id, game, version, seed, started_at, finished_at, winner, end_reason, config_hash, event_count, source_path) \
         VALUES (?1, 'testgame', '0', ?2, 0, 100, 0, 'victory', '0', 10, 'g.jsonl')",
        rusqlite::params![id, seed],
    ).unwrap();
}

fn insert_numeric_metric(conn: &Connection, game_id: &str, name: &str, value: f64) {
    conn.execute(
        "INSERT OR REPLACE INTO game_metrics \
         (game_id, metric_name, player, tag, value_kind, value_numeric, value_text) \
         VALUES (?1, ?2, -1, '', 'scalar', ?3, NULL)",
        rusqlite::params![game_id, name, value],
    )
    .unwrap();
}

fn insert_agent_row(conn: &Connection, game_id: &str, player: i64, name: &str, won: i64, score: i64) {
    conn.execute(
        "INSERT INTO agent_stats (game_id, player, agent_name, won, score) VALUES (?1, ?2, ?3, ?4, ?5)",
        rusqlite::params![game_id, player, name, won, score],
    )
    .unwrap();
}

fn insert_likert(conn: &Connection, game_id: &str, seat: i64, question: &str, score: i64) {
    conn.execute(
        "INSERT OR REPLACE INTO critique_likert \
         (game_id, seat, question, score, spec_version) VALUES (?1, ?2, ?3, ?4, 1)",
        rusqlite::params![game_id, seat, question, score],
    )
    .unwrap();
}

fn insert_tag(conn: &Connection, game_id: &str, seat: i64, tag: &str, sev: i64, ref_card: &str) {
    conn.execute(
        "INSERT OR REPLACE INTO critique_tags \
         (game_id, seat, tag, severity, ref_card) VALUES (?1, ?2, ?3, ?4, ?5)",
        rusqlite::params![game_id, seat, tag, sev, ref_card],
    )
    .unwrap();
}

// ---------------------------------------------------------------------

#[test]
fn enumerate_paired_metrics_splits_into_three_buckets() {
    let a = open();
    insert_game(&a, "g1", 1);
    insert_numeric_metric(&a, "g1", "pegs", 12.0);
    insert_numeric_metric(&a, "g1", "event_count", 30.0);

    let b = open();
    insert_game(&b, "g2", 2);
    insert_numeric_metric(&b, "g2", "pegs", 14.0);
    insert_numeric_metric(&b, "g2", "turn_length", 8.0);

    let PairedMetrics {
        paired,
        only_baseline,
        only_variant,
    } = enumerate_paired_metrics(&a, &b).unwrap();

    let paired_names: Vec<&str> = paired.iter().map(|k| k.name.as_str()).collect();
    let baseline_only_names: Vec<&str> =
        only_baseline.iter().map(|k| k.name.as_str()).collect();
    let variant_only_names: Vec<&str> =
        only_variant.iter().map(|k| k.name.as_str()).collect();

    assert_eq!(paired_names, vec!["pegs"]);
    assert_eq!(baseline_only_names, vec!["event_count"]);
    assert_eq!(variant_only_names, vec!["turn_length"]);
}

#[test]
fn enumerate_paired_metrics_on_identical_dbs_returns_all_paired() {
    let a = open();
    let b = open();
    for conn in [&a, &b] {
        insert_game(conn, "g1", 1);
        insert_numeric_metric(conn, "g1", "pegs", 12.0);
        insert_numeric_metric(conn, "g1", "event_count", 30.0);
    }
    let paired = enumerate_paired_metrics(&a, &b).unwrap();
    assert_eq!(paired.paired.len(), 2);
    assert!(paired.only_baseline.is_empty());
    assert!(paired.only_variant.is_empty());
}

#[test]
fn enumerate_paired_metrics_on_empty_dbs_returns_empty_paired_metrics() {
    let a = open();
    let b = open();
    let paired = enumerate_paired_metrics(&a, &b).unwrap();
    assert!(paired.paired.is_empty());
    assert!(paired.only_baseline.is_empty());
    assert!(paired.only_variant.is_empty());
}

#[test]
fn fetch_numeric_samples_returns_per_game_values_in_game_id_order() {
    let conn = open();
    insert_game(&conn, "g_beta", 2);
    insert_game(&conn, "g_alpha", 1);
    insert_numeric_metric(&conn, "g_beta", "pegs", 14.0);
    insert_numeric_metric(&conn, "g_alpha", "pegs", 12.0);

    let key = MetricKey {
        name: "pegs".into(),
        player: None,
        tag: None,
    };
    let samples = fetch_numeric_samples(&conn, &key).unwrap();
    // ORDER BY game_id: "g_alpha" < "g_beta" lexicographically.
    assert!(
        (samples[0] - 12.0).abs() < 1e-9,
        "expected 12.0 first, got {samples:?}"
    );
    assert!(
        (samples[1] - 14.0).abs() < 1e-9,
        "expected 14.0 second, got {samples:?}"
    );
    assert_eq!(samples.len(), 2);
}

#[test]
fn fetch_numeric_samples_missing_key_returns_empty_vec() {
    let conn = open();
    insert_game(&conn, "g1", 1);
    insert_numeric_metric(&conn, "g1", "pegs", 12.0);

    let key = MetricKey {
        name: "not_present".into(),
        player: None,
        tag: None,
    };
    let samples = fetch_numeric_samples(&conn, &key).unwrap();
    assert!(samples.is_empty());
}

#[test]
fn fetch_numeric_samples_respects_player_sentinel() {
    // Insert two rows for the same metric — one game-scoped (player=-1),
    // one player-scoped (player=0). fetch should honor the sentinel.
    let conn = open();
    insert_game(&conn, "g1", 1);
    conn.execute(
        "INSERT INTO game_metrics (game_id, metric_name, player, tag, value_kind, value_numeric) \
         VALUES ('g1', 'pegs', -1, '', 'scalar', 1.0)",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO game_metrics (game_id, metric_name, player, tag, value_kind, value_numeric) \
         VALUES ('g1', 'pegs', 0, '', 'scalar', 2.0)",
        [],
    )
    .unwrap();

    let game_key = MetricKey {
        name: "pegs".into(),
        player: None,
        tag: None,
    };
    let player_key = MetricKey {
        name: "pegs".into(),
        player: Some(0),
        tag: None,
    };
    assert_eq!(fetch_numeric_samples(&conn, &game_key).unwrap(), vec![1.0]);
    assert_eq!(fetch_numeric_samples(&conn, &player_key).unwrap(), vec![2.0]);
}

#[test]
fn fetch_numeric_samples_skips_text_kind_rows() {
    let conn = open();
    insert_game(&conn, "g1", 1);
    conn.execute(
        "INSERT INTO game_metrics (game_id, metric_name, player, tag, value_kind, value_numeric, value_text) \
         VALUES ('g1', 'archetype', -1, '', 'tag', NULL, 'aggro')",
        [],
    )
    .unwrap();
    let key = MetricKey {
        name: "archetype".into(),
        player: None,
        tag: None,
    };
    let samples = fetch_numeric_samples(&conn, &key).unwrap();
    assert!(
        samples.is_empty(),
        "tag-kind rows must not surface in numeric-sample fetches"
    );
}

#[test]
fn fetch_agent_outcomes_returns_wins_and_games_per_agent() {
    let conn = open();
    insert_game(&conn, "g1", 1);
    insert_game(&conn, "g2", 2);
    insert_agent_row(&conn, "g1", 0, "random", 1, 100);
    insert_agent_row(&conn, "g1", 1, "heuristic", 0, 80);
    insert_agent_row(&conn, "g2", 0, "random", 0, 70);
    insert_agent_row(&conn, "g2", 1, "heuristic", 1, 110);

    let outcomes = fetch_agent_outcomes(&conn).unwrap();
    let as_map: std::collections::BTreeMap<&str, (u64, u64)> = outcomes
        .iter()
        .map(|(name, wins, games)| (name.as_str(), (*wins, *games)))
        .collect();
    assert_eq!(as_map.get("random"), Some(&(1, 2)));
    assert_eq!(as_map.get("heuristic"), Some(&(1, 2)));
}

#[test]
fn fetch_likert_samples_returns_scores_per_game_and_seat() {
    let conn = open();
    insert_game(&conn, "g1", 1);
    insert_game(&conn, "g2", 2);
    insert_likert(&conn, "g1", 0, "agency", 4);
    insert_likert(&conn, "g1", 1, "agency", 5);
    insert_likert(&conn, "g2", 0, "agency", 3);
    insert_likert(&conn, "g1", 0, "fairness", 5); // different question, not returned

    let samples = fetch_likert_samples(&conn, "agency").unwrap();
    assert_eq!(samples.len(), 3);
    // Order: by (game_id, seat). g1 seat 0 → 4, g1 seat 1 → 5, g2 seat 0 → 3.
    assert!((samples[0] - 4.0).abs() < 1e-9);
    assert!((samples[1] - 5.0).abs() < 1e-9);
    assert!((samples[2] - 3.0).abs() < 1e-9);
}

#[test]
fn fetch_likert_questions_returns_distinct_sorted() {
    let conn = open();
    insert_game(&conn, "g1", 1);
    insert_likert(&conn, "g1", 0, "fairness", 4);
    insert_likert(&conn, "g1", 1, "agency", 5);
    insert_likert(&conn, "g1", 0, "agency", 3); // duplicate question

    let qs = fetch_likert_questions(&conn).unwrap();
    assert_eq!(qs, vec!["agency".to_owned(), "fairness".to_owned()]);
}

#[test]
fn fetch_total_critique_responses_counts_distinct_seat_pairs() {
    let conn = open();
    insert_game(&conn, "g1", 1);
    insert_game(&conn, "g2", 2);
    // seat 0 + 1 in g1; seat 0 in g2 → 3 distinct (game,seat) pairs.
    insert_likert(&conn, "g1", 0, "agency", 4);
    insert_likert(&conn, "g1", 0, "fairness", 4); // same (g1, 0) — shouldn't double-count
    insert_likert(&conn, "g1", 1, "agency", 3);
    insert_likert(&conn, "g2", 0, "agency", 5);

    assert_eq!(fetch_total_critique_responses(&conn).unwrap(), 3);
}

#[test]
fn fetch_tag_totals_returns_mention_counts_sorted_alpha() {
    let conn = open();
    insert_game(&conn, "g1", 1);
    insert_tag(&conn, "g1", 0, "forced_sacrifice", 3, "typhoon");
    insert_tag(&conn, "g1", 1, "forced_sacrifice", 2, "");
    insert_tag(&conn, "g1", 0, "lack_of_agency", 3, "");

    let totals = fetch_tag_totals(&conn).unwrap();
    assert_eq!(
        totals,
        vec![
            playtest_metrics::CritiqueTagTotal {
                tag: "forced_sacrifice".into(),
                count: 2,
            },
            playtest_metrics::CritiqueTagTotal {
                tag: "lack_of_agency".into(),
                count: 1,
            },
        ]
    );
}

#[test]
fn fetch_games_count_sums_games_table() {
    let conn = open();
    insert_game(&conn, "g1", 1);
    insert_game(&conn, "g2", 2);
    insert_game(&conn, "g3", 3);
    assert_eq!(fetch_games_count(&conn).unwrap(), 3);
}

#[test]
fn metric_key_label_renders_all_four_shapes() {
    assert_eq!(
        MetricKey {
            name: "x".into(),
            player: None,
            tag: None
        }
        .label(),
        "x"
    );
    assert_eq!(
        MetricKey {
            name: "x".into(),
            player: Some(1),
            tag: None
        }
        .label(),
        "x@p1"
    );
    assert_eq!(
        MetricKey {
            name: "x".into(),
            player: None,
            tag: Some("fives".into())
        }
        .label(),
        "x:fives"
    );
    assert_eq!(
        MetricKey {
            name: "x".into(),
            player: Some(0),
            tag: Some("runs".into())
        }
        .label(),
        "x@p0:runs"
    );
}
