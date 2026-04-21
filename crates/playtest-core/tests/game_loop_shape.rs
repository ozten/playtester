//! End-to-end shape test for `GameLoop`.
//!
//! Defines a minimal two-player "tally" game (chance picks a target,
//! players alternate adding 1–3 points, first to target wins) and
//! drives it through the loop. Exercises every routing arm:
//! chance → player → illegal action → agent error → immediate game-over.

use async_trait::async_trait;
use core::ops::Range;
use playtest_core::{
    Actor, Agent, AgentError, EndReason, Game, GameError, GameLoop, GameResult, PlayerId,
};
use playtest_ports::{Clock, GameEventSink, GameEventSinkError, Rng, RngError, UnixMillis};
use serde::Serialize;

// ---------- The test game -----------------------------------------------

#[derive(Debug, Default, Clone)]
struct TallyState {
    target: Option<u64>,
    scores: [u64; 2],
    next_player: PlayerId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Tally(u8);

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind")]
enum TallyEvent {
    TargetSet { target: u64 },
    Score { player: PlayerId, amount: u8 },
}

struct TallyGame;

impl Game for TallyGame {
    type State = TallyState;
    type Action = Tally;
    type Event = TallyEvent;
    type PublicView = TallyState;
    type Config = ();

    fn initial_state(&self, _seed: u64, _cfg: &()) -> TallyState {
        TallyState::default()
    }

    fn next_actor(&self, state: &TallyState) -> Actor {
        if state.target.is_none() {
            Actor::Chance
        } else {
            Actor::Player(state.next_player)
        }
    }

    fn legal_actions(&self, state: &TallyState, _player: PlayerId) -> Vec<Tally> {
        if state.target.is_none() {
            Vec::new()
        } else {
            vec![Tally(1), Tally(2), Tally(3)]
        }
    }

    fn apply_action(
        &self,
        _state: &TallyState,
        player: PlayerId,
        action: &Tally,
    ) -> Result<Vec<TallyEvent>, GameError> {
        if !(1..=3).contains(&action.0) {
            return Err(GameError::IllegalAction {
                player,
                message: format!("amount must be 1..=3, got {}", action.0),
            });
        }
        Ok(vec![TallyEvent::Score {
            player,
            amount: action.0,
        }])
    }

    fn resolve_chance(
        &self,
        _state: &TallyState,
        rng: &mut dyn Rng,
    ) -> Result<TallyEvent, GameError> {
        let target = rng
            .gen_range(5..10)
            .map_err(|source| GameError::RngFailed { source })?;
        Ok(TallyEvent::TargetSet { target })
    }

    fn apply_event(&self, state: &mut TallyState, event: &TallyEvent) {
        match *event {
            TallyEvent::TargetSet { target } => state.target = Some(target),
            TallyEvent::Score { player, amount } => {
                state.scores[player as usize] += u64::from(amount);
                state.next_player = 1 - player;
            }
        }
    }

    fn public_view(&self, state: &TallyState, _player: PlayerId) -> TallyState {
        state.clone()
    }

    fn determinize(
        &self,
        state: &TallyState,
        _observer: PlayerId,
        _rng: &mut dyn Rng,
    ) -> TallyState {
        // TallyGame has no hidden information — identity is correct.
        state.clone()
    }

    fn game_over(&self, state: &TallyState) -> Option<GameResult> {
        let target = state.target?;
        if state.scores[0] >= target || state.scores[1] >= target {
            let winner = u8::from(state.scores[1] >= target);
            let scores = state
                .scores
                .iter()
                .map(|&s| i32::try_from(s).unwrap_or(i32::MAX))
                .collect();
            Some(GameResult {
                winner: Some(winner),
                reason: EndReason::Victory,
                scores,
            })
        } else {
            None
        }
    }
}

// ---------- Helper agents -----------------------------------------------

struct FixedChoice(usize);

#[async_trait]
impl<G: Game + ?Sized> Agent<G> for FixedChoice
where
    G::PublicView: Send + Sync,
    G::Action: Send + Sync,
{
    async fn choose(
        &mut self,
        _view: &G::PublicView,
        _legal: &[G::Action],
    ) -> Result<usize, AgentError> {
        Ok(self.0)
    }
}

struct OutOfBounds;

#[async_trait]
impl<G: Game + ?Sized> Agent<G> for OutOfBounds
where
    G::PublicView: Send + Sync,
    G::Action: Send + Sync,
{
    async fn choose(
        &mut self,
        _view: &G::PublicView,
        legal: &[G::Action],
    ) -> Result<usize, AgentError> {
        Ok(legal.len() + 5)
    }
}

struct AlwaysFails;

#[async_trait]
impl<G: Game + ?Sized> Agent<G> for AlwaysFails
where
    G::PublicView: Send + Sync,
    G::Action: Send + Sync,
{
    async fn choose(
        &mut self,
        _view: &G::PublicView,
        _legal: &[G::Action],
    ) -> Result<usize, AgentError> {
        Err(AgentError::Other("simulated agent failure".into()))
    }
}

// ---------- Test ports --------------------------------------------------

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

struct ScriptedRng {
    values: std::collections::VecDeque<u64>,
}

impl ScriptedRng {
    fn new(values: impl IntoIterator<Item = u64>) -> Self {
        Self {
            values: values.into_iter().collect(),
        }
    }
}

impl Rng for ScriptedRng {
    fn next_u64(&mut self) -> u64 {
        self.values.pop_front().unwrap_or(0)
    }
    fn gen_range(&mut self, range: Range<u64>) -> Result<u64, RngError> {
        if range.start >= range.end {
            return Err(RngError::InvalidRange {
                start: range.start,
                end: range.end,
            });
        }
        Ok(self.values.pop_front().unwrap_or(range.start))
    }
}

#[allow(dead_code)]
struct FixedClock(UnixMillis);
impl Clock for FixedClock {
    fn now(&mut self) -> UnixMillis {
        self.0
    }
}

// ---------- Scenarios ---------------------------------------------------

#[tokio::test]
async fn happy_path_chance_then_alternating_players_to_victory() {
    let game = TallyGame;
    let mut loop_ = GameLoop::new(&game, game.initial_state(0, &()));

    let mut agents: Vec<Box<dyn Agent<TallyGame>>> = vec![
        Box::new(FixedChoice(2)), // player 0 always picks Tally(3)
        Box::new(FixedChoice(0)), // player 1 always picks Tally(1)
    ];
    let mut rng = ScriptedRng::new([5]); // target = 5
    let mut sink = CapturingSink::default();

    let result = loop_
        .run(agents.as_mut_slice(), &mut rng, &mut sink)
        .await
        .unwrap();

    // Player 0 scores 3 + 3 (= 6) before player 1 reaches 5.
    // After: p0 plays tally 3 (=3), p1 plays tally 1 (=1), p0 plays 3 (=6 ≥ 5).
    assert_eq!(result.winner, Some(0));
    assert_eq!(result.reason, EndReason::Victory);
    assert_eq!(result.scores, vec![6, 1]);

    // 1 chance event + 3 score events = 4 lines emitted.
    assert_eq!(sink.lines.len(), 4, "emitted lines: {:?}", sink.lines);
    assert!(sink.lines[0].contains("TargetSet"));
    assert!(sink.lines[1].contains("Score"));
}

#[tokio::test]
async fn agent_returning_out_of_range_index_is_rejected_by_loop() {
    let game = TallyGame;
    let mut loop_ = GameLoop::new(&game, game.initial_state(0, &()));

    let mut agents: Vec<Box<dyn Agent<TallyGame>>> =
        vec![Box::new(OutOfBounds), Box::new(FixedChoice(0))];
    let mut rng = ScriptedRng::new([5]);
    let mut sink = CapturingSink::default();

    let err = loop_
        .run(agents.as_mut_slice(), &mut rng, &mut sink)
        .await
        .unwrap_err();

    assert!(
        matches!(
            err,
            GameError::AgentChoseOutOfBounds {
                player: 0,
                legal_count: 3,
                ..
            }
        ),
        "got {err:?}"
    );
}

#[tokio::test]
async fn agent_error_is_surfaced_with_player_context() {
    let game = TallyGame;
    let mut loop_ = GameLoop::new(&game, game.initial_state(0, &()));

    let mut agents: Vec<Box<dyn Agent<TallyGame>>> =
        vec![Box::new(AlwaysFails), Box::new(FixedChoice(0))];
    let mut rng = ScriptedRng::new([5]);
    let mut sink = CapturingSink::default();

    let err = loop_
        .run(agents.as_mut_slice(), &mut rng, &mut sink)
        .await
        .unwrap_err();

    assert!(matches!(err, GameError::AgentFailed { player: 0, .. }));
}

#[tokio::test]
async fn game_over_on_initial_state_short_circuits_immediately() {
    // A degenerate `Game` that declares game-over before any action.
    struct AlreadyDone;

    impl Game for AlreadyDone {
        type State = ();
        type Action = ();
        type Event = TallyEvent;
        type PublicView = ();
        type Config = ();

        fn initial_state(&self, _seed: u64, (): &()) {}
        fn next_actor(&self, (): &()) -> Actor {
            Actor::Player(0)
        }
        fn legal_actions(&self, (): &(), _player: PlayerId) -> Vec<()> {
            vec![]
        }
        fn apply_action(
            &self,
            (): &(),
            _player: PlayerId,
            (): &(),
        ) -> Result<Vec<TallyEvent>, GameError> {
            unreachable!()
        }
        fn resolve_chance(&self, (): &(), _rng: &mut dyn Rng) -> Result<TallyEvent, GameError> {
            unreachable!()
        }
        fn apply_event(&self, (): &mut (), _event: &TallyEvent) {}
        fn public_view(&self, (): &(), _player: PlayerId) {}
        fn determinize(&self, (): &(), _observer: PlayerId, _rng: &mut dyn Rng) {}
        fn game_over(&self, (): &()) -> Option<GameResult> {
            Some(GameResult {
                winner: None,
                reason: EndReason::Draw,
                scores: vec![0, 0],
            })
        }
    }

    let game = AlreadyDone;
    let mut loop_ = GameLoop::new(&game, ());
    let mut agents: Vec<Box<dyn Agent<AlreadyDone>>> = vec![];
    let mut rng = ScriptedRng::new([]);
    let mut sink = CapturingSink::default();

    let result = loop_
        .run(agents.as_mut_slice(), &mut rng, &mut sink)
        .await
        .unwrap();

    assert_eq!(result.reason, EndReason::Draw);
    assert!(sink.lines.is_empty(), "no events should be emitted");
}

#[test]
fn determinize_on_tally_game_returns_equal_state() {
    // TallyGame has no hidden information, so determinize is the
    // identity function. This is the trait-level smoke test for the
    // invariant `public_view(determinize(s, p, rng), p) == public_view(s, p)`.
    let game = TallyGame;
    let state = TallyState {
        target: Some(7),
        scores: [3, 2],
        next_player: 0,
    };
    let mut rng = ScriptedRng::new([42, 17, 99]);
    let d0 = game.determinize(&state, 0, &mut rng);
    let d1 = game.determinize(&state, 1, &mut rng);
    assert_eq!(d0.target, state.target);
    assert_eq!(d0.scores, state.scores);
    assert_eq!(d0.next_player, state.next_player);
    assert_eq!(d1.target, state.target);
    assert_eq!(d1.scores, state.scores);
    assert_eq!(d1.next_player, state.next_player);
}

#[tokio::test]
async fn loop_emits_one_line_per_event_to_the_sink() {
    // Regression test for the "one event = one JSONL line" contract.
    let game = TallyGame;
    let mut loop_ = GameLoop::new(&game, game.initial_state(0, &()));

    let mut agents: Vec<Box<dyn Agent<TallyGame>>> = vec![
        Box::new(FixedChoice(2)), // 3 each turn
        Box::new(FixedChoice(2)),
    ];
    let mut rng = ScriptedRng::new([9]); // target = 9
    let mut sink = CapturingSink::default();

    loop_
        .run(agents.as_mut_slice(), &mut rng, &mut sink)
        .await
        .unwrap();

    // Each emitted line parses as valid JSON on its own — this is the
    // JSONL contract the `playtest-log` crate will rely on.
    for line in &sink.lines {
        let _: serde_json::Value = serde_json::from_str(line)
            .unwrap_or_else(|e| panic!("line {line:?} did not parse as JSON: {e}"));
    }
}
