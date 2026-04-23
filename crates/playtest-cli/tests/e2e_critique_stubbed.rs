//! End-to-end Phase 5 validation: a 2-player Cribbage game driven by
//! two stubbed LlmAgents with `--critique`-equivalent plumbing, end-to-
//! end through the registry-level dispatcher. Proves:
//!
//! 1. The main JSONL log is byte-clean of critique records (R5.7).
//! 2. The `<gid>.critique.jsonl` sidecar has exactly one
//!    `questionnaire_response` per LLM seat.
//! 3. Mixed-agent runs (`llm,random`) emit exactly one record
//!    (random seat is skipped).
//! 4. Running without critique plumbing produces no `.critique.jsonl`
//!    file at all.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use playtest_adapters::{ProductionFileSystem, ProductionGameEventSink};
use playtest_agents::{
    CritiqueSidecar, CritiqueSidecarHeader, QuestionnaireSpec, default_questionnaire_v1,
    sha256_hex,
};
use playtest_ports::{FileSystem, LlmClient, LlmError, LlmRequest, LlmResponse};
use playtest_registry::game_registry::lookup as lookup_game;
use playtest_registry::play::{LlmCliDeps, RunExtras, run_single_game_into_sink_with_extras};
use tempfile::TempDir;
use tokio::sync::Mutex as TokioMutex;

/// Stub LLM: returns a well-formed questionnaire on the last call
/// (the critique) and `{"action_index": 0, "plan": "", "notes": ""}`
/// on every gameplay call. We detect critique requests by the
/// presence of `"questionnaire"` or the instruction keyword `likert`
/// in the system blocks — the simplest reliable signal.
struct DualStub {
    gameplay_replies: Mutex<usize>,
    critique_reply: String,
    requests: Mutex<Vec<LlmRequest>>,
}

impl DualStub {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            gameplay_replies: Mutex::new(0),
            critique_reply: r#"{
                "likert": {"agency":4,"fairness":5,"tension":3,"pacing":4,"variety":3,"frustration":2,"satisfaction":4,"would_play_again":5},
                "open_ended": {"worst_moment":"nothing serious","what_would_you_change":"no change"}
            }"#.to_owned(),
            requests: Mutex::new(Vec::new()),
        })
    }

    fn critique_call_count(&self) -> usize {
        self.requests
            .lock()
            .unwrap()
            .iter()
            .filter(|r| r.system_blocks.last().is_some_and(|b| b.text.contains("Likert")))
            .count()
    }
}

#[async_trait]
impl LlmClient for DualStub {
    async fn complete(&self, req: LlmRequest) -> Result<LlmResponse, LlmError> {
        // Detect critique via the last system block (critique
        // instructions enumerate every Likert key).
        let is_critique = req
            .system_blocks
            .last()
            .is_some_and(|b| b.text.contains("Likert") && b.text.contains("agency"));
        self.requests.lock().unwrap().push(req);
        let text = if is_critique {
            self.critique_reply.clone()
        } else {
            *self.gameplay_replies.lock().unwrap() += 1;
            r#"{"action_index":0,"plan":"","notes":""}"#.to_owned()
        };
        Ok(LlmResponse {
            text,
            input_tokens: 100,
            output_tokens: 20,
            cache_read_input_tokens: 80,
            cache_creation_input_tokens: 0,
        })
    }
}

fn fs_handle() -> Arc<TokioMutex<dyn FileSystem + Send>> {
    Arc::new(TokioMutex::new(ProductionFileSystem::new()))
}

#[test]
fn llm_llm_run_with_critique_emits_two_questionnaire_records() {
    let out = TempDir::new().unwrap();
    let log_path = out.path().join("game-0000.jsonl");
    let critique_path = out.path().join("game-0000.critique.jsonl");

    let spec = Arc::new(default_questionnaire_v1());
    let rules_text = include_str!("../../games/cribbage/rules_for_llm.md");
    let header = CritiqueSidecarHeader::new(
        "cribbage",
        42,
        spec.sha256(),
        sha256_hex(rules_text.as_bytes()),
    );

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let critique_sidecar = Arc::new(
        rt.block_on(CritiqueSidecar::new(fs_handle(), critique_path.clone(), header))
            .unwrap(),
    );
    drop(rt);

    let stub = DualStub::new();
    let llm_deps = LlmCliDeps {
        client: stub.clone() as Arc<dyn LlmClient>,
        sidecar: None,
        model: "claude-stub".into(),
        max_tokens: Some(256),
        critique_sidecar: Some(critique_sidecar.clone()),
        critique_spec: Some(spec.clone()),
    };
    let extras = RunExtras::new().with_llm_deps(&llm_deps);
    let game = lookup_game("cribbage").unwrap();
    let agents = vec!["llm".to_owned(), "llm".to_owned()];

    let fs = ProductionFileSystem::new();
    let mut sink = ProductionGameEventSink::new(fs, &log_path);
    run_single_game_into_sink_with_extras(&game, &agents, 42, Some(0), &extras, &mut sink)
        .expect("game runs");

    // --- Main log R5.7 invariant ---
    let main_log = std::fs::read_to_string(&log_path).unwrap();
    assert!(
        !main_log.contains("questionnaire_response"),
        "main JSONL log must not contain critique records (R5.7)"
    );
    assert!(
        !main_log.contains("coded_tag"),
        "main JSONL log must not contain coded_tag records (R5.7)"
    );

    // --- Critique sidecar content ---
    let critique_text = std::fs::read_to_string(&critique_path).unwrap();
    let lines: Vec<&str> = critique_text.lines().collect();
    assert_eq!(lines.len(), 3, "header + 2 questionnaire_response lines");
    assert!(lines[0].contains("critique_sidecar_header"));
    assert!(lines[1].contains("questionnaire_response"));
    assert!(lines[2].contains("questionnaire_response"));
    // Both seats represented.
    let seats_seen: Vec<_> = lines
        .iter()
        .skip(1)
        .filter_map(|l| l.find("\"seat\":").map(|i| l.as_bytes()[i + 7]))
        .collect();
    assert!(seats_seen.contains(&b'0'));
    assert!(seats_seen.contains(&b'1'));

    assert_eq!(
        stub.critique_call_count(),
        2,
        "critique LLM call count must equal LLM seat count"
    );
}

#[test]
fn llm_random_run_with_critique_skips_random_seat() {
    let out = TempDir::new().unwrap();
    let log_path = out.path().join("game-0000.jsonl");
    let critique_path = out.path().join("game-0000.critique.jsonl");

    let spec = Arc::new(default_questionnaire_v1());
    let header = CritiqueSidecarHeader::new("cribbage", 7, spec.sha256(), sha256_hex(b""));
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let critique_sidecar = Arc::new(
        rt.block_on(CritiqueSidecar::new(fs_handle(), critique_path.clone(), header))
            .unwrap(),
    );
    drop(rt);

    let stub = DualStub::new();
    let llm_deps = LlmCliDeps {
        client: stub.clone() as Arc<dyn LlmClient>,
        sidecar: None,
        model: "claude-stub".into(),
        max_tokens: Some(256),
        critique_sidecar: Some(critique_sidecar),
        critique_spec: Some(spec),
    };
    let extras = RunExtras::new().with_llm_deps(&llm_deps);
    let game = lookup_game("cribbage").unwrap();
    let agents = vec!["llm".to_owned(), "random".to_owned()];

    let fs = ProductionFileSystem::new();
    let mut sink = ProductionGameEventSink::new(fs, &log_path);
    run_single_game_into_sink_with_extras(&game, &agents, 7, Some(0), &extras, &mut sink).unwrap();

    let critique_text = std::fs::read_to_string(&critique_path).unwrap();
    let lines: Vec<&str> = critique_text.lines().collect();
    assert_eq!(
        lines.len(),
        2,
        "header + 1 questionnaire_response (llm seat only)"
    );
    assert!(lines[1].contains("questionnaire_response"));
    assert!(lines[1].contains("\"seat\":0"));
    assert_eq!(stub.critique_call_count(), 1);
}

#[test]
fn run_without_critique_deps_does_not_create_critique_sidecar() {
    // Sanity: the opt-in path is off by default. Back-compat with
    // every Phase-3 caller.
    let out = TempDir::new().unwrap();
    let log_path = out.path().join("game-0000.jsonl");
    let critique_path = out.path().join("game-0000.critique.jsonl");

    let stub = DualStub::new();
    let llm_deps = LlmCliDeps {
        client: stub.clone() as Arc<dyn LlmClient>,
        sidecar: None,
        model: "claude-stub".into(),
        max_tokens: Some(256),
        critique_sidecar: None,
        critique_spec: None,
    };
    let extras = RunExtras::new().with_llm_deps(&llm_deps);
    let game = lookup_game("cribbage").unwrap();
    let agents = vec!["llm".to_owned(), "random".to_owned()];

    let fs = ProductionFileSystem::new();
    let mut sink = ProductionGameEventSink::new(fs, &log_path);
    run_single_game_into_sink_with_extras(&game, &agents, 7, Some(0), &extras, &mut sink).unwrap();

    assert!(
        !critique_path.exists(),
        "critique sidecar must not be created when critique_spec/critique_sidecar are None"
    );
    assert_eq!(stub.critique_call_count(), 0);
}

// Silence "unused type import" lints — the helper types are needed
// elsewhere in the test fixture setup.
#[allow(dead_code)]
fn _type_silencer(_s: QuestionnaireSpec) {}
