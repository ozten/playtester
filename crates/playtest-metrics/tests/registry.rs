//! Integration tests for `MetricRegistry` and `BuiltInMetrics`.
//!
//! Uses a synthetic `TestGame` rather than importing `playtest-cribbage`
//! (per the plan's verification: `playtest-metrics` must not depend on
//! any specific game crate).

use playtest_core::{Actor, EndReason, Game, GameError, GameResult, PlayerId};
use playtest_log::{LogHeader, LogRecord, SCHEMA_VERSION};
use playtest_metrics::{
    BuiltInMetrics, GameLog, MetricKind, MetricRegistry, MetricScope, MetricValueKind,
    validate_values_against_defs,
};
use playtest_ports::Rng;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

// ---------- Synthetic game -----------------------------------------------

#[derive(Debug)]
struct TestGame;

#[derive(Clone, PartialEq, Eq)]
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

// ---------- Helpers ------------------------------------------------------

fn header_with_agents(seed: u64, agents: Vec<String>) -> LogHeader {
    header_with(seed, agents, 0)
}

fn header_with(seed: u64, agents: Vec<String>, started_at: u64) -> LogHeader {
    LogHeader {
        schema: SCHEMA_VERSION,
        game: "testgame".into(),
        version: "0.0.0".into(),
        seed,
        agents,
        started_at,
        config_hash: "0".repeat(64),
    }
}

fn event(tag: &str) -> TestEvent {
    TestEvent { tag: tag.into() }
}

fn build_log(
    seed: u64,
    agents: Vec<String>,
    events: Vec<TestEvent>,
    final_result: Option<GameResult>,
) -> GameLog<TestGame> {
    build_log_with_timing(seed, agents, events, final_result, 0, 0)
}

fn build_log_with_timing(
    seed: u64,
    agents: Vec<String>,
    events: Vec<TestEvent>,
    final_result: Option<GameResult>,
    started_at: u64,
    finished_at: u64,
) -> GameLog<TestGame> {
    let mut records: Vec<Result<LogRecord<TestEvent>, playtest_log::ReadError>> = Vec::new();
    records.push(Ok(LogRecord::Header(header_with(seed, agents, started_at))));
    for (tick, payload) in events.into_iter().enumerate() {
        records.push(Ok(LogRecord::Event {
            tick: tick as u64,
            payload,
        }));
    }
    if let Some(res) = final_result {
        records.push(Ok(LogRecord::Final {
            winner: res.winner,
            reason: res.reason,
            scores: res.scores,
            finished_at,
        }));
    }
    GameLog::<TestGame>::from_records(records).expect("log builds cleanly")
}

// ---------- Scenarios ----------------------------------------------------

#[test]
fn builtin_metrics_definitions_are_internally_consistent() {
    let defs = <BuiltInMetrics as MetricRegistry<TestGame>>::metric_definitions(&BuiltInMetrics);
    // No duplicate names.
    playtest_metrics::validate_definitions(&defs).expect("no duplicates");
    // Every name we claim in the `BuiltInMetrics::*` constants actually appears.
    for name in [
        BuiltInMetrics::GAME_LENGTH_TICKS,
        BuiltInMetrics::WINNER,
        BuiltInMetrics::END_REASON,
        BuiltInMetrics::SCORE_MARGIN,
        BuiltInMetrics::AGENT_NAME,
        BuiltInMetrics::FINAL_SCORE,
        BuiltInMetrics::WALL_CLOCK_MS,
    ] {
        assert!(defs.iter().any(|d| d.name == name), "missing def: {name}");
    }
}

#[test]
fn builtin_extraction_matches_definitions() {
    let log = build_log(
        42,
        vec!["random".into(), "random".into()],
        vec![event("a"), event("b"), event("c")],
        Some(GameResult {
            winner: Some(0),
            reason: EndReason::Victory,
            scores: vec![121, 97],
        }),
    );
    let g = Uuid::new_v4();
    let defs = <BuiltInMetrics as MetricRegistry<TestGame>>::metric_definitions(&BuiltInMetrics);
    let values = BuiltInMetrics.extract(g, &log);
    validate_values_against_defs(&defs, &values).expect("emitted values match defs");
}

#[test]
fn builtin_game_length_ticks_counts_events() {
    let log = build_log(
        0,
        vec!["random".into(), "random".into()],
        (0..7).map(|i| event(&format!("e{i}"))).collect(),
        None,
    );
    let values = BuiltInMetrics.extract(Uuid::nil(), &log);
    let v = values
        .iter()
        .find(|v| v.metric_name == BuiltInMetrics::GAME_LENGTH_TICKS)
        .expect("game_length_ticks emitted");
    assert_eq!(v.value, MetricValueKind::Count(7));
    assert_eq!(v.player, None);
}

#[test]
fn builtin_winner_and_end_reason_from_final_record() {
    let log = build_log(
        0,
        vec!["random".into(), "random".into()],
        vec![],
        Some(GameResult {
            winner: Some(1),
            reason: EndReason::Victory,
            scores: vec![98, 121],
        }),
    );
    let values = BuiltInMetrics.extract(Uuid::nil(), &log);
    let winner = values
        .iter()
        .find(|v| v.metric_name == BuiltInMetrics::WINNER)
        .expect("winner emitted");
    assert_eq!(winner.value, MetricValueKind::Tag("player_1".into()));
    let reason = values
        .iter()
        .find(|v| v.metric_name == BuiltInMetrics::END_REASON)
        .expect("end_reason emitted");
    assert_eq!(reason.value, MetricValueKind::Tag("victory".into()));
}

#[test]
fn builtin_score_margin_is_max_minus_min() {
    let log = build_log(
        0,
        vec!["a".into(), "b".into()],
        vec![],
        Some(GameResult {
            winner: Some(0),
            reason: EndReason::Victory,
            scores: vec![121, 97],
        }),
    );
    let values = BuiltInMetrics.extract(Uuid::nil(), &log);
    let margin = values
        .iter()
        .find(|v| v.metric_name == BuiltInMetrics::SCORE_MARGIN)
        .expect("score_margin emitted");
    assert_eq!(margin.value, MetricValueKind::Count(24));
}

#[test]
fn builtin_agent_name_emitted_per_player() {
    let log = build_log(0, vec!["random".into(), "scripted".into()], vec![], None);
    let values = BuiltInMetrics.extract(Uuid::nil(), &log);
    let names: Vec<_> = values
        .iter()
        .filter(|v| v.metric_name == BuiltInMetrics::AGENT_NAME)
        .collect();
    assert_eq!(names.len(), 2);
    assert!(names.iter().any(
        |v| v.player == Some(0) && matches!(&v.value, MetricValueKind::Tag(t) if t == "random")
    ));
    assert!(
        names.iter().any(|v| v.player == Some(1)
            && matches!(&v.value, MetricValueKind::Tag(t) if t == "scripted"))
    );
}

#[test]
fn builtin_final_score_emitted_per_player_only_when_final_record_present() {
    let log_no_final = build_log(0, vec!["a".into(), "b".into()], vec![], None);
    let values = BuiltInMetrics.extract(Uuid::nil(), &log_no_final);
    assert!(
        values
            .iter()
            .all(|v| v.metric_name != BuiltInMetrics::FINAL_SCORE),
        "final_score should not be emitted without a Final record"
    );

    let log_with_final = build_log(
        0,
        vec!["a".into(), "b".into()],
        vec![],
        Some(GameResult {
            winner: Some(1),
            reason: EndReason::Victory,
            scores: vec![100, 121],
        }),
    );
    let values = BuiltInMetrics.extract(Uuid::nil(), &log_with_final);
    let scores: Vec<_> = values
        .iter()
        .filter(|v| v.metric_name == BuiltInMetrics::FINAL_SCORE)
        .collect();
    assert_eq!(scores.len(), 2);
    let s0 = scores.iter().find(|v| v.player == Some(0)).unwrap();
    assert_eq!(s0.value, MetricValueKind::Count(100));
    let s1 = scores.iter().find(|v| v.player == Some(1)).unwrap();
    assert_eq!(s1.value, MetricValueKind::Count(121));
}

#[test]
fn unfinished_log_produces_sensible_tags() {
    // Edge case: log cut off before the Final record. Plan: "a game
    // with zero events (header + final only) produces sensible
    // built-in metric values" — this is the mirror case (header +
    // events, no final), which is what a mid-crash log looks like.
    let log = build_log(
        0,
        vec!["random".into(), "random".into()],
        (0..3).map(|i| event(&format!("e{i}"))).collect(),
        None,
    );
    let values = BuiltInMetrics.extract(Uuid::nil(), &log);
    let winner = values
        .iter()
        .find(|v| v.metric_name == BuiltInMetrics::WINNER)
        .unwrap();
    assert_eq!(winner.value, MetricValueKind::Tag("unfinished".into()));
    let reason = values
        .iter()
        .find(|v| v.metric_name == BuiltInMetrics::END_REASON)
        .unwrap();
    assert_eq!(reason.value, MetricValueKind::Tag("unfinished".into()));
    let margin = values
        .iter()
        .find(|v| v.metric_name == BuiltInMetrics::SCORE_MARGIN)
        .unwrap();
    assert_eq!(margin.value, MetricValueKind::Count(0));
}

#[test]
fn zero_event_log_has_header_and_final_only() {
    let log = build_log(
        0,
        vec!["a".into(), "b".into()],
        vec![],
        Some(GameResult {
            winner: None,
            reason: EndReason::Draw,
            scores: vec![60, 60],
        }),
    );
    let values = BuiltInMetrics.extract(Uuid::nil(), &log);
    let len = values
        .iter()
        .find(|v| v.metric_name == BuiltInMetrics::GAME_LENGTH_TICKS)
        .unwrap();
    assert_eq!(len.value, MetricValueKind::Count(0));
    let winner = values
        .iter()
        .find(|v| v.metric_name == BuiltInMetrics::WINNER)
        .unwrap();
    assert_eq!(winner.value, MetricValueKind::Tag("draw".into()));
    let reason = values
        .iter()
        .find(|v| v.metric_name == BuiltInMetrics::END_REASON)
        .unwrap();
    assert_eq!(reason.value, MetricValueKind::Tag("draw".into()));
}

#[test]
fn end_reason_other_preserves_the_inner_string() {
    let log = build_log(
        0,
        vec!["a".into(), "b".into()],
        vec![],
        Some(GameResult {
            winner: Some(0),
            reason: EndReason::Other("deadline_exceeded".into()),
            scores: vec![1, 0],
        }),
    );
    let values = BuiltInMetrics.extract(Uuid::nil(), &log);
    let reason = values
        .iter()
        .find(|v| v.metric_name == BuiltInMetrics::END_REASON)
        .unwrap();
    assert_eq!(
        reason.value,
        MetricValueKind::Tag("other:deadline_exceeded".into())
    );
}

#[test]
fn registry_definitions_match_emitted_values_across_three_log_shapes() {
    // "Built-in metrics tested against at least three different log
    // fixtures" from the plan's Verification section.
    let defs = <BuiltInMetrics as MetricRegistry<TestGame>>::metric_definitions(&BuiltInMetrics);

    let logs = [
        build_log(1, vec!["a".into(), "b".into()], vec![event("x")], None),
        build_log(
            2,
            vec!["a".into(), "b".into()],
            (0..12).map(|i| event(&format!("e{i}"))).collect(),
            Some(GameResult {
                winner: Some(0),
                reason: EndReason::Victory,
                scores: vec![121, 70],
            }),
        ),
        build_log(
            3,
            vec!["a".into(), "b".into()],
            vec![],
            Some(GameResult {
                winner: None,
                reason: EndReason::Draw,
                scores: vec![0, 0],
            }),
        ),
    ];

    for (i, log) in logs.iter().enumerate() {
        let values = BuiltInMetrics.extract(Uuid::new_v4(), log);
        validate_values_against_defs(&defs, &values).unwrap_or_else(|e| panic!("fixture {i}: {e}"));
    }
}

#[test]
fn wall_clock_ms_is_finished_at_minus_started_at() {
    let log = build_log_with_timing(
        0,
        vec!["a".into(), "b".into()],
        vec![],
        Some(GameResult {
            winner: Some(0),
            reason: EndReason::Victory,
            scores: vec![121, 98],
        }),
        1_000,
        1_420,
    );
    let values = BuiltInMetrics.extract(Uuid::nil(), &log);
    let wall = values
        .iter()
        .find(|v| v.metric_name == BuiltInMetrics::WALL_CLOCK_MS)
        .expect("wall_clock_ms emitted");
    assert_eq!(wall.value, MetricValueKind::Count(420));
    assert_eq!(wall.player, None);
}

#[test]
fn wall_clock_ms_absent_when_no_final_record() {
    let log = build_log(
        0,
        vec!["a".into(), "b".into()],
        (0..3).map(|i| event(&format!("e{i}"))).collect(),
        None,
    );
    let values = BuiltInMetrics.extract(Uuid::nil(), &log);
    assert!(
        values
            .iter()
            .all(|v| v.metric_name != BuiltInMetrics::WALL_CLOCK_MS),
        "wall_clock_ms should be absent without a Final record"
    );
}

#[test]
fn wall_clock_ms_clamps_to_zero_on_backwards_clock() {
    // Stub clocks or tape divergence can produce finished_at < started_at.
    // The metric clamps rather than emitting a nonsense negative number.
    let log = build_log_with_timing(
        0,
        vec!["a".into(), "b".into()],
        vec![],
        Some(GameResult {
            winner: Some(0),
            reason: EndReason::Victory,
            scores: vec![121, 0],
        }),
        2_000,
        1_000,
    );
    let values = BuiltInMetrics.extract(Uuid::nil(), &log);
    let wall = values
        .iter()
        .find(|v| v.metric_name == BuiltInMetrics::WALL_CLOCK_MS)
        .expect("wall_clock_ms emitted");
    assert_eq!(wall.value, MetricValueKind::Count(0));
}

#[test]
fn wall_clock_ms_absent_for_v1_log_without_finished_at() {
    // Backward-compat: a v1 log has `finished_at: 0` via serde default;
    // GameLog::load maps that to `finished_at: None`, and the metric
    // stays absent rather than being misreported as "0 ms".
    let log = build_log_with_timing(
        0,
        vec!["a".into(), "b".into()],
        vec![],
        Some(GameResult {
            winner: Some(0),
            reason: EndReason::Victory,
            scores: vec![121, 98],
        }),
        1_000,
        0, // simulates the v1 default
    );
    let values = BuiltInMetrics.extract(Uuid::nil(), &log);
    assert!(
        values
            .iter()
            .all(|v| v.metric_name != BuiltInMetrics::WALL_CLOCK_MS),
        "wall_clock_ms should be absent when finished_at was the v1 default"
    );
}

#[test]
fn game_log_load_rejects_logs_with_no_header() {
    // The underlying LogReader just yields records; we build empty.
    let records: Vec<Result<LogRecord<TestEvent>, _>> = vec![];
    let err = GameLog::<TestGame>::from_records(records).unwrap_err();
    assert!(matches!(err, playtest_metrics::LoadError::MissingHeader));
}

#[test]
fn game_log_load_rejects_duplicate_headers() {
    let header = header_with_agents(0, vec!["a".into(), "b".into()]);
    let records: Vec<Result<LogRecord<TestEvent>, _>> = vec![
        Ok(LogRecord::Header(header.clone())),
        Ok(LogRecord::Header(header)),
    ];
    let err = GameLog::<TestGame>::from_records(records).unwrap_err();
    assert!(matches!(err, playtest_metrics::LoadError::DuplicateHeader));
}

#[test]
fn metric_def_enumerates_all_four_kinds_and_both_scopes() {
    // Sanity: every kind and scope is usable in a MetricDef.
    let kinds = [
        MetricKind::Scalar,
        MetricKind::Count,
        MetricKind::Tag,
        MetricKind::Bool,
    ];
    let scopes = [MetricScope::Game, MetricScope::Player];
    for k in kinds {
        for s in scopes {
            let def = playtest_metrics::MetricDef {
                name: format!("{k:?}_{s:?}"),
                kind: k,
                scope: s,
                description: String::new(),
            };
            let json = serde_json::to_string(&def).unwrap();
            let back: playtest_metrics::MetricDef = serde_json::from_str(&json).unwrap();
            assert_eq!(def, back);
        }
    }
}
