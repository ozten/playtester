//! `LlmAgent` happy-path and error-path tests using a hand-rolled stub
//! `LlmClient`. The stub captures each `LlmRequest` so we can assert on
//! system-block ordering and cache flags, and lets the test pre-queue
//! reply texts per call.
//!
//! A final end-to-end test drives `LlmAgent<CribbageGame>` through a full
//! game using a "always return action_index 0" stub.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use playtest_adapters::{ProductionRng, StubGameEventSink};
use playtest_agents::{LlmAgent, LlmAgentConfig};
use playtest_core::{Actor, Agent, AgentError, EndReason, Game, GameLoop};
use playtest_cribbage::{CribbageConfig, CribbageGame};
use playtest_ports::{LlmClient, LlmError, LlmRequest, LlmResponse};

/// Advance the initial `CribbageGame` state through the Deal phase so
/// the state has a non-empty legal-action slice for seat 0 (non-dealer,
/// who discards first).
fn discard_phase_state(seed: u64) -> <CribbageGame as Game>::State {
    let game = CribbageGame::new();
    let mut state = game.initial_state(seed, &CribbageConfig);
    let mut rng = ProductionRng::from_seed(seed);
    // Resolve chance events until we reach Discard — the deal produces
    // 12 `DealCard` events.
    while matches!(game.next_actor(&state), Actor::Chance) {
        let event = game.resolve_chance(&state, &mut rng).unwrap();
        game.apply_event(&mut state, &event);
    }
    state
}

/// In-memory `LlmClient` used by these tests.
///
/// Pre-queue reply texts via `replies`; each `complete` call pops one
/// off the front. Every incoming request is recorded into `requests` so
/// assertions can inspect system-block ordering, cache flags, etc.
/// If `panic_on_call` is true, any invocation panics.
#[derive(Default)]
struct StubClient {
    replies: Mutex<Vec<Result<LlmResponse, LlmError>>>,
    requests: Mutex<Vec<LlmRequest>>,
    panic_on_call: bool,
}

impl StubClient {
    fn with_reply_texts(texts: Vec<String>) -> Arc<Self> {
        let responses: Vec<Result<LlmResponse, LlmError>> = texts
            .into_iter()
            .map(|t| {
                Ok(LlmResponse {
                    text: t,
                    input_tokens: 100,
                    output_tokens: 20,
                    cache_read_input_tokens: 0,
                    cache_creation_input_tokens: 0,
                })
            })
            .collect();
        Arc::new(Self {
            replies: Mutex::new(responses),
            ..Default::default()
        })
    }

    fn with_results(results: Vec<Result<LlmResponse, LlmError>>) -> Arc<Self> {
        Arc::new(Self {
            replies: Mutex::new(results),
            ..Default::default()
        })
    }

    fn panicking() -> Arc<Self> {
        Arc::new(Self {
            panic_on_call: true,
            ..Default::default()
        })
    }

    fn request_count(&self) -> usize {
        self.requests.lock().unwrap().len()
    }

    fn first_request(&self) -> LlmRequest {
        self.requests.lock().unwrap()[0].clone()
    }
}

#[async_trait]
impl LlmClient for StubClient {
    async fn complete(&self, req: LlmRequest) -> Result<LlmResponse, LlmError> {
        assert!(
            !self.panic_on_call,
            "StubClient::complete invoked but panic_on_call is true"
        );
        self.requests.lock().unwrap().push(req);
        let mut queue = self.replies.lock().unwrap();
        if queue.is_empty() {
            return Err(LlmError::Transport("stub out of replies".into()));
        }
        queue.remove(0)
    }
}

fn rules_text() -> Arc<str> {
    Arc::from("RULES_FIXTURE".to_owned().into_boxed_str())
}

fn card_catalog() -> Arc<str> {
    Arc::from("CARD_CATALOG_FIXTURE".to_owned().into_boxed_str())
}

fn cfg(llm: Arc<dyn LlmClient>) -> LlmAgentConfig {
    LlmAgentConfig {
        llm,
        model: "claude-haiku-test".into(),
        rules_text: rules_text(),
        card_catalog: card_catalog(),
        sidecar: None,
        max_tokens: 256,
        temperature: None,
    }
}

// -----------------------------------------------------------------------
// Happy path + system-block assertions
// -----------------------------------------------------------------------

#[tokio::test]
async fn happy_path_returns_index_and_updates_scratch() {
    let stub = StubClient::with_reply_texts(vec![
        "{\"action_index\": 0, \"plan\": \"attacking\", \"notes\": \"keep aces\"}".into(),
    ]);
    let mut agent: LlmAgent<CribbageGame> = LlmAgent::new(0, cfg(stub.clone()));

    let game = CribbageGame::new();
    let state = discard_phase_state(42);
    let view = game.public_view(&state, state.to_act);
    // Two dummy legal actions so the agent does not short-circuit.
    let legal = game.legal_actions(&state, state.to_act);
    assert!(legal.len() >= 2, "expected a discard phase with many legals");

    let idx = agent.choose(&view, &legal, &state).await.unwrap();
    assert_eq!(idx, 0);
    assert_eq!(agent.scratch().plan, "attacking");
    assert_eq!(agent.scratch().notes, "keep aces");
    assert_eq!(agent.scratch().turn_log.len(), 1);
    assert!(agent.scratch().turn_log[0].contains("chose index=0"));
}

#[tokio::test]
async fn first_turn_system_blocks_carry_cache_flags() {
    let stub = StubClient::with_reply_texts(vec![
        "{\"action_index\": 0, \"plan\": \"\", \"notes\": \"\"}".into(),
    ]);
    let mut agent: LlmAgent<CribbageGame> = LlmAgent::new(0, cfg(stub.clone()));

    let game = CribbageGame::new();
    let state = discard_phase_state(42);
    let view = game.public_view(&state, state.to_act);
    let legal = game.legal_actions(&state, state.to_act);
    let _ = agent.choose(&view, &legal, &state).await.unwrap();

    let req = stub.first_request();
    assert_eq!(req.system_blocks.len(), 3);
    assert!(req.system_blocks[0].cache);
    assert!(req.system_blocks[1].cache);
    assert!(!req.system_blocks[2].cache);
    assert_eq!(req.system_blocks[0].text, "RULES_FIXTURE");
    assert_eq!(req.system_blocks[1].text, "CARD_CATALOG_FIXTURE");
    assert_eq!(req.model, "claude-haiku-test");
    assert_eq!(req.messages.len(), 1);
}

#[tokio::test]
async fn single_legal_action_short_circuits_without_calling_llm() {
    // The stub panics on any complete() call.
    let stub = StubClient::panicking();
    let mut agent: LlmAgent<CribbageGame> = LlmAgent::new(0, cfg(stub));

    // Build a small legal vector with exactly one entry; view and state
    // content does not matter because the agent never calls the LLM.
    let game = CribbageGame::new();
    let state = discard_phase_state(1);
    let view = game.public_view(&state, state.to_act);
    let legal_full = game.legal_actions(&state, state.to_act);
    let legal = vec![legal_full[0].clone()];

    let idx = agent.choose(&view, &legal, &state).await.unwrap();
    assert_eq!(idx, 0);
    assert!(agent.scratch().turn_log[0].contains("forced"));
}

#[tokio::test]
async fn reply_keys_in_any_order_parse_correctly() {
    let stub = StubClient::with_reply_texts(vec![
        "{\"notes\": \"x\", \"plan\": \"y\", \"action_index\": 2}".into(),
    ]);
    let mut agent: LlmAgent<CribbageGame> = LlmAgent::new(0, cfg(stub));

    let game = CribbageGame::new();
    let state = discard_phase_state(42);
    let view = game.public_view(&state, state.to_act);
    let legal = game.legal_actions(&state, state.to_act);
    assert!(legal.len() > 3);
    let idx = agent.choose(&view, &legal, &state).await.unwrap();
    assert_eq!(idx, 2);
    assert_eq!(agent.scratch().plan, "y");
    assert_eq!(agent.scratch().notes, "x");
}

// -----------------------------------------------------------------------
// Error paths
// -----------------------------------------------------------------------

#[tokio::test]
async fn non_json_reply_retries_once_then_surfaces_parse_failure() {
    let stub = StubClient::with_reply_texts(vec![
        "I choose the first one.".into(),
        "still not JSON, sorry".into(),
    ]);
    let mut agent: LlmAgent<CribbageGame> = LlmAgent::new(0, cfg(stub.clone()));

    let game = CribbageGame::new();
    let state = discard_phase_state(42);
    let view = game.public_view(&state, state.to_act);
    let legal = game.legal_actions(&state, state.to_act);
    let err = agent.choose(&view, &legal, &state).await.unwrap_err();
    match err {
        AgentError::Other(msg) => {
            assert!(msg.contains("failed to parse"), "unexpected message: {msg}");
        }
        other @ AgentError::Timeout => panic!("expected AgentError::Other, got {other:?}"),
    }
    assert_eq!(stub.request_count(), 2, "retry must call LLM exactly twice");
}

#[tokio::test]
async fn out_of_range_action_index_is_rejected() {
    let stub = StubClient::with_reply_texts(vec![
        "{\"action_index\": 99, \"plan\": \"\", \"notes\": \"\"}".into(),
    ]);
    let mut agent: LlmAgent<CribbageGame> = LlmAgent::new(0, cfg(stub));

    let game = CribbageGame::new();
    let state = discard_phase_state(42);
    let view = game.public_view(&state, state.to_act);
    let legal = game.legal_actions(&state, state.to_act);
    // Sanity: we really have fewer than 99 legal actions.
    assert!(legal.len() < 99);
    let err = agent.choose(&view, &legal, &state).await.unwrap_err();
    match err {
        AgentError::Other(msg) => {
            assert!(msg.contains("out of"), "unexpected message: {msg}");
            assert!(msg.contains("range"));
        }
        other @ AgentError::Timeout => panic!("expected AgentError::Other, got {other:?}"),
    }
}

#[tokio::test]
async fn budget_exceeded_error_surfaces_as_agent_other() {
    let stub = StubClient::with_results(vec![Err(LlmError::BudgetExceeded {
        requested: 100,
        remaining: 50,
    })]);
    let mut agent: LlmAgent<CribbageGame> = LlmAgent::new(0, cfg(stub));

    let game = CribbageGame::new();
    let state = discard_phase_state(42);
    let view = game.public_view(&state, state.to_act);
    let legal = game.legal_actions(&state, state.to_act);
    let err = agent.choose(&view, &legal, &state).await.unwrap_err();
    match err {
        AgentError::Other(msg) => {
            assert!(
                msg.to_lowercase().contains("budget"),
                "unexpected message: {msg}"
            );
        }
        other @ AgentError::Timeout => panic!("expected AgentError::Other, got {other:?}"),
    }
}

// -----------------------------------------------------------------------
// Full-game integration
// -----------------------------------------------------------------------

/// A stub that always replies with action_index 0.
struct AlwaysFirstStub;

#[async_trait]
impl LlmClient for AlwaysFirstStub {
    async fn complete(&self, _req: LlmRequest) -> Result<LlmResponse, LlmError> {
        Ok(LlmResponse {
            text: "{\"action_index\": 0, \"plan\": \"first\", \"notes\": \"\"}".into(),
            input_tokens: 50,
            output_tokens: 10,
            cache_read_input_tokens: 40,
            cache_creation_input_tokens: 0,
        })
    }
}

#[tokio::test]
async fn full_cribbage_game_with_llm_agents_terminates() {
    let llm: Arc<dyn LlmClient> = Arc::new(AlwaysFirstStub);
    let game = CribbageGame::new();
    let mut agents: Vec<Box<dyn Agent<CribbageGame>>> = vec![
        Box::new(LlmAgent::<CribbageGame>::new(0, cfg(llm.clone()))),
        Box::new(LlmAgent::<CribbageGame>::new(1, cfg(llm.clone()))),
    ];

    let mut loop_ = GameLoop::new(&game, game.initial_state(42, &CribbageConfig));
    let mut chance_rng = ProductionRng::from_seed(42);
    let mut sink = StubGameEventSink::new();

    let result = loop_
        .run(agents.as_mut_slice(), &mut chance_rng, &mut sink)
        .await
        .unwrap_or_else(|e| panic!("LlmAgent game loop error: {e}"));

    assert_eq!(result.reason, EndReason::Victory);
    assert!(result.winner.is_some());

    // Sanity: the game advanced past the dealing phase at minimum.
    // Skipping the Agent lets ISMCTS and heuristic agents play too, but
    // here we just want to prove the LLM agent plays legally end-to-end.
    let _ = Actor::Player(0);
}
