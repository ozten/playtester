//! Stub `LlmClient`: returns a canned response, optionally echoing the
//! user prompt so assertions can pin the exact string.

use async_trait::async_trait;
use playtest_ports::{LlmClient, LlmError, LlmRequest, LlmResponse};

#[derive(Debug, Clone)]
pub struct StubLlmClient {
    canned_text: String,
    input_tokens: u32,
    output_tokens: u32,
    cache_read_input_tokens: u32,
    cache_creation_input_tokens: u32,
}

impl StubLlmClient {
    #[must_use]
    pub fn new(canned_text: impl Into<String>) -> Self {
        Self {
            canned_text: canned_text.into(),
            input_tokens: 0,
            output_tokens: 0,
            cache_read_input_tokens: 0,
            cache_creation_input_tokens: 0,
        }
    }

    #[must_use]
    pub fn with_token_counts(mut self, input: u32, output: u32) -> Self {
        self.input_tokens = input;
        self.output_tokens = output;
        self
    }

    /// Extend the canned response with Anthropic-style cache accounting.
    /// Both fields default to 0 when unset.
    #[must_use]
    pub fn with_cache_tokens(mut self, cache_read: u32, cache_creation: u32) -> Self {
        self.cache_read_input_tokens = cache_read;
        self.cache_creation_input_tokens = cache_creation;
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
            cache_read_input_tokens: self.cache_read_input_tokens,
            cache_creation_input_tokens: self.cache_creation_input_tokens,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use playtest_ports::{ChatMessage, ChatRole};

    #[tokio::test]
    async fn canned_response_is_returned() {
        let c = StubLlmClient::new("hi").with_token_counts(3, 2);
        let resp = c
            .complete(LlmRequest {
                system_blocks: vec![],
                messages: vec![ChatMessage {
                    role: ChatRole::User,
                    content: "whatever".into(),
                }],
                model: "claude-test".into(),
                max_tokens: 8,
                temperature: None,
            })
            .await
            .unwrap();
        assert_eq!(resp.text, "hi");
        assert_eq!(resp.input_tokens, 3);
        assert_eq!(resp.output_tokens, 2);
        assert_eq!(resp.cache_read_input_tokens, 0);
        assert_eq!(resp.cache_creation_input_tokens, 0);
    }

    #[tokio::test]
    async fn cache_tokens_round_trip() {
        let c = StubLlmClient::new("hi")
            .with_token_counts(10, 4)
            .with_cache_tokens(7, 3);
        let resp = c
            .complete(LlmRequest {
                system_blocks: vec![],
                messages: vec![ChatMessage {
                    role: ChatRole::User,
                    content: "x".into(),
                }],
                model: "claude-test".into(),
                max_tokens: 8,
                temperature: None,
            })
            .await
            .unwrap();
        assert_eq!(resp.cache_read_input_tokens, 7);
        assert_eq!(resp.cache_creation_input_tokens, 3);
    }
}
