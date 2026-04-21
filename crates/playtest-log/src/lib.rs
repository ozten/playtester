//! Event log: JSONL writer, streaming reader, and replay.
//!
//! One file per game. Header line + N event lines + optional final line.
//! State snapshots are derived by replaying events from the seed, never
//! serialized separately (the "snapshot = replay" invariant in the
//! architectural memo).
//!
//! The wire format for event lines is identical to what `GameLoop`
//! emits directly, so the loop can feed a raw `GameEventSink` and this
//! crate's [`LogReader`] parses the same stream.

pub mod header;
pub mod reader;
pub mod record;
pub mod replay;
pub mod writer;

pub use header::{LogHeader, SCHEMA_VERSION, compute_config_hash};
pub use reader::{LogReader, ReadError};
pub use record::LogRecord;
pub use replay::{Replay, ReplayError, replay, replay_final};
pub use writer::{EventLogWriter, WriteError};
