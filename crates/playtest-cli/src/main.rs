//! `playtest` binary: subcommand dispatch for `play`, `replay`,
//! `report`, and `serve`.
//!
//! The CLI uses [`anyhow::Result`] at the binary boundary and
//! propagates the [`thiserror`]-based errors from the libraries
//! underneath.

use anyhow::Result;
use clap::{Parser, Subcommand};

mod commands;

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

    /// Start the HTTP + SSE server.
    Serve(commands::serve::ServeArgs),

    /// Dump the HTTP API's OpenAPI 3.1 spec to a file (or `-` for stdout).
    ApiSchema(commands::api_schema::ApiSchemaArgs),

    /// Round-robin matchups between a pool of agents. Emits a markdown
    /// win-rate matrix.
    Matchup(commands::matchup::MatchupArgs),

    /// Phase 5: offline coder pass over `.critique.jsonl` sidecars.
    /// Extracts structured tags from open-ended responses using a
    /// second LLM call per seat.
    CritiqueCode(commands::critique_code::CritiqueCodeArgs),

    /// Phase 6: diff two ingested log directories, flag statistically
    /// significant metric and critique deltas, emit a markdown "what
    /// changed" report.
    Compare(commands::compare::CompareArgs),
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match &cli.command {
        Command::Play(args) => commands::play::run(args),
        Command::Replay(args) => commands::replay::run(args),
        Command::Report(args) => commands::report::run(args),
        Command::Serve(args) => commands::serve::run(args),
        Command::ApiSchema(args) => commands::api_schema::run(args),
        Command::Matchup(args) => commands::matchup::run(args),
        Command::CritiqueCode(args) => commands::critique_code::run(args),
        Command::Compare(args) => commands::compare::run(args),
    }
}
