//! `LlmClient` port adapters: stub, production, record, playback.
//!
//! Production returns [`LlmError::NotConfigured`](playtest_ports::LlmError)
//! until Phase 3 lands a real provider. Record/playback are wired so that
//! Phase-0 and Phase-1 tests can still exercise the determinism plumbing
//! end-to-end.

pub mod playback;
pub mod production;
pub mod record;
pub mod stub;

pub use playback::PlaybackLlmClient;
pub use production::ProductionLlmClient;
pub use record::RecordLlmClient;
pub use stub::StubLlmClient;

pub(crate) const PORT_TAG: &str = "llm_client";
pub(crate) const CALL_COMPLETE: &str = "complete";
