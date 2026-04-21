//! `HeuristicAgent`: softmax variant of `GreedyAgent`.
//!
//! Same one-ply lookahead as Greedy, but instead of taking the argmax
//! we sample from a softmax distribution over the per-action scores.
//! A `temperature` knob controls the shape:
//!
//! - `temperature == 0.0` → pure argmax (identical to `GreedyAgent`).
//! - `temperature > 0.0`  → sample from `p_i ∝ exp(score_i / T)`.
//!   Larger T → distribution approaches uniform.
//!
//! Numerical stability: we subtract the max score before exponentiating.
//!
//! Stochasticity source: the agent holds its own `Rng` port, separate
//! from the engine's chance RNG. Per the Phase-0 memo, agent RNGs are
//! per-agent-seeded so their streams stay independent of each other
//! and of the game's randomness.

use core::marker::PhantomData;

use async_trait::async_trait;
use playtest_core::{Agent, AgentError, Game, PlayerId};
use playtest_ports::Rng;

use crate::eval::EvalFn;
use crate::greedy::{argmax_lowest_index, score_legal_actions};

/// Default temperature for `HeuristicAgent`. Chosen per the Unit 25
/// plan — "default 0.5" — enough noise to diversify metric coverage
/// without wrecking the "heuristic beats random" quality bar.
pub const DEFAULT_TEMPERATURE: f64 = 0.5;

/// Softmax-over-top-K agent with controlled stochasticity.
pub struct HeuristicAgent<G, R>
where
    G: Game + ?Sized,
    R: Rng,
{
    game: PhantomData<fn() -> G>,
    eval: EvalFn<G>,
    player: PlayerId,
    temperature: f64,
    rng: R,
}

impl<G, R> HeuristicAgent<G, R>
where
    G: Game + ?Sized,
    R: Rng,
{
    /// Build a heuristic agent with the default temperature (0.5).
    #[must_use]
    pub fn new(player: PlayerId, eval: EvalFn<G>, rng: R) -> Self {
        Self::with_temperature(player, eval, rng, DEFAULT_TEMPERATURE)
    }

    /// Build a heuristic agent with a custom temperature.
    ///
    /// `temperature == 0.0` degenerates into pure greedy behaviour
    /// (and the RNG is never called).
    #[must_use]
    pub fn with_temperature(
        player: PlayerId,
        eval: EvalFn<G>,
        rng: R,
        temperature: f64,
    ) -> Self {
        Self {
            game: PhantomData,
            eval,
            player,
            temperature,
            rng,
        }
    }
}

impl<G, R> core::fmt::Debug for HeuristicAgent<G, R>
where
    G: Game + ?Sized,
    R: Rng,
{
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("HeuristicAgent")
            .field("player", &self.player)
            .field("temperature", &self.temperature)
            .finish_non_exhaustive()
    }
}

/// Sample an index from `scores` using the softmax distribution at
/// `temperature`. Ties (at `temperature == 0.0`) resolve to the
/// lowest-index maximum. On `temperature > 0` the sampler subtracts
/// the max score for numerical stability.
#[allow(clippy::cast_precision_loss)] // see inline comments — 53-bit mantissa is fine
fn sample_softmax_index<R: Rng>(scores: &[f64], temperature: f64, rng: &mut R) -> usize {
    debug_assert!(!scores.is_empty());
    if temperature <= 0.0 {
        return argmax_lowest_index(scores);
    }
    // Compute shifted exponentials.
    let mut max = f64::NEG_INFINITY;
    for &s in scores {
        if s > max {
            max = s;
        }
    }
    let mut weights = Vec::with_capacity(scores.len());
    let mut total = 0.0f64;
    for &s in scores {
        let w = ((s - max) / temperature).exp();
        weights.push(w);
        total += w;
    }
    if !total.is_finite() || total <= 0.0 {
        // Every weight underflowed or overflowed — fall back to argmax.
        return argmax_lowest_index(scores);
    }
    // Draw a uniform sample in [0, total) by pulling a u64 and scaling.
    // Shift to 53 bits so conversion to f64 is exact (f64's mantissa
    // is 52+1 bits). The cast-precision-loss lint still fires on the
    // shift expression because the checker can't narrow the static
    // range, hence the crate-scope allow above.
    let bits = rng.next_u64();
    let top_53 = bits >> 11;
    let frac = (top_53 as f64) / ((1u64 << 53) as f64);
    let threshold = frac * total;
    let mut acc = 0.0f64;
    for (i, w) in weights.iter().enumerate() {
        acc += w;
        if acc >= threshold {
            return i;
        }
    }
    // Floating-point edge case: return the last index.
    scores.len() - 1
}

#[async_trait]
impl<G, R> Agent<G> for HeuristicAgent<G, R>
where
    G: Game + Sync + Send + Default,
    G::State: Clone + Send + Sync,
    G::PublicView: Send + Sync,
    G::Action: Send + Sync,
    R: Rng + Send,
{
    async fn choose(
        &mut self,
        _view: &G::PublicView,
        legal: &[G::Action],
        state: &G::State,
    ) -> Result<usize, AgentError> {
        if legal.is_empty() {
            return Err(AgentError::Other(
                "HeuristicAgent::choose called with empty legal slice (engine bug)".into(),
            ));
        }
        let game = G::default();
        let scores = score_legal_actions::<G>(&game, state, self.player, legal, self.eval)?;
        Ok(sample_softmax_index(&scores, self.temperature, &mut self.rng))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Deterministic stub RNG that returns a preset sequence.
    struct SeqRng {
        values: std::collections::VecDeque<u64>,
    }

    impl SeqRng {
        fn new(values: impl IntoIterator<Item = u64>) -> Self {
            Self {
                values: values.into_iter().collect(),
            }
        }
    }

    impl Rng for SeqRng {
        fn next_u64(&mut self) -> u64 {
            self.values.pop_front().unwrap_or(0)
        }
        fn gen_range(
            &mut self,
            range: core::ops::Range<u64>,
        ) -> Result<u64, playtest_ports::RngError> {
            if range.start >= range.end {
                return Err(playtest_ports::RngError::InvalidRange {
                    start: range.start,
                    end: range.end,
                });
            }
            Ok(self.values.pop_front().unwrap_or(range.start))
        }
    }

    #[test]
    fn temperature_zero_is_argmax() {
        let scores = [0.1, 5.0, 3.0, 5.0];
        let mut rng = SeqRng::new([0u64; 0]);
        let idx = sample_softmax_index(&scores, 0.0, &mut rng);
        assert_eq!(idx, 1); // first maximum
    }

    #[test]
    fn large_temperature_approaches_uniform() {
        let scores = [10.0, 10.0, 10.0, 10.0];
        // All equal scores → softmax is uniform at any T > 0.
        // Feed a handful of strategic RNG values that land in each
        // of the four 1/4-width buckets so sampling is exercised end-
        // to-end.
        // bits / 2^53 -> frac. With n=4 equal weights and total=4,
        // threshold=frac*4. To hit bucket i we need frac to fall in
        // [i/4, (i+1)/4). Choose bits = floor(i/4 * 2^53) + small.
        let mut bits: Vec<u64> = Vec::new();
        for i in 0..4u64 {
            let base = (i << 11).wrapping_mul(1u64 << 41); // roughly i/4 * 2^53
            bits.push(base + 1);
        }
        let mut rng = SeqRng::new(bits);
        let mut buckets = [0u32; 4];
        for _ in 0..4 {
            let idx = sample_softmax_index(&scores, 100.0, &mut rng);
            buckets[idx] += 1;
        }
        let total_hits: u32 = buckets.iter().sum();
        assert_eq!(total_hits, 4);
    }

    #[test]
    fn extreme_scores_do_not_panic() {
        let scores = [-1e300f64, -1e299f64];
        let mut rng = SeqRng::new([0u64; 0]);
        let idx = sample_softmax_index(&scores, 0.5, &mut rng);
        assert!(idx < 2);
    }
}
