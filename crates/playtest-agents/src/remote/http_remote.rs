//! `HttpRemoteAgent`: game-agnostic agent that defers its `choose` to an
//! external decision-maker reached through a [`RemoteAgentTransport`].
//!
//! The agent itself knows nothing about HTTP — the name reflects its
//! Phase 2.5 use case (browser tabs submitting via `POST .../actions`).
//! Any transport that speaks the same port trait would work.

use core::marker::PhantomData;
use std::sync::Arc;

use async_trait::async_trait;
use playtest_core::{Agent, AgentError, Game, PlayerId};
use serde::Serialize;

use super::transport::RemoteAgentTransport;

/// Defers `choose` to a [`RemoteAgentTransport`] keyed by `seat`.
///
/// `G::Action` must be `Serialize` so legal actions can be sent to the
/// external client for display. Every production `Game` impl in the repo
/// already satisfies this (the JSONL log requires it).
pub struct HttpRemoteAgent<G>
where
    G: Game + ?Sized,
{
    seat: PlayerId,
    transport: Arc<dyn RemoteAgentTransport>,
    _game: PhantomData<fn() -> G>,
}

impl<G> HttpRemoteAgent<G>
where
    G: Game + ?Sized,
{
    /// Build an agent that will consult `transport` for every decision at
    /// `seat`.
    #[must_use]
    pub fn new(seat: PlayerId, transport: Arc<dyn RemoteAgentTransport>) -> Self {
        Self {
            seat,
            transport,
            _game: PhantomData,
        }
    }
}

impl<G> core::fmt::Debug for HttpRemoteAgent<G>
where
    G: Game + ?Sized,
{
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("HttpRemoteAgent")
            .field("seat", &self.seat)
            .finish_non_exhaustive()
    }
}

#[async_trait]
impl<G> Agent<G> for HttpRemoteAgent<G>
where
    G: Game + ?Sized,
    G::State: Send + Sync,
    G::PublicView: Send + Sync,
    G::Action: Send + Sync + Serialize,
{
    async fn choose(
        &mut self,
        _view: &G::PublicView,
        legal: &[G::Action],
        _state: &G::State,
    ) -> Result<usize, AgentError> {
        if legal.is_empty() {
            return Err(AgentError::Other(
                "HttpRemoteAgent::choose called with empty legal slice (engine bug)".into(),
            ));
        }
        let legal_json = legal
            .iter()
            .map(serde_json::to_value)
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| {
                AgentError::Other(format!("serialize legal action for remote prompt: {e}"))
            })?;

        let prompt_id = self
            .transport
            .issue_prompt(self.seat, legal_json)
            .await
            .map_err(|e| AgentError::Other(e.to_string()))?;

        let idx = self
            .transport
            .await_action(self.seat, prompt_id)
            .await
            .map_err(|e| AgentError::Other(e.to_string()))?;

        if idx >= legal.len() {
            return Err(AgentError::Other(format!(
                "remote transport returned action_index {} but only {} legal actions were offered",
                idx,
                legal.len()
            )));
        }
        Ok(idx)
    }
}
