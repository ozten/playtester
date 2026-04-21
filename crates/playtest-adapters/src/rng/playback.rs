//! `Playback<Rng>`: reads a tape and returns stored outputs.

use core::ops::Range;
use std::path::Path;

use playtest_ports::{Rng, RngError};
use serde_json::{Value, json};

use crate::recording::{TapeError, TapeReader};

use super::{CALL_GEN_RANGE, CALL_NEXT_U64, PORT_TAG};

#[derive(Debug)]
pub struct PlaybackRng {
    tape: TapeReader,
}

impl PlaybackRng {
    pub fn open(tape_path: impl AsRef<Path>) -> Result<Self, TapeError> {
        let tape = TapeReader::open(tape_path.as_ref().to_path_buf(), PORT_TAG)?;
        Ok(Self { tape })
    }

    #[must_use]
    pub fn remaining(&self) -> usize {
        self.tape.remaining()
    }
}

impl Rng for PlaybackRng {
    fn next_u64(&mut self) -> u64 {
        let out = self
            .tape
            .next_output(CALL_NEXT_U64, &Value::Null)
            .unwrap_or_else(|e| panic!("Playback<Rng>: {e}"));
        serde_json::from_value(out)
            .unwrap_or_else(|e| panic!("Playback<Rng>: tape entry was not a u64: {e}"))
    }

    fn gen_range(&mut self, range: Range<u64>) -> Result<u64, RngError> {
        let args = json!({ "start": range.start, "end": range.end });
        let out = self
            .tape
            .next_output(CALL_GEN_RANGE, &args)
            .unwrap_or_else(|e| panic!("Playback<Rng>: {e}"));
        if let Some(ok) = out.get("ok") {
            let v: u64 = serde_json::from_value(ok.clone())
                .unwrap_or_else(|e| panic!("Playback<Rng>: tape ok value was not a u64: {e}"));
            Ok(v)
        } else if let Some(err) = out.get("err_invalid_range") {
            let start = err.get("start").and_then(Value::as_u64).unwrap_or(0);
            let end = err.get("end").and_then(Value::as_u64).unwrap_or(0);
            Err(RngError::InvalidRange { start, end })
        } else {
            panic!("Playback<Rng>: unrecognized gen_range output shape: {out}");
        }
    }
}
