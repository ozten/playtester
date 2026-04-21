//! `Playback<LlmClient>`: reads a tape and replays recorded responses.
//!
//! Divergence (mismatched request) surfaces as [`LlmError::TapeDivergence`]
//! rather than a panic, because the trait has an error channel for
//! exactly this case — Phase 3 production clients will want to treat
//! replay divergence as a recoverable signal, not an abort.

use std::path::Path;
use std::sync::Mutex;

use async_trait::async_trait;
use playtest_ports::{LlmClient, LlmError, LlmRequest, LlmResponse};
use serde_json::Value;

use crate::recording::{TapeError, TapeReader};

use super::record::encode_request;
use super::{CALL_COMPLETE, PORT_TAG};

pub struct PlaybackLlmClient {
    tape: Mutex<TapeReader>,
}

impl std::fmt::Debug for PlaybackLlmClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PlaybackLlmClient").finish_non_exhaustive()
    }
}

impl PlaybackLlmClient {
    pub fn open(tape_path: impl AsRef<Path>) -> Result<Self, TapeError> {
        let tape = TapeReader::open(tape_path.as_ref().to_path_buf(), PORT_TAG)?;
        Ok(Self {
            tape: Mutex::new(tape),
        })
    }

    #[must_use]
    pub fn remaining(&self) -> usize {
        self.tape
            .lock()
            .expect("tape mutex is never poisoned here")
            .remaining()
    }
}

fn decode_response(out: &Value) -> Result<LlmResponse, LlmError> {
    if let Some(ok) = out.get("ok") {
        let text = ok
            .get("text")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned();
        let input_tokens =
            u32::try_from(ok.get("input_tokens").and_then(Value::as_u64).unwrap_or(0))
                .unwrap_or(u32::MAX);
        let output_tokens =
            u32::try_from(ok.get("output_tokens").and_then(Value::as_u64).unwrap_or(0))
                .unwrap_or(u32::MAX);
        Ok(LlmResponse {
            text,
            input_tokens,
            output_tokens,
        })
    } else if out.get("err_not_configured").is_some() {
        Err(LlmError::NotConfigured)
    } else if let Some(err) = out.get("err_budget_exceeded") {
        let requested = err.get("requested").and_then(Value::as_u64).unwrap_or(0);
        let remaining = err.get("remaining").and_then(Value::as_u64).unwrap_or(0);
        Err(LlmError::BudgetExceeded {
            requested,
            remaining,
        })
    } else if let Some(m) = out.get("err_transport") {
        let msg = m.as_str().unwrap_or("").to_owned();
        Err(LlmError::Transport(msg))
    } else if out.get("err_tape_divergence").is_some() {
        Err(LlmError::TapeDivergence)
    } else {
        panic!("Playback<LlmClient>: unrecognized output shape: {out}");
    }
}

#[async_trait]
impl LlmClient for PlaybackLlmClient {
    async fn complete(&self, req: LlmRequest) -> Result<LlmResponse, LlmError> {
        let args = encode_request(&req);
        let Ok(out) = self
            .tape
            .lock()
            .expect("tape mutex is never poisoned here")
            .next_output(CALL_COMPLETE, &args)
        else {
            return Err(LlmError::TapeDivergence);
        };
        decode_response(&out)
    }
}
