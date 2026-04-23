//! Production `LlmClient`: Anthropic + OpenAI-compatible provider.
//!
//! Single adapter, two provider shapes selected by [`ProviderKind`]:
//!
//! - `Anthropic` — POSTs to `https://api.anthropic.com/v1/messages`,
//!   emits `cache_control: { type: "ephemeral" }` on `SystemBlock`s whose
//!   `cache` field is `true`, and decodes the full four-field token
//!   accounting including `cache_read_input_tokens` and
//!   `cache_creation_input_tokens`. HTTP 429 is retried exactly once
//!   after a fixed 500 ms sleep.
//! - `OpenAICompat { base_url }` — POSTs to `{base_url}/chat/completions`;
//!   all `SystemBlock`s are concatenated into a single `system` message
//!   (the `cache` flag is silently ignored — Ollama / llama.cpp do not
//!   surface cache semantics). `cache_*_input_tokens` default to 0.
//!   `base_url` is SSRF-guarded at construction: only `localhost`,
//!   `127.0.0.1`, and `::1` are allowed.
//!
//! The adapter enforces a per-client token budget via an atomic counter.
//! The pre-check runs *before* the HTTP request so an over-budget call
//! never touches the network. On success, `input_tokens + output_tokens`
//! are decremented from the remaining budget.
//!
//! API keys live in a local [`SecretString`] with a redacted `Debug`
//! impl. Transport errors are scanned for the key substring and for
//! common credential-bearing header names; any match replaces the whole
//! message with a neutral sanitized string.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use playtest_ports::{
    ChatMessage, ChatRole, LlmClient, LlmError, LlmRequest, LlmResponse, SystemBlock,
};
use serde_json::{Value, json};
use url::Url;

const ANTHROPIC_ENDPOINT: &str = "https://api.anthropic.com/v1/messages";
const ANTHROPIC_VERSION: &str = "2023-06-01";
const DEFAULT_TIMEOUT_MS: u64 = 60_000;
const RETRY_SLEEP_MS: u64 = 500;
const SANITIZED_TRANSPORT_MSG: &str =
    "transport error: sanitized (response contained credentials)";

/// A credential string whose `Debug` impl never reveals the inner value.
///
/// This is a local stand-in for the `secrecy` crate — we need precisely
/// two things (redacted `Debug` and a single `expose` accessor), so the
/// whole-crate dependency is unjustified.
#[derive(Clone)]
pub struct SecretString(String);

impl SecretString {
    #[must_use]
    pub fn new(s: impl Into<String>) -> Self {
        Self(s.into())
    }

    /// Read `var_name` from the process environment. An unset variable
    /// and an empty-string variable collapse to the same "not configured"
    /// signal: the caller gets an empty `SecretString`, which
    /// [`ProductionLlmClient::new`] rejects at construction.
    #[must_use]
    pub fn from_env(var_name: &str) -> Self {
        Self(std::env::var(var_name).unwrap_or_default())
    }

    #[must_use]
    pub fn expose(&self) -> &str {
        &self.0
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl std::fmt::Debug for SecretString {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("<redacted>")
    }
}

/// Which provider shape to speak.
#[derive(Debug, Clone)]
pub enum ProviderKind {
    Anthropic,
    OpenAICompat { base_url: Url },
}

/// Adapter-construction configuration.
#[derive(Debug, Clone)]
pub struct ProductionLlmConfig {
    pub provider: ProviderKind,
    pub api_key: SecretString,
    /// `None` is unbounded (represented internally as `u64::MAX`).
    pub budget_tokens: Option<u64>,
    /// Per-request HTTP timeout.
    pub request_timeout_ms: u64,
}

impl ProductionLlmConfig {
    #[must_use]
    pub fn new(provider: ProviderKind, api_key: SecretString) -> Self {
        Self {
            provider,
            api_key,
            budget_tokens: None,
            request_timeout_ms: DEFAULT_TIMEOUT_MS,
        }
    }

    #[must_use]
    pub fn with_budget_tokens(mut self, tokens: u64) -> Self {
        self.budget_tokens = Some(tokens);
        self
    }

    #[must_use]
    pub fn with_timeout_ms(mut self, ms: u64) -> Self {
        self.request_timeout_ms = ms;
        self
    }
}

/// Production [`LlmClient`] adapter.
///
/// Two shapes:
///
/// - [`ProductionLlmClient::new`]`(cfg)` — fully configured; makes real
///   HTTP calls against Anthropic or an OpenAI-compatible endpoint.
/// - [`ProductionLlmClient::not_configured`]`()` — placeholder that
///   returns [`LlmError::NotConfigured`] from every `complete` call.
///   Used by the adapter-quartet plumbing when no LLM is wired.
pub struct ProductionLlmClient {
    cfg: Option<ProductionLlmConfig>,
    http: reqwest::Client,
    remaining_budget: AtomicU64,
    /// Override for the Anthropic endpoint. `None` → the real
    /// `https://api.anthropic.com/v1/messages`. Used by integration
    /// tests that point the adapter at a local pact mock server;
    /// production callers leave it `None`.
    anthropic_endpoint_override: Option<String>,
}

impl std::fmt::Debug for ProductionLlmClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProductionLlmClient")
            .field("cfg", &self.cfg)
            .field(
                "remaining_budget",
                &self.remaining_budget.load(Ordering::Relaxed),
            )
            .finish_non_exhaustive()
    }
}

impl Default for ProductionLlmClient {
    fn default() -> Self {
        Self::not_configured()
    }
}

impl ProductionLlmClient {
    /// Construct a fully-configured production client.
    ///
    /// Validates:
    /// - API key is non-empty (otherwise [`LlmError::NotConfigured`]).
    /// - If the provider is `OpenAICompat`, the `base_url` host is one
    ///   of `localhost`, `127.0.0.1`, `::1` (SSRF guard).
    pub fn new(cfg: ProductionLlmConfig) -> Result<Self, LlmError> {
        if cfg.api_key.is_empty() {
            return Err(LlmError::NotConfigured);
        }
        if let ProviderKind::OpenAICompat { base_url } = &cfg.provider {
            validate_local_base_url(base_url)?;
        }

        let http = reqwest::Client::builder()
            .timeout(Duration::from_millis(cfg.request_timeout_ms))
            .build()
            .map_err(|e| LlmError::Transport(format!("build http client: {e}")))?;

        let remaining = cfg.budget_tokens.unwrap_or(u64::MAX);
        Ok(Self {
            cfg: Some(cfg),
            http,
            remaining_budget: AtomicU64::new(remaining),
            anthropic_endpoint_override: None,
        })
    }

    /// Construct a client that returns [`LlmError::NotConfigured`] for
    /// every `complete` call. Preserves the adapter quartet's "always
    /// constructible" property for callers that haven't wired a real
    /// provider yet (notably the record/playback plumbing tests).
    #[must_use]
    pub fn not_configured() -> Self {
        Self {
            cfg: None,
            http: reqwest::Client::new(),
            remaining_budget: AtomicU64::new(u64::MAX),
            anthropic_endpoint_override: None,
        }
    }

    /// Redirect the Anthropic branch's HTTP POST at a caller-chosen
    /// base URL instead of the real `api.anthropic.com`.
    ///
    /// The adapter will POST to `{override_base}/v1/messages`, preserving
    /// the request shape, headers, and retry policy. This exists solely
    /// for integration tests that stand up a local mock HTTP server;
    /// production callers never set it.
    ///
    /// Marked `#[doc(hidden)]` because it is not part of the stable
    /// adapter API.
    #[doc(hidden)]
    #[must_use]
    pub fn with_anthropic_endpoint_override(mut self, override_base: impl Into<String>) -> Self {
        self.anthropic_endpoint_override = Some(override_base.into());
        self
    }

    /// Observe the remaining budget. For tests and diagnostics.
    #[must_use]
    pub fn remaining_budget(&self) -> u64 {
        self.remaining_budget.load(Ordering::Relaxed)
    }
}

fn validate_local_base_url(base_url: &Url) -> Result<(), LlmError> {
    let host = base_url.host_str().unwrap_or("");
    // `Url::host_str` strips brackets from IPv6 literals, so `[::1]` in
    // the input arrives here as `::1`. Accept both spellings to be
    // explicit.
    if matches!(host, "localhost" | "127.0.0.1" | "::1" | "[::1]") {
        Ok(())
    } else {
        Err(LlmError::Transport(format!(
            "base_url host '{host}' not allowed; only localhost/127.0.0.1/::1 are permitted (SSRF guard)"
        )))
    }
}

/// Serialize `messages` to the OpenAI/Anthropic chat-array shape.
///
/// Anthropic accepts only `user` and `assistant` roles at the messages
/// level; a stray `System` role in the array is coerced to `user` (the
/// port contract places system prompts in `system_blocks`, so this
/// should not happen in practice — the coercion is belt-and-suspenders).
fn serialize_messages(messages: &[ChatMessage]) -> Vec<Value> {
    messages
        .iter()
        .map(|m| {
            let role = match m.role {
                ChatRole::Assistant => "assistant",
                // System messages shouldn't appear in the array path —
                // collapse to `user` to avoid a 400 from Anthropic.
                ChatRole::User | ChatRole::System => "user",
            };
            json!({ "role": role, "content": m.content })
        })
        .collect()
}

fn serialize_anthropic_system_blocks(blocks: &[SystemBlock]) -> Vec<Value> {
    blocks
        .iter()
        .map(|b| {
            if b.cache {
                json!({
                    "type": "text",
                    "text": b.text,
                    "cache_control": { "type": "ephemeral" },
                })
            } else {
                json!({ "type": "text", "text": b.text })
            }
        })
        .collect()
}

fn concat_system_blocks(blocks: &[SystemBlock]) -> String {
    let mut s = String::new();
    for (i, b) in blocks.iter().enumerate() {
        if i > 0 {
            s.push_str("\n\n");
        }
        s.push_str(&b.text);
    }
    s
}

/// Build the Anthropic `v1/messages` request body.
fn build_anthropic_body(req: &LlmRequest) -> Value {
    let mut body = json!({
        "model": req.model,
        "max_tokens": req.max_tokens,
        "system": serialize_anthropic_system_blocks(&req.system_blocks),
        "messages": serialize_messages(&req.messages),
    });
    if let Some(t) = req.temperature {
        body.as_object_mut()
            .expect("just built a json object")
            .insert("temperature".into(), json!(t));
    }
    body
}

/// Build the OpenAI-compat `/chat/completions` body.
fn build_openai_compat_body(req: &LlmRequest) -> Value {
    let sys = concat_system_blocks(&req.system_blocks);
    let mut messages: Vec<Value> = Vec::with_capacity(req.messages.len() + 1);
    if !sys.is_empty() {
        messages.push(json!({ "role": "system", "content": sys }));
    }
    messages.extend(serialize_messages(&req.messages));
    let mut body = json!({
        "model": req.model,
        "max_tokens": req.max_tokens,
        "messages": messages,
    });
    if let Some(t) = req.temperature {
        body.as_object_mut()
            .expect("just built a json object")
            .insert("temperature".into(), json!(t));
    }
    body
}

fn parse_anthropic_response(v: &Value) -> Result<LlmResponse, LlmError> {
    let text = v
        .get("content")
        .and_then(Value::as_array)
        .and_then(|a| a.first())
        .and_then(|b| b.get("text"))
        .and_then(Value::as_str)
        .ok_or_else(|| {
            LlmError::Transport("malformed response: missing content[0].text".into())
        })?
        .to_owned();

    let usage = v
        .get("usage")
        .ok_or_else(|| LlmError::Transport("malformed response: missing usage".into()))?;
    let input_tokens = u32_field(usage, "input_tokens").unwrap_or(0);
    let output_tokens = u32_field(usage, "output_tokens").unwrap_or(0);
    let cache_read_input_tokens = u32_field(usage, "cache_read_input_tokens").unwrap_or(0);
    let cache_creation_input_tokens =
        u32_field(usage, "cache_creation_input_tokens").unwrap_or(0);

    Ok(LlmResponse {
        text,
        input_tokens,
        output_tokens,
        cache_read_input_tokens,
        cache_creation_input_tokens,
    })
}

fn parse_openai_compat_response(v: &Value) -> Result<LlmResponse, LlmError> {
    let text = v
        .get("choices")
        .and_then(Value::as_array)
        .and_then(|a| a.first())
        .and_then(|c| c.get("message"))
        .and_then(|m| m.get("content"))
        .and_then(Value::as_str)
        .ok_or_else(|| {
            LlmError::Transport(
                "malformed response: missing choices[0].message.content".into(),
            )
        })?
        .to_owned();

    let usage = v
        .get("usage")
        .ok_or_else(|| LlmError::Transport("malformed response: missing usage".into()))?;
    let input_tokens = u32_field(usage, "prompt_tokens").unwrap_or(0);
    let output_tokens = u32_field(usage, "completion_tokens").unwrap_or(0);

    Ok(LlmResponse {
        text,
        input_tokens,
        output_tokens,
        cache_read_input_tokens: 0,
        cache_creation_input_tokens: 0,
    })
}

fn u32_field(v: &Value, key: &str) -> Option<u32> {
    v.get(key).and_then(Value::as_u64).and_then(|n| {
        u32::try_from(n).ok()
    })
}

/// Scan `raw` for secrets and, if any are found, replace the whole
/// message with a neutral sanitized string.
fn sanitize_transport_error(raw: &str, api_key: &str) -> String {
    if !api_key.is_empty() && raw.contains(api_key) {
        return SANITIZED_TRANSPORT_MSG.to_owned();
    }
    let lower = raw.to_ascii_lowercase();
    if lower.contains("authorization:") || lower.contains("x-api-key") {
        return SANITIZED_TRANSPORT_MSG.to_owned();
    }
    raw.to_owned()
}

#[async_trait]
impl LlmClient for ProductionLlmClient {
    async fn complete(&self, req: LlmRequest) -> Result<LlmResponse, LlmError> {
        let Some(cfg) = self.cfg.as_ref() else {
            return Err(LlmError::NotConfigured);
        };

        // 1. Budget pre-check — before any HTTP traffic.
        let requested = u64::from(req.max_tokens);
        let remaining = self.remaining_budget.load(Ordering::Acquire);
        if requested > remaining {
            return Err(LlmError::BudgetExceeded {
                requested,
                remaining,
            });
        }

        // 2. Dispatch on provider.
        let response = match &cfg.provider {
            ProviderKind::Anthropic => self.call_anthropic(cfg, &req).await?,
            ProviderKind::OpenAICompat { base_url } => {
                self.call_openai_compat(cfg, base_url, &req).await?
            }
        };

        // 3. Decrement the budget using actual usage.
        let used = u64::from(response.input_tokens)
            .saturating_add(u64::from(response.output_tokens));
        if used > 0 {
            let _ = self.remaining_budget.fetch_update(
                Ordering::AcqRel,
                Ordering::Acquire,
                |cur| Some(cur.saturating_sub(used)),
            );
        }

        Ok(response)
    }
}

impl ProductionLlmClient {
    async fn call_anthropic(
        &self,
        cfg: &ProductionLlmConfig,
        req: &LlmRequest,
    ) -> Result<LlmResponse, LlmError> {
        let body = build_anthropic_body(req);
        let api_key = cfg.api_key.expose();

        // Integration tests can redirect this branch at a local mock
        // server via `with_anthropic_endpoint_override`.
        let endpoint: String = if let Some(base) = self.anthropic_endpoint_override.as_deref() {
            let base = base.trim_end_matches('/');
            format!("{base}/v1/messages")
        } else {
            ANTHROPIC_ENDPOINT.to_owned()
        };

        // One retry on 429, fixed 500 ms sleep.
        let mut attempt = 0u8;
        loop {
            let send_result = self
                .http
                .post(&endpoint)
                .header("x-api-key", api_key)
                .header("anthropic-version", ANTHROPIC_VERSION)
                .header("content-type", "application/json")
                .json(&body)
                .send()
                .await;

            let resp = match send_result {
                Ok(r) => r,
                Err(e) => {
                    let msg = sanitize_transport_error(&e.to_string(), api_key);
                    return Err(LlmError::Transport(msg));
                }
            };

            let status = resp.status();
            if status.is_success() {
                let value: Value = resp.json().await.map_err(|e| {
                    LlmError::Transport(sanitize_transport_error(
                        &format!("decode response: {e}"),
                        api_key,
                    ))
                })?;
                return parse_anthropic_response(&value);
            }

            if status.as_u16() == 429 && attempt == 0 {
                attempt = 1;
                tokio::time::sleep(Duration::from_millis(RETRY_SLEEP_MS)).await;
                continue;
            }

            let body_text = resp.text().await.unwrap_or_default();
            let raw = format!("anthropic {status}: {body_text}");
            return Err(LlmError::Transport(sanitize_transport_error(
                &raw, api_key,
            )));
        }
    }

    async fn call_openai_compat(
        &self,
        cfg: &ProductionLlmConfig,
        base_url: &Url,
        req: &LlmRequest,
    ) -> Result<LlmResponse, LlmError> {
        let endpoint = base_url
            .join("chat/completions")
            .map_err(|e| LlmError::Transport(format!("bad base_url join: {e}")))?;
        let body = build_openai_compat_body(req);
        let api_key = cfg.api_key.expose();

        let send_result = self
            .http
            .post(endpoint)
            .bearer_auth(api_key)
            .header("content-type", "application/json")
            .json(&body)
            .send()
            .await;

        let resp = match send_result {
            Ok(r) => r,
            Err(e) => {
                let msg = sanitize_transport_error(&e.to_string(), api_key);
                return Err(LlmError::Transport(msg));
            }
        };

        let status = resp.status();
        if status.is_success() {
            let value: Value = resp.json().await.map_err(|e| {
                LlmError::Transport(sanitize_transport_error(
                    &format!("decode response: {e}"),
                    api_key,
                ))
            })?;
            return parse_openai_compat_response(&value);
        }

        let body_text = resp.text().await.unwrap_or_default();
        let raw = format!("openai-compat {status}: {body_text}");
        Err(LlmError::Transport(sanitize_transport_error(
            &raw, api_key,
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use playtest_ports::{ChatMessage, ChatRole};

    fn req(user: &str) -> LlmRequest {
        LlmRequest {
            system_blocks: vec![],
            messages: vec![ChatMessage {
                role: ChatRole::User,
                content: user.into(),
            }],
            model: "claude-test".into(),
            max_tokens: 16,
            temperature: None,
        }
    }

    #[tokio::test]
    async fn not_configured_placeholder_returns_not_configured() {
        let c = ProductionLlmClient::not_configured();
        let err = c.complete(req("hi")).await.unwrap_err();
        assert!(matches!(err, LlmError::NotConfigured));
    }

    #[tokio::test]
    async fn empty_api_key_is_rejected_at_construction() {
        let cfg = ProductionLlmConfig::new(ProviderKind::Anthropic, SecretString::new(""));
        let err = ProductionLlmClient::new(cfg).unwrap_err();
        assert!(matches!(err, LlmError::NotConfigured));
    }

    #[test]
    fn secret_string_debug_is_redacted() {
        let s = SecretString::new("super-secret-token");
        let dbg = format!("{s:?}");
        assert!(!dbg.contains("super-secret-token"));
        assert!(dbg.contains("redacted"));
    }

    #[test]
    fn config_debug_does_not_leak_api_key() {
        let cfg = ProductionLlmConfig::new(
            ProviderKind::Anthropic,
            SecretString::new("sk-do-not-leak-me-123"),
        );
        let dbg = format!("{cfg:?}");
        assert!(!dbg.contains("sk-do-not-leak-me-123"));
    }

    #[test]
    fn sanitize_blanks_out_messages_with_the_key_substring() {
        let key = "sk-ant-SHHH";
        let raw = format!("error 400: {{\"echo\":\"{key} was here\"}}");
        let out = sanitize_transport_error(&raw, key);
        assert!(!out.contains(key));
        assert_eq!(out, SANITIZED_TRANSPORT_MSG);
    }

    #[test]
    fn sanitize_blanks_out_authorization_headers() {
        let out = sanitize_transport_error("Authorization: Bearer xyz", "unrelated");
        assert_eq!(out, SANITIZED_TRANSPORT_MSG);
    }

    #[test]
    fn sanitize_blanks_out_x_api_key_header() {
        let out = sanitize_transport_error("x-api-key: abc", "unrelated");
        assert_eq!(out, SANITIZED_TRANSPORT_MSG);
    }

    #[test]
    fn sanitize_keeps_clean_messages_intact() {
        let out = sanitize_transport_error("connection refused", "sk-not-in-msg");
        assert_eq!(out, "connection refused");
    }

    #[test]
    fn anthropic_system_block_serialization_marks_cached_blocks_only() {
        let blocks = vec![
            SystemBlock {
                text: "rules".into(),
                cache: true,
            },
            SystemBlock {
                text: "turn".into(),
                cache: false,
            },
        ];
        let v = serialize_anthropic_system_blocks(&blocks);
        assert_eq!(v[0]["cache_control"]["type"], "ephemeral");
        assert!(v[1].get("cache_control").is_none());
    }

    #[test]
    fn openai_compat_concatenation_joins_blocks_with_blank_line() {
        let blocks = vec![
            SystemBlock {
                text: "a".into(),
                cache: true,
            },
            SystemBlock {
                text: "b".into(),
                cache: false,
            },
        ];
        assert_eq!(concat_system_blocks(&blocks), "a\n\nb");
    }

    #[test]
    fn ssrf_guard_accepts_local_hosts() {
        for host in ["http://localhost:11434/v1/", "http://127.0.0.1:8080/v1/", "http://[::1]:8080/v1/"] {
            let u: Url = host.parse().unwrap();
            assert!(validate_local_base_url(&u).is_ok(), "should accept {host}");
        }
    }

    #[test]
    fn ssrf_guard_rejects_remote_hosts() {
        let u: Url = "http://example.com/v1/".parse().unwrap();
        let err = validate_local_base_url(&u).unwrap_err();
        match err {
            LlmError::Transport(msg) => {
                assert!(msg.contains("example.com"));
                assert!(msg.contains("localhost"));
                assert!(msg.contains("127.0.0.1"));
            }
            other => panic!("expected Transport, got {other:?}"),
        }
    }
}
