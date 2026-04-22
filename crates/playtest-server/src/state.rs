//! Shared server state: a map of active runs, each with its live
//! broadcast channel and durable metadata.
//!
//! `AppState` is the axum handler's entry point into the running
//! engine. It is cloned (via `Arc`) into every route; mutations to the
//! active-runs map go through `DashMap` which is sharded-lock-free.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

use dashmap::DashMap;
use playtest_api::RunSummary;
use serde::{Deserialize, Serialize};
use tokio::sync::{broadcast, watch};
use uuid::Uuid;

use crate::turn_coordinator::TurnCoordinator;

/// Top-level server state. Cheap to `clone`; internally reference-counted.
#[derive(Debug, Clone)]
pub struct AppState {
    pub data_dir: PathBuf,
    pub active_runs: Arc<DashMap<Uuid, RunHandle>>,
    pub shutdown: broadcast::Sender<()>,
}

impl AppState {
    #[must_use]
    pub fn new(data_dir: PathBuf, shutdown: broadcast::Sender<()>) -> Self {
        Self {
            data_dir,
            active_runs: Arc::new(DashMap::new()),
            shutdown,
        }
    }

    /// Directory holding the per-game JSONL logs for a run.
    #[must_use]
    pub fn run_dir(&self, run_id: Uuid) -> PathBuf {
        self.data_dir.join("runs").join(run_id.to_string())
    }
}

/// Live handle to a single run. Registered in [`AppState::active_runs`]
/// when the run is created and kept for the lifetime of the server —
/// completed runs remain queryable until the process exits.
#[derive(Debug)]
pub struct RunHandle {
    /// Snapshot of the run's metadata at creation time, plus fields
    /// that mutate (e.g. status) via the `status_rx` watch channel.
    pub summary: std::sync::RwLock<RunSummary>,

    /// Latest lifecycle status, published by the run-supervisor task.
    pub status_rx: watch::Receiver<playtest_api::RunStatus>,

    /// Run-level SSE fan-out — emits [`RunFrame`] JSON strings as
    /// games start/finish and when the run completes.
    pub run_broadcaster: broadcast::Sender<RunFrame>,

    /// Per-game broadcasters, registered as each game starts. Keyed
    /// by the stable `game-NNNN` id used in log filenames.
    pub game_broadcasters: DashMap<String, broadcast::Sender<String>>,

    /// Per-game `TurnCoordinator`s, registered for games that have at
    /// least one `http-remote` seat. Absent for AI-only games. Keyed
    /// the same way as `game_broadcasters`.
    pub turn_coordinators: DashMap<String, Arc<TurnCoordinator>>,

    /// Summaries of games seen so far, keyed by stable game id.
    pub games: DashMap<String, playtest_api::GameSummary>,

    /// Total games requested for the run (mirrors `summary.games_count`
    /// but avoids the lock for hot-path reads).
    pub games_requested: u32,
}

impl RunHandle {
    /// Snapshot the run-level games list in stable id-sorted order.
    #[must_use]
    pub fn games_snapshot(&self) -> Vec<playtest_api::GameSummary> {
        let mut out = BTreeMap::new();
        for e in &self.games {
            out.insert(e.key().clone(), e.value().clone());
        }
        out.into_values().collect()
    }
}

/// Event on the run-level SSE stream.
///
/// This is intentionally separate from `playtest_api::SseFrame` which
/// is scoped to per-game streams. Keeping the shape small (tag + data)
/// avoids dragging a richer game-event type across crates.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", content = "data", rename_all = "snake_case")]
pub enum RunFrame {
    GameStarted {
        game_id: String,
    },
    GameFinished {
        game_id: String,
        winner: Option<u32>,
        scores: Vec<i32>,
    },
    RunComplete,
}
