//! `GreedyAgent`: one-ply lookahead using an `EvalFn`.
//!
//! For every legal action, the agent simulates applying it to a clone
//! of the full game state, folds the resulting events into that clone,
//! builds the post-action public view, and scores it with the
//! evaluation function. The argmax wins; ties break deterministically
//! on the lowest legal-action index.
//!
//! `GreedyAgent` is stateless across turns — it owns a zero-sized
//! `Game` instance and the eval function pointer. No RNG needed: the
//! strategy is fully deterministic.
//!
//! Simulation sketch:
//!
//! ```text
//! for (i, action) in legal.iter().enumerate():
//!   cloned = state.clone()
//!   events = game.apply_action(&cloned, self.player, action)?
//!   for e in events: game.apply_event(&mut cloned, &e)
//!   view = game.public_view(&cloned, self.player)
//!   score[i] = (self.eval)(&view, self.player)
//! return argmax(score) with lowest-index tie-break
//! ```

use core::marker::PhantomData;

use async_trait::async_trait;
use playtest_core::{Agent, AgentError, Game, PlayerId};

use crate::eval::EvalFn;

/// One-ply greedy action selector.
///
/// `G` is the game the agent plays. Evaluation is carried as a function
/// pointer of type [`EvalFn<G>`].
pub struct GreedyAgent<G>
where
    G: Game + ?Sized,
{
    game: PhantomData<fn() -> G>,
    eval: EvalFn<G>,
    player: PlayerId,
}

impl<G> GreedyAgent<G>
where
    G: Game + ?Sized,
{
    /// Construct a greedy agent that plays for `player` using `eval`.
    #[must_use]
    pub fn new(player: PlayerId, eval: EvalFn<G>) -> Self {
        Self {
            game: PhantomData,
            eval,
            player,
        }
    }
}

impl<G> core::fmt::Debug for GreedyAgent<G>
where
    G: Game + ?Sized,
{
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("GreedyAgent")
            .field("player", &self.player)
            .finish_non_exhaustive()
    }
}

/// Score every legal action and return a `Vec<f64>` aligned with
/// `legal`. Shared by [`GreedyAgent`] and [`crate::HeuristicAgent`].
pub(crate) fn score_legal_actions<G>(
    game: &G,
    state: &G::State,
    player: PlayerId,
    legal: &[G::Action],
    eval: EvalFn<G>,
) -> Result<Vec<f64>, AgentError>
where
    G: Game + ?Sized,
    G::State: Clone,
{
    let mut scores = Vec::with_capacity(legal.len());
    for action in legal {
        let mut cloned = state.clone();
        let events = game.apply_action(&cloned, player, action).map_err(|e| {
            AgentError::Other(format!(
                "GreedyAgent/HeuristicAgent: apply_action rejected candidate: {e}"
            ))
        })?;
        for ev in &events {
            game.apply_event(&mut cloned, ev);
        }
        let view = game.public_view(&cloned, player);
        let s = eval(&view, player);
        if s.is_nan() {
            return Err(AgentError::Other(
                "eval function returned NaN — cannot rank actions".into(),
            ));
        }
        scores.push(s);
    }
    Ok(scores)
}

/// Pick the index of the maximum score. Ties break on the lowest index.
pub(crate) fn argmax_lowest_index(scores: &[f64]) -> usize {
    debug_assert!(!scores.is_empty(), "argmax called on empty scores");
    let mut best_idx = 0;
    let mut best_score = scores[0];
    for (i, &s) in scores.iter().enumerate().skip(1) {
        if s > best_score {
            best_score = s;
            best_idx = i;
        }
    }
    best_idx
}

#[async_trait]
impl<G> Agent<G> for GreedyAgent<G>
where
    G: Game + Sync + Send + Default,
    G::State: Clone + Send + Sync,
    G::PublicView: Send + Sync,
    G::Action: Send + Sync,
{
    async fn choose(
        &mut self,
        _view: &G::PublicView,
        legal: &[G::Action],
        state: &G::State,
    ) -> Result<usize, AgentError> {
        if legal.is_empty() {
            return Err(AgentError::Other(
                "GreedyAgent::choose called with empty legal slice (engine bug)".into(),
            ));
        }
        // `G::default()` instantiates a zero-sized game marker. Both
        // `CribbageGame` and `ShipWreckGame` are zero-sized — `Default`
        // is the cheap way to thread an owned instance into
        // `score_legal_actions` without forcing the caller to pass one.
        let game = G::default();
        let scores = score_legal_actions::<G>(&game, state, self.player, legal, self.eval)?;
        Ok(argmax_lowest_index(&scores))
    }
}
