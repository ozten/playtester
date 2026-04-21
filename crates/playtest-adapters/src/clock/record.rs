//! `Record<Clock>`: wraps an inner clock and tees every `now()` call to a
//! JSONL tape on disk. Paired with [`PlaybackClock`](super::PlaybackClock).

use std::path::Path;

use playtest_ports::{Clock, UnixMillis};
use serde_json::{Value, json};

use crate::recording::{TapeError, TapeWriter};

use super::{CALL_NOW, PORT_TAG};

#[derive(Debug)]
pub struct RecordClock<C: Clock> {
    inner: C,
    tape: TapeWriter,
}

impl<C: Clock> RecordClock<C> {
    /// Wrap `inner` and direct all recorded calls to a fresh tape at
    /// `tape_path`.
    pub fn create(inner: C, tape_path: impl AsRef<Path>) -> Result<Self, TapeError> {
        let tape = TapeWriter::create(tape_path.as_ref().to_path_buf(), PORT_TAG)?;
        Ok(Self { inner, tape })
    }

    /// Flush the underlying tape. Call at end of game so a crash cannot
    /// leave a half-written tape on disk.
    pub fn flush(&mut self) -> Result<(), TapeError> {
        self.tape.flush()
    }
}

impl<C: Clock> Clock for RecordClock<C> {
    fn now(&mut self) -> UnixMillis {
        let t = self.inner.now();
        // Appending to the tape can fail (disk full, permission, etc.), but
        // the `Clock` trait returns `UnixMillis` with no error channel. The
        // record adapter is a test/dev artifact; if the tape cannot be
        // written, we surface it the only way we can without changing the
        // trait: a panic that names the exact tape path.
        self.tape
            .append(CALL_NOW, Value::Null, json!(t))
            .unwrap_or_else(|e| panic!("Record<Clock>: failed to append to tape: {e}"));
        t
    }
}
