use async_trait::async_trait;
use reqwest::StatusCode;
use serde_json::{Value, json};

use crate::common::{
    data_url_image_source, json_u64, parse_json_object_compatible, parse_json_object_text,
    text_content_part,
};
use crate::{
    ChatCompletion, ChatMessage, ChatModel, ChatRequest, ChatUsage, ContentPart, LlmError,
    MessageRole,
};

/// Chat model for Anthropic's Messages API.
#[derive(Clone)]
pub struct AnthropicChatModel {
    api_key: String,
    model: String,
    base_url: String,
    anthropic_version: String,
    max_tokens: u32,
    client: reqwest::Client,
}

impl AnthropicChatModel {
    /// Creates an Anthropic model from an API key and model id.
    #[must_use]
    pub fn new(api_key: impl Into<String>, model: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            model: model.into(),
            base_url: "https://api.anthropic.com/v1".to_owned(),
            anthropic_version: "2023-06-01".to_owned(),
            max_tokens: 4096,
            client: reqwest::Client::new(),
        }
    }

    /// Creates an Anthropic model from `ANTHROPIC_API_KEY`.
    pub fn from_env(model: impl Into<String>) -> Result<Self, LlmError> {
        let api_key = std::env::var("ANTHROPIC_API_KEY")
            .map_err(|_| LlmError::Provider("ANTHROPIC_API_KEY is not set".to_owned()))?;
        Ok(Self::new(api_key, model))
    }

    /// Overrides the Anthropic API base URL.
    #[must_use]
    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into().trim_end_matches('/').to_owned();
        self
    }

    /// Overrides the `anthropic-version` request header.
    #[must_use]
    pub fn with_anthropic_version(mut self, anthropic_version: impl Into<String>) -> Self {
        self.anthropic_version = anthropic_version.into();
        self
    }

    /// Sets the maximum output tokens requested from Anthropic.
    #[must_use]
    pub fn with_max_tokens(mut self, max_tokens: u32) -> Self {
        self.max_tokens = max_tokens;
        self
    }

    fn messages_url(&self) -> String {
        format!("{}/messages", self.base_url)
    }
}

#[async_trait]
impl ChatModel for AnthropicChatModel {
    fn provider(&self) -> &str {
        "anthropic"
    }

    fn model(&self) -> &str {
        &self.model
    }

    async fn invoke_json(&self, request: ChatRequest) -> Result<ChatCompletion<Value>, LlmError> {
        let payload = anthropic_messages_payload(&self.model, self.max_tokens, request);
        let response = self
            .client
            .post(self.messages_url())
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", &self.anthropic_version)
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

        let content = parse_anthropic_message(&raw_response)?;
        Ok(ChatCompletion {
            model: raw_response
                .get("model")
                .and_then(Value::as_str)
                .unwrap_or(&self.model)
                .to_owned(),
            content,
            usage: parse_anthropic_usage(&raw_response),
            raw_response: Some(raw_response),
        })
    }
}

pub(crate) fn parse_anthropic_usage(raw_response: &Value) -> Option<ChatUsage> {
    let usage = raw_response.get("usage")?;
    let input_tokens = json_u64(usage.get("input_tokens"))?;
    let output_tokens = json_u64(usage.get("output_tokens"))?;
    let cached_tokens = json_u64(usage.get("cache_read_input_tokens"));

    Some(ChatUsage {
        prompt_tokens: input_tokens + cached_tokens.unwrap_or(0),
        prompt_cached_tokens: cached_tokens,
        prompt_cache_creation_tokens: json_u64(usage.get("cache_creation_input_tokens")),
        prompt_image_tokens: None,
        completion_tokens: output_tokens,
        total_tokens: input_tokens + output_tokens,
    })
}

pub(crate) fn anthropic_messages_payload(
    model: &str,
    max_tokens: u32,
    request: ChatRequest,
) -> Value {
    let mut system_parts = Vec::new();
    let mut messages = Vec::new();

    for message in request.messages {
        if message.role == MessageRole::System {
            system_parts.extend(message.content.into_iter().filter_map(text_content_part));
        } else {
            messages.push(anthropic_message(message));
        }
    }

    let mut payload = json!({
        "model": model,
        "max_tokens": max_tokens,
        "messages": messages,
    });

    if !system_parts.is_empty() {
        payload["system"] = Value::String(system_parts.join("\n\n"));
    }

    if let Some(schema) = request.output_schema {
        let mut input_schema = schema;
        if let Some(object) = input_schema.as_object_mut() {
            object.remove("title");
        }
        payload["tools"] = json!([
            {
                "name": "agent_output",
                "description": "Extract information in the format of agent_output",
                "input_schema": input_schema,
                "cache_control": {
                    "type": "ephemeral"
                }
            }
        ]);
        payload["tool_choice"] = json!({
            "type": "tool",
            "name": "agent_output"
        });
    }

    payload
}

fn anthropic_message(message: ChatMessage) -> Value {
    let role = match message.role {
        MessageRole::Assistant => "assistant",
        MessageRole::User | MessageRole::Tool | MessageRole::System => "user",
    };
    json!({
        "role": role,
        "content": anthropic_content(message.content),
    })
}

fn anthropic_content(parts: Vec<ContentPart>) -> Value {
    Value::Array(parts.into_iter().map(anthropic_content_part).collect())
}

fn anthropic_content_part(part: ContentPart) -> Value {
    match part {
        ContentPart::Text { text } => json!({
            "type": "text",
            "text": text,
        }),
        ContentPart::ImageUrl { image_url, .. } => match data_url_image_source(&image_url) {
            Some((media_type, data)) => json!({
                "type": "image",
                "source": {
                    "type": "base64",
                    "media_type": media_type,
                    "data": data,
                }
            }),
            None => json!({
                "type": "text",
                "text": format!("[image_url: {image_url}]"),
            }),
        },
    }
}

pub(crate) fn parse_anthropic_message(raw_response: &Value) -> Result<Value, LlmError> {
    match raw_response.get("stop_reason").and_then(Value::as_str) {
        Some("refusal") => return Err(LlmError::Provider("model refused request".to_owned())),
        Some("max_tokens") => {
            return Err(LlmError::Provider(
                "Anthropic response reached max_tokens before completing JSON".to_owned(),
            ));
        }
        _ => {}
    }

    if let Some(input) = raw_response
        .get("content")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .find(|part| part.get("type").and_then(Value::as_str) == Some("tool_use"))
        .and_then(|part| part.get("input"))
    {
        return parse_json_object_compatible(input, "Anthropic tool input");
    }

    let text = raw_response
        .get("content")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .find(|part| part.get("type").and_then(Value::as_str) == Some("text"))
        .and_then(|part| part.get("text").and_then(Value::as_str))
        .ok_or_else(|| {
            LlmError::Provider("missing Anthropic tool use or text content".to_owned())
        })?;

    parse_json_object_text(text, "Anthropic text content")
}
