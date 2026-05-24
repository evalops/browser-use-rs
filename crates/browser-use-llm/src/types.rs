use async_trait::async_trait;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
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
