//! `RandomAgent`: picks uniformly at random over the legal actions.
//!
//! Holds its **own** `Rng` port, deliberately separate from the engine's
//! game RNG. Rationale: agent stochasticity is independent from the
//! game's chance events, and we want to be able to record/playback the
//! two streams independently (different tape files per agent lets us
//! narrow down which RNG contributed to a divergence).

use core::marker::PhantomData;

use async_trait::async_trait;
use playtest_core::{Agent, AgentError, Game};
use playtest_ports::Rng;

/// Uniform-random action selector, generic over the game it plays and
/// the concrete RNG adapter it uses.
#[derive(Debug)]
pub struct RandomAgent<G, R>
where
    G: Game + ?Sized,
    R: Rng,
{
    rng: R,
    _game: PhantomData<fn() -> G>,
}

impl<G, R> RandomAgent<G, R>
where
    G: Game + ?Sized,
    R: Rng,
{
    pub fn new(rng: R) -> Self {
        Self {
            rng,
            _game: PhantomData,
        }
    }

    /// Consume the agent and return the underlying RNG. Handy for tests
    /// that need to inspect the final state of the RNG tape cursor.
    pub fn into_rng(self) -> R {
        self.rng
    }
}

#[async_trait]
impl<G, R> Agent<G> for RandomAgent<G, R>
where
    G: Game + ?Sized,
    G::PublicView: Send + Sync,
    G::Action: Send + Sync,
    R: Rng + Send,
{
    async fn choose(
        &mut self,
        _view: &G::PublicView,
        legal: &[G::Action],
        _state: &G::State,
    ) -> Result<usize, AgentError> {
        let n = u64::try_from(legal.len()).map_err(|_| {
            AgentError::Other(format!(
                "legal-actions slice too large for a u64 range: {}",
                legal.len()
            ))
        })?;
        if n == 0 {
            return Err(AgentError::Other(
                "RandomAgent::choose called with empty legal slice (engine bug)".into(),
            ));
        }
        let idx = self
            .rng
            .gen_range(0..n)
            .map_err(|e| AgentError::Other(format!("rng port error: {e}")))?;
        Ok(usize::try_from(idx).expect("idx < n <= usize::MAX"))
    }
}
