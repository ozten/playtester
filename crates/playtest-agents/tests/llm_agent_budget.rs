//! When `LlmClient` returns `BudgetExceeded`, `LlmAgent` must:
//! - surface the error as `AgentError::Other` containing "budget", and
//! - append a sidecar record with `budget_exceeded = true` and
//!   `chosen_index = None`.

use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use playtest_adapters::{ProductionRng, StubFileSystem};
use playtest_agents::{LlmAgent, LlmAgentConfig, LlmSidecar, SidecarHeader};
use playtest_core::{Actor, Agent, AgentError, Game};
use playtest_cribbage::{CribbageConfig, CribbageGame};
use playtest_ports::{FileSystem, LlmClient, LlmError, LlmRequest, LlmResponse};
use tokio::sync::Mutex;

/// See comment in `llm_agent_cribbage_stub.rs` — advance past Deal so
/// there are real legal actions to hand to the agent.
fn discard_phase_state(seed: u64) -> <CribbageGame as Game>::State {
    let game = CribbageGame::new();
    let mut state = game.initial_state(seed, &CribbageConfig);
    let mut rng = ProductionRng::from_seed(seed);
    while matches!(game.next_actor(&state), Actor::Chance) {
        let event = game.resolve_chance(&state, &mut rng).unwrap();
        game.apply_event(&mut state, &event);
    }
    state
}

struct BudgetStub;

#[async_trait]
impl LlmClient for BudgetStub {
    async fn complete(&self, _req: LlmRequest) -> Result<LlmResponse, LlmError> {
        Err(LlmError::BudgetExceeded {
            requested: 100,
            remaining: 50,
        })
    }
}

#[tokio::test]
async fn budget_exceeded_writes_sidecar_and_returns_error() {
    let fs: Arc<Mutex<dyn FileSystem + Send>> = Arc::new(Mutex::new(StubFileSystem::new()));
    let path = PathBuf::from("/run/games/g1.llm.jsonl");
    let sidecar = Arc::new(
        LlmSidecar::new(
            fs.clone(),
            path.clone(),
            SidecarHeader::new("cribbage", 7, "rules-hash", "catalog-hash"),
        )
        .await
        .unwrap(),
    );

    let cfg = LlmAgentConfig {
        llm: Arc::new(BudgetStub),
        model: "claude-haiku-test".into(),
        rules_text: Arc::from("RULES".to_owned().into_boxed_str()),
        card_catalog: Arc::from("CATALOG".to_owned().into_boxed_str()),
        sidecar: Some(sidecar),
        max_tokens: 128,
        temperature: None,
    };

    let state = discard_phase_state(42);
    let mut agent: LlmAgent<CribbageGame> = LlmAgent::new(state.to_act, cfg);

    let game = CribbageGame::new();
    let view = game.public_view(&state, state.to_act);
    let legal = game.legal_actions(&state, state.to_act);
    assert!(legal.len() > 1, "test requires multi-action turn");

    let err = agent.choose(&view, &legal, &state).await.unwrap_err();
    match err {
        AgentError::Other(msg) => assert!(
            msg.to_lowercase().contains("budget"),
            "unexpected message: {msg}"
        ),
        other @ AgentError::Timeout => panic!("expected AgentError::Other, got {other:?}"),
    }

    // Inspect sidecar contents via the filesystem port.
    let guard = fs.lock().await;
    let bytes = guard.read(&path).unwrap();
    let text = String::from_utf8(bytes).unwrap();
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 2, "expected header + 1 call record");
    assert!(lines[0].contains("\"kind\":\"sidecar_header\""));
    assert!(lines[1].contains("\"kind\":\"llm_call\""));
    assert!(lines[1].contains("\"budget_exceeded\":true"));
    assert!(lines[1].contains("\"chosen_index\":null"));
}
