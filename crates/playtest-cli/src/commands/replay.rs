//! `playtest replay` — reconstruct and print the tick-by-tick state of
//! a recorded game log.

use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};
use clap::Args as ClapArgs;
use playtest_core::{Game, PlayerId};
use playtest_cribbage::{CribbageConfig, CribbageGame};
use playtest_greatgyre::{GreatGyreConfig, GreatGyreGame};
use playtest_log::LogHeader;
use playtest_shipwreck::{ShipWreckConfig, ShipWreckGame};
use serde::Serialize;

/// CLI flags for the `replay` subcommand.
#[derive(Debug, ClapArgs)]
pub struct ReplayArgs {
    /// Path to a JSONL event log produced by `playtest play`.
    pub path: PathBuf,

    /// Print only the state at this tick (1-based). When unset, every
    /// tick's state is printed.
    #[arg(long)]
    pub tick: Option<u64>,

    /// Seat (0-based `PlayerId`) to compute `Game::public_view` from
    /// for the per-tick view dump. Must be paired with
    /// `--public-view-out`. This is the L0 QA oracle: one redacted
    /// `PublicView` per tick, exactly what a real client for that seat
    /// is entitled to know at that point in the game.
    #[arg(long)]
    pub public_view_observer: Option<PlayerId>,

    /// Path to write the public-view dump: one JSON line per tick,
    /// `{"tick": N, "view": <PublicView>}`, `N` matching the same
    /// 1-based tick numbering as `--tick`. Requires
    /// `--public-view-observer`.
    #[arg(long)]
    pub public_view_out: Option<PathBuf>,
}

/// Write one JSON line per snapshot: `{"tick": N, "view": <PublicView>}`,
/// `N` 1-based (snapshot index 0 is the state after tick 1's event).
/// Generic over any `Game` whose `PublicView` serializes — every
/// registered game's view type does, so this is shared across the
/// three `replay_<game>` functions below rather than duplicated per
/// game.
fn write_public_view_dump<G>(
    game: &G,
    snapshots: &[G::State],
    observer: PlayerId,
    out_path: &Path,
) -> Result<()>
where
    G: Game,
    G::PublicView: Serialize,
{
    let file = std::fs::File::create(out_path)
        .with_context(|| format!("creating {}", out_path.display()))?;
    let mut writer = std::io::BufWriter::new(file);
    for (i, state) in snapshots.iter().enumerate() {
        let tick = u64::try_from(i).map_err(|_| anyhow!("tick index overflow"))? + 1;
        let view = game.public_view(state, observer);
        let line = serde_json::json!({ "tick": tick, "view": view });
        serde_json::to_writer(&mut writer, &line)
            .with_context(|| format!("writing tick {tick} to {}", out_path.display()))?;
        writer.write_all(b"\n")?;
    }
    writer
        .flush()
        .with_context(|| format!("flushing {}", out_path.display()))?;
    Ok(())
}

/// Run the `replay` command.
///
/// # Errors
/// Returns an error if the log is missing, malformed, references an
/// unknown game, or replay-validation fails (schema/config/tick
/// mismatch).
pub fn run(args: &ReplayArgs) -> Result<()> {
    let dump = match (&args.public_view_observer, &args.public_view_out) {
        (Some(observer), Some(out)) => Some((*observer, out.as_path())),
        (None, None) => None,
        _ => bail!(
            "--public-view-observer and --public-view-out must be passed together"
        ),
    };
    let game_name = peek_game_name(&args.path)?;
    match game_name.as_str() {
        "cribbage" => replay_cribbage(&args.path, args.tick, dump),
        "shipwreck" => replay_shipwreck(&args.path, args.tick, dump),
        "greatgyre" => replay_greatgyre(&args.path, args.tick, dump),
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

fn replay_cribbage(
    path: &std::path::Path,
    tick_filter: Option<u64>,
    dump: Option<(PlayerId, &std::path::Path)>,
) -> Result<()> {
    let cfg = CribbageConfig;
    let game = CribbageGame::new();
    let replayed = playtest_log::replay::<CribbageGame>(&game, CribbageGame::NAME, &cfg, path)?;

    if let Some((observer, out)) = dump {
        write_public_view_dump(&game, &replayed.snapshots, observer, out)?;
    }

    print_header(&replayed.header, replayed.snapshots.len());
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

fn replay_shipwreck(
    path: &std::path::Path,
    tick_filter: Option<u64>,
    dump: Option<(PlayerId, &std::path::Path)>,
) -> Result<()> {
    // ShipWreck's player count is serialized into the header's
    // `config_hash`; we need to reconstruct a matching config before
    // calling `replay`, otherwise the hash check fails. Recover the
    // agent count from the header and build the config from it.
    let agent_count = peek_agent_count(path)?;
    let n = u8::try_from(agent_count)
        .map_err(|_| anyhow!("log header has too many agents: {agent_count}"))?;
    let cfg = ShipWreckConfig::new(n)
        .with_context(|| format!("log header has invalid agent count: {n}"))?;
    let game = ShipWreckGame::new();
    let replayed = playtest_log::replay::<ShipWreckGame>(&game, ShipWreckGame::NAME, &cfg, path)?;

    if let Some((observer, out)) = dump {
        write_public_view_dump(&game, &replayed.snapshots, observer, out)?;
    }

    print_header(&replayed.header, replayed.snapshots.len());
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

fn replay_greatgyre(
    path: &std::path::Path,
    tick_filter: Option<u64>,
    dump: Option<(PlayerId, &std::path::Path)>,
) -> Result<()> {
    // Same shape as `replay_shipwreck`: Great Gyre's config is also
    // driven by the agent count, serialized into the header's
    // `config_hash`.
    let agent_count = peek_agent_count(path)?;
    let n = u8::try_from(agent_count)
        .map_err(|_| anyhow!("log header has too many agents: {agent_count}"))?;
    let cfg = GreatGyreConfig::new(n)
        .with_context(|| format!("log header has invalid agent count: {n}"))?;
    let game = GreatGyreGame::new();
    let replayed = playtest_log::replay::<GreatGyreGame>(&game, GreatGyreGame::NAME, &cfg, path)?;

    if let Some((observer, out)) = dump {
        write_public_view_dump(&game, &replayed.snapshots, observer, out)?;
    }

    print_header(&replayed.header, replayed.snapshots.len());
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

/// Read the `agents` array from the log header without decoding the
/// event stream. Parallel to [`peek_game_name`].
fn peek_agent_count(path: &std::path::Path) -> Result<usize> {
    let file = std::fs::File::open(path).with_context(|| format!("opening {}", path.display()))?;
    let reader = std::io::BufReader::new(file);
    let first = reader
        .lines()
        .next()
        .ok_or_else(|| anyhow!("log {} is empty", path.display()))??;
    let v: serde_json::Value = serde_json::from_str(&first)
        .with_context(|| format!("parsing header line of {}", path.display()))?;
    let agents = v
        .get("agents")
        .and_then(|a| a.as_array())
        .ok_or_else(|| anyhow!("log header missing `agents` field: {first}"))?;
    Ok(agents.len())
}

fn print_header(header: &LogHeader, tick_count: usize) {
    println!(
        "game: {} version: {} seed: {} agents: {:?}",
        header.game, header.version, header.seed, header.agents,
    );
    println!(
        "started_at: {} config_hash: {}",
        header.started_at, header.config_hash,
    );
    println!("ticks recorded: {tick_count}");
}
