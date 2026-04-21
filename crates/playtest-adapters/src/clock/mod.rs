//! `Clock` port adapters: stub, production, record, playback.

pub mod playback;
pub mod production;
pub mod record;
pub mod stub;

pub use playback::PlaybackClock;
pub use production::ProductionClock;
pub use record::RecordClock;
pub use stub::StubClock;

/// Name tag used in tape headers and call names. All `Clock` tapes carry
/// this value so `Playback<Clock>` can reject a tape meant for another port.
pub(crate) const PORT_TAG: &str = "clock";
pub(crate) const CALL_NOW: &str = "now";
