//! Construct agents by name.
//!
//! Since Phase 2 introduced per-game eval functions
//! ([`cribbage_eval`](playtest_cribbage::cribbage_eval) and
//! [`shipwreck_eval`](playtest_shipwreck::shipwreck_eval)), agent
//! construction split into two per-game entry points. The original
//! generic [`build_agent`] remains for agents that don't need a
//! per-game eval (Random, HTTP-remote) — it's now a thin shim that the
//! per-game factories fall through to for those names.
//!
//! ## Per-seat build context
//!
//! Phase 2.5 added one more dimension: the `http-remote` agent needs a
//! server-provided transport port at construction time. Rather than
//! continue growing the positional signature, all three factories take
//! an [`AgentBuildCtx`] that carries the seat, seed, and any optional
//! transports the caller can supply. CLI callers pass
//! [`AgentBuildCtx::cli`] (no transport); the HTTP server builds a ctx
//! with a `Some(transport)` for any seat it wants to back with a
//! remote client.

use std::sync::Arc;

use anyhow::{Result, bail};
use playtest_adapters::ProductionRng;
use playtest_agents::{
    DEFAULT_TEMPERATURE, GreedyAgent, HeuristicAgent, HttpRemoteAgent, ISMCTSAgent, ISMCTSConfig,
    RandomAgent, RemoteAgentTransport, parse_config_overrides,
};
use playtest_core::{Agent, Game, PlayerId};
use playtest_cribbage::{CribbageGame, cribbage_eval};
use playtest_shipwreck::{ShipWreckGame, shipwreck_eval};

/// Agent names accepted by [`build_cribbage_agent`] /
/// [`build_shipwreck_agent`] / [`build_agent`], in display order.
///
/// ISMCTS agents additionally accept a parameter suffix after `:`, e.g.
/// `"ismcts-cribbage:iter=2000,c=1.4"`. The base name (before the `:`)
/// is what [`is_known_agent`] checks against.
///
/// `"http-remote"` is a game-agnostic interactive kind — it requires the
/// caller to supply a [`RemoteAgentTransport`] via [`AgentBuildCtx`].
/// The CLI rejects it (no coordinator); the HTTP server accepts it.
pub const KNOWN_AGENTS: &[&str] = &[
    "random",
    "http-remote",
    "greedy-cribbage",
    "heuristic-cribbage",
    "ismcts-cribbage",
    "greedy-shipwreck",
    "heuristic-shipwreck",
    "ismcts-shipwreck",
];

/// Default ISMCTS budget for agents built via [`build_cribbage_agent`]
/// / [`build_shipwreck_agent`] without explicit parameter overrides.
/// Tuned to clear R2.3's 65% bar while staying reasonable on 10K-game
/// benchmarks. Larger than `ISMCTSConfig::default()` because the
/// Phase-2 exit criterion needs strong play.
const DEFAULT_ISMCTS_ITERATIONS: u32 = 1000;

fn default_ismcts_config() -> ISMCTSConfig {
    ISMCTSConfig {
        iterations: DEFAULT_ISMCTS_ITERATIONS,
        exploration_c: std::f64::consts::SQRT_2,
        rollout_depth: 50,
        seed: 0,
    }
}

/// Per-seat build context used by the agent factories.
///
/// `seed` and `player` have the same meaning as the prior positional
/// arguments. `remote_transport` is populated by the HTTP server when
/// a given seat is backed by a remote client; it stays `None` for CLI
/// calls so `http-remote` fails fast with a helpful message.
#[derive(Clone)]
pub struct AgentBuildCtx {
    pub seed: u64,
    pub player: PlayerId,
    pub remote_transport: Option<Arc<dyn RemoteAgentTransport>>,
}

impl AgentBuildCtx {
    /// Ctx for a CLI caller — no remote transport.
    #[must_use]
    pub fn cli(seed: u64, player: PlayerId) -> Self {
        Self {
            seed,
            player,
            remote_transport: None,
        }
    }
}

impl core::fmt::Debug for AgentBuildCtx {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("AgentBuildCtx")
            .field("seed", &self.seed)
            .field("player", &self.player)
            .field("remote_transport", &self.remote_transport.is_some())
            .finish()
    }
}

/// Split an agent spec like `"ismcts-cribbage:iter=2000,c=1.4"` into
/// `("ismcts-cribbage", Some("iter=2000,c=1.4"))`. Names without a `:`
/// return `(name, None)`.
#[must_use]
pub fn split_agent_spec(spec: &str) -> (&str, Option<&str>) {
    match spec.split_once(':') {
        Some((name, params)) => (name, Some(params)),
        None => (spec, None),
    }
}

/// Is `spec` either a base name in [`KNOWN_AGENTS`] or a parameterized
/// form `"<base>:params"` where `<base>` is in [`KNOWN_AGENTS`]?
#[must_use]
pub fn is_known_agent(spec: &str) -> bool {
    let (name, _) = split_agent_spec(spec);
    KNOWN_AGENTS.contains(&name)
}

fn ismcts_config_from_params(params: Option<&str>, seed: u64) -> Result<ISMCTSConfig> {
    let mut cfg = default_ismcts_config();
    cfg.seed = seed;
    if let Some(p) = params {
        let overrides = parse_config_overrides(p).map_err(|e| anyhow::anyhow!(e))?;
        // Only override non-default fields. `parse_config_overrides`
        // starts from `ISMCTSConfig::default()`, so we merge manually
        // against *that* baseline: if the parsed value differs from
        // the default, treat it as explicit.
        let d = ISMCTSConfig::default();
        if overrides.iterations != d.iterations {
            cfg.iterations = overrides.iterations;
        }
        if (overrides.exploration_c - d.exploration_c).abs() > f64::EPSILON {
            cfg.exploration_c = overrides.exploration_c;
        }
        if overrides.rollout_depth != d.rollout_depth {
            cfg.rollout_depth = overrides.rollout_depth;
        }
        if overrides.seed != d.seed {
            cfg.seed = overrides.seed;
        }
    }
    Ok(cfg)
}

fn build_http_remote<G>(ctx: &AgentBuildCtx) -> Result<Box<dyn Agent<G>>>
where
    G: Game + ?Sized + 'static,
    G::State: Send + Sync + 'static,
    G::PublicView: Send + Sync + 'static,
    G::Action: Send + Sync + serde::Serialize + 'static,
{
    let transport = ctx.remote_transport.as_ref().ok_or_else(|| {
        anyhow::anyhow!(
            "http-remote agent requires a server-provided transport; use POST /api/runs with \
             agents=[\"http-remote\", ...] instead of the CLI"
        )
    })?;
    Ok(Box::new(HttpRemoteAgent::<G>::new(
        ctx.player,
        transport.clone(),
    )))
}

/// Generic agent builder — handles agent kinds that don't depend on a
/// per-game eval function (today: `"random"` and `"http-remote"`).
///
/// Per-game search agents (`greedy-cribbage`, `heuristic-cribbage`,
/// `greedy-shipwreck`, `heuristic-shipwreck`) must be built via
/// [`build_cribbage_agent`] / [`build_shipwreck_agent`] respectively,
/// because they need the concrete game's eval function at construction.
///
/// # Errors
/// Returns an error listing [`KNOWN_AGENTS`] if `name` is not
/// registered or is a per-game kind in disguise (e.g. trying to build
/// `greedy-cribbage` via the generic path). Returns an error for
/// `http-remote` when `ctx.remote_transport` is `None`.
pub fn build_agent<G>(name: &str, ctx: &AgentBuildCtx) -> Result<Box<dyn Agent<G>>>
where
    G: Game + ?Sized + 'static,
    G::State: Send + Sync + 'static,
    G::PublicView: Send + Sync + 'static,
    G::Action: Send + Sync + serde::Serialize + 'static,
{
    match name {
        "random" => {
            let rng = ProductionRng::from_seed(ctx.seed);
            Ok(Box::new(RandomAgent::<G, _>::new(rng)))
        }
        "http-remote" => build_http_remote::<G>(ctx),
        other => bail!(
            "unknown or non-generic agent: {other}; known: {}",
            KNOWN_AGENTS.join(", ")
        ),
    }
}

/// Build an agent typed for `CribbageGame`.
///
/// Accepts parameterized forms for ISMCTS, e.g.
/// `"ismcts-cribbage:iter=2000,c=1.4,depth=30"`. Unknown override keys
/// return an error.
///
/// # Errors
/// Returns an error if `spec` doesn't match a known agent or is a
/// ShipWreck-specific kind. Returns an error for `http-remote` when
/// `ctx.remote_transport` is `None`.
pub fn build_cribbage_agent(
    spec: &str,
    ctx: &AgentBuildCtx,
) -> Result<Box<dyn Agent<CribbageGame>>> {
    let (name, params) = split_agent_spec(spec);
    match name {
        "random" | "http-remote" => build_agent::<CribbageGame>(name, ctx),
        "greedy-cribbage" => Ok(Box::new(GreedyAgent::<CribbageGame>::new(
            ctx.player,
            cribbage_eval,
        ))),
        "heuristic-cribbage" => {
            let rng = ProductionRng::from_seed(ctx.seed);
            Ok(Box::new(HeuristicAgent::<CribbageGame, _>::with_temperature(
                ctx.player,
                cribbage_eval,
                rng,
                DEFAULT_TEMPERATURE,
            )))
        }
        "ismcts-cribbage" => {
            let cfg = ismcts_config_from_params(params, ctx.seed)?;
            Ok(Box::new(ISMCTSAgent::<CribbageGame>::with_eval(
                cfg,
                ctx.player,
                cribbage_eval,
            )))
        }
        other => bail!(
            "unknown agent: {other} (for cribbage); known: {}",
            KNOWN_AGENTS.join(", ")
        ),
    }
}

/// Build an agent typed for `ShipWreckGame`.
///
/// Accepts parameterized forms for ISMCTS, e.g.
/// `"ismcts-shipwreck:iter=1000,c=1.4"`.
///
/// # Errors
/// Returns an error if `spec` doesn't match a known agent or is a
/// Cribbage-specific kind. Returns an error for `http-remote` when
/// `ctx.remote_transport` is `None`.
pub fn build_shipwreck_agent(
    spec: &str,
    ctx: &AgentBuildCtx,
) -> Result<Box<dyn Agent<ShipWreckGame>>> {
    let (name, params) = split_agent_spec(spec);
    match name {
        "random" | "http-remote" => build_agent::<ShipWreckGame>(name, ctx),
        "greedy-shipwreck" => Ok(Box::new(GreedyAgent::<ShipWreckGame>::new(
            ctx.player,
            shipwreck_eval,
        ))),
        "heuristic-shipwreck" => {
            let rng = ProductionRng::from_seed(ctx.seed);
            Ok(Box::new(HeuristicAgent::<ShipWreckGame, _>::with_temperature(
                ctx.player,
                shipwreck_eval,
                rng,
                DEFAULT_TEMPERATURE,
            )))
        }
        "ismcts-shipwreck" => {
            let cfg = ismcts_config_from_params(params, ctx.seed)?;
            Ok(Box::new(ISMCTSAgent::<ShipWreckGame>::with_eval(
                cfg,
                ctx.player,
                shipwreck_eval,
            )))
        }
        other => bail!(
            "unknown agent: {other} (for shipwreck); known: {}",
            KNOWN_AGENTS.join(", ")
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use playtest_agents::RemoteTransportError;
    use serde_json::Value as JsonValue;

    /// Minimal stub for tests that just need the `http-remote` code path
    /// to succeed in constructing an agent — no round-trip logic.
    struct NeverCalledTransport;

    #[async_trait]
    impl RemoteAgentTransport for NeverCalledTransport {
        async fn issue_prompt(
            &self,
            _seat: u8,
            _legal_json: Vec<JsonValue>,
        ) -> Result<u64, RemoteTransportError> {
            unreachable!("test does not exercise issue_prompt")
        }
        async fn await_action(
            &self,
            _seat: u8,
            _prompt_id: u64,
        ) -> Result<usize, RemoteTransportError> {
            unreachable!("test does not exercise await_action")
        }
    }

    #[test]
    fn http_remote_is_in_known_agents() {
        assert!(KNOWN_AGENTS.contains(&"http-remote"));
        assert!(is_known_agent("http-remote"));
    }

    #[test]
    fn build_http_remote_without_transport_fails_with_helpful_message() {
        let ctx = AgentBuildCtx::cli(42, 0);
        let err = build_cribbage_agent("http-remote", &ctx)
            .err()
            .expect("must fail without transport");
        let msg = err.to_string();
        assert!(
            msg.contains("requires a server-provided transport"),
            "message was: {msg}"
        );
    }

    #[test]
    fn build_http_remote_with_transport_succeeds_for_both_games() {
        let ctx = AgentBuildCtx {
            seed: 42,
            player: 0,
            remote_transport: Some(Arc::new(NeverCalledTransport)),
        };
        build_cribbage_agent("http-remote", &ctx).expect("cribbage http-remote");
        build_shipwreck_agent("http-remote", &ctx).expect("shipwreck http-remote");
    }

    #[test]
    fn non_remote_agents_ignore_transport() {
        let ctx = AgentBuildCtx {
            seed: 42,
            player: 0,
            remote_transport: Some(Arc::new(NeverCalledTransport)),
        };
        // Having a transport present shouldn't break non-remote kinds.
        build_cribbage_agent("random", &ctx).expect("random with transport present");
        build_cribbage_agent("greedy-cribbage", &ctx).expect("greedy with transport present");
    }

    #[test]
    fn generic_build_agent_rejects_per_game_kinds() {
        let ctx = AgentBuildCtx::cli(1, 0);
        let err = build_agent::<CribbageGame>("greedy-cribbage", &ctx)
            .err()
            .expect("per-game kinds not in generic path");
        let msg = err.to_string();
        assert!(msg.contains("unknown or non-generic"), "message was: {msg}");
    }
}

