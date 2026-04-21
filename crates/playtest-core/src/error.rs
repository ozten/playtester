//! Errors surfaced by the engine and agents.

use playtest_ports::{GameEventSinkError, RngError};

use crate::actor::PlayerId;

/// Errors produced by an `Agent::choose` call.
///
/// Agents are async and may wrap arbitrary failure modes (network, LLM
/// timeouts, scripted-rule bugs). The loop attaches player context when
/// it surfaces these as [`GameError::AgentError`].
#[derive(Debug, thiserror::Error)]
pub enum AgentError {
    #[error("agent timed out waiting for user or network input")]
    Timeout,

    #[error("agent implementation returned an error: {0}")]
    Other(String),
}

/// Errors surfaced by the [`GameLoop`](crate::GameLoop).
#[derive(Debug, thiserror::Error)]
pub enum GameError {
    /// `apply_action` returned an error: the game rules rejected the
    /// action. Engine is authoritative — this is a bug in the agent or
    /// a misuse of the action-selection protocol.
    #[error("player {player} chose an illegal action: {message}")]
    IllegalAction { player: PlayerId, message: String },

    /// The agent returned an index past the end of the legal-actions
    /// slice. Protocol violation.
    #[error(
        "player {player} returned action index {chosen} but only {legal_count} legal actions were offered"
    )]
    AgentChoseOutOfBounds {
        player: PlayerId,
        chosen: usize,
        legal_count: usize,
    },

    /// `legal_actions` returned an empty slice for a non-chance actor.
    /// Either the game is genuinely stalemated or the Game impl is
    /// wrong — the loop can't tell which, so surface the raw fact.
    #[error("player {player} has no legal actions (stalemate?)")]
    NoLegalActions { player: PlayerId },

    /// The agent failed during its turn.
    #[error("agent for player {player} failed: {source}")]
    AgentFailed {
        player: PlayerId,
        #[source]
        source: AgentError,
    },

    /// `resolve_chance` failed.
    #[error("chance resolution failed: {message}")]
    ChanceFailed { message: String },

    /// The `Rng` port returned an error while resolving a chance event.
    #[error("rng port error during chance resolution: {source}")]
    RngFailed {
        #[source]
        source: RngError,
    },

    /// The `GameEventSink` port rejected a write.
    #[error("event sink rejected a write: {source}")]
    SinkFailed {
        #[source]
        source: GameEventSinkError,
    },

    /// Event serialization failed. This is a programmer error (the Game
    /// impl's `Event` type should always serialize); surface it rather
    /// than panic so the game loop's context stays attached.
    #[error("event serialization failed: {source}")]
    EventSerialization {
        #[source]
        source: serde_json::Error,
    },
}
