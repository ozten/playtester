//! Integration tests for `POST /api/runs/{run_id}/games/{game_id}/actions`.
//!
//! Exercises the rejection taxonomy at the HTTP layer. The full
//! happy-path round-trip lives in `http_remote_e2e.rs` (Unit 6); the
//! coordinator-internal rejection semantics are already covered by
//! `turn_coordinator::tests` in `src/turn_coordinator.rs`.
//!
//! Tests in this file deliberately do NOT create runs that stay blocked
//! on pending remote prompts — the plan's "No submission timeout /
//! abandoned-game GC" scope boundary means a stuck remote agent
//! genuinely hangs the game loop, and a dropped tokio runtime waits on
//! spawn_blocking threads. Coverage of rejection paths that require a
//! live coordinator is in `http_remote_e2e.rs`, where the test always
//! submits enough actions to let the game finish.

mod common;

use common::SpawnedServer;
use playtest_api::{ApiErrorCode, ApiResponse, CreateRunRequest, RunSummary};
use serde_json::json;

async fn create_ai_run(server: &SpawnedServer) -> String {
    let client = reqwest::Client::new();
    let req = CreateRunRequest {
        game: "cribbage".into(),
        agents: vec!["random".into(), "random".into()],
        games_count: 1,
        seed: Some(7),
        config: None,
    };
    let resp: ApiResponse<RunSummary> = client
        .post(format!("{}/api/runs", server.base_url))
        .json(&req)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    resp.data.expect("run created").id
}

#[tokio::test]
async fn unknown_run_returns_404_run_not_found() {
    let server = SpawnedServer::start().await;
    let client = reqwest::Client::new();

    let resp = client
        .post(format!(
            "{}/api/runs/00000000-0000-0000-0000-000000000000/games/game-0000/actions",
            server.base_url
        ))
        .json(&json!({"seat": 0, "prompt_id": 0, "action_index": 0}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status().as_u16(), 404);
    let body: ApiResponse<()> = resp.json().await.unwrap();
    assert_eq!(body.errors[0].code, ApiErrorCode::RunNotFound);

    server.shutdown();
}

#[tokio::test]
async fn malformed_run_id_returns_404_run_not_found() {
    let server = SpawnedServer::start().await;
    let client = reqwest::Client::new();

    let resp = client
        .post(format!(
            "{}/api/runs/not-a-uuid/games/game-0000/actions",
            server.base_url
        ))
        .json(&json!({"seat": 0, "prompt_id": 0, "action_index": 0}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status().as_u16(), 404);
    let body: ApiResponse<()> = resp.json().await.unwrap();
    assert_eq!(body.errors[0].code, ApiErrorCode::RunNotFound);

    server.shutdown();
}

#[tokio::test]
async fn submit_to_ai_only_run_returns_game_not_found() {
    // AI-only runs never allocate a TurnCoordinator, so the route's
    // `turn_coordinators.get(&gid)` returns None and the handler maps
    // to GameNotFound — correct semantically (there is no interactive
    // game to submit to) and cheap to assert.
    let server = SpawnedServer::start().await;
    let client = reqwest::Client::new();

    let run_id = create_ai_run(&server).await;

    let resp = client
        .post(format!(
            "{}/api/runs/{run_id}/games/game-0000/actions",
            server.base_url
        ))
        .json(&json!({"seat": 0, "prompt_id": 0, "action_index": 0}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status().as_u16(), 404);
    let body: ApiResponse<()> = resp.json().await.unwrap();
    assert_eq!(body.errors[0].code, ApiErrorCode::GameNotFound);

    server.shutdown();
}

#[tokio::test]
async fn malformed_body_returns_400() {
    let server = SpawnedServer::start().await;
    let client = reqwest::Client::new();

    let run_id = create_ai_run(&server).await;

    let resp = client
        .post(format!(
            "{}/api/runs/{run_id}/games/game-0000/actions",
            server.base_url
        ))
        .header("content-type", "application/json")
        .body("{not valid json")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status().as_u16(), 400);

    server.shutdown();
}

#[tokio::test]
async fn missing_required_fields_returns_400() {
    let server = SpawnedServer::start().await;
    let client = reqwest::Client::new();

    let run_id = create_ai_run(&server).await;

    // Missing `action_index`.
    let resp = client
        .post(format!(
            "{}/api/runs/{run_id}/games/game-0000/actions",
            server.base_url
        ))
        .json(&json!({"seat": 0, "prompt_id": 0}))
        .send()
        .await
        .unwrap();
    assert!(
        resp.status().is_client_error(),
        "expected 4xx, got {}",
        resp.status()
    );

    server.shutdown();
}
