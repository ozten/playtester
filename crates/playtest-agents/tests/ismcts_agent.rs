//! `ISMCTSAgent` correctness + convergence tests.
//!
//! We build minimal test games here rather than depending on the real
//! Cribbage/ShipWreck crates — this keeps the test hermetic and lets us
//! construct state configurations that exercise specific algorithm
//! paths (dominant action, depth cutoff, eval fallback, etc.).

use core::ops::Range;

use playtest_agents::{EvalFn, ISMCTSAgent, ISMCTSConfig};
use playtest_core::{
    Actor, Agent, AgentError, EndReason, Game, GameError, GameResult, PlayerId,
};
use playtest_ports::{Rng, RngError};
use serde::Serialize;

// ---------------------------------------------------------------------
// "Pick" game: deterministic test game. Each player picks a value; after
// a fixed number of turns, whoever picked the higher sum wins.
// Actions: {0, 1, 2, 3}. Higher is always better.
// ---------------------------------------------------------------------

#[derive(Clone, Default)]
struct PickState {
    scores: [i32; 2],
    next_player: PlayerId,
    turn: u32,
}

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
struct Pick(i32);

#[derive(Clone, Serialize)]
struct Picked {
    player: PlayerId,
    value: i32,
}

#[derive(Default, Debug, Clone, Copy)]
struct PickGame;

impl Game for PickGame {
    type State = PickState;
    type Action = Pick;
    type Event = Picked;
    type PublicView = PickState;
    type Config = ();

    fn initial_state(&self, _seed: u64, (): &()) -> PickState {
        PickState::default()
    }
    fn next_actor(&self, s: &PickState) -> Actor {
        Actor::Player(s.next_player)
    }
    fn legal_actions(&self, _s: &PickState, _p: PlayerId) -> Vec<Pick> {
        vec![Pick(0), Pick(1), Pick(2), Pick(3)]
    }
    fn apply_action(
        &self,
        _s: &PickState,
        p: PlayerId,
        a: &Pick,
    ) -> Result<Vec<Picked>, GameError> {
        Ok(vec![Picked {
            player: p,
            value: a.0,
        }])
    }
    fn resolve_chance(&self, _s: &PickState, _r: &mut dyn Rng) -> Result<Picked, GameError> {
        unreachable!()
    }
    fn apply_event(&self, s: &mut PickState, e: &Picked) {
        s.scores[e.player as usize] += e.value;
        s.next_player = 1 - e.player;
        s.turn += 1;
    }
    fn public_view(&self, s: &PickState, _p: PlayerId) -> PickState {
        s.clone()
    }
    fn determinize(&self, s: &PickState, _o: PlayerId, _r: &mut dyn Rng) -> PickState {
        s.clone()
    }
    fn game_over(&self, s: &PickState) -> Option<GameResult> {
        if s.turn >= 8 {
            let (w, reason) = match s.scores[0].cmp(&s.scores[1]) {
                std::cmp::Ordering::Greater => (Some(0u8), EndReason::Victory),
                std::cmp::Ordering::Less => (Some(1u8), EndReason::Victory),
                std::cmp::Ordering::Equal => (None, EndReason::Draw),
            };
            Some(GameResult {
                winner: w,
                reason,
                scores: vec![s.scores[0], s.scores[1]],
            })
        } else {
            None
        }
    }
}

fn pick_eval(view: &<PickGame as Game>::PublicView, player: PlayerId) -> f64 {
    f64::from(view.scores[player as usize] - view.scores[1 - player as usize])
}

const PICK_EVAL: EvalFn<PickGame> = pick_eval;

// ---------------------------------------------------------------------
// "Bias" game: a degenerate single-decision game where one action
// deterministically yields a win for the agent and all others yield a
// loss. Used to assert that the argmax converges to the correct branch.
// ---------------------------------------------------------------------

#[derive(Clone, Default)]
struct BiasState {
    done: Option<PlayerId>, // winner (or None while pending)
    choice: Option<i32>,
}

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
struct BiasAction(i32);

#[derive(Clone, Serialize)]
struct BiasEvent {
    player: PlayerId,
    choice: i32,
}

#[derive(Default, Debug, Clone, Copy)]
struct BiasGame;

impl Game for BiasGame {
    type State = BiasState;
    type Action = BiasAction;
    type Event = BiasEvent;
    type PublicView = BiasState;
    type Config = ();

    fn initial_state(&self, _seed: u64, (): &()) -> BiasState {
        BiasState::default()
    }
    fn next_actor(&self, _s: &BiasState) -> Actor {
        Actor::Player(0)
    }
    fn legal_actions(&self, s: &BiasState, _p: PlayerId) -> Vec<BiasAction> {
        if s.done.is_some() {
            Vec::new()
        } else {
            vec![BiasAction(0), BiasAction(1), BiasAction(2)]
        }
    }
    fn apply_action(
        &self,
        _s: &BiasState,
        p: PlayerId,
        a: &BiasAction,
    ) -> Result<Vec<BiasEvent>, GameError> {
        Ok(vec![BiasEvent {
            player: p,
            choice: a.0,
        }])
    }
    fn resolve_chance(&self, _s: &BiasState, _r: &mut dyn Rng) -> Result<BiasEvent, GameError> {
        unreachable!()
    }
    fn apply_event(&self, s: &mut BiasState, e: &BiasEvent) {
        s.choice = Some(e.choice);
        // Only choice 0 wins for player 0; all others lose.
        s.done = Some(u8::from(e.choice != 0));
    }
    fn public_view(&self, s: &BiasState, _p: PlayerId) -> BiasState {
        s.clone()
    }
    fn determinize(&self, s: &BiasState, _o: PlayerId, _r: &mut dyn Rng) -> BiasState {
        s.clone()
    }
    fn game_over(&self, s: &BiasState) -> Option<GameResult> {
        s.done.map(|w| GameResult {
            winner: Some(w),
            reason: EndReason::Victory,
            scores: vec![
                i32::from(w == 0),
                i32::from(w == 1),
            ],
        })
    }
}

// Eval: +1 for action-0 outcomes, -1 otherwise. Used in the
// "eval fallback is used" test.
fn bias_eval(view: &<BiasGame as Game>::PublicView, _player: PlayerId) -> f64 {
    match view.choice {
        Some(0) => 1000.0,
        Some(_) => -1000.0,
        None => 0.0,
    }
}

const BIAS_EVAL: EvalFn<BiasGame> = bias_eval;

// ---------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------

#[tokio::test]
async fn dominant_action_convergence() {
    // BiasGame: action 0 → win; 1, 2 → loss. ISMCTS at modest budget
    // should pick 0 every time.
    let game = BiasGame;
    let state = game.initial_state(0, &());
    let view = game.public_view(&state, 0);
    let legal = game.legal_actions(&state, 0);

    let cfg = ISMCTSConfig {
        iterations: 500,
        exploration_c: std::f64::consts::SQRT_2,
        rollout_depth: 10,
        seed: 42,
    };
    let mut agent: ISMCTSAgent<BiasGame> = ISMCTSAgent::new(cfg, 0);
    let idx = agent.choose(&view, &legal, &state).await.unwrap();
    assert_eq!(
        idx, 0,
        "ISMCTS should pick the winning action; chose {idx:?}"
    );
}

#[tokio::test]
async fn single_legal_action_returns_zero_without_iterating() {
    let game = PickGame;
    // Hand-built state — game not over, single legal action provided.
    let state = PickState::default();
    let view = game.public_view(&state, 0);
    let legal = vec![Pick(42)];
    let cfg = ISMCTSConfig {
        iterations: 1,
        exploration_c: 1.0,
        rollout_depth: 1,
        seed: 0,
    };
    let mut agent: ISMCTSAgent<PickGame> = ISMCTSAgent::with_eval(cfg, 0, PICK_EVAL);
    assert_eq!(agent.choose(&view, &legal, &state).await.unwrap(), 0);
}

#[tokio::test]
async fn empty_legal_slice_returns_error() {
    let game = PickGame;
    let state = game.initial_state(0, &());
    let view = game.public_view(&state, 0);
    let cfg = ISMCTSConfig::default();
    let mut agent: ISMCTSAgent<PickGame> = ISMCTSAgent::new(cfg, 0);
    let err = agent.choose(&view, &[], &state).await.unwrap_err();
    assert!(matches!(err, AgentError::Other(_)));
}

#[tokio::test]
async fn rollout_depth_cutoff_still_returns_valid_index() {
    // Depth 1 → every iteration immediately hits the depth cap and
    // falls back to eval. ISMCTS should still return a legal index.
    let game = PickGame;
    let state = game.initial_state(0, &());
    let view = game.public_view(&state, 0);
    let legal = game.legal_actions(&state, 0);
    let cfg = ISMCTSConfig {
        iterations: 100,
        exploration_c: std::f64::consts::SQRT_2,
        rollout_depth: 1,
        seed: 1,
    };
    let mut agent: ISMCTSAgent<PickGame> = ISMCTSAgent::with_eval(cfg, 0, PICK_EVAL);
    let idx = agent.choose(&view, &legal, &state).await.unwrap();
    assert!(idx < legal.len(), "returned oob index {idx}");
}

#[tokio::test]
async fn eval_fallback_biases_selection() {
    // LongPickGame: never terminates inside rollout_depth=0 but exposes
    // a very strong eval gradient (scale 1e5) that sigmoid-normalization
    // can resolve. Verify ISMCTS follows the eval signal.
    let game = LongPickGame;
    let state = game.initial_state(0, &());
    let view = game.public_view(&state, 0);
    let legal = game.legal_actions(&state, 0);

    let cfg = ISMCTSConfig {
        iterations: 200,
        exploration_c: std::f64::consts::SQRT_2,
        // rollout_depth = 0 → eval decides every iteration.
        rollout_depth: 0,
        seed: 7,
    };
    let mut agent: ISMCTSAgent<LongPickGame> =
        ISMCTSAgent::with_eval(cfg, 0, LONG_PICK_EVAL);
    let idx = agent.choose(&view, &legal, &state).await.unwrap();
    // LongPickGame's legal is [Pick(0), Pick(1), Pick(2), Pick(3)]; the
    // eval rewards bigger picks by +500 each. Expect index 3.
    assert_eq!(idx, 3, "eval-driven ISMCTS should prefer action 3");
}

// LongPickGame: like PickGame but takes more turns to terminate so
// rollouts at depth 0 never see the terminal state — the eval must
// drive the decision. Eval scale is big enough to survive ISMCTS's
// sigmoid normalization.
#[derive(Clone, Default)]
struct LongPickState {
    scores: [i32; 2],
    next_player: PlayerId,
    turn: u32,
}

#[derive(Default, Debug, Clone, Copy)]
struct LongPickGame;

impl Game for LongPickGame {
    type State = LongPickState;
    type Action = Pick;
    type Event = Picked;
    type PublicView = LongPickState;
    type Config = ();

    fn initial_state(&self, _seed: u64, (): &()) -> LongPickState {
        LongPickState::default()
    }
    fn next_actor(&self, s: &LongPickState) -> Actor {
        Actor::Player(s.next_player)
    }
    fn legal_actions(&self, _s: &LongPickState, _p: PlayerId) -> Vec<Pick> {
        vec![Pick(0), Pick(1), Pick(2), Pick(3)]
    }
    fn apply_action(
        &self,
        _s: &LongPickState,
        p: PlayerId,
        a: &Pick,
    ) -> Result<Vec<Picked>, GameError> {
        Ok(vec![Picked {
            player: p,
            value: a.0,
        }])
    }
    fn resolve_chance(
        &self,
        _s: &LongPickState,
        _r: &mut dyn Rng,
    ) -> Result<Picked, GameError> {
        unreachable!()
    }
    fn apply_event(&self, s: &mut LongPickState, e: &Picked) {
        s.scores[e.player as usize] += e.value;
        s.next_player = 1 - e.player;
        s.turn += 1;
    }
    fn public_view(&self, s: &LongPickState, _p: PlayerId) -> LongPickState {
        s.clone()
    }
    fn determinize(
        &self,
        s: &LongPickState,
        _o: PlayerId,
        _r: &mut dyn Rng,
    ) -> LongPickState {
        s.clone()
    }
    fn game_over(&self, s: &LongPickState) -> Option<GameResult> {
        if s.turn >= 100 {
            Some(GameResult {
                winner: None,
                reason: EndReason::Draw,
                scores: vec![s.scores[0], s.scores[1]],
            })
        } else {
            None
        }
    }
}

// Amplified eval: 500 per point of own_score. Inside the ISMCTS
// normalization (sigmoid at scale 200), this is plenty to separate
// actions.
fn long_pick_eval(view: &<LongPickGame as Game>::PublicView, player: PlayerId) -> f64 {
    500.0 * f64::from(view.scores[player as usize])
}

const LONG_PICK_EVAL: EvalFn<LongPickGame> = long_pick_eval;

#[tokio::test]
async fn determinize_equivalent_to_identity_works() {
    // PickGame's determinize is state.clone() (no hidden info). ISMCTS
    // should work without panic under this "identity determinization".
    let game = PickGame;
    let state = game.initial_state(0, &());
    let view = game.public_view(&state, 0);
    let legal = game.legal_actions(&state, 0);
    let cfg = ISMCTSConfig {
        iterations: 50,
        exploration_c: std::f64::consts::SQRT_2,
        rollout_depth: 20,
        seed: 99,
    };
    let mut agent: ISMCTSAgent<PickGame> = ISMCTSAgent::with_eval(cfg, 0, PICK_EVAL);
    let idx = agent.choose(&view, &legal, &state).await.unwrap();
    assert!(idx < legal.len());
}

#[tokio::test]
async fn convergence_more_iterations_tends_to_prefer_dominant() {
    // Run ISMCTS at low and high iteration budgets; the high-budget
    // agent should pick the dominant action at least as often as the
    // low-budget one. This is a weak convergence sanity check — the
    // full R2.3 test lives in the game crates.
    let game = BiasGame;
    let state = game.initial_state(0, &());
    let view = game.public_view(&state, 0);
    let legal = game.legal_actions(&state, 0);
    let mut picks_at_100 = 0u32;
    let mut picks_at_1000 = 0u32;
    for seed in 0..20u64 {
        let cfg_low = ISMCTSConfig {
            iterations: 100,
            exploration_c: std::f64::consts::SQRT_2,
            rollout_depth: 10,
            seed,
        };
        let cfg_high = ISMCTSConfig {
            iterations: 1000,
            exploration_c: std::f64::consts::SQRT_2,
            rollout_depth: 10,
            seed,
        };
        let mut low: ISMCTSAgent<BiasGame> = ISMCTSAgent::new(cfg_low, 0);
        let mut high: ISMCTSAgent<BiasGame> = ISMCTSAgent::new(cfg_high, 0);
        if low.choose(&view, &legal, &state).await.unwrap() == 0 {
            picks_at_100 += 1;
        }
        if high.choose(&view, &legal, &state).await.unwrap() == 0 {
            picks_at_1000 += 1;
        }
    }
    assert!(
        picks_at_1000 >= picks_at_100,
        "high-budget ISMCTS ({picks_at_1000}/20) should beat low-budget ({picks_at_100}/20)"
    );
    // High-budget should be near-perfect on BiasGame.
    assert!(
        picks_at_1000 >= 18,
        "expected ISMCTS(1000) to dominate BiasGame, got {picks_at_1000}/20"
    );
}

#[tokio::test]
async fn bias_eval_used_when_rollout_depth_zero() {
    // BiasGame with a zero rollout depth. The eval function heavily
    // rewards the action-0 outcome. ISMCTS should still pick action 0.
    let game = BiasGame;
    let state = game.initial_state(0, &());
    let view = game.public_view(&state, 0);
    let legal = game.legal_actions(&state, 0);
    let cfg = ISMCTSConfig {
        iterations: 200,
        exploration_c: std::f64::consts::SQRT_2,
        rollout_depth: 0,
        seed: 5,
    };
    let mut agent: ISMCTSAgent<BiasGame> = ISMCTSAgent::with_eval(cfg, 0, BIAS_EVAL);
    let idx = agent.choose(&view, &legal, &state).await.unwrap();
    assert_eq!(idx, 0);
}

// ---------------------------------------------------------------------
// Minimal dyn-Agent safety: ISMCTSAgent is usable behind `Box<dyn>`.
// ---------------------------------------------------------------------

#[tokio::test]
async fn boxed_dyn_agent_works_on_pick_game() {
    let game = PickGame;
    let state = game.initial_state(0, &());
    let view = game.public_view(&state, 0);
    let legal = game.legal_actions(&state, 0);
    let cfg = ISMCTSConfig {
        iterations: 50,
        exploration_c: std::f64::consts::SQRT_2,
        rollout_depth: 10,
        seed: 0,
    };
    let mut agent: Box<dyn Agent<PickGame>> =
        Box::new(ISMCTSAgent::<PickGame>::with_eval(cfg, 0, PICK_EVAL));
    let idx = agent.choose(&view, &legal, &state).await.unwrap();
    assert!(idx < legal.len());
}

// Silence "unused" lint on the Rng import we pulled in for the mock games.
#[allow(dead_code)]
fn _rng_probe(r: &mut dyn Rng) -> Result<u64, RngError> {
    r.gen_range(0u64..Range { start: 0, end: 4 }.end)
}
