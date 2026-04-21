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
    DEFAULT_TEMPERATURE, GreedyAgent, HeuristicAgent, RandomAgent,
};
use playtest_core::{Agent, Game, PlayerId};
use playtest_cribbage::{CribbageGame, cribbage_eval};
use playtest_shipwreck::{ShipWreckGame, shipwreck_eval};

/// Agent names accepted by [`build_cribbage_agent`] /
/// [`build_shipwreck_agent`] / [`build_agent`], in display order.
pub const KNOWN_AGENTS: &[&str] = &[
    "random",
    "greedy-cribbage",
    "heuristic-cribbage",
    "greedy-shipwreck",
    "heuristic-shipwreck",
];

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
/// # Errors
/// Returns an error if `name` doesn't match a known agent or is a
/// ShipWreck-specific kind.
pub fn build_cribbage_agent(
    name: &str,
    seed: u64,
    player: PlayerId,
) -> Result<Box<dyn Agent<CribbageGame>>> {
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
        other => bail!(
            "unknown agent: {other} (for cribbage); known: {}",
            KNOWN_AGENTS.join(", ")
        ),
    }
}

/// Build an agent typed for `ShipWreckGame`.
///
/// # Errors
/// Returns an error if `name` doesn't match a known agent or is a
/// Cribbage-specific kind.
pub fn build_shipwreck_agent(
    name: &str,
    seed: u64,
    player: PlayerId,
) -> Result<Box<dyn Agent<ShipWreckGame>>> {
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
        other => bail!(
            "unknown agent: {other} (for shipwreck); known: {}",
            KNOWN_AGENTS.join(", ")
        ),
    }
}
