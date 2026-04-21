//! `Playback<FileSystem>`: replays a tape recorded by
//! [`RecordFileSystem`](super::RecordFileSystem).

use std::cell::RefCell;
use std::path::Path;

use playtest_ports::{FileSystem, FsError};
use serde_json::{Value, json};

use crate::recording::{TapeError, TapeReader};

use super::record::encode_bytes;
use super::{CALL_APPEND_LINE, CALL_EXISTS, CALL_READ, CALL_WRITE, PORT_TAG};

#[derive(Debug)]
pub struct PlaybackFileSystem {
    tape: RefCell<TapeReader>,
}

impl PlaybackFileSystem {
    pub fn open(tape_path: impl AsRef<Path>) -> Result<Self, TapeError> {
        let tape = TapeReader::open(tape_path.as_ref().to_path_buf(), PORT_TAG)?;
        Ok(Self {
            tape: RefCell::new(tape),
        })
    }

    #[must_use]
    pub fn remaining(&self) -> usize {
        self.tape.borrow().remaining()
    }

    fn next_output(&self, call: &str, args: &Value) -> Value {
        self.tape
            .borrow_mut()
            .next_output(call, args)
            .unwrap_or_else(|e| panic!("Playback<FileSystem>: {e}"))
    }
}

fn decode_fs_result<T, F>(call: &str, out: &Value, decode_ok: F) -> Result<T, FsError>
where
    F: FnOnce(&Value) -> T,
{
    if let Some(ok) = out.get("ok") {
        Ok(decode_ok(ok))
    } else if let Some(err) = out.get("err_not_found") {
        let path = err
            .get("path")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned();
        Err(FsError::NotFound { path })
    } else if let Some(err) = out.get("err_io") {
        let path = err
            .get("path")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned();
        let message = err
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("replayed i/o error")
            .to_owned();
        Err(FsError::Io {
            path,
            source: std::io::Error::other(message),
        })
    } else if let Some(err) = out.get("err_tape_divergence") {
        let expected = err
            .get("expected")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned();
        let actual = err
            .get("actual")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned();
        Err(FsError::TapeDivergence { expected, actual })
    } else {
        panic!("Playback<FileSystem>: unrecognized {call} output shape: {out}");
    }
}

fn decode_bytes(value: &Value) -> Vec<u8> {
    serde_json::from_value(value.clone())
        .unwrap_or_else(|e| panic!("Playback<FileSystem>: byte payload was malformed: {e}"))
}

impl FileSystem for PlaybackFileSystem {
    fn read(&self, path: &Path) -> Result<Vec<u8>, FsError> {
        let args = json!({ "path": path.display().to_string() });
        let out = self.next_output(CALL_READ, &args);
        decode_fs_result(CALL_READ, &out, decode_bytes)
    }

    fn write(&mut self, path: &Path, bytes: &[u8]) -> Result<(), FsError> {
        let args = json!({
            "path": path.display().to_string(),
            "bytes": encode_bytes(bytes),
        });
        let out = self.next_output(CALL_WRITE, &args);
        decode_fs_result(CALL_WRITE, &out, |_| ())
    }

    fn append_line(&mut self, path: &Path, line: &str) -> Result<(), FsError> {
        let args = json!({ "path": path.display().to_string(), "line": line });
        let out = self.next_output(CALL_APPEND_LINE, &args);
        decode_fs_result(CALL_APPEND_LINE, &out, |_| ())
    }

    fn exists(&self, path: &Path) -> bool {
        let args = json!({ "path": path.display().to_string() });
        let out = self.next_output(CALL_EXISTS, &args);
        out.as_bool().unwrap_or_else(|| {
            panic!("Playback<FileSystem>: `exists` output was not a boolean: {out}")
        })
    }
}
