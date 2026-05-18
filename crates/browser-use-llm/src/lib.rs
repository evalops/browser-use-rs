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

#[async_trait]
impl<T> ChatModel for Box<T>
where
    T: ChatModel + ?Sized,
{
    fn provider(&self) -> &str {
        self.as_ref().provider()
    }

    fn model(&self) -> &str {
        self.as_ref().model()
    }

    async fn invoke_json(&self, request: ChatRequest) -> Result<ChatCompletion<Value>, LlmError> {
        self.as_ref().invoke_json(request).await
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpenAiStructuredOutputMode {
    JsonSchema,
    JsonObject,
    PromptOnly,
    ToolCall,
}

#[derive(Clone)]
pub struct OpenAiCompatibleChatModel {
    api_key: String,
    model: String,
    base_url: String,
    provider_name: String,
    schema_name: String,
    structured_output_mode: OpenAiStructuredOutputMode,
    client: reqwest::Client,
}

#[derive(Clone)]
pub struct AnthropicChatModel {
    api_key: String,
    model: String,
    base_url: String,
    anthropic_version: String,
    max_tokens: u32,
    client: reqwest::Client,
}

#[derive(Clone)]
pub struct GeminiChatModel {
    api_key: String,
    model: String,
    base_url: String,
    client: reqwest::Client,
}

#[derive(Clone)]
pub struct OllamaChatModel {
    model: String,
    base_url: String,
    client: reqwest::Client,
}

impl OllamaChatModel {
    #[must_use]
    pub fn new(model: impl Into<String>) -> Self {
        Self {
            model: model.into(),
            base_url: "http://localhost:11434".to_owned(),
            client: reqwest::Client::new(),
        }
    }

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
            raw_response: Some(raw_response),
        })
    }
}

impl GeminiChatModel {
    #[must_use]
    pub fn new(api_key: impl Into<String>, model: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            model: model.into(),
            base_url: "https://generativelanguage.googleapis.com/v1beta".to_owned(),
            client: reqwest::Client::new(),
        }
    }

    pub fn from_env(model: impl Into<String>) -> Result<Self, LlmError> {
        let api_key = std::env::var("GEMINI_API_KEY")
            .map_err(|_| LlmError::Provider("GEMINI_API_KEY is not set".to_owned()))?;
        Ok(Self::new(api_key, model))
    }

    #[must_use]
    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into().trim_end_matches('/').to_owned();
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
        let payload = gemini_generate_content_payload(request);
        let response = self
            .client
            .post(self.generate_content_url())
            .header("x-goog-api-key", &self.api_key)
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
            raw_response: Some(raw_response),
        })
    }
}

impl AnthropicChatModel {
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

    pub fn from_env(model: impl Into<String>) -> Result<Self, LlmError> {
        let api_key = std::env::var("ANTHROPIC_API_KEY")
            .map_err(|_| LlmError::Provider("ANTHROPIC_API_KEY is not set".to_owned()))?;
        Ok(Self::new(api_key, model))
    }

    #[must_use]
    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into().trim_end_matches('/').to_owned();
        self
    }

    #[must_use]
    pub fn with_anthropic_version(mut self, anthropic_version: impl Into<String>) -> Self {
        self.anthropic_version = anthropic_version.into();
        self
    }

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
            raw_response: Some(raw_response),
        })
    }
}

impl OpenAiCompatibleChatModel {
    #[must_use]
    pub fn new(api_key: impl Into<String>, model: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            model: model.into(),
            base_url: "https://api.openai.com/v1".to_owned(),
            provider_name: "openai-compatible".to_owned(),
            schema_name: "agent_output".to_owned(),
            structured_output_mode: OpenAiStructuredOutputMode::JsonSchema,
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
    pub fn with_provider_name(mut self, provider_name: impl Into<String>) -> Self {
        self.provider_name = provider_name.into();
        self
    }

    #[must_use]
    pub fn with_schema_name(mut self, schema_name: impl Into<String>) -> Self {
        self.schema_name = schema_name.into();
        self
    }

    #[must_use]
    pub fn with_structured_output_mode(
        mut self,
        structured_output_mode: OpenAiStructuredOutputMode,
    ) -> Self {
        self.structured_output_mode = structured_output_mode;
        self
    }

    fn chat_completions_url(&self) -> String {
        format!("{}/chat/completions", self.base_url)
    }
}

#[async_trait]
impl ChatModel for OpenAiCompatibleChatModel {
    fn provider(&self) -> &str {
        &self.provider_name
    }

    fn model(&self) -> &str {
        &self.model
    }

    async fn invoke_json(&self, request: ChatRequest) -> Result<ChatCompletion<Value>, LlmError> {
        let payload = openai_chat_payload(
            &self.model,
            &self.schema_name,
            self.structured_output_mode,
            request,
        );
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

fn openai_chat_payload(
    model: &str,
    schema_name: &str,
    structured_output_mode: OpenAiStructuredOutputMode,
    mut request: ChatRequest,
) -> Value {
    let output_schema = request.output_schema.take();
    if matches!(
        structured_output_mode,
        OpenAiStructuredOutputMode::JsonObject | OpenAiStructuredOutputMode::PromptOnly
    ) {
        if let Some(schema) = output_schema.as_ref() {
            append_json_schema_instruction(&mut request.messages, schema);
        }
    }

    let messages: Vec<Value> = request.messages.into_iter().map(openai_message).collect();
    let mut payload = json!({
        "model": model,
        "messages": messages,
    });

    if let Some(schema) = output_schema {
        match structured_output_mode {
            OpenAiStructuredOutputMode::JsonSchema => {
                payload["response_format"] = json!({
                    "type": "json_schema",
                    "json_schema": {
                        "name": schema_name,
                        "strict": true,
                        "schema": schema,
                    },
                });
            }
            OpenAiStructuredOutputMode::JsonObject => {
                payload["response_format"] = json!({ "type": "json_object" });
            }
            OpenAiStructuredOutputMode::ToolCall => {
                payload["tools"] = json!([
                    {
                        "type": "function",
                        "function": {
                            "name": schema_name,
                            "description": "Return the structured browser-use agent output.",
                            "parameters": schema,
                            "strict": true,
                        }
                    }
                ]);
                payload["tool_choice"] = json!({
                    "type": "function",
                    "function": {
                        "name": schema_name,
                    }
                });
            }
            OpenAiStructuredOutputMode::PromptOnly => {}
        }
    }

    payload
}

fn append_json_schema_instruction(messages: &mut Vec<ChatMessage>, schema: &Value) {
    let schema_text = serde_json::to_string_pretty(schema).unwrap_or_else(|_| schema.to_string());
    let instruction =
        format!("\n\nReturn only a valid JSON object that matches this schema:\n{schema_text}");

    if let Some(message) = messages
        .iter_mut()
        .rev()
        .find(|message| matches!(message.role, MessageRole::User | MessageRole::System))
    {
        append_text_to_message(message, instruction);
    } else {
        messages.push(ChatMessage::text(MessageRole::User, instruction));
    }
}

fn append_text_to_message(message: &mut ChatMessage, text: String) {
    if let Some(ContentPart::Text { text: existing }) = message.content.last_mut() {
        existing.push_str(&text);
    } else {
        message.content.push(ContentPart::Text { text });
    }
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

fn ollama_chat_payload(model: &str, request: ChatRequest) -> Value {
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
        ContentPart::ImageUrl { image_url } => {
            if let Some((_media_type, data)) = data_url_image_source(&image_url) {
                Some(data)
            } else {
                Some(image_url)
            }
        }
        ContentPart::Text { .. } => None,
    }
}

fn gemini_generate_content_payload(request: ChatRequest) -> Value {
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

    if let Some(schema) = request.output_schema {
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
        ContentPart::ImageUrl { image_url } => match data_url_image_source(&image_url) {
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

fn parse_openai_chat_completion(raw_response: &Value) -> Result<Value, LlmError> {
    if let Some(finish_reason) = raw_response
        .pointer("/choices/0/finish_reason")
        .and_then(Value::as_str)
        .filter(|reason| matches!(*reason, "length" | "content_filter"))
    {
        return Err(LlmError::Provider(format!(
            "chat completion stopped with {finish_reason} before completing structured output"
        )));
    }

    let message = raw_response
        .pointer("/choices/0/message")
        .ok_or_else(|| LlmError::Provider("missing chat completion message".to_owned()))?;

    if let Some(refusal) = message.get("refusal").and_then(Value::as_str) {
        return Err(LlmError::Provider(format!(
            "model refused request: {refusal}"
        )));
    }

    if message.get("tool_calls").is_some() || message.get("function_call").is_some() {
        let arguments = openai_tool_call_arguments(message).ok_or_else(|| {
            LlmError::Provider("missing chat completion tool call arguments".to_owned())
        })?;

        return parse_json_object_compatible(arguments, "chat completion tool call arguments");
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

fn openai_tool_call_arguments(message: &Value) -> Option<&Value> {
    message
        .get("tool_calls")
        .and_then(Value::as_array)
        .and_then(|tool_calls| tool_calls.first())
        .and_then(|tool_call| tool_call.pointer("/function/arguments"))
        .or_else(|| message.pointer("/function_call/arguments"))
}

fn parse_json_object_compatible(value: &Value, source: &str) -> Result<Value, LlmError> {
    let parsed = match value {
        Value::String(text) => serde_json::from_str(text)
            .map_err(|error| LlmError::InvalidStructuredOutput(format!("{source}: {error}")))?,
        Value::Object(_) => value.clone(),
        _ => {
            return Err(LlmError::InvalidStructuredOutput(format!(
                "{source} were not a JSON object"
            )));
        }
    };

    if parsed.is_object() {
        Ok(parsed)
    } else {
        Err(LlmError::InvalidStructuredOutput(format!(
            "{source} were not a JSON object"
        )))
    }
}

fn parse_ollama_chat_response(raw_response: &Value) -> Result<Value, LlmError> {
    let content = raw_response
        .pointer("/message/content")
        .and_then(Value::as_str)
        .ok_or_else(|| LlmError::Provider("missing Ollama message content".to_owned()))?;

    serde_json::from_str(content)
        .map_err(|error| LlmError::InvalidStructuredOutput(error.to_string()))
}

fn parse_gemini_generate_content(raw_response: &Value) -> Result<Value, LlmError> {
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

    serde_json::from_str(text).map_err(|error| LlmError::InvalidStructuredOutput(error.to_string()))
}

fn anthropic_messages_payload(model: &str, max_tokens: u32, request: ChatRequest) -> Value {
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
        payload["output_config"] = json!({
            "format": {
                "type": "json_schema",
                "schema": schema,
            }
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
        ContentPart::ImageUrl { image_url } => match data_url_image_source(&image_url) {
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

fn text_content_part(part: ContentPart) -> Option<String> {
    match part {
        ContentPart::Text { text } => Some(text),
        ContentPart::ImageUrl { image_url } => Some(format!("[image_url: {image_url}]")),
    }
}

fn data_url_image_source(image_url: &str) -> Option<(String, String)> {
    let rest = image_url.strip_prefix("data:")?;
    let (media_type, data) = rest.split_once(";base64,")?;
    if !media_type.starts_with("image/") || data.is_empty() {
        return None;
    }
    Some((media_type.to_owned(), data.to_owned()))
}

fn parse_anthropic_message(raw_response: &Value) -> Result<Value, LlmError> {
    match raw_response.get("stop_reason").and_then(Value::as_str) {
        Some("refusal") => return Err(LlmError::Provider("model refused request".to_owned())),
        Some("max_tokens") => {
            return Err(LlmError::Provider(
                "Anthropic response reached max_tokens before completing JSON".to_owned(),
            ));
        }
        _ => {}
    }

    let text = raw_response
        .get("content")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .find(|part| part.get("type").and_then(Value::as_str) == Some("text"))
        .and_then(|part| part.get("text").and_then(Value::as_str))
        .ok_or_else(|| LlmError::Provider("missing Anthropic text content".to_owned()))?;

    serde_json::from_str(text).map_err(|error| LlmError::InvalidStructuredOutput(error.to_string()))
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
    fn openai_compatible_model_reports_provider_alias() {
        let model =
            OpenAiCompatibleChatModel::new("test-key", "test-model").with_provider_name("deepseek");

        assert_eq!(model.provider(), "deepseek");
        assert_eq!(model.model(), "test-model");
    }

    #[test]
    fn openai_payload_uses_structured_outputs_format() {
        let payload = openai_chat_payload(
            "gpt-test",
            "agent_output",
            OpenAiStructuredOutputMode::JsonSchema,
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
    fn openai_json_object_mode_embeds_schema_instruction() {
        let payload = openai_chat_payload(
            "deepseek-test",
            "agent_output",
            OpenAiStructuredOutputMode::JsonObject,
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

        let content = payload["messages"][0]["content"].as_str().expect("content");
        assert_eq!(payload["response_format"]["type"], "json_object");
        assert!(content.contains("Return JSON"));
        assert!(content.contains("valid JSON object"));
        assert!(content.contains("\"ok\""));
    }

    #[test]
    fn openai_prompt_only_mode_omits_response_format() {
        let payload = openai_chat_payload(
            "cerebras-test",
            "agent_output",
            OpenAiStructuredOutputMode::PromptOnly,
            ChatRequest {
                messages: vec![ChatMessage::text(MessageRole::User, "Extract the result")],
                output_schema: Some(json!({
                    "type": "object",
                    "properties": {
                        "answer": { "type": "string" }
                    },
                    "required": ["answer"]
                })),
            },
        );

        let content = payload["messages"][0]["content"].as_str().expect("content");
        assert!(payload.get("response_format").is_none());
        assert!(content.contains("Extract the result"));
        assert!(content.contains("\"answer\""));
    }

    #[test]
    fn openai_tool_call_mode_uses_strict_function_schema() {
        let payload = openai_chat_payload(
            "tool-call-test",
            "agent_output",
            OpenAiStructuredOutputMode::ToolCall,
            ChatRequest {
                messages: vec![ChatMessage::text(MessageRole::User, "Return a tool call")],
                output_schema: Some(json!({
                    "type": "object",
                    "properties": {
                        "ok": { "type": "boolean" }
                    },
                    "required": ["ok"]
                })),
            },
        );

        assert!(payload.get("response_format").is_none());
        assert_eq!(payload["tools"][0]["type"], "function");
        assert_eq!(payload["tools"][0]["function"]["name"], "agent_output");
        assert_eq!(payload["tools"][0]["function"]["strict"], true);
        assert_eq!(
            payload["tools"][0]["function"]["parameters"]["properties"]["ok"]["type"],
            "boolean"
        );
        assert_eq!(payload["tool_choice"]["function"]["name"], "agent_output");
    }

    #[test]
    fn openai_payload_preserves_multimodal_content_parts() {
        let payload = openai_chat_payload(
            "gpt-test",
            "agent_output",
            OpenAiStructuredOutputMode::JsonSchema,
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
    fn ollama_payload_uses_format_schema() {
        let payload = ollama_chat_payload(
            "llama-test",
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

        assert_eq!(payload["model"], "llama-test");
        assert_eq!(payload["stream"], false);
        assert_eq!(payload["messages"][0]["role"], "user");
        assert_eq!(payload["messages"][0]["content"], "Return JSON");
        assert_eq!(payload["format"]["properties"]["ok"]["type"], "boolean");
    }

    #[test]
    fn ollama_payload_preserves_data_url_images() {
        let payload = ollama_chat_payload(
            "llava-test",
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

        assert_eq!(payload["messages"][0]["content"], "what changed?");
        assert_eq!(payload["messages"][0]["images"][0], "abc");
    }

    #[test]
    fn anthropic_payload_uses_structured_outputs_format() {
        let payload = anthropic_messages_payload(
            "claude-test",
            2048,
            ChatRequest {
                messages: vec![
                    ChatMessage::text(MessageRole::System, "Return JSON only"),
                    ChatMessage::text(MessageRole::User, "Extract the result"),
                ],
                output_schema: Some(json!({
                    "type": "object",
                    "properties": {
                        "ok": { "type": "boolean" }
                    },
                    "required": ["ok"],
                    "additionalProperties": false
                })),
            },
        );

        assert_eq!(payload["model"], "claude-test");
        assert_eq!(payload["max_tokens"], 2048);
        assert_eq!(payload["system"], "Return JSON only");
        assert_eq!(payload["messages"][0]["role"], "user");
        assert_eq!(payload["messages"][0]["content"][0]["type"], "text");
        assert_eq!(payload["output_config"]["format"]["type"], "json_schema");
        assert_eq!(
            payload["output_config"]["format"]["schema"]["properties"]["ok"]["type"],
            "boolean"
        );
    }

    #[test]
    fn anthropic_payload_preserves_data_url_images() {
        let payload = anthropic_messages_payload(
            "claude-test",
            1024,
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
        assert_eq!(payload["messages"][0]["content"][1]["type"], "image");
        assert_eq!(
            payload["messages"][0]["content"][1]["source"]["media_type"],
            "image/png"
        );
        assert_eq!(
            payload["messages"][0]["content"][1]["source"]["data"],
            "abc"
        );
    }

    #[test]
    fn gemini_payload_uses_structured_outputs_format() {
        let payload = gemini_generate_content_payload(ChatRequest {
            messages: vec![
                ChatMessage::text(MessageRole::System, "Return JSON only"),
                ChatMessage::text(MessageRole::User, "Extract the result"),
            ],
            output_schema: Some(json!({
                "type": "object",
                "properties": {
                    "ok": { "type": "boolean" }
                },
                "required": ["ok"]
            })),
        });

        assert_eq!(
            payload["systemInstruction"]["parts"][0]["text"],
            "Return JSON only"
        );
        assert_eq!(payload["contents"][0]["role"], "user");
        assert_eq!(
            payload["contents"][0]["parts"][0]["text"],
            "Extract the result"
        );
        assert_eq!(
            payload["generationConfig"]["responseFormat"]["text"]["mimeType"],
            "application/json"
        );
        assert_eq!(
            payload["generationConfig"]["responseFormat"]["text"]["schema"]["properties"]["ok"]["type"],
            "boolean"
        );
    }

    #[test]
    fn gemini_payload_preserves_data_url_images() {
        let payload = gemini_generate_content_payload(ChatRequest {
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
        });

        assert_eq!(payload["contents"][0]["parts"][0]["text"], "what changed?");
        assert_eq!(
            payload["contents"][0]["parts"][1]["inlineData"]["mimeType"],
            "image/png"
        );
        assert_eq!(
            payload["contents"][0]["parts"][1]["inlineData"]["data"],
            "abc"
        );
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

    #[test]
    fn parses_tool_call_json_chat_completion() {
        let raw = json!({
            "model": "tool-call-test",
            "choices": [
                {
                    "message": {
                        "role": "assistant",
                        "content": null,
                        "tool_calls": [
                            {
                                "type": "function",
                                "function": {
                                    "name": "agent_output",
                                    "arguments": "{\"ok\":true}"
                                }
                            }
                        ]
                    }
                }
            ]
        });

        let parsed = parse_openai_chat_completion(&raw).expect("parse tool call");

        assert_eq!(parsed, json!({ "ok": true }));
    }

    #[test]
    fn parses_legacy_function_call_json_chat_completion() {
        let raw = json!({
            "model": "function-call-test",
            "choices": [
                {
                    "finish_reason": "function_call",
                    "message": {
                        "role": "assistant",
                        "function_call": {
                            "name": "agent_output",
                            "arguments": {
                                "ok": true
                            }
                        }
                    }
                }
            ]
        });

        let parsed = parse_openai_chat_completion(&raw).expect("parse function call");

        assert_eq!(parsed, json!({ "ok": true }));
    }

    #[test]
    fn openai_parser_rejects_malformed_tool_call_arguments_and_truncation() {
        let malformed = json!({
            "choices": [
                {
                    "finish_reason": "tool_calls",
                    "message": {
                        "role": "assistant",
                        "tool_calls": [
                            {
                                "type": "function",
                                "function": {
                                    "name": "agent_output",
                                    "arguments": "{\"ok\""
                                }
                            }
                        ]
                    }
                }
            ]
        });
        let missing_arguments = json!({
            "choices": [
                {
                    "finish_reason": "tool_calls",
                    "message": {
                        "role": "assistant",
                        "tool_calls": [
                            {
                                "type": "function",
                                "function": {
                                    "name": "agent_output"
                                }
                            }
                        ]
                    }
                }
            ]
        });
        let truncated = json!({
            "choices": [
                {
                    "finish_reason": "length",
                    "message": {
                        "role": "assistant",
                        "content": "{\"ok\""
                    }
                }
            ]
        });

        assert!(matches!(
            parse_openai_chat_completion(&malformed),
            Err(LlmError::InvalidStructuredOutput(message))
                if message.contains("tool call arguments")
        ));
        assert!(matches!(
            parse_openai_chat_completion(&missing_arguments),
            Err(LlmError::Provider(message)) if message.contains("tool call arguments")
        ));
        assert!(matches!(
            parse_openai_chat_completion(&truncated),
            Err(LlmError::Provider(message)) if message.contains("length")
        ));
    }

    #[test]
    fn parses_anthropic_json_text_message() {
        let raw = json!({
            "model": "claude-test",
            "stop_reason": "end_turn",
            "content": [
                {
                    "type": "text",
                    "text": "{\"ok\":true}"
                }
            ]
        });

        let parsed = parse_anthropic_message(&raw).expect("parse completion");

        assert_eq!(parsed, json!({ "ok": true }));
    }

    #[test]
    fn parses_ollama_json_text_message() {
        let raw = json!({
            "model": "llama-test",
            "message": {
                "role": "assistant",
                "content": "{\"ok\":true}"
            },
            "done": true
        });

        let parsed = parse_ollama_chat_response(&raw).expect("parse completion");

        assert_eq!(parsed, json!({ "ok": true }));
    }

    #[test]
    fn parses_gemini_json_text_message() {
        let raw = json!({
            "modelVersion": "gemini-test",
            "candidates": [
                {
                    "finishReason": "STOP",
                    "content": {
                        "parts": [
                            {
                                "text": "{\"ok\":true}"
                            }
                        ]
                    }
                }
            ]
        });

        let parsed = parse_gemini_generate_content(&raw).expect("parse completion");

        assert_eq!(parsed, json!({ "ok": true }));
    }

    #[test]
    fn gemini_parser_rejects_safety_stops() {
        let raw = json!({
            "candidates": [
                {
                    "finishReason": "SAFETY",
                    "content": {
                        "parts": [
                            {
                                "text": "{\"ok\":true}"
                            }
                        ]
                    }
                }
            ]
        });

        assert!(matches!(
            parse_gemini_generate_content(&raw),
            Err(LlmError::Provider(message)) if message.contains("SAFETY")
        ));
    }

    #[test]
    fn anthropic_parser_rejects_refusal_and_truncation() {
        let refusal = json!({
            "stop_reason": "refusal",
            "content": [{ "type": "text", "text": "no" }]
        });
        let truncated = json!({
            "stop_reason": "max_tokens",
            "content": [{ "type": "text", "text": "{\"ok\"" }]
        });

        assert!(matches!(
            parse_anthropic_message(&refusal),
            Err(LlmError::Provider(message)) if message.contains("refused")
        ));
        assert!(matches!(
            parse_anthropic_message(&truncated),
            Err(LlmError::Provider(message)) if message.contains("max_tokens")
        ));
    }
}
