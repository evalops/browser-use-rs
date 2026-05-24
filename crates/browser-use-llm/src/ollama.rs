use async_trait::async_trait;
use reqwest::StatusCode;
use serde_json::{Value, json};

use crate::common::{data_url_image_source, parse_json_object_text};
use crate::{
    ChatCompletion, ChatMessage, ChatModel, ChatRequest, ContentPart, LlmError, MessageRole,
};

/// Chat model for a local or remote Ollama chat endpoint.
#[derive(Clone)]
pub struct OllamaChatModel {
    model: String,
    base_url: String,
    client: reqwest::Client,
}

impl OllamaChatModel {
    /// Creates an Ollama model using the default local base URL.
    #[must_use]
    pub fn new(model: impl Into<String>) -> Self {
        Self {
            model: model.into(),
            base_url: "http://localhost:11434".to_owned(),
            client: reqwest::Client::new(),
        }
    }

    /// Overrides the Ollama base URL.
    #[must_use]
    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into().trim_end_matches('/').to_owned();
        self
    }

    fn chat_url(&self) -> String {
        format!("{}/api/chat", self.base_url)
    }
}

#[async_trait]
impl ChatModel for OllamaChatModel {
    fn provider(&self) -> &str {
        "ollama"
    }

    fn model(&self) -> &str {
        &self.model
    }

    async fn invoke_json(&self, request: ChatRequest) -> Result<ChatCompletion<Value>, LlmError> {
        let payload = ollama_chat_payload(&self.model, request);
        let response = self
            .client
            .post(self.chat_url())
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
                .get("error")
                .and_then(Value::as_str)
                .map_or_else(|| raw_response.to_string(), ToOwned::to_owned);
            return if status == StatusCode::TOO_MANY_REQUESTS {
                Err(LlmError::RateLimited(message))
            } else {
                Err(LlmError::Provider(format!("HTTP {status}: {message}")))
            };
        }

        let content = parse_ollama_chat_response(&raw_response)?;
        Ok(ChatCompletion {
            model: raw_response
                .get("model")
                .and_then(Value::as_str)
                .unwrap_or(&self.model)
                .to_owned(),
            content,
            usage: None,
            raw_response: Some(raw_response),
        })
    }
}

pub(crate) fn ollama_chat_payload(model: &str, request: ChatRequest) -> Value {
    let messages: Vec<Value> = request.messages.into_iter().map(ollama_message).collect();
    let mut payload = json!({
        "model": model,
        "messages": messages,
        "stream": false,
    });

    if let Some(schema) = request.output_schema {
        payload["format"] = schema;
    }

    payload
}

fn ollama_message(message: ChatMessage) -> Value {
    let mut value = json!({
        "role": ollama_role(&message.role),
        "content": ollama_text_content(&message.content),
    });

    let images: Vec<String> = message
        .content
        .into_iter()
        .filter_map(ollama_image_part)
        .collect();
    if !images.is_empty() {
        value["images"] = Value::Array(images.into_iter().map(Value::String).collect());
    }

    value
}

fn ollama_role(role: &MessageRole) -> &'static str {
    match role {
        MessageRole::System => "system",
        MessageRole::Assistant => "assistant",
        MessageRole::User | MessageRole::Tool => "user",
    }
}

fn ollama_text_content(parts: &[ContentPart]) -> String {
    parts
        .iter()
        .filter_map(|part| match part {
            ContentPart::Text { text } => Some(text.as_str()),
            ContentPart::ImageUrl { .. } => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn ollama_image_part(part: ContentPart) -> Option<String> {
    match part {
        ContentPart::ImageUrl { image_url, .. } => {
            if let Some((_media_type, data)) = data_url_image_source(&image_url) {
                Some(data)
            } else {
                Some(image_url)
            }
        }
        ContentPart::Text { .. } => None,
    }
}

pub(crate) fn parse_ollama_chat_response(raw_response: &Value) -> Result<Value, LlmError> {
    let content = raw_response
        .pointer("/message/content")
        .and_then(Value::as_str)
        .ok_or_else(|| LlmError::Provider("missing Ollama message content".to_owned()))?;

    parse_json_object_text(content, "Ollama message content")
}
