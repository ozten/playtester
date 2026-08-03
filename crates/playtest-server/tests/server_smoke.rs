//! End-to-end HTTP smoke test against a real server instance.
//!
//! Spawns the axum app on an ephemeral port, runs a short Cribbage
//! self-play (2 games, seed 42), and verifies the documented
//! endpoints return the expected shapes.

mod common;

use std::time::Duration;

use common::SpawnedServer;
use playtest_api::{API_VERSION, ApiErrorCode, ApiResponse, CreateRunRequest, EventPage, GameSummary, RunStatus, RunSummary};
use serde::Deserialize;
use serde_json::json;

#[derive(Debug, Deserialize)]
struct HealthBody {
    status: String,
    api_version: String,
}

#[tokio::test]
async fn health_returns_api_version() {
    let server = SpawnedServer::start().await;
    let client = reqwest::Client::new();

    let resp: ApiResponse<HealthBody> = client
        .get(format!("{}/api/health", server.base_url))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    assert_eq!(resp.api_version, API_VERSION);
    let body = resp.data.expect("data");
    assert_eq!(body.status, "ok");
    assert_eq!(body.api_version, API_VERSION);

    server.shutdown();
}

#[tokio::test]
async fn create_run_drives_games_to_completion() {
    let server = SpawnedServer::start().await;
    let client = reqwest::Client::new();

    let req = CreateRunRequest {
        game: "cribbage".into(),
        agents: vec!["random".into(), "random".into()],
        games_count: 2,
        seed: Some(42),
        config: None,
    };

    let post: ApiResponse<RunSummary> = client
        .post(format!("{}/api/runs", server.base_url))
        .json(&req)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(post.errors.is_empty(), "unexpected errors: {:?}", post.errors);
    let run = post.data.expect("data");
    let run_id = run.id.clone();
    assert_eq!(run.game, "cribbage");
    assert_eq!(run.games_count, 2);

    // Poll until the run completes (the two random-vs-random
    // Cribbage games should finish well inside a second).
    let mut final_summary: Option<RunSummary> = None;
    for _ in 0..100 {
        let r: ApiResponse<RunSummary> = client
            .get(format!("{}/api/runs/{run_id}", server.base_url))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        let s = r.data.expect("data");
        if matches!(s.status, RunStatus::Completed) {
            final_summary = Some(s);
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let s = final_summary.expect("run did not complete in time");
    assert_eq!(s.games_completed, 2);
    assert!(s.finished_at.is_some());

    // List games.
    let games: ApiResponse<Vec<GameSummary>> = client
        .get(format!("{}/api/runs/{run_id}/games", server.base_url))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let games = games.data.expect("data");
    assert_eq!(games.len(), 2);
    let first_gid = games[0].id.clone();

    // Event page.
    let page: ApiResponse<EventPage> = client
        .get(format!(
            "{}/api/runs/{run_id}/games/{first_gid}/events?offset=0&limit=5",
            server.base_url
        ))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let page = page.data.expect("data");
    assert!(page.total >= 1, "total should be >= 1, got {page:?}");
    assert!(page.events.len() <= 5);
    assert_eq!(page.events[0].kind, "header");

    server.shutdown();
}

#[tokio::test]
async fn unknown_game_returns_400() {
    let server = SpawnedServer::start().await;
    let client = reqwest::Client::new();

    let req = json!({
        "game": "notreal",
        "agents": ["random", "random"],
        "games_count": 1,
        "seed": 1,
    });

    let resp = client
        .post(format!("{}/api/runs", server.base_url))
        .json(&req)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status().as_u16(), 400);
    let body: ApiResponse<()> = resp.json().await.unwrap();
    assert!(body.errors.iter().any(|e| e.code == ApiErrorCode::UnknownGame));

    server.shutdown();
}

#[tokio::test]
async fn five_agent_cribbage_returns_400() {
    let server = SpawnedServer::start().await;
    let client = reqwest::Client::new();

    let req = json!({
        "game": "cribbage",
        "agents": ["random", "random", "random", "random", "random"],
        "games_count": 1,
        "seed": 1,
    });

    let resp = client
        .post(format!("{}/api/runs", server.base_url))
        .json(&req)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status().as_u16(), 400);
    let body: ApiResponse<()> = resp.json().await.unwrap();
    assert!(body.errors.iter().any(|e| e.code == ApiErrorCode::InvalidConfig));

    server.shutdown();
}

#[tokio::test]
async fn three_agent_cribbage_returns_400() {
    let server = SpawnedServer::start().await;
    let client = reqwest::Client::new();

    let req = json!({
        "game": "cribbage",
        "agents": ["random", "random", "random"],
        "games_count": 1,
        "seed": 1,
    });

    let resp = client
        .post(format!("{}/api/runs", server.base_url))
        .json(&req)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status().as_u16(), 400);
    let body: ApiResponse<()> = resp.json().await.unwrap();
    assert!(body.errors.iter().any(|e| e.code == ApiErrorCode::InvalidConfig));

    server.shutdown();
}

#[tokio::test]
async fn one_agent_shipwreck_returns_400() {
    let server = SpawnedServer::start().await;
    let client = reqwest::Client::new();

    let req = json!({
        "game": "shipwreck",
        "agents": ["random"],
        "games_count": 1,
        "seed": 1,
    });

    let resp = client
        .post(format!("{}/api/runs", server.base_url))
        .json(&req)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status().as_u16(), 400);
    let body: ApiResponse<()> = resp.json().await.unwrap();
    assert!(body.errors.iter().any(|e| e.code == ApiErrorCode::InvalidConfig));

    server.shutdown();
}

#[tokio::test]
async fn four_agent_shipwreck_run_is_accepted() {
    let server = SpawnedServer::start().await;
    let client = reqwest::Client::new();

    let req = CreateRunRequest {
        game: "shipwreck".into(),
        agents: vec![
            "random".into(),
            "random".into(),
            "random".into(),
            "random".into(),
        ],
        games_count: 1,
        seed: Some(7),
        config: None,
    };

    let post: ApiResponse<RunSummary> = client
        .post(format!("{}/api/runs", server.base_url))
        .json(&req)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(post.errors.is_empty(), "unexpected errors: {:?}", post.errors);
    let run = post.data.expect("data");
    assert_eq!(run.game, "shipwreck");
    assert_eq!(run.agents.len(), 4);

    server.shutdown();
}

#[tokio::test]
async fn reports_endpoints_are_stubbed_as_501() {
    let server = SpawnedServer::start().await;
    let client = reqwest::Client::new();

    for path in [
        "/api/reports".to_owned(),
        "/api/reports/abc".to_owned(),
        "/api/reports/abc/markdown".to_owned(),
    ] {
        let req_builder = if path == "/api/reports" {
            client
                .post(format!("{}{}", server.base_url, path))
                .json(&json!({"run_id": "dummy"}))
        } else {
            client.get(format!("{}{}", server.base_url, path))
        };
        let resp = req_builder.send().await.unwrap();
        assert_eq!(resp.status().as_u16(), 501, "{path} should be 501");
        let body: ApiResponse<serde_json::Value> = resp.json().await.unwrap();
        assert!(body
            .errors
            .iter()
            .any(|e| e.code == ApiErrorCode::NotImplemented));
    }

    server.shutdown();
}
