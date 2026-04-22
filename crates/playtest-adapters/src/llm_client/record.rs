//! `Record<LlmClient>`: wraps any inner client and tees every
//! `(request, response)` pair to a tape.
//!
//! Like [`RecordFileSystem`](crate::filesystem::RecordFileSystem), this
//! uses interior mutability: `LlmClient::complete` takes `&self` (async
//! clients are typically shared), but the tape needs `&mut`.
//!
//! The tape encoding is internal to this crate. It is a direct
//! `serde_json` projection of [`LlmRequest`]/[`LlmResponse`]; Phase 3's
//! shape extension is a clean rewrite with no versioned migration.

use std::path::Path;
use std::sync::Mutex;

use async_trait::async_trait;
use playtest_ports::{LlmClient, LlmError, LlmRequest, LlmResponse};
use serde_json::{Value, json};

use crate::recording::{TapeError, TapeWriter};

use super::{CALL_COMPLETE, PORT_TAG};

pub struct RecordLlmClient<C: LlmClient + Send + Sync> {
    inner: C,
    tape: Mutex<TapeWriter>,
}

impl<C: LlmClient + Send + Sync> std::fmt::Debug for RecordLlmClient<C> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RecordLlmClient").finish_non_exhaustive()
    }
}

impl<C: LlmClient + Send + Sync> RecordLlmClient<C> {
    pub fn create(inner: C, tape_path: impl AsRef<Path>) -> Result<Self, TapeError> {
        let tape = TapeWriter::create(tape_path.as_ref().to_path_buf(), PORT_TAG)?;
        Ok(Self {
            inner,
            tape: Mutex::new(tape),
        })
    }

    pub fn flush(&mut self) -> Result<(), TapeError> {
        self.tape
            .get_mut()
            .expect("tape mutex is never poisoned here")
            .flush()
    }
}

pub(super) fn encode_request(req: &LlmRequest) -> Value {
    serde_json::to_value(req).expect("LlmRequest serializes cleanly")
}

fn encode_response(result: &Result<LlmResponse, LlmError>) -> Value {
    match result {
        Ok(r) => json!({
            "ok": serde_json::to_value(r).expect("LlmResponse serializes cleanly"),
        }),
        Err(LlmError::NotConfigured) => json!({ "err_not_configured": {} }),
        Err(LlmError::BudgetExceeded {
            requested,
            remaining,
        }) => json!({
            "err_budget_exceeded": { "requested": requested, "remaining": remaining }
        }),
        Err(LlmError::Transport(m)) => json!({ "err_transport": m }),
        Err(LlmError::TapeDivergence) => json!({ "err_tape_divergence": {} }),
    }
}

#[async_trait]
impl<C: LlmClient + Send + Sync> LlmClient for RecordLlmClient<C> {
    async fn complete(&self, req: LlmRequest) -> Result<LlmResponse, LlmError> {
        let args = encode_request(&req);
        let result = self.inner.complete(req).await;
        let output = encode_response(&result);
        self.tape
            .lock()
            .expect("tape mutex is never poisoned here")
            .append(CALL_COMPLETE, args, output)
            .unwrap_or_else(|e| panic!("Record<LlmClient>: failed to append to tape: {e}"));
        result
    }
}
