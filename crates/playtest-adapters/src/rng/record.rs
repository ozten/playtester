//! `Record<Rng>`: wraps an inner RNG and tees every call to a tape.

use core::ops::Range;
use std::path::Path;

use playtest_ports::{Rng, RngError};
use serde_json::{Value, json};

use crate::recording::{TapeError, TapeWriter};

use super::{CALL_GEN_RANGE, CALL_NEXT_U64, PORT_TAG};

#[derive(Debug)]
pub struct RecordRng<R: Rng> {
    inner: R,
    tape: TapeWriter,
}

impl<R: Rng> RecordRng<R> {
    pub fn create(inner: R, tape_path: impl AsRef<Path>) -> Result<Self, TapeError> {
        let tape = TapeWriter::create(tape_path.as_ref().to_path_buf(), PORT_TAG)?;
        Ok(Self { inner, tape })
    }

    pub fn flush(&mut self) -> Result<(), TapeError> {
        self.tape.flush()
    }
}

impl<R: Rng> Rng for RecordRng<R> {
    fn next_u64(&mut self) -> u64 {
        let v = self.inner.next_u64();
        self.tape
            .append(CALL_NEXT_U64, Value::Null, json!(v))
            .unwrap_or_else(|e| panic!("Record<Rng>: failed to append to tape: {e}"));
        v
    }

    fn gen_range(&mut self, range: Range<u64>) -> Result<u64, RngError> {
        let result = self.inner.gen_range(range.clone());
        // Record success and failure symmetrically so playback can reproduce
        // either case without guessing.
        let args = json!({ "start": range.start, "end": range.end });
        let output = match &result {
            Ok(v) => json!({ "ok": v }),
            Err(RngError::InvalidRange { start, end }) => {
                json!({ "err_invalid_range": { "start": start, "end": end } })
            }
        };
        self.tape
            .append(CALL_GEN_RANGE, args, output)
            .unwrap_or_else(|e| panic!("Record<Rng>: failed to append to tape: {e}"));
        result
    }
}
