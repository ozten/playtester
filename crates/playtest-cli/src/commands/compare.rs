//! `playtest compare` — diff two ingested log directories via Welch /
//! two-proportion tests with Benjamini–Hochberg (default) or
//! Bonferroni multiple-comparison correction. Writes a "what changed"
//! markdown report.
//!
//! The subcommand is a pure reader: both dirs are ingested into
//! separate in-memory SQLite DBs, the compare engine runs, and the
//! result is rendered via the Phase-6 markdown reporter.
//!
//! Game-name dispatch follows the same pattern as `playtest report`
//! so foreign logs in either dir are skipped with a warning.

use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::{Args as ClapArgs, ValueEnum};
use playtest_cribbage::{CribbageGame, CribbageMetrics};
use playtest_metrics::{
    CompareOpts, Correction, MarkdownBuilder, ingest_directory, init_schema, run_compare,
    write_compare_report,
};
use playtest_registry::game_registry::{RegisteredGame, lookup as lookup_game};
use playtest_shipwreck::{ShipWreckGame, ShipWreckMetrics};
use rusqlite::Connection;

/// CLI flags for the `compare` subcommand.
#[derive(Debug, ClapArgs)]
pub struct CompareArgs {
    /// Game name (e.g. `cribbage`). Both dirs' logs must match.
    #[arg(long)]
    pub game: String,

    /// Baseline directory of JSONL event logs.
    #[arg(long)]
    pub baseline: PathBuf,

    /// Variant directory of JSONL event logs.
    #[arg(long)]
    pub variant: PathBuf,

    /// Output markdown file. Overwritten on each run.
    #[arg(long)]
    pub out: PathBuf,

    /// Significance threshold. Default 0.05.
    #[arg(long, default_value_t = 0.05)]
    pub alpha: f64,

    /// Multiple-comparison correction method. Default `bh`
    /// (Benjamini–Hochberg FDR).
    #[arg(long, value_enum, default_value_t = CorrectionArg::Bh)]
    pub correction: CorrectionArg,
}

/// Clap-facing mirror of [`Correction`]. Clap's `value_enum` needs
/// its own type so the help text reads naturally (`bh` / `bonferroni`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum CorrectionArg {
    /// Benjamini–Hochberg FDR correction (default).
    Bh,
    /// Bonferroni correction.
    Bonferroni,
}

impl From<CorrectionArg> for Correction {
    fn from(c: CorrectionArg) -> Self {
        match c {
            CorrectionArg::Bh => Self::BenjaminiHochberg,
            CorrectionArg::Bonferroni => Self::Bonferroni,
        }
    }
}

/// Run `playtest compare`.
///
/// # Errors
/// Propagates ingestion, query, and filesystem errors.
pub fn run(args: &CompareArgs) -> Result<()> {
    let game = lookup_game(&args.game)?;

    let mut baseline_conn = Connection::open_in_memory()
        .context("opening baseline in-memory SQLite")?;
    let mut variant_conn = Connection::open_in_memory()
        .context("opening variant in-memory SQLite")?;
    init_schema(&baseline_conn).context("applying baseline schema")?;
    init_schema(&variant_conn).context("applying variant schema")?;

    let baseline_report = ingest_dir(&game, &mut baseline_conn, &args.baseline)
        .with_context(|| format!("ingesting baseline {}", args.baseline.display()))?;
    let variant_report = ingest_dir(&game, &mut variant_conn, &args.variant)
        .with_context(|| format!("ingesting variant {}", args.variant.display()))?;

    let opts = CompareOpts {
        alpha: args.alpha,
        correction: args.correction.into(),
    };
    let result =
        run_compare(&baseline_conn, &variant_conn, &opts).context("running compare")?;

    let mut md = MarkdownBuilder::new();
    write_compare_report(&mut md, &result);
    std::fs::write(&args.out, md.into_string())
        .with_context(|| format!("writing report to {}", args.out.display()))?;

    println!(
        "baseline: {} | variant: {} → {} ({} flagged, {} rejected, {} unchanged)",
        baseline_report.summary(),
        variant_report.summary(),
        args.out.display(),
        result.flagged.len(),
        result.rejected.len(),
        result.unchanged.len()
    );
    Ok(())
}

fn ingest_dir(
    game: &RegisteredGame,
    conn: &mut Connection,
    dir: &std::path::Path,
) -> Result<playtest_metrics::IngestReport> {
    Ok(match game {
        RegisteredGame::Cribbage(_) => ingest_directory::<CribbageGame, _>(
            conn,
            dir,
            CribbageGame::NAME,
            &CribbageMetrics,
        )?,
        RegisteredGame::ShipWreck(_) => ingest_directory::<ShipWreckGame, _>(
            conn,
            dir,
            ShipWreckGame::NAME,
            &ShipWreckMetrics,
        )?,
    })
}
