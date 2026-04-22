//! End-to-end test: drive a full Cribbage game via HTTP submissions.
//!
//! This is the load-bearing Phase 2.5 integration test. It proves the
//! entire stack works together:
//!
//! 1. Create a run with `agents: ["http-remote", "random"]` and
//!    subscribe to the per-game SSE stream.
//! 2. Every `turn_prompt` frame received is answered with
//!    `action_index: 0` (pick the first legal action) via
//!    `POST .../actions`.
//! 3. The game runs to completion; `final` frame fires.
//! 4. The on-disk JSONL log is complete and contains no `turn_prompt`
//!    records (turn_prompt is ephemeral by design — see the Phase 2.5
//!    plan's Key Technical Decisions).
//!
//! The response to every prompt is always `action_index: 0`. This is
//! always legal (the engine guarantees a non-empty legal-actions list
//! when it calls an agent) and is deterministic given the seed + the
//! random opponent's RNG stream, so the log shape is reproducible
//! across CI runs.

mod common;

use std::time::Duration;

use common::SpawnedServer;
use futures::StreamExt;
use playtest_api::{
    ApiResponse, CreateRunRequest, GameSummary, RunStatus, RunSummary, SubmitActionBody,
    SubmitActionResponse,
};
use serde_json::Value as JsonValue;

#[tokio::test]
async fn human_vs_random_cribbage_runs_to_completion() {
    let server = SpawnedServer::start().await;
    let client = reqwest::Client::new();

    let run_id = create_run(&client, &server.base_url).await;
    let game_id = wait_for_first_game(&client, &server.base_url, &run_id).await;

    let prompts_answered = drive_game_via_http(
        &client,
        &server.base_url,
        &run_id,
        &game_id,
    )
    .await;

    assert!(
        prompts_answered >= 4,
        "Cribbage should have prompted the http-remote seat more than {prompts_answered} times"
    );

    wait_for_run_done(&client, &server.base_url, &run_id).await;

    assert_log_invariants(&server.data_dir, &run_id, &game_id).await;

    server.shutdown();
}

async fn create_run(client: &reqwest::Client, base_url: &str) -> String {
    let req = CreateRunRequest {
        game: "cribbage".into(),
        agents: vec!["http-remote".into(), "random".into()],
        games_count: 1,
        seed: Some(42),
        config: None,
    };
    let post: ApiResponse<RunSummary> = client
        .post(format!("{base_url}/api/runs"))
        .json(&req)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    post.data.expect("run created").id
}

async fn drive_game_via_http(
    client: &reqwest::Client,
    base_url: &str,
    run_id: &str,
    game_id: &str,
) -> u32 {
    let url = format!("{base_url}/api/runs/{run_id}/games/{game_id}/stream");
    let resp = client.get(&url).send().await.unwrap();
    assert!(resp.status().is_success(), "stream status {}", resp.status());
    let mut stream = resp.bytes_stream();

    let mut buf = String::new();
    let mut saw_final = false;
    let mut prompts_answered: u32 = 0;
    // Cribbage to 121 with random opponent typically takes 40-120
    // prompts for the http-remote seat. The deadline is generous.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);

    while tokio::time::Instant::now() < deadline && !saw_final {
        let next = tokio::time::timeout(Duration::from_millis(500), stream.next()).await;
        let Ok(Some(Ok(chunk))) = next else { continue };
        buf.push_str(std::str::from_utf8(&chunk).unwrap_or(""));

        while let Some(idx) = buf.find("\n\n") {
            let record: String = buf[..idx].to_owned();
            buf.drain(..idx + 2);

            let (event_name, data_line) = parse_sse_record(&record);
            match event_name.as_deref() {
                Some("turn_prompt") => {
                    answer_prompt(client, base_url, run_id, game_id, &data_line).await;
                    prompts_answered += 1;
                }
                Some("final") => saw_final = true,
                _ => {}
            }
        }
    }
    assert!(saw_final, "game did not reach `final` within deadline");
    prompts_answered
}

async fn answer_prompt(
    client: &reqwest::Client,
    base_url: &str,
    run_id: &str,
    game_id: &str,
    data_line: &str,
) {
    let prompt = parse_turn_prompt(data_line);
    let body = SubmitActionBody {
        seat: prompt.seat,
        prompt_id: prompt.prompt_id,
        action_index: 0,
    };
    let resp = client
        .post(format!(
            "{base_url}/api/runs/{run_id}/games/{game_id}/actions"
        ))
        .json(&body)
        .send()
        .await
        .unwrap();
    let status = resp.status();
    let body_text = resp.text().await.unwrap_or_default();
    assert!(
        status.is_success(),
        "submit rejected with status {status} (prompt_id={}, legal_len={}): {body_text}",
        prompt.prompt_id,
        prompt.legal_len,
    );
    let env: ApiResponse<SubmitActionResponse> = serde_json::from_str(&body_text)
        .expect("submit response is the documented envelope");
    assert!(
        env.data.is_some_and(|d| d.accepted),
        "submit response must carry accepted=true"
    );
}

async fn wait_for_run_done(client: &reqwest::Client, base_url: &str, run_id: &str) {
    for _ in 0..50 {
        let r: ApiResponse<RunSummary> = client
            .get(format!("{base_url}/api/runs/{run_id}"))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        if matches!(
            r.data.map(|s| s.status),
            Some(RunStatus::Completed | RunStatus::Failed)
        ) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(40)).await;
    }
}

async fn assert_log_invariants(
    data_dir: &std::path::Path,
    run_id: &str,
    game_id: &str,
) {
    let log_path = data_dir
        .join("runs")
        .join(run_id)
        .join(format!("{game_id}.jsonl"));
    let log = tokio::fs::read_to_string(&log_path)
        .await
        .expect("log exists");
    assert!(
        log.contains("\"kind\":\"header\""),
        "log must contain header line"
    );
    assert!(
        log.contains("\"kind\":\"final\""),
        "log must contain final line"
    );
    assert!(
        !log.contains("turn_prompt"),
        "turn_prompt must not leak into the JSONL log (Phase 2.5 invariant)"
    );
}

async fn wait_for_first_game(
    client: &reqwest::Client,
    base_url: &str,
    run_id: &str,
) -> String {
    for _ in 0..100 {
        let games: ApiResponse<Vec<GameSummary>> = client
            .get(format!("{base_url}/api/runs/{run_id}/games"))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        if let Some(g) = games.data.and_then(|v| v.into_iter().next()) {
            return g.id;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("first game never registered");
}

fn parse_sse_record(record: &str) -> (Option<String>, String) {
    let mut event_name = None;
    let mut data = String::new();
    for line in record.lines() {
        if let Some(rest) = line.strip_prefix("event:") {
            event_name = Some(rest.trim().to_owned());
        } else if let Some(rest) = line.strip_prefix("data:") {
            data.push_str(rest.trim());
        }
    }
    (event_name, data)
}

#[allow(clippy::struct_field_names)]
struct Prompt {
    seat: u8,
    prompt_id: u64,
    legal_len: usize,
}

fn parse_turn_prompt(data: &str) -> Prompt {
    let v: JsonValue = serde_json::from_str(data).expect("turn_prompt data is json");
    let seat = u8::try_from(v.get("seat").and_then(JsonValue::as_u64).expect("seat"))
        .expect("seat fits in u8");
    let prompt_id = v
        .get("prompt_id")
        .and_then(JsonValue::as_u64)
        .expect("prompt_id");
    let legal_len = v
        .get("legal_actions")
        .and_then(JsonValue::as_array)
        .map(Vec::len)
        .expect("legal_actions");
    Prompt {
        seat,
        prompt_id,
        legal_len,
    }
}
