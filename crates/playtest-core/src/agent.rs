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
//!
//! ## Trait-surgery note (Unit 25)
//!
//! `choose` was evolved to also take `state: &G::State`. Two reasons:
//!
//! 1. Phase-2 search agents (`GreedyAgent`, `HeuristicAgent`,
//!    `ISMCTSAgent`) need to simulate legal actions one ply forward
//!    via `Game::apply_action` + `Game::apply_event`, which require
//!    a full state, not just a redacted public view. Reconstructing
//!    state from the view doesn't generalize (some games can't).
//! 2. Deterministic one-shot evaluation — `eval(view, player)` —
//!    becomes a clean "score the view after each candidate action"
//!    loop inside the agent, with no side channels.
//!
//! Well-behaved agents that want to honor the hidden-information
//! contract should read strictly from `view`. Agents that want to
//! peek (for benchmarking against a perfect-info baseline) can read
//! `state`. The `Game::determinize` method remains the proper way to
//! produce an information-set-consistent state for search.

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
    /// `view` is the redacted, player-visible snapshot. `state` is the
    /// full engine state — passed so search agents can run one-ply
    /// simulations without peeking at hidden information (see
    /// `GreedyAgent` / `ISMCTSAgent`). Agents that don't need it can
    /// take `_state: &G::State` and ignore it.
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
        state: &G::State,
    ) -> Result<usize, AgentError>;
}
