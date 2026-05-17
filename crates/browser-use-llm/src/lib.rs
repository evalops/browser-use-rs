//! LLM provider contracts for schema-guided agent calls.

use async_trait::async_trait;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum MessageRole {
    System,
    User,
    Assistant,
    Tool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentPart {
    Text { text: String },
    ImageUrl { image_url: String },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ChatMessage {
    pub role: MessageRole,
    pub content: Vec<ContentPart>,
}

impl ChatMessage {
    #[must_use]
    pub fn text(role: MessageRole, text: impl Into<String>) -> Self {
        Self {
            role,
            content: vec![ContentPart::Text { text: text.into() }],
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ChatRequest {
    pub messages: Vec<ChatMessage>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_schema: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ChatCompletion<T> {
    pub model: String,
    pub content: T,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw_response: Option<Value>,
}

#[derive(Debug, Error)]
pub enum LlmError {
    #[error("provider error: {0}")]
    Provider(String),
    #[error("rate limited: {0}")]
    RateLimited(String),
    #[error("invalid structured output: {0}")]
    InvalidStructuredOutput(String),
}

#[async_trait]
pub trait ChatModel: Send + Sync {
    fn provider(&self) -> &str;

    fn model(&self) -> &str;

    async fn invoke_json(&self, request: ChatRequest) -> Result<ChatCompletion<Value>, LlmError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_message_uses_content_parts() {
        let message = ChatMessage::text(MessageRole::User, "hello");

        assert_eq!(
            message.content,
            vec![ContentPart::Text {
                text: "hello".to_owned()
            }]
        );
    }
}
