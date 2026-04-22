//! Integration tests for `ProductionLlmClient` with an
//! OpenAI-compatible provider (Ollama / llama.cpp shape).
//!
//! The wiremock server is bound on `127.0.0.1`, which passes the SSRF
//! guard. Covered scenarios:
//!
//! - Happy path: both `system_blocks` are concatenated into one
//!   `system` message in the outgoing payload; `cache_*` fields on the
//!   response are zeroed.
//! - SSRF guard: `base_url` with a public host is rejected at `new()`.
//! - Budget pre-check still fires before any HTTP traffic on this
//!   provider.

use playtest_adapters::{
    ProductionLlmClient, ProductionLlmConfig, ProviderKind, SecretString,
};
use playtest_ports::{ChatMessage, ChatRole, LlmClient, LlmError, LlmRequest, SystemBlock};
use serde_json::json;
use url::Url;
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const TEST_KEY: &str = "sk-local-compat-test";

fn req_with_two_system_blocks(user: &str) -> LlmRequest {
    LlmRequest {
        system_blocks: vec![
            SystemBlock {
                text: "SYS-ALPHA".into(),
                cache: true, // ignored by OpenAI-compat branch
            },
            SystemBlock {
                text: "SYS-BETA".into(),
                cache: false,
            },
        ],
        messages: vec![ChatMessage {
            role: ChatRole::User,
            content: user.into(),
        }],
        model: "llama-test".into(),
        max_tokens: 64,
        temperature: Some(0.3),
    }
}

fn local_client(server: &MockServer) -> ProductionLlmClient {
    // wiremock always binds on 127.0.0.1, which is allowed by the SSRF
    // guard. Join a trailing slash so `base_url.join("chat/completions")`
    // stays under the base path.
    let base_url: Url = format!("{}/v1/", server.uri()).parse().unwrap();
    let cfg = ProductionLlmConfig::new(
        ProviderKind::OpenAICompat { base_url },
        SecretString::new(TEST_KEY),
    );
    ProductionLlmClient::new(cfg).expect("local base_url passes SSRF guard")
}

#[tokio::test]
async fn happy_path_concatenates_system_blocks_and_zeros_cache_fields() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .and(header("authorization", format!("Bearer {TEST_KEY}").as_str()))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "choices": [{
                "message": {"role": "assistant", "content": "local-hello"},
            }],
            "usage": {
                "prompt_tokens": 33,
                "completion_tokens": 17,
            },
        })))
        .mount(&server)
        .await;

    let client = local_client(&server);
    let resp = client.complete(req_with_two_system_blocks("hi")).await.unwrap();

    assert_eq!(resp.text, "local-hello");
    assert_eq!(resp.input_tokens, 33);
    assert_eq!(resp.output_tokens, 17);
    assert_eq!(resp.cache_read_input_tokens, 0);
    assert_eq!(resp.cache_creation_input_tokens, 0);

    // Inspect the outgoing body to prove the system blocks collapsed
    // into one `system` message.
    let recv = server.received_requests().await.unwrap();
    assert_eq!(recv.len(), 1);
    let body: serde_json::Value = serde_json::from_slice(&recv[0].body).unwrap();
    let messages = body.get("messages").and_then(|v| v.as_array()).unwrap();
    assert!(!messages.is_empty(), "expected at least the system message");
    assert_eq!(messages[0]["role"], "system");
    let concat = messages[0]["content"].as_str().unwrap();
    assert!(concat.contains("SYS-ALPHA"), "got: {concat}");
    assert!(concat.contains("SYS-BETA"), "got: {concat}");
    // User message follows.
    assert_eq!(messages[1]["role"], "user");
    assert_eq!(messages[1]["content"], "hi");
    // OpenAI-compat payload must not carry an Anthropic `system` top-level array.
    assert!(body.get("system").is_none());
}

#[tokio::test]
async fn ssrf_guard_rejects_non_local_base_url_at_construction() {
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
            assert!(msg.contains("127.0.0.1"));
            assert!(msg.contains("::1"));
        }
        other => panic!("expected Transport, got {other:?}"),
    }
}

#[tokio::test]
async fn ssrf_guard_accepts_ipv6_loopback() {
    // The actual reachability is not what's under test — the SSRF
    // guard is. Accept at construction is enough.
    let cfg = ProductionLlmConfig::new(
        ProviderKind::OpenAICompat {
            base_url: Url::parse("http://[::1]:11434/v1/").unwrap(),
        },
        SecretString::new(TEST_KEY),
    );
    ProductionLlmClient::new(cfg).expect("::1 is allowed");
}

#[tokio::test]
async fn budget_is_enforced_on_openai_compat_before_any_http() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(500))
        .expect(0)
        .mount(&server)
        .await;

    let base_url: Url = format!("{}/v1/", server.uri()).parse().unwrap();
    let cfg = ProductionLlmConfig::new(
        ProviderKind::OpenAICompat { base_url },
        SecretString::new(TEST_KEY),
    )
    .with_budget_tokens(20);
    let client = ProductionLlmClient::new(cfg).unwrap();

    let mut req = req_with_two_system_blocks("q");
    req.max_tokens = 200;

    let err = client.complete(req).await.unwrap_err();
    match err {
        LlmError::BudgetExceeded {
            requested,
            remaining,
        } => {
            assert_eq!(requested, 200);
            assert_eq!(remaining, 20);
        }
        other => panic!("expected BudgetExceeded, got {other:?}"),
    }
}
