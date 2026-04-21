//! `FileSystem` port adapters: stub, production, record, playback.

pub mod playback;
pub mod production;
pub mod record;
pub mod stub;

pub use playback::PlaybackFileSystem;
pub use production::ProductionFileSystem;
pub use record::RecordFileSystem;
pub use stub::StubFileSystem;

pub(crate) const PORT_TAG: &str = "filesystem";
pub(crate) const CALL_READ: &str = "read";
pub(crate) const CALL_WRITE: &str = "write";
pub(crate) const CALL_APPEND_LINE: &str = "append_line";
pub(crate) const CALL_EXISTS: &str = "exists";
