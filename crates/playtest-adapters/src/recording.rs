//! Shared recording-tape helper used by every `record` / `playback` adapter.
//!
//! Tapes are **port I/O** artifacts (category 1 in the
//! `playtest-ports` three-categories table): a sidecar file capturing the
//! exact sequence of calls made to an input port and the outputs returned.
//! They exist so a test run under `Playback<Port>` can reproduce the
//! non-determinism of a prior run under `Record<Port>` bit-for-bit.
//!
//! Tapes are intentionally **not** written through the [`FileSystem`] port:
//! they are infrastructure for making that port testable, so flowing them
//! back through it would invite recursion. Record/playback tape I/O
//! therefore goes directly to `std::fs`.
//!
//! # Format
//!
//! JSONL. The first line is a header:
//!
//! ```text
//! {"kind":"header","schema":1,"port":"rng"}
//! ```
//!
//! Subsequent lines are entries, one per port call:
//!
//! ```text
//! {"seq":0,"call":"gen_range","args":{"start":0,"end":52},"output":17}
//! ```
//!
//! [`FileSystem`]: playtest_ports::FileSystem

use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Current tape schema version. Bump when the header or entry shape changes
/// in a way existing tapes cannot be read under.
pub const SCHEMA_VERSION: u32 = 1;

/// Errors produced by the recording-tape helpers.
#[derive(Debug, thiserror::Error)]
pub enum TapeError {
    #[error("i/o error on tape {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("malformed tape {path} at line {line}: {source}")]
    Malformed {
        path: PathBuf,
        line: usize,
        #[source]
        source: serde_json::Error,
    },

    #[error("tape {path} is missing its header line")]
    MissingHeader { path: PathBuf },

    #[error("tape {path} has schema version {actual} but this build only understands {expected}")]
    SchemaMismatch {
        path: PathBuf,
        expected: u32,
        actual: u32,
    },

    #[error(
        "tape {path} was written for port `{actual}` but is being replayed against port `{expected}`"
    )]
    PortMismatch {
        path: PathBuf,
        expected: String,
        actual: String,
    },

    #[error(
        "tape exhausted at call {seq}: playback requested a `{call}` beyond the last recorded entry"
    )]
    Exhausted { seq: u64, call: String },

    #[error(
        "tape divergence at seq {seq}: expected call `{expected_call}`, playback made `{actual_call}`"
    )]
    CallDivergence {
        seq: u64,
        expected_call: String,
        actual_call: String,
    },

    #[error(
        "tape divergence at seq {seq} for call `{call}`: expected args {expected}, playback made {actual}"
    )]
    ArgsDivergence {
        seq: u64,
        call: String,
        expected: Value,
        actual: Value,
    },
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "kind")]
enum HeaderLine {
    #[serde(rename = "header")]
    Header { schema: u32, port: String },
}

#[derive(Debug, Serialize, Deserialize)]
struct EntryLine {
    seq: u64,
    call: String,
    args: Value,
    output: Value,
}

/// Append-only writer used by `record` adapters.
#[derive(Debug)]
pub struct TapeWriter {
    path: PathBuf,
    file: BufWriter<File>,
    next_seq: u64,
}

impl TapeWriter {
    /// Create a fresh tape at `path` for the given port name, truncating any
    /// existing file and writing the header line.
    pub fn create(path: impl Into<PathBuf>, port: &str) -> Result<Self, TapeError> {
        let path = path.into();
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            std::fs::create_dir_all(parent).map_err(|source| TapeError::Io {
                path: path.clone(),
                source,
            })?;
        }
        let file = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&path)
            .map_err(|source| TapeError::Io {
                path: path.clone(),
                source,
            })?;
        let mut writer = BufWriter::new(file);
        let header = HeaderLine::Header {
            schema: SCHEMA_VERSION,
            port: port.to_owned(),
        };
        let line = serde_json::to_string(&header).expect("header serializes");
        writeln!(writer, "{line}").map_err(|source| TapeError::Io {
            path: path.clone(),
            source,
        })?;
        Ok(Self {
            path,
            file: writer,
            next_seq: 0,
        })
    }

    /// Append one recorded port call.
    pub fn append(&mut self, call: &str, args: Value, output: Value) -> Result<(), TapeError> {
        let entry = EntryLine {
            seq: self.next_seq,
            call: call.to_owned(),
            args,
            output,
        };
        let line = serde_json::to_string(&entry).expect("entry serializes");
        writeln!(self.file, "{line}").map_err(|source| TapeError::Io {
            path: self.path.clone(),
            source,
        })?;
        self.next_seq += 1;
        Ok(())
    }

    /// Flush any buffered data to disk. Record adapters should call this
    /// from their equivalent of `Drop` or end-of-game hook.
    pub fn flush(&mut self) -> Result<(), TapeError> {
        self.file.flush().map_err(|source| TapeError::Io {
            path: self.path.clone(),
            source,
        })
    }
}

impl Drop for TapeWriter {
    fn drop(&mut self) {
        // Best-effort flush. If the user needed to see the error they had the
        // chance via `flush()`; here we just want a crash not to lose data.
        let _ = self.file.flush();
    }
}

/// Cursor-style reader used by `playback` adapters. The entire tape is
/// loaded eagerly: tapes are bounded by game length, and eager loading
/// lets us surface a malformed tape up-front instead of mid-replay.
#[derive(Debug)]
pub struct TapeReader {
    path: PathBuf,
    port: String,
    entries: Vec<EntryLine>,
    cursor: usize,
}

impl TapeReader {
    /// Open an existing tape at `path`, verifying it matches the expected
    /// port name and schema version.
    pub fn open(path: impl Into<PathBuf>, expected_port: &str) -> Result<Self, TapeError> {
        let path = path.into();
        let file = File::open(&path).map_err(|source| TapeError::Io {
            path: path.clone(),
            source,
        })?;
        let reader = BufReader::new(file);
        let mut lines = reader.lines().enumerate();

        let (line_no, header_line) = match lines.next() {
            Some((n, Ok(l))) => (n, l),
            Some((_, Err(source))) => {
                return Err(TapeError::Io { path, source });
            }
            None => return Err(TapeError::MissingHeader { path }),
        };

        let HeaderLine::Header { schema, port } =
            serde_json::from_str(&header_line).map_err(|source| TapeError::Malformed {
                path: path.clone(),
                line: line_no,
                source,
            })?;

        if schema != SCHEMA_VERSION {
            return Err(TapeError::SchemaMismatch {
                path,
                expected: SCHEMA_VERSION,
                actual: schema,
            });
        }
        if port != expected_port {
            return Err(TapeError::PortMismatch {
                path,
                expected: expected_port.to_owned(),
                actual: port,
            });
        }

        let mut entries = Vec::new();
        for (line_no, maybe_line) in lines {
            let line = maybe_line.map_err(|source| TapeError::Io {
                path: path.clone(),
                source,
            })?;
            if line.trim().is_empty() {
                continue;
            }
            let entry: EntryLine =
                serde_json::from_str(&line).map_err(|source| TapeError::Malformed {
                    path: path.clone(),
                    line: line_no,
                    source,
                })?;
            entries.push(entry);
        }

        Ok(Self {
            path,
            port: expected_port.to_owned(),
            entries,
            cursor: 0,
        })
    }

    /// Advance the cursor: verify that the next recorded entry matches the
    /// call the playback adapter just observed, and return its output.
    ///
    /// Divergence is a hard error — the whole point of the record/playback
    /// pair is to catch non-determinism the moment it appears.
    pub fn next_output(&mut self, call: &str, args: &Value) -> Result<Value, TapeError> {
        let Some(entry) = self.entries.get(self.cursor) else {
            return Err(TapeError::Exhausted {
                seq: self.cursor as u64,
                call: call.to_owned(),
            });
        };

        if entry.call != call {
            return Err(TapeError::CallDivergence {
                seq: entry.seq,
                expected_call: entry.call.clone(),
                actual_call: call.to_owned(),
            });
        }
        if &entry.args != args {
            return Err(TapeError::ArgsDivergence {
                seq: entry.seq,
                call: call.to_owned(),
                expected: entry.args.clone(),
                actual: args.clone(),
            });
        }

        let output = entry.output.clone();
        self.cursor += 1;
        Ok(output)
    }

    /// Number of recorded entries still unread.
    #[must_use]
    pub fn remaining(&self) -> usize {
        self.entries.len() - self.cursor
    }

    /// Port the tape was recorded for. Handy for diagnostics.
    #[must_use]
    pub fn port(&self) -> &str {
        &self.port
    }

    /// Path on disk. Handy for diagnostics.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tempfile::tempdir;

    #[test]
    fn roundtrip_empty_tape_is_immediately_exhausted() {
        let dir = tempdir().unwrap();
        let tape = dir.path().join("empty.jsonl");

        {
            let mut writer = TapeWriter::create(&tape, "rng").unwrap();
            writer.flush().unwrap();
        }

        let mut reader = TapeReader::open(&tape, "rng").unwrap();
        let err = reader.next_output("next_u64", &json!(null)).unwrap_err();
        assert!(matches!(err, TapeError::Exhausted { .. }));
    }

    #[test]
    fn roundtrip_records_and_replays_entries_in_order() {
        let dir = tempdir().unwrap();
        let tape = dir.path().join("seq.jsonl");

        {
            let mut writer = TapeWriter::create(&tape, "rng").unwrap();
            writer.append("next_u64", json!(null), json!(42)).unwrap();
            writer
                .append("gen_range", json!({"start": 0, "end": 52}), json!(17))
                .unwrap();
            writer.flush().unwrap();
        }

        let mut reader = TapeReader::open(&tape, "rng").unwrap();
        assert_eq!(reader.remaining(), 2);
        assert_eq!(
            reader.next_output("next_u64", &json!(null)).unwrap(),
            json!(42)
        );
        assert_eq!(
            reader
                .next_output("gen_range", &json!({"start": 0, "end": 52}))
                .unwrap(),
            json!(17)
        );
        assert_eq!(reader.remaining(), 0);
    }

    #[test]
    fn schema_mismatch_is_surfaced() {
        let dir = tempdir().unwrap();
        let tape = dir.path().join("oldschema.jsonl");
        std::fs::write(
            &tape,
            "{\"kind\":\"header\",\"schema\":0,\"port\":\"rng\"}\n",
        )
        .unwrap();

        let err = TapeReader::open(&tape, "rng").unwrap_err();
        assert!(matches!(
            err,
            TapeError::SchemaMismatch {
                expected: SCHEMA_VERSION,
                actual: 0,
                ..
            }
        ));
    }

    #[test]
    fn port_mismatch_is_surfaced() {
        let dir = tempdir().unwrap();
        let tape = dir.path().join("wrongport.jsonl");
        {
            let mut writer = TapeWriter::create(&tape, "clock").unwrap();
            writer.flush().unwrap();
        }
        let err = TapeReader::open(&tape, "rng").unwrap_err();
        assert!(matches!(err, TapeError::PortMismatch { .. }));
    }

    #[test]
    fn call_divergence_is_surfaced() {
        let dir = tempdir().unwrap();
        let tape = dir.path().join("diverge.jsonl");
        {
            let mut writer = TapeWriter::create(&tape, "rng").unwrap();
            writer.append("next_u64", json!(null), json!(1)).unwrap();
            writer.flush().unwrap();
        }
        let mut reader = TapeReader::open(&tape, "rng").unwrap();
        let err = reader
            .next_output("gen_range", &json!({"start": 0, "end": 3}))
            .unwrap_err();
        assert!(matches!(err, TapeError::CallDivergence { .. }));
    }

    #[test]
    fn args_divergence_is_surfaced() {
        let dir = tempdir().unwrap();
        let tape = dir.path().join("argdiverge.jsonl");
        {
            let mut writer = TapeWriter::create(&tape, "rng").unwrap();
            writer
                .append("gen_range", json!({"start": 0, "end": 52}), json!(1))
                .unwrap();
            writer.flush().unwrap();
        }
        let mut reader = TapeReader::open(&tape, "rng").unwrap();
        let err = reader
            .next_output("gen_range", &json!({"start": 0, "end": 53}))
            .unwrap_err();
        assert!(matches!(err, TapeError::ArgsDivergence { .. }));
    }

    #[test]
    fn missing_header_is_surfaced() {
        let dir = tempdir().unwrap();
        let tape = dir.path().join("empty-file.jsonl");
        std::fs::write(&tape, "").unwrap();
        let err = TapeReader::open(&tape, "rng").unwrap_err();
        assert!(matches!(err, TapeError::MissingHeader { .. }));
    }
}
