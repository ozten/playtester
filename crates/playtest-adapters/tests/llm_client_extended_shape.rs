//! Coverage for the Phase 3 `LlmRequest`/`LlmResponse` shape extensions.
//!
//! The roundtrip integration test in `record_playback_roundtrip.rs`
//! exercises one cache-enabled request; this file pins the specific
//! scenarios from Unit 1 of the Phase 3 plan:
//!
//! 1. Stub round-trips the extended request/response unchanged.
//! 2. Record-then-playback tape bytes decode back into an identical
//!    request/response pair.
//! 3. Empty `system_blocks` + single user message works.
//! 4. Four `SystemBlock`s each with `cache: true` round-trip through a
//!    tape.
//! 5. Playback with a diverging request returns `TapeDivergence`.

use playtest_adapters::{PlaybackLlmClient, RecordLlmClient, StubLlmClient};
use playtest_ports::{
    ChatMessage, ChatRole, LlmClient, LlmError, LlmRequest, LlmResponse, SystemBlock,
};
use tempfile::tempdir;

fn user_req(
    system_blocks: Vec<SystemBlock>,
    user: &str,
    temperature: Option<f32>,
) -> LlmRequest {
    LlmRequest {
        system_blocks,
        messages: vec![ChatMessage {
            role: ChatRole::User,
            content: user.into(),
        }],
        model: "claude-test".into(),
        max_tokens: 128,
        temperature,
    }
}

fn assert_responses_eq(a: &LlmResponse, b: &LlmResponse) {
    assert_eq!(a.text, b.text);
    assert_eq!(a.input_tokens, b.input_tokens);
    assert_eq!(a.output_tokens, b.output_tokens);
    assert_eq!(a.cache_read_input_tokens, b.cache_read_input_tokens);
    assert_eq!(
        a.cache_creation_input_tokens,
        b.cache_creation_input_tokens
    );
}

#[tokio::test]
async fn stub_roundtrips_extended_shape() {
    let stub = StubLlmClient::new("ok")
        .with_token_counts(11, 7)
        .with_cache_tokens(9, 3);
    let req = user_req(
        vec![SystemBlock {
            text: "you are a helpful assistant".into(),
            cache: true,
        }],
        "hello",
        Some(0.5),
    );
    let resp = stub.complete(req).await.unwrap();
    assert_eq!(resp.text, "ok");
    assert_eq!(resp.input_tokens, 11);
    assert_eq!(resp.output_tokens, 7);
    assert_eq!(resp.cache_read_input_tokens, 9);
    assert_eq!(resp.cache_creation_input_tokens, 3);
}

#[tokio::test]
async fn empty_system_blocks_and_single_user_message_works() {
    let stub = StubLlmClient::new("hi");
    let req = user_req(vec![], "ping", None);
    let resp = stub.complete(req).await.unwrap();
    assert_eq!(resp.text, "hi");
    assert_eq!(resp.cache_read_input_tokens, 0);
    assert_eq!(resp.cache_creation_input_tokens, 0);
}

#[tokio::test]
async fn four_cached_system_blocks_round_trip_through_tape() {
    let dir = tempdir().unwrap();
    let tape = dir.path().join("llm_four_cached.jsonl");

    let system_blocks = (0..4)
        .map(|i| SystemBlock {
            text: format!("system block {i}"),
            cache: true,
        })
        .collect::<Vec<_>>();
    let req = user_req(system_blocks, "four-cache-blocks", Some(0.1));

    let recorded = {
        let stub = StubLlmClient::new("four-cache-ok")
            .with_token_counts(100, 10)
            .with_cache_tokens(80, 20);
        let mut record = RecordLlmClient::create(stub, &tape).unwrap();
        let resp = record.complete(req.clone()).await.unwrap();
        record.flush().unwrap();
        resp
    };

    let playback = PlaybackLlmClient::open(&tape).unwrap();
    let replayed = playback.complete(req.clone()).await.unwrap();

    assert_responses_eq(&recorded, &replayed);
    assert_eq!(replayed.cache_read_input_tokens, 80);
    assert_eq!(replayed.cache_creation_input_tokens, 20);
}

#[tokio::test]
async fn playback_divergence_surfaces_as_tape_divergence_error() {
    let dir = tempdir().unwrap();
    let tape = dir.path().join("llm_divergence.jsonl");

    {
        let stub = StubLlmClient::new("recorded");
        let mut record = RecordLlmClient::create(stub, &tape).unwrap();
        let _ = record
            .complete(user_req(
                vec![SystemBlock {
                    text: "sys".into(),
                    cache: false,
                }],
                "recorded-input",
                None,
            ))
            .await
            .unwrap();
        record.flush().unwrap();
    }

    let playback = PlaybackLlmClient::open(&tape).unwrap();
    let err = playback
        .complete(user_req(
            vec![SystemBlock {
                text: "sys".into(),
                cache: false,
            }],
            "diverging-input",
            None,
        ))
        .await
        .unwrap_err();
    assert!(matches!(err, LlmError::TapeDivergence));
}
