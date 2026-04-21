//! `ScriptedAgent`: picks the action with the highest priority score.
//!
//! The priority function is provided at construction time. Games
//! typically export factory functions (`playtest_cribbage::scripted::
//! pair_up_preference` etc.) so callers just write
//! `ScriptedAgent::new(pair_up_preference)`.
//!
//! Tie-break: lowest legal-action index wins. This is deterministic and
//! matches the "actions are an ordered list" contract in `Game::legal_actions`.

use core::marker::PhantomData;

use async_trait::async_trait;
use playtest_core::{Agent, AgentError, Game};

/// A priority-function-driven agent.
///
/// `F` is any `Fn(&G::PublicView, &G::Action) -> i32`. The function
/// must be `Send + 'static` so the agent can be used across awaits in
/// async runtimes — which is trivial for plain function items and most
/// closures.
pub struct ScriptedAgent<G, F>
where
    G: Game + ?Sized,
{
    priority: F,
    _game: PhantomData<fn() -> G>,
}

impl<G, F> ScriptedAgent<G, F>
where
    G: Game + ?Sized,
    F: Fn(&G::PublicView, &G::Action) -> i32,
{
    pub fn new(priority: F) -> Self {
        Self {
            priority,
            _game: PhantomData,
        }
    }
}

impl<G, F> std::fmt::Debug for ScriptedAgent<G, F>
where
    G: Game + ?Sized,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ScriptedAgent").finish_non_exhaustive()
    }
}

#[async_trait]
impl<G, F> Agent<G> for ScriptedAgent<G, F>
where
    G: Game + ?Sized,
    G::PublicView: Send + Sync,
    G::Action: Send + Sync,
    F: Fn(&G::PublicView, &G::Action) -> i32 + Send,
{
    async fn choose(
        &mut self,
        view: &G::PublicView,
        legal: &[G::Action],
        _state: &G::State,
    ) -> Result<usize, AgentError> {
        if legal.is_empty() {
            return Err(AgentError::Other(
                "ScriptedAgent::choose called with empty legal slice (engine bug)".into(),
            ));
        }

        // Manual fold rather than `iter().max_by_key()` so the tie-break
        // rule (lowest index wins) is visible in the code.
        let mut best_idx = 0;
        let mut best_score = (self.priority)(view, &legal[0]);
        for (i, action) in legal.iter().enumerate().skip(1) {
            let score = (self.priority)(view, action);
            if score > best_score {
                best_score = score;
                best_idx = i;
            }
        }
        Ok(best_idx)
    }
}
