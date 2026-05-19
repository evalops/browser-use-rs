//! LLM provider contracts for schema-guided agent calls.
//!
//! The core agent talks to language models through the [`ChatModel`] trait
//! instead of depending on a single provider SDK. Each concrete model below
//! converts the common [`ChatRequest`] shape into that provider's HTTP API,
//! parses the provider response back into JSON, and normalizes usage/error
//! data for the rest of the system.

use async_trait::async_trait;
use reqwest::{
    StatusCode,
    header::{AUTHORIZATION, CONTENT_TYPE, HeaderMap, HeaderName, HeaderValue},
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
/// Role attached to one chat message.
pub enum MessageRole {
    /// System/developer instructions.
    System,
    /// User-authored task or observation content.
    User,
    /// Model-authored content from a previous turn.
    Assistant,
    /// Tool result content, retained for provider compatibility.
    Tool,
}

/// Image detail hint used by vision-capable providers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ImageDetailLevel {
    /// Let the provider choose image detail.
    Auto,
    /// Ask the provider for a low-token image encoding.
    Low,
    /// Ask the provider for a higher-detail image encoding.
    High,
}

impl ImageDetailLevel {
    /// Returns the provider-facing string for this detail level.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Low => "low",
            Self::High => "high",
        }
    }
}

/// One piece of multimodal chat-message content.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentPart {
    /// Plain text content.
    Text {
        /// Text payload.
        text: String,
    },
    /// Image content referenced by URL or data URL.
    ImageUrl {
        /// Image URL, often a `data:` URL for screenshots.
        image_url: String,
        /// Optional vision detail hint.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        detail: Option<ImageDetailLevel>,
    },
}

/// A provider-neutral chat message.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ChatMessage {
    /// Conversation role for this message.
    pub role: MessageRole,
    /// Ordered content parts in the message.
    pub content: Vec<ContentPart>,
}

impl ChatMessage {
    /// Creates a text-only message.
    #[must_use]
    pub fn text(role: MessageRole, text: impl Into<String>) -> Self {
        Self {
            role,
            content: vec![ContentPart::Text { text: text.into() }],
        }
    }
}

/// Provider-neutral request sent from the agent to a chat model.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ChatRequest {
    /// Messages to send in order.
    pub messages: Vec<ChatMessage>,
    /// Optional JSON Schema describing the structured response expected.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_schema: Option<Value>,
}

/// Normalized token accounting returned by providers when available.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ChatUsage {
    /// Tokens in the prompt/input side of the request.
    pub prompt_tokens: u64,
    /// Prompt tokens served from a provider cache.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_cached_tokens: Option<u64>,
    /// Tokens written into a provider cache.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_cache_creation_tokens: Option<u64>,
    /// Provider-reported image tokens, when separate from text tokens.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_image_tokens: Option<u64>,
    /// Tokens in the completion/output side of the response.
    pub completion_tokens: u64,
    /// Total tokens reported by the provider.
    pub total_tokens: u64,
}

/// Normalized completion returned by any [`ChatModel`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ChatCompletion<T> {
    /// Provider model identifier that produced the response.
    pub model: String,
    /// Parsed completion content.
    pub content: T,
    /// Optional usage accounting.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<ChatUsage>,
    /// Raw provider response retained for diagnostics and conformance tests.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw_response: Option<Value>,
}

/// Errors normalized across provider clients.
#[derive(Debug, Error)]
pub enum LlmError {
    /// Provider returned an error or the local HTTP call failed.
    #[error("provider error: {0}")]
    Provider(String),
    /// Provider indicated a rate limit.
    #[error("rate limited: {0}")]
    RateLimited(String),
    /// Provider response could not be parsed as the requested structured JSON.
    #[error("invalid structured output: {0}")]
    InvalidStructuredOutput(String),
}

#[async_trait]
/// Async interface implemented by all chat providers.
///
/// The trait returns JSON because the browser-use agent asks providers for a
/// schema-shaped `AgentOutput`. Provider adapters are responsible for turning
/// provider-specific response formats, tool calls, or JSON-mode text into that
/// common JSON value.
pub trait ChatModel: Send + Sync {
    /// Stable provider name used for logging and configuration.
    fn provider(&self) -> &str;

    /// Provider model identifier.
    fn model(&self) -> &str;

    /// Invokes the model and returns structured JSON content.
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
/// Strategy used when asking OpenAI-compatible APIs for structured output.
pub enum OpenAiStructuredOutputMode {
    /// Use `response_format: json_schema`.
    JsonSchema,
    /// Use `response_format: json_object`.
    JsonObject,
    /// Add schema instructions to the prompt without API-level enforcement.
    PromptOnly,
    /// Ask the model to return a tool call whose arguments contain the JSON.
    ToolCall,
}

/// Optional JSON Schema rewrite applied before sending OpenAI-wire requests.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpenAiSchemaTransform {
    /// Send the schema as generated.
    Default,
    /// Remove schema keywords unsupported by Mistral's OpenAI-compatible API.
    MistralCompatible,
}

/// Chat model for OpenAI-compatible `/chat/completions` APIs.
#[derive(Clone)]
pub struct OpenAiCompatibleChatModel {
    api_key: String,
    model: String,
    base_url: String,
    provider_name: String,
    schema_name: String,
    structured_output_mode: OpenAiStructuredOutputMode,
    schema_transform: OpenAiSchemaTransform,
    default_headers: HeaderMap,
    client: reqwest::Client,
}

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

/// Chat model for Google's Gemini `generateContent` API.
#[derive(Clone)]
pub struct GeminiChatModel {
    api_key: String,
    model: String,
    base_url: String,
    supports_structured_output: bool,
    client: reqwest::Client,
}

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

impl OpenAiCompatibleChatModel {
    /// Creates an OpenAI-wire model using the default OpenAI base URL.
    #[must_use]
    pub fn new(api_key: impl Into<String>, model: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            model: model.into(),
            base_url: "https://api.openai.com/v1".to_owned(),
            provider_name: "openai-compatible".to_owned(),
            schema_name: "agent_output".to_owned(),
            structured_output_mode: OpenAiStructuredOutputMode::JsonSchema,
            schema_transform: OpenAiSchemaTransform::Default,
            default_headers: HeaderMap::new(),
            client: reqwest::Client::new(),
        }
    }

    /// Creates an OpenAI-compatible model from `OPENAI_API_KEY`.
    pub fn from_env(model: impl Into<String>) -> Result<Self, LlmError> {
        let api_key = std::env::var("OPENAI_API_KEY")
            .map_err(|_| LlmError::Provider("OPENAI_API_KEY is not set".to_owned()))?;
        Ok(Self::new(api_key, model))
    }

    /// Overrides the OpenAI-wire API base URL.
    #[must_use]
    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into().trim_end_matches('/').to_owned();
        self
    }

    /// Sets the provider label returned by [`ChatModel::provider`].
    #[must_use]
    pub fn with_provider_name(mut self, provider_name: impl Into<String>) -> Self {
        self.provider_name = provider_name.into();
        self
    }

    /// Sets the JSON Schema name sent to structured-output APIs.
    #[must_use]
    pub fn with_schema_name(mut self, schema_name: impl Into<String>) -> Self {
        self.schema_name = schema_name.into();
        self
    }

    /// Selects the structured-output strategy for OpenAI-wire APIs.
    #[must_use]
    pub fn with_structured_output_mode(
        mut self,
        structured_output_mode: OpenAiStructuredOutputMode,
    ) -> Self {
        self.structured_output_mode = structured_output_mode;
        self
    }

    /// Selects a provider-specific schema rewrite.
    #[must_use]
    pub fn with_schema_transform(mut self, schema_transform: OpenAiSchemaTransform) -> Self {
        self.schema_transform = schema_transform;
        self
    }

    /// Adds a default HTTP header to every OpenAI-wire request.
    ///
    /// Authorization and content-type are rejected because this adapter owns
    /// those headers and overriding them would make provider behavior hard to
    /// reason about.
    pub fn try_with_default_header(
        mut self,
        name: impl AsRef<str>,
        value: impl AsRef<str>,
    ) -> Result<Self, LlmError> {
        let name = HeaderName::from_bytes(name.as_ref().as_bytes()).map_err(|error| {
            LlmError::Provider(format!("invalid OpenAI-wire default header name: {error}"))
        })?;
        if name == AUTHORIZATION || name == CONTENT_TYPE {
            return Err(LlmError::Provider(format!(
                "refusing to override reserved OpenAI-wire header `{}`",
                name.as_str()
            )));
        }
        let value = HeaderValue::from_str(value.as_ref()).map_err(|error| {
            LlmError::Provider(format!("invalid OpenAI-wire default header value: {error}"))
        })?;
        self.default_headers.insert(name, value);
        Ok(self)
    }

    /// Returns a configured default header value, if present and valid UTF-8.
    #[must_use]
    pub fn default_header_value(&self, name: &str) -> Option<&str> {
        let name = HeaderName::from_bytes(name.as_bytes()).ok()?;
        self.default_headers
            .get(&name)
            .and_then(|value| value.to_str().ok())
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
            self.schema_transform,
            request,
        );
        let mut builder = self
            .client
            .post(self.chat_completions_url())
            .bearer_auth(&self.api_key);
        for (name, value) in &self.default_headers {
            builder = builder.header(name, value);
        }
        let response = builder
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
            if self.provider_name == "groq" {
                if let Some(content) = parse_groq_failed_generation_response(&raw_response)? {
                    return Ok(ChatCompletion {
                        model: raw_response
                            .get("model")
                            .and_then(Value::as_str)
                            .unwrap_or(&self.model)
                            .to_owned(),
                        content,
                        usage: parse_openai_compatible_usage(&raw_response),
                        raw_response: Some(raw_response),
                    });
                }
            }

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
            usage: parse_openai_compatible_usage(&raw_response),
            raw_response: Some(raw_response),
        })
    }
}

fn openai_chat_payload(
    model: &str,
    schema_name: &str,
    structured_output_mode: OpenAiStructuredOutputMode,
    schema_transform: OpenAiSchemaTransform,
    mut request: ChatRequest,
) -> Value {
    let output_schema = request
        .output_schema
        .take()
        .map(|schema| transform_schema(schema, schema_transform));
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

fn transform_schema(schema: Value, schema_transform: OpenAiSchemaTransform) -> Value {
    match schema_transform {
        OpenAiSchemaTransform::Default => schema,
        OpenAiSchemaTransform::MistralCompatible => {
            strip_mistral_unsupported_schema_keywords(schema)
        }
    }
}

fn strip_mistral_unsupported_schema_keywords(value: Value) -> Value {
    const UNSUPPORTED: &[&str] = &["minLength", "maxLength", "pattern", "format"];

    match value {
        Value::Object(map) => Value::Object(
            map.into_iter()
                .filter_map(|(key, value)| {
                    if UNSUPPORTED.contains(&key.as_str()) {
                        None
                    } else {
                        Some((key, strip_mistral_unsupported_schema_keywords(value)))
                    }
                })
                .collect(),
        ),
        Value::Array(values) => Value::Array(
            values
                .into_iter()
                .map(strip_mistral_unsupported_schema_keywords)
                .collect(),
        ),
        other => other,
    }
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
                ContentPart::ImageUrl { image_url, detail } => {
                    let mut image_url_value = json!({
                        "url": image_url,
                    });
                    if let Some(detail) = detail {
                        image_url_value["detail"] = json!(detail.as_str());
                    }
                    json!({
                        "type": "image_url",
                        "image_url": image_url_value,
                    })
                }
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

#[cfg(test)]
fn gemini_generate_content_payload(request: ChatRequest) -> Value {
    gemini_generate_content_payload_with_mode(request, true)
}

fn gemini_generate_content_payload_with_mode(
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
        Value::String(text) => parse_json_object_text(text, "chat completion content"),
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
        Value::String(text) => return parse_json_object_text(text, source),
        Value::Object(_) => value.clone(),
        Value::Array(values)
            if values.len() == 1 && values.first().is_some_and(Value::is_object) =>
        {
            values[0].clone()
        }
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

fn parse_json_object_text(text: &str, source: &str) -> Result<Value, LlmError> {
    let candidate = strip_json_code_fence(text.trim());
    match parse_json_object_candidate(candidate, source) {
        Ok(parsed) => Ok(parsed),
        Err(first_error) => {
            if let Some(extracted) = extract_balanced_json_object(candidate) {
                return parse_json_object_candidate(extracted, source);
            }
            Err(first_error)
        }
    }
}

fn parse_json_object_candidate(candidate: &str, source: &str) -> Result<Value, LlmError> {
    let parsed: Value = serde_json::from_str(candidate)
        .map_err(|error| LlmError::InvalidStructuredOutput(format!("{source}: {error}")))?;
    if parsed.is_object() {
        return Ok(parsed);
    }
    if let Value::Array(values) = &parsed {
        if values.len() == 1 && values.first().is_some_and(Value::is_object) {
            return Ok(values[0].clone());
        }
    }
    Err(LlmError::InvalidStructuredOutput(format!(
        "{source} was not a JSON object"
    )))
}

fn strip_json_code_fence(text: &str) -> &str {
    let Some(stripped) = text.strip_prefix("```") else {
        return text;
    };
    let Some(stripped) = stripped.strip_suffix("```") else {
        return text;
    };
    let stripped = stripped.trim();
    stripped.strip_prefix("json").unwrap_or(stripped).trim()
}

fn extract_balanced_json_object(text: &str) -> Option<&str> {
    let mut start = None;
    let mut depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;

    for (index, ch) in text.char_indices() {
        if start.is_none() {
            if ch == '{' {
                start = Some(index);
                depth = 1;
            }
            continue;
        }

        if in_string {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_string = false;
            }
            continue;
        }

        match ch {
            '"' => in_string = true,
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(&text[start.expect("start set")..=index]);
                }
            }
            _ => {}
        }
    }

    None
}

fn parse_groq_failed_generation_response(raw_response: &Value) -> Result<Option<Value>, LlmError> {
    let Some(failed_generation) = raw_response
        .pointer("/error/failed_generation")
        .and_then(Value::as_str)
    else {
        return Ok(None);
    };

    parse_json_object_text(failed_generation, "Groq failed_generation").map(Some)
}

fn parse_ollama_chat_response(raw_response: &Value) -> Result<Value, LlmError> {
    let content = raw_response
        .pointer("/message/content")
        .and_then(Value::as_str)
        .ok_or_else(|| LlmError::Provider("missing Ollama message content".to_owned()))?;

    parse_json_object_text(content, "Ollama message content")
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

    parse_json_object_text(text, "Gemini text content")
}

fn parse_openai_compatible_usage(raw_response: &Value) -> Option<ChatUsage> {
    let usage = raw_response.get("usage")?;
    let prompt_tokens = json_u64(usage.get("prompt_tokens"));
    let completion_tokens = json_u64(usage.get("completion_tokens"));
    let total_tokens =
        json_u64(usage.get("total_tokens")).unwrap_or(prompt_tokens? + completion_tokens?);

    Some(ChatUsage {
        prompt_tokens: prompt_tokens?,
        prompt_cached_tokens: first_json_u64(&[
            usage.pointer("/prompt_tokens_details/cached_tokens"),
            usage.pointer("/input_tokens_details/cached_tokens"),
            usage.get("cache_read_input_tokens"),
        ]),
        prompt_cache_creation_tokens: first_json_u64(&[
            usage.get("cache_creation_input_tokens"),
            usage.get("prompt_cache_creation_tokens"),
        ]),
        prompt_image_tokens: first_json_u64(&[
            usage.pointer("/prompt_tokens_details/image_tokens"),
            usage.get("prompt_image_tokens"),
        ]),
        completion_tokens: completion_tokens?,
        total_tokens,
    })
}

fn parse_anthropic_usage(raw_response: &Value) -> Option<ChatUsage> {
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

fn parse_gemini_usage(raw_response: &Value) -> Option<ChatUsage> {
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

fn first_json_u64(values: &[Option<&Value>]) -> Option<u64> {
    values.iter().find_map(|value| json_u64(*value))
}

fn json_u64(value: Option<&Value>) -> Option<u64> {
    value.and_then(Value::as_u64)
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

fn text_content_part(part: ContentPart) -> Option<String> {
    match part {
        ContentPart::Text { text } => Some(text),
        ContentPart::ImageUrl { image_url, .. } => Some(format!("[image_url: {image_url}]")),
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
    fn openai_compatible_model_stores_safe_default_headers() {
        let model = OpenAiCompatibleChatModel::new("test-key", "test-model")
            .try_with_default_header("HTTP-Referer", "https://evalops.dev")
            .expect("referer header")
            .try_with_default_header("X-OpenRouter-Title", "EvalOps browser-use-rs")
            .expect("title header");

        assert_eq!(
            model.default_header_value("HTTP-Referer"),
            Some("https://evalops.dev")
        );
        assert_eq!(
            model.default_header_value("x-openrouter-title"),
            Some("EvalOps browser-use-rs")
        );
    }

    #[test]
    fn openai_compatible_model_rejects_reserved_default_headers() {
        let error = match OpenAiCompatibleChatModel::new("test-key", "test-model")
            .try_with_default_header("Authorization", "Bearer replacement")
        {
            Ok(_) => panic!("authorization header must be reserved"),
            Err(error) => error,
        };
        assert!(error.to_string().contains("reserved OpenAI-wire header"));

        let error = match OpenAiCompatibleChatModel::new("test-key", "test-model")
            .try_with_default_header("X-OpenRouter-Title", "bad\nvalue")
        {
            Ok(_) => panic!("invalid header value must be rejected"),
            Err(error) => error,
        };
        assert!(
            error
                .to_string()
                .contains("invalid OpenAI-wire default header value")
        );
    }

    #[test]
    fn openai_compatible_usage_reads_cached_token_details() {
        let usage = parse_openai_compatible_usage(&json!({
            "usage": {
                "prompt_tokens": 100,
                "prompt_tokens_details": {
                    "cached_tokens": 40
                },
                "completion_tokens": 25,
                "total_tokens": 125
            }
        }))
        .expect("usage");

        assert_eq!(usage.prompt_tokens, 100);
        assert_eq!(usage.prompt_cached_tokens, Some(40));
        assert_eq!(usage.completion_tokens, 25);
        assert_eq!(usage.total_tokens, 125);
    }

    #[test]
    fn anthropic_usage_matches_upstream_cache_counting() {
        let usage = parse_anthropic_usage(&json!({
            "usage": {
                "input_tokens": 75,
                "cache_read_input_tokens": 20,
                "cache_creation_input_tokens": 10,
                "output_tokens": 15
            }
        }))
        .expect("usage");

        assert_eq!(usage.prompt_tokens, 95);
        assert_eq!(usage.prompt_cached_tokens, Some(20));
        assert_eq!(usage.prompt_cache_creation_tokens, Some(10));
        assert_eq!(usage.completion_tokens, 15);
        assert_eq!(usage.total_tokens, 90);
    }

    #[test]
    fn gemini_usage_includes_thoughts_and_image_tokens() {
        let usage = parse_gemini_usage(&json!({
            "usageMetadata": {
                "promptTokenCount": 50,
                "candidatesTokenCount": 12,
                "thoughtsTokenCount": 8,
                "totalTokenCount": 70,
                "cachedContentTokenCount": 5,
                "promptTokensDetails": [
                    { "modality": "TEXT", "tokenCount": 30 },
                    { "modality": "IMAGE", "tokenCount": 20 }
                ]
            }
        }))
        .expect("usage");

        assert_eq!(usage.prompt_tokens, 50);
        assert_eq!(usage.prompt_cached_tokens, Some(5));
        assert_eq!(usage.prompt_image_tokens, Some(20));
        assert_eq!(usage.completion_tokens, 20);
        assert_eq!(usage.total_tokens, 70);
    }

    #[test]
    fn openai_payload_uses_structured_outputs_format() {
        let payload = openai_chat_payload(
            "gpt-test",
            "agent_output",
            OpenAiStructuredOutputMode::JsonSchema,
            OpenAiSchemaTransform::Default,
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
    fn mistral_schema_transform_strips_unsupported_validation_keywords() {
        let payload = openai_chat_payload(
            "mistral-test",
            "agent_output",
            OpenAiStructuredOutputMode::JsonSchema,
            OpenAiSchemaTransform::MistralCompatible,
            ChatRequest {
                messages: vec![ChatMessage::text(MessageRole::User, "Return JSON")],
                output_schema: Some(json!({
                    "type": "object",
                    "properties": {
                        "email": {
                            "type": "string",
                            "format": "email",
                            "minLength": 3,
                            "maxLength": 128,
                            "pattern": ".+@.+"
                        }
                    },
                    "required": ["email"]
                })),
            },
        );

        let schema = &payload["response_format"]["json_schema"]["schema"];
        assert_eq!(schema["properties"]["email"]["type"], "string");
        assert!(schema["properties"]["email"].get("format").is_none());
        assert!(schema["properties"]["email"].get("minLength").is_none());
        assert!(schema["properties"]["email"].get("maxLength").is_none());
        assert!(schema["properties"]["email"].get("pattern").is_none());
    }

    #[test]
    fn openai_json_object_mode_embeds_schema_instruction() {
        let payload = openai_chat_payload(
            "deepseek-test",
            "agent_output",
            OpenAiStructuredOutputMode::JsonObject,
            OpenAiSchemaTransform::Default,
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
            OpenAiSchemaTransform::Default,
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
            OpenAiSchemaTransform::Default,
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
            OpenAiSchemaTransform::Default,
            ChatRequest {
                messages: vec![ChatMessage {
                    role: MessageRole::User,
                    content: vec![
                        ContentPart::Text {
                            text: "what changed?".to_owned(),
                        },
                        ContentPart::ImageUrl {
                            image_url: "data:image/png;base64,abc".to_owned(),
                            detail: None,
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
    fn openai_payload_includes_image_detail_when_present() {
        let payload = openai_chat_payload(
            "gpt-test",
            "agent_output",
            OpenAiStructuredOutputMode::JsonSchema,
            OpenAiSchemaTransform::Default,
            ChatRequest {
                messages: vec![ChatMessage {
                    role: MessageRole::User,
                    content: vec![ContentPart::ImageUrl {
                        image_url: "data:image/png;base64,abc".to_owned(),
                        detail: Some(ImageDetailLevel::High),
                    }],
                }],
                output_schema: None,
            },
        );

        assert_eq!(
            payload["messages"][0]["content"][0]["image_url"]["detail"],
            "high"
        );
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
                            detail: None,
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
        assert_eq!(payload["tools"][0]["name"], "agent_output");
        assert_eq!(payload["tools"][0]["cache_control"]["type"], "ephemeral");
        assert_eq!(
            payload["tools"][0]["input_schema"]["properties"]["ok"]["type"],
            "boolean"
        );
        assert_eq!(payload["tool_choice"]["type"], "tool");
        assert_eq!(payload["tool_choice"]["name"], "agent_output");
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
                            detail: None,
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
    fn gemini_prompt_fallback_embeds_schema_instruction() {
        let payload = gemini_generate_content_payload_with_mode(
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
                    "required": ["ok"]
                })),
            },
            false,
        );

        assert!(payload.get("generationConfig").is_none());
        assert_eq!(
            payload["systemInstruction"]["parts"][0]["text"],
            "Return JSON only"
        );
        let prompt = payload["contents"][0]["parts"][0]["text"]
            .as_str()
            .expect("fallback prompt");
        assert!(prompt.contains("Extract the result"));
        assert!(prompt.contains("valid JSON object"));
        assert!(prompt.contains("\"ok\""));
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
                        detail: None,
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
    fn parses_prompt_fallback_json_wrappers() {
        let fenced = parse_json_object_text("```json\n{\"ok\":true}\n```", "fenced")
            .expect("parse fenced JSON");
        let extracted =
            parse_json_object_text("<function=AgentOutput>{\"ok\":true}</function>", "tagged")
                .expect("parse tagged JSON");
        let singleton =
            parse_json_object_text("[{\"ok\":true}]", "singleton").expect("parse singleton array");

        assert_eq!(fenced, json!({ "ok": true }));
        assert_eq!(extracted, json!({ "ok": true }));
        assert_eq!(singleton, json!({ "ok": true }));
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
    fn parses_anthropic_tool_use_message() {
        let raw = json!({
            "model": "claude-test",
            "stop_reason": "tool_use",
            "content": [
                {
                    "type": "tool_use",
                    "name": "agent_output",
                    "input": {
                        "ok": true
                    }
                }
            ]
        });

        let parsed = parse_anthropic_message(&raw).expect("parse tool use");

        assert_eq!(parsed, json!({ "ok": true }));
    }

    #[test]
    fn parses_groq_failed_generation_json() {
        let raw = json!({
            "error": {
                "failed_generation": "<function=AgentOutput>{\"ok\":true}</function>"
            }
        });

        let parsed = parse_groq_failed_generation_response(&raw)
            .expect("failed_generation parser")
            .expect("failed_generation content");

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
