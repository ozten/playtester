//! Rollout policies for ISMCTS.
//!
//! A rollout takes a determinized state (already descended through the
//! tree), plays it out to a terminal state or a depth cutoff, and
//! returns a reward from the **root player's** perspective, scaled to
//! `[0, 1]` where 1.0 = "root player won".
//!
//! The default [`RandomRolloutPolicy`] picks a legal action uniformly
//! at random each ply. For chance nodes (`Actor::Chance`), it calls the
//! game's `resolve_chance` so randomness is threaded through the
//! rollout RNG rather than the engine's game RNG (which never runs
//! inside search).
//!
//! When the depth cutoff fires, the policy invokes the optional eval
//! function (Unit 25's `EvalFn<G>`) to estimate the state's value,
//! squashing the score to `[0, 1]` via `normalize_eval`. Agents that
//! don't supply an eval fall back to 0.5 — a neutral "no information"
//! reward — which lets ISMCTS remain correct (if not as strong) on
//! games where no eval is provided.

use playtest_core::{Actor, Game, GameResult, PlayerId};
use playtest_ports::Rng;

use crate::eval::EvalFn;

/// A rollout policy: given a state already cloned from the root's
/// determinization, produce a reward in `[0, 1]` for `observer`.
pub trait RolloutPolicy<G>
where
    G: Game + ?Sized,
{
    /// Roll out from `state` for at most `max_depth` plies.
    fn rollout(
        &mut self,
        game: &G,
        state: &mut G::State,
        observer: PlayerId,
        eval: Option<EvalFn<G>>,
        max_depth: u32,
    ) -> f64;
}

/// Default random-playout rollout policy.
///
/// Holds a dedicated rollout RNG so rollouts don't consume the tree
/// descent RNG (determinization draws from a separate stream).
pub struct RandomRolloutPolicy<R: Rng> {
    rng: R,
}

impl<R: Rng> RandomRolloutPolicy<R> {
    pub fn new(rng: R) -> Self {
        Self { rng }
    }

    pub fn rng_mut(&mut self) -> &mut R {
        &mut self.rng
    }
}

impl<G, R> RolloutPolicy<G> for RandomRolloutPolicy<R>
where
    G: Game + ?Sized,
    G::State: Clone,
    R: Rng,
{
    fn rollout(
        &mut self,
        game: &G,
        state: &mut G::State,
        observer: PlayerId,
        eval: Option<EvalFn<G>>,
        max_depth: u32,
    ) -> f64 {
        for _ in 0..max_depth {
            if let Some(result) = game.game_over(state) {
                return terminal_reward(&result, observer);
            }
            match game.next_actor(state) {
                Actor::Chance => {
                    // Resolve chance through our rollout RNG; if it
                    // fails the game is in a bad state — fall back to
                    // eval/0.5 to let the caller continue gracefully.
                    match game.resolve_chance(state, &mut self.rng) {
                        Ok(ev) => game.apply_event(state, &ev),
                        Err(_) => return eval_or_neutral(game, state, observer, eval),
                    }
                }
                Actor::Player(p) => {
                    let legal = game.legal_actions(state, p);
                    if legal.is_empty() {
                        // Deadlock — treat as no-information.
                        return eval_or_neutral(game, state, observer, eval);
                    }
                    let n = u64::try_from(legal.len()).unwrap_or(1);
                    let idx = match self.rng.gen_range(0..n) {
                        Ok(x) => usize::try_from(x).unwrap_or(0),
                        Err(_) => 0,
                    };
                    match game.apply_action(state, p, &legal[idx]) {
                        Ok(events) => {
                            for ev in &events {
                                game.apply_event(state, ev);
                            }
                        }
                        Err(_) => return eval_or_neutral(game, state, observer, eval),
                    }
                }
            }
        }
        // Depth cutoff reached; fall back to eval (or neutral).
        eval_or_neutral(game, state, observer, eval)
    }
}

/// Reward for a terminal state, from `observer`'s perspective.
/// 1.0 = observer won; 0.0 = observer lost; 0.5 = tie/draw.
#[must_use]
pub fn terminal_reward(result: &GameResult, observer: PlayerId) -> f64 {
    match result.winner {
        Some(w) if w == observer => 1.0,
        Some(_) => 0.0,
        None => 0.5,
    }
}

/// Derive a `[0, 1]` reward from the eval function if provided,
/// otherwise return the neutral 0.5. We pair eval with a running
/// per-state normalizer via `normalize_eval`.
fn eval_or_neutral<G>(
    game: &G,
    state: &G::State,
    observer: PlayerId,
    eval: Option<EvalFn<G>>,
) -> f64
where
    G: Game + ?Sized,
{
    let Some(e) = eval else {
        return 0.5;
    };
    let view = game.public_view(state, observer);
    let raw = e(&view, observer);
    normalize_eval(raw)
}

/// Squash an arbitrary real score to `[0, 1]` with a logistic.
/// The scale constant is tuned so typical eval magnitudes (tens of
/// points for Cribbage, tens-to-hundreds for ShipWreck) map into the
/// rough range `[0.1, 0.9]` — preserving relative order without
/// saturating.
///
/// Formula: `sigma(x / SCALE)` where `sigma(y) = 1 / (1 + exp(-y))`.
#[must_use]
pub fn normalize_eval(raw: f64) -> f64 {
    const SCALE: f64 = 200.0;
    let y = raw / SCALE;
    let clamped = y.clamp(-50.0, 50.0); // guard against overflow
    1.0 / (1.0 + (-clamped).exp())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terminal_reward_maps_winner_correctly() {
        let r_win = GameResult {
            winner: Some(0),
            reason: playtest_core::EndReason::Victory,
            scores: vec![1, 0],
        };
        assert!((terminal_reward(&r_win, 0) - 1.0).abs() < f64::EPSILON);
        assert!(terminal_reward(&r_win, 1).abs() < f64::EPSILON);
        let r_draw = GameResult {
            winner: None,
            reason: playtest_core::EndReason::Draw,
            scores: vec![0, 0],
        };
        assert!((terminal_reward(&r_draw, 0) - 0.5).abs() < f64::EPSILON);
    }

    #[test]
    fn normalize_eval_maps_zero_to_half() {
        assert!((normalize_eval(0.0) - 0.5).abs() < 1e-9);
    }

    #[test]
    fn normalize_eval_bounded_in_unit_interval() {
        for &raw in &[-1e6f64, -100.0, -1.0, 0.0, 1.0, 100.0, 1e6f64] {
            let v = normalize_eval(raw);
            assert!((0.0..=1.0).contains(&v), "normalize_eval({raw}) = {v}");
        }
    }

    #[test]
    fn normalize_eval_monotonic() {
        let a = normalize_eval(-10.0);
        let b = normalize_eval(0.0);
        let c = normalize_eval(10.0);
        assert!(a < b && b < c);
    }
}
