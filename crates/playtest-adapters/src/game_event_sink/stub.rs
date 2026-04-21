//! Stub `GameEventSink`: collects lines in a `Vec<String>` for inspection.

use playtest_ports::{GameEventSink, GameEventSinkError};

#[derive(Debug, Default, Clone)]
pub struct StubGameEventSink {
    lines: Vec<String>,
    closed: bool,
}

impl StubGameEventSink {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Snapshot the lines captured so far.
    #[must_use]
    pub fn lines(&self) -> &[String] {
        &self.lines
    }

    /// Mark the sink closed so subsequent writes fail with
    /// [`GameEventSinkError::Closed`], matching the behavior of the
    /// production sink after its owning `EventLogWriter` shuts down.
    pub fn close(&mut self) {
        self.closed = true;
    }
}

impl GameEventSink for StubGameEventSink {
    fn emit(&mut self, line: &str) -> Result<(), GameEventSinkError> {
        if self.closed {
            return Err(GameEventSinkError::Closed);
        }
        // Normalize trailing newline the same way the production sink
        // does, so tests compare apples to apples.
        let mut s = line.to_owned();
        if !s.ends_with('\n') {
            s.push('\n');
        }
        self.lines.push(s);
        Ok(())
    }

    fn flush(&mut self) -> Result<(), GameEventSinkError> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn emit_collects_lines_with_newlines() {
        let mut sink = StubGameEventSink::new();
        sink.emit("a").unwrap();
        sink.emit("b\n").unwrap();
        assert_eq!(sink.lines(), &["a\n".to_owned(), "b\n".to_owned()]);
    }

    #[test]
    fn closed_sink_rejects_writes() {
        let mut sink = StubGameEventSink::new();
        sink.close();
        let err = sink.emit("x").unwrap_err();
        assert!(matches!(err, GameEventSinkError::Closed));
    }
}
