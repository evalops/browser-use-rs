use async_trait::async_trait;
use reqwest::{
    StatusCode,
    header::{AUTHORIZATION, CONTENT_TYPE, HeaderMap, HeaderName, HeaderValue},
};
use serde_json::{Value, json};

use crate::common::{
    append_json_schema_instruction, first_json_u64, json_u64, parse_json_object_compatible,
    parse_json_object_text,
};
use crate::{
    ChatCompletion, ChatMessage, ChatModel, ChatRequest, ChatUsage, ContentPart, LlmError,
    MessageRole,
};

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

pub(crate) fn openai_chat_payload(
    model: &str,
    schema_name: &str,
    structured_output_mode: OpenAiStructuredOutputMode,
    schema_transform: OpenAiSchemaTransform,
    mut request: ChatRequest,
) -> Value {
    let output_schema = request.output_schema.take().map(|schema| {
        let schema = transform_schema(schema, schema_transform);
        if matches!(
            structured_output_mode,
            OpenAiStructuredOutputMode::JsonSchema | OpenAiStructuredOutputMode::ToolCall
        ) {
            strict_openai_schema(schema)
        } else {
            schema
        }
    });
    if matches!(
        structured_output_mode,
        OpenAiStructuredOutputMode::JsonObject | OpenAiStructuredOutputMode::PromptOnly
    ) {
        // Providers in JSON-object or prompt-only mode do not receive a strict
        // schema envelope, so we append the schema as text to keep the model's
        // target shape visible without changing the provider transport mode.
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
                // Tool-call mode is useful for OpenAI-compatible providers that
                // implement function calling but not the newer json_schema
                // response_format. The same schema is carried as parameters.
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

fn strict_openai_schema(value: Value) -> Value {
    let mut schema = strict_openai_schema_value(value);
    if let Some(definitions) = schema.get("$defs").and_then(Value::as_object).cloned() {
        // OpenAI accepts local refs generally, but rejects the browser action
        // union when an anyOf variant's property is only a $defs ref.
        inline_any_of_property_refs(&mut schema, &definitions);
    }
    schema
}

fn strict_openai_schema_value(value: Value) -> Value {
    match value {
        Value::Object(mut entries) => {
            for entry in entries.values_mut() {
                *entry = strict_openai_schema_value(std::mem::take(entry));
            }

            if let Some(definitions) = entries.remove("definitions") {
                entries.insert("$defs".to_owned(), definitions);
            }
            if let Some(Value::String(reference)) = entries.get_mut("$ref") {
                if let Some(target) = reference.strip_prefix("#/definitions/") {
                    *reference = format!("#/$defs/{target}");
                }
            }

            remove_openai_strict_unsupported_keywords(&mut entries);

            if let Some(ref_schema) = single_ref_all_of(&entries) {
                return ref_schema;
            }

            if let Some(one_of) = entries.remove("oneOf") {
                entries.entry("anyOf".to_owned()).or_insert(one_of);
            }

            if is_object_schema(&entries) {
                if let Some(Value::Object(properties)) = entries.get_mut("properties") {
                    properties.retain(|_, schema| !matches!(schema, Value::Bool(_)));
                }
                entries.insert("additionalProperties".to_owned(), Value::Bool(false));
                if let Some(Value::Object(properties)) = entries.get("properties") {
                    entries.insert(
                        "required".to_owned(),
                        Value::Array(properties.keys().cloned().map(Value::String).collect()),
                    );
                }
            }

            Value::Object(entries)
        }
        Value::Array(values) => {
            Value::Array(values.into_iter().map(strict_openai_schema_value).collect())
        }
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => value,
    }
}

fn inline_any_of_property_refs(value: &mut Value, definitions: &serde_json::Map<String, Value>) {
    match value {
        Value::Object(entries) => {
            if let Some(Value::Array(variants)) = entries.get_mut("anyOf") {
                for variant in variants {
                    inline_direct_property_refs(variant, definitions);
                }
            }

            for entry in entries.values_mut() {
                inline_any_of_property_refs(entry, definitions);
            }
        }
        Value::Array(values) => {
            for entry in values {
                inline_any_of_property_refs(entry, definitions);
            }
        }
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {}
    }
}

fn inline_direct_property_refs(value: &mut Value, definitions: &serde_json::Map<String, Value>) {
    let Value::Object(entries) = value else {
        return;
    };
    let Some(Value::Object(properties)) = entries.get_mut("properties") else {
        return;
    };

    for property in properties.values_mut() {
        if let Some(definition) = local_definition_ref(property, definitions) {
            *property = definition;
        }
    }
}

fn local_definition_ref(
    value: &Value,
    definitions: &serde_json::Map<String, Value>,
) -> Option<Value> {
    let reference = value.as_object()?.get("$ref")?.as_str()?;
    let name = reference.strip_prefix("#/$defs/")?;
    definitions.get(name).cloned()
}

fn remove_openai_strict_unsupported_keywords(entries: &mut serde_json::Map<String, Value>) {
    for key in [
        "default",
        "format",
        "minimum",
        "maximum",
        "multipleOf",
        "minLength",
        "maxLength",
        "pattern",
        "minItems",
        "maxItems",
        "patternProperties",
        "not",
        "dependentRequired",
        "dependentSchemas",
        "if",
        "then",
        "else",
    ] {
        entries.remove(key);
    }
}

fn single_ref_all_of(entries: &serde_json::Map<String, Value>) -> Option<Value> {
    if entries.len() != 1 {
        return None;
    }
    let Some(Value::Array(items)) = entries.get("allOf") else {
        return None;
    };
    let [Value::Object(item)] = items.as_slice() else {
        return None;
    };
    if item.len() == 1 && item.contains_key("$ref") {
        Some(Value::Object(item.clone()))
    } else {
        None
    }
}

fn is_object_schema(entries: &serde_json::Map<String, Value>) -> bool {
    if entries.contains_key("properties") {
        return true;
    }

    match entries.get("type") {
        Some(Value::String(kind)) => kind == "object",
        Some(Value::Array(kinds)) => kinds.iter().any(|kind| kind == "object"),
        _ => false,
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

pub(crate) fn parse_openai_chat_completion(raw_response: &Value) -> Result<Value, LlmError> {
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

pub(crate) fn parse_groq_failed_generation_response(
    raw_response: &Value,
) -> Result<Option<Value>, LlmError> {
    let Some(failed_generation) = raw_response
        .pointer("/error/failed_generation")
        .and_then(Value::as_str)
    else {
        return Ok(None);
    };

    parse_json_object_text(failed_generation, "Groq failed_generation").map(Some)
}

pub(crate) fn parse_openai_compatible_usage(raw_response: &Value) -> Option<ChatUsage> {
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
