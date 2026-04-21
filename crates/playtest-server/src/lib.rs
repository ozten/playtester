//! `playtest-server` — axum HTTP + SSE frontend for the playtester
//! engine.
//!
//! The server exposes a REST surface plus Server-Sent Events streams
//! for each live game; the wire types live in [`playtest_api`]. Run
//! supervisors dispatch to the same game + agent registries that the
//! `playtest` CLI uses (re-exported from `playtest_cli`), so the HTTP
//! and CLI paths share one canonical lookup.
//!
//! # Architectural shape
//!
//! - [`ServerConfig`] carries the bind socket and the on-disk data
//!   directory where run logs get written.
//! - [`run`] builds an axum app and runs it until `ctrl_c` is
//!   received. Active runs are tracked in an [`state::AppState`]; each
//!   run has a dedicated `tokio::sync::broadcast` fan-out for SSE
//!   subscribers.
//! - No game-specific code lives in this crate. Game crates are only
//!   reachable through `playtest_cli`'s static registry.
//!
//! # Localhost default
//!
//! [`ServerConfig::default_bind`] is `127.0.0.1:7878`. Binding on a
//! non-loopback address emits a WARN log; the server does not refuse
//! to do so, but it is an opt-in posture.

#![allow(clippy::missing_errors_doc)]

use std::net::SocketAddr;
use std::path::PathBuf;

use anyhow::{Context, Result};
use tokio::sync::broadcast;

pub mod routes;
pub mod runner;
pub mod schema;
pub mod sse;
pub mod state;

pub use schema::openapi_json;

use state::AppState;

/// Configuration for the HTTP server.
#[derive(Debug, Clone)]
pub struct ServerConfig {
    /// Address the axum server should bind to.
    pub bind: SocketAddr,

    /// Root of the on-disk data directory. Each run writes its
    /// per-game JSONL logs to `<data_dir>/runs/<run_id>/game-<n>.jsonl`.
    pub data_dir: PathBuf,
}

impl ServerConfig {
    /// Default loopback bind address used by the CLI.
    #[must_use]
    pub fn default_bind() -> SocketAddr {
        SocketAddr::from(([127, 0, 0, 1], 7878))
    }
}

/// Run the HTTP server until `ctrl_c` is received.
///
/// This function performs the bind, constructs the axum router, and
/// installs a graceful-shutdown hook. It returns once the server has
/// stopped accepting new connections and all in-flight requests have
/// completed (or the shutdown timeout elapses).
pub async fn run(cfg: ServerConfig) -> Result<()> {
    if !cfg.bind.ip().is_loopback() {
        tracing::warn!(
            bind = %cfg.bind,
            "server bound to a non-loopback address; the playtester API is \
             unauthenticated and is intended for localhost use only"
        );
    }

    tokio::fs::create_dir_all(cfg.data_dir.join("runs"))
        .await
        .with_context(|| format!("creating data dir {}", cfg.data_dir.display()))?;

    // Shared broadcast channel used to signal graceful shutdown to
    // every spawned run-supervisor task.
    let (shutdown_tx, _shutdown_rx) = broadcast::channel::<()>(1);

    let state = AppState::new(cfg.data_dir.clone(), shutdown_tx.clone());
    let app = routes::build_router(state.clone());

    let listener = tokio::net::TcpListener::bind(cfg.bind)
        .await
        .with_context(|| format!("binding {}", cfg.bind))?;
    let local_addr = listener.local_addr().unwrap_or(cfg.bind);
    tracing::info!(bind = %local_addr, "playtest-server listening");

    let shutdown_signal = async move {
        let _ = tokio::signal::ctrl_c().await;
        tracing::info!("ctrl_c received; starting graceful shutdown");
        let _ = shutdown_tx.send(());
    };

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal)
        .await
        .context("axum serve loop exited with error")?;

    Ok(())
}
