//! `SharedLlmAgent<G>` — an `Arc<Mutex<LlmAgent<G>>>` wrapper that
//! implements both `Agent<G>` (for the game loop) and
//! `PostGameCritic<G>` (for the post-game pass) so one concrete agent
//! state is shared between the two roles.
//!
//! The gameplay path mutates the underlying `LlmAgent`'s scratch and
//! tick; the critique path reads the same scratch to embed in the
//! questionnaire prompt. Holding the agent via `Arc<Mutex<_>>` is the
//! established codebase pattern for sharing a mut-self port across
//! roles (see `docs/solutions/architecture-patterns/sharing-mut-self-port-via-arc-mutex-2026-04-23.md`).
//!
//! In practice, contention is zero — the game loop calls `choose`
//! serially during gameplay, and the critic calls `post_game_critique`
//! serially after gameplay. The mutex is there for correctness against
//! the type system, not for concurrent access.

use core::marker::PhantomData;
use std::sync::Arc;

use async_trait::async_trait;
use playtest_core::{Agent, AgentError, Game, GameResult, PlayerId};
use serde::Serialize;
use tokio::sync::Mutex;

use super::sidecar::CritiqueSidecar;
use super::spec::QuestionnaireSpec;
use crate::llm::agent::LlmAgent;

/// The post-game critique role. Implemented by [`SharedLlmAgent`];
/// non-LLM agents do not get a critic handle (the dispatcher simply
/// stores `None` for those seats).
#[async_trait]
pub trait PostGameCritic<G: Game + ?Sized>: Send + Sync {
    async fn post_game_critique(
        &self,
        view: &G::PublicView,
        result: &GameResult,
        spec: &QuestionnaireSpec,
        sidecar: &CritiqueSidecar,
        persona_addendum: Option<&str>,
    ) -> Result<(), AgentError>;
}

/// A handle to a shared `LlmAgent<G>` — one instance, two roles.
///
/// Construct once via [`SharedLlmAgent::new`]; call
/// [`SharedLlmAgent::clone_handle`] to get a second handle for the
/// critic role. Both handles point at the same inner mutex, so the
/// gameplay path's mutations to `scratch` are visible to the critic
/// path.
pub struct SharedLlmAgent<G: Game + ?Sized> {
    inner: Arc<Mutex<LlmAgent<G>>>,
    _game: PhantomData<fn() -> G>,
}

impl<G: Game + ?Sized> SharedLlmAgent<G> {
    #[must_use]
    pub fn new(agent: LlmAgent<G>) -> Self {
        Self {
            inner: Arc::new(Mutex::new(agent)),
            _game: PhantomData,
        }
    }

    /// Get another handle pointing at the same underlying agent.
    #[must_use]
    pub fn clone_handle(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
            _game: PhantomData,
        }
    }

    /// Acquire a blocking lock — test-only, for assertions that need
    /// to inspect the inner agent's state.
    #[cfg(test)]
    pub async fn lock(&self) -> tokio::sync::MutexGuard<'_, LlmAgent<G>> {
        self.inner.lock().await
    }
}

impl<G: Game + ?Sized> core::fmt::Debug for SharedLlmAgent<G> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("SharedLlmAgent")
            .field("strong_count", &Arc::strong_count(&self.inner))
            .finish_non_exhaustive()
    }
}

#[async_trait]
impl<G> Agent<G> for SharedLlmAgent<G>
where
    G: Game + ?Sized + Send + Sync + 'static,
    G::State: Send + Sync + 'static,
    G::PublicView: Send + Sync + Serialize + 'static,
    G::Action: Send + Sync + Serialize + 'static,
{
    async fn choose(
        &mut self,
        view: &G::PublicView,
        legal: &[G::Action],
        state: &G::State,
    ) -> Result<usize, AgentError> {
        self.inner.lock().await.choose(view, legal, state).await
    }
}

#[async_trait]
impl<G> PostGameCritic<G> for SharedLlmAgent<G>
where
    G: Game + ?Sized + Send + Sync + 'static,
    G::PublicView: Send + Sync + Serialize + 'static,
{
    async fn post_game_critique(
        &self,
        view: &G::PublicView,
        result: &GameResult,
        spec: &QuestionnaireSpec,
        sidecar: &CritiqueSidecar,
        persona_addendum: Option<&str>,
    ) -> Result<(), AgentError> {
        let guard = self.inner.lock().await;
        guard
            .post_game_critique(view, result, spec, sidecar, persona_addendum)
            .await
    }
}

/// Constructor helper used by the registry: takes a pre-built
/// `LlmAgent<G>` and returns both trait-object handles pointing at
/// the same shared state.
#[must_use]
pub fn build_shared_handles<G>(
    agent: LlmAgent<G>,
) -> (Box<dyn Agent<G>>, Box<dyn PostGameCritic<G>>)
where
    G: Game + ?Sized + Send + Sync + 'static,
    G::State: Send + Sync + 'static,
    G::PublicView: Send + Sync + Serialize + 'static,
    G::Action: Send + Sync + Serialize + 'static,
{
    let shared = SharedLlmAgent::new(agent);
    let agent_handle: Box<dyn Agent<G>> = Box::new(shared.clone_handle());
    let critic_handle: Box<dyn PostGameCritic<G>> = Box::new(shared);
    (agent_handle, critic_handle)
}

// Silence unused-import lints when this module is compiled without
// the PlayerId path being exercised directly — the trait bounds on
// the blanket impls need Game::PublicView but not PlayerId.
#[allow(dead_code)]
fn _seat_id_typesig_silencer(_p: PlayerId) {}
