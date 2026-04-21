//! `playtest replay` — reconstruct and print the tick-by-tick state of
//! a recorded game log.

use std::io::BufRead;
use std::path::PathBuf;

use anyhow::{Context, Result, anyhow, bail};
use clap::Args as ClapArgs;
use playtest_cribbage::{CribbageConfig, CribbageGame};

/// CLI flags for the `replay` subcommand.
#[derive(Debug, ClapArgs)]
pub struct ReplayArgs {
    /// Path to a JSONL event log produced by `playtest play`.
    pub path: PathBuf,

    /// Print only the state at this tick (1-based). When unset, every
    /// tick's state is printed.
    #[arg(long)]
    pub tick: Option<u64>,
}

/// Run the `replay` command.
///
/// # Errors
/// Returns an error if the log is missing, malformed, references an
/// unknown game, or replay-validation fails (schema/config/tick
/// mismatch).
pub fn run(args: &ReplayArgs) -> Result<()> {
    let game_name = peek_game_name(&args.path)?;
    match game_name.as_str() {
        "cribbage" => replay_cribbage(&args.path, args.tick),
        other => bail!("unknown game in log header: {other}"),
    }
}

/// Read just the header line and extract the `game` field without
/// knowing the event schema. Lets us dispatch to the right typed
/// replay path.
fn peek_game_name(path: &std::path::Path) -> Result<String> {
    let file = std::fs::File::open(path).with_context(|| format!("opening {}", path.display()))?;
    let reader = std::io::BufReader::new(file);
    let first = reader
        .lines()
        .next()
        .ok_or_else(|| anyhow!("log {} is empty", path.display()))??;
    let v: serde_json::Value = serde_json::from_str(&first)
        .with_context(|| format!("parsing header line of {}", path.display()))?;
    let name = v
        .get("game")
        .and_then(|g| g.as_str())
        .ok_or_else(|| anyhow!("log header has no `game` field: {first}"))?;
    Ok(name.to_owned())
}

fn replay_cribbage(path: &std::path::Path, tick_filter: Option<u64>) -> Result<()> {
    let cfg = CribbageConfig;
    let game = CribbageGame::new();
    let replayed = playtest_log::replay::<CribbageGame>(&game, CribbageGame::NAME, &cfg, path)?;

    println!(
        "game: {} version: {} seed: {} agents: {:?}",
        replayed.header.game, replayed.header.version, replayed.header.seed, replayed.header.agents,
    );
    println!(
        "started_at: {} config_hash: {}",
        replayed.header.started_at, replayed.header.config_hash
    );
    println!("ticks recorded: {}", replayed.snapshots.len());

    if let Some(t) = tick_filter {
        if t == 0 {
            bail!("tick 0 is the initial state before any event; pass a tick 1..=N");
        }
        let idx = usize::try_from(t - 1).map_err(|_| anyhow!("tick {t} too large to index"))?;
        let snap = replayed.snapshots.get(idx).ok_or_else(|| {
            anyhow!(
                "tick {t} out of range (log has {} ticks)",
                replayed.snapshots.len()
            )
        })?;
        println!("--- state at tick {t} ---");
        println!("{snap:#?}");
    } else {
        for (i, snap) in replayed.snapshots.iter().enumerate() {
            println!("--- tick {} ---", i + 1);
            println!("{snap:#?}");
        }
    }

    if let Some(result) = replayed.result {
        println!("--- final ---");
        println!(
            "winner: {:?} reason: {:?} scores: {:?}",
            result.winner, result.reason, result.scores
        );
    } else {
        println!("--- log truncated (no final record) ---");
    }

    Ok(())
}
