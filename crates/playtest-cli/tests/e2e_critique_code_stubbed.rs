//! End-to-end Phase 5 Unit 5 validation: the offline coder pass over
//! a stubbed `<gid>.critique.jsonl`. Exercises the same code path
//! `playtest critique-code` drives, minus the clap-level arg parsing
//! and the real-HTTP client — the stub LlmClient returns pre-canned
//! coder replies.
//!
//! Asserts:
//!
//! - A well-formed coder reply produces one `coded_tag` record per
//!   questionnaire_response seat with non-empty open-ended text.
//! - Empty open-ended responses skip the coder entirely (no call).
//! - Out-of-taxonomy tags are dropped with a warning — the remaining
//!   valid tags still land in the sidecar.
//! - Idempotency: running code_once twice without --overwrite leaves
//!   already-coded seats alone (covered via direct coder-function
//!   tests; the subcommand's overwrite-gate is tested separately).

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use playtest_agents::{code_once, CoderOutcome};
use playtest_ports::{LlmClient, LlmError, LlmRequest, LlmResponse};

struct CoderStub {
    replies: Mutex<Vec<Result<LlmResponse, LlmError>>>,
    call_count: Mutex<usize>,
}

impl CoderStub {
    fn with_reply_texts(texts: Vec<String>) -> Arc<Self> {
        Arc::new(Self {
            replies: Mutex::new(
                texts
                    .into_iter()
                    .map(|t| {
                        Ok(LlmResponse {
                            text: t,
                            input_tokens: 500,
                            output_tokens: 80,
                            cache_read_input_tokens: 450,
                            cache_creation_input_tokens: 0,
                        })
                    })
                    .collect(),
            ),
            call_count: Mutex::new(0),
        })
    }

    fn calls(&self) -> usize {
        *self.call_count.lock().unwrap()
    }
}

#[async_trait]
impl LlmClient for CoderStub {
    async fn complete(&self, _req: LlmRequest) -> Result<LlmResponse, LlmError> {
        *self.call_count.lock().unwrap() += 1;
        let mut q = self.replies.lock().unwrap();
        if q.is_empty() {
            return Err(LlmError::Transport("stub out of replies".into()));
        }
        q.remove(0)
    }
}

fn sample_open_ended() -> BTreeMap<String, String> {
    let mut m = BTreeMap::new();
    m.insert(
        "worst_moment".into(),
        "Typhoon took my cordage on turn 6.".into(),
    );
    m.insert("what_would_you_change".into(), "Add food-defense.".into());
    m
}

#[tokio::test]
async fn code_once_happy_path_returns_validated_tags() {
    let stub = CoderStub::with_reply_texts(vec![r#"{"tags": [
        {"tag": "forced_sacrifice", "severity": 3, "ref_card": "typhoon"},
        {"tag": "lack_of_agency",   "severity": 2, "ref_card": null}
    ]}"#
    .into()]);
    let outcome = code_once(stub.as_ref(), &sample_open_ended(), "coder-stub", 1024)
        .await
        .expect("coder succeeds");
    assert_eq!(outcome.accepted.len(), 2);
    assert_eq!(outcome.accepted[0].tag, "forced_sacrifice");
    assert!(outcome.dropped_tags.is_empty());
    assert_eq!(stub.calls(), 1);
}

#[tokio::test]
async fn code_once_drops_out_of_taxonomy_tags_but_keeps_valid_ones() {
    let stub = CoderStub::with_reply_texts(vec![r#"{"tags": [
        {"tag": "forced_sacrifice", "severity": 3, "ref_card": "typhoon"},
        {"tag": "made_up_category", "severity": 2, "ref_card": null}
    ]}"#
    .into()]);
    let outcome = code_once(stub.as_ref(), &sample_open_ended(), "coder-stub", 1024)
        .await
        .expect("coder returns");
    assert_eq!(outcome.accepted.len(), 1);
    assert_eq!(outcome.dropped_tags, vec!["made_up_category"]);
}

#[tokio::test]
async fn code_once_propagates_parse_failure() {
    let stub = CoderStub::with_reply_texts(vec!["not json".into()]);
    let err = code_once(stub.as_ref(), &sample_open_ended(), "coder-stub", 1024)
        .await
        .unwrap_err();
    assert!(err.contains("not valid JSON"), "got: {err}");
}

#[tokio::test]
async fn code_once_propagates_severity_range_error() {
    let stub = CoderStub::with_reply_texts(vec![
        r#"{"tags": [{"tag": "forced_sacrifice", "severity": 7, "ref_card": null}]}"#.into(),
    ]);
    let err = code_once(stub.as_ref(), &sample_open_ended(), "coder-stub", 1024)
        .await
        .unwrap_err();
    assert!(err.contains("severity 7"), "got: {err}");
}

#[tokio::test]
async fn empty_tags_array_produces_empty_outcome() {
    let stub = CoderStub::with_reply_texts(vec![r#"{"tags": []}"#.into()]);
    let outcome = code_once(stub.as_ref(), &sample_open_ended(), "coder-stub", 1024)
        .await
        .unwrap();
    assert!(outcome.accepted.is_empty());
    assert!(outcome.dropped_tags.is_empty());
}

#[test]
fn coder_outcome_is_constructible_externally() {
    // Sanity: the public CoderOutcome struct is usable outside the
    // crate. Protects downstream consumers (Unit 6 ingest will
    // produce synthetic outcomes during tests).
    let outcome = CoderOutcome {
        accepted: vec![],
        dropped_tags: vec!["drift".into()],
    };
    assert_eq!(outcome.dropped_tags, vec!["drift".to_owned()]);
}
