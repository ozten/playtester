//! Stub `LlmClient`: returns a canned response, optionally echoing the
//! user prompt so assertions can pin the exact string.

use async_trait::async_trait;
use playtest_ports::{LlmClient, LlmError, LlmRequest, LlmResponse};

#[derive(Debug, Clone)]
pub struct StubLlmClient {
    canned_text: String,
    input_tokens: u32,
    output_tokens: u32,
}

impl StubLlmClient {
    #[must_use]
    pub fn new(canned_text: impl Into<String>) -> Self {
        Self {
            canned_text: canned_text.into(),
            input_tokens: 0,
            output_tokens: 0,
        }
    }

    #[must_use]
    pub fn with_token_counts(mut self, input: u32, output: u32) -> Self {
        self.input_tokens = input;
        self.output_tokens = output;
        self
    }
}

#[async_trait]
impl LlmClient for StubLlmClient {
    async fn complete(&self, _req: LlmRequest) -> Result<LlmResponse, LlmError> {
        Ok(LlmResponse {
            text: self.canned_text.clone(),
            input_tokens: self.input_tokens,
            output_tokens: self.output_tokens,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn canned_response_is_returned() {
        let c = StubLlmClient::new("hi").with_token_counts(3, 2);
        let resp = c
            .complete(LlmRequest {
                system: None,
                user: "whatever".into(),
                max_tokens: 8,
            })
            .await
            .unwrap();
        assert_eq!(resp.text, "hi");
        assert_eq!(resp.input_tokens, 3);
        assert_eq!(resp.output_tokens, 2);
    }
}
