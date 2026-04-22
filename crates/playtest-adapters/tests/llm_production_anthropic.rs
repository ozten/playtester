//! Integration tests for `ProductionLlmClient` against a mock Anthropic
//! HTTP server.
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
//! - Edge case: `OpenAICompat` with a non-local `base_url` is rejected
//!   at `new()` — covered in the OpenAI-compat test file; here we pin
//!   the error-when-unconfigured path.
//! - Error path: HTTP 429 retried once, then surfaces as `Transport`.
//! - Error path: malformed response body missing `usage` → `Transport`.
//! - Security: an error body containing the configured key is
//!   sanitized.
//! - Integration: record-then-playback against the mock server produces
//!   bit-for-bit identical responses including the extended token
//!   fields.

use playtest_adapters::{
    PlaybackLlmClient, ProductionLlmClient, ProductionLlmConfig, ProviderKind, RecordLlmClient,
    SecretString,
};
use playtest_ports::{ChatMessage, ChatRole, LlmClient, LlmError, LlmRequest, SystemBlock};
use serde_json::json;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use tempfile::tempdir;
use url::Url;
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, Respond, ResponseTemplate};

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

/// Build a production client pointed at `server` via the
/// crate-internal `with_anthropic_endpoint_override` test hook. Keeps
/// the Anthropic code path under test without touching environment
/// variables (which would violate the workspace's
/// `unsafe_code = "forbid"` lint via `std::env::set_var`).
fn fresh_client(server: &MockServer) -> ProductionLlmClient {
    let cfg = ProductionLlmConfig::new(ProviderKind::Anthropic, SecretString::new(TEST_KEY));
    ProductionLlmClient::new(cfg)
        .expect("client builds with real key")
        .with_anthropic_endpoint_override(server.uri())
}

#[tokio::test]
async fn happy_path_two_cached_blocks_decode_all_four_token_fields() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .and(header("x-api-key", TEST_KEY))
        .and(header("anthropic-version", "2023-06-01"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({
                "content": [{"type": "text", "text": "hello-world"}],
                "usage": {
                    "input_tokens": 11,
                    "output_tokens": 7,
                    "cache_read_input_tokens": 5,
                    "cache_creation_input_tokens": 9,
                },
            })),
        )
        .mount(&server)
        .await;

    let client = fresh_client(&server);
    let resp = client.complete(cached_req("hello")).await.unwrap();
    assert_eq!(resp.text, "hello-world");
    assert_eq!(resp.input_tokens, 11);
    assert_eq!(resp.output_tokens, 7);
    assert_eq!(resp.cache_read_input_tokens, 5);
    assert_eq!(resp.cache_creation_input_tokens, 9);
}

struct ConditionalCache {
    count: Arc<AtomicU32>,
}

impl Respond for ConditionalCache {
    fn respond(&self, _: &wiremock::Request) -> ResponseTemplate {
        let n = self.count.fetch_add(1, Ordering::SeqCst);
        let (creation, read) = if n == 0 { (120_u32, 0_u32) } else { (0, 120) };
        ResponseTemplate::new(200).set_body_json(json!({
            "content": [{"type": "text", "text": format!("turn-{n}")}],
            "usage": {
                "input_tokens": 130,
                "output_tokens": 20,
                "cache_read_input_tokens": read,
                "cache_creation_input_tokens": creation,
            },
        }))
    }
}

struct CountingResponder {
    count: Arc<AtomicU32>,
    status: u16,
    body: &'static str,
}

impl Respond for CountingResponder {
    fn respond(&self, _: &wiremock::Request) -> ResponseTemplate {
        self.count.fetch_add(1, Ordering::SeqCst);
        ResponseTemplate::new(self.status).set_body_string(self.body)
    }
}

#[tokio::test]
async fn two_consecutive_calls_surface_cache_creation_then_cache_read() {
    let server = MockServer::start().await;
    let call_count = Arc::new(AtomicU32::new(0));

    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(ConditionalCache {
            count: call_count.clone(),
        })
        .mount(&server)
        .await;

    let client = fresh_client(&server);
    let r1 = client.complete(cached_req("q1")).await.unwrap();
    let r2 = client.complete(cached_req("q2")).await.unwrap();

    assert!(r1.cache_creation_input_tokens > 0);
    assert_eq!(r1.cache_read_input_tokens, 0);
    assert_eq!(r2.cache_creation_input_tokens, 0);
    assert!(r2.cache_read_input_tokens > 0);
}

#[tokio::test]
async fn budget_exceeded_is_returned_before_any_http_call() {
    let server = MockServer::start().await;
    // Any request that reaches the server fails the test.
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(500))
        .expect(0)
        .mount(&server)
        .await;

    let cfg = ProductionLlmConfig::new(ProviderKind::Anthropic, SecretString::new(TEST_KEY))
        .with_budget_tokens(50);
    let client = ProductionLlmClient::new(cfg)
        .unwrap()
        .with_anthropic_endpoint_override(server.uri());

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
    let server = MockServer::start().await;
    let call_count = Arc::new(AtomicU32::new(0));

    Mock::given(method("POST"))
        .respond_with(CountingResponder {
            count: call_count.clone(),
            status: 429,
            body: "rate limited",
        })
        .mount(&server)
        .await;

    let client = fresh_client(&server);
    let err = client.complete(cached_req("x")).await.unwrap_err();
    match err {
        LlmError::Transport(msg) => {
            assert!(msg.contains("429"), "got: {msg}");
        }
        other => panic!("expected Transport, got {other:?}"),
    }
    assert_eq!(call_count.load(Ordering::SeqCst), 2, "should have retried once");
}

#[tokio::test]
async fn malformed_response_missing_usage_returns_transport() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "content": [{"type": "text", "text": "no usage here"}],
        })))
        .mount(&server)
        .await;

    let client = fresh_client(&server);
    let err = client.complete(cached_req("x")).await.unwrap_err();
    match err {
        LlmError::Transport(msg) => assert!(msg.contains("usage"), "got: {msg}"),
        other => panic!("expected Transport, got {other:?}"),
    }
}

#[tokio::test]
async fn error_body_containing_api_key_is_sanitized() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(
            ResponseTemplate::new(400)
                .set_body_string(format!("invalid: api key {TEST_KEY} was rejected")),
        )
        .mount(&server)
        .await;

    let client = fresh_client(&server);
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
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "content": [{"type": "text", "text": "record-me"}],
            "usage": {
                "input_tokens": 42,
                "output_tokens": 13,
                "cache_read_input_tokens": 8,
                "cache_creation_input_tokens": 3,
            },
        })))
        .mount(&server)
        .await;

    let dir = tempdir().unwrap();
    let tape = dir.path().join("llm.jsonl");
    let req = cached_req("tape");

    let recorded = {
        let inner = fresh_client(&server);
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
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "content": [{"type": "text", "text": "ok"}],
            "usage": {
                "input_tokens": 1,
                "output_tokens": 1,
                "cache_read_input_tokens": 0,
                "cache_creation_input_tokens": 0,
            },
        })))
        .mount(&server)
        .await;

    let client = fresh_client(&server);
    client.complete(cached_req("x")).await.unwrap();

    let recv = server.received_requests().await.unwrap();
    assert_eq!(recv.len(), 1);
    let body: serde_json::Value = serde_json::from_slice(&recv[0].body).unwrap();
    let sys = body.get("system").and_then(|v| v.as_array()).unwrap();
    assert_eq!(sys.len(), 2);
    for block in sys {
        assert_eq!(block["cache_control"]["type"], "ephemeral");
    }
    // Messages array must carry `user` role (Anthropic treats system at
    // top level).
    let msgs = body.get("messages").and_then(|v| v.as_array()).unwrap();
    assert_eq!(msgs[0]["role"], "user");
}
