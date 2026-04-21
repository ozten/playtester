//! `HeuristicAgent` correctness tests.
//!
//! - temperature = 0 is identical to greedy argmax
//! - temperature → high spreads samples across actions
//! - empty legal slice errors
//! - integrates with GameLoop

use core::ops::Range;

use playtest_adapters::StubRng;
use playtest_agents::{EvalFn, GreedyAgent, HeuristicAgent};
use playtest_core::{
    Actor, Agent, AgentError, EndReason, Game, GameError, GameLoop, GameResult, PlayerId,
};
use playtest_ports::{GameEventSink, GameEventSinkError, Rng, RngError};
use serde::Serialize;

// ---------- Same trivial game as greedy_agent.rs ------------------------

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

fn pick_eval(view: &<PickGame as Game>::PublicView, player: PlayerId) -> f64 {
    f64::from(view.scores[player as usize])
}

const EVAL: EvalFn<PickGame> = pick_eval;

#[tokio::test]
async fn temperature_zero_matches_greedy() {
    let game = PickGame;
    let state = game.initial_state(0, &());
    let view = game.public_view(&state, 0);
    let legal = game.legal_actions(&state, 0);

    let mut greedy: GreedyAgent<PickGame> = GreedyAgent::new(0, EVAL);
    let rng = StubRng::seeded(0x00C0_FFEE);
    let mut heuristic: HeuristicAgent<PickGame, _> =
        HeuristicAgent::with_temperature(0, EVAL, rng, 0.0);

    let g = greedy.choose(&view, &legal, &state).await.unwrap();
    let h = heuristic.choose(&view, &legal, &state).await.unwrap();
    assert_eq!(g, h);
}

#[tokio::test]
async fn high_temperature_spreads_samples_across_actions() {
    let game = PickGame;
    let state = game.initial_state(0, &());
    let view = game.public_view(&state, 0);
    let legal = game.legal_actions(&state, 0);

    // Use the production-quality RNG tied to a seed so the spread
    // signal is reliable. Heuristic at T=1000 is close to uniform.
    let rng = playtest_adapters::ProductionRng::from_seed(2026);
    let mut agent: HeuristicAgent<PickGame, _> =
        HeuristicAgent::with_temperature(0, EVAL, rng, 1000.0);

    let mut hits = [0u32; 4];
    for _ in 0..400 {
        let idx = agent.choose(&view, &legal, &state).await.unwrap();
        hits[idx] += 1;
    }
    // Every action gets hit at least a handful of times.
    for (i, &c) in hits.iter().enumerate() {
        assert!(c >= 20, "bucket {i} had {c} hits (expected > 20): {hits:?}");
    }
}

#[tokio::test]
async fn empty_legal_slice_returns_agent_error() {
    let rng = StubRng::seeded(1);
    let mut agent: HeuristicAgent<PickGame, _> = HeuristicAgent::new(0, EVAL, rng);
    let state = PickState::default();
    let view = state.clone();
    let err = agent.choose(&view, &[], &state).await.unwrap_err();
    assert!(matches!(err, AgentError::Other(_)));
}

// ---------- Integration test --------------------------------------------

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
async fn game_loop_with_heuristic_agents_reaches_terminal_state() {
    let game = PickGame;
    let mut loop_ = GameLoop::new(&game, game.initial_state(0, &()));

    let mut agents: Vec<Box<dyn Agent<PickGame>>> = vec![
        Box::new(HeuristicAgent::<PickGame, _>::with_temperature(
            0,
            EVAL,
            playtest_adapters::ProductionRng::from_seed(11),
            0.5,
        )),
        Box::new(HeuristicAgent::<PickGame, _>::with_temperature(
            1,
            EVAL,
            playtest_adapters::ProductionRng::from_seed(22),
            0.5,
        )),
    ];
    let mut rng = UnusedRng;
    let mut sink = CapturingSink::default();

    let result = loop_
        .run(agents.as_mut_slice(), &mut rng, &mut sink)
        .await
        .unwrap();

    // At T=0.5 with one strictly dominant action (+10) and T small, we
    // nearly always pick +10 (30 total), but either side could get a
    // different variant occasionally. Just assert a completed terminal.
    assert!(result.scores[0] > 0);
    assert!(result.scores[1] > 0);
}
