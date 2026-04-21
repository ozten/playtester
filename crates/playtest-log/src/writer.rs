//! `EventLogWriter`: converts typed records to JSONL lines and writes
//! them through a [`GameEventSink`].
//!
//! The sink is line-oriented and format-ignorant by design (see the
//! three-categories-of-recording doc on `playtest-ports::lib`). This
//! writer is where the JSON format lives. The event-line format is
//! identical to what `GameLoop` emits — the writer is the thing that
//! sandwiches the loop's emissions with a header at the start and a
//! final record at the end.

use core::marker::PhantomData;

use playtest_core::GameResult;
use playtest_ports::{GameEventSink, GameEventSinkError, UnixMillis};
use serde::Serialize;

use crate::header::LogHeader;
use crate::record::LogRecord;

/// Errors produced by [`EventLogWriter`].
#[derive(Debug, thiserror::Error)]
pub enum WriteError {
    #[error("serializing log record: {0}")]
    Serde(#[from] serde_json::Error),

    #[error("event sink rejected a write: {0}")]
    Sink(#[from] GameEventSinkError),

    #[error("header must be written exactly once before any events")]
    HeaderAlreadyWritten,

    #[error("header must be written before any events or the final record")]
    HeaderNotWritten,

    #[error("final record has already been written; no further writes allowed")]
    AlreadyFinished,
}

/// Write a typed stream of [`LogRecord<E>`]s to a [`GameEventSink`].
///
/// Lifecycle: `write_header` → N × `write_event` → `finish`. Violating
/// the order yields [`WriteError::HeaderAlreadyWritten`],
/// [`WriteError::HeaderNotWritten`], or [`WriteError::AlreadyFinished`].
pub struct EventLogWriter<'s, E> {
    sink: &'s mut dyn GameEventSink,
    state: State,
    _e: PhantomData<fn() -> E>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    Fresh,
    HeaderWritten,
    Finished,
}

/// Borrow-compatible sibling of `LogRecord::Event`. Serializes to the
/// same JSON wire shape. We cannot use the enum directly here because
/// its `payload` field owns the event; write paths need to borrow.
#[derive(Serialize)]
struct EventLine<'a, E> {
    kind: &'static str,
    tick: u64,
    payload: &'a E,
}

impl<'s, E: Serialize> EventLogWriter<'s, E> {
    pub fn new(sink: &'s mut dyn GameEventSink) -> Self {
        Self {
            sink,
            state: State::Fresh,
            _e: PhantomData,
        }
    }

    /// Write the header line. Must be called exactly once, before any
    /// events.
    ///
    /// # Errors
    /// Returns [`WriteError::HeaderAlreadyWritten`] if called twice, or
    /// [`WriteError::AlreadyFinished`] if the writer is done.
    pub fn write_header(&mut self, header: &LogHeader) -> Result<(), WriteError> {
        match self.state {
            State::HeaderWritten => return Err(WriteError::HeaderAlreadyWritten),
            State::Finished => return Err(WriteError::AlreadyFinished),
            State::Fresh => {}
        }
        let rec: LogRecord<E> = LogRecord::Header(header.clone());
        self.emit(&rec)?;
        self.state = State::HeaderWritten;
        Ok(())
    }

    /// Write one event record. `tick` should come from the game loop's
    /// counter.
    ///
    /// # Errors
    /// See [`WriteError`].
    pub fn write_event(&mut self, tick: u64, payload: &E) -> Result<(), WriteError> {
        match self.state {
            State::Fresh => return Err(WriteError::HeaderNotWritten),
            State::Finished => return Err(WriteError::AlreadyFinished),
            State::HeaderWritten => {}
        }
        let line = serde_json::to_string(&EventLine {
            kind: "event",
            tick,
            payload,
        })?;
        self.sink.emit(&line)?;
        Ok(())
    }

    /// Write the final record and flush the sink.
    ///
    /// Matches the plan's "finish() writes Final *and* calls flush()"
    /// contract so a soak-test crash cannot leave a half-written log.
    /// `finished_at` is the wall-clock time the loop ended, captured by
    /// the caller via the `Clock` port — the writer stays clock-free.
    ///
    /// # Errors
    /// See [`WriteError`].
    pub fn finish(
        &mut self,
        result: &GameResult,
        finished_at: UnixMillis,
    ) -> Result<(), WriteError> {
        match self.state {
            State::Fresh => return Err(WriteError::HeaderNotWritten),
            State::Finished => return Err(WriteError::AlreadyFinished),
            State::HeaderWritten => {}
        }
        let rec: LogRecord<E> = LogRecord::final_from_result(result, finished_at);
        self.emit(&rec)?;
        self.sink.flush()?;
        self.state = State::Finished;
        Ok(())
    }

    fn emit(&mut self, rec: &LogRecord<E>) -> Result<(), WriteError> {
        let line = serde_json::to_string(rec)?;
        self.sink.emit(&line)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use playtest_core::EndReason;
    use serde::{Deserialize, Serialize};

    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
    struct Ping {
        n: u32,
    }

    #[derive(Default)]
    struct CapturingSink {
        lines: Vec<String>,
        flushed: bool,
    }

    impl GameEventSink for CapturingSink {
        fn emit(&mut self, line: &str) -> Result<(), GameEventSinkError> {
            self.lines.push(line.to_owned());
            Ok(())
        }
        fn flush(&mut self) -> Result<(), GameEventSinkError> {
            self.flushed = true;
            Ok(())
        }
    }

    fn sample_header() -> LogHeader {
        LogHeader {
            schema: crate::header::SCHEMA_VERSION,
            game: "tally".into(),
            version: "0.0.0".into(),
            seed: 1,
            agents: vec!["random".into()],
            started_at: 0,
            config_hash: "0".repeat(64),
        }
    }

    #[test]
    fn happy_path_writes_header_events_final_and_flushes() {
        let mut sink = CapturingSink::default();
        let mut writer: EventLogWriter<Ping> = EventLogWriter::new(&mut sink);
        writer.write_header(&sample_header()).unwrap();
        writer.write_event(0, &Ping { n: 1 }).unwrap();
        writer.write_event(1, &Ping { n: 2 }).unwrap();
        writer
            .finish(
                &GameResult {
                    winner: Some(0),
                    reason: EndReason::Victory,
                    scores: vec![2, 0],
                },
                1_700_000_000_420,
            )
            .unwrap();
        assert_eq!(sink.lines.len(), 4);
        assert!(sink.lines[0].contains("\"kind\":\"header\""));
        assert!(sink.lines[1].contains("\"kind\":\"event\""));
        assert!(sink.lines[3].contains("\"kind\":\"final\""));
        assert!(
            sink.lines[3].contains("\"finished_at\":1700000000420"),
            "final line missing finished_at: {}",
            sink.lines[3]
        );
        assert!(sink.flushed);
    }

    #[test]
    fn writing_header_twice_is_rejected() {
        let mut sink = CapturingSink::default();
        let mut writer: EventLogWriter<Ping> = EventLogWriter::new(&mut sink);
        writer.write_header(&sample_header()).unwrap();
        let err = writer.write_header(&sample_header()).unwrap_err();
        assert!(matches!(err, WriteError::HeaderAlreadyWritten));
    }

    #[test]
    fn events_before_header_are_rejected() {
        let mut sink = CapturingSink::default();
        let mut writer: EventLogWriter<Ping> = EventLogWriter::new(&mut sink);
        let err = writer.write_event(0, &Ping { n: 1 }).unwrap_err();
        assert!(matches!(err, WriteError::HeaderNotWritten));
    }

    #[test]
    fn writes_after_finish_are_rejected() {
        let mut sink = CapturingSink::default();
        let mut writer: EventLogWriter<Ping> = EventLogWriter::new(&mut sink);
        writer.write_header(&sample_header()).unwrap();
        writer
            .finish(
                &GameResult {
                    winner: None,
                    reason: EndReason::Draw,
                    scores: vec![],
                },
                0,
            )
            .unwrap();
        let err = writer.write_event(0, &Ping { n: 1 }).unwrap_err();
        assert!(matches!(err, WriteError::AlreadyFinished));
    }
}
