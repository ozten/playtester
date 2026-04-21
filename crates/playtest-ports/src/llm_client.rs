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

use async_trait::async_trait;

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

/// A single LLM request. Shape is intentionally minimal; Phase 3 will
/// extend it with prompt caching, model selection, and tool use.
#[derive(Debug, Clone)]
pub struct LlmRequest {
    pub system: Option<String>,
    pub user: String,
    pub max_tokens: u32,
}

/// A single LLM response. Fields are the subset we know we will need for
/// post-game critique (Phase 5) and cost observability (Phase 3).
#[derive(Debug, Clone)]
pub struct LlmResponse {
    pub text: String,
    pub input_tokens: u32,
    pub output_tokens: u32,
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
