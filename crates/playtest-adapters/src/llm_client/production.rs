//! Production `LlmClient`: returns [`LlmError::NotConfigured`] until
//! Phase 3 replaces this with a real provider.
//!
//! Keeping the type wired end-to-end (even as a placeholder) means the
//! adapter quartet discipline applies uniformly across every port, and
//! tests that exercise the wiring don't need to be conditionally compiled.

use async_trait::async_trait;
use playtest_ports::{LlmClient, LlmError, LlmRequest, LlmResponse};

#[derive(Debug, Default, Clone, Copy)]
pub struct ProductionLlmClient;

impl ProductionLlmClient {
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

#[async_trait]
impl LlmClient for ProductionLlmClient {
    async fn complete(&self, _req: LlmRequest) -> Result<LlmResponse, LlmError> {
        Err(LlmError::NotConfigured)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn production_returns_not_configured() {
        let c = ProductionLlmClient::new();
        let err = c
            .complete(LlmRequest {
                system: None,
                user: "hi".into(),
                max_tokens: 16,
            })
            .await
            .unwrap_err();
        assert!(matches!(err, LlmError::NotConfigured));
    }
}
