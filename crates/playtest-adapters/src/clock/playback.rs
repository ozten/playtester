//! `Playback<Clock>`: reads a tape written by [`RecordClock`](super::RecordClock)
//! and returns stored times in order. Divergence from the recorded call
//! pattern panics with a clear message — that is the test signal for
//! non-determinism.

use std::path::Path;

use playtest_ports::{Clock, UnixMillis};
use serde_json::Value;

use crate::recording::{TapeError, TapeReader};

use super::{CALL_NOW, PORT_TAG};

#[derive(Debug)]
pub struct PlaybackClock {
    tape: TapeReader,
}

impl PlaybackClock {
    pub fn open(tape_path: impl AsRef<Path>) -> Result<Self, TapeError> {
        let tape = TapeReader::open(tape_path.as_ref().to_path_buf(), PORT_TAG)?;
        Ok(Self { tape })
    }

    #[must_use]
    pub fn remaining(&self) -> usize {
        self.tape.remaining()
    }
}

impl Clock for PlaybackClock {
    fn now(&mut self) -> UnixMillis {
        let out = self
            .tape
            .next_output(CALL_NOW, &Value::Null)
            .unwrap_or_else(|e| panic!("Playback<Clock>: {e}"));
        serde_json::from_value(out)
            .unwrap_or_else(|e| panic!("Playback<Clock>: tape entry was not a UnixMillis: {e}"))
    }
}
