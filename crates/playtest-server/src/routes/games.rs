//! Game endpoints under a run:
//!
//! - `GET /api/runs/:id/games` — list games in a run.
//! - `GET /api/runs/:id/games/:gid` — full game metadata.
//! - `GET /api/runs/:id/games/:gid/events?offset=N&limit=M` — paginated
//!   log lines.
//! - `GET /api/runs/:id/games/:gid/stream` — live SSE stream with a
//!   `Last-Event-ID` catch-up path backed by the JSONL file.

use std::convert::Infallible;
use std::path::PathBuf;
use std::time::Duration;

use async_stream::stream;
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode, header::HeaderValue},
    response::sse::{Event, KeepAlive, Sse},
    routing::get,
};
use futures::Stream;
use playtest_api::{
    ApiError, ApiErrorCode, ApiResponse, EventPage, GameMetadata, GameSummary, LogLineDto,
    SseFrame,
};
use serde::Deserialize;
use tokio::sync::broadcast;
use uuid::Uuid;

use crate::routes::{ApiResult, api_error};
use crate::sse::line_to_sse_frame;
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/runs/{id}/games", get(list_games))
        .route("/api/runs/{id}/games/{gid}", get(get_game))
        .route("/api/runs/{id}/games/{gid}/events", get(list_events))
        .route("/api/runs/{id}/games/{gid}/stream", get(game_stream))
}

async fn list_games(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<Vec<GameSummary>> {
    let run_id = parse_run_id(&id)?;
    let handle = state.active_runs.get(&run_id).ok_or_else(|| {
        api_error(ApiError::new(
            ApiErrorCode::RunNotFound,
            format!("no run with id {id}"),
        ))
    })?;
    Ok(Json(ApiResponse::ok(handle.games_snapshot())))
}

async fn get_game(
    State(state): State<AppState>,
    Path((id, gid)): Path<(String, String)>,
) -> ApiResult<GameMetadata> {
    let run_id = parse_run_id(&id)?;
    let path = state.run_dir(run_id).join(format!("{gid}.jsonl"));

    let text = read_log_as_utf8(&path, &gid).await?;
    let (header, final_rec) = scan_header_and_final(&text)?;
    let header = header.ok_or_else(|| {
        api_error(ApiError::new(
            ApiErrorCode::Internal,
            "log missing header record",
        ))
    })?;

    let hdr = parse_header_fields(&header);
    let (finished_at, winner, scores) = parse_final_fields(final_rec.as_ref());

    let summary = GameSummary {
        id: gid,
        run_id: Some(id),
        game: hdr.game,
        started_at: hdr.started_at,
        finished_at,
        winner,
    };

    Ok(Json(ApiResponse::ok(GameMetadata {
        summary,
        schema: hdr.schema,
        version: hdr.version,
        seed: hdr.seed,
        config_hash: hdr.config_hash,
        agents: hdr.agents,
        scores,
    })))
}

type ErrResponse = (axum::http::StatusCode, Json<ApiResponse<()>>);

async fn read_log_as_utf8(
    path: &std::path::Path,
    gid: &str,
) -> Result<String, ErrResponse> {
    let bytes = tokio::fs::read(path).await.map_err(|_| {
        api_error(ApiError::new(
            ApiErrorCode::GameNotFound,
            format!("no game log for {gid}"),
        ))
    })?;
    String::from_utf8(bytes).map_err(|e| {
        api_error(ApiError::new(
            ApiErrorCode::Internal,
            format!("non-utf8 log: {e}"),
        ))
    })
}

fn scan_header_and_final(
    text: &str,
) -> Result<(Option<serde_json::Value>, Option<serde_json::Value>), ErrResponse> {
    let mut header: Option<serde_json::Value> = None;
    let mut final_rec: Option<serde_json::Value> = None;
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let v: serde_json::Value = serde_json::from_str(trimmed).map_err(|e| {
            api_error(ApiError::new(
                ApiErrorCode::Internal,
                format!("malformed log line: {e}"),
            ))
        })?;
        match v.get("kind").and_then(serde_json::Value::as_str) {
            Some("header") => header = Some(v),
            Some("final") => final_rec = Some(v),
            _ => {}
        }
    }
    Ok((header, final_rec))
}

struct ParsedHeader {
    game: String,
    schema: u32,
    version: String,
    seed: u64,
    config_hash: String,
    started_at: u64,
    agents: Vec<String>,
}

fn parse_header_fields(header: &serde_json::Value) -> ParsedHeader {
    ParsedHeader {
        game: header
            .get("game")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("unknown")
            .to_owned(),
        schema: header
            .get("schema")
            .and_then(serde_json::Value::as_u64)
            .and_then(|n| u32::try_from(n).ok())
            .unwrap_or(0),
        version: header
            .get("version")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("")
            .to_owned(),
        seed: header
            .get("seed")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0),
        config_hash: header
            .get("config_hash")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("")
            .to_owned(),
        started_at: header
            .get("started_at")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0),
        agents: header
            .get("agents")
            .and_then(serde_json::Value::as_array)
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(str::to_owned))
                    .collect()
            })
            .unwrap_or_default(),
    }
}

fn parse_final_fields(
    final_rec: Option<&serde_json::Value>,
) -> (Option<u64>, Option<u32>, Option<Vec<i32>>) {
    final_rec.map_or((None, None, None), |v| {
        let finished_at = v.get("finished_at").and_then(serde_json::Value::as_u64);
        let winner = v
            .get("winner")
            .and_then(serde_json::Value::as_u64)
            .and_then(|n| u32::try_from(n).ok());
        let scores: Vec<i32> = v
            .get("scores")
            .and_then(serde_json::Value::as_array)
            .map(|arr| {
                arr.iter()
                    .filter_map(|n| n.as_i64().and_then(|x| i32::try_from(x).ok()))
                    .collect()
            })
            .unwrap_or_default();
        (finished_at, winner, Some(scores))
    })
}

#[derive(Debug, Deserialize)]
pub struct EventsQuery {
    #[serde(default)]
    pub offset: u64,
    #[serde(default = "default_limit")]
    pub limit: u32,
}

fn default_limit() -> u32 {
    100
}

async fn list_events(
    State(state): State<AppState>,
    Path((id, gid)): Path<(String, String)>,
    Query(q): Query<EventsQuery>,
) -> ApiResult<EventPage> {
    let run_id = parse_run_id(&id)?;
    if q.limit == 0 || q.limit > 10_000 {
        return Err(api_error(ApiError::new(
            ApiErrorCode::InvalidPaginationParams,
            "limit must be 1..=10000",
        )));
    }

    let path = state.run_dir(run_id).join(format!("{gid}.jsonl"));
    let bytes = tokio::fs::read(&path).await.map_err(|_| {
        api_error(ApiError::new(
            ApiErrorCode::GameNotFound,
            format!("no game log for {gid}"),
        ))
    })?;
    let text = std::str::from_utf8(&bytes).map_err(|e| {
        api_error(ApiError::new(
            ApiErrorCode::Internal,
            format!("non-utf8 log: {e}"),
        ))
    })?;

    let all_lines: Vec<&str> = text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect();
    let total = all_lines.len() as u64;

    let start = usize::try_from(q.offset).unwrap_or(usize::MAX).min(all_lines.len());
    let end = start.saturating_add(q.limit as usize).min(all_lines.len());

    let mut events = Vec::with_capacity(end.saturating_sub(start));
    for line in &all_lines[start..end] {
        let line_json: serde_json::Value = serde_json::from_str(line).map_err(|e| {
            api_error(ApiError::new(
                ApiErrorCode::Internal,
                format!("malformed log line: {e}"),
            ))
        })?;
        let kind = line_json
            .get("kind")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("event")
            .to_owned();
        events.push(LogLineDto {
            kind,
            line: line_json,
        });
    }

    Ok(Json(ApiResponse::ok(EventPage {
        offset: q.offset,
        limit: q.limit,
        total,
        events,
    })))
}

async fn game_stream(
    State(state): State<AppState>,
    Path((id, gid)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<
    Sse<impl Stream<Item = Result<Event, Infallible>>>,
    (StatusCode, Json<ApiResponse<()>>),
> {
    let run_id = parse_run_id(&id)?;

    // Capture the run's game broadcaster *first*, before reading the
    // file tail, so any events emitted during catch-up get buffered in
    // the 1024-slot broadcast channel rather than lost.
    let subscriber: Option<broadcast::Receiver<String>> = state
        .active_runs
        .get(&run_id)
        .and_then(|h| h.game_broadcasters.get(&gid).map(|e| e.subscribe()));

    let path = state.run_dir(run_id).join(format!("{gid}.jsonl"));

    // Not-found check: either the run exists and the file is present,
    // or we return 404.
    if !path.exists()
        && state
            .active_runs
            .get(&run_id)
            .is_none_or(|h| !h.game_broadcasters.contains_key(&gid))
    {
        return Err(api_error(ApiError::new(
            ApiErrorCode::GameNotFound,
            format!("no game log for {gid}"),
        )));
    }

    let last_event_id: u64 = parse_last_event_id(&headers);

    let s = build_game_stream(path, subscriber, last_event_id).await;

    Ok(Sse::new(s).keep_alive(
        KeepAlive::new()
            .interval(Duration::from_secs(15))
            .text("heartbeat"),
    ))
}

fn parse_last_event_id(headers: &HeaderMap) -> u64 {
    headers
        .get(axum::http::header::HeaderName::from_static("last-event-id"))
        .and_then(|v: &HeaderValue| v.to_str().ok())
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(0)
}

async fn build_game_stream(
    path: PathBuf,
    subscriber: Option<broadcast::Receiver<String>>,
    last_event_id: u64,
) -> impl Stream<Item = Result<Event, Infallible>> {
    // Read the file once up front. Events emitted after this snapshot
    // are delivered via the broadcaster, and any overlap is filtered
    // out by tick id below.
    let catch_up_lines: Vec<String> = match tokio::fs::read_to_string(&path).await {
        Ok(text) => text
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .map(ToOwned::to_owned)
            .collect(),
        Err(_) => Vec::new(),
    };

    stream! {
        let mut highest_event_tick: Option<u64> = None;

        for line in &catch_up_lines {
            let Some((tick, frame)) = line_to_sse_frame(line) else {
                continue;
            };
            // Skip anything already delivered to this client.
            if let Some(t) = tick
                && last_event_id > 0
                && t <= last_event_id
            {
                continue;
            }
            if let SseFrame::Event(_) = &frame
                && let Some(t) = tick
            {
                highest_event_tick = Some(highest_event_tick.map_or(t, |m| m.max(t)));
            }
            let ev = frame_to_event(&frame, tick);
            yield Ok(ev);
        }

        // Switch to the live feed. If no broadcaster exists, the game
        // already finished and we're done.
        let Some(mut rx) = subscriber else {
            return;
        };

        loop {
            match rx.recv().await {
                Ok(line) => {
                    let Some((tick, frame)) = line_to_sse_frame(&line) else {
                        continue;
                    };
                    // Drop duplicates that were already delivered
                    // during catch-up (race between the file-read
                    // snapshot and the live feed).
                    if let SseFrame::Event(_) = &frame
                        && let Some(t) = tick
                        && let Some(high) = highest_event_tick
                        && t <= high
                    {
                        continue;
                    }
                    if let SseFrame::Event(_) = &frame
                        && let Some(t) = tick
                    {
                        highest_event_tick = Some(
                            highest_event_tick.map_or(t, |m| m.max(t)),
                        );
                    }
                    let ev = frame_to_event(&frame, tick);
                    yield Ok(ev);
                    if matches!(frame, SseFrame::Final(_)) {
                        break;
                    }
                }
                Err(broadcast::error::RecvError::Closed) => break,
                Err(broadcast::error::RecvError::Lagged(_)) => {
                    // Localhost-only scope: the 1024-slot buffer is
                    // ample. If we ever hit this, just keep going —
                    // the client sees an `id:` jump and can refetch
                    // missing events via the paginated endpoint.
                }
            }
        }
    }
}

fn frame_to_event(frame: &SseFrame, tick: Option<u64>) -> Event {
    let (event_name, data) = match frame {
        SseFrame::Header(v) => ("header", v.clone()),
        SseFrame::Event(v) => ("event", v.clone()),
        SseFrame::Final(v) => ("final", v.clone()),
        SseFrame::TurnPrompt(p) => (
            "turn_prompt",
            serde_json::to_value(p).unwrap_or(serde_json::Value::Null),
        ),
        SseFrame::Heartbeat => ("heartbeat", serde_json::Value::Null),
    };
    let mut ev = Event::default()
        .event(event_name)
        .data(serde_json::to_string(&data).unwrap_or_default());
    if let Some(t) = tick {
        ev = ev.id(t.to_string());
    }
    ev
}

fn parse_run_id(
    id: &str,
) -> Result<Uuid, (StatusCode, Json<ApiResponse<()>>)> {
    Uuid::parse_str(id).map_err(|_| {
        api_error(ApiError::new(
            ApiErrorCode::RunNotFound,
            format!("invalid run id: {id}"),
        ))
    })
}
