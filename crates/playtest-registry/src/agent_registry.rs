//! Construct agents by name.
//!
//! Since Phase 2 introduced per-game eval functions
//! ([`cribbage_eval`](playtest_cribbage::cribbage_eval) and
//! [`shipwreck_eval`](playtest_shipwreck::shipwreck_eval)), agent
//! construction split into two per-game entry points. The original
//! generic [`build_agent`] remains for agents that don't need a
//! per-game eval (Random) — it's now a thin shim that the per-game
//! factories fall through to for those names.

use anyhow::{Result, bail};
use playtest_adapters::ProductionRng;
use playtest_agents::{
    DEFAULT_TEMPERATURE, GreedyAgent, HeuristicAgent, ISMCTSAgent, ISMCTSConfig, RandomAgent,
    parse_config_overrides,
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
pub const KNOWN_AGENTS: &[&str] = &[
    "random",
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

/// Generic agent builder — handles agent kinds that don't depend on a
/// per-game eval function (today, just `"random"`).
///
/// Per-game search agents (`greedy-cribbage`, `heuristic-cribbage`,
/// `greedy-shipwreck`, `heuristic-shipwreck`) must be built via
/// [`build_cribbage_agent`] / [`build_shipwreck_agent`] respectively,
/// because they need the concrete game's eval function at construction.
///
/// # Errors
/// Returns an error listing [`KNOWN_AGENTS`] if `name` is not
/// registered or is a per-game kind in disguise (e.g. trying to build
/// `greedy-cribbage` via the generic path).
pub fn build_agent<G>(name: &str, seed: u64) -> Result<Box<dyn Agent<G>>>
where
    G: Game + ?Sized + 'static,
    G::State: Send + Sync + 'static,
    G::PublicView: Send + Sync + 'static,
    G::Action: Send + Sync + 'static,
{
    match name {
        "random" => {
            let rng = ProductionRng::from_seed(seed);
            Ok(Box::new(RandomAgent::<G, _>::new(rng)))
        }
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
/// ShipWreck-specific kind.
pub fn build_cribbage_agent(
    spec: &str,
    seed: u64,
    player: PlayerId,
) -> Result<Box<dyn Agent<CribbageGame>>> {
    let (name, params) = split_agent_spec(spec);
    match name {
        "random" => build_agent::<CribbageGame>(name, seed),
        "greedy-cribbage" => Ok(Box::new(GreedyAgent::<CribbageGame>::new(
            player,
            cribbage_eval,
        ))),
        "heuristic-cribbage" => {
            let rng = ProductionRng::from_seed(seed);
            Ok(Box::new(HeuristicAgent::<CribbageGame, _>::with_temperature(
                player,
                cribbage_eval,
                rng,
                DEFAULT_TEMPERATURE,
            )))
        }
        "ismcts-cribbage" => {
            let cfg = ismcts_config_from_params(params, seed)?;
            Ok(Box::new(ISMCTSAgent::<CribbageGame>::with_eval(
                cfg,
                player,
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
/// Cribbage-specific kind.
pub fn build_shipwreck_agent(
    spec: &str,
    seed: u64,
    player: PlayerId,
) -> Result<Box<dyn Agent<ShipWreckGame>>> {
    let (name, params) = split_agent_spec(spec);
    match name {
        "random" => build_agent::<ShipWreckGame>(name, seed),
        "greedy-shipwreck" => Ok(Box::new(GreedyAgent::<ShipWreckGame>::new(
            player,
            shipwreck_eval,
        ))),
        "heuristic-shipwreck" => {
            let rng = ProductionRng::from_seed(seed);
            Ok(Box::new(HeuristicAgent::<ShipWreckGame, _>::with_temperature(
                player,
                shipwreck_eval,
                rng,
                DEFAULT_TEMPERATURE,
            )))
        }
        "ismcts-shipwreck" => {
            let cfg = ismcts_config_from_params(params, seed)?;
            Ok(Box::new(ISMCTSAgent::<ShipWreckGame>::with_eval(
                cfg,
                player,
                shipwreck_eval,
            )))
        }
        other => bail!(
            "unknown agent: {other} (for shipwreck); known: {}",
            KNOWN_AGENTS.join(", ")
        ),
    }
}
