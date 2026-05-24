use super::*;
use serde_json::json;

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
    let fenced =
        parse_json_object_text("```json\n{\"ok\":true}\n```", "fenced").expect("parse fenced JSON");
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
