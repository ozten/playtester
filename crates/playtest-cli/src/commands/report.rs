//! `playtest report` — ingest a directory of JSONL event logs into
//! SQLite, run the canned reporter queries, and emit a markdown file.
//!
//! The default flow is pure: in-memory SQLite, ingest, report, write.
//! `--db <path>` swaps the in-memory database for a file; when the
//! file already exists the ingest pass is idempotent via the Unit 14
//! deterministic `game_id`, so the same report can be regenerated
//! without re-reading the event logs.
//!
//! The command dispatches on `--game` the same way `play` and
//! `replay` do — see [`playtest_registry::game_registry`]. Generic
//! sections (Summary, Per-agent) come from `playtest_metrics::reporter`;
//! game-specific sections come from the game crate.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use clap::Args as ClapArgs;
use playtest_cribbage::{CribbageGame, CribbageMetrics};
use playtest_metrics::{
    MarkdownBuilder, ingest_directory, init_schema, write_per_agent_section,
    write_subjective_critique_section, write_summary_section,
};
use playtest_registry::game_registry::{RegisteredGame, lookup as lookup_game};
use playtest_shipwreck::{ShipWreckGame, ShipWreckMetrics};
use rusqlite::Connection;

/// CLI flags for the `report` subcommand.
#[derive(Debug, ClapArgs)]
pub struct ReportArgs {
    /// Game name (e.g. `cribbage`). Reports only include games whose
    /// log header matches this name; foreign logs are skipped with a
    /// warning.
    #[arg(long)]
    pub game: String,

    /// Directory containing JSONL event logs. Each `*.jsonl` in the
    /// directory is ingested.
    #[arg(long)]
    pub games: PathBuf,

    /// Output markdown file. Overwritten on each run.
    #[arg(long)]
    pub out: PathBuf,

    /// SQLite database path. When set, the database persists across
    /// invocations; re-ingesting is idempotent. When unset, an
    /// in-memory database is used and discarded on exit.
    #[arg(long)]
    pub db: Option<PathBuf>,
}

/// Run the `report` command.
///
/// # Errors
/// Propagates ingestion, query, and filesystem errors.
pub fn run(args: &ReportArgs) -> Result<()> {
    let game = lookup_game(&args.game)?;

    let mut conn = open_connection(args.db.as_deref())?;
    init_schema(&conn).context("applying SQLite schema")?;

    let ingest_report = match &game {
        RegisteredGame::Cribbage(_) => ingest_directory::<CribbageGame, _>(
            &mut conn,
            &args.games,
            CribbageGame::NAME,
            &CribbageMetrics,
        )
        .with_context(|| format!("ingesting {}", args.games.display()))?,
        RegisteredGame::ShipWreck(_) => ingest_directory::<ShipWreckGame, _>(
            &mut conn,
            &args.games,
            ShipWreckGame::NAME,
            &ShipWreckMetrics,
        )
        .with_context(|| format!("ingesting {}", args.games.display()))?,
    };

    let report_md = build_report(&game, &conn, &ingest_report)?;
    std::fs::write(&args.out, &report_md)
        .with_context(|| format!("writing report to {}", args.out.display()))?;

    // One-liner summary to stdout so an operator can sanity-check
    // without opening the report.
    println!("{} → {}", ingest_report.summary(), args.out.display());
    Ok(())
}

fn open_connection(db: Option<&Path>) -> Result<Connection> {
    match db {
        Some(path) => {
            if let Some(parent) = path.parent()
                && !parent.as_os_str().is_empty()
                && !parent.exists()
            {
                std::fs::create_dir_all(parent)
                    .with_context(|| format!("creating parent directory {}", parent.display()))?;
            }
            Connection::open(path).with_context(|| format!("opening SQLite at {}", path.display()))
        }
        None => Connection::open_in_memory().context("opening in-memory SQLite"),
    }
}

fn build_report(
    game: &RegisteredGame,
    conn: &Connection,
    ingest_report: &playtest_metrics::IngestReport,
) -> Result<String> {
    let mut md = MarkdownBuilder::new();
    md.h1(&format!("Playtest report — {}", game_name(game)));

    md.paragraph(&format!("Source: `{}`.", ingest_report.summary()));
    if !ingest_report.errors.is_empty() {
        md.h3("Ingestion errors");
        for e in &ingest_report.errors {
            md.bullet(&format!("`{}` — {}", e.path.display(), e.reason));
        }
        md.end_block();
    }

    write_summary_section(&mut md, conn).context("writing Summary section")?;
    write_per_agent_section(&mut md, conn).context("writing Per-agent section")?;
    write_subjective_critique_section(&mut md, conn)
        .context("writing Subjective critique section")?;

    match game {
        RegisteredGame::Cribbage(_) => {
            playtest_cribbage::report::write_game_shape_section(&mut md, conn)
                .context("writing Cribbage game-shape section")?;
            playtest_cribbage::report::write_scoring_breakdown_section(&mut md, conn)
                .context("writing Cribbage scoring-breakdown section")?;
            playtest_cribbage::report::write_per_card_section(&mut md, conn)
                .context("writing Cribbage per-card section")?;
        }
        RegisteredGame::ShipWreck(_) => {
            playtest_shipwreck::report::write_game_shape_section(&mut md, conn)
                .context("writing ShipWreck game-shape section")?;
            playtest_shipwreck::report::write_per_player_section(&mut md, conn)
                .context("writing ShipWreck per-player section")?;
        }
    }

    Ok(md.into_string())
}

fn game_name(game: &RegisteredGame) -> &'static str {
    match game {
        RegisteredGame::Cribbage(_) => CribbageGame::NAME,
        RegisteredGame::ShipWreck(_) => ShipWreckGame::NAME,
    }
}
