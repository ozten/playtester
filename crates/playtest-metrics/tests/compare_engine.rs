//! Integration tests for `run_compare`.
//!
//! Seeds two in-memory SQLite DBs by hand, runs the engine, and
//! asserts on the structured result. The engine is game-agnostic
//! here — these tests use the same synthetic-DB approach as the
//! `compare_primitives.rs` tests.

use playtest_metrics::{
    CompareOpts, CompareResult, Correction, CritiqueAvailability, FindingKind, init_schema,
    run_compare,
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

fn insert_agent_row(
    conn: &Connection,
    game_id: &str,
    player: i64,
    name: &str,
    won: i64,
    score: i64,
) {
    conn.execute(
        "INSERT INTO agent_stats (game_id, player, agent_name, won, score) VALUES (?1, ?2, ?3, ?4, ?5)",
        rusqlite::params![game_id, player, name, won, score],
    )
    .unwrap();
}

fn insert_likert(conn: &Connection, game_id: &str, seat: i64, q: &str, score: i64) {
    conn.execute(
        "INSERT OR REPLACE INTO critique_likert \
         (game_id, seat, question, score, spec_version) VALUES (?1, ?2, ?3, ?4, 1)",
        rusqlite::params![game_id, seat, q, score],
    )
    .unwrap();
}

/// Seed `conn` with `n_games` games; each emits `metric_name = value`
/// (so Welch sees `n_games` identical samples on this side).
fn seed_games_with_metric(conn: &Connection, n_games: usize, metric_name: &str, value: f64) {
    for i in 0..n_games {
        let gid = format!("g{i:04}");
        insert_game(conn, &gid, i64::try_from(i).unwrap_or(0));
        insert_numeric_metric(conn, &gid, metric_name, value);
    }
}

/// Same, but with noise: value varies per game via `value_gen(i)`.
fn seed_games_with_noisy_metric(
    conn: &Connection,
    n_games: usize,
    metric_name: &str,
    value_gen: impl Fn(usize) -> f64,
) {
    for i in 0..n_games {
        let gid = format!("g{i:04}");
        insert_game(conn, &gid, i64::try_from(i).unwrap_or(0));
        insert_numeric_metric(conn, &gid, metric_name, value_gen(i));
    }
}

// -----------------------------------------------------------------

#[test]
fn identical_dbs_produce_zero_flagged_findings() {
    let a = open();
    let b = open();
    seed_games_with_metric(&a, 100, "pegs", 12.0);
    seed_games_with_metric(&b, 100, "pegs", 12.0);
    // Both cohorts play the same agent with the same win record.
    for i in 0..100 {
        let gid = format!("g{i:04}");
        insert_agent_row(&a, &gid, 0, "random", i64::from(i < 50), 100);
        insert_agent_row(&b, &gid, 0, "random", i64::from(i < 50), 100);
    }

    let result = run_compare(&b, &a, &CompareOpts::default()).unwrap();
    assert!(
        result.flagged.is_empty(),
        "identical DBs must produce zero flagged findings: got {:?}",
        result.flagged
    );
    assert_eq!(result.n_baseline, 100);
    assert_eq!(result.n_variant, 100);
    assert_eq!(result.critique, CritiqueAvailability::Neither);
}

#[test]
fn large_mean_shift_is_flagged() {
    // Variant cohort (A) has pegs = 20 across 100 games; baseline
    // (B) has pegs = 10. Delta is huge; Welch produces p < 0.001.
    let baseline = open();
    let variant = open();
    seed_games_with_noisy_metric(&baseline, 100, "pegs", |i| {
        #[allow(clippy::cast_precision_loss)]
        let x = (i as f64).sin() * 0.5;
        10.0 + x
    });
    seed_games_with_noisy_metric(&variant, 100, "pegs", |i| {
        #[allow(clippy::cast_precision_loss)]
        let x = (i as f64).sin() * 0.5;
        20.0 + x
    });

    let result = run_compare(&baseline, &variant, &CompareOpts::default()).unwrap();
    let pegs = result
        .flagged
        .iter()
        .find(|f| f.label == "pegs")
        .expect("pegs must be flagged");
    assert_eq!(pegs.kind, FindingKind::NumericMetric);
    // delta = variant − baseline ≈ 10.0.
    assert!(
        (pegs.outcome.delta - 10.0).abs() < 0.5,
        "expected delta ≈ 10, got {:?}",
        pegs.outcome
    );
    assert!(pegs.significant);
}

#[test]
fn bonferroni_flags_fewer_or_equal_than_bh_on_same_input() {
    // Three noisy-but-different metrics; use Bonferroni vs BH and
    // verify the flagged count only shrinks.
    let baseline = open();
    let variant = open();
    seed_games_with_noisy_metric(&baseline, 100, "pegs", |i| {
        #[allow(clippy::cast_precision_loss)]
        let x = (i as f64).sin() * 0.5;
        10.0 + x
    });
    seed_games_with_noisy_metric(&variant, 100, "pegs", |i| {
        #[allow(clippy::cast_precision_loss)]
        let x = (i as f64).sin() * 0.5;
        12.0 + x // mild shift, on the edge of significance
    });

    let bh_result = run_compare(
        &baseline,
        &variant,
        &CompareOpts {
            alpha: 0.05,
            correction: Correction::BenjaminiHochberg,
        },
    )
    .unwrap();
    let bonf_result = run_compare(
        &baseline,
        &variant,
        &CompareOpts {
            alpha: 0.05,
            correction: Correction::Bonferroni,
        },
    )
    .unwrap();

    assert!(
        bonf_result.flagged.len() <= bh_result.flagged.len(),
        "Bonferroni must flag ≤ BH ({} vs {})",
        bonf_result.flagged.len(),
        bh_result.flagged.len()
    );
    // Total finding count should match (correction only affects split).
    let total_bh = bh_result.flagged.len() + bh_result.rejected.len() + bh_result.unchanged.len();
    let total_bonf =
        bonf_result.flagged.len() + bonf_result.rejected.len() + bonf_result.unchanged.len();
    assert_eq!(total_bh, total_bonf);
}

#[test]
fn only_baseline_and_only_variant_metrics_are_reported() {
    let baseline = open();
    let variant = open();
    seed_games_with_metric(&baseline, 10, "metric_only_in_baseline", 5.0);
    seed_games_with_metric(&variant, 10, "metric_only_in_variant", 5.0);

    let result = run_compare(&baseline, &variant, &CompareOpts::default()).unwrap();
    let baseline_only: Vec<&str> =
        result.only_baseline.iter().map(|k| k.name.as_str()).collect();
    let variant_only: Vec<&str> =
        result.only_variant.iter().map(|k| k.name.as_str()).collect();
    assert_eq!(baseline_only, vec!["metric_only_in_baseline"]);
    assert_eq!(variant_only, vec!["metric_only_in_variant"]);
    // Those metrics are NOT in `flagged` (delta is undefined for
    // a one-sided metric).
    assert!(
        !result
            .flagged
            .iter()
            .any(|f| f.label == "metric_only_in_baseline"),
        "one-sided metrics must not appear in flagged"
    );
}

#[test]
fn critique_availability_detects_one_sided_data() {
    let baseline = open();
    let variant = open();
    insert_game(&baseline, "g1", 1);
    insert_game(&variant, "g1", 1);
    insert_likert(&baseline, "g1", 0, "agency", 4);
    // variant has no critique data.

    let result = run_compare(&baseline, &variant, &CompareOpts::default()).unwrap();
    assert_eq!(result.critique, CritiqueAvailability::OnlyBaseline);
    // No Likert or CodedTag findings should appear.
    assert!(
        !result
            .flagged
            .iter()
            .chain(result.unchanged.iter())
            .any(|f| matches!(f.kind, FindingKind::LikertQuestion | FindingKind::CodedTag)),
        "no critique findings when only one side has data"
    );
}

#[test]
fn critique_both_sides_produces_likert_findings() {
    let baseline = open();
    let variant = open();
    // Seed 10 games each with agency Likert = 4 on baseline, = 2 on variant.
    for i in 0..10 {
        let gid = format!("g{i:03}");
        insert_game(&baseline, &gid, i64::from(i));
        insert_game(&variant, &gid, i64::from(i));
        insert_likert(&baseline, &gid, 0, "agency", 4);
        insert_likert(&variant, &gid, 0, "agency", 2);
    }

    let result = run_compare(&baseline, &variant, &CompareOpts::default()).unwrap();
    assert_eq!(result.critique, CritiqueAvailability::Both);
    let agency = result
        .flagged
        .iter()
        .chain(result.rejected.iter())
        .chain(result.unchanged.iter())
        .find(|f| f.kind == FindingKind::LikertQuestion && f.label == "agency")
        .expect("agency finding must exist");
    // delta = variant − baseline = 2 − 4 = −2.
    assert!((agency.outcome.delta - (-2.0)).abs() < 1e-6);
}

#[test]
fn per_agent_win_rate_shift_is_flagged() {
    // 100 games each: baseline agent wins 50%, variant agent wins 80%.
    let baseline = open();
    let variant = open();
    for i in 0..100 {
        let gid = format!("g{i:04}");
        insert_game(&baseline, &gid, i64::from(i));
        insert_game(&variant, &gid, i64::from(i));
        insert_agent_row(&baseline, &gid, 0, "random", i64::from(i < 50), 100);
        insert_agent_row(&variant, &gid, 0, "random", i64::from(i < 80), 100);
    }

    let result = run_compare(&baseline, &variant, &CompareOpts::default()).unwrap();
    let random = result
        .flagged
        .iter()
        .find(|f| f.kind == FindingKind::AgentWinRate && f.label == "random")
        .expect("random agent win-rate finding must exist");
    assert!(
        (random.outcome.delta - 0.3).abs() < 1e-6,
        "variant − baseline = 0.80 − 0.50 = 0.30, got delta = {}",
        random.outcome.delta
    );
}

#[test]
fn empty_databases_produce_empty_result_no_panic() {
    let baseline = open();
    let variant = open();
    let result: CompareResult = run_compare(&baseline, &variant, &CompareOpts::default()).unwrap();
    assert!(result.flagged.is_empty());
    assert!(result.rejected.is_empty());
    assert!(result.unchanged.is_empty());
    assert!(result.only_baseline.is_empty());
    assert!(result.only_variant.is_empty());
    assert_eq!(result.n_baseline, 0);
    assert_eq!(result.n_variant, 0);
    assert_eq!(result.critique, CritiqueAvailability::Neither);
}

#[test]
fn flagged_findings_are_sorted_by_absolute_delta_desc() {
    // Three metrics with different delta magnitudes; all should
    // clear correction in this contrived data.
    let baseline = open();
    let variant = open();
    // One games row per game_id per DB; metrics share game_ids.
    for i in 0..100 {
        let gid = format!("g{i:04}");
        insert_game(&baseline, &gid, i64::from(i));
        insert_game(&variant, &gid, i64::from(i));
        insert_numeric_metric(&baseline, &gid, "small_delta", 10.0);
        insert_numeric_metric(&baseline, &gid, "medium_delta", 10.0);
        insert_numeric_metric(&baseline, &gid, "large_delta", 10.0);
        insert_numeric_metric(&variant, &gid, "small_delta", 10.5);
        insert_numeric_metric(&variant, &gid, "medium_delta", 12.0);
        insert_numeric_metric(&variant, &gid, "large_delta", 20.0);
    }

    let result = run_compare(&baseline, &variant, &CompareOpts::default()).unwrap();
    let numeric_flagged: Vec<&str> = result
        .flagged
        .iter()
        .filter(|f| f.kind == FindingKind::NumericMetric)
        .map(|f| f.label.as_str())
        .collect();
    // All three should be flagged given the huge effect + zero variance.
    assert_eq!(numeric_flagged.len(), 3);
    // Sort: |delta| desc → large, medium, small.
    assert_eq!(numeric_flagged, vec!["large_delta", "medium_delta", "small_delta"]);
}

#[test]
fn identical_dbs_with_all_three_families_produce_zero_flagged() {
    // The R6.9 "cosmetic change" exit criterion — two byte-identical
    // DBs must produce zero flagged findings across all families.
    let baseline = open();
    let variant = open();
    for i in 0..50 {
        let gid = format!("g{i:03}");
        insert_game(&baseline, &gid, i64::from(i));
        insert_game(&variant, &gid, i64::from(i));
        insert_numeric_metric(&baseline, &gid, "pegs", 12.0);
        insert_numeric_metric(&variant, &gid, "pegs", 12.0);
        insert_agent_row(&baseline, &gid, 0, "random", i64::from(i < 25), 100);
        insert_agent_row(&variant, &gid, 0, "random", i64::from(i < 25), 100);
        insert_likert(&baseline, &gid, 0, "agency", 4);
        insert_likert(&variant, &gid, 0, "agency", 4);
    }

    for correction in [Correction::BenjaminiHochberg, Correction::Bonferroni] {
        let result = run_compare(
            &baseline,
            &variant,
            &CompareOpts {
                alpha: 0.05,
                correction,
            },
        )
        .unwrap();
        assert!(
            result.flagged.is_empty(),
            "identical DBs with correction={correction:?} produced {} flagged (R6.9 violation)",
            result.flagged.len()
        );
    }
}
