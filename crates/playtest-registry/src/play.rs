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
use playtest_agents::{
    CritiqueSidecar, LlmSidecar, PostGameCritic, QuestionnaireSpec, RemoteAgentTransport,
    StdioAgentConfig,
};
use playtest_core::{Agent, Game, GameLoop};
use playtest_cribbage::{CribbageConfig, CribbageGame, Event as CribbageEvent};
use playtest_greatgyre::{Event as GreatGyreEvent, GreatGyreConfig, GreatGyreGame};
use playtest_log::{EventLogWriter, LogHeader, LogRecord, SCHEMA_VERSION, compute_config_hash};
use playtest_ports::{Clock, GameEventSink, LlmClient};
use playtest_shipwreck::{
    Event as ShipWreckEvent, EventCard, ShipWreckConfig, ShipWreckGame,
};

use crate::agent_registry::{
    AgentBuildCtx, BuiltAgent, build_cribbage_agent_with_critic, build_greatgyre_agent_with_critic,
    build_shipwreck_agent_with_critic, validate_llm_provider_consistency,
};
use crate::game_registry::RegisteredGame;

/// Optional per-seat remote transports. `None` (or a vec of all-`None`)
/// means no seat is backed by a remote client — the CLI case. The HTTP
/// server passes `Some(&vec)` where `vec[i] == Some(transport)` exactly
/// for seats whose agent name is `"http-remote"`.
pub type RemoteTransports = [Option<Arc<dyn RemoteAgentTransport>>];

/// Parallel critic vec produced by splitting a `Vec<BuiltAgent<G>>`.
/// One slot per seat; `Some` only for `llm` seats.
type CritiqueSlots<G> = Vec<Option<Box<dyn PostGameCritic<G>>>>;
type AgentSlots<G> = Vec<Box<dyn Agent<G>>>;

/// LLM-related dependencies for a single game.
///
/// Every `llm:*` seat in the game shares one `client` and one `sidecar`
/// (the sidecar is `Option<_>` so library tests can exercise `build_llm`
/// without wiring a file-system write path).
#[derive(Clone)]
pub struct LlmCliDeps {
    pub client: Arc<dyn LlmClient>,
    pub sidecar: Option<Arc<LlmSidecar>>,
    pub model: String,
    pub max_tokens: Option<u32>,
    /// Phase 5: when set, every `llm` seat's agent will emit one
    /// `questionnaire_response` record into this sidecar after
    /// `GameLoop::run` returns. Non-`llm` seats are untouched.
    pub critique_sidecar: Option<Arc<CritiqueSidecar>>,
    /// Phase 5: questionnaire schema the critique pass uses. Required
    /// when `critique_sidecar` is `Some`. Shared `Arc<_>` so multiple
    /// games in one batch use byte-identical spec (stable `sha256()`).
    pub critique_spec: Option<Arc<QuestionnaireSpec>>,
}

impl core::fmt::Debug for LlmCliDeps {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("LlmCliDeps")
            .field("model", &self.model)
            .field("max_tokens", &self.max_tokens)
            .field("sidecar", &self.sidecar.is_some())
            .field("critique_sidecar", &self.critique_sidecar.is_some())
            .field("critique_spec", &self.critique_spec.is_some())
            .finish_non_exhaustive()
    }
}

/// Optional run-time dependencies layered on top of the base
/// `run_single_game_into_sink` signature. Callers who don't care
/// about LLM or stdio can pass `None` for everything and match the
/// legacy shape.
#[derive(Default)]
pub struct RunExtras<'a> {
    pub remote_transports: Option<&'a RemoteTransports>,
    pub llm_deps: Option<&'a LlmCliDeps>,
    pub stdio_cfg: Option<&'a StdioAgentConfig>,
    /// Phase 6: per-event-card disables for ShipWreck. Every listed
    /// card has `<kind>_enabled` flipped to false in the game's
    /// config; cohorts with different disable lists produce different
    /// config hashes automatically. Silent no-op for non-ShipWreck
    /// games.
    pub shipwreck_disabled_events: Vec<EventCard>,
}

impl<'a> RunExtras<'a> {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn with_remote_transports(mut self, t: &'a RemoteTransports) -> Self {
        self.remote_transports = Some(t);
        self
    }

    #[must_use]
    pub fn with_llm_deps(mut self, d: &'a LlmCliDeps) -> Self {
        self.llm_deps = Some(d);
        self
    }

    #[must_use]
    pub fn with_stdio_cfg(mut self, c: &'a StdioAgentConfig) -> Self {
        self.stdio_cfg = Some(c);
        self
    }

    #[must_use]
    pub fn with_disabled_shipwreck_events(mut self, events: Vec<EventCard>) -> Self {
        self.shipwreck_disabled_events = events;
        self
    }
}

/// Run one game, writing every log line (header + events + final) to
/// `sink`.
///
/// `remote_transports`, when supplied, must have `agent_names.len()`
/// entries. `None` for a seat means that seat is built with the normal
/// factory path (no remote transport).
///
/// This is the legacy two-arg-and-change entry point preserved for the
/// HTTP-remote callsite in `playtest-server`. For Phase-3 callers who
/// need LLM / stdio context, use
/// [`run_single_game_into_sink_with_extras`].
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
    let mut extras = RunExtras::new();
    if let Some(t) = remote_transports {
        extras = extras.with_remote_transports(t);
    }
    run_single_game_into_sink_with_extras(game, agent_names, seed, fixed_time, &extras, sink)
}

/// Phase-3 extension of [`run_single_game_into_sink`]. Every optional
/// per-seat dependency (`llm` / `stdio`) is bundled into
/// [`RunExtras`]; any absent field stays `None` and the corresponding
/// kind cannot be built.
///
/// # Errors
/// Propagates I/O, serialisation, and engine errors. Also errors if
/// `extras.remote_transports.len()` mismatches `agent_names.len()`.
pub fn run_single_game_into_sink_with_extras(
    game: &RegisteredGame,
    agent_names: &[String],
    seed: u64,
    fixed_time: Option<u64>,
    extras: &RunExtras<'_>,
    sink: &mut dyn GameEventSink,
) -> Result<()> {
    if let Some(t) = extras.remote_transports
        && t.len() != agent_names.len()
    {
        anyhow::bail!(
            "remote_transports length {} does not match agent_names length {}",
            t.len(),
            agent_names.len()
        );
    }
    // Reject mixed-provider LLM seats up front. The run has exactly one
    // `ProductionLlmClient` (see `LlmCliDeps`), so disagreement here
    // would surface mid-game as a silent fallback. Fail loud.
    validate_llm_provider_consistency(agent_names)?;
    match game {
        RegisteredGame::Cribbage(g) => run_cribbage_into_sink(*g, agent_names, seed, fixed_time, extras, sink),
        RegisteredGame::ShipWreck(g) => {
            run_shipwreck_into_sink(*g, agent_names, seed, fixed_time, extras, sink)
        }
        RegisteredGame::GreatGyre(g) => {
            run_greatgyre_into_sink(*g, agent_names, seed, fixed_time, extras, sink)
        }
    }
}

fn build_ctx_for_seat(i: usize, seed: u64, extras: &RunExtras<'_>) -> AgentBuildCtx {
    // Derive a distinct per-agent seed: master_seed XORed with a
    // SplitMix-style bit-mix of the agent index. Keeps the two
    // agents' RNG streams independent of the chance RNG and of
    // each other.
    let mix = 0x9E37_79B9_7F4A_7C15u64.wrapping_mul(u64::try_from(i + 1).expect("small i"));
    let agent_seed = seed ^ mix;
    let player = u8::try_from(i).expect("player index fits in u8");
    let remote_transport = extras
        .remote_transports
        .and_then(|t| t.get(i))
        .and_then(Clone::clone);
    let (llm_client, llm_model, llm_max_tokens, llm_sidecar) = extras.llm_deps.map_or(
        (None, None, None, None),
        |d| {
            (
                Some(d.client.clone()),
                Some(d.model.clone()),
                d.max_tokens,
                d.sidecar.clone(),
            )
        },
    );
    AgentBuildCtx {
        seed: agent_seed,
        player,
        remote_transport,
        llm_client,
        llm_sidecar,
        llm_model,
        llm_max_tokens,
        stdio_cfg: extras.stdio_cfg.cloned(),
    }
}

fn run_cribbage_into_sink(
    game: CribbageGame,
    agent_names: &[String],
    seed: u64,
    fixed_time: Option<u64>,
    extras: &RunExtras<'_>,
    sink: &mut dyn GameEventSink,
) -> Result<()> {
    let cfg = CribbageConfig;

    let built: Vec<BuiltAgent<CribbageGame>> = agent_names
        .iter()
        .enumerate()
        .map(|(i, name)| {
            let ctx = build_ctx_for_seat(i, seed, extras);
            build_cribbage_agent_with_critic(name, &ctx)
        })
        .collect::<Result<Vec<_>>>()?;

    let (mut agents, critics): (AgentSlots<CribbageGame>, CritiqueSlots<CribbageGame>) =
        built.into_iter().map(|b| (b.agent, b.critic)).unzip();

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

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;

    let result = rt.block_on(loop_.run(agents.as_mut_slice(), &mut chance_rng, sink))?;

    // Phase 5: post-game critique. Runs after the engine has finished
    // emitting its events into `sink`; critique records never reach the
    // main log. Failures log to stderr and do not fail the run.
    if let Some(deps) = extras.llm_deps
        && let (Some(spec), Some(sidecar)) =
            (deps.critique_spec.as_ref(), deps.critique_sidecar.as_ref())
    {
        let final_state = loop_.state();
        for (seat, critic_opt) in critics.iter().enumerate() {
            if let Some(critic) = critic_opt {
                let seat_id = u8::try_from(seat).expect("seat fits in u8");
                let view = game.public_view(final_state, seat_id);
                if let Err(e) = rt.block_on(critic.post_game_critique(
                    &view, &result, spec, sidecar, None,
                )) {
                    eprintln!("post-game critique failed for seat {seat}: {e}");
                }
            }
        }
    }

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
    extras: &RunExtras<'_>,
    sink: &mut dyn GameEventSink,
) -> Result<()> {
    // ShipWreck's config is driven by the agent count — the CLI's
    // `--agents a,b,c` implies a 3-player game. The registry validates
    // the count is in range before we get here, so it's safe to unwrap.
    let n = u8::try_from(agent_names.len()).expect("agent count fits in u8");
    let mut cfg = ShipWreckConfig::new(n)
        .expect("agent count validated against registry player range before dispatch");
    // Phase 6: apply any per-event-card disables from --shipwreck-
    // disable-event. Each flipped flag changes the config_hash, so
    // restricted-play cohorts land in separate SQLite buckets.
    for kind in &extras.shipwreck_disabled_events {
        cfg = cfg.with_event_card(*kind, false);
    }

    let built: Vec<BuiltAgent<ShipWreckGame>> = agent_names
        .iter()
        .enumerate()
        .map(|(i, name)| {
            let ctx = build_ctx_for_seat(i, seed, extras);
            build_shipwreck_agent_with_critic(name, &ctx)
        })
        .collect::<Result<Vec<_>>>()?;

    let (mut agents, critics): (AgentSlots<ShipWreckGame>, CritiqueSlots<ShipWreckGame>) =
        built.into_iter().map(|b| (b.agent, b.critic)).unzip();

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

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;

    let result = rt.block_on(loop_.run(agents.as_mut_slice(), &mut chance_rng, sink))?;

    // Phase 5: post-game critique (see the identical block in
    // `run_cribbage_into_sink` for invariants).
    if let Some(deps) = extras.llm_deps
        && let (Some(spec), Some(sidecar)) =
            (deps.critique_spec.as_ref(), deps.critique_sidecar.as_ref())
    {
        let final_state = loop_.state();
        for (seat, critic_opt) in critics.iter().enumerate() {
            if let Some(critic) = critic_opt {
                let seat_id = u8::try_from(seat).expect("seat fits in u8");
                let view = game.public_view(final_state, seat_id);
                if let Err(e) = rt.block_on(critic.post_game_critique(
                    &view, &result, spec, sidecar, None,
                )) {
                    eprintln!("post-game critique failed for seat {seat}: {e}");
                }
            }
        }
    }

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

fn run_greatgyre_into_sink(
    game: GreatGyreGame,
    agent_names: &[String],
    seed: u64,
    fixed_time: Option<u64>,
    extras: &RunExtras<'_>,
    sink: &mut dyn GameEventSink,
) -> Result<()> {
    // Great Gyre's config is driven by the agent count, same as
    // ShipWreck — the registry validates the count is in range before
    // we get here.
    let n = u8::try_from(agent_names.len()).expect("agent count fits in u8");
    let cfg = GreatGyreConfig::new(n)
        .expect("agent count validated against registry player range before dispatch");

    let built: Vec<BuiltAgent<GreatGyreGame>> = agent_names
        .iter()
        .enumerate()
        .map(|(i, name)| {
            let ctx = build_ctx_for_seat(i, seed, extras);
            build_greatgyre_agent_with_critic(name, &ctx)
        })
        .collect::<Result<Vec<_>>>()?;

    let (mut agents, critics): (AgentSlots<GreatGyreGame>, CritiqueSlots<GreatGyreGame>) =
        built.into_iter().map(|b| (b.agent, b.critic)).unzip();

    let started_at = if let Some(t) = fixed_time {
        t
    } else {
        ProductionClock::new().now()
    };

    let header = LogHeader {
        schema: SCHEMA_VERSION,
        game: GreatGyreGame::NAME.to_owned(),
        version: env!("CARGO_PKG_VERSION").to_owned(),
        seed,
        agents: agent_names.to_vec(),
        started_at,
        config_hash: compute_config_hash(&cfg)?,
    };

    {
        let mut writer: EventLogWriter<GreatGyreEvent> = EventLogWriter::new(sink);
        writer.write_header(&header)?;
    }

    let mut loop_ = GameLoop::new(&game, game.initial_state(seed, &cfg));
    let mut chance_rng = ProductionRng::from_seed(seed);

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;

    let result = rt.block_on(loop_.run(agents.as_mut_slice(), &mut chance_rng, sink))?;

    // Phase 5: post-game critique (see the identical block in
    // `run_cribbage_into_sink` for invariants). No `llm` seats are
    // wired up for Great Gyre yet (Unit 5 scope), so `critics` is
    // always all-`None` here — this block is a no-op today and kept
    // only so the pattern stays uniform across games.
    if let Some(deps) = extras.llm_deps
        && let (Some(spec), Some(sidecar)) =
            (deps.critique_spec.as_ref(), deps.critique_sidecar.as_ref())
    {
        let final_state = loop_.state();
        for (seat, critic_opt) in critics.iter().enumerate() {
            if let Some(critic) = critic_opt {
                let seat_id = u8::try_from(seat).expect("seat fits in u8");
                let view = game.public_view(final_state, seat_id);
                if let Err(e) = rt.block_on(critic.post_game_critique(
                    &view, &result, spec, sidecar, None,
                )) {
                    eprintln!("post-game critique failed for seat {seat}: {e}");
                }
            }
        }
    }

    let finished_at = if let Some(t) = fixed_time {
        t
    } else {
        ProductionClock::new().now()
    };

    let final_line = serde_json::to_string(&LogRecord::<GreatGyreEvent>::Final {
        winner: result.winner,
        reason: result.reason,
        scores: result.scores,
        finished_at,
    })?;
    sink.emit(&final_line)?;
    sink.flush()?;
    Ok(())
}
