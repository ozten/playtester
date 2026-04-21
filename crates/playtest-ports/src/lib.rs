//! Port traits: every external-system interaction crosses one of these.
//!
//! This crate defines the contracts. Implementations (stub, production,
//! record, playback) live in `playtest-adapters`.
//!
//! All ports here are object-safe (usable via `&mut dyn Port`) *except*
//! [`LlmClient`], which is the one place we accept the extra coupling of
//! `async_trait` so asynchronous LLM calls can live behind the same
//! record/playback discipline as every other external system.

pub mod clock;
pub mod event_sink;
pub mod filesystem;
pub mod llm_client;
pub mod rng;

pub use clock::{Clock, UnixMillis};
pub use event_sink::{EventSink, EventSinkError};
pub use filesystem::{FileSystem, FsError};
pub use llm_client::{LlmClient, LlmError, LlmRequest, LlmResponse};
pub use rng::{Rng, RngError};
