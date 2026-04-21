//! Filesystem port: the only gateway to persistent files.
//!
//! The engine proper never touches files. The CLI and `playtest-log`
//! crate funnel every read/write through this port so record/playback
//! tapes fully control what the test sees on disk.

use std::path::Path;

/// Errors produced by the [`FileSystem`] port.
#[derive(Debug, thiserror::Error)]
pub enum FsError {
    #[error("file not found: {path}")]
    NotFound { path: String },

    #[error("i/o error at {path}: {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },

    #[error("tape divergence: expected operation on {expected}, got {actual}")]
    TapeDivergence { expected: String, actual: String },
}

/// A minimal filesystem surface for the engine and CLI.
///
/// Adapter variants:
/// - `stub` — in-memory `HashMap<PathBuf, Vec<u8>>`.
/// - `production` — `std::fs` with all the usual caveats.
/// - `record` — tees every operation to a tape.
/// - `playback` — reads a tape, replaying stored results.
///
/// Intentionally narrow: no directory listing, no metadata queries yet.
/// Add them when a concrete caller needs them.
pub trait FileSystem {
    /// Read a file's entire contents.
    fn read(&self, path: &Path) -> Result<Vec<u8>, FsError>;

    /// Write `bytes` to `path`, creating the file (and parent directories)
    /// if necessary and truncating existing content.
    fn write(&mut self, path: &Path, bytes: &[u8]) -> Result<(), FsError>;

    /// Append `line` to `path`, adding a trailing newline. Creates the file
    /// (and parent directories) if necessary.
    fn append_line(&mut self, path: &Path, line: &str) -> Result<(), FsError>;

    /// Return `true` if a regular file exists at `path`.
    fn exists(&self, path: &Path) -> bool;
}
