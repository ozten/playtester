//! `ScriptedAgent` behavior: priority ordering, tie-break, error on
//! empty legal slice; plus a small GameLoop integration that drives
//! `RandomAgent` vs `RandomAgent` end-to-end.

use core::ops::Range;

use playtest_adapters::{PlaybackRng, RecordRng, StubRng};
use playtest_agents::{RandomAgent, ScriptedAgent};
use playtest_core::{
    Actor, Agent, AgentError, EndReason, Game, GameError, GameLoop, GameResult, PlayerId,
};
use playtest_ports::{GameEventSink, GameEventSinkError, Rng, RngError};
use serde::Serialize;
use tempfile::tempdir;

// ---------- Null-game scaffolding for trait-level tests -----------------

struct NullGame;

#[derive(Clone, PartialEq, Eq, Hash)]
struct Tag(u32);

#[derive(Clone, Serialize)]
struct NoopEvent;

impl Game for NullGame {
    type State = ();
    type Action = Tag;
    type Event = NoopEvent;
    type PublicView = ();
    type Config = ();

    fn initial_state(&self, _seed: u64, (): &()) {}
    fn next_actor(&self, (): &()) -> Actor {
        Actor::Player(0)
    }
    fn legal_actions(&self, (): &(), _p: PlayerId) -> Vec<Tag> {
        Vec::new()
    }
    fn apply_action(&self, (): &(), _p: PlayerId, _a: &Tag) -> Result<Vec<NoopEvent>, GameError> {
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

#[tokio::test]
async fn priority_prefers_first_action_returns_index_zero() {
    let mut agent: ScriptedAgent<NullGame, _> = ScriptedAgent::new(|(): &(), a: &Tag| {
        // higher priority for smaller tags — action #0 always wins
        -i32::try_from(a.0).unwrap_or(0)
    });
    let legal = vec![Tag(0), Tag(1), Tag(2)];
    assert_eq!(agent.choose(&(), &legal, &()).await.unwrap(), 0);
}

#[tokio::test]
async fn priority_prefers_last_action_returns_index_n_minus_one() {
    let mut agent: ScriptedAgent<NullGame, _> = ScriptedAgent::new(|(): &(), a: &Tag| {
        // higher priority for larger tags
        i32::try_from(a.0).unwrap_or(0)
    });
    let legal = vec![Tag(0), Tag(1), Tag(2), Tag(10)];
    assert_eq!(agent.choose(&(), &legal, &()).await.unwrap(), 3);
}

#[tokio::test]
async fn tie_break_picks_lowest_index() {
    let mut agent: ScriptedAgent<NullGame, _> = ScriptedAgent::new(|(): &(), _a: &Tag| 0);
    let legal = vec![Tag(5), Tag(5), Tag(5), Tag(5)];
    assert_eq!(agent.choose(&(), &legal, &()).await.unwrap(), 0);
}

#[tokio::test]
async fn empty_legal_slice_returns_agent_error() {
    let mut agent: ScriptedAgent<NullGame, _> = ScriptedAgent::new(|(): &(), _a: &Tag| 0);
    let err = agent.choose(&(), &[], &()).await.unwrap_err();
    assert!(matches!(err, AgentError::Other(_)));
}

// ---------- GameLoop integration with RandomAgent vs RandomAgent --------

/// Minimal two-player game: each turn, the current player picks an amount
/// 1..=3 to add to their score. First to 10 wins. No chance events.
#[derive(Default, Clone)]
struct TallyState {
    scores: [u32; 2],
    next_player: PlayerId,
}

#[derive(Clone, PartialEq, Eq, Hash)]
struct Add(u8);

#[derive(Clone, Serialize)]
struct Scored {
    player: PlayerId,
    amount: u8,
}

struct TallyGame;

impl Game for TallyGame {
    type State = TallyState;
    type Action = Add;
    type Event = Scored;
    type PublicView = TallyState;
    type Config = ();

    fn initial_state(&self, _seed: u64, (): &()) -> TallyState {
        TallyState::default()
    }
    fn next_actor(&self, s: &TallyState) -> Actor {
        Actor::Player(s.next_player)
    }
    fn legal_actions(&self, _s: &TallyState, _p: PlayerId) -> Vec<Add> {
        vec![Add(1), Add(2), Add(3)]
    }
    fn apply_action(
        &self,
        _s: &TallyState,
        player: PlayerId,
        a: &Add,
    ) -> Result<Vec<Scored>, GameError> {
        Ok(vec![Scored {
            player,
            amount: a.0,
        }])
    }
    fn resolve_chance(&self, _s: &TallyState, _rng: &mut dyn Rng) -> Result<Scored, GameError> {
        unreachable!()
    }
    fn apply_event(&self, s: &mut TallyState, e: &Scored) {
        s.scores[e.player as usize] += u32::from(e.amount);
        s.next_player = 1 - e.player;
    }
    fn public_view(&self, s: &TallyState, _p: PlayerId) -> TallyState {
        s.clone()
    }
    fn determinize(
        &self,
        s: &TallyState,
        _observer: PlayerId,
        _rng: &mut dyn Rng,
    ) -> TallyState {
        s.clone()
    }
    fn game_over(&self, s: &TallyState) -> Option<GameResult> {
        if s.scores[0] >= 10 || s.scores[1] >= 10 {
            let winner = u8::from(s.scores[1] >= 10);
            Some(GameResult {
                winner: Some(winner),
                reason: EndReason::Victory,
                scores: s
                    .scores
                    .iter()
                    .map(|&v| i32::try_from(v).unwrap_or(i32::MAX))
                    .collect(),
            })
        } else {
            None
        }
    }
}

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

// The engine needs *some* Rng even if the game never pulls from it
// (the trait takes `&mut dyn Rng`). Use a stub — never called.
struct UnusedRng;
impl Rng for UnusedRng {
    fn next_u64(&mut self) -> u64 {
        unreachable!("game has no chance events")
    }
    fn gen_range(&mut self, _range: Range<u64>) -> Result<u64, RngError> {
        unreachable!("game has no chance events")
    }
}

#[tokio::test]
async fn game_loop_with_two_random_agents_completes_with_a_valid_result() {
    let game = TallyGame;
    let mut loop_ = GameLoop::new(&game, game.initial_state(0, &()));

    let mut agents: Vec<Box<dyn Agent<TallyGame>>> = vec![
        Box::new(RandomAgent::<TallyGame, _>::new(StubRng::seeded(11))),
        Box::new(RandomAgent::<TallyGame, _>::new(StubRng::seeded(22))),
    ];
    let mut rng = UnusedRng;
    let mut sink = CapturingSink::default();

    let result = loop_
        .run(agents.as_mut_slice(), &mut rng, &mut sink)
        .await
        .unwrap();

    assert_eq!(result.reason, EndReason::Victory);
    assert!(result.winner.is_some());
    let w = result.winner.unwrap();
    assert!(
        result.scores[w as usize] >= 10,
        "winner did not actually hit 10: {:?}",
        result.scores
    );
    // Every emitted line must be valid JSON.
    for line in &sink.lines {
        let _: serde_json::Value = serde_json::from_str(line).expect("valid JSON line");
    }
}

// ------------- Replay determinism via Playback<Rng> ---------------------

/// Play 40 turns of RandomAgent choices against a recorded RNG tape,
/// then replay the same sequence against `Playback<Rng>` and check the
/// choices match bit-for-bit. This is the integration that matters: it
/// proves agent-level determinism composes with port-level record/playback.
#[tokio::test]
async fn random_agent_with_playback_rng_reproduces_choices_exactly() {
    let dir = tempdir().unwrap();
    let tape = dir.path().join("agent_rng.jsonl");
    let legal_size = 5_u32;
    let legal: Vec<Tag> = (0..legal_size).map(Tag).collect();

    // Record pass: drive RandomAgent off a recorded RNG.
    let recorded: Vec<usize> = {
        let record = RecordRng::create(StubRng::seeded(0x00C0_FFEE), &tape).unwrap();
        let mut agent: RandomAgent<NullGame, _> = RandomAgent::new(record);
        let mut out = Vec::with_capacity(40);
        for _ in 0..40 {
            out.push(agent.choose(&(), &legal, &()).await.unwrap());
        }
        let mut rec = agent.into_rng();
        rec.flush().unwrap();
        out
    };

    // Playback pass: same agent shape, tape-backed RNG.
    let replayed: Vec<usize> = {
        let playback = PlaybackRng::open(&tape).unwrap();
        let mut agent: RandomAgent<NullGame, _> = RandomAgent::new(playback);
        let mut out = Vec::with_capacity(40);
        for _ in 0..40 {
            out.push(agent.choose(&(), &legal, &()).await.unwrap());
        }
        out
    };

    assert_eq!(recorded, replayed);
}
