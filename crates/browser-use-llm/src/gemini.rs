use async_trait::async_trait;
use reqwest::StatusCode;
use serde_json::{Value, json};

use crate::common::{
    append_json_schema_instruction, data_url_image_source, json_u64, parse_json_object_text,
    text_content_part,
};
use crate::{
    ChatCompletion, ChatMessage, ChatModel, ChatRequest, ChatUsage, ContentPart, LlmError,
    MessageRole,
};

/// Chat model for Google's Gemini `generateContent` API.
#[derive(Clone)]
pub struct GeminiChatModel {
    api_key: String,
    model: String,
    base_url: String,
    supports_structured_output: bool,
    client: reqwest::Client,
}

impl GeminiChatModel {
    /// Creates a Gemini model from an API key and model id.
    #[must_use]
    pub fn new(api_key: impl Into<String>, model: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            model: model.into(),
            base_url: "https://generativelanguage.googleapis.com/v1beta".to_owned(),
            supports_structured_output: true,
            client: reqwest::Client::new(),
        }
    }

    /// Creates a Gemini model from `GEMINI_API_KEY`.
    pub fn from_env(model: impl Into<String>) -> Result<Self, LlmError> {
        let api_key = std::env::var("GEMINI_API_KEY")
            .map_err(|_| LlmError::Provider("GEMINI_API_KEY is not set".to_owned()))?;
        Ok(Self::new(api_key, model))
    }

    /// Overrides the Gemini API base URL.
    #[must_use]
    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into().trim_end_matches('/').to_owned();
        self
    }

    /// Enables or disables Gemini's native structured-output configuration.
    #[must_use]
    pub fn with_structured_output_support(mut self, supports_structured_output: bool) -> Self {
        self.supports_structured_output = supports_structured_output;
        self
    }

    fn generate_content_url(&self) -> String {
        if self.model.starts_with("models/") {
            format!("{}/{}:generateContent", self.base_url, self.model)
        } else {
            format!("{}/models/{}:generateContent", self.base_url, self.model)
        }
    }
}

#[async_trait]
impl ChatModel for GeminiChatModel {
    fn provider(&self) -> &str {
        "gemini"
    }

    fn model(&self) -> &str {
        &self.model
    }

    async fn invoke_json(&self, request: ChatRequest) -> Result<ChatCompletion<Value>, LlmError> {
        let payload =
            gemini_generate_content_payload_with_mode(request, self.supports_structured_output);
        let response = self
            .client
            .post(self.generate_content_url())
            .header("x-goog-api-key", &self.api_key)
            .header("x-goog-api-client", gemini_api_client_header())
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

        let content = parse_gemini_generate_content(&raw_response)?;
        Ok(ChatCompletion {
            model: raw_response
                .get("modelVersion")
                .and_then(Value::as_str)
                .unwrap_or(&self.model)
                .to_owned(),
            content,
            usage: parse_gemini_usage(&raw_response),
            raw_response: Some(raw_response),
        })
    }
}

#[must_use]
pub(crate) fn gemini_api_client_header() -> String {
    format!("browser-use-rs/{}", env!("CARGO_PKG_VERSION"))
}

#[cfg(test)]
pub(crate) fn gemini_generate_content_payload(request: ChatRequest) -> Value {
    gemini_generate_content_payload_with_mode(request, true)
}

pub(crate) fn gemini_generate_content_payload_with_mode(
    mut request: ChatRequest,
    supports_structured_output: bool,
) -> Value {
    let output_schema = request.output_schema.take();
    if !supports_structured_output {
        if let Some(schema) = output_schema.as_ref() {
            append_json_schema_instruction(&mut request.messages, schema);
        }
    }

    let mut system_parts = Vec::new();
    let mut contents = Vec::new();

    for message in request.messages {
        if message.role == MessageRole::System {
            system_parts.extend(message.content.into_iter().filter_map(text_content_part));
        } else {
            contents.push(gemini_content(message));
        }
    }

    let mut payload = json!({
        "contents": contents,
    });

    if !system_parts.is_empty() {
        payload["systemInstruction"] = json!({
            "parts": [
                {
                    "text": system_parts.join("\n\n")
                }
            ]
        });
    }

    if let Some(schema) = output_schema.filter(|_| supports_structured_output) {
        payload["generationConfig"] = json!({
            "responseFormat": {
                "text": {
                    "mimeType": "application/json",
                    "schema": schema,
                }
            }
        });
    }

    payload
}

fn gemini_content(message: ChatMessage) -> Value {
    let role = match message.role {
        MessageRole::Assistant => "model",
        MessageRole::User | MessageRole::Tool | MessageRole::System => "user",
    };
    json!({
        "role": role,
        "parts": message.content.into_iter().map(gemini_part).collect::<Vec<_>>(),
    })
}

fn gemini_part(part: ContentPart) -> Value {
    match part {
        ContentPart::Text { text } => json!({
            "text": text,
        }),
        ContentPart::ImageUrl { image_url, .. } => match data_url_image_source(&image_url) {
            Some((media_type, data)) => json!({
                "inlineData": {
                    "mimeType": media_type,
                    "data": data,
                }
            }),
            None => json!({
                "text": format!("[image_url: {image_url}]"),
            }),
        },
    }
}

pub(crate) fn parse_gemini_generate_content(raw_response: &Value) -> Result<Value, LlmError> {
    if let Some(finish_reason) = raw_response
        .pointer("/candidates/0/finishReason")
        .and_then(Value::as_str)
        .filter(|reason| matches!(*reason, "SAFETY" | "RECITATION" | "PROHIBITED_CONTENT"))
    {
        return Err(LlmError::Provider(format!(
            "Gemini response stopped with {finish_reason}"
        )));
    }

    let text = raw_response
        .pointer("/candidates/0/content/parts")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .find_map(|part| part.get("text").and_then(Value::as_str))
        .ok_or_else(|| LlmError::Provider("missing Gemini text content".to_owned()))?;

    parse_json_object_text(text, "Gemini text content")
}

pub(crate) fn parse_gemini_usage(raw_response: &Value) -> Option<ChatUsage> {
    let usage = raw_response.get("usageMetadata")?;
    let prompt_tokens = json_u64(usage.get("promptTokenCount"))?;
    let completion_tokens = json_u64(usage.get("candidatesTokenCount")).unwrap_or(0)
        + json_u64(usage.get("thoughtsTokenCount")).unwrap_or(0);

    Some(ChatUsage {
        prompt_tokens,
        prompt_cached_tokens: json_u64(usage.get("cachedContentTokenCount")),
        prompt_cache_creation_tokens: None,
        prompt_image_tokens: gemini_prompt_image_tokens(usage),
        completion_tokens,
        total_tokens: json_u64(usage.get("totalTokenCount"))
            .unwrap_or(prompt_tokens + completion_tokens),
    })
}

fn gemini_prompt_image_tokens(usage: &Value) -> Option<u64> {
    let total = usage
        .get("promptTokensDetails")
        .and_then(Value::as_array)?
        .iter()
        .filter(|detail| detail.get("modality").and_then(Value::as_str) == Some("IMAGE"))
        .filter_map(|detail| json_u64(detail.get("tokenCount")))
        .sum::<u64>();
    (total > 0).then_some(total)
}
