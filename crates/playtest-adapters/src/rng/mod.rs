//! `Rng` port adapters: stub, production, record, playback.

pub mod playback;
pub mod production;
pub mod record;
pub mod stub;

pub use playback::PlaybackRng;
pub use production::ProductionRng;
pub use record::RecordRng;
pub use stub::StubRng;

pub(crate) const PORT_TAG: &str = "rng";
pub(crate) const CALL_NEXT_U64: &str = "next_u64";
pub(crate) const CALL_GEN_RANGE: &str = "gen_range";
