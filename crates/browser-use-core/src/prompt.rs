//! Prompt and schema construction for agent model calls.
//!
//! This module translates browser state, history, settings, managed-file state,
//! and tool schemas into provider-neutral [`ChatRequest`] values. The functions
//! here are deliberately deterministic because tests compare their output
//! against browser-use compatibility expectations.
//!
//! Prompt building is intentionally a pure data transformation: the browser has
//! already been observed, history has already been recorded, and this module
//! only decides what the model is allowed to see and return.
//!
//! ```mermaid
//! flowchart LR
//!     State["BrowserStateSummary"] --> Text["browser state JSON/text"]
//!     History["AgentHistory"] --> Prior["previous results"]
//!     Files["ManagedFileSystem"] --> FileCtx["available/read files"]
//!     Settings["AgentSettings"] --> Policy["vision, actions, secrets, schema"]
//!     Text --> Request["ChatRequest"]
//!     Prior --> Request
//!     FileCtx --> Request
//!     Policy --> Request
//!     Policy --> Schema["AgentOutput JSON Schema"]
//!     Schema --> Compat["schema_to_compat_value"]
//!     Compat --> Request
//! ```

use crate::{
    ActionResult, AgentHistory, AgentHistoryItem, AgentRunError, AgentSettings, BrowserAction,
    BrowserStateSummary, LlmScreenshotSize, ManagedFileSystem, MessageCompactionSettings,
    SensitiveDataValue, now_seconds,
};
use base64::Engine;
use browser_use_llm::{ChatMessage, ChatRequest, ContentPart, ImageDetailLevel, MessageRole};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

mod schema;

pub use schema::schema_to_compat_value;
pub(crate) use schema::{
    excluded_action_error, schema_for_agent_output_with_settings,
    schema_for_final_response_after_failure,
};
#[cfg(test)]
pub(crate) use schema::{schema_for_agent_output, schema_variant_action_name};
use schema::{schema_for_judgement_result, schema_for_message_compaction_output};

pub(crate) const MAX_PROMPT_CONTENT_CHARS: usize = 60_000;
const MAX_PROMPT_ERROR_CHARS: usize = 200;
const PROMPT_ERROR_EDGE_CHARS: usize = 100;

pub(crate) fn actions_for_execution(
    actions: &[BrowserAction],
    settings: &AgentSettings,
    current_url: &str,
) -> Vec<BrowserAction> {
    let sensitive_data = applicable_sensitive_data_values(&settings.sensitive_data, current_url);
    if sensitive_data.is_empty() && settings.extraction_schema.is_none() {
        return actions.to_vec();
    }

    actions
        .iter()
        .map(|action| {
            // Execution receives a copy of the model action, not a mutation of
            // history. That separation is why redacted prompt/history records
            // can coexist with real secrets and default extraction schemas at
            // the browser side-effect boundary.
            let action =
                action_with_default_extraction_schema(action, settings.extraction_schema.as_ref());
            if sensitive_data.is_empty() {
                return action;
            }
            let Ok(mut value) = serde_json::to_value(&action) else {
                return action.clone();
            };
            replace_sensitive_placeholders_in_value(&mut value, &sensitive_data);
            serde_json::from_value(value).unwrap_or_else(|_| action.clone())
        })
        .collect()
}

fn action_with_default_extraction_schema(
    action: &BrowserAction,
    extraction_schema: Option<&Value>,
) -> BrowserAction {
    let (BrowserAction::Extract(params), Some(schema)) = (action, extraction_schema) else {
        return action.clone();
    };
    if params.output_schema.is_some() {
        return action.clone();
    }

    let mut params = params.clone();
    params.output_schema = Some(schema.clone());
    BrowserAction::Extract(params)
}

pub(crate) fn scale_coordinate_click_actions_for_prompt(
    actions: &[BrowserAction],
    settings: &AgentSettings,
    state: &BrowserStateSummary,
) -> Vec<BrowserAction> {
    let Some(size) = settings.llm_screenshot_size else {
        return actions.to_vec();
    };
    let Some(page_info) = state.page_info else {
        return actions.to_vec();
    };
    if page_info.viewport_width == 0 || page_info.viewport_height == 0 {
        return actions.to_vec();
    }

    actions
        .iter()
        .map(|action| match action {
            BrowserAction::Click(params)
                if params.index.is_none()
                    && params.coordinate_x.is_some()
                    && params.coordinate_y.is_some() =>
            {
                // Vision models may see a downscaled screenshot, so coordinate
                // clicks are first interpreted in screenshot space and then
                // mapped back to the live viewport before CDP receives them.
                let mut scaled = params.clone();
                scaled.coordinate_x = scaled
                    .coordinate_x
                    .map(|x| scale_llm_coordinate(x, size.width(), page_info.viewport_width));
                scaled.coordinate_y = scaled
                    .coordinate_y
                    .map(|y| scale_llm_coordinate(y, size.height(), page_info.viewport_height));
                BrowserAction::Click(scaled)
            }
            _ => action.clone(),
        })
        .collect()
}

fn scale_llm_coordinate(coordinate: i32, llm_dimension: u32, viewport_dimension: u32) -> i32 {
    if llm_dimension == 0 {
        return coordinate;
    }
    ((f64::from(coordinate) / f64::from(llm_dimension)) * f64::from(viewport_dimension)).trunc()
        as i32
}

fn replace_sensitive_placeholders_in_value(
    value: &mut Value,
    sensitive_data: &BTreeMap<String, String>,
) {
    match value {
        Value::String(text) => {
            *text = replace_sensitive_placeholders_in_string(text, sensitive_data);
        }
        Value::Array(items) => {
            for item in items {
                replace_sensitive_placeholders_in_value(item, sensitive_data);
            }
        }
        Value::Object(entries) => {
            for entry in entries.values_mut() {
                replace_sensitive_placeholders_in_value(entry, sensitive_data);
            }
        }
        Value::Null | Value::Bool(_) | Value::Number(_) => {}
    }
}

fn replace_sensitive_placeholders_in_string(
    text: &str,
    sensitive_data: &BTreeMap<String, String>,
) -> String {
    // Both whole-value placeholders and tagged placeholders are supported:
    // `<secret>key</secret>` is convenient in natural-language params, while a
    // bare key is useful when a model returns exactly the placeholder value.
    let secret_pattern =
        regex::Regex::new(r"<secret>(.*?)</secret>").expect("valid secret tag regex");
    let replaced = secret_pattern
        .replace_all(text, |captures: &regex::Captures<'_>| {
            let placeholder = captures.get(1).map(|match_| match_.as_str()).unwrap_or("");
            sensitive_replacement_value(placeholder, sensitive_data)
                .unwrap_or_else(|| captures[0].to_owned())
        })
        .into_owned();

    sensitive_replacement_value(&replaced, sensitive_data).unwrap_or(replaced)
}

fn sensitive_replacement_value(
    placeholder: &str,
    sensitive_data: &BTreeMap<String, String>,
) -> Option<String> {
    let secret = sensitive_data.get(placeholder)?;
    if placeholder.ends_with("bu_2fa_code") {
        return totp_code(secret, now_seconds() as u64);
    }

    Some(secret.clone())
}

fn totp_code(secret: &str, unix_seconds: u64) -> Option<String> {
    totp_code_at(secret, unix_seconds, 30, 6)
}

pub(crate) fn totp_code_at(
    secret: &str,
    unix_seconds: u64,
    period_seconds: u64,
    digits: u32,
) -> Option<String> {
    if period_seconds == 0 || digits == 0 || digits > 9 {
        return None;
    }

    let normalized_secret = secret
        .chars()
        .filter(|character| !character.is_whitespace())
        .collect::<String>()
        .to_ascii_uppercase();
    let unpadded_secret = normalized_secret.trim_end_matches('=');
    let key_bytes = data_encoding::BASE32_NOPAD
        .decode(unpadded_secret.as_bytes())
        .or_else(|_| data_encoding::BASE32.decode(normalized_secret.as_bytes()))
        .ok()?;
    let counter = unix_seconds / period_seconds;
    let message = counter.to_be_bytes();
    let key = ring::hmac::Key::new(ring::hmac::HMAC_SHA1_FOR_LEGACY_USE_ONLY, &key_bytes);
    let tag = ring::hmac::sign(&key, &message);
    let digest = tag.as_ref();
    let offset = usize::from(digest.last()? & 0x0f);
    let binary = (u32::from(digest.get(offset)? & 0x7f) << 24)
        | (u32::from(*digest.get(offset + 1)?) << 16)
        | (u32::from(*digest.get(offset + 2)?) << 8)
        | u32::from(*digest.get(offset + 3)?);
    let code = binary % 10_u32.pow(digits);

    Some(format!("{code:0width$}", width = digits as usize))
}

/// Builds the normal step request without managed file-system context.
pub fn build_step_request(
    task: &str,
    state: &BrowserStateSummary,
    history: &AgentHistory,
    settings: &AgentSettings,
) -> Result<ChatRequest, AgentRunError> {
    build_step_request_with_file_system(task, state, history, settings, None)
}

/// Builds the normal step request with optional managed file-system context.
pub fn build_step_request_with_file_system(
    task: &str,
    state: &BrowserStateSummary,
    history: &AgentHistory,
    settings: &AgentSettings,
    file_system: Option<&ManagedFileSystem>,
) -> Result<ChatRequest, AgentRunError> {
    let mut state_for_text = state.clone();
    state_for_text.screenshot = None;
    if !settings.include_recent_events {
        state_for_text.recent_events = None;
    }
    if !settings.include_attributes.is_empty() {
        state_for_text.dom_state.text = state
            .dom_state
            .llm_representation_with_attributes(&settings.include_attributes);
    }
    state_for_text.dom_state.text = truncate_clickable_elements_text(
        &state_for_text.dom_state.text,
        settings.max_clickable_elements_length,
    );
    let state_json = serde_json::to_string_pretty(&state_for_text)
        .map_err(|error| AgentRunError::InvalidOutput(error.to_string()))?;
    let agent_history = render_previous_results(history, settings.max_history_items);
    let page_stats = render_page_stats(state);
    let agent_state =
        render_agent_state_description(task, &page_stats, history, state, settings, file_system);
    let read_state = render_read_state_description(history)
        .map(|description| format!("\n<read_state>\n{description}\n</read_state>\n"))
        .unwrap_or_default();
    let sensitive_values = collect_sensitive_data_values(&settings.sensitive_data);
    let user_text = redact_sensitive_string(
        &format!(
            "<agent_history>\n{agent_history}\n</agent_history>\n\n<agent_state>\n{agent_state}\n</agent_state>\n<browser_state>\n{state_json}\n</browser_state>{read_state}"
        ),
        &sensitive_values,
    );
    let mut user_content = vec![ContentPart::Text { text: user_text }];
    user_content.extend(settings.sample_images.iter().cloned());
    if settings.use_vision.accepts_prompt_image()
        && let Some(screenshot) = state.screenshot.as_deref()
    {
        user_content.push(ContentPart::ImageUrl {
            image_url: prompt_screenshot_data_url(screenshot, settings.llm_screenshot_size),
            detail: Some(settings.vision_detail_level),
        });
    }
    append_latest_action_result_images(&mut user_content, history, settings.vision_detail_level);
    Ok(ChatRequest {
        messages: vec![
            ChatMessage::text(MessageRole::System, render_system_message(settings)),
            ChatMessage {
                role: MessageRole::User,
                content: user_content,
            },
        ],
        output_schema: Some(schema_for_agent_output_with_settings(settings)),
    })
}

fn render_system_message(settings: &AgentSettings) -> String {
    let mut message = settings.override_system_message.clone().unwrap_or_else(|| {
        format!(
            "You are controlling a browser. Return a JSON object matching AgentOutput. \
	         Use at most {} actions in this step. Avoid repeating the same action \
	         sequence; if the browser is not changing, choose a different strategy \
	         or finish with done.",
            settings.max_actions_per_step
        )
    });
    if let Some(extension) = settings
        .extend_system_message
        .as_deref()
        .filter(|extension| !extension.is_empty())
    {
        message.push('\n');
        message.push_str(extension);
    }

    message
}

pub(crate) fn build_final_response_after_failure_request(
    task: &str,
    state: &BrowserStateSummary,
    history: &AgentHistory,
    settings: &AgentSettings,
    file_system: Option<&ManagedFileSystem>,
    failures: u32,
) -> Result<ChatRequest, AgentRunError> {
    let mut request =
        build_step_request_with_file_system(task, state, history, settings, file_system)?;
    request.output_schema = Some(schema_for_final_response_after_failure(settings));
    let instruction = format!(
        "You failed {failures} times. We are terminating the agent. Your only available action is done. Return exactly one done action. \
         If the task is not fully finished, set success to false. Include everything useful you found for the original task in done.text."
    );
    if let Some(message) = request
        .messages
        .iter_mut()
        .find(|message| message.role == MessageRole::User)
    {
        message
            .content
            .push(ContentPart::Text { text: instruction });
    }
    Ok(request)
}

pub(crate) fn should_inject_step_budget_warning(steps_used: usize, max_steps: usize) -> bool {
    max_steps > 0
        && steps_used < max_steps
        && steps_used.saturating_mul(4) >= max_steps.saturating_mul(3)
}

pub(crate) fn build_step_request_with_budget_warning(
    task: &str,
    state: &BrowserStateSummary,
    history: &AgentHistory,
    settings: &AgentSettings,
    file_system: Option<&ManagedFileSystem>,
    steps_used: usize,
    max_steps: usize,
) -> Result<ChatRequest, AgentRunError> {
    let mut request =
        build_step_request_with_file_system(task, state, history, settings, file_system)?;
    let steps_remaining = max_steps.saturating_sub(steps_used);
    let pct = steps_used
        .saturating_mul(100)
        .checked_div(max_steps)
        .unwrap_or(0);
    let instruction = format!(
        "BUDGET WARNING: You have used {steps_used}/{max_steps} steps ({pct}%). {steps_remaining} steps remaining. \
         If the task cannot be completed in the remaining steps, prioritize: \
         (1) consolidate your results (save to files if the file system is in use), \
         (2) call done with what you have. \
         Partial results are far more valuable than exhausting all steps with nothing saved."
    );
    if let Some(message) = request
        .messages
        .iter_mut()
        .find(|message| message.role == MessageRole::User)
    {
        message
            .content
            .push(ContentPart::Text { text: instruction });
    }
    Ok(request)
}

pub(crate) fn build_final_response_after_step_limit_request(
    task: &str,
    state: &BrowserStateSummary,
    history: &AgentHistory,
    settings: &AgentSettings,
    file_system: Option<&ManagedFileSystem>,
    max_steps: usize,
) -> Result<ChatRequest, AgentRunError> {
    let mut request =
        build_step_request_with_file_system(task, state, history, settings, file_system)?;
    request.output_schema = Some(schema_for_final_response_after_failure(settings));
    let instruction = format!(
        "You reached max_steps ({max_steps}) - this is your last step. Your only available action is done. \
         Return exactly one done action. If the task is not fully finished, set success to false. \
         Include everything useful you found for the original task in done.text."
    );
    if let Some(message) = request
        .messages
        .iter_mut()
        .find(|message| message.role == MessageRole::User)
    {
        message
            .content
            .push(ContentPart::Text { text: instruction });
    }
    Ok(request)
}

pub(crate) fn build_judge_request(
    task: &str,
    history: &AgentHistory,
    settings: &AgentSettings,
) -> ChatRequest {
    let final_result = history.final_result().unwrap_or_default();
    let trajectory = render_judge_trajectory(history);
    let ground_truth = settings
        .ground_truth
        .as_deref()
        .map(|ground_truth| {
            format!(
                "\n<ground_truth>\n{}\n</ground_truth>\n",
                truncate_judge_text(ground_truth)
            )
        })
        .unwrap_or_default();
    let user_prompt = format!(
        "<task>\n{}\n</task>\n{ground_truth}<agent_trajectory>\n{}\n</agent_trajectory>\n\n<final_result>\n{}\n</final_result>\n\nEvaluate this agent execution and respond with the exact JSON object requested.",
        truncate_judge_text(task),
        truncate_judge_text(&trajectory),
        truncate_judge_text(final_result)
    );
    let mut user_content = vec![ContentPart::Text { text: user_prompt }];
    if settings.use_vision.accepts_prompt_image() {
        for screenshot in history.screenshots(Some(10), false).into_iter().flatten() {
            user_content.push(ContentPart::ImageUrl {
                image_url: prompt_screenshot_data_url(screenshot, settings.llm_screenshot_size),
                detail: Some(settings.vision_detail_level),
            });
        }
    }

    ChatRequest {
        messages: vec![
            ChatMessage::text(MessageRole::System, render_judge_system_message()),
            ChatMessage {
                role: MessageRole::User,
                content: user_content,
            },
        ],
        output_schema: Some(schema_for_judgement_result()),
    }
}

pub(crate) fn build_message_compaction_request(
    history: &AgentHistory,
    settings: &MessageCompactionSettings,
    sensitive_data: &BTreeMap<String, SensitiveDataValue>,
) -> ChatRequest {
    let full_history_text = render_history_items_for_compaction(history);
    let mut sections = Vec::new();
    if let Some(memory) = non_empty_prompt_text(history.compacted_memory.as_deref()) {
        sections.push(format!(
            "<previous_compacted_memory>\n{memory}\n</previous_compacted_memory>"
        ));
    }
    sections.push(format!(
        "<agent_history>\n{full_history_text}\n</agent_history>"
    ));
    if settings.include_read_state
        && let Some(read_state) = render_read_state_description(history)
    {
        sections.push(format!("<read_state>\n{read_state}\n</read_state>"));
    }
    let sensitive_values = collect_sensitive_data_values(sensitive_data);
    let user_prompt = redact_sensitive_string(&sections.join("\n\n"), &sensitive_values);

    let mut system_prompt = "You are summarizing an agent run for prompt compaction.\n\
         Capture task requirements, key facts, decisions, partial progress, errors, and next steps.\n\
         Preserve important entities, values, URLs, and file paths.\n\
         CRITICAL: Only mark a step as completed if you see explicit success confirmation in the history. \
         If a step was started but not explicitly confirmed complete, mark it as \"IN-PROGRESS\". \
         Never infer completion from context - only report what was confirmed.\n\
         Respond with exactly a JSON object matching MessageCompactionOutput: summary. Do not add prose outside JSON."
        .to_owned();
    if settings.summary_max_chars > 0 {
        system_prompt.push_str(&format!(
            " Keep summary under {} characters if possible.",
            settings.summary_max_chars
        ));
    }

    ChatRequest {
        messages: vec![
            ChatMessage::text(MessageRole::System, system_prompt),
            ChatMessage::text(MessageRole::User, user_prompt),
        ],
        output_schema: Some(schema_for_message_compaction_output()),
    }
}

fn render_judge_system_message() -> String {
    "You are an expert judge evaluating browser automation agent performance.\n\
     Decide whether the agent satisfied the user task, whether the final output is complete, \
     whether browser/tool actions appear effective, and whether any captcha or impossible-task \
     condition blocked success. Ground truth, when provided, has highest priority.\n\
     Respond with exactly a JSON object matching JudgementResult: reasoning, verdict, \
     failure_reason, impossible_task, reached_captcha. Do not add prose outside JSON."
        .to_owned()
}

fn render_judge_trajectory(history: &AgentHistory) -> String {
    history
        .items
        .iter()
        .enumerate()
        .map(|(index, item)| {
            let model_output = item
                .model_output
                .as_ref()
                .and_then(|output| serde_json::to_string_pretty(output).ok())
                .unwrap_or_else(|| "null".to_owned());
            let result =
                serde_json::to_string_pretty(&item.result).unwrap_or_else(|_| "[]".to_owned());
            format!(
                "Step {}\nURL: {}\nTitle: {}\nModel output:\n{}\nAction result:\n{}",
                index + 1,
                item.state.url,
                item.state.title,
                model_output,
                result
            )
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

fn truncate_judge_text(text: &str) -> String {
    const MAX_CHARS: usize = 40_000;
    if text.chars().count() <= MAX_CHARS {
        return text.to_owned();
    }
    let truncated = text
        .chars()
        .take(MAX_CHARS.saturating_sub(23))
        .collect::<String>();
    format!("{truncated}...[text truncated]...")
}

pub(crate) fn latest_history_step_number(history: &AgentHistory) -> Option<usize> {
    history.items.last()?;
    Some(
        history
            .items
            .iter()
            .filter_map(|item| item.metadata.as_ref().map(|metadata| metadata.step_number))
            .max()
            .unwrap_or(history.items.len()),
    )
}

pub(crate) fn retain_first_and_recent_history_items(
    history: &mut AgentHistory,
    keep_last_items: usize,
) {
    let keep_with_first = keep_last_items.saturating_add(1);
    if history.items.len() <= keep_with_first {
        return;
    }

    let first = history.items[0].clone();
    let mut retained = vec![first];
    if keep_last_items > 0 {
        let recent_start = history.items.len().saturating_sub(keep_last_items);
        retained.extend(history.items[recent_start..].iter().cloned());
    }
    history.items = retained;
}

fn screenshot_data_url(screenshot: &str) -> String {
    if screenshot.starts_with("data:image/") {
        screenshot.to_owned()
    } else {
        format!("data:image/png;base64,{screenshot}")
    }
}

fn prompt_screenshot_data_url(screenshot: &str, size: Option<LlmScreenshotSize>) -> String {
    size.and_then(|size| resize_screenshot_for_prompt(screenshot, size))
        .unwrap_or_else(|| screenshot_data_url(screenshot))
}

fn resize_screenshot_for_prompt(screenshot: &str, size: LlmScreenshotSize) -> Option<String> {
    let base64_png = screenshot_base64_payload(screenshot);
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(base64_png)
        .ok()?;
    let image = image::load_from_memory(&bytes).ok()?;
    if image.width() == size.width() && image.height() == size.height() {
        return Some(screenshot_data_url(screenshot));
    }

    let resized = image.resize_exact(
        size.width(),
        size.height(),
        image::imageops::FilterType::Lanczos3,
    );
    let mut buffer = std::io::Cursor::new(Vec::new());
    resized
        .write_to(&mut buffer, image::ImageFormat::Png)
        .ok()?;
    Some(format!(
        "data:image/png;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(buffer.into_inner())
    ))
}

fn screenshot_base64_payload(screenshot: &str) -> &str {
    if let Some((prefix, payload)) = screenshot.split_once(',')
        && prefix.starts_with("data:image/")
    {
        payload
    } else {
        screenshot
    }
}

fn append_latest_action_result_images(
    content: &mut Vec<ContentPart>,
    history: &AgentHistory,
    vision_detail_level: ImageDetailLevel,
) {
    let Some(latest) = history.items.last() else {
        return;
    };

    for image in latest.result.iter().flat_map(|result| result.images.iter()) {
        let Some(data) = image.get("data").and_then(Value::as_str) else {
            continue;
        };
        if data.is_empty() {
            continue;
        }
        let name = image
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or("unknown");

        content.push(ContentPart::Text {
            text: format!("Image from file: {name}"),
        });
        content.push(ContentPart::ImageUrl {
            image_url: action_result_image_data_url(name, data),
            detail: Some(vision_detail_level),
        });
    }
}

fn action_result_image_data_url(name: &str, data: &str) -> String {
    if data.starts_with("data:image/") {
        return data.to_owned();
    }

    let media_type = if name.to_ascii_lowercase().ends_with(".png") {
        "image/png"
    } else {
        "image/jpeg"
    };
    format!("data:{media_type};base64,{data}")
}

fn truncate_clickable_elements_text(text: &str, max_chars: usize) -> String {
    if max_chars == 0 {
        return "[clickable elements omitted by max_clickable_elements_length]".to_owned();
    }
    if text.chars().count() <= max_chars {
        return text.to_owned();
    }

    let truncated = text.chars().take(max_chars).collect::<String>();
    format!("{truncated}\n...[clickable elements truncated to {max_chars} chars]")
}

fn render_page_stats(state: &BrowserStateSummary) -> String {
    let stats = if state.dom_state.page_stats.is_empty() {
        fallback_page_stats(state)
    } else {
        state.dom_state.page_stats
    };
    let mut stats_text = "<page_stats>".to_owned();
    if stats.total_elements < 10 {
        stats_text.push_str("Page appears empty (SPA not loaded?) - ");
    } else if stats.total_elements > 20 && stats.text_chars < stats.total_elements.saturating_mul(5)
    {
        stats_text
            .push_str("Page appears to show skeleton/placeholder content (still loading?) - ");
    }
    stats_text.push_str(&format!(
        "{} links, {} interactive, {} iframes",
        stats.links, stats.interactive_elements, stats.iframes
    ));
    if stats.shadow_open > 0 || stats.shadow_closed > 0 {
        stats_text.push_str(&format!(
            ", {} shadow(open), {} shadow(closed)",
            stats.shadow_open, stats.shadow_closed
        ));
    }
    if stats.images > 0 {
        stats_text.push_str(&format!(", {} images", stats.images));
    }
    stats_text.push_str(&format!(
        ", {} scroll containers, {} total elements, {} text chars",
        stats.scroll_containers, stats.total_elements, stats.text_chars
    ));

    if let Some(page_info) = state.page_info {
        stats_text.push_str(&format!(
            ", {}px above, {}px below",
            page_info.pixels_above, page_info.pixels_below
        ));
    }

    stats_text.push_str("</page_stats>");
    stats_text
}

fn fallback_page_stats(state: &BrowserStateSummary) -> browser_use_dom::DomPageStats {
    let indexed_elements = state.dom_state.selector_map.values();
    browser_use_dom::DomPageStats {
        links: indexed_elements
            .clone()
            .filter(|element| element.tag_name == "a")
            .count() as u32,
        iframes: indexed_elements
            .clone()
            .filter(|element| matches!(element.tag_name.as_str(), "iframe" | "frame"))
            .count() as u32,
        scroll_containers: indexed_elements
            .clone()
            .filter(|element| element.is_scrollable)
            .count() as u32,
        interactive_elements: indexed_elements
            .clone()
            .filter(|element| element.is_interactive)
            .count() as u32,
        total_elements: state.dom_state.selector_map.len() as u32,
        text_chars: state.dom_state.text.chars().count() as u32,
        ..browser_use_dom::DomPageStats::default()
    }
}

fn render_agent_state_description(
    task: &str,
    page_stats: &str,
    history: &AgentHistory,
    state: &BrowserStateSummary,
    settings: &AgentSettings,
    file_system: Option<&ManagedFileSystem>,
) -> String {
    let mut description = format!("Task:\n{task}\n\nPage stats:\n{page_stats}");
    if let Some(file_system) = file_system {
        let todo_contents = file_system.get_todo_contents();
        let todo_contents = if todo_contents.is_empty() {
            "[empty todo.md, fill it when applicable]".to_owned()
        } else {
            todo_contents
        };
        description.push_str(&format!(
            "\n\n<file_system>\n{}\n</file_system>\n<todo_contents>\n{todo_contents}\n</todo_contents>",
            file_system.describe()
        ));
    }
    if let Some(message) = render_planning_context(history, settings) {
        description.push_str(&format!("\n\nPlanning:\n{message}"));
    }
    if let Some(message) = render_loop_awareness(history, state, settings) {
        description.push_str(&format!("\n\nLoop awareness:\n{message}"));
    }
    if let Some(message) = render_sensitive_data_description(&state.url, settings) {
        description.push_str(&format!("\n\n<sensitive_data>{message}</sensitive_data>"));
    }
    if !settings.available_file_paths.is_empty() {
        description.push_str(&format!(
            "\n\n<available_file_paths>{}\nUse with absolute paths</available_file_paths>",
            settings.available_file_paths.join("\n")
        ));
    }
    description
}

fn render_sensitive_data_description(
    current_url: &str,
    settings: &AgentSettings,
) -> Option<String> {
    let placeholders = sensitive_data_placeholders_for_url(&settings.sensitive_data, current_url);
    if placeholders.is_empty() {
        return None;
    }

    let first = placeholders.first().expect("placeholder exists");
    let formatted_placeholders = placeholders
        .iter()
        .map(|placeholder| format!("  - {placeholder}"))
        .collect::<Vec<_>>()
        .join("\n");

    Some(format!(
        "SENSITIVE DATA - Use these placeholders for secure input:\n{formatted_placeholders}\n\nIMPORTANT: When entering sensitive values, you MUST wrap the placeholder name in <secret> tags.\nExample: To enter the value for \"{first}\", use: <secret>{first}</secret>\nThe system will automatically replace these tags with the actual secret values."
    ))
}

fn sensitive_data_placeholders_for_url(
    sensitive_data: &BTreeMap<String, SensitiveDataValue>,
    current_url: &str,
) -> Vec<String> {
    let mut placeholders = BTreeSet::new();
    for (key_or_domain, value) in sensitive_data {
        match value {
            SensitiveDataValue::Value(_) => {
                placeholders.insert(key_or_domain.clone());
            }
            SensitiveDataValue::Domain(domain_values)
                if match_url_with_domain_pattern(current_url, key_or_domain) =>
            {
                placeholders.extend(domain_values.keys().cloned());
            }
            SensitiveDataValue::Domain(_) => {}
        }
    }

    placeholders.into_iter().collect()
}

fn collect_sensitive_data_values(
    sensitive_data: &BTreeMap<String, SensitiveDataValue>,
) -> BTreeMap<String, String> {
    let mut values = BTreeMap::new();
    for (key_or_domain, value) in sensitive_data {
        match value {
            SensitiveDataValue::Value(secret) if !secret.is_empty() => {
                values.insert(key_or_domain.clone(), secret.clone());
            }
            SensitiveDataValue::Value(_) => {}
            SensitiveDataValue::Domain(domain_values) => {
                for (placeholder, secret) in domain_values {
                    if !secret.is_empty() {
                        values.insert(placeholder.clone(), secret.clone());
                    }
                }
            }
        }
    }

    values
}

fn applicable_sensitive_data_values(
    sensitive_data: &BTreeMap<String, SensitiveDataValue>,
    current_url: &str,
) -> BTreeMap<String, String> {
    let mut values = BTreeMap::new();
    for (key_or_domain, value) in sensitive_data {
        match value {
            SensitiveDataValue::Value(secret) if !secret.is_empty() => {
                values.insert(key_or_domain.clone(), secret.clone());
            }
            SensitiveDataValue::Value(_) => {}
            SensitiveDataValue::Domain(secrets)
                if match_url_with_domain_pattern(current_url, key_or_domain) =>
            {
                for (placeholder, secret) in secrets {
                    if !secret.is_empty() {
                        values.insert(placeholder.clone(), secret.clone());
                    }
                }
            }
            SensitiveDataValue::Domain(_) => {}
        }
    }

    values
}

fn redact_sensitive_string(value: &str, sensitive_values: &BTreeMap<String, String>) -> String {
    let mut redacted = value.to_owned();
    let mut entries = sensitive_values.iter().collect::<Vec<_>>();
    entries.sort_by_key(|entry| std::cmp::Reverse(entry.1.len()));
    for (placeholder, secret) in entries {
        redacted = redacted.replace(secret, &format!("<secret>{placeholder}</secret>"));
    }

    redacted
}

pub(crate) fn match_url_with_domain_pattern(url: &str, domain_pattern: &str) -> bool {
    if is_new_tab_page(url) {
        return false;
    }

    let Ok(parsed_url) = url::Url::parse(url) else {
        return false;
    };
    let scheme = parsed_url.scheme().to_ascii_lowercase();
    let Some(domain) = parsed_url.host_str().map(str::to_ascii_lowercase) else {
        return false;
    };
    if scheme.is_empty() || domain.is_empty() {
        return false;
    }

    let domain_pattern = domain_pattern.to_ascii_lowercase();
    let (pattern_scheme, pattern_domain) = domain_pattern
        .split_once("://")
        .map_or(("https", domain_pattern.as_str()), |(scheme, domain)| {
            (scheme, domain)
        });
    let pattern_domain = pattern_domain
        .split_once(':')
        .map_or(pattern_domain, |(domain, _)| domain);

    if !glob_match(&scheme, pattern_scheme) {
        return false;
    }
    if pattern_domain == "*" || domain == pattern_domain {
        return true;
    }

    if !pattern_domain.contains('*') {
        return false;
    }
    if pattern_domain.matches("*.").count() > 1 || pattern_domain.matches(".*").count() > 1 {
        return false;
    }
    if pattern_domain.ends_with(".*") {
        return false;
    }
    let bare_domain = pattern_domain.replace("*.", "");
    if bare_domain.contains('*') {
        return false;
    }

    if let Some(parent_domain) = pattern_domain.strip_prefix("*.")
        && domain == parent_domain
    {
        return true;
    }

    glob_match(&domain, pattern_domain)
}

fn is_new_tab_page(url: &str) -> bool {
    matches!(
        url,
        "about:blank"
            | "chrome://new-tab-page/"
            | "chrome://new-tab-page"
            | "chrome://newtab/"
            | "chrome://newtab"
    )
}

fn glob_match(value: &str, pattern: &str) -> bool {
    let pattern = format!("^{}$", regex::escape(pattern).replace("\\*", ".*"));
    regex::Regex::new(&pattern)
        .map(|regex| regex.is_match(value))
        .unwrap_or(false)
}

pub(crate) fn render_previous_results(
    history: &AgentHistory,
    max_history_items: Option<usize>,
) -> String {
    enum HistoryPromptEntry<'a> {
        Item(&'a AgentHistoryItem),
        Omitted(usize),
    }

    let total_items = history.items.len();
    let entries: Vec<HistoryPromptEntry<'_>> = match max_history_items {
        None => history.items.iter().map(HistoryPromptEntry::Item).collect(),
        Some(max_history_items) if total_items <= max_history_items => {
            history.items.iter().map(HistoryPromptEntry::Item).collect()
        }
        Some(0) => vec![HistoryPromptEntry::Omitted(total_items)],
        Some(max_history_items) => {
            let omitted_count = total_items - max_history_items;
            let recent_items_count = max_history_items - 1;
            let recent_start = total_items.saturating_sub(recent_items_count);
            let mut entries = vec![
                HistoryPromptEntry::Item(&history.items[0]),
                HistoryPromptEntry::Omitted(omitted_count),
            ];
            entries.extend(
                history
                    .items
                    .iter()
                    .skip(recent_start)
                    .map(HistoryPromptEntry::Item),
            );
            entries
        }
    };

    let mut rendered = Vec::new();
    if let Some(memory) = non_empty_prompt_text(history.compacted_memory.as_deref()) {
        rendered.push(render_compacted_memory(memory));
    }
    if history.items.is_empty() {
        rendered.push("Agent initialized".to_owned());
    }
    for entry in entries {
        match entry {
            HistoryPromptEntry::Item(item) => {
                if let Some(item_text) = render_history_item_for_prompt(item) {
                    rendered.push(item_text);
                }
            }
            HistoryPromptEntry::Omitted(omitted_count) if omitted_count > 0 => {
                rendered.push(format!(
                    "<sys>[... {omitted_count} previous steps omitted...]</sys>"
                ));
            }
            HistoryPromptEntry::Omitted(_) => {}
        }
    }

    truncate_prompt_content(rendered.join("\n"))
}

fn render_compacted_memory(memory: &str) -> String {
    format!(
        "<compacted_memory>\n\
         <!-- Summary of prior steps. Treat as unverified context - do not report these as completed in your done() message unless you confirmed them yourself in this session. -->\n\
         {memory}\n\
         </compacted_memory>"
    )
}

pub(crate) fn render_history_items_for_compaction(history: &AgentHistory) -> String {
    let mut rendered = if history.items.is_empty() {
        vec!["Agent initialized".to_owned()]
    } else {
        Vec::new()
    };
    for item in &history.items {
        if let Some(item_text) = render_history_item_for_prompt(item) {
            rendered.push(item_text);
        }
    }
    truncate_prompt_content(rendered.join("\n"))
}

fn render_history_item_for_prompt(item: &AgentHistoryItem) -> Option<String> {
    let mut content_parts = Vec::new();
    if let Some(output) = item.model_output.as_ref() {
        let brain = output.current_brain();
        if let Some(evaluation) = non_empty_prompt_text(brain.evaluation_previous_goal.as_deref()) {
            content_parts.push(evaluation.to_owned());
        }
        if let Some(memory) = non_empty_prompt_text(brain.memory.as_deref()) {
            content_parts.push(memory.to_owned());
        }
        if let Some(next_goal) = non_empty_prompt_text(brain.next_goal.as_deref()) {
            content_parts.push(next_goal.to_owned());
        }
    }
    if let Some(action_results) = render_action_results_for_prompt(&item.result) {
        content_parts.push(action_results);
    }

    (!content_parts.is_empty()).then(|| format!("<step>\n{}", content_parts.join("\n")))
}

fn render_action_results_for_prompt(results: &[ActionResult]) -> Option<String> {
    let mut lines = Vec::new();
    for result in results {
        if let Some(memory) = non_empty_prompt_text(result.long_term_memory.as_deref()) {
            lines.push(memory.to_owned());
        } else if !result.include_extracted_content_only_once
            && let Some(content) = non_empty_prompt_text(result.extracted_content.as_deref())
        {
            lines.push(content.to_owned());
        }

        if let Some(error) = non_empty_prompt_text(result.error.as_deref()) {
            lines.push(truncate_error_for_prompt(error));
        }
    }

    if lines.is_empty() {
        None
    } else {
        Some(truncate_prompt_content(format!(
            "Result\n{}",
            lines.join("\n")
        )))
    }
}

pub(crate) fn render_read_state_description(history: &AgentHistory) -> Option<String> {
    let latest = history.items.last()?;
    let mut blocks = Vec::new();
    for result in &latest.result {
        if result.include_extracted_content_only_once
            && let Some(extracted_content) =
                non_empty_prompt_text(result.extracted_content.as_deref())
        {
            let index = blocks.len();
            blocks.push(format!(
                "<read_state_{index}>\n{extracted_content}\n</read_state_{index}>"
            ));
        }
    }

    if blocks.is_empty() {
        None
    } else {
        Some(truncate_prompt_content(blocks.join("\n")))
    }
}

fn non_empty_prompt_text(text: Option<&str>) -> Option<&str> {
    text.filter(|value| !value.is_empty())
}

fn render_planning_context(history: &AgentHistory, settings: &AgentSettings) -> Option<String> {
    if !settings.enable_planning || settings.flash_mode {
        return None;
    }

    let steps_without_plan_update = history
        .items
        .iter()
        .rev()
        .take_while(|item| {
            item.model_output
                .as_ref()
                .and_then(|output| output.plan_update.as_ref())
                .is_none()
        })
        .count();
    let recent_failures = history
        .items
        .iter()
        .rev()
        .take_while(|item| item.result.iter().any(|result| result.error.is_some()))
        .count();

    let mut message = format!(
        "When useful, include `current_plan_item` and `plan_update` to keep multi-step work explicit. Replan after {} stalled/error steps; avoid exploring for more than {} steps without a plan update.",
        settings.planning_replan_on_stall, settings.planning_exploration_limit
    );

    if settings.planning_replan_on_stall > 0 && recent_failures >= settings.planning_replan_on_stall
    {
        message.push_str(
            " Recent steps have failed or stalled, so revise the plan before continuing.",
        );
    } else if settings.planning_exploration_limit > 0
        && steps_without_plan_update >= settings.planning_exploration_limit
    {
        message.push_str(" You have explored for several steps without updating the plan; provide a concise plan_update.");
    }

    Some(message)
}

fn render_loop_awareness(
    history: &AgentHistory,
    state: &BrowserStateSummary,
    settings: &AgentSettings,
) -> Option<String> {
    if !settings.loop_detection_enabled {
        return None;
    }

    let mut messages = Vec::new();
    if let Some((count, window)) = repeated_action_nudge(history, settings.loop_detection_window) {
        messages.push(format!(
            "Heads up: you have repeated a similar action {count} times in the last {window} actions. If this is intentional and making progress, carry on. If not, try a different approach."
        ));
    }

    let stagnant_pages = consecutive_stagnant_pages(history, state);
    if stagnant_pages >= 5 {
        messages.push(format!(
            "The page content has not changed across {stagnant_pages} consecutive observations. Your actions might not be having the intended effect."
        ));
    }

    if messages.is_empty() {
        None
    } else {
        Some(messages.join("\n\n"))
    }
}

fn repeated_action_nudge(history: &AgentHistory, window: usize) -> Option<(usize, usize)> {
    if window == 0 {
        return None;
    }

    let signatures = history
        .items
        .iter()
        .rev()
        .flat_map(|item| item.model_output.as_ref())
        .flat_map(|output| output.action.iter())
        .filter(|action| !matches!(action.name(), "wait" | "done" | "go_back"))
        .take(window)
        .filter_map(action_similarity_signature)
        .collect::<Vec<_>>();

    if signatures.len() < 5 {
        return None;
    }

    let mut counts = std::collections::BTreeMap::<String, usize>::new();
    for signature in &signatures {
        *counts.entry(signature.clone()).or_default() += 1;
    }
    let max_count = counts.values().copied().max().unwrap_or_default();
    (max_count >= 5).then_some((max_count, signatures.len()))
}

fn action_similarity_signature(action: &BrowserAction) -> Option<String> {
    match action {
        BrowserAction::Click(params) => params.index.map(|index| format!("click|{index}")),
        BrowserAction::Input(params) => Some(format!(
            "input|{}|{}",
            params.index,
            params.text.trim().to_ascii_lowercase()
        )),
        BrowserAction::Navigate(params) => Some(format!("navigate|{}", params.url)),
        BrowserAction::Search(params) => Some(format!(
            "search|{:?}|{}",
            params.engine,
            normalized_search_query(&params.query)
        )),
        BrowserAction::Scroll(params) => Some(format!("scroll|{}|{:?}", params.down, params.index)),
        other => serde_json::to_string(other).ok(),
    }
}

fn normalized_search_query(query: &str) -> String {
    let mut tokens = query
        .split(|character: char| !character.is_alphanumeric())
        .filter(|token| !token.is_empty())
        .map(str::to_ascii_lowercase)
        .collect::<Vec<_>>();
    tokens.sort();
    tokens.dedup();
    tokens.join("|")
}

fn consecutive_stagnant_pages(history: &AgentHistory, state: &BrowserStateSummary) -> usize {
    let mut count = 0;
    for item in history.items.iter().rev() {
        if item.state.url == state.url && item.state.dom_state.text == state.dom_state.text {
            count += 1;
        } else {
            break;
        }
    }
    count
}

fn truncate_prompt_content(content: String) -> String {
    if content.chars().count() <= MAX_PROMPT_CONTENT_CHARS {
        return content;
    }

    let truncated = content
        .chars()
        .take(MAX_PROMPT_CONTENT_CHARS)
        .collect::<String>();
    format!("{truncated}\n... [Content truncated at 60k characters]")
}

fn truncate_error_for_prompt(error: &str) -> String {
    if error.chars().count() <= MAX_PROMPT_ERROR_CHARS {
        return error.to_owned();
    }

    let prefix = error
        .chars()
        .take(PROMPT_ERROR_EDGE_CHARS)
        .collect::<String>();
    let suffix = error
        .chars()
        .rev()
        .take(PROMPT_ERROR_EDGE_CHARS)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<String>();
    format!("{prefix}......{suffix}")
}

pub(crate) fn repeated_action_loop(history: &AgentHistory, window: usize) -> bool {
    if window < 2 || history.items.len() < window {
        return false;
    }

    let signatures: Option<Vec<String>> = history
        .items
        .iter()
        .rev()
        .take(window)
        .map(|item| {
            item.model_output
                .as_ref()
                .and_then(|output| action_sequence_similarity_signature(&output.action))
        })
        .collect();

    let Some(signatures) = signatures else {
        return false;
    };
    let Some(first) = signatures.first() else {
        return false;
    };

    signatures.iter().all(|signature| signature == first)
}

fn action_sequence_similarity_signature(actions: &[BrowserAction]) -> Option<String> {
    let signatures = actions
        .iter()
        .filter(|action| !matches!(action.name(), "wait" | "done" | "go_back"))
        .filter_map(action_similarity_signature)
        .collect::<Vec<_>>();
    if signatures.is_empty() {
        None
    } else {
        Some(signatures.join("||"))
    }
}
