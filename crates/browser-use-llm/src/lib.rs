//! LLM provider contracts for schema-guided agent calls.

use async_trait::async_trait;
use reqwest::StatusCode;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
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

#[derive(Clone)]
pub struct OpenAiCompatibleChatModel {
    api_key: String,
    model: String,
    base_url: String,
    schema_name: String,
    client: reqwest::Client,
}

impl OpenAiCompatibleChatModel {
    #[must_use]
    pub fn new(api_key: impl Into<String>, model: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            model: model.into(),
            base_url: "https://api.openai.com/v1".to_owned(),
            schema_name: "agent_output".to_owned(),
            client: reqwest::Client::new(),
        }
    }

    pub fn from_env(model: impl Into<String>) -> Result<Self, LlmError> {
        let api_key = std::env::var("OPENAI_API_KEY")
            .map_err(|_| LlmError::Provider("OPENAI_API_KEY is not set".to_owned()))?;
        Ok(Self::new(api_key, model))
    }

    #[must_use]
    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into().trim_end_matches('/').to_owned();
        self
    }

    #[must_use]
    pub fn with_schema_name(mut self, schema_name: impl Into<String>) -> Self {
        self.schema_name = schema_name.into();
        self
    }

    fn chat_completions_url(&self) -> String {
        format!("{}/chat/completions", self.base_url)
    }
}

#[async_trait]
impl ChatModel for OpenAiCompatibleChatModel {
    fn provider(&self) -> &str {
        "openai-compatible"
    }

    fn model(&self) -> &str {
        &self.model
    }

    async fn invoke_json(&self, request: ChatRequest) -> Result<ChatCompletion<Value>, LlmError> {
        let payload = openai_chat_payload(&self.model, &self.schema_name, request);
        let response = self
            .client
            .post(self.chat_completions_url())
            .bearer_auth(&self.api_key)
            .json(&payload)
            .send()
            .await
            .map_err(|error| LlmError::Provider(error.to_string()))?;
        let status = response.status();
        let raw_response = response
            .json::<Value>()
            .await
            .map_err(|error| LlmError::Provider(error.to_string()))?;

        if !status.is_success() {
            let message = raw_response
                .pointer("/error/message")
                .and_then(Value::as_str)
                .map_or_else(|| raw_response.to_string(), ToOwned::to_owned);
            return if status == StatusCode::TOO_MANY_REQUESTS {
                Err(LlmError::RateLimited(message))
            } else {
                Err(LlmError::Provider(format!("HTTP {status}: {message}")))
            };
        }

        let content = parse_openai_chat_completion(&raw_response)?;
        Ok(ChatCompletion {
            model: raw_response
                .get("model")
                .and_then(Value::as_str)
                .unwrap_or(&self.model)
                .to_owned(),
            content,
            raw_response: Some(raw_response),
        })
    }
}

fn openai_chat_payload(model: &str, schema_name: &str, request: ChatRequest) -> Value {
    let messages: Vec<Value> = request.messages.into_iter().map(openai_message).collect();
    let mut payload = json!({
        "model": model,
        "messages": messages,
    });

    if let Some(schema) = request.output_schema {
        payload["response_format"] = json!({
            "type": "json_schema",
            "json_schema": {
                "name": schema_name,
                "strict": true,
                "schema": schema,
            },
        });
    }

    payload
}

fn openai_message(message: ChatMessage) -> Value {
    json!({
        "role": openai_role(&message.role),
        "content": openai_content(message.content),
    })
}

fn openai_role(role: &MessageRole) -> &'static str {
    match role {
        MessageRole::System => "system",
        MessageRole::User => "user",
        MessageRole::Assistant => "assistant",
        MessageRole::Tool => "tool",
    }
}

fn openai_content(parts: Vec<ContentPart>) -> Value {
    if let [ContentPart::Text { text }] = parts.as_slice() {
        return Value::String(text.clone());
    }

    Value::Array(
        parts
            .into_iter()
            .map(|part| match part {
                ContentPart::Text { text } => json!({
                    "type": "text",
                    "text": text,
                }),
                ContentPart::ImageUrl { image_url } => json!({
                    "type": "image_url",
                    "image_url": {
                        "url": image_url,
                    },
                }),
            })
            .collect(),
    )
}

fn parse_openai_chat_completion(raw_response: &Value) -> Result<Value, LlmError> {
    let message = raw_response
        .pointer("/choices/0/message")
        .ok_or_else(|| LlmError::Provider("missing chat completion message".to_owned()))?;

    if let Some(refusal) = message.get("refusal").and_then(Value::as_str) {
        return Err(LlmError::Provider(format!(
            "model refused request: {refusal}"
        )));
    }

    let content = message
        .get("content")
        .ok_or_else(|| LlmError::Provider("missing chat completion content".to_owned()))?;

    match content {
        Value::String(text) => serde_json::from_str(text)
            .map_err(|error| LlmError::InvalidStructuredOutput(error.to_string())),
        Value::Array(_) | Value::Object(_) => Ok(content.clone()),
        _ => Err(LlmError::InvalidStructuredOutput(
            "chat completion content was not JSON-compatible".to_owned(),
        )),
    }
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

    #[test]
    fn openai_payload_uses_structured_outputs_format() {
        let payload = openai_chat_payload(
            "gpt-test",
            "agent_output",
            ChatRequest {
                messages: vec![ChatMessage::text(MessageRole::User, "Return JSON")],
                output_schema: Some(json!({
                    "type": "object",
                    "properties": {
                        "ok": { "type": "boolean" }
                    },
                    "required": ["ok"]
                })),
            },
        );

        assert_eq!(payload["model"], "gpt-test");
        assert_eq!(payload["messages"][0]["role"], "user");
        assert_eq!(payload["messages"][0]["content"], "Return JSON");
        assert_eq!(payload["response_format"]["type"], "json_schema");
        assert_eq!(
            payload["response_format"]["json_schema"]["schema"]["properties"]["ok"]["type"],
            "boolean"
        );
        assert_eq!(payload["response_format"]["json_schema"]["strict"], true);
    }

    #[test]
    fn openai_payload_preserves_multimodal_content_parts() {
        let payload = openai_chat_payload(
            "gpt-test",
            "agent_output",
            ChatRequest {
                messages: vec![ChatMessage {
                    role: MessageRole::User,
                    content: vec![
                        ContentPart::Text {
                            text: "what changed?".to_owned(),
                        },
                        ContentPart::ImageUrl {
                            image_url: "data:image/png;base64,abc".to_owned(),
                        },
                    ],
                }],
                output_schema: None,
            },
        );

        assert_eq!(payload["messages"][0]["content"][0]["type"], "text");
        assert_eq!(payload["messages"][0]["content"][1]["type"], "image_url");
    }

    #[test]
    fn parses_stringified_json_chat_completion() {
        let raw = json!({
            "model": "gpt-test",
            "choices": [
                {
                    "message": {
                        "role": "assistant",
                        "content": "{\"ok\":true}"
                    }
                }
            ]
        });

        let parsed = parse_openai_chat_completion(&raw).expect("parse completion");

        assert_eq!(parsed, json!({ "ok": true }));
    }
}
