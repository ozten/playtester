//! Single-observer Information-Set MCTS (SO-ISMCTS).
//!
//! SO-ISMCTS is the Cowling/Powley/Whitehouse 2012 algorithm: MCTS
//! adapted for imperfect-information games by determinizing the hidden
//! state once per iteration. The *observer* is the root player
//! (`self.player`); reward is always scored from that perspective.
//!
//! Algorithm sketch (one `choose` call):
//!
//! ```text
//! root = NodeArena.alloc()
//! for _ in 0..iterations:
//!     s = Game::determinize(state, observer, rng_tree)
//!     path, s = select_and_expand(root, s, arena, rng_tree, c)
//!     reward = rollout(s, observer, eval, max_depth)
//!     backpropagate(path, reward)
//! return argmax_{legal action a} visits(root.children[a])
//! ```
//!
//! **Per-iteration determinization** is what makes this "information-
//! set" MCTS rather than plain perfect-info MCTS: each iteration sees a
//! different consistent world, and the selection policy averages over
//! those worlds. Because the legal-action set at a given tree node can
//! differ between iterations (e.g., opponent holds different cards),
//! children are keyed by *action* rather than by positional index — see
//! `node.rs`.
//!
//! **One-observer limitation.** All tree nodes share the observer's
//! point of view. When opponents act inside the tree, we don't build a
//! separate subtree for their information set — we still score UCB1
//! from the root observer. This is the "SO" in SO-ISMCTS and the
//! simplest variant; multi-observer (MO-ISMCTS) is a later extension
//! if accuracy needs it.
//!
//! **Tree not reused across turns.** `choose` builds a fresh tree per
//! call. Tree reuse across successive turns is mentioned in the risk
//! memo as a future optimization if R2.3 regresses; we don't do it yet.

pub mod node;
pub mod rollout;
pub mod ucb;

use core::marker::PhantomData;

use async_trait::async_trait;
use playtest_core::{Actor, Agent, AgentError, Game, PlayerId};
use playtest_ports::Rng;
use rand_chacha::ChaCha20Rng;
use rand_chacha::rand_core::{RngCore, SeedableRng};

use crate::eval::EvalFn;

use self::node::{NodeArena, NodeId};
use self::rollout::{RandomRolloutPolicy, RolloutPolicy, terminal_reward};
use self::ucb::ucb1;

/// Configuration for [`ISMCTSAgent`].
#[derive(Debug, Clone, Copy)]
pub struct ISMCTSConfig {
    /// Number of tree-descent iterations per `choose`. Larger = better
    /// play, proportional to wall-clock cost.
    pub iterations: u32,
    /// UCB1 exploration constant. Classical value is `sqrt(2)`; games
    /// with sparse rewards often want 1.4–2.0.
    pub exploration_c: f64,
    /// Maximum plies rolled out before falling back to the eval
    /// function. Set high for short games (Cribbage: ~60 plies caps
    /// one hand cleanly); lower for rollout-cost-sensitive games.
    pub rollout_depth: u32,
    /// Seed for the ISMCTS RNG (tree descent + rollouts).
    pub seed: u64,
}

impl Default for ISMCTSConfig {
    fn default() -> Self {
        Self {
            iterations: 1000,
            exploration_c: std::f64::consts::SQRT_2,
            rollout_depth: 50,
            seed: 0,
        }
    }
}

/// Single-observer Information-Set MCTS agent. Generic over any
/// [`Game`] with a working `determinize` impl.
pub struct ISMCTSAgent<G>
where
    G: Game + ?Sized,
{
    _game: PhantomData<fn() -> G>,
    config: ISMCTSConfig,
    player: PlayerId,
    eval: Option<EvalFn<G>>,
    /// Tree/descent RNG. Seeded from `config.seed` mixed with
    /// `player` so two players constructed with the same seed produce
    /// independent streams.
    tree_rng: ChaCha20Rng,
    /// Rollout RNG. Seeded from `config.seed` mixed with a separate
    /// constant so it's independent from the tree RNG.
    rollout: RandomRolloutPolicy<ChaCha20RngAdapter>,
    /// Arena reused across decisions to amortize the heap cost.
    arena: NodeArena<G::Action>,
}

/// Thin adapter to present `rand_chacha::ChaCha20Rng` as a
/// `playtest_ports::Rng`. We don't use the adapter crate's
/// `ProductionRng` because that would add a dependency on
/// `playtest-adapters` to this crate; the adapter crate is a *sibling*
/// of `playtest-agents` in the architecture and should stay downstream.
///
/// Both implementations wrap the same underlying ChaCha20Rng, so the
/// stream characteristics are identical.
pub struct ChaCha20RngAdapter {
    inner: ChaCha20Rng,
}

impl ChaCha20RngAdapter {
    #[must_use]
    pub fn from_seed(seed: u64) -> Self {
        Self {
            inner: ChaCha20Rng::seed_from_u64(seed),
        }
    }
}

impl Rng for ChaCha20RngAdapter {
    fn next_u64(&mut self) -> u64 {
        self.inner.next_u64()
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
        let span = range.end - range.start;
        Ok(range.start + self.inner.next_u64() % span)
    }
}

impl<G> ISMCTSAgent<G>
where
    G: Game + ?Sized,
    G::Action: core::hash::Hash + Eq,
{
    /// Build an ISMCTS agent with no eval function. At depth cutoff
    /// the rollout returns the neutral 0.5 reward.
    #[must_use]
    pub fn new(config: ISMCTSConfig, player: PlayerId) -> Self {
        Self::build(config, player, None)
    }

    /// Build an ISMCTS agent with an eval function used at the rollout
    /// depth cutoff. Strongly recommended for games where average-
    /// length rollouts exceed `rollout_depth`.
    #[must_use]
    pub fn with_eval(config: ISMCTSConfig, player: PlayerId, eval: EvalFn<G>) -> Self {
        Self::build(config, player, Some(eval))
    }

    fn build(config: ISMCTSConfig, player: PlayerId, eval: Option<EvalFn<G>>) -> Self {
        // Mix the player index into each stream seed so two ISMCTS
        // instances seeded identically but playing different seats
        // don't produce correlated rollouts.
        let base = config.seed;
        let tree_seed = base ^ (u64::from(player).wrapping_mul(0x9E37_79B9_7F4A_7C15));
        let rollout_seed = base ^ (u64::from(player).wrapping_mul(0xBF58_476D_1CE4_E5B9));

        Self {
            _game: PhantomData,
            config,
            player,
            eval,
            tree_rng: ChaCha20Rng::seed_from_u64(tree_seed),
            rollout: RandomRolloutPolicy::new(ChaCha20RngAdapter::from_seed(rollout_seed)),
            arena: NodeArena::new(),
        }
    }

    /// Run the full MCTS search from `state` and return the
    /// best-action index into `legal` (highest visit count; ties broken
    /// by lowest action index within `legal`).
    fn search(
        &mut self,
        game: &G,
        legal: &[G::Action],
        state: &G::State,
    ) -> usize
    where
        G: Game,
        G::State: Clone,
        G::Action: Clone,
    {
        self.arena.clear();
        let root = self.arena.alloc();

        for _ in 0..self.config.iterations {
            // Per-iteration determinization from the observer's POV.
            let mut sample = {
                let mut tree_rng_adapter = TreeRngAdapter {
                    rng: &mut self.tree_rng,
                };
                game.determinize(state, self.player, &mut tree_rng_adapter)
            };

            // Descend + expand.
            let leaf = self.select_and_expand(game, root, &mut sample);

            // Rollout.
            let reward = if let Some(result) = game.game_over(&sample) {
                terminal_reward(&result, self.player)
            } else {
                <RandomRolloutPolicy<ChaCha20RngAdapter> as RolloutPolicy<G>>::rollout(
                    &mut self.rollout,
                    game,
                    &mut sample,
                    self.player,
                    self.eval,
                    self.config.rollout_depth,
                )
            };

            // Backpropagate.
            for node_id in &leaf.path {
                let n = self.arena.get_mut(*node_id);
                n.visits += 1;
                n.total_value += reward;
            }
        }

        // Pick the child of the root with the highest visit count.
        self.argmax_root_visits(root, legal)
    }

    /// Descend the tree from `node` using UCB1, expanding one unvisited
    /// child per iteration. Mutates `state` in place (applies the
    /// action chosen at each step). Returns the leaf path (root first).
    fn select_and_expand(
        &mut self,
        game: &G,
        root: NodeId,
        state: &mut G::State,
    ) -> SelectExpandResult
    where
        G: Game,
        G::Action: Clone,
    {
        let mut path = vec![root];
        let mut current = root;

        loop {
            if game.game_over(state).is_some() {
                return SelectExpandResult { path };
            }
            let actor = game.next_actor(state);
            let acting_player = match actor {
                Actor::Player(p) => p,
                Actor::Chance => {
                    // Resolve chance in-place through the tree rng and
                    // continue — we don't create separate tree nodes
                    // for chance outcomes, we roll them into the
                    // descent (classic MCTS handling).
                    let mut tree_rng_adapter = TreeRngAdapter {
                        rng: &mut self.tree_rng,
                    };
                    match game.resolve_chance(state, &mut tree_rng_adapter) {
                        Ok(ev) => {
                            game.apply_event(state, &ev);
                            continue;
                        }
                        Err(_) => return SelectExpandResult { path },
                    }
                }
            };
            let legal = game.legal_actions(state, acting_player);
            if legal.is_empty() {
                return SelectExpandResult { path };
            }

            // Any unexpanded action? Expand one.
            let unexpanded: Vec<&G::Action> = legal
                .iter()
                .filter(|a| !self.arena.get(current).children.contains_key(*a))
                .collect();
            if !unexpanded.is_empty() {
                // Pick the first unexpanded action in `legal` order.
                // Deterministic given the tree_rng stream — we don't
                // need extra randomness here because the determinized
                // state already carries the per-iteration variation.
                let action = unexpanded[0].clone();
                // Apply action to state.
                if let Ok(events) = game.apply_action(state, acting_player, &action) {
                    for ev in &events {
                        game.apply_event(state, ev);
                    }
                }
                // Allocate a child node.
                let child_id = self.arena.alloc();
                self.arena
                    .get_mut(current)
                    .children
                    .insert(action, child_id);
                path.push(child_id);
                return SelectExpandResult { path };
            }

            // All legal actions expanded — UCB1 select among them.
            let parent_visits = self.arena.get(current).visits;
            let mut best: Option<(&G::Action, NodeId, f64)> = None;
            for action in &legal {
                let child_id = self.arena.get(current).children[action];
                let child = self.arena.get(child_id);
                let score = ucb1(
                    child.mean_value(),
                    child.visits,
                    parent_visits,
                    self.config.exploration_c,
                );
                match best {
                    None => best = Some((action, child_id, score)),
                    Some((_, _, s)) if score > s => best = Some((action, child_id, score)),
                    _ => {}
                }
            }
            let Some((action, child_id, _)) = best else {
                return SelectExpandResult { path };
            };
            let action = action.clone();
            if let Ok(events) = game.apply_action(state, acting_player, &action) {
                for ev in &events {
                    game.apply_event(state, ev);
                }
            }
            path.push(child_id);
            current = child_id;
        }
    }

    /// From the root, pick the legal-action index with the highest
    /// visit count on its child. Ties break on lowest `legal` index.
    /// Actions that were never expanded (no child at all) get a count
    /// of 0 — they only win if every legal action was unexpanded, in
    /// which case the lowest-index one wins.
    fn argmax_root_visits(&self, root: NodeId, legal: &[G::Action]) -> usize
    where
        G::Action: Eq + core::hash::Hash,
    {
        debug_assert!(!legal.is_empty());
        let root_node = self.arena.get(root);
        let mut best_idx = 0usize;
        let mut best_visits: i64 = -1;
        for (i, a) in legal.iter().enumerate() {
            let v = root_node
                .children
                .get(a)
                .map_or(0i64, |id| i64::from(self.arena.get(*id).visits));
            if v > best_visits {
                best_visits = v;
                best_idx = i;
            }
        }
        best_idx
    }
}

struct SelectExpandResult {
    path: Vec<NodeId>,
}

/// Wraps `&mut ChaCha20Rng` behind the `Rng` port without moving it.
struct TreeRngAdapter<'a> {
    rng: &'a mut ChaCha20Rng,
}

impl Rng for TreeRngAdapter<'_> {
    fn next_u64(&mut self) -> u64 {
        self.rng.next_u64()
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
        let span = range.end - range.start;
        Ok(range.start + self.rng.next_u64() % span)
    }
}

impl<G> core::fmt::Debug for ISMCTSAgent<G>
where
    G: Game + ?Sized,
{
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ISMCTSAgent")
            .field("player", &self.player)
            .field("config", &self.config)
            .field("has_eval", &self.eval.is_some())
            .finish_non_exhaustive()
    }
}

#[async_trait]
impl<G> Agent<G> for ISMCTSAgent<G>
where
    G: Game + Sync + Send + Default,
    G::State: Clone + Send + Sync,
    G::PublicView: Send + Sync,
    G::Action: Send + Sync + Clone + Eq + core::hash::Hash,
{
    async fn choose(
        &mut self,
        _view: &G::PublicView,
        legal: &[G::Action],
        state: &G::State,
    ) -> Result<usize, AgentError> {
        if legal.is_empty() {
            return Err(AgentError::Other(
                "ISMCTSAgent::choose called with empty legal slice (engine bug)".into(),
            ));
        }
        if legal.len() == 1 {
            return Ok(0);
        }
        // Every in-tree `Game` impl is zero-sized and `Default`-
        // constructible. Mirroring Greedy/Heuristic's convention, we
        // instantiate the game marker locally rather than threading
        // one through the Agent trait.
        let game = G::default();
        Ok(self.search(&game, legal, state))
    }
}

// ---------------------------------------------------------------------
// Parameterized-form parsing for agent-registry strings like
// "ismcts-cribbage:iter=2000,c=1.4,rollout_depth=30,seed=7".
// ---------------------------------------------------------------------

/// Parse `"key1=value1,key2=value2"` into an [`ISMCTSConfig`], starting
/// from the defaults and overriding named fields. Unknown keys return
/// an error string so the registry can surface typos.
///
/// # Errors
/// Returns `Err(String)` when a key/value pair is malformed or names an
/// unknown field.
pub fn parse_config_overrides(params: &str) -> Result<ISMCTSConfig, String> {
    let mut cfg = ISMCTSConfig::default();
    if params.is_empty() {
        return Ok(cfg);
    }
    for pair in params.split(',') {
        let pair = pair.trim();
        if pair.is_empty() {
            continue;
        }
        let (k, v) = pair
            .split_once('=')
            .ok_or_else(|| format!("expected `key=value`, got `{pair}`"))?;
        let k = k.trim();
        let v = v.trim();
        match k {
            "iter" | "iterations" => {
                cfg.iterations = v
                    .parse::<u32>()
                    .map_err(|e| format!("bad iterations `{v}`: {e}"))?;
            }
            "c" | "exploration_c" => {
                cfg.exploration_c = v
                    .parse::<f64>()
                    .map_err(|e| format!("bad exploration_c `{v}`: {e}"))?;
            }
            "depth" | "rollout_depth" => {
                cfg.rollout_depth = v
                    .parse::<u32>()
                    .map_err(|e| format!("bad rollout_depth `{v}`: {e}"))?;
            }
            "seed" => {
                cfg.seed = v
                    .parse::<u64>()
                    .map_err(|e| format!("bad seed `{v}`: {e}"))?;
            }
            other => return Err(format!("unknown ismcts config key `{other}`")),
        }
    }
    Ok(cfg)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_empty_overrides_yields_defaults() {
        let cfg = parse_config_overrides("").unwrap();
        let dflt = ISMCTSConfig::default();
        assert_eq!(cfg.iterations, dflt.iterations);
        assert!((cfg.exploration_c - dflt.exploration_c).abs() < 1e-9);
        assert_eq!(cfg.rollout_depth, dflt.rollout_depth);
        assert_eq!(cfg.seed, dflt.seed);
    }

    #[test]
    fn parse_overrides_applies_values() {
        let cfg = parse_config_overrides("iter=123,c=1.4,depth=10,seed=9").unwrap();
        assert_eq!(cfg.iterations, 123);
        assert!((cfg.exploration_c - 1.4).abs() < 1e-9);
        assert_eq!(cfg.rollout_depth, 10);
        assert_eq!(cfg.seed, 9);
    }

    #[test]
    fn parse_rejects_unknown_key() {
        let err = parse_config_overrides("banana=5").unwrap_err();
        assert!(err.contains("banana"), "unexpected error: {err}");
    }

    #[test]
    fn parse_rejects_malformed_pair() {
        let err = parse_config_overrides("iter").unwrap_err();
        assert!(err.contains("key=value"));
    }
}
