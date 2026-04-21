//! Streaming log reader.
//!
//! Iterates over [`LogRecord<E>`]s without loading the whole file.
//! Line numbers are tracked so parse errors point at the offending
//! line, not silent truncation.

use core::marker::PhantomData;
use std::io::BufRead;

use playtest_ports::GameEventSinkError;
use serde::de::DeserializeOwned;

use crate::record::LogRecord;

/// Errors produced by [`LogReader`] and related file-based helpers.
#[derive(Debug, thiserror::Error)]
pub enum ReadError {
    #[error("i/o error reading log: {0}")]
    Io(#[from] std::io::Error),

    #[error("malformed JSON on line {line}: {source}")]
    Malformed {
        /// 1-based line number for human consumption.
        line: usize,
        #[source]
        source: serde_json::Error,
    },

    #[error("sink error during replay: {0}")]
    Sink(#[from] GameEventSinkError),
}

/// Streaming iterator over the records in a log file.
pub struct LogReader<E, R: BufRead> {
    lines: std::io::Lines<R>,
    next_line_no: usize,
    _e: PhantomData<fn() -> E>,
}

impl<E: DeserializeOwned, R: BufRead> LogReader<E, R> {
    pub fn new(reader: R) -> Self {
        Self {
            lines: reader.lines(),
            next_line_no: 1,
            _e: PhantomData,
        }
    }
}

impl<E: DeserializeOwned, R: BufRead> Iterator for LogReader<E, R> {
    type Item = Result<LogRecord<E>, ReadError>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            let raw = match self.lines.next()? {
                Ok(s) => s,
                Err(e) => return Some(Err(ReadError::Io(e))),
            };
            let line_no = self.next_line_no;
            self.next_line_no += 1;
            if raw.trim().is_empty() {
                continue;
            }
            return Some(
                serde_json::from_str::<LogRecord<E>>(&raw).map_err(|source| ReadError::Malformed {
                    line: line_no,
                    source,
                }),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::header::{LogHeader, SCHEMA_VERSION};
    use serde::{Deserialize, Serialize};
    use std::io::Cursor;

    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
    struct Ping {
        n: u32,
    }

    fn header_line() -> String {
        let h = LogHeader {
            schema: SCHEMA_VERSION,
            game: "tally".into(),
            version: "0.0.0".into(),
            seed: 1,
            agents: vec![],
            started_at: 0,
            config_hash: "0".repeat(64),
        };
        serde_json::to_string(&LogRecord::<Ping>::Header(h)).unwrap()
    }

    #[test]
    fn reader_yields_header_events_final_in_order() {
        let mut buf = String::new();
        buf.push_str(&header_line());
        buf.push('\n');
        for n in 0..3 {
            let rec: LogRecord<Ping> = LogRecord::Event {
                tick: u64::from(n),
                payload: Ping { n },
            };
            buf.push_str(&serde_json::to_string(&rec).unwrap());
            buf.push('\n');
        }
        let fin: LogRecord<Ping> = LogRecord::Final {
            winner: None,
            reason: playtest_core::EndReason::Draw,
            scores: vec![],
        };
        buf.push_str(&serde_json::to_string(&fin).unwrap());
        buf.push('\n');

        let reader = LogReader::<Ping, _>::new(Cursor::new(buf));
        let records: Vec<_> = reader.collect::<Result<_, _>>().unwrap();
        assert_eq!(records.len(), 5);
        assert!(matches!(records[0], LogRecord::Header(_)));
        assert!(matches!(records[4], LogRecord::Final { .. }));
    }

    #[test]
    fn malformed_line_surfaces_with_accurate_line_number() {
        let mut buf = String::new();
        buf.push_str(&header_line());
        buf.push('\n');
        // valid, malformed, then valid — malformed is line 3 (1-based).
        buf.push_str("{\"kind\":\"event\",\"tick\":0,\"payload\":{\"n\":1}}\n");
        buf.push_str("not-json-at-all\n");
        buf.push_str("{\"kind\":\"event\",\"tick\":1,\"payload\":{\"n\":2}}\n");

        let mut reader = LogReader::<Ping, _>::new(Cursor::new(buf));
        reader.next().unwrap().unwrap(); // header
        reader.next().unwrap().unwrap(); // event 0
        let err = reader.next().unwrap().unwrap_err();
        match err {
            ReadError::Malformed { line, .. } => assert_eq!(line, 3),
            other => panic!("expected Malformed, got {other:?}"),
        }
    }

    #[test]
    fn blank_lines_are_skipped() {
        let mut buf = String::new();
        buf.push_str(&header_line());
        buf.push_str("\n\n\n");
        let fin: LogRecord<Ping> = LogRecord::Final {
            winner: None,
            reason: playtest_core::EndReason::Draw,
            scores: vec![],
        };
        buf.push_str(&serde_json::to_string(&fin).unwrap());
        buf.push('\n');

        let reader = LogReader::<Ping, _>::new(Cursor::new(buf));
        let records: Vec<_> = reader.collect::<Result<_, _>>().unwrap();
        assert_eq!(records.len(), 2);
    }
}
