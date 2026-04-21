//! Run-supervisor task: drives one playtester run end-to-end.
//!
//! The supervisor is a tokio task that owns the per-run lifecycle:
//!
//! 1. Creates the run's on-disk directory under `data_dir/runs/<id>/`.
//! 2. For each game (indexed 0..games_count), spawns a
//!    `spawn_blocking` task that constructs adapters, wraps the
//!    `ProductionGameEventSink` in a `BroadcastGameEventSink`, and
//!    drives the `GameLoop` synchronously. The blocking task is
//!    necessary because the engine loop is CPU-bound Rust and would
//!    starve the tokio runtime if run on an async worker.
//! 3. After each game, parses the JSONL log tail for the `Final`
//!    record, publishes a `game_finished` `RunFrame` on the run-level
//!    broadcaster, and updates the run summary.
//! 4. On completion, publishes `run_complete` and updates the watch
//!    channel to `Completed` (or `Failed` on error).

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Context;
use dashmap::DashMap;
use playtest_adapters::{BroadcastGameEventSink, ProductionFileSystem, ProductionGameEventSink};
use playtest_api::{GameSummary, RunStatus};
use playtest_registry::game_registry::RegisteredGame;
use playtest_registry::play::run_single_game_into_sink;
use tokio::sync::{broadcast, watch};
use uuid::Uuid;

use crate::state::{AppState, RunFrame, RunHandle};

/// Parameters for spawning a run.
pub struct RunSpec {
    pub run_id: Uuid,
    pub game: RegisteredGame,
    pub game_name: String,
    pub agent_names: Vec<String>,
    pub games_count: u32,
    pub seed: u64,
}

/// Capacity of every broadcast channel created by the server. 1024 is
/// ample for live SSE fan-out at Cribbage's event rate (a full game
/// emits ~200 events).
pub const BROADCAST_CAPACITY: usize = 1024;

/// Spawn the run-supervisor task and register a [`RunHandle`] in
/// `state`. Returns the initial status receiver so the caller can
/// observe early lifecycle transitions.
#[must_use]
pub fn spawn(state: &AppState, spec: RunSpec) -> watch::Receiver<RunStatus> {
    let (status_tx, status_rx) = watch::channel(RunStatus::Pending);
    let (run_broadcaster, _) = broadcast::channel::<RunFrame>(BROADCAST_CAPACITY);

    let handle = RunHandle {
        summary: std::sync::RwLock::new(playtest_api::RunSummary {
            id: spec.run_id.to_string(),
            game: spec.game_name.clone(),
            agents: spec.agent_names.clone(),
            games_count: spec.games_count,
            games_completed: 0,
            seed: spec.seed,
            status: RunStatus::Pending,
            created_at: now_ms(),
            finished_at: None,
        }),
        status_rx: status_rx.clone(),
        run_broadcaster: run_broadcaster.clone(),
        game_broadcasters: DashMap::new(),
        games: DashMap::new(),
        games_requested: spec.games_count,
    };

    state.active_runs.insert(spec.run_id, handle);

    let run_dir = state.run_dir(spec.run_id);
    let shutdown_rx = state.shutdown.subscribe();
    let active_runs = state.active_runs.clone();

    tokio::spawn(async move {
        if let Err(err) = supervise(
            spec,
            run_dir,
            status_tx,
            run_broadcaster,
            active_runs,
            shutdown_rx,
        )
        .await
        {
            tracing::error!(%err, "run supervisor failed");
        }
    });

    status_rx
}

async fn supervise(
    spec: RunSpec,
    run_dir: PathBuf,
    status_tx: watch::Sender<RunStatus>,
    run_broadcaster: broadcast::Sender<RunFrame>,
    active_runs: Arc<DashMap<Uuid, RunHandle>>,
    mut shutdown_rx: broadcast::Receiver<()>,
) -> anyhow::Result<()> {
    tokio::fs::create_dir_all(&run_dir)
        .await
        .with_context(|| format!("creating run dir {}", run_dir.display()))?;

    let _ = status_tx.send(RunStatus::Running);
    mutate_summary(&active_runs, spec.run_id, |s| s.status = RunStatus::Running);

    let game = Arc::new(spec.game);
    let agent_names = Arc::new(spec.agent_names.clone());

    for idx in 0..spec.games_count {
        if shutdown_rx.try_recv().is_ok() {
            tracing::info!(run_id = %spec.run_id, "shutdown requested; aborting run");
            break;
        }

        let ctx = PerGameCtx {
            run_id: spec.run_id,
            game_name: spec.game_name.clone(),
            game_id: format!("game-{idx:04}"),
            out_path: run_dir.join(format!("game-{idx:04}.jsonl")),
            per_game_seed: spec.seed.wrapping_add(u64::from(idx)),
            idx,
        };

        if let Err(err) = run_one_game(
            &ctx,
            game.clone(),
            agent_names.clone(),
            &run_broadcaster,
            &active_runs,
        )
        .await
        {
            tracing::error!(%err, run_id = %spec.run_id, game_id = %ctx.game_id, "game failed");
            let _ = status_tx.send(RunStatus::Failed);
            mutate_summary(&active_runs, spec.run_id, |s| {
                s.status = RunStatus::Failed;
                s.finished_at = Some(now_ms());
            });
            return Err(err);
        }
    }

    let _ = run_broadcaster.send(RunFrame::RunComplete);
    let _ = status_tx.send(RunStatus::Completed);
    mutate_summary(&active_runs, spec.run_id, |s| {
        s.status = RunStatus::Completed;
        s.finished_at = Some(now_ms());
    });
    Ok(())
}

struct PerGameCtx {
    run_id: Uuid,
    game_name: String,
    game_id: String,
    out_path: PathBuf,
    per_game_seed: u64,
    idx: u32,
}

async fn run_one_game(
    ctx: &PerGameCtx,
    game: Arc<RegisteredGame>,
    agent_names: Arc<Vec<String>>,
    run_broadcaster: &broadcast::Sender<RunFrame>,
    active_runs: &Arc<DashMap<Uuid, RunHandle>>,
) -> anyhow::Result<()> {
    let started_at = now_ms();

    // Create and register the per-game broadcaster *before* the
    // engine starts emitting, so any subscriber who connects while
    // the game is running can find it.
    let (game_tx, _) = broadcast::channel::<String>(BROADCAST_CAPACITY);
    if let Some(entry) = active_runs.get(&ctx.run_id) {
        entry
            .game_broadcasters
            .insert(ctx.game_id.clone(), game_tx.clone());
        entry.games.insert(
            ctx.game_id.clone(),
            GameSummary {
                id: ctx.game_id.clone(),
                run_id: Some(ctx.run_id.to_string()),
                game: ctx.game_name.clone(),
                started_at,
                finished_at: None,
                winner: None,
            },
        );
    }

    let _ = run_broadcaster.send(RunFrame::GameStarted {
        game_id: ctx.game_id.clone(),
    });

    let out_path = ctx.out_path.clone();
    let game_tx_cloned = game_tx.clone();
    let per_game_seed = ctx.per_game_seed;

    let join = tokio::task::spawn_blocking(move || -> anyhow::Result<()> {
        let fs = ProductionFileSystem::new();
        let inner = ProductionGameEventSink::new(fs, out_path);
        let mut sink = BroadcastGameEventSink::new(inner, game_tx_cloned);
        run_single_game_into_sink(
            game.as_ref(),
            agent_names.as_ref(),
            per_game_seed,
            None,
            &mut sink,
        )?;
        Ok(())
    });

    join.await.context("spawn_blocking join failed")??;

    let (winner, scores) = read_final_winner_scores(&ctx.out_path)
        .await
        .unwrap_or((None, Vec::new()));
    if let Some(entry) = active_runs.get(&ctx.run_id)
        && let Some(mut s) = entry.games.get_mut(&ctx.game_id)
    {
        s.finished_at = Some(now_ms());
        s.winner = winner;
    }
    mutate_summary(active_runs, ctx.run_id, |s| {
        s.games_completed = ctx.idx + 1;
    });

    let _ = run_broadcaster.send(RunFrame::GameFinished {
        game_id: ctx.game_id.clone(),
        winner,
        scores,
    });

    // Dropping the per-game broadcaster closes any live SSE streams
    // for this game, which is the documented "game done" signal to
    // clients.
    drop(game_tx);
    if let Some(entry) = active_runs.get(&ctx.run_id) {
        entry.game_broadcasters.remove(&ctx.game_id);
    }
    Ok(())
}

fn mutate_summary(
    active_runs: &DashMap<Uuid, RunHandle>,
    run_id: Uuid,
    f: impl FnOnce(&mut playtest_api::RunSummary),
) {
    if let Some(entry) = active_runs.get(&run_id)
        && let Ok(mut s) = entry.summary.write()
    {
        f(&mut s);
    }
}

async fn read_final_winner_scores(path: &std::path::Path) -> Option<(Option<u32>, Vec<i32>)> {
    let bytes = tokio::fs::read(path).await.ok()?;
    let text = std::str::from_utf8(&bytes).ok()?;
    for line in text.lines().rev() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let v: serde_json::Value = serde_json::from_str(line).ok()?;
        if v.get("kind").and_then(serde_json::Value::as_str) == Some("final") {
            let winner = v.get("winner").and_then(serde_json::Value::as_u64).map(|n| u32::try_from(n).unwrap_or(u32::MAX));
            let scores = v
                .get("scores")
                .and_then(serde_json::Value::as_array)
                .map(|arr| {
                    arr.iter()
                        .map(|n| i32::try_from(n.as_i64().unwrap_or(0)).unwrap_or(0))
                        .collect()
                })
                .unwrap_or_default();
            return Some((winner, scores));
        }
    }
    None
}

fn now_ms() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}
