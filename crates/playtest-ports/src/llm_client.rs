//! LLM client port: asynchronous request/response for language-model calls.
//!
//! Defined here so the record/playback infrastructure covers LLM I/O
//! symmetrically with every other external system. No production adapter
//! is wired in Phase 0–1 — the roadmap's LLM integration is Phase 3.
//!
//! Unlike the other ports, `LlmClient` is not guaranteed object-safe: it
//! uses `async_trait` which is object-safe, but later migration to native
//! async-in-traits would drop that. Call sites should prefer generics
//! (`<L: LlmClient>`) over trait objects where possible.
//!
//! # Shape (Phase 3)
//!
//! [`LlmRequest`] and [`LlmResponse`] carry Anthropic prompt-caching
//! primitives:
//!
//! - `system_blocks: Vec<SystemBlock>` — each block carries a plain
//!   `cache: bool`; the production adapter emits
//!   `"cache_control": {"type": "ephemeral"}` on blocks where `cache` is
//!   `true` and omits it otherwise.
//! - `messages: Vec<ChatMessage>` — the chat turn sequence (role + text).
//! - Per-call token accounting on the response side includes
//!   `cache_read_input_tokens` and `cache_creation_input_tokens`. Providers
//!   that don't report these leave them at 0.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

/// Errors produced by the [`LlmClient`] port.
#[derive(Debug, thiserror::Error)]
pub enum LlmError {
    #[error("llm client not configured — production adapter lands in phase 3")]
    NotConfigured,

    #[error("token budget exceeded: requested {requested} > remaining {remaining}")]
    BudgetExceeded { requested: u64, remaining: u64 },

    #[error("transport error: {0}")]
    Transport(String),

    #[error("tape divergence: replay call did not match recorded request")]
    TapeDivergence,
}

/// Chat role for a single [`ChatMessage`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ChatRole {
    System,
    User,
    Assistant,
}

/// A single chat message.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: ChatRole,
    pub content: String,
}

/// A single block within the system prompt.
///
/// `cache` is a plain bool. `true` means the Anthropic production adapter
/// emits `"cache_control": {"type": "ephemeral"}` for this block; `false`
/// means omit `cache_control` entirely.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SystemBlock {
    pub text: String,
    pub cache: bool,
}

/// A single LLM request.
///
/// The shape carries Anthropic prompt-caching primitives directly
/// (`system_blocks`) so adapters can emit cache breakpoints without
/// reinterpreting flat prompt strings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LlmRequest {
    pub system_blocks: Vec<SystemBlock>,
    pub messages: Vec<ChatMessage>,
    pub model: String,
    pub max_tokens: u32,
    pub temperature: Option<f32>,
}

/// A single LLM response. Fields are the subset we know we will need for
/// post-game critique (Phase 5) and cost observability (Phase 3).
///
/// `cache_read_input_tokens` and `cache_creation_input_tokens` default to
/// 0 for providers that don't report them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LlmResponse {
    pub text: String,
    pub input_tokens: u32,
    pub output_tokens: u32,
    pub cache_read_input_tokens: u32,
    pub cache_creation_input_tokens: u32,
}

/// An asynchronous LLM client.
///
/// Adapter variants:
/// - `stub` — returns a canned response; used by Phase 0–1 tests that
///   exercise the LLM port's wiring without making real API calls.
/// - `production` — returns [`LlmError::NotConfigured`] until Phase 3.
/// - `record` — wraps an inner client, tees every request/response pair.
/// - `playback` — reads a tape; diverging calls return
///   [`LlmError::TapeDivergence`].
#[async_trait]
pub trait LlmClient: Send + Sync {
    async fn complete(&self, req: LlmRequest) -> Result<LlmResponse, LlmError>;
}
