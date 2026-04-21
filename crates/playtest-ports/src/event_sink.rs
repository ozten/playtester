//! Event sink port: where serialized event-log lines are written.
//!
//! Deliberately line-oriented and ignorant of JSON: the `playtest-log`
//! crate formats records into strings, this port writes them. Keeping the
//! port dumb means the stub adapter is just a `Vec<String>`.

/// Errors produced by the [`EventSink`] port.
#[derive(Debug, thiserror::Error)]
pub enum EventSinkError {
    #[error("i/o error: {0}")]
    Io(#[from] std::io::Error),

    #[error("sink is closed and can no longer accept writes")]
    Closed,
}

/// A line-oriented destination for serialized game events.
///
/// Adapter variants:
/// - `stub` — collects lines in a `Vec<String>` for test inspection.
/// - `production` — appends to a JSONL file via the filesystem port.
/// - `record` — for symmetry; typically aliases the production writer.
/// - `playback` — no-op that asserts it is never written to during replay
///   (replay is strictly a read path).
pub trait EventSink {
    /// Write a single event-log line. The caller is responsible for
    /// serialization; this port only knows about strings.
    ///
    /// Implementations must add a trailing newline if one is not present.
    fn emit(&mut self, line: &str) -> Result<(), EventSinkError>;

    /// Flush buffered writes to the underlying medium. Game-end writes
    /// must flush before the binary exits, otherwise a crash could leave
    /// a half-written log on disk.
    fn flush(&mut self) -> Result<(), EventSinkError>;
}
