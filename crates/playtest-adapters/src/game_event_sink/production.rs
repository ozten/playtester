//! Production `GameEventSink`: appends lines through the [`FileSystem`]
//! port to a single JSONL log file per game.
//!
//! This sink is intentionally simple: it holds the file path and defers
//! every write to whatever `FileSystem` impl it was constructed with.
//! Under `StubFileSystem` the writes go to an in-memory `HashMap`; under
//! `ProductionFileSystem` they hit disk via `std::fs`.

use std::path::PathBuf;

use playtest_ports::{FileSystem, GameEventSink, GameEventSinkError};

#[derive(Debug)]
pub struct ProductionGameEventSink<F: FileSystem> {
    fs: F,
    path: PathBuf,
    closed: bool,
}

impl<F: FileSystem> ProductionGameEventSink<F> {
    pub fn new(fs: F, path: impl Into<PathBuf>) -> Self {
        Self {
            fs,
            path: path.into(),
            closed: false,
        }
    }

    pub fn close(&mut self) {
        self.closed = true;
    }

    /// Consume the sink and return the underlying filesystem so tests
    /// using `StubFileSystem` can inspect what was written.
    pub fn into_inner(self) -> F {
        self.fs
    }
}

impl<F: FileSystem> GameEventSink for ProductionGameEventSink<F> {
    fn emit(&mut self, line: &str) -> Result<(), GameEventSinkError> {
        if self.closed {
            return Err(GameEventSinkError::Closed);
        }
        self.fs
            .append_line(&self.path, line)
            .map_err(|e| GameEventSinkError::Io(std::io::Error::other(e.to_string())))
    }

    fn flush(&mut self) -> Result<(), GameEventSinkError> {
        // `FileSystem::append_line` is not buffered at the port layer, so
        // there is nothing to flush here. The concrete `ProductionFileSystem`
        // drops its `File` after each append, which flushes the OS buffer.
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::filesystem::StubFileSystem;
    use std::path::Path;

    #[test]
    fn emit_appends_lines_to_underlying_fs() {
        let fs = StubFileSystem::new();
        let mut sink = ProductionGameEventSink::new(fs, "/log.jsonl");
        sink.emit("line one").unwrap();
        sink.emit("line two\n").unwrap();
        let fs = sink.into_inner();
        assert_eq!(
            fs.snapshot(Path::new("/log.jsonl")).unwrap(),
            b"line one\nline two\n"
        );
    }

    #[test]
    fn closed_sink_rejects_writes() {
        let fs = StubFileSystem::new();
        let mut sink = ProductionGameEventSink::new(fs, "/log.jsonl");
        sink.close();
        let err = sink.emit("x").unwrap_err();
        assert!(matches!(err, GameEventSinkError::Closed));
    }
}
