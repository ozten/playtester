//! Serialize → deserialize → eq roundtrip tests for every public wire type.
//!
//! These tests are the behavioural contract of `playtest-api`: if any
//! of them fails, consumers of the wire format (the SvelteKit
//! frontend, the OpenAPI dump) will break.

use playtest_api::{
    API_VERSION, AgentRegistryEntry, ApiError, ApiErrorCode, ApiResponse, CreateRunRequest,
    EventPage, GameMetadata, GameRegistryEntry, GameSummary, LogLineDto, RunStatus, RunSummary,
    SseFrame, http_status,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

fn roundtrip<T>(value: &T) -> T
where
    T: Serialize + for<'de> Deserialize<'de>,
{
    let json = serde_json::to_string(value).expect("serialize");
    serde_json::from_str(&json).expect("deserialize")
}

// ---------- Request / response types ------------------------------------

#[test]
fn create_run_request_roundtrips() {
    let req = CreateRunRequest {
        game: "cribbage".to_owned(),
        agents: vec!["random".to_owned(), "random".to_owned()],
        games_count: 10,
        seed: Some(42),
        config: Some(json!({ "target_score": 121 })),
    };
    assert_eq!(roundtrip(&req), req);
}

#[test]
fn create_run_request_omits_optional_fields() {
    let req = CreateRunRequest {
        game: "cribbage".to_owned(),
        agents: vec!["random".to_owned(), "random".to_owned()],
        games_count: 1,
        seed: None,
        config: None,
    };
    let json = serde_json::to_value(&req).expect("serialize");
    assert!(
        json.get("seed").is_none(),
        "seed should be omitted when None, got: {json}",
    );
    assert!(
        json.get("config").is_none(),
        "config should be omitted when None, got: {json}",
    );
    assert_eq!(roundtrip(&req), req);
}

#[test]
fn run_summary_roundtrips() {
    let summary = RunSummary {
        id: "run-abc".to_owned(),
        game: "cribbage".to_owned(),
        agents: vec!["random".to_owned(), "random".to_owned()],
        games_count: 5,
        games_completed: 3,
        seed: 7,
        status: RunStatus::Running,
        created_at: 1_700_000_000_000,
        finished_at: None,
    };
    assert_eq!(roundtrip(&summary), summary);
}

#[test]
fn run_status_all_variants_roundtrip() {
    for status in [
        RunStatus::Pending,
        RunStatus::Running,
        RunStatus::Completed,
        RunStatus::Failed,
    ] {
        assert_eq!(roundtrip(&status), status);
    }
}

#[test]
fn run_status_unknown_variant_fails_cleanly() {
    // Regression: do not silently drop unknown variants.
    let bogus = "\"Quantum\"";
    let res: Result<RunStatus, _> = serde_json::from_str(bogus);
    assert!(
        res.is_err(),
        "unknown RunStatus variant must fail deserialization, got {res:?}",
    );
}

// ---------- Game browsing types -----------------------------------------

#[test]
fn game_summary_roundtrips() {
    let gs = GameSummary {
        id: "g-1".to_owned(),
        run_id: Some("run-abc".to_owned()),
        game: "cribbage".to_owned(),
        started_at: 1_700_000_000_000,
        finished_at: Some(1_700_000_000_999),
        winner: Some(0),
    };
    assert_eq!(roundtrip(&gs), gs);
}

#[test]
fn game_metadata_roundtrips() {
    let meta = GameMetadata {
        summary: GameSummary {
            id: "g-1".to_owned(),
            run_id: Some("run-abc".to_owned()),
            game: "cribbage".to_owned(),
            started_at: 1_700_000_000_000,
            finished_at: Some(1_700_000_000_999),
            winner: Some(1),
        },
        schema: 2,
        version: "0.1.0".to_owned(),
        seed: 42,
        config_hash: "deadbeef".to_owned(),
        agents: vec!["random".to_owned(), "random".to_owned()],
        scores: Some(vec![98, 121]),
    };
    assert_eq!(roundtrip(&meta), meta);
}

#[test]
fn event_page_roundtrips() {
    let page = EventPage {
        offset: 0,
        limit: 100,
        total: 3,
        events: vec![
            LogLineDto {
                kind: "header".to_owned(),
                line: json!({"kind":"header","schema":2,"game":"cribbage"}),
            },
            LogLineDto {
                kind: "event".to_owned(),
                line: json!({"kind":"event","tick":0,"payload":{}}),
            },
            LogLineDto {
                kind: "final".to_owned(),
                line: json!({"kind":"final","winner":0,"scores":[121,98]}),
            },
        ],
    };
    assert_eq!(roundtrip(&page), page);
}

// ---------- Registry types ----------------------------------------------

#[test]
fn game_registry_entry_roundtrips() {
    let entry = GameRegistryEntry {
        id: "cribbage".to_owned(),
        display_name: "Cribbage (2p)".to_owned(),
        config_schema: json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "type": "object",
            "properties": { "target_score": { "type": "integer" } }
        }),
    };
    assert_eq!(roundtrip(&entry), entry);
}

#[test]
fn agent_registry_entry_roundtrips() {
    let entry = AgentRegistryEntry {
        id: "random".to_owned(),
        display_name: "Uniform Random".to_owned(),
        supported_games: vec!["cribbage".to_owned()],
    };
    assert_eq!(roundtrip(&entry), entry);
}

// ---------- SSE frame types ---------------------------------------------

#[test]
fn sse_frame_event_has_kind_and_data_fields() {
    let payload = json!({"tick": 7, "payload": {"DealCard": "AH"}});
    let frame = SseFrame::Event(payload.clone());
    let as_json: Value = serde_json::to_value(&frame).expect("serialize");

    assert_eq!(as_json.get("kind"), Some(&Value::String("event".into())));
    assert_eq!(as_json.get("data"), Some(&payload));

    assert_eq!(roundtrip(&frame), frame);
}

#[test]
fn sse_frame_header_and_final_have_kind_and_data_fields() {
    let header_payload = json!({"schema": 2, "game": "cribbage"});
    let header = SseFrame::Header(header_payload.clone());
    let header_json: Value = serde_json::to_value(&header).expect("serialize");
    assert_eq!(header_json.get("kind"), Some(&Value::String("header".into())));
    assert_eq!(header_json.get("data"), Some(&header_payload));
    assert_eq!(roundtrip(&header), header);

    let final_payload = json!({"winner": 0, "scores": [121, 98]});
    let fin = SseFrame::Final(final_payload.clone());
    let fin_json: Value = serde_json::to_value(&fin).expect("serialize");
    assert_eq!(fin_json.get("kind"), Some(&Value::String("final".into())));
    assert_eq!(fin_json.get("data"), Some(&final_payload));
    assert_eq!(roundtrip(&fin), fin);
}

#[test]
fn sse_frame_heartbeat_has_no_data_field() {
    let frame = SseFrame::Heartbeat;
    let as_json: Value = serde_json::to_value(&frame).expect("serialize");
    assert_eq!(
        as_json.get("kind"),
        Some(&Value::String("heartbeat".into())),
    );
    assert!(
        as_json.get("data").is_none(),
        "heartbeat should have no data field, got: {as_json}",
    );
    assert_eq!(roundtrip(&frame), frame);
}

// ---------- ApiResponse envelope ----------------------------------------

#[test]
fn api_response_ok_always_carries_api_version() {
    let env = ApiResponse::ok(RunStatus::Completed);
    assert_eq!(env.api_version, API_VERSION);
    let round: ApiResponse<RunStatus> = roundtrip(&env);
    assert_eq!(round.api_version, API_VERSION);
    assert_eq!(round.data, Some(RunStatus::Completed));
    assert!(round.errors.is_empty());
}

#[test]
fn api_response_errors_can_carry_multiple_errors() {
    let err1 = ApiError::new(ApiErrorCode::UnknownGame, "no such game");
    let err2 = ApiError::with_details(
        ApiErrorCode::InvalidConfig,
        "target_score out of range",
        json!({"field": "target_score", "max": 200}),
    );
    let env: ApiResponse<RunSummary> = ApiResponse::fail(vec![err1.clone(), err2.clone()]);

    assert_eq!(env.api_version, API_VERSION);
    assert!(env.data.is_none());
    assert_eq!(env.errors.len(), 2);

    let round: ApiResponse<RunSummary> = roundtrip(&env);
    assert_eq!(round.errors, vec![err1, err2]);
    assert!(round.data.is_none());
}

#[test]
fn api_response_partial_carries_both_data_and_errors() {
    let summary = RunSummary {
        id: "run-abc".to_owned(),
        game: "cribbage".to_owned(),
        agents: vec!["random".to_owned(), "random".to_owned()],
        games_count: 5,
        games_completed: 3,
        seed: 7,
        status: RunStatus::Failed,
        created_at: 1_700_000_000_000,
        finished_at: Some(1_700_000_000_999),
    };
    let err = ApiError::new(ApiErrorCode::Internal, "game 4 crashed");
    let env = ApiResponse::partial(summary.clone(), vec![err.clone()]);

    let round: ApiResponse<RunSummary> = roundtrip(&env);
    assert_eq!(round.api_version, API_VERSION);
    assert_eq!(round.data, Some(summary));
    assert_eq!(round.errors, vec![err]);
}

// ---------- Error code → HTTP status mapping ----------------------------

#[test]
fn http_status_covers_every_variant() {
    assert_eq!(http_status(ApiErrorCode::UnknownGame), 400);
    assert_eq!(http_status(ApiErrorCode::UnknownAgent), 400);
    assert_eq!(http_status(ApiErrorCode::InvalidConfig), 400);
    assert_eq!(http_status(ApiErrorCode::InvalidPaginationParams), 400);
    assert_eq!(http_status(ApiErrorCode::RunNotFound), 404);
    assert_eq!(http_status(ApiErrorCode::GameNotFound), 404);
    assert_eq!(http_status(ApiErrorCode::Internal), 500);
}

#[test]
fn api_error_roundtrips_with_and_without_details() {
    let without = ApiError::new(ApiErrorCode::RunNotFound, "no such run");
    assert_eq!(roundtrip(&without), without);

    let with = ApiError::with_details(
        ApiErrorCode::InvalidPaginationParams,
        "offset must be >= 0",
        json!({"offset": -1}),
    );
    assert_eq!(roundtrip(&with), with);
}
