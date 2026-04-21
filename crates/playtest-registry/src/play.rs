//! Dispatch helper: run one game of a [`RegisteredGame`] into a
//! caller-supplied [`GameEventSink`].
//!
//! Extracted so the `playtest-server` crate can wrap a sink in a
//! broadcast adapter while reusing the same dispatcher the CLI's
//! `play` subcommand uses. Keeping the game-specific `match` in this
//! one place preserves the architectural invariant: no game-specific
//! identifiers appear in `playtest-server/src/`.

use anyhow::Result;
use playtest_adapters::{ProductionClock, ProductionRng};
use playtest_core::{Agent, Game, GameLoop};
use playtest_cribbage::{CribbageConfig, CribbageGame, Event as CribbageEvent};
use playtest_log::{EventLogWriter, LogHeader, LogRecord, SCHEMA_VERSION, compute_config_hash};
use playtest_ports::{Clock, GameEventSink};

use crate::agent_registry::build_agent;
use crate::game_registry::RegisteredGame;

/// Run one game, writing every log line (header + events + final) to
/// `sink`.
///
/// # Errors
/// Propagates I/O, serialisation, and engine errors.
pub fn run_single_game_into_sink(
    game: &RegisteredGame,
    agent_names: &[String],
    seed: u64,
    fixed_time: Option<u64>,
    sink: &mut dyn GameEventSink,
) -> Result<()> {
    match game {
        RegisteredGame::Cribbage(g) => {
            run_cribbage_into_sink(*g, agent_names, seed, fixed_time, sink)
        }
    }
}

fn run_cribbage_into_sink(
    game: CribbageGame,
    agent_names: &[String],
    seed: u64,
    fixed_time: Option<u64>,
    sink: &mut dyn GameEventSink,
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
            let mix = 0x9E37_79B9_7F4A_7C15u64
                .wrapping_mul(u64::try_from(i + 1).expect("small i"));
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

    {
        let mut writer: EventLogWriter<CribbageEvent> = EventLogWriter::new(sink);
        writer.write_header(&header)?;
    }

    let mut loop_ = GameLoop::new(&game, game.initial_state(seed, &cfg));
    let mut chance_rng = ProductionRng::from_seed(seed);

    let result = {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        rt.block_on(loop_.run(agents.as_mut_slice(), &mut chance_rng, sink))?
    };

    // Capture finish time the same way started_at was captured: honor
    // `fixed_time` when set so deterministic runs produce identical
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
