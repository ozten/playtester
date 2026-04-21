//! `playtest play` — run N games of a registered game with two
//! registered agents, writing one JSONL event log per game.

use std::path::{Path, PathBuf};

use anyhow::{Result, bail};
use clap::Args as ClapArgs;
use playtest_adapters::{
    ProductionClock, ProductionFileSystem, ProductionGameEventSink, ProductionRng,
};
use playtest_core::{Agent, Game, GameLoop};
use playtest_cribbage::{CribbageConfig, CribbageGame, Event as CribbageEvent};
use playtest_log::{EventLogWriter, LogHeader, LogRecord, SCHEMA_VERSION, compute_config_hash};
use playtest_ports::{Clock, GameEventSink};

use crate::agent_registry::build_agent;
use crate::game_registry::{RegisteredGame, lookup as lookup_game};

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
    match game {
        RegisteredGame::Cribbage(g) => run_cribbage(*g, agent_names, seed, fixed_time, out_path),
    }
}

fn run_cribbage(
    game: CribbageGame,
    agent_names: &[String],
    seed: u64,
    fixed_time: Option<u64>,
    out_path: &Path,
) -> Result<()> {
    let cfg = CribbageConfig;

    let mut agents: Vec<Box<dyn Agent<CribbageGame>>> = agent_names
        .iter()
        .enumerate()
        .map(|(i, name)| {
            // Derive a distinct per-agent seed: master_seed XORed with a
            // SplitMix-style bit-mix of the agent index. Keeps the two
            // agents' RNG streams independent of the chance RNG and of
            // each other.
            let mix = 0x9E37_79B9_7F4A_7C15u64.wrapping_mul(u64::try_from(i + 1).expect("small i"));
            let agent_seed = seed ^ mix;
            build_agent::<CribbageGame>(name, agent_seed)
        })
        .collect::<Result<Vec<_>>>()?;

    let started_at = if let Some(t) = fixed_time {
        t
    } else {
        ProductionClock::new().now()
    };

    let header = LogHeader {
        schema: SCHEMA_VERSION,
        game: CribbageGame::NAME.to_owned(),
        version: env!("CARGO_PKG_VERSION").to_owned(),
        seed,
        agents: agent_names.to_vec(),
        started_at,
        config_hash: compute_config_hash(&cfg)?,
    };

    let fs = ProductionFileSystem::new();
    let mut sink = ProductionGameEventSink::new(fs, out_path);

    {
        let mut writer: EventLogWriter<CribbageEvent> = EventLogWriter::new(&mut sink);
        writer.write_header(&header)?;
    }

    let mut loop_ = GameLoop::new(&game, game.initial_state(seed, &cfg));
    let mut chance_rng = ProductionRng::from_seed(seed);

    let result = {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        rt.block_on(loop_.run(agents.as_mut_slice(), &mut chance_rng, &mut sink))?
    };

    // Capture finish time the same way started_at was captured: honor
    // --fixed-time when set so deterministic runs produce identical
    // bytes end-to-end. Otherwise read from the production clock.
    let finished_at = if let Some(t) = fixed_time {
        t
    } else {
        ProductionClock::new().now()
    };

    let final_line = serde_json::to_string(&LogRecord::<CribbageEvent>::Final {
        winner: result.winner,
        reason: result.reason,
        scores: result.scores,
        finished_at,
    })?;
    sink.emit(&final_line)?;
    sink.flush()?;
    Ok(())
}
