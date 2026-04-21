//! `RandomAgent` behavior: index bounds, determinism under seeded RNG,
//! uniformity at 10K draws, and error on empty legal slice.

use async_trait::async_trait;
use playtest_adapters::{ProductionRng, StubRng};
use playtest_agents::RandomAgent;
use playtest_core::{Actor, Agent, AgentError, Game, GameError, GameResult, PlayerId};
use playtest_ports::{Rng, RngError};
use serde::Serialize;

/// Degenerate game used only so `Agent<G>` has a `G` to parameterize over.
/// It never actually plays — the tests drive `choose` directly.
struct NullGame;

#[derive(Clone, PartialEq, Eq)]
struct Noop;

#[derive(Clone, Serialize)]
struct NoopEvent;

impl Game for NullGame {
    type State = ();
    type Action = Noop;
    type Event = NoopEvent;
    type PublicView = ();
    type Config = ();

    fn initial_state(&self, _seed: u64, (): &()) {}
    fn next_actor(&self, (): &()) -> Actor {
        Actor::Player(0)
    }
    fn legal_actions(&self, (): &(), _p: PlayerId) -> Vec<Noop> {
        Vec::new()
    }
    fn apply_action(&self, (): &(), _p: PlayerId, _a: &Noop) -> Result<Vec<NoopEvent>, GameError> {
        unreachable!()
    }
    fn resolve_chance(&self, (): &(), _rng: &mut dyn Rng) -> Result<NoopEvent, GameError> {
        unreachable!()
    }
    fn apply_event(&self, (): &mut (), _e: &NoopEvent) {}
    fn public_view(&self, (): &(), _p: PlayerId) {}
    fn determinize(&self, (): &(), _observer: PlayerId, _rng: &mut dyn Rng) {}
    fn game_over(&self, (): &()) -> Option<GameResult> {
        None
    }
}

fn legal(n: usize) -> Vec<Noop> {
    (0..n).map(|_| Noop).collect()
}

#[tokio::test]
async fn choose_returns_index_in_range() {
    let mut agent: RandomAgent<NullGame, _> = RandomAgent::new(StubRng::seeded(42));
    let actions = legal(4);
    for _ in 0..64 {
        let idx = agent.choose(&(), &actions).await.unwrap();
        assert!(idx < 4, "RandomAgent returned out-of-range index {idx}");
    }
}

#[tokio::test]
async fn identical_seeds_produce_identical_choice_sequences() {
    let mut a: RandomAgent<NullGame, _> = RandomAgent::new(StubRng::seeded(2026));
    let mut b: RandomAgent<NullGame, _> = RandomAgent::new(StubRng::seeded(2026));
    let actions = legal(7);
    for _ in 0..128 {
        let ia = a.choose(&(), &actions).await.unwrap();
        let ib = b.choose(&(), &actions).await.unwrap();
        assert_eq!(ia, ib, "identical seeds diverged");
    }
}

#[tokio::test]
async fn uniform_distribution_over_10k_draws() {
    // Three-way split, 10K draws: each bucket's expected count is 3333.
    // `ChaCha20Rng` is well-tested; we only need a very loose chi-square
    // sanity check to catch ordering / modulo bugs in `RandomAgent`.
    let mut agent: RandomAgent<NullGame, _> = RandomAgent::new(ProductionRng::from_seed(1));
    let actions = legal(3);

    let mut counts = [0u32; 3];
    for _ in 0..10_000 {
        let idx = agent.choose(&(), &actions).await.unwrap();
        counts[idx] += 1;
    }

    // Each bucket expected ~3333. With 10K draws, chi-square at 2 df is
    // well below 20 for any plausible seed. We use a wide tolerance
    // (each bucket within +/- 500 of expected) because the test must
    // pass on every CI run.
    for (i, &c) in counts.iter().enumerate() {
        let deviation = i64::from(c) - 3333;
        assert!(
            deviation.abs() < 500,
            "bucket {i} had {c} draws (deviation {deviation}); expected ~3333"
        );
    }
}

#[tokio::test]
async fn empty_legal_slice_returns_agent_error() {
    let mut agent: RandomAgent<NullGame, _> = RandomAgent::new(StubRng::seeded(1));
    let err = agent.choose(&(), &[]).await.unwrap_err();
    assert!(matches!(err, AgentError::Other(_)));
}

/// Wrap an Rng so we can confirm the agent propagates an Rng error
/// as an AgentError (not a panic).
struct AlwaysInvalidRng;

impl Rng for AlwaysInvalidRng {
    fn next_u64(&mut self) -> u64 {
        0
    }
    fn gen_range(&mut self, _range: core::ops::Range<u64>) -> Result<u64, RngError> {
        Err(RngError::InvalidRange { start: 0, end: 0 })
    }
}

// `RandomAgent` needs `R: Rng + Send`; unit-struct types are Send by default.
#[async_trait]
trait _ConstraintProof: Send {}
#[async_trait]
impl _ConstraintProof for AlwaysInvalidRng {}

#[tokio::test]
async fn rng_port_error_is_surfaced_as_agent_error() {
    let mut agent: RandomAgent<NullGame, _> = RandomAgent::new(AlwaysInvalidRng);
    let actions = legal(3);
    let err = agent.choose(&(), &actions).await.unwrap_err();
    assert!(matches!(err, AgentError::Other(_)));
}
