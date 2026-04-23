//! Integration tests for the Phase 5 "Subjective critique" markdown
//! section. Inserts critique rows directly via SQL so the test stays
//! independent of the ingest pipeline (which has its own test file).

use playtest_metrics::{
    MarkdownBuilder, init_schema, write_subjective_critique_section,
};
use rusqlite::Connection;

fn open() -> Connection {
    let conn = Connection::open_in_memory().unwrap();
    init_schema(&conn).unwrap();
    // A stub `games` row so the FK on critique_likert can point
    // somewhere valid — the CASCADE gives us free cleanup in the
    // per-test teardown.
    conn.execute(
        "INSERT INTO games (id, game, version, seed, started_at, finished_at, winner, end_reason, config_hash, event_count, source_path) \
         VALUES ('g1', 'cribbage', '0', 0, 0, 100, 0, 'victory', '0', 10, 'g1.jsonl')",
        [],
    ).unwrap();
    conn
}

fn insert_likert(
    conn: &Connection,
    seat: i64,
    question: &str,
    score: i64,
    spec_version: i64,
) {
    conn.execute(
        "INSERT OR REPLACE INTO critique_likert \
         (game_id, seat, question, score, spec_version) VALUES \
         ('g1', ?1, ?2, ?3, ?4)",
        rusqlite::params![seat, question, score, spec_version],
    )
    .unwrap();
}

fn insert_tag(conn: &Connection, seat: i64, tag: &str, severity: i64, ref_card: &str) {
    conn.execute(
        "INSERT OR REPLACE INTO critique_tags \
         (game_id, seat, tag, severity, ref_card) VALUES \
         ('g1', ?1, ?2, ?3, ?4)",
        rusqlite::params![seat, tag, severity, ref_card],
    )
    .unwrap();
}

#[test]
fn section_is_omitted_entirely_when_no_critique_data() {
    let conn = open();
    let mut md = MarkdownBuilder::new();
    write_subjective_critique_section(&mut md, &conn).unwrap();
    let out = md.into_string();
    assert!(
        !out.contains("Subjective critique"),
        "empty critique must not render a heading"
    );
}

#[test]
fn likert_table_renders_means_and_ci_when_n_ge_5() {
    let conn = open();
    // Five data points for `agency`: [3, 4, 4, 5, 4]; mean = 4.0.
    for (seat, score) in [(0, 3), (1, 4), (2, 4), (3, 5), (4, 4)] {
        insert_likert(&conn, seat, "agency", score, 1);
    }

    let mut md = MarkdownBuilder::new();
    write_subjective_critique_section(&mut md, &conn).unwrap();
    let out = md.into_string();
    assert!(out.contains("## Subjective critique"));
    assert!(out.contains("### Likert means"));
    assert!(out.contains("| agency"));
    assert!(out.contains("4.00"));
    // CI should be present because n = 5.
    assert!(
        out.contains('['),
        "expected a [lower, upper] CI for n=5; got: {out}"
    );
}

#[test]
fn likert_table_shows_dash_for_small_n() {
    let conn = open();
    // Only 2 data points — below the CI threshold of 5.
    insert_likert(&conn, 0, "agency", 4, 1);
    insert_likert(&conn, 1, "agency", 5, 1);

    let mut md = MarkdownBuilder::new();
    write_subjective_critique_section(&mut md, &conn).unwrap();
    let out = md.into_string();
    assert!(out.contains("## Subjective critique"));
    assert!(out.contains("| agency"));
    assert!(
        out.contains("—"),
        "expected `—` placeholder for n<5 CI; got: {out}"
    );
}

#[test]
fn overall_tag_histogram_is_ordered_by_count_desc() {
    let conn = open();
    // `forced_sacrifice` mentioned 3×, `lack_of_agency` 1×.
    insert_tag(&conn, 0, "forced_sacrifice", 3, "typhoon");
    insert_tag(&conn, 1, "forced_sacrifice", 3, "shark");
    insert_tag(&conn, 2, "forced_sacrifice", 2, "");
    insert_tag(&conn, 3, "lack_of_agency", 2, "");

    let mut md = MarkdownBuilder::new();
    write_subjective_critique_section(&mut md, &conn).unwrap();
    let out = md.into_string();
    assert!(out.contains("### Coded tags (overall)"));
    // `forced_sacrifice` should appear before `lack_of_agency`.
    let fs_pos = out.find("forced_sacrifice").unwrap();
    let la_pos = out.find("lack_of_agency").unwrap();
    assert!(
        fs_pos < la_pos,
        "expected `forced_sacrifice` before `lack_of_agency` (count desc)"
    );
}

#[test]
fn per_card_table_suppresses_low_mention_cards() {
    let conn = open();
    // Threshold is ≥ 3 mentions. `typhoon` gets 3 distinct tags
    // (each a separate PK row) → subsection renders. `shark` gets
    // 1 tag → subsection suppressed.
    insert_tag(&conn, 0, "forced_sacrifice", 3, "typhoon");
    insert_tag(&conn, 1, "random_loss", 2, "typhoon");
    insert_tag(&conn, 2, "lack_of_agency", 2, "typhoon");
    insert_tag(&conn, 3, "forced_sacrifice", 3, "shark");

    let mut md = MarkdownBuilder::new();
    write_subjective_critique_section(&mut md, &conn).unwrap();
    let out = md.into_string();
    assert!(
        out.contains("### Coded tags (per card)"),
        "expected per-card heading: {out}"
    );
    assert!(out.contains("**typhoon**"), "expected typhoon subsection: {out}");
    assert!(
        !out.contains("**shark**"),
        "single-mention card must NOT get a per-card subsection: {out}"
    );
}

#[test]
fn mixed_spec_versions_render_a_warning_banner() {
    let conn = open();
    insert_likert(&conn, 0, "agency", 4, 1);
    insert_likert(&conn, 1, "agency", 3, 2);

    let mut md = MarkdownBuilder::new();
    write_subjective_critique_section(&mut md, &conn).unwrap();
    let out = md.into_string();
    assert!(
        out.contains("**Warning:**"),
        "expected warning banner for mixed spec versions: {out}"
    );
    assert!(out.contains("1, 2"), "expected list of versions: {out}");
}

#[test]
fn tag_only_run_still_renders_section_without_likert_subsection() {
    // Sanity: if someone deletes the questionnaire but keeps
    // coded_tag data (unusual but possible after a failed ingest),
    // the section still renders the tag histograms and no Likert
    // subsection.
    let conn = open();
    insert_tag(&conn, 0, "boring_early_game", 3, "");

    let mut md = MarkdownBuilder::new();
    write_subjective_critique_section(&mut md, &conn).unwrap();
    let out = md.into_string();
    assert!(out.contains("## Subjective critique"));
    assert!(out.contains("boring_early_game"));
    assert!(
        !out.contains("### Likert means"),
        "Likert subsection must be omitted when no data: {out}"
    );
}
