//! `HttpRemoteAgent` behavior: transport round-trip, ordering, bounds,
//! cancellation propagation, and loop stability under many calls.
//!
//! These tests drive the agent against a hand-rolled in-memory stub
//! transport — no server, no channels, no HTTP. The stub lets the test
//! pre-queue action-index responses and records every prompt the agent
//! issues, so assertions can focus on the agent's contract with the
//! transport port without dragging in production infrastructure.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use playtest_agents::{HttpRemoteAgent, RemoteAgentTransport, RemoteTransportError};
use playtest_core::{Actor, Agent, AgentError, Game, GameError, GameResult, PlayerId};
use playtest_ports::Rng;
use serde::Serialize;
use serde_json::Value as JsonValue;

/// Degenerate game parameterized only so the `Agent<G>` trait has a `G`.
/// Tests drive `choose` directly.
struct NullGame;

#[derive(Clone, PartialEq, Eq, Hash, Serialize)]
struct NoopAction {
    label: u32,
}

#[derive(Clone, Serialize)]
struct NoopEvent;

impl Game for NullGame {
    type State = ();
    type Action = NoopAction;
    type Event = NoopEvent;
    type PublicView = ();
    type Config = ();

    fn initial_state(&self, _seed: u64, (): &()) {}
    fn next_actor(&self, (): &()) -> Actor {
        Actor::Player(0)
    }
    fn legal_actions(&self, (): &(), _p: PlayerId) -> Vec<NoopAction> {
        Vec::new()
    }
    fn apply_action(
        &self,
        (): &(),
        _p: PlayerId,
        _a: &NoopAction,
    ) -> Result<Vec<NoopEvent>, GameError> {
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

fn legal(n: usize) -> Vec<NoopAction> {
    (0..n)
        .map(|i| NoopAction {
            label: u32::try_from(i).unwrap(),
        })
        .collect()
}

/// Minimal in-memory `RemoteAgentTransport` for unit tests.
///
/// Pre-queue responses via [`StubTransport::push_action`] / [`push_err`];
/// each `await_action` call pops the front. Prompts are recorded so tests
/// can assert on `prompt_id` monotonicity and the legal-actions payload.
#[derive(Default)]
struct StubTransport {
    next_prompt_id: AtomicU64,
    issued: Mutex<Vec<IssuedPrompt>>,
    responses: Mutex<VecDeque<Result<usize, RemoteTransportError>>>,
}

#[derive(Clone)]
struct IssuedPrompt {
    seat: u8,
    prompt_id: u64,
    legal_json: Vec<JsonValue>,
}

impl StubTransport {
    fn push_action(&self, idx: usize) {
        self.responses.lock().unwrap().push_back(Ok(idx));
    }

    fn push_err(&self, err: RemoteTransportError) {
        self.responses.lock().unwrap().push_back(Err(err));
    }

    fn issued(&self) -> Vec<IssuedPrompt> {
        self.issued.lock().unwrap().clone()
    }
}

#[async_trait]
impl RemoteAgentTransport for StubTransport {
    async fn issue_prompt(
        &self,
        seat: u8,
        legal_json: Vec<JsonValue>,
    ) -> Result<u64, RemoteTransportError> {
        let prompt_id = self.next_prompt_id.fetch_add(1, Ordering::SeqCst);
        self.issued.lock().unwrap().push(IssuedPrompt {
            seat,
            prompt_id,
            legal_json,
        });
        Ok(prompt_id)
    }

    async fn await_action(
        &self,
        _seat: u8,
        _prompt_id: u64,
    ) -> Result<usize, RemoteTransportError> {
        self.responses
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or(Err(RemoteTransportError::Cancelled))
    }
}

#[tokio::test]
async fn happy_path_single_choice_returns_queued_index() {
    let transport = Arc::new(StubTransport::default());
    transport.push_action(0);

    let mut agent: HttpRemoteAgent<NullGame> = HttpRemoteAgent::new(0, transport.clone());
    let idx = agent.choose(&(), &legal(3), &()).await.unwrap();
    assert_eq!(idx, 0);

    let prompts = transport.issued();
    assert_eq!(prompts.len(), 1);
    assert_eq!(prompts[0].seat, 0);
    assert_eq!(prompts[0].prompt_id, 0);
    assert_eq!(prompts[0].legal_json.len(), 3);
}

#[tokio::test]
async fn sequence_of_three_returns_in_order() {
    let transport = Arc::new(StubTransport::default());
    transport.push_action(0);
    transport.push_action(1);
    transport.push_action(2);

    let mut agent: HttpRemoteAgent<NullGame> = HttpRemoteAgent::new(1, transport.clone());
    let actions = legal(3);

    let mut got = Vec::new();
    for _ in 0..3 {
        got.push(agent.choose(&(), &actions, &()).await.unwrap());
    }
    assert_eq!(got, vec![0, 1, 2]);

    let prompts = transport.issued();
    assert_eq!(prompts.len(), 3);
    assert!(prompts.iter().all(|p| p.seat == 1));
    // prompt_id monotonicity
    assert_eq!(
        prompts.iter().map(|p| p.prompt_id).collect::<Vec<_>>(),
        vec![0, 1, 2]
    );
}

#[tokio::test]
async fn single_element_legal_slice_still_prompts_and_returns_zero() {
    // Even when there is only one legal action, the agent still issues
    // the prompt. The contract with the client is "we always ask"; it is
    // up to the UI to auto-submit when the list has one element.
    let transport = Arc::new(StubTransport::default());
    transport.push_action(0);

    let mut agent: HttpRemoteAgent<NullGame> = HttpRemoteAgent::new(0, transport.clone());
    let idx = agent.choose(&(), &legal(1), &()).await.unwrap();
    assert_eq!(idx, 0);
    assert_eq!(transport.issued().len(), 1);
}

#[tokio::test]
async fn empty_legal_slice_returns_agent_error_without_prompting() {
    let transport = Arc::new(StubTransport::default());

    let mut agent: HttpRemoteAgent<NullGame> = HttpRemoteAgent::new(0, transport.clone());
    let err = agent.choose(&(), &[], &()).await.unwrap_err();
    assert!(matches!(err, AgentError::Other(_)));
    assert!(
        transport.issued().is_empty(),
        "must not emit a prompt with zero legal actions"
    );
}

#[tokio::test]
async fn cancelled_transport_surfaces_as_agent_error() {
    let transport = Arc::new(StubTransport::default());
    transport.push_err(RemoteTransportError::Cancelled);

    let mut agent: HttpRemoteAgent<NullGame> = HttpRemoteAgent::new(0, transport.clone());
    let err = agent.choose(&(), &legal(2), &()).await.unwrap_err();
    let AgentError::Other(msg) = err else {
        panic!("expected AgentError::Other, got {err:?}");
    };
    assert!(msg.contains("cancelled"), "message was: {msg}");
}

#[tokio::test]
async fn out_of_bounds_index_from_transport_is_rejected() {
    // The server-side validates action_index < legal.len(), but the agent
    // has to double-check in case a non-production transport misbehaves.
    let transport = Arc::new(StubTransport::default());
    transport.push_action(5);

    let mut agent: HttpRemoteAgent<NullGame> = HttpRemoteAgent::new(0, transport.clone());
    let err = agent.choose(&(), &legal(3), &()).await.unwrap_err();
    let AgentError::Other(msg) = err else {
        panic!("expected AgentError::Other, got {err:?}");
    };
    assert!(msg.contains('5'), "message was: {msg}");
    assert!(msg.contains('3'), "message was: {msg}");
}

#[tokio::test]
async fn round_trips_thousand_times_without_leaks() {
    // Integration-style: prove the agent holds no per-call state that
    // accumulates unbounded.
    let transport = Arc::new(StubTransport::default());
    for _ in 0..1000 {
        transport.push_action(0);
    }

    let mut agent: HttpRemoteAgent<NullGame> = HttpRemoteAgent::new(0, transport.clone());
    let actions = legal(1);
    for _ in 0..1000 {
        agent.choose(&(), &actions, &()).await.unwrap();
    }
    assert_eq!(transport.issued().len(), 1000);
}

#[tokio::test]
async fn legal_payload_is_serialized_action_values() {
    // The agent must serialize each Action to JSON and pass the vec to
    // issue_prompt. Confirm the NoopAction's `label` field survives the
    // round-trip — that is the Action shape the frontend will parse.
    let transport = Arc::new(StubTransport::default());
    transport.push_action(2);

    let mut agent: HttpRemoteAgent<NullGame> = HttpRemoteAgent::new(0, transport.clone());
    agent.choose(&(), &legal(3), &()).await.unwrap();

    let prompts = transport.issued();
    let labels: Vec<i64> = prompts[0]
        .legal_json
        .iter()
        .map(|v| v.get("label").and_then(JsonValue::as_i64).unwrap())
        .collect();
    assert_eq!(labels, vec![0, 1, 2]);
}
