//! `playtest serve` — start the HTTP + SSE server.
//!
//! Thin wrapper that translates CLI flags into a `ServerConfig` and
//! hands off to [`playtest_server::run`] inside a tokio runtime.

use std::net::SocketAddr;
use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::Args as ClapArgs;
use playtest_server::ServerConfig;

#[derive(Debug, ClapArgs)]
pub struct ServeArgs {
    /// Socket to bind. Defaults to loopback-only on port 7878.
    #[arg(long, default_value = "127.0.0.1:7878")]
    pub bind: SocketAddr,

    /// Root of the on-disk data directory. Created if missing.
    #[arg(long, default_value = "./playtest-data")]
    pub data_dir: PathBuf,
}

pub fn run(args: &ServeArgs) -> Result<()> {
    if !args.bind.ip().is_loopback() {
        eprintln!(
            "WARN: binding to non-loopback address {}; the API is unauthenticated \
             and is intended for localhost use",
            args.bind
        );
    }

    let cfg = ServerConfig {
        bind: args.bind,
        data_dir: args.data_dir.clone(),
    };

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("building tokio runtime")?;
    rt.block_on(playtest_server::run(cfg))
}
