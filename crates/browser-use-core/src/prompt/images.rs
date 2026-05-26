use crate::{AgentHistory, AgentSettings, BrowserStateSummary, LlmScreenshotSize};
use base64::Engine;
use browser_use_llm::{ContentPart, ImageDetailLevel};
use serde_json::Value;

use super::is_new_tab_page;

const PLACEHOLDER_4PX_SCREENSHOT: &str = "iVBORw0KGgoAAAANSUhEUgAAAAQAAAAECAIAAAAmkwkpAAAAFElEQVR4nGP8//8/AwwwMSAB3BwAlm4DBfIlvvkAAAAASUVORK5CYII=";

pub(super) fn prompt_screenshot_data_url(
    screenshot: &str,
    size: Option<LlmScreenshotSize>,
) -> String {
    size.and_then(|size| resize_screenshot_for_prompt(screenshot, size))
        .unwrap_or_else(|| screenshot_data_url(screenshot))
}

pub(super) fn prompt_visible_screenshot<'a>(
    state: &'a BrowserStateSummary,
    settings: &AgentSettings,
) -> Option<&'a str> {
    if !settings.use_vision.accepts_prompt_image() || is_new_tab_page(&state.url) {
        return None;
    }
    state
        .screenshot
        .as_deref()
        .filter(|screenshot| !screenshot.trim().is_empty())
        .filter(|screenshot| !is_placeholder_4px_screenshot(screenshot))
}

pub(super) fn append_latest_action_result_images(
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

fn screenshot_data_url(screenshot: &str) -> String {
    if screenshot.starts_with("data:image/") {
        screenshot.to_owned()
    } else {
        format!("data:image/png;base64,{screenshot}")
    }
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

fn is_placeholder_4px_screenshot(screenshot: &str) -> bool {
    screenshot_base64_payload(screenshot).trim() == PLACEHOLDER_4PX_SCREENSHOT
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
