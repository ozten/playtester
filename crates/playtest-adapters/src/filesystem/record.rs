//! `Record<FileSystem>`: wraps an inner filesystem, tees each call to a
//! tape. Write/read payloads are stored as JSON arrays of byte values —
//! human-readable for small payloads and debuggable by hand.

use std::cell::RefCell;
use std::path::Path;

use playtest_ports::{FileSystem, FsError};
use serde_json::{Value, json};

use crate::recording::{TapeError, TapeWriter};

use super::{CALL_APPEND_LINE, CALL_EXISTS, CALL_READ, CALL_WRITE, PORT_TAG};

/// `Record<FileSystem>` uses interior mutability on the tape so the
/// port's `read` / `exists` methods (which take `&self`) can still append
/// entries. The tape is the only thing that needs to mutate on read.
#[derive(Debug)]
pub struct RecordFileSystem<F: FileSystem> {
    inner: F,
    tape: RefCell<TapeWriter>,
}

impl<F: FileSystem> RecordFileSystem<F> {
    pub fn create(inner: F, tape_path: impl AsRef<Path>) -> Result<Self, TapeError> {
        let tape = TapeWriter::create(tape_path.as_ref().to_path_buf(), PORT_TAG)?;
        Ok(Self {
            inner,
            tape: RefCell::new(tape),
        })
    }

    pub fn flush(&mut self) -> Result<(), TapeError> {
        self.tape.get_mut().flush()
    }

    fn append(&self, call: &str, args: Value, output: Value) {
        self.tape
            .borrow_mut()
            .append(call, args, output)
            .unwrap_or_else(|e| panic!("Record<FileSystem>: failed to append to tape: {e}"));
    }
}

pub(super) fn encode_fs_result<T: serde::Serialize>(result: &Result<T, FsError>) -> Value {
    match result {
        Ok(v) => json!({ "ok": serde_json::to_value(v).expect("output serializes") }),
        Err(FsError::NotFound { path }) => json!({ "err_not_found": { "path": path } }),
        Err(FsError::Io { path, source }) => {
            json!({ "err_io": { "path": path, "message": source.to_string() } })
        }
        Err(FsError::TapeDivergence { expected, actual }) => {
            json!({ "err_tape_divergence": { "expected": expected, "actual": actual } })
        }
    }
}

pub(super) fn encode_bytes(bytes: &[u8]) -> Value {
    serde_json::to_value(bytes).expect("byte slice always serializes")
}

impl<F: FileSystem> FileSystem for RecordFileSystem<F> {
    fn read(&self, path: &Path) -> Result<Vec<u8>, FsError> {
        let result = self.inner.read(path);
        let args = json!({ "path": path.display().to_string() });
        let output = encode_fs_result(&result);
        self.append(CALL_READ, args, output);
        result
    }

    fn write(&mut self, path: &Path, bytes: &[u8]) -> Result<(), FsError> {
        let result = self.inner.write(path, bytes);
        let args = json!({
            "path": path.display().to_string(),
            "bytes": encode_bytes(bytes),
        });
        let output = encode_fs_result(&result);
        self.append(CALL_WRITE, args, output);
        result
    }

    fn append_line(&mut self, path: &Path, line: &str) -> Result<(), FsError> {
        let result = self.inner.append_line(path, line);
        let args = json!({ "path": path.display().to_string(), "line": line });
        let output = encode_fs_result(&result);
        self.append(CALL_APPEND_LINE, args, output);
        result
    }

    fn exists(&self, path: &Path) -> bool {
        let result = self.inner.exists(path);
        let args = json!({ "path": path.display().to_string() });
        let output = json!(result);
        self.append(CALL_EXISTS, args, output);
        result
    }
}
