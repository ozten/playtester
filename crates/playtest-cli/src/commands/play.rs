//! `playtest play` — run N games of a registered game with two
//! registered agents, writing one JSONL event log per game.

use std::path::{Path, PathBuf};

use anyhow::{Result, bail};
use clap::Args as ClapArgs;
use playtest_adapters::{ProductionFileSystem, ProductionGameEventSink};
use playtest_registry::game_registry::{RegisteredGame, lookup as lookup_game};
use playtest_registry::play::run_single_game_into_sink;

/// CLI flags for the `play` subcommand.
#[derive(Debug, ClapArgs)]
pub struct PlayArgs {
    /// Game name (e.g. `cribbage`).
    #[arg(long)]
    pub game: String,

    /// Comma-separated agent names, one per player (e.g. `random,random`).
    #[arg(long)]
    pub agents: String,

    /// Number of games to play.
    #[arg(long)]
    pub games: u32,

    /// Master seed. Per-game seeds derive as `seed + game_index`.
    #[arg(long)]
    pub seed: u64,

    /// Output directory for JSONL event logs. Created if missing.
    #[arg(long)]
    pub out: PathBuf,

    /// Fan games out across rayon worker threads.
    #[arg(long)]
    pub parallel: bool,

    /// Pin the log header's `started_at` to a fixed Unix-ms value.
    /// When unset, the production clock is used — so two runs of the
    /// same command produce bit-identical event bodies but differing
    /// header timestamps. Tests that want bit-for-bit determinism
    /// across runs should pass `--fixed-time 0`.
    #[arg(long)]
    pub fixed_time: Option<u64>,
}

/// Run the `play` command.
///
/// # Errors
/// Propagates any game-run error, I/O error, or registry-lookup error.
pub fn run(args: &PlayArgs) -> Result<()> {
    let game = lookup_game(&args.game)?;
    let agent_names: Vec<String> = args.agents.split(',').map(str::to_owned).collect();

    if agent_names.len() != 2 {
        bail!(
            "expected 2 agents (2-player only for Phase 0), got {}: `{}`",
            agent_names.len(),
            args.agents
        );
    }

    if args.games == 0 {
        return Ok(());
    }

    std::fs::create_dir_all(&args.out)?;

    let indices: Vec<u32> = (0..args.games).collect();
    let run_one = |idx: u32| -> Result<()> {
        let per_game_seed = args.seed.wrapping_add(u64::from(idx));
        let out_path = args.out.join(format!("game-{idx:04}.jsonl"));
        run_single_game(
            &game,
            &agent_names,
            per_game_seed,
            args.fixed_time,
            &out_path,
        )
    };

    if args.parallel {
        use rayon::prelude::*;
        indices.into_par_iter().try_for_each(run_one)?;
    } else {
        for idx in indices {
            run_one(idx)?;
        }
    }

    Ok(())
}

/// Run one game. Dispatches on the registered game's variant.
fn run_single_game(
    game: &RegisteredGame,
    agent_names: &[String],
    seed: u64,
    fixed_time: Option<u64>,
    out_path: &Path,
) -> Result<()> {
    let fs = ProductionFileSystem::new();
    let mut sink = ProductionGameEventSink::new(fs, out_path);
    run_single_game_into_sink(game, agent_names, seed, fixed_time, &mut sink)
}
