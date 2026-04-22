//! Run-lifecycle endpoints:
//!
//! - `POST /api/runs` — create a run (spawns a supervisor task).
//! - `GET /api/runs` — list active + completed runs.
//! - `GET /api/runs/:id` — fetch a run's summary.
//! - `GET /api/runs/:id/stream` — SSE stream of run-level events
//!   (game start/finish + run-complete).

use std::convert::Infallible;
use std::time::Duration;

use async_stream::stream;
use axum::{
    Json, Router,
    extract::{Path, State},
    response::sse::{Event, KeepAlive, Sse},
    routing::{get, post},
};
use futures::Stream;
use playtest_api::{
    ApiError, ApiErrorCode, ApiResponse, CreateRunRequest, RunStatus, RunSummary,
};
use playtest_registry::agent_registry::{KNOWN_AGENTS, is_known_agent, split_agent_spec};
use playtest_registry::game_registry::{KNOWN_GAMES, lookup as lookup_game};
use uuid::Uuid;

use crate::routes::{ApiResult, api_error};
use crate::runner::{self, RunSpec};
use crate::state::{AppState, RunFrame};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/runs", post(create_run).get(list_runs))
        .route("/api/runs/{id}", get(get_run))
        .route("/api/runs/{id}/stream", get(run_stream))
}

async fn create_run(
    State(state): State<AppState>,
    Json(req): Json<CreateRunRequest>,
) -> ApiResult<RunSummary> {
    // Validate game id.
    let game = lookup_game(&req.game).map_err(|_| {
        api_error(ApiError::with_details(
            ApiErrorCode::UnknownGame,
            format!("unknown game: {}", req.game),
            serde_json::json!({"game": req.game, "known": KNOWN_GAMES}),
        ))
    })?;

    // Validate each agent name. Accept parameterized forms like
    // `"<agent-name>:key1=v1,key2=v2"` — `is_known_agent` splits
    // off the `:params` suffix before checking.
    for name in &req.agents {
        if !is_known_agent(name) {
            return Err(api_error(ApiError::with_details(
                ApiErrorCode::UnknownAgent,
                format!("unknown agent: {name}"),
                serde_json::json!({"agent": name, "known": KNOWN_AGENTS}),
            )));
        }
        // Reject CLI-only kinds at the HTTP boundary. Both `llm` and
        // `stdio` require caller-supplied dependencies (API key, child
        // process path) that the HTTP server deliberately does not
        // accept over the wire — Phase 3 keeps these as CLI-only kinds.
        let (base, _params) = split_agent_spec(name);
        if base == "llm" || base == "stdio" {
            return Err(api_error(ApiError::with_details(
                ApiErrorCode::AgentKindNotAllowedHere,
                format!(
                    "agent kind '{base}' is CLI-only; use `playtest play --agents ...` from \
                     the command line"
                ),
                serde_json::json!({"agent": name, "kind": base}),
            )));
        }
    }

    // Enforce 2-player constraint at the route boundary (same check
    // the CLI's `play` command makes).
    if req.agents.len() != 2 {
        return Err(api_error(ApiError::with_details(
            ApiErrorCode::InvalidConfig,
            format!(
                "expected 2 agents (2-player only for current phase), got {}",
                req.agents.len()
            ),
            serde_json::json!({"agents_count": req.agents.len()}),
        )));
    }

    if req.games_count == 0 {
        return Err(api_error(ApiError::new(
            ApiErrorCode::InvalidConfig,
            "games_count must be >= 1",
        )));
    }

    let run_id = Uuid::new_v4();
    let seed = req.seed.unwrap_or_else(rand_seed);

    let _status_rx = runner::spawn(
        &state,
        RunSpec {
            run_id,
            game,
            game_name: req.game.clone(),
            agent_names: req.agents.clone(),
            games_count: req.games_count,
            seed,
        },
    );

    // Read back the registered summary so the caller sees exactly
    // what subsequent `GET /api/runs/:id` calls will return.
    let summary = state
        .active_runs
        .get(&run_id)
        .and_then(|h| h.summary.read().ok().map(|s| s.clone()))
        .ok_or_else(|| {
            api_error(ApiError::new(
                ApiErrorCode::Internal,
                "run registered but summary unavailable",
            ))
        })?;

    Ok(Json(ApiResponse::ok(summary)))
}

async fn list_runs(State(state): State<AppState>) -> Json<ApiResponse<Vec<RunSummary>>> {
    let mut rows: Vec<RunSummary> = state
        .active_runs
        .iter()
        .filter_map(|e| e.summary.read().ok().map(|s| s.clone()))
        .collect();
    rows.sort_by_key(|r| std::cmp::Reverse(r.created_at));
    Json(ApiResponse::ok(rows))
}

async fn get_run(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<RunSummary> {
    let run_id = parse_run_id(&id)?;
    let summary = state
        .active_runs
        .get(&run_id)
        .and_then(|h| h.summary.read().ok().map(|s| s.clone()))
        .ok_or_else(|| {
            api_error(ApiError::new(
                ApiErrorCode::RunNotFound,
                format!("no run with id {id}"),
            ))
        })?;
    Ok(Json(ApiResponse::ok(summary)))
}

async fn run_stream(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<
    Sse<impl Stream<Item = Result<Event, Infallible>>>,
    (axum::http::StatusCode, Json<ApiResponse<()>>),
> {
    let run_id = parse_run_id(&id)?;
    let (mut rx, status_rx) = {
        let handle = state.active_runs.get(&run_id).ok_or_else(|| {
            api_error(ApiError::new(
                ApiErrorCode::RunNotFound,
                format!("no run with id {id}"),
            ))
        })?;
        (handle.run_broadcaster.subscribe(), handle.status_rx.clone())
    };

    // If the run is already `Completed` at subscribe time, replay the
    // known games as `GameStarted`+`GameFinished` frames for catch-up
    // and then close. For still-running runs, drop straight into the
    // live subscription.
    let initial_frames = if matches!(*status_rx.borrow(), RunStatus::Completed | RunStatus::Failed)
    {
        let mut out = Vec::new();
        if let Some(handle) = state.active_runs.get(&run_id) {
            for g in handle.games_snapshot() {
                out.push(RunFrame::GameStarted {
                    game_id: g.id.clone(),
                });
                out.push(RunFrame::GameFinished {
                    game_id: g.id,
                    winner: g.winner,
                    scores: Vec::new(),
                });
            }
            out.push(RunFrame::RunComplete);
        }
        out
    } else {
        Vec::new()
    };

    let live_done = matches!(
        *status_rx.borrow(),
        RunStatus::Completed | RunStatus::Failed
    );

    let s = stream! {
        for frame in initial_frames {
            let line = serde_json::to_string(&frame).unwrap_or_default();
            let ev = Event::default().event(frame_kind(&frame)).data(line);
            yield Ok(ev);
        }
        if live_done {
            return;
        }
        while let Ok(frame) = rx.recv().await {
            let line = serde_json::to_string(&frame).unwrap_or_default();
            let ev = Event::default().event(frame_kind(&frame)).data(line);
            yield Ok(ev);
            if matches!(frame, RunFrame::RunComplete) {
                break;
            }
        }
    };

    Ok(Sse::new(s).keep_alive(
        KeepAlive::new()
            .interval(Duration::from_secs(15))
            .text("heartbeat"),
    ))
}

fn frame_kind(frame: &RunFrame) -> &'static str {
    match frame {
        RunFrame::GameStarted { .. } => "game_started",
        RunFrame::GameFinished { .. } => "game_finished",
        RunFrame::RunComplete => "run_complete",
    }
}

fn parse_run_id(
    id: &str,
) -> Result<Uuid, (axum::http::StatusCode, Json<ApiResponse<()>>)> {
    Uuid::parse_str(id).map_err(|_| {
        api_error(ApiError::new(
            ApiErrorCode::RunNotFound,
            format!("invalid run id: {id}"),
        ))
    })
}

fn rand_seed() -> u64 {
    // Small helper: UUIDv4's random bytes are a fine seed source and
    // we already depend on `uuid`, so pull 8 bytes from a fresh
    // `Uuid::new_v4()` rather than adding a direct `rand` dep here.
    let u = Uuid::new_v4();
    let bytes = u.as_bytes();
    u64::from_le_bytes([
        bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
    ])
}
