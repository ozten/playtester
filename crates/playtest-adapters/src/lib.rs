//! Adapter implementations for every port in `playtest-ports`.
//!
//! Four variants per port:
//! - `stub` — deterministic, hardcoded behavior for unit tests
//! - `production` — real implementation (`std::fs`, `ChaCha20Rng`, `SystemTime`, etc.)
//! - `record` — wraps another adapter, tees inputs/outputs to a tape file
//! - `playback` — reads a tape file and replays stored outputs
//!
//! The [`recording`] module provides the shared JSONL tape format used by
//! every record/playback pair.

pub mod clock;
pub mod filesystem;
pub mod game_event_sink;
pub mod llm_client;
pub mod recording;
pub mod rng;

pub use clock::{PlaybackClock, ProductionClock, RecordClock, StubClock};
pub use filesystem::{PlaybackFileSystem, ProductionFileSystem, RecordFileSystem, StubFileSystem};
pub use game_event_sink::{
    PlaybackGameEventSink, ProductionGameEventSink, RecordGameEventSink, StubGameEventSink,
};
pub use llm_client::{PlaybackLlmClient, ProductionLlmClient, RecordLlmClient, StubLlmClient};
pub use recording::{SCHEMA_VERSION, TapeError, TapeReader, TapeWriter};
pub use rng::{PlaybackRng, ProductionRng, RecordRng, StubRng};
