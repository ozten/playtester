//! Unit 6 — `POST /api/runs` must reject `llm` and `stdio` agent kinds
//! with `AgentKindNotAllowedHere` (HTTP 400).
//!
//! The rationale lives in the Phase 3 plan: both agent kinds need
//! caller-supplied dependencies (API key, child binary path) that the
//! HTTP surface deliberately doesn't accept. Users targeting these
//! kinds must run `playtest play --agents ...` from the CLI instead.

mod common;

use common::SpawnedServer;
use playtest_api::{ApiErrorCode, ApiResponse};
use serde_json::json;

#[tokio::test]
async fn post_runs_rejects_llm_agent_kind() {
    let server = SpawnedServer::start().await;
    let client = reqwest::Client::new();

    let resp = client
        .post(format!("{}/api/runs", server.base_url))
        .json(&json!({
            "game": "cribbage",
            "agents": ["llm", "random"],
            "games_count": 1,
            "seed": 7,
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status().as_u16(), 400);
    let body: ApiResponse<()> = resp.json().await.unwrap();
    assert_eq!(
        body.errors[0].code,
        ApiErrorCode::AgentKindNotAllowedHere,
        "expected AgentKindNotAllowedHere, got {:?}",
        body.errors
    );
    assert!(
        body.errors[0].message.contains("CLI-only")
            || body.errors[0].message.contains("playtest play"),
        "message should point at the CLI; got: {}",
        body.errors[0].message
    );

    server.shutdown();
}

#[tokio::test]
async fn post_runs_rejects_parameterized_llm_agent_kind() {
    let server = SpawnedServer::start().await;
    let client = reqwest::Client::new();

    let resp = client
        .post(format!("{}/api/runs", server.base_url))
        .json(&json!({
            "game": "cribbage",
            "agents": ["llm:provider=anthropic,model=claude-haiku-4-5-20251001", "random"],
            "games_count": 1,
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status().as_u16(), 400);
    let body: ApiResponse<()> = resp.json().await.unwrap();
    assert_eq!(body.errors[0].code, ApiErrorCode::AgentKindNotAllowedHere);

    server.shutdown();
}

#[tokio::test]
async fn post_runs_rejects_stdio_agent_kind() {
    let server = SpawnedServer::start().await;
    let client = reqwest::Client::new();

    let resp = client
        .post(format!("{}/api/runs", server.base_url))
        .json(&json!({
            "game": "cribbage",
            "agents": ["stdio:cmd=/bin/cat", "random"],
            "games_count": 1,
            "seed": 3,
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status().as_u16(), 400);
    let body: ApiResponse<()> = resp.json().await.unwrap();
    assert_eq!(
        body.errors[0].code,
        ApiErrorCode::AgentKindNotAllowedHere,
        "expected AgentKindNotAllowedHere, got {:?}",
        body.errors
    );

    server.shutdown();
}

#[tokio::test]
async fn post_runs_rejects_bare_stdio_kind() {
    let server = SpawnedServer::start().await;
    let client = reqwest::Client::new();

    let resp = client
        .post(format!("{}/api/runs", server.base_url))
        .json(&json!({
            "game": "cribbage",
            "agents": ["random", "stdio"],
            "games_count": 1,
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status().as_u16(), 400);
    let body: ApiResponse<()> = resp.json().await.unwrap();
    assert_eq!(body.errors[0].code, ApiErrorCode::AgentKindNotAllowedHere);

    server.shutdown();
}
