use serde_json::Value;

use crate::{ChatMessage, ContentPart, LlmError, MessageRole};

pub(crate) fn append_json_schema_instruction(messages: &mut Vec<ChatMessage>, schema: &Value) {
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

pub(crate) fn parse_json_object_compatible(value: &Value, source: &str) -> Result<Value, LlmError> {
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

pub(crate) fn parse_json_object_text(text: &str, source: &str) -> Result<Value, LlmError> {
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

pub(crate) fn first_json_u64(values: &[Option<&Value>]) -> Option<u64> {
    values.iter().find_map(|value| json_u64(*value))
}

pub(crate) fn json_u64(value: Option<&Value>) -> Option<u64> {
    value.and_then(Value::as_u64)
}

pub(crate) fn text_content_part(part: ContentPart) -> Option<String> {
    match part {
        ContentPart::Text { text } => Some(text),
        ContentPart::ImageUrl { image_url, .. } => Some(format!("[image_url: {image_url}]")),
    }
}

pub(crate) fn data_url_image_source(image_url: &str) -> Option<(String, String)> {
    let rest = image_url.strip_prefix("data:")?;
    let (media_type, data) = rest.split_once(";base64,")?;
    if !media_type.starts_with("image/") || data.is_empty() {
        return None;
    }
    Some((media_type.to_owned(), data.to_owned()))
}
