//! `GameEventSink` port adapters.
//!
//! Asymmetric with the input ports: the game event log *is* the authoritative
//! history, so `record` aliases `production` and `playback` is a no-op that
//! errors on use. See the three-categories-of-recording doc in
//! [`playtest_ports`] for why.

pub mod broadcast;
pub mod playback;
pub mod production;
pub mod stub;

pub use broadcast::BroadcastGameEventSink;
pub use playback::PlaybackGameEventSink;
pub use production::ProductionGameEventSink;
pub use stub::StubGameEventSink;

/// `Record<GameEventSink>` is a type alias for the production sink, not a
/// separate adapter. The asymmetry is the whole point of the "input vs.
/// output ports" decision documented in the plan and `playtest-ports`.
pub type RecordGameEventSink<F> = ProductionGameEventSink<F>;
