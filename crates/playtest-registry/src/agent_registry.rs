//! Construct agents by name. Generic over `G: Game` so it works for
//! every game in [`crate::game_registry`].

use anyhow::{Result, bail};
use playtest_adapters::ProductionRng;
use playtest_agents::RandomAgent;
use playtest_core::{Agent, Game};

/// Agent names accepted by [`build_agent`], in display order.
pub const KNOWN_AGENTS: &[&str] = &["random"];

/// Build an agent for game `G` by user-visible name.
///
/// `seed` drives the agent's internal RNG (for `RandomAgent`); it
/// must be distinct from the engine's chance-RNG seed so agent
/// stochasticity is independent from the game's chance events — see
/// the plan's rationale for per-agent RNGs.
///
/// # Errors
/// Returns an error listing [`KNOWN_AGENTS`] if `name` is not
/// registered.
pub fn build_agent<G>(name: &str, seed: u64) -> Result<Box<dyn Agent<G>>>
where
    G: Game + ?Sized + 'static,
    G::PublicView: Send + Sync + 'static,
    G::Action: Send + Sync + 'static,
{
    match name {
        "random" => {
            let rng = ProductionRng::from_seed(seed);
            Ok(Box::new(RandomAgent::<G, _>::new(rng)))
        }
        other => bail!("unknown agent: {other}; known: {}", KNOWN_AGENTS.join(", ")),
    }
}
