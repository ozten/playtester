//! `GreedyAgent` correctness tests.
//!
//! - Picks a dominating action deterministically
//! - Ties break on the lowest legal-action index
//! - Drives a GameLoop end-to-end on a trivial test game

use core::ops::Range;

use async_trait::async_trait;
use playtest_agents::{EvalFn, GreedyAgent};
use playtest_core::{
    Actor, Agent, AgentError, EndReason, Game, GameError, GameLoop, GameResult, PlayerId,
};
use playtest_ports::{GameEventSink, GameEventSinkError, Rng, RngError};
use serde::Serialize;

// ---------- A trivial score-game used by the tests ----------------------

#[derive(Clone, Default)]
struct PickState {
    scores: [i32; 2],
    next_player: PlayerId,
    turn: u32,
}

#[derive(Clone, PartialEq, Eq, Debug)]
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
        // Actions: +1, +10, +5, +10 — two ties at 10.
        vec![Pick(1), Pick(10), Pick(5), Pick(10)]
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
        if s.turn >= 6 {
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

/// Eval = just the player's current score. Higher = better.
fn pick_eval(view: &<PickGame as Game>::PublicView, player: PlayerId) -> f64 {
    f64::from(view.scores[player as usize])
}

const EVAL: EvalFn<PickGame> = pick_eval;

#[tokio::test]
async fn picks_dominating_action() {
    let game = PickGame;
    let mut agent: GreedyAgent<PickGame> = GreedyAgent::new(0, EVAL);
    let state = game.initial_state(0, &());
    let view = game.public_view(&state, 0);
    let legal = game.legal_actions(&state, 0);
    // Legal: +1, +10, +5, +10. Score after each: 1, 10, 5, 10.
    // argmax with lowest-index tiebreak -> index 1 (first 10).
    let idx = agent.choose(&view, &legal, &state).await.unwrap();
    assert_eq!(idx, 1);
}

#[tokio::test]
async fn ties_break_on_lowest_index() {
    // Game returns four actions, two tying at top score.
    let game = PickGame;
    let mut agent: GreedyAgent<PickGame> = GreedyAgent::new(0, EVAL);
    let state = game.initial_state(0, &());
    let view = game.public_view(&state, 0);
    let legal = game.legal_actions(&state, 0);
    // Two actions yield +10 (indices 1 and 3). Expect 1.
    let idx = agent.choose(&view, &legal, &state).await.unwrap();
    assert_eq!(idx, 1, "should pick lowest-index maximum");
}

#[tokio::test]
async fn empty_legal_slice_returns_agent_error() {
    let mut agent: GreedyAgent<PickGame> = GreedyAgent::new(0, EVAL);
    let state = PickState::default();
    let view = state.clone();
    let err = agent.choose(&view, &[], &state).await.unwrap_err();
    assert!(matches!(err, AgentError::Other(_)));
}

// ---------- Integration test: GameLoop with GreedyAgent both seats ------

#[derive(Default)]
struct CapturingSink {
    lines: Vec<String>,
}
impl GameEventSink for CapturingSink {
    fn emit(&mut self, line: &str) -> Result<(), GameEventSinkError> {
        self.lines.push(line.to_owned());
        Ok(())
    }
    fn flush(&mut self) -> Result<(), GameEventSinkError> {
        Ok(())
    }
}

struct UnusedRng;
impl Rng for UnusedRng {
    fn next_u64(&mut self) -> u64 {
        unreachable!("no chance events")
    }
    fn gen_range(&mut self, _: Range<u64>) -> Result<u64, RngError> {
        unreachable!()
    }
}

#[tokio::test]
async fn game_loop_with_two_greedy_agents_reaches_terminal_state() {
    let game = PickGame;
    let mut loop_ = GameLoop::new(&game, game.initial_state(0, &()));

    let mut agents: Vec<Box<dyn Agent<PickGame>>> = vec![
        Box::new(GreedyAgent::<PickGame>::new(0, EVAL)),
        Box::new(GreedyAgent::<PickGame>::new(1, EVAL)),
    ];
    let mut rng = UnusedRng;
    let mut sink = CapturingSink::default();

    let result = loop_
        .run(agents.as_mut_slice(), &mut rng, &mut sink)
        .await
        .unwrap();

    // With both agents always picking +10, after 6 turns each has 30.
    assert_eq!(result.scores, vec![30, 30]);
    assert_eq!(result.reason, EndReason::Draw);
}

// ---------- NaN detection -----------------------------------------------

fn nan_eval(_view: &<PickGame as Game>::PublicView, _player: PlayerId) -> f64 {
    f64::NAN
}

#[tokio::test]
async fn nan_eval_returns_error() {
    let game = PickGame;
    let mut agent: GreedyAgent<PickGame> = GreedyAgent::new(0, nan_eval);
    let state = game.initial_state(0, &());
    let view = game.public_view(&state, 0);
    let legal = game.legal_actions(&state, 0);
    let err = agent.choose(&view, &legal, &state).await.unwrap_err();
    assert!(
        matches!(err, AgentError::Other(ref m) if m.contains("NaN")),
        "unexpected error: {err:?}"
    );
}

// ---------- Ensure Agent trait surgery didn't break Box<dyn Agent> ------

#[async_trait]
trait _DynSafetyProbe: Send {}
#[async_trait]
impl _DynSafetyProbe for GreedyAgent<PickGame> {}
