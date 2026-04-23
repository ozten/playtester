//! Integration tests for `LlmAgent::post_game_critique` — the post-
//! game questionnaire call that emits one `questionnaire_response`
//! record into a `CritiqueSidecar`.
//!
//! Structurally mirrors `llm_agent_cribbage_stub.rs` — a hand-rolled
//! stub `LlmClient` captures every request and lets the test pre-
//! queue replies. No real HTTP traffic.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use playtest_adapters::{ProductionFileSystem, ProductionRng};
use playtest_agents::{
    CritiqueSidecar, CritiqueSidecarHeader, LlmAgent, LlmAgentConfig, QuestionnaireSpec,
    default_questionnaire_v1,
};
use playtest_core::{Actor, AgentError, EndReason, Game, GameResult};
use playtest_cribbage::{CribbageConfig, CribbageGame};
use playtest_ports::{FileSystem, LlmClient, LlmError, LlmRequest, LlmResponse};
use tokio::sync::Mutex as TokioMutex;

/// Stub LlmClient: pre-queued replies, captured requests.
struct StubClient {
    replies: Mutex<Vec<Result<LlmResponse, LlmError>>>,
    requests: Mutex<Vec<LlmRequest>>,
}

impl StubClient {
    fn with_reply_texts(texts: Vec<String>) -> Arc<Self> {
        let responses: Vec<Result<LlmResponse, LlmError>> = texts
            .into_iter()
            .map(|t| {
                Ok(LlmResponse {
                    text: t,
                    input_tokens: 200,
                    output_tokens: 40,
                    cache_read_input_tokens: 150,
                    cache_creation_input_tokens: 0,
                })
            })
            .collect();
        Arc::new(Self {
            replies: Mutex::new(responses),
            requests: Mutex::new(Vec::new()),
        })
    }

    fn with_results(results: Vec<Result<LlmResponse, LlmError>>) -> Arc<Self> {
        Arc::new(Self {
            replies: Mutex::new(results),
            requests: Mutex::new(Vec::new()),
        })
    }

    fn request_count(&self) -> usize {
        self.requests.lock().unwrap().len()
    }

    fn last_user_message(&self) -> Option<String> {
        let reqs = self.requests.lock().unwrap();
        let last = reqs.last()?;
        last.messages
            .iter()
            .find(|m| matches!(m.role, playtest_ports::ChatRole::User))
            .map(|m| m.content.clone())
    }
}

#[async_trait]
impl LlmClient for StubClient {
    async fn complete(&self, req: LlmRequest) -> Result<LlmResponse, LlmError> {
        self.requests.lock().unwrap().push(req);
        let mut queue = self.replies.lock().unwrap();
        if queue.is_empty() {
            return Err(LlmError::Transport("stub out of replies".into()));
        }
        queue.remove(0)
    }
}

fn cfg(llm: Arc<dyn LlmClient>) -> LlmAgentConfig {
    LlmAgentConfig {
        llm,
        model: "claude-stub-critique".into(),
        rules_text: Arc::from("RULES_FIXTURE".to_owned().into_boxed_str()),
        card_catalog: Arc::from("CATALOG_FIXTURE".to_owned().into_boxed_str()),
        sidecar: None,
        max_tokens: 512,
        temperature: None,
    }
}

fn sample_view_and_result() -> (
    <CribbageGame as Game>::PublicView,
    GameResult,
) {
    let game = CribbageGame::new();
    let state = game.initial_state(42, &CribbageConfig);
    let view = game.public_view(&state, state.to_act);
    let result = GameResult {
        winner: Some(0_u8),
        reason: EndReason::Victory,
        scores: vec![121, 104],
    };
    (view, result)
}

fn reply_with_all_required_keys() -> String {
    r#"{
        "likert": {
            "agency": 4, "fairness": 5, "tension": 3, "pacing": 4,
            "variety": 3, "frustration": 2, "satisfaction": 4, "would_play_again": 5
        },
        "open_ended": {
            "worst_moment": "dealer flipped the his-heels jack against me",
            "what_would_you_change": "his heels should be two points, not three"
        }
    }"#
    .to_owned()
}

async fn fresh_sidecar() -> (Arc<CritiqueSidecar>, PathBuf, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("g.critique.jsonl");
    let fs: Arc<TokioMutex<dyn FileSystem + Send>> =
        Arc::new(TokioMutex::new(ProductionFileSystem::new()));
    let spec = default_questionnaire_v1();
    let header = CritiqueSidecarHeader::new("cribbage", 42, spec.sha256(), "rules-sha-stub");
    let sc = Arc::new(CritiqueSidecar::new(fs, path.clone(), header).await.unwrap());
    (sc, path, dir)
}

fn read_sidecar(path: &PathBuf) -> Vec<String> {
    std::fs::read_to_string(path)
        .unwrap()
        .lines()
        .map(str::to_owned)
        .collect()
}

// ---------------------------------------------------------------------

#[tokio::test]
async fn happy_path_appends_one_questionnaire_response_record() {
    let stub = StubClient::with_reply_texts(vec![reply_with_all_required_keys()]);
    let agent: LlmAgent<CribbageGame> = LlmAgent::new(0, cfg(stub.clone()));
    let (view, result) = sample_view_and_result();
    let (sidecar, path, _dir) = fresh_sidecar().await;
    let spec = default_questionnaire_v1();

    agent
        .post_game_critique(&view, &result, &spec, &sidecar, None)
        .await
        .expect("critique succeeds on well-formed reply");

    let lines = read_sidecar(&path);
    assert_eq!(lines.len(), 2, "header + one questionnaire_response");
    assert!(lines[0].contains("critique_sidecar_header"));
    assert!(lines[1].contains("\"kind\":\"questionnaire_response\""));
    assert!(lines[1].contains("\"seat\":0"));
    assert!(lines[1].contains("\"agency\":4"));
    assert_eq!(stub.request_count(), 1, "exactly one LLM call on happy path");
}

#[tokio::test]
async fn persona_addendum_is_threaded_into_user_message() {
    let stub = StubClient::with_reply_texts(vec![reply_with_all_required_keys()]);
    let agent: LlmAgent<CribbageGame> = LlmAgent::new(0, cfg(stub.clone()));
    let (view, result) = sample_view_and_result();
    let (sidecar, _path, _dir) = fresh_sidecar().await;
    let spec = default_questionnaire_v1();

    agent
        .post_game_critique(
            &view,
            &result,
            &spec,
            &sidecar,
            Some("You are an aggressive player who values tempo."),
        )
        .await
        .unwrap();

    let msg = stub.last_user_message().expect("captured request");
    assert!(
        msg.contains("persona_addendum"),
        "persona_addendum key must appear in user message; got: {msg}"
    );
    assert!(msg.contains("aggressive player"));
}

#[tokio::test]
async fn out_of_range_likert_triggers_retry_then_accepts_valid_reply() {
    // First reply has agency=9 (invalid); second is well-formed.
    let bad = r#"{
        "likert": {
            "agency": 9, "fairness": 3, "tension": 3, "pacing": 3,
            "variety": 3, "frustration": 3, "satisfaction": 3, "would_play_again": 3
        },
        "open_ended": {"worst_moment": "x", "what_would_you_change": "y"}
    }"#
    .to_owned();
    let stub = StubClient::with_reply_texts(vec![bad, reply_with_all_required_keys()]);
    let agent: LlmAgent<CribbageGame> = LlmAgent::new(0, cfg(stub.clone()));
    let (view, result) = sample_view_and_result();
    let (sidecar, path, _dir) = fresh_sidecar().await;
    let spec = default_questionnaire_v1();

    agent
        .post_game_critique(&view, &result, &spec, &sidecar, None)
        .await
        .expect("retry must succeed");
    assert_eq!(stub.request_count(), 2, "retry must call LLM exactly twice");
    let lines = read_sidecar(&path);
    assert_eq!(lines.len(), 2, "header + one questionnaire_response");
    assert!(lines[1].contains("\"agency\":4"));
}

#[tokio::test]
async fn non_json_then_non_json_surfaces_parse_failure() {
    let stub = StubClient::with_reply_texts(vec![
        "I think the game was okay.".into(),
        "Still not JSON, sorry.".into(),
    ]);
    let agent: LlmAgent<CribbageGame> = LlmAgent::new(0, cfg(stub.clone()));
    let (view, result) = sample_view_and_result();
    let (sidecar, path, _dir) = fresh_sidecar().await;
    let spec = default_questionnaire_v1();

    let err = agent
        .post_game_critique(&view, &result, &spec, &sidecar, None)
        .await
        .unwrap_err();
    match err {
        AgentError::Other(msg) => {
            assert!(
                msg.contains("critique parse failed"),
                "expected 'critique parse failed', got: {msg}"
            );
        }
        AgentError::Timeout => panic!("unexpected Timeout"),
    }
    assert_eq!(stub.request_count(), 2);
    // Sidecar got header only — no questionnaire_response record on failure.
    let lines = read_sidecar(&path);
    assert_eq!(lines.len(), 1, "header only — no record on parse failure");
}

#[tokio::test]
async fn budget_exceeded_on_first_call_surfaces_as_agent_other() {
    let stub = StubClient::with_results(vec![Err(LlmError::BudgetExceeded {
        requested: 512,
        remaining: 100,
    })]);
    let agent: LlmAgent<CribbageGame> = LlmAgent::new(0, cfg(stub.clone()));
    let (view, result) = sample_view_and_result();
    let (sidecar, path, _dir) = fresh_sidecar().await;
    let spec = default_questionnaire_v1();

    let err = agent
        .post_game_critique(&view, &result, &spec, &sidecar, None)
        .await
        .unwrap_err();
    match err {
        AgentError::Other(msg) => {
            assert!(
                msg.to_lowercase().contains("budget"),
                "expected 'budget' in message, got: {msg}"
            );
            assert!(msg.contains("critique"));
        }
        AgentError::Timeout => panic!("unexpected Timeout"),
    }
    // No retry on BudgetExceeded.
    assert_eq!(stub.request_count(), 1);
    // Sidecar: header only.
    let lines = read_sidecar(&path);
    assert_eq!(lines.len(), 1);
}

#[tokio::test]
async fn missing_required_likert_key_triggers_retry() {
    // First reply omits `variety`; second is complete.
    let missing = r#"{
        "likert": {
            "agency": 3, "fairness": 3, "tension": 3, "pacing": 3,
            "frustration": 3, "satisfaction": 3, "would_play_again": 3
        },
        "open_ended": {"worst_moment": "x", "what_would_you_change": "y"}
    }"#
    .to_owned();
    let stub = StubClient::with_reply_texts(vec![missing, reply_with_all_required_keys()]);
    let agent: LlmAgent<CribbageGame> = LlmAgent::new(0, cfg(stub.clone()));
    let (view, result) = sample_view_and_result();
    let (sidecar, path, _dir) = fresh_sidecar().await;
    let spec = default_questionnaire_v1();

    agent
        .post_game_critique(&view, &result, &spec, &sidecar, None)
        .await
        .expect("retry must succeed");
    assert_eq!(stub.request_count(), 2);
    assert_eq!(read_sidecar(&path).len(), 2);
}

#[tokio::test]
async fn first_call_sends_cached_rules_and_catalog_blocks() {
    let stub = StubClient::with_reply_texts(vec![reply_with_all_required_keys()]);
    let agent: LlmAgent<CribbageGame> = LlmAgent::new(0, cfg(stub.clone()));
    let (view, result) = sample_view_and_result();
    let (sidecar, _path, _dir) = fresh_sidecar().await;
    let spec = default_questionnaire_v1();

    agent
        .post_game_critique(&view, &result, &spec, &sidecar, None)
        .await
        .unwrap();

    let req = stub.requests.lock().unwrap()[0].clone();
    assert_eq!(req.system_blocks.len(), 3);
    assert!(req.system_blocks[0].cache, "rules block must be cacheable");
    assert_eq!(req.system_blocks[0].text, "RULES_FIXTURE");
    assert!(req.system_blocks[1].cache, "catalog block must be cacheable");
    assert_eq!(req.system_blocks[1].text, "CATALOG_FIXTURE");
    assert!(
        !req.system_blocks[2].cache,
        "critique instructions block must not be cached"
    );
    assert!(
        req.system_blocks[2].text.contains("agency"),
        "instructions must list every Likert key"
    );
}

#[tokio::test]
async fn scratch_is_not_mutated_by_critique() {
    // Seed a scratch buffer with pre-existing state; critique must
    // not overwrite it. (The agent scratch is a private field, so we
    // can't directly seed it — but we can run a gameplay `choose`
    // first and then confirm critique doesn't clobber the resulting
    // plan/notes.)
    //
    // Simpler check: run two critiques back-to-back and confirm the
    // request-count is 2 (i.e. each runs independently; no state
    // carried forward that would change the second prompt's shape).
    let stub = StubClient::with_reply_texts(vec![
        reply_with_all_required_keys(),
        reply_with_all_required_keys(),
    ]);
    let agent: LlmAgent<CribbageGame> = LlmAgent::new(0, cfg(stub.clone()));
    let (view, result) = sample_view_and_result();
    let (sidecar, path, _dir) = fresh_sidecar().await;
    let spec = default_questionnaire_v1();

    agent
        .post_game_critique(&view, &result, &spec, &sidecar, None)
        .await
        .unwrap();
    agent
        .post_game_critique(&view, &result, &spec, &sidecar, None)
        .await
        .unwrap();
    assert_eq!(read_sidecar(&path).len(), 3, "header + 2 responses");
    // Both calls should have byte-identical user messages (no
    // scratch drift between them).
    let reqs = stub.requests.lock().unwrap();
    let msg_a = reqs[0]
        .messages
        .iter()
        .find(|m| matches!(m.role, playtest_ports::ChatRole::User))
        .map(|m| &m.content);
    let msg_b = reqs[1]
        .messages
        .iter()
        .find(|m| matches!(m.role, playtest_ports::ChatRole::User))
        .map(|m| &m.content);
    assert_eq!(msg_a, msg_b, "back-to-back critiques must send the same prompt");
}

#[tokio::test]
async fn seat_id_round_trips_into_record() {
    let stub = StubClient::with_reply_texts(vec![reply_with_all_required_keys()]);
    let agent: LlmAgent<CribbageGame> = LlmAgent::new(1, cfg(stub.clone()));
    let (view, result) = sample_view_and_result();
    let (sidecar, path, _dir) = fresh_sidecar().await;
    let spec = default_questionnaire_v1();

    agent
        .post_game_critique(&view, &result, &spec, &sidecar, None)
        .await
        .unwrap();
    let lines = read_sidecar(&path);
    assert!(lines[1].contains("\"seat\":1"));
}

// Silence "unused import" warnings for imports the helper fns need
// only on their test-only code paths.
#[allow(dead_code)]
fn _unused_silencer(
    _x: ProductionRng,
    _y: Actor,
    _z: BTreeMap<String, u8>,
    _w: QuestionnaireSpec,
) {
}
