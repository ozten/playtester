//! `playtest` binary: subcommand dispatch for `play`, `replay`, and
//! `report`.
//!
//! The CLI uses [`anyhow::Result`] at the binary boundary and
//! propagates the [`thiserror`]-based errors from the libraries
//! underneath.

use anyhow::Result;
use clap::{Parser, Subcommand};

mod agent_registry;
mod commands;
mod game_registry;

#[derive(Parser)]
#[command(
    name = "playtest",
    version,
    about = "Game playtesting harness for deterministic multi-agent simulation"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Run N games and write one JSONL event log per game.
    Play(commands::play::PlayArgs),

    /// Replay a recorded JSONL event log and print its states.
    Replay(commands::replay::ReplayArgs),

    /// Ingest a directory of event logs and write a markdown report.
    Report(commands::report::ReportArgs),
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match &cli.command {
        Command::Play(args) => commands::play::run(args),
        Command::Replay(args) => commands::replay::run(args),
        Command::Report(args) => commands::report::run(args),
    }
}
