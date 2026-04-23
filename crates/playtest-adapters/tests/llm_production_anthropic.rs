//! Integration tests for `ProductionLlmClient` against a Pact mock
//! Anthropic provider.
//!
//! Scenarios covered (from Unit 3 of the Phase 3 plan):
//!
//! - Happy path: two cached `system_blocks` round-trip; all four token
//!   fields decode correctly.
//! - Happy path: two consecutive calls simulate cache creation on turn 1
//!   and cache read on turn 2 — proves the adapter plumbs the extended
//!   fields through.
//! - Edge case: budget pre-check returns `BudgetExceeded` without
//!   sending any HTTP request.
//! - Error path: HTTP 429 retried exactly once (two interactions must
//!   both be satisfied by the adapter's retry loop), then surfaces as
//!   `Transport`.
//! - Error path: malformed response body missing `usage` → `Transport`.
//! - Security: an error body containing the configured key is
//!   sanitized.
//! - Integration: record-then-playback against the mock server produces
//!   bit-for-bit identical responses including the extended token
//!   fields.
//! - Security: an `OpenAICompat` base_url with a non-local host is
//!   rejected at construction.
//! - Protocol: `cache_control: { type: "ephemeral" }` is emitted on
//!   `SystemBlock`s whose `cache` flag is true, and the `user` message
//!   is in the `messages` array (not folded into `system`).

use pact_consumer::prelude::*;
use playtest_adapters::{
    PlaybackLlmClient, ProductionLlmClient, ProductionLlmConfig, ProviderKind, RecordLlmClient,
    SecretString,
};
use playtest_ports::{ChatMessage, ChatRole, LlmClient, LlmError, LlmRequest, SystemBlock};
use serde_json::json;
use tempfile::tempdir;
use url::Url;

const TEST_KEY: &str = "sk-ant-test-KEY-123";

fn cached_req(user: &str) -> LlmRequest {
    LlmRequest {
        system_blocks: vec![
            SystemBlock {
                text: "Rules: play legally.".into(),
                cache: true,
            },
            SystemBlock {
                text: "Card catalog: A..K.".into(),
                cache: true,
            },
        ],
        messages: vec![ChatMessage {
            role: ChatRole::User,
            content: user.into(),
        }],
        model: "claude-haiku-test".into(),
        max_tokens: 64,
        temperature: Some(0.2),
    }
}

/// Strip the trailing slash from a pact mock-server URL so the adapter
/// can append `/v1/messages` cleanly.
fn mock_base(url: &Url) -> String {
    url.as_str().trim_end_matches('/').to_string()
}

fn anthropic_client(endpoint: impl Into<String>) -> ProductionLlmClient {
    let cfg = ProductionLlmConfig::new(ProviderKind::Anthropic, SecretString::new(TEST_KEY));
    ProductionLlmClient::new(cfg)
        .expect("client builds with real key")
        .with_anthropic_endpoint_override(endpoint)
}

fn usage_response(
    text: &str,
    input: u32,
    output: u32,
    cache_read: u32,
    cache_creation: u32,
) -> serde_json::Value {
    json!({
        "content": [{"type": "text", "text": text}],
        "usage": {
            "input_tokens": input,
            "output_tokens": output,
            "cache_read_input_tokens": cache_read,
            "cache_creation_input_tokens": cache_creation,
        },
    })
}

#[tokio::test]
async fn happy_path_two_cached_blocks_decode_all_four_token_fields() {
    let pact = PactBuilder::new("playtest-adapters", "anthropic-api")
        .interaction(
            "POST /v1/messages returns all four token fields",
            "",
            |mut i| {
                i.request
                    .method("POST")
                    .path("/v1/messages")
                    .header("x-api-key", TEST_KEY)
                    .header("anthropic-version", "2023-06-01");
                i.response
                    .status(200)
                    .header("content-type", "application/json")
                    .json_body(usage_response("hello-world", 11, 7, 5, 9));
                i
            },
        )
        .start_mock_server(None, None);

    let client = anthropic_client(mock_base(&pact.url()));
    let resp = client.complete(cached_req("hello")).await.unwrap();
    assert_eq!(resp.text, "hello-world");
    assert_eq!(resp.input_tokens, 11);
    assert_eq!(resp.output_tokens, 7);
    assert_eq!(resp.cache_read_input_tokens, 5);
    assert_eq!(resp.cache_creation_input_tokens, 9);
}

#[tokio::test]
async fn two_consecutive_calls_surface_cache_creation_then_cache_read() {
    // Two interactions differentiated by the user-message body, so
    // pact can pick the right response per call without relying on
    // ordering assumptions.
    let pact = PactBuilder::new("playtest-adapters", "anthropic-api")
        .interaction("first call — cache creation", "", |mut i| {
            i.request
                .method("POST")
                .path("/v1/messages")
                .json_body(json_pattern!({
                    "model": "claude-haiku-test",
                    "max_tokens": 64,
                    "system": [
                        {"type": "text", "text": "Rules: play legally.",
                         "cache_control": {"type": "ephemeral"}},
                        {"type": "text", "text": "Card catalog: A..K.",
                         "cache_control": {"type": "ephemeral"}},
                    ],
                    "messages": [{"role": "user", "content": "q1"}],
                    // `temperature` uses `like!` so the f32→f64 widening
                    // (`0.2f32 → 0.20000000298023224f64`) does not
                    // trip pact's exact decimal matcher — we only care
                    // that the field is a number.
                    "temperature": like!(0.2),
                }));
            i.response
                .status(200)
                .json_body(usage_response("turn-0", 130, 20, 0, 120));
            i
        })
        .interaction("second call — cache read", "", |mut i| {
            i.request
                .method("POST")
                .path("/v1/messages")
                .json_body(json_pattern!({
                    "model": "claude-haiku-test",
                    "max_tokens": 64,
                    "system": [
                        {"type": "text", "text": "Rules: play legally.",
                         "cache_control": {"type": "ephemeral"}},
                        {"type": "text", "text": "Card catalog: A..K.",
                         "cache_control": {"type": "ephemeral"}},
                    ],
                    "messages": [{"role": "user", "content": "q2"}],
                    "temperature": like!(0.2),
                }));
            i.response
                .status(200)
                .json_body(usage_response("turn-1", 130, 20, 120, 0));
            i
        })
        .start_mock_server(None, None);

    let client = anthropic_client(mock_base(&pact.url()));
    let r1 = client.complete(cached_req("q1")).await.unwrap();
    let r2 = client.complete(cached_req("q2")).await.unwrap();

    assert!(r1.cache_creation_input_tokens > 0);
    assert_eq!(r1.cache_read_input_tokens, 0);
    assert_eq!(r2.cache_creation_input_tokens, 0);
    assert!(r2.cache_read_input_tokens > 0);
}

#[tokio::test]
async fn budget_exceeded_is_returned_before_any_http_call() {
    // Zero interactions — pact's ValidatingMockServer panics on Drop
    // if the adapter fires an unexpected request, so this test also
    // asserts "no HTTP traffic" as a side effect.
    let pact = PactBuilder::new("playtest-adapters", "anthropic-api").start_mock_server(None, None);

    let cfg = ProductionLlmConfig::new(ProviderKind::Anthropic, SecretString::new(TEST_KEY))
        .with_budget_tokens(50);
    let client = ProductionLlmClient::new(cfg)
        .unwrap()
        .with_anthropic_endpoint_override(mock_base(&pact.url()));

    let mut req = cached_req("anything");
    req.max_tokens = 100;

    let err = client.complete(req).await.unwrap_err();
    match err {
        LlmError::BudgetExceeded {
            requested,
            remaining,
        } => {
            assert_eq!(requested, 100);
            assert_eq!(remaining, 50);
        }
        other => panic!("expected BudgetExceeded, got {other:?}"),
    }
}

#[tokio::test]
async fn http_429_retries_once_then_propagates_as_transport() {
    // Two interactions, both returning 429. The adapter's retry-once
    // policy must satisfy both — if it only fired one request, the
    // second interaction would be unmatched and pact's Drop would
    // panic; if it fired three, the third would be unexpected and
    // Drop would also panic.
    let pact = PactBuilder::new("playtest-adapters", "anthropic-api")
        .interaction("initial call — rate limited", "", |mut i| {
            i.request.method("POST").path("/v1/messages");
            i.response.status(429).body("rate limited");
            i
        })
        .interaction("retry — still rate limited", "", |mut i| {
            i.request.method("POST").path("/v1/messages");
            i.response.status(429).body("rate limited");
            i
        })
        .start_mock_server(None, None);

    let client = anthropic_client(mock_base(&pact.url()));
    let err = client.complete(cached_req("x")).await.unwrap_err();
    match err {
        LlmError::Transport(msg) => {
            assert!(msg.contains("429"), "got: {msg}");
        }
        other => panic!("expected Transport, got {other:?}"),
    }
}

#[tokio::test]
async fn malformed_response_missing_usage_returns_transport() {
    let pact = PactBuilder::new("playtest-adapters", "anthropic-api")
        .interaction("200 response missing `usage`", "", |mut i| {
            i.request.method("POST").path("/v1/messages");
            i.response.status(200).json_body(json!({
                "content": [{"type": "text", "text": "no usage here"}],
            }));
            i
        })
        .start_mock_server(None, None);

    let client = anthropic_client(mock_base(&pact.url()));
    let err = client.complete(cached_req("x")).await.unwrap_err();
    match err {
        LlmError::Transport(msg) => assert!(msg.contains("usage"), "got: {msg}"),
        other => panic!("expected Transport, got {other:?}"),
    }
}

#[tokio::test]
async fn error_body_containing_api_key_is_sanitized() {
    let pact = PactBuilder::new("playtest-adapters", "anthropic-api")
        .interaction("400 error body leaks the api key", "", |mut i| {
            i.request.method("POST").path("/v1/messages");
            i.response
                .status(400)
                .body(format!("invalid: api key {TEST_KEY} was rejected"));
            i
        })
        .start_mock_server(None, None);

    let client = anthropic_client(mock_base(&pact.url()));
    let err = client.complete(cached_req("x")).await.unwrap_err();
    match err {
        LlmError::Transport(msg) => {
            assert!(
                !msg.contains(TEST_KEY),
                "sanitized message should not contain key; got: {msg}"
            );
            assert!(msg.contains("sanitized"));
        }
        other => panic!("expected Transport, got {other:?}"),
    }
}

#[tokio::test]
async fn record_then_playback_against_mock_server_round_trips_exactly() {
    let pact = PactBuilder::new("playtest-adapters", "anthropic-api")
        .interaction("200 response used for record/playback round trip", "", |mut i| {
            i.request.method("POST").path("/v1/messages");
            i.response
                .status(200)
                .json_body(usage_response("record-me", 42, 13, 8, 3));
            i
        })
        .start_mock_server(None, None);

    let dir = tempdir().unwrap();
    let tape = dir.path().join("llm.jsonl");
    let req = cached_req("tape");

    let recorded = {
        let inner = anthropic_client(mock_base(&pact.url()));
        let mut rec = RecordLlmClient::create(inner, &tape).unwrap();
        let resp = rec.complete(req.clone()).await.unwrap();
        rec.flush().unwrap();
        resp
    };

    let playback = PlaybackLlmClient::open(&tape).unwrap();
    let replayed = playback.complete(req).await.unwrap();

    assert_eq!(recorded, replayed);
    assert_eq!(replayed.text, "record-me");
    assert_eq!(replayed.input_tokens, 42);
    assert_eq!(replayed.output_tokens, 13);
    assert_eq!(replayed.cache_read_input_tokens, 8);
    assert_eq!(replayed.cache_creation_input_tokens, 3);
}

#[tokio::test]
async fn ssrf_guard_at_construction_rejects_non_local_base_url() {
    let cfg = ProductionLlmConfig::new(
        ProviderKind::OpenAICompat {
            base_url: Url::parse("http://example.com/v1/").unwrap(),
        },
        SecretString::new(TEST_KEY),
    );
    let err = ProductionLlmClient::new(cfg).unwrap_err();
    match err {
        LlmError::Transport(msg) => {
            assert!(msg.contains("example.com"));
            assert!(msg.contains("localhost"));
        }
        other => panic!("expected Transport, got {other:?}"),
    }
}

#[tokio::test]
async fn request_sends_cache_control_ephemeral_on_cached_system_blocks() {
    // The expected request body encodes the full invariant this test
    // protects: both system blocks carry `cache_control: ephemeral`
    // and the user message is in the `messages` array (not folded
    // into `system`). Pact only returns 200 if the adapter sends a
    // byte-compatible body.
    let pact = PactBuilder::new("playtest-adapters", "anthropic-api")
        .interaction(
            "request has cache_control ephemeral on cached system blocks",
            "",
            |mut i| {
                i.request
                    .method("POST")
                    .path("/v1/messages")
                    .json_body(json_pattern!({
                        "model": "claude-haiku-test",
                        "max_tokens": 64,
                        "system": [
                            {"type": "text", "text": "Rules: play legally.",
                             "cache_control": {"type": "ephemeral"}},
                            {"type": "text", "text": "Card catalog: A..K.",
                             "cache_control": {"type": "ephemeral"}},
                        ],
                        "messages": [{"role": "user", "content": "x"}],
                        "temperature": like!(0.2),
                    }));
                i.response.status(200).json_body(usage_response("ok", 1, 1, 0, 0));
                i
            },
        )
        .start_mock_server(None, None);

    let client = anthropic_client(mock_base(&pact.url()));
    client.complete(cached_req("x")).await.unwrap();
}
