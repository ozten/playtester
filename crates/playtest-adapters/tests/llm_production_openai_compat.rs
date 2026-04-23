//! Integration tests for `ProductionLlmClient` with an
//! OpenAI-compatible provider (Ollama / llama.cpp shape), backed by a
//! Pact mock server.
//!
//! Pact binds the mock on `127.0.0.1`, which passes the SSRF guard.
//! Covered scenarios:
//!
//! - Happy path: both `system_blocks` are concatenated into one
//!   `system` message in the outgoing payload; `cache_*` fields on the
//!   response are zeroed.
//! - SSRF guard: `base_url` with a public host is rejected at `new()`.
//! - Budget pre-check still fires before any HTTP traffic on this
//!   provider.

use pact_consumer::prelude::*;
use playtest_adapters::{
    ProductionLlmClient, ProductionLlmConfig, ProviderKind, SecretString,
};
use playtest_ports::{ChatMessage, ChatRole, LlmClient, LlmError, LlmRequest, SystemBlock};
use serde_json::json;
use url::Url;

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

fn local_client(mock_url: &Url) -> ProductionLlmClient {
    // Pact's mock server binds on 127.0.0.1, which the SSRF guard
    // accepts. Re-anchor under a `/v1/` prefix so the adapter's
    // `base_url.join("chat/completions")` lands on `/v1/chat/completions`.
    let base_url: Url = format!("{}v1/", mock_url.as_str()).parse().unwrap();
    let cfg = ProductionLlmConfig::new(
        ProviderKind::OpenAICompat { base_url },
        SecretString::new(TEST_KEY),
    );
    ProductionLlmClient::new(cfg).expect("local base_url passes SSRF guard")
}

#[tokio::test]
async fn happy_path_concatenates_system_blocks_and_zeros_cache_fields() {
    // Encode the outgoing body invariant in the pact request matcher:
    // the two system blocks collapse into a single `system` message
    // joined with "\n\n", the `user` message follows, and no
    // Anthropic-style top-level `system` field appears.
    let pact = PactBuilder::new("playtest-adapters", "openai-compat-api")
        .interaction(
            "POST /v1/chat/completions with concatenated system blocks",
            "",
            |mut i| {
                i.request
                    .method("POST")
                    .path("/v1/chat/completions")
                    .header("authorization", format!("Bearer {TEST_KEY}"))
                    .json_body(json_pattern!({
                        "model": "llama-test",
                        "max_tokens": 64,
                        "messages": [
                            {"role": "system", "content": "SYS-ALPHA\n\nSYS-BETA"},
                            {"role": "user", "content": "hi"},
                        ],
                        // `temperature` uses `like!` to tolerate the
                        // f32→f64 decimal widening; we only care that
                        // the field is a number.
                        "temperature": like!(0.3),
                    }));
                i.response.status(200).json_body(json!({
                    "choices": [{
                        "message": {"role": "assistant", "content": "local-hello"},
                    }],
                    "usage": {
                        "prompt_tokens": 33,
                        "completion_tokens": 17,
                    },
                }));
                i
            },
        )
        .start_mock_server(None, None);

    let client = local_client(&pact.url());
    let resp = client
        .complete(req_with_two_system_blocks("hi"))
        .await
        .unwrap();

    assert_eq!(resp.text, "local-hello");
    assert_eq!(resp.input_tokens, 33);
    assert_eq!(resp.output_tokens, 17);
    assert_eq!(resp.cache_read_input_tokens, 0);
    assert_eq!(resp.cache_creation_input_tokens, 0);
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
    // Zero interactions — pact's Drop panics the test if the adapter
    // fires an unexpected HTTP call.
    let pact = PactBuilder::new("playtest-adapters", "openai-compat-api").start_mock_server(None, None);

    let base_url: Url = format!("{}v1/", pact.url().as_str()).parse().unwrap();
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
