//! Game event sink: where the engine writes the authoritative stream of
//! domain events that *is* a game's history (see the three-categories
//! discussion in [`crate`]).
//!
//! Deliberately line-oriented and ignorant of JSON: the `playtest-log`
//! crate formats records into strings, this port writes them. Keeping the
//! port dumb means the stub adapter is just a `Vec<String>`.

/// Errors produced by the [`GameEventSink`] port.
#[derive(Debug, thiserror::Error)]
pub enum GameEventSinkError {
    #[error("i/o error: {0}")]
    Io(#[from] std::io::Error),

    #[error("sink is closed and can no longer accept writes")]
    Closed,
}

/// A line-oriented destination for serialized game events.
///
/// This is an **output** port: the engine produces to it, never consumes
/// from it. Its adapter story is therefore asymmetric with the input
/// ports (Clock, Rng, FileSystem, LlmClient):
///
/// - `stub` — collects lines in a `Vec<String>` for test inspection.
/// - `production` — appends to a JSONL file via the filesystem port.
/// - `record` — aliases the production writer; there is no separate
///   sidecar tape because the event log *is* the tape for the game.
/// - `playback` — no-op that asserts it is never written to during
///   replay (replay is strictly a read path over an existing log).
pub trait GameEventSink {
    /// Write a single event-log line. The caller is responsible for
    /// serialization; this port only knows about strings.
    ///
    /// Implementations must add a trailing newline if one is not present.
    fn emit(&mut self, line: &str) -> Result<(), GameEventSinkError>;

    /// Flush buffered writes to the underlying medium. Game-end writes
    /// must flush before the binary exits, otherwise a crash could leave
    /// a half-written log on disk.
    fn flush(&mut self) -> Result<(), GameEventSinkError>;
}
