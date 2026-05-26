use std::time::{SystemTime, UNIX_EPOCH};

use base64::Engine;
use browser_use_cdp::BrowserError;
use browser_use_dom::BrowserStateSummary;
use browser_use_llm::{ChatRequest, ContentPart, ImageDetailLevel, MessageRole};
use encoding_rs::Encoding;
use serde_json::Value;

use crate::{
    ActionResult, AgentCurrentState, AgentHistory, AgentOutput, AgentSettings, BrowserAction,
    GenerateGif, LlmScreenshotSize, ManagedFileSystem, extract_start_url_from_task, search_url,
};

use super::AgentRunError;

pub(crate) fn encode_conversation_snapshot(
    snapshot: &str,
    encoding: Option<&str>,
) -> Result<Vec<u8>, AgentRunError> {
    let encoding = encoding.unwrap_or("utf-8");
    let Some(encoding_impl) = Encoding::for_label(encoding.as_bytes()) else {
        return Err(AgentRunError::ConversationEncoding {
            encoding: encoding.to_owned(),
        });
    };
    let (bytes, _, had_errors) = encoding_impl.encode(snapshot);
    if had_errors {
        return Err(AgentRunError::ConversationEncodingLossy {
            encoding: encoding.to_owned(),
        });
    }
    Ok(bytes.into_owned())
}

pub(crate) fn generate_gif_output_path(generate_gif: &GenerateGif) -> Option<std::path::PathBuf> {
    match generate_gif {
        GenerateGif::Disabled => None,
        GenerateGif::Enabled => Some(std::path::PathBuf::from("agent_history.gif")),
        GenerateGif::Path(path) => Some(expand_user_path(path)),
    }
}

pub(super) fn write_history_gif(
    history: &AgentHistory,
    path: &std::path::Path,
) -> Result<(), String> {
    let mut frames = Vec::new();
    for screenshot in history.screenshots(None, false).into_iter().flatten() {
        if let Some(frame) = decode_gif_frame(screenshot)? {
            frames.push(frame);
        }
    }
    if frames.is_empty() {
        return Ok(());
    }

    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    let file = std::fs::File::create(path).map_err(|error| error.to_string())?;
    let mut encoder = image::codecs::gif::GifEncoder::new(file);
    encoder
        .set_repeat(image::codecs::gif::Repeat::Infinite)
        .map_err(|error| error.to_string())?;
    let (target_width, target_height) = frames[0].dimensions();
    for frame in frames {
        if frame.width() != target_width || frame.height() != target_height {
            continue;
        }
        let delay = image::Delay::from_numer_denom_ms(3000, 1);
        encoder
            .encode_frame(image::Frame::from_parts(frame, 0, 0, delay))
            .map_err(|error| error.to_string())?;
    }
    Ok(())
}

fn decode_gif_frame(screenshot: &str) -> Result<Option<image::RgbaImage>, String> {
    let screenshot = screenshot.trim();
    let screenshot = screenshot
        .strip_prefix("data:image/png;base64,")
        .unwrap_or(screenshot);
    let bytes = match base64::engine::general_purpose::STANDARD.decode(screenshot) {
        Ok(bytes) => bytes,
        Err(error) => return Err(error.to_string()),
    };
    let image = match image::load_from_memory(&bytes) {
        Ok(image) => image.to_rgba8(),
        Err(error) => return Err(error.to_string()),
    };
    if image.width() <= 4 && image.height() <= 4 {
        return Ok(None);
    }
    Ok(Some(image))
}

pub(super) fn result_requests_screenshot(result: &ActionResult) -> bool {
    result
        .metadata
        .as_ref()
        .and_then(|metadata| metadata.get("include_screenshot"))
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

pub(super) fn format_conversation_snapshot(
    request: &ChatRequest,
    model_output: &AgentOutput,
) -> String {
    let mut lines = Vec::new();
    for message in &request.messages {
        lines.push(format!(" {} ", message_role_name(&message.role)));
        lines.push(render_message_content(&message.content));
        lines.push(String::new());
    }
    lines.push(serde_json::to_string_pretty(model_output).unwrap_or_else(|_| "{}".to_owned()));
    lines.join("\n")
}

fn message_role_name(role: &MessageRole) -> &'static str {
    match role {
        MessageRole::System => "system",
        MessageRole::User => "user",
        MessageRole::Assistant => "assistant",
        MessageRole::Tool => "tool",
    }
}

fn render_message_content(content: &[ContentPart]) -> String {
    content
        .iter()
        .map(|part| match part {
            ContentPart::Text { text } => text.clone(),
            ContentPart::ImageUrl { image_url, detail } => {
                let detail = detail.map(ImageDetailLevel::as_str).unwrap_or("auto");
                format!("<image_url detail=\"{detail}\">\n{image_url}\n</image_url>")
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

pub(crate) fn expand_user_path(path: &str) -> std::path::PathBuf {
    if path == "~" {
        return std::env::var_os("HOME")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| std::path::PathBuf::from(path));
    }
    if let Some(rest) = path.strip_prefix("~/")
        && let Some(home) = std::env::var_os("HOME")
    {
        return std::path::PathBuf::from(home).join(rest);
    }
    std::path::PathBuf::from(path)
}

pub(crate) fn now_seconds() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs_f64())
        .unwrap_or(0.0)
}

pub(super) fn is_single_done_output(output: &AgentOutput) -> bool {
    matches!(output.action.as_slice(), [BrowserAction::Done(_)])
}

pub(super) fn settings_with_direct_start_url(
    task: &str,
    mut settings: AgentSettings,
) -> AgentSettings {
    if settings.directly_open_url
        && settings.initial_actions.is_empty()
        && let Some(url) = extract_start_url_from_task(task)
    {
        settings.initial_actions =
            vec![BrowserAction::Navigate(browser_use_tools::NavigateAction {
                url,
                new_tab: false,
            })];
    }
    settings
}

pub(super) fn settings_with_llm_screenshot_default(
    mut settings: AgentSettings,
    model_name: &str,
) -> AgentSettings {
    if settings.llm_screenshot_size.is_none() && model_name.starts_with("claude-sonnet") {
        settings.llm_screenshot_size =
            Some(LlmScreenshotSize::new(1400, 850).expect("valid Claude Sonnet screenshot size"));
    }
    settings
}

pub(super) fn managed_file_system_for_settings(
    settings: &AgentSettings,
) -> Result<ManagedFileSystem, BrowserError> {
    match settings
        .file_system_path
        .as_deref()
        .filter(|path| !path.is_empty())
    {
        Some(path) => ManagedFileSystem::new(path),
        None => ManagedFileSystem::new_in_temp(),
    }
}

pub(super) fn initial_actions_model_output(
    actions: Vec<BrowserAction>,
    flash_mode: bool,
) -> AgentOutput {
    if flash_mode {
        return AgentOutput {
            current_state: AgentCurrentState::default(),
            thinking: None,
            evaluation_previous_goal: None,
            memory: Some("Initial navigation".to_owned()),
            next_goal: None,
            current_plan_item: None,
            plan_update: None,
            action: actions,
        };
    }

    AgentOutput {
        current_state: AgentCurrentState::default(),
        thinking: None,
        evaluation_previous_goal: Some("Start".to_owned()),
        memory: None,
        next_goal: Some("Initial navigation".to_owned()),
        current_plan_item: None,
        plan_update: None,
        action: actions,
    }
}

pub(super) fn initial_actions_state_history(actions: &[BrowserAction]) -> BrowserStateSummary {
    BrowserStateSummary {
        dom_state: Default::default(),
        url: initial_actions_url(actions).unwrap_or_default(),
        title: "Initial Actions".to_owned(),
        tabs: Vec::new(),
        screenshot: None,
        page_info: None,
        pixels_above: 0,
        pixels_below: 0,
        browser_errors: Vec::new(),
        is_pdf_viewer: false,
        recent_events: None,
        pending_network_requests: Vec::new(),
        pagination_buttons: Vec::new(),
        closed_popup_messages: Vec::new(),
    }
}

fn initial_actions_url(actions: &[BrowserAction]) -> Option<String> {
    actions.iter().find_map(|action| match action {
        BrowserAction::Navigate(params) => Some(params.url.clone()),
        BrowserAction::Search(params) => Some(search_url(&params.engine, &params.query)),
        _ => None,
    })
}
