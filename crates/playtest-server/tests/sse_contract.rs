//! Integration test for the per-game SSE stream.
//!
//! Spawns a server, posts a 1-game Cribbage run, connects to the
//! game's SSE endpoint, and asserts the expected `header` → `event`…
//! → `final` frame sequence arrives.

mod common;

use std::time::Duration;

use common::SpawnedServer;
use futures::StreamExt;
use playtest_api::{ApiResponse, CreateRunRequest, GameSummary, RunStatus, RunSummary};

#[tokio::test]
async fn sse_stream_emits_header_events_and_final() {
    let server = SpawnedServer::start().await;
    let client = reqwest::Client::new();

    let req = CreateRunRequest {
        game: "cribbage".into(),
        agents: vec!["random".into(), "random".into()],
        games_count: 1,
        seed: Some(123),
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
    let run_id = post.data.expect("run").id;

    // Give the supervisor a tick to register the first game's
    // broadcaster, then subscribe. The race is benign either way:
    // if the game already finished, catch-up from the JSONL file
    // still delivers the full sequence.
    let mut game_id: Option<String> = None;
    for _ in 0..50 {
        let games: ApiResponse<Vec<GameSummary>> = client
            .get(format!("{}/api/runs/{run_id}/games", server.base_url))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        if let Some(g) = games.data.and_then(|v| v.into_iter().next()) {
            game_id = Some(g.id);
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let game_id = game_id.expect("game registered in time");

    let url = format!(
        "{}/api/runs/{run_id}/games/{game_id}/stream",
        server.base_url
    );

    let resp = client.get(&url).send().await.unwrap();
    assert!(resp.status().is_success(), "status: {}", resp.status());
    let mut stream = resp.bytes_stream();

    let mut buf = String::new();
    let mut saw_header = false;
    let mut saw_event = false;
    let mut saw_final = false;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);

    while tokio::time::Instant::now() < deadline {
        let Some(chunk) = tokio::time::timeout(Duration::from_millis(500), stream.next())
            .await
            .ok()
            .flatten()
        else {
            continue;
        };
        let Ok(chunk) = chunk else { break };
        let text = std::str::from_utf8(&chunk).unwrap_or("");
        buf.push_str(text);

        for record in split_sse_records(&buf) {
            let mut event_name: Option<&str> = None;
            for line in record.lines() {
                if let Some(rest) = line.strip_prefix("event:") {
                    event_name = Some(rest.trim());
                }
            }
            match event_name {
                Some("header") => saw_header = true,
                Some("event") => saw_event = true,
                Some("final") => saw_final = true,
                _ => {}
            }
        }
        // Trim the buffer up to the last `\n\n` so partial records stay.
        if let Some(idx) = buf.rfind("\n\n") {
            buf.drain(..idx + 2);
        }

        if saw_header && saw_event && saw_final {
            break;
        }
        // Also stop if the run is done and the stream closed.
        let r: ApiResponse<RunSummary> = client
            .get(format!("{}/api/runs/{run_id}", server.base_url))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        if matches!(
            r.data.map(|s| s.status),
            Some(RunStatus::Completed | RunStatus::Failed)
        ) && saw_final
        {
            break;
        }
    }

    assert!(saw_header, "expected header frame");
    assert!(saw_event, "expected at least one event frame");
    assert!(saw_final, "expected final frame");

    server.shutdown();
}

/// Very small SSE splitter: records are separated by `\n\n`.
fn split_sse_records(buf: &str) -> Vec<&str> {
    buf.split("\n\n")
        .filter(|r| !r.trim().is_empty())
        .collect()
}
