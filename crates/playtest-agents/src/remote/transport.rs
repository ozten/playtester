//! `RemoteAgentTransport`: the port [`HttpRemoteAgent`](super::http_remote::HttpRemoteAgent)
//! talks through to reach an external decision-maker (a browser tab today;
//! Phase 3 adds a stdio-subprocess sibling).
//!
//! The trait lives in `playtest-agents` (not `playtest-ports`) because the
//! four-variant stub/production/record/playback discipline does not apply
//! here — browser input is non-deterministic by definition, and replay at
//! the action level is already covered by the JSONL event log (the chosen
//! action becomes a normal game event). See the Phase 2.5 plan's
//! "Key Technical Decisions" for the full rationale.
//!
//! The production implementation is provided by the server (see
//! `playtest_server::TurnCoordinator`). Unit tests in this crate use a
//! simple in-memory stub.
//!
//! ## Shape
//!
//! - `issue_prompt(seat, legal_json) -> prompt_id` publishes a turn prompt
//!   and returns a per-game monotonic id the caller echoes on submission.
//! - `await_action(seat, prompt_id) -> action_index` blocks the agent's
//!   async task until the submission arrives (or the transport cancels).
//!
//! Both methods are async so the production impl can broadcast on a tokio
//! channel and await a oneshot/mpsc without the agent needing to know.

use async_trait::async_trait;
use serde_json::Value as JsonValue;

/// Transport that routes prompts out to an external decision-maker and
/// action submissions back in.
#[async_trait]
pub trait RemoteAgentTransport: Send + Sync {
    /// Publish a turn prompt for `seat` carrying the serialized legal
    /// actions. Returns the per-game monotonic `prompt_id` assigned to
    /// this prompt; clients echo it on the corresponding submission.
    ///
    /// # Errors
    /// Returns [`RemoteTransportError::Other`] if the transport's publish
    /// path fails (e.g., the broadcaster is gone).
    async fn issue_prompt(
        &self,
        seat: u8,
        legal_json: Vec<JsonValue>,
    ) -> Result<u64, RemoteTransportError>;

    /// Block until an action index is submitted for `(seat, prompt_id)`.
    ///
    /// # Errors
    /// Returns [`RemoteTransportError::Cancelled`] if the transport shuts
    /// down before a submission arrives (typical at game end).
    async fn await_action(
        &self,
        seat: u8,
        prompt_id: u64,
    ) -> Result<usize, RemoteTransportError>;
}

/// Failure modes for a remote-agent transport call.
#[derive(Debug, thiserror::Error)]
pub enum RemoteTransportError {
    /// Transport was closed before the expected submission arrived.
    /// Normal at game end; upstream maps this to `AgentError::Other` with
    /// an informative message so the run supervisor records a failure.
    #[error("remote agent transport cancelled (game ended before submission)")]
    Cancelled,

    /// Any other transport failure. Carries a human-readable explanation.
    #[error("remote agent transport error: {0}")]
    Other(String),
}
