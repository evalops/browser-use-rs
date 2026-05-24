//! LLM provider contracts for schema-guided agent calls.
//!
//! The core agent talks to language models through the [`ChatModel`] trait
//! instead of depending on a single provider SDK. Each concrete model below
//! converts the common [`ChatRequest`] shape into that provider's HTTP API,
//! parses the provider response back into JSON, and normalizes usage/error
//! data for the rest of the system.
//!
//! ```mermaid
//! flowchart LR
//!     Core["browser-use-core"] --> Request["ChatRequest"]
//!     Request --> Adapter["provider adapter"]
//!     Adapter --> Payload["HTTP payload"]
//!     Payload --> Provider["OpenAI / Anthropic / Gemini / Ollama / compatible"]
//!     Provider --> Raw["provider response"]
//!     Raw --> Parse["JSON extraction + usage normalization"]
//!     Parse --> Completion["ChatCompletion<Value>"]
//!     Completion --> Core
//! ```

mod anthropic;
mod common;
mod gemini;
mod ollama;
mod openai;
mod types;

pub use anthropic::AnthropicChatModel;
pub use gemini::GeminiChatModel;
pub use ollama::OllamaChatModel;
pub use openai::{OpenAiCompatibleChatModel, OpenAiSchemaTransform, OpenAiStructuredOutputMode};
pub use types::{
    ChatCompletion, ChatMessage, ChatModel, ChatRequest, ChatUsage, ContentPart, ImageDetailLevel,
    LlmError, MessageRole,
};

#[cfg(test)]
pub(crate) use anthropic::{
    anthropic_messages_payload, parse_anthropic_message, parse_anthropic_usage,
};
#[cfg(test)]
pub(crate) use common::parse_json_object_text;
#[cfg(test)]
pub(crate) use gemini::{
    gemini_generate_content_payload, gemini_generate_content_payload_with_mode,
    parse_gemini_generate_content, parse_gemini_usage,
};
#[cfg(test)]
pub(crate) use ollama::{ollama_chat_payload, parse_ollama_chat_response};
#[cfg(test)]
pub(crate) use openai::{
    openai_chat_payload, parse_groq_failed_generation_response, parse_openai_chat_completion,
    parse_openai_compatible_usage,
};

#[cfg(test)]
mod tests;
