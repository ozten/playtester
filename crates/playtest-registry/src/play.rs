//! Dispatch helper: run one game of a [`RegisteredGame`] into a
//! caller-supplied [`GameEventSink`].
//!
//! Extracted so the `playtest-server` crate can wrap a sink in a
//! broadcast adapter while reusing the same dispatcher the CLI's
//! `play` subcommand uses. Keeping the game-specific `match` in this
//! one place preserves the architectural invariant: no game-specific
//! identifiers appear in `playtest-server/src/`.

use std::sync::Arc;

use anyhow::Result;
use playtest_adapters::{ProductionClock, ProductionRng};
use playtest_agents::RemoteAgentTransport;
use playtest_core::{Agent, Game, GameLoop};
use playtest_cribbage::{CribbageConfig, CribbageGame, Event as CribbageEvent};
use playtest_log::{EventLogWriter, LogHeader, LogRecord, SCHEMA_VERSION, compute_config_hash};
use playtest_ports::{Clock, GameEventSink};
use playtest_shipwreck::{Event as ShipWreckEvent, ShipWreckConfig, ShipWreckGame};

use crate::agent_registry::{AgentBuildCtx, build_cribbage_agent, build_shipwreck_agent};
use crate::game_registry::RegisteredGame;

/// Optional per-seat remote transports. `None` (or a vec of all-`None`)
/// means no seat is backed by a remote client — the CLI case. The HTTP
/// server passes `Some(&vec)` where `vec[i] == Some(transport)` exactly
/// for seats whose agent name is `"http-remote"`.
pub type RemoteTransports = [Option<Arc<dyn RemoteAgentTransport>>];

/// Run one game, writing every log line (header + events + final) to
/// `sink`.
///
/// `remote_transports`, when supplied, must have `agent_names.len()`
/// entries. `None` for a seat means that seat is built with the normal
/// factory path (no remote transport).
///
/// # Errors
/// Propagates I/O, serialisation, and engine errors.
pub fn run_single_game_into_sink(
    game: &RegisteredGame,
    agent_names: &[String],
    seed: u64,
    fixed_time: Option<u64>,
    remote_transports: Option<&RemoteTransports>,
    sink: &mut dyn GameEventSink,
) -> Result<()> {
    if let Some(t) = remote_transports
        && t.len() != agent_names.len()
    {
        anyhow::bail!(
            "remote_transports length {} does not match agent_names length {}",
            t.len(),
            agent_names.len()
        );
    }
    match game {
        RegisteredGame::Cribbage(g) => {
            run_cribbage_into_sink(*g, agent_names, seed, fixed_time, remote_transports, sink)
        }
        RegisteredGame::ShipWreck(g) => {
            run_shipwreck_into_sink(*g, agent_names, seed, fixed_time, remote_transports, sink)
        }
    }
}

fn build_ctx_for_seat(
    i: usize,
    seed: u64,
    remote_transports: Option<&RemoteTransports>,
) -> AgentBuildCtx {
    // Derive a distinct per-agent seed: master_seed XORed with a
    // SplitMix-style bit-mix of the agent index. Keeps the two
    // agents' RNG streams independent of the chance RNG and of
    // each other.
    let mix =
        0x9E37_79B9_7F4A_7C15u64.wrapping_mul(u64::try_from(i + 1).expect("small i"));
    let agent_seed = seed ^ mix;
    let player = u8::try_from(i).expect("player index fits in u8");
    let remote_transport = remote_transports
        .and_then(|t| t.get(i))
        .and_then(Clone::clone);
    AgentBuildCtx {
        seed: agent_seed,
        player,
        remote_transport,
    }
}

fn run_cribbage_into_sink(
    game: CribbageGame,
    agent_names: &[String],
    seed: u64,
    fixed_time: Option<u64>,
    remote_transports: Option<&RemoteTransports>,
    sink: &mut dyn GameEventSink,
) -> Result<()> {
    let cfg = CribbageConfig;

    let mut agents: Vec<Box<dyn Agent<CribbageGame>>> = agent_names
        .iter()
        .enumerate()
        .map(|(i, name)| {
            let ctx = build_ctx_for_seat(i, seed, remote_transports);
            build_cribbage_agent(name, &ctx)
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

fn run_shipwreck_into_sink(
    game: ShipWreckGame,
    agent_names: &[String],
    seed: u64,
    fixed_time: Option<u64>,
    remote_transports: Option<&RemoteTransports>,
    sink: &mut dyn GameEventSink,
) -> Result<()> {
    // ShipWreck's config is driven by the agent count — the CLI's
    // `--agents a,b,c` implies a 3-player game. The registry validates
    // the count is in range before we get here, so it's safe to unwrap.
    let n = u8::try_from(agent_names.len()).expect("agent count fits in u8");
    let cfg = ShipWreckConfig::new(n)
        .expect("agent count validated against registry player range before dispatch");

    let mut agents: Vec<Box<dyn Agent<ShipWreckGame>>> = agent_names
        .iter()
        .enumerate()
        .map(|(i, name)| {
            let ctx = build_ctx_for_seat(i, seed, remote_transports);
            build_shipwreck_agent(name, &ctx)
        })
        .collect::<Result<Vec<_>>>()?;

    let started_at = if let Some(t) = fixed_time {
        t
    } else {
        ProductionClock::new().now()
    };

    let header = LogHeader {
        schema: SCHEMA_VERSION,
        game: ShipWreckGame::NAME.to_owned(),
        version: env!("CARGO_PKG_VERSION").to_owned(),
        seed,
        agents: agent_names.to_vec(),
        started_at,
        config_hash: compute_config_hash(&cfg)?,
    };

    {
        let mut writer: EventLogWriter<ShipWreckEvent> = EventLogWriter::new(sink);
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

    let finished_at = if let Some(t) = fixed_time {
        t
    } else {
        ProductionClock::new().now()
    };

    let final_line = serde_json::to_string(&LogRecord::<ShipWreckEvent>::Final {
        winner: result.winner,
        reason: result.reason,
        scores: result.scores,
        finished_at,
    })?;
    sink.emit(&final_line)?;
    sink.flush()?;
    Ok(())
}
