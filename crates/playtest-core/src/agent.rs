//! The `Agent` trait: a strategy that picks actions for a player.
//!
//! Agents are async from day one (see architectural invariant #6):
//! Phase 3's TerminalAgent blocks on stdin, and LlmAgent makes network
//! calls. Sync agents (`RandomAgent`, `ScriptedAgent`) simply return
//! ready futures and pay no runtime cost.
//!
//! Agents return an **index** into the legal-actions slice, not an
//! `Action`. This prevents agents from fabricating moves and matches
//! the stdio protocol shape expected in Phase 3 (invariant #7).

use async_trait::async_trait;

use crate::Game;
use crate::error::AgentError;

/// An action-selection strategy, parameterized by the game it plays.
///
/// `&mut self` allows agents to maintain state across turns (tree
/// search reuse, learning counters, conversation history). Sync agents
/// that don't need this can just ignore the mutability.
#[async_trait]
pub trait Agent<G: Game + ?Sized>: Send {
    /// Choose an index into `legal`. The caller guarantees `legal` is
    /// non-empty.
    ///
    /// # Errors
    /// Return [`AgentError`] when the agent cannot produce a choice —
    /// network failure, timeout, LLM budget exceeded, etc. The engine
    /// will wrap this in a [`GameError::AgentFailed`](crate::GameError::AgentFailed)
    /// with the player's id attached.
    async fn choose(
        &mut self,
        view: &G::PublicView,
        legal: &[G::Action],
    ) -> Result<usize, AgentError>;
}
