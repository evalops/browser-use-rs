//! Core agent contracts for browser-use-rs.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io::Write;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use thiserror::Error;
use tokio::time::{sleep, timeout};
use uuid::Uuid;

use async_trait::async_trait;
use base64::Engine;
use browser_use_cdp::{BrowserError, BrowserSession, FoundElement};
use url::form_urlencoded;

pub use browser_use_dom::BrowserStateSummary;
pub use browser_use_llm::{
    AnthropicChatModel, ChatCompletion, ChatMessage, ChatModel, ChatRequest, ContentPart,
    GeminiChatModel, LlmError, MessageRole, OllamaChatModel, OpenAiCompatibleChatModel,
};
pub use browser_use_tools::{BrowserAction, SearchEngine};

/// Version of the upstream browser-use source that this crate initially targets.
pub const INITIAL_UPSTREAM_COMMIT: &str = "933e28c599ddd74c15a48568f159da95547e40dd";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct AgentSettings {
    #[serde(default = "default_use_vision")]
    pub use_vision: bool,
    #[serde(default = "default_max_failures")]
    pub max_failures: u32,
    #[serde(default = "default_max_actions_per_step")]
    pub max_actions_per_step: usize,
    #[serde(default = "default_llm_timeout_seconds")]
    pub llm_timeout_seconds: u64,
    #[serde(default = "default_step_timeout_seconds")]
    pub step_timeout_seconds: u64,
    #[serde(default = "default_loop_detection_window")]
    pub loop_detection_window: usize,
    #[serde(default = "default_loop_detection_enabled")]
    pub loop_detection_enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_history_items: Option<usize>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub include_attributes: Vec<String>,
}

impl Default for AgentSettings {
    fn default() -> Self {
        Self {
            use_vision: default_use_vision(),
            max_failures: default_max_failures(),
            max_actions_per_step: default_max_actions_per_step(),
            llm_timeout_seconds: default_llm_timeout_seconds(),
            step_timeout_seconds: default_step_timeout_seconds(),
            loop_detection_window: default_loop_detection_window(),
            loop_detection_enabled: default_loop_detection_enabled(),
            max_history_items: None,
            include_attributes: Vec::new(),
        }
    }
}

fn default_use_vision() -> bool {
    true
}

fn default_max_failures() -> u32 {
    5
}

fn default_max_actions_per_step() -> usize {
    5
}

fn default_llm_timeout_seconds() -> u64 {
    60
}

fn default_step_timeout_seconds() -> u64 {
    180
}

fn default_loop_detection_window() -> usize {
    20
}

fn default_loop_detection_enabled() -> bool {
    true
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct AgentTask {
    pub id: Uuid,
    pub task: String,
    #[serde(default)]
    pub settings: AgentSettings,
}

impl AgentTask {
    #[must_use]
    pub fn new(task: impl Into<String>) -> Self {
        Self {
            id: Uuid::now_v7(),
            task: task.into(),
            settings: AgentSettings::default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct AgentOutput {
    pub current_state: AgentCurrentState,
    pub action: Vec<BrowserAction>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct AgentCurrentState {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thinking: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evaluation_previous_goal: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memory: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_goal: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ActionResult {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extracted_content: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub long_term_memory: Option<String>,
    #[serde(default)]
    pub include_extracted_content_only_once: bool,
    #[serde(default)]
    pub include_in_memory: bool,
    #[serde(default)]
    pub is_done: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub success: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<Value>,
}

impl ActionResult {
    #[must_use]
    pub fn extracted(text: impl Into<String>) -> Self {
        let text = text.into();
        Self {
            extracted_content: Some(text.clone()),
            error: None,
            long_term_memory: Some(text),
            include_extracted_content_only_once: false,
            include_in_memory: false,
            is_done: false,
            success: None,
            metadata: None,
        }
    }

    #[must_use]
    pub fn error(error: impl Into<String>) -> Self {
        Self {
            extracted_content: None,
            error: Some(error.into()),
            long_term_memory: None,
            include_extracted_content_only_once: false,
            include_in_memory: true,
            is_done: false,
            success: None,
            metadata: None,
        }
    }

    #[must_use]
    pub fn done(text: impl Into<String>, success: bool) -> Self {
        Self {
            extracted_content: Some(text.into()),
            error: None,
            long_term_memory: None,
            include_extracted_content_only_once: false,
            include_in_memory: true,
            is_done: true,
            success: Some(success),
            metadata: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct AgentHistoryItem {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_output: Option<AgentOutput>,
    pub result: Vec<ActionResult>,
    pub state: BrowserStateSummary,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<StepMetadata>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct StepMetadata {
    pub step_start_time: f64,
    pub step_end_time: f64,
    pub step_number: usize,
    #[serde(default)]
    pub step_interval: Option<f64>,
}

impl StepMetadata {
    #[must_use]
    pub fn duration_seconds(&self) -> f64 {
        self.step_end_time - self.step_start_time
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct AgentHistory {
    #[serde(default)]
    pub items: Vec<AgentHistoryItem>,
}

impl AgentHistory {
    #[must_use]
    pub fn final_result(&self) -> Option<&str> {
        self.final_done_result()
            .and_then(|result| result.extracted_content.as_deref())
    }

    #[must_use]
    pub fn is_done(&self) -> bool {
        self.final_done_result().is_some()
    }

    #[must_use]
    pub fn is_successful(&self) -> Option<bool> {
        self.final_done_result().and_then(|result| result.success)
    }

    #[must_use]
    pub fn errors(&self) -> Vec<&str> {
        self.items
            .iter()
            .flat_map(|item| item.result.iter())
            .filter_map(|result| result.error.as_deref())
            .collect()
    }

    #[must_use]
    pub fn total_duration_seconds(&self) -> f64 {
        self.items
            .iter()
            .filter_map(|item| item.metadata.as_ref())
            .map(StepMetadata::duration_seconds)
            .sum()
    }

    #[must_use]
    pub fn number_of_steps(&self) -> usize {
        self.items.len()
    }

    #[must_use]
    pub fn urls(&self) -> Vec<&str> {
        self.items
            .iter()
            .map(|item| item.state.url.as_str())
            .collect()
    }

    #[must_use]
    pub fn action_results(&self) -> Vec<&ActionResult> {
        self.items
            .iter()
            .flat_map(|item| item.result.iter())
            .collect()
    }

    #[must_use]
    pub fn extracted_content(&self) -> Vec<&str> {
        self.action_results()
            .into_iter()
            .filter_map(|result| result.extracted_content.as_deref())
            .collect()
    }

    #[must_use]
    pub fn action_names(&self) -> Vec<&'static str> {
        self.items
            .iter()
            .filter_map(|item| item.model_output.as_ref())
            .flat_map(|output| output.action.iter())
            .map(BrowserAction::name)
            .collect()
    }

    fn final_done_result(&self) -> Option<&ActionResult> {
        self.items
            .iter()
            .rev()
            .flat_map(|item| item.result.iter().rev())
            .find(|result| result.is_done)
    }
}

#[async_trait]
pub trait ActionExecutor {
    async fn execute(&mut self, action: &BrowserAction) -> ActionResult;
}

pub struct BrowserActionExecutor<S> {
    session: S,
}

impl<S> BrowserActionExecutor<S> {
    #[must_use]
    pub fn new(session: S) -> Self {
        Self { session }
    }

    #[must_use]
    pub fn session(&self) -> &S {
        &self.session
    }
}

impl<S> BrowserActionExecutor<S>
where
    S: BrowserSession + Send + Sync,
{
    pub async fn execute_sequence(&mut self, actions: &[BrowserAction]) -> Vec<ActionResult> {
        let mut results = Vec::new();

        for (index, action) in actions.iter().enumerate() {
            if index > 0 && matches!(action, BrowserAction::Done(_)) {
                break;
            }

            let needs_page_change_guard = !action.terminates_sequence();
            let before = if needs_page_change_guard {
                match self.session.state(false).await {
                    Ok(state) => Some(state),
                    Err(error) => {
                        results.push(ActionResult::error(error.to_string()));
                        break;
                    }
                }
            } else {
                None
            };
            let result = self.execute(action).await;
            let should_stop =
                result.is_done || result.error.is_some() || action.terminates_sequence();
            let page_changed = if should_stop {
                false
            } else if let Some(before) = before {
                match self.session.state(false).await {
                    Ok(after) => after.url != before.url,
                    Err(error) => {
                        results.push(result);
                        results.push(ActionResult::error(error.to_string()));
                        break;
                    }
                }
            } else {
                false
            };

            results.push(result);

            if should_stop || page_changed {
                break;
            }
        }

        results
    }
}

#[async_trait]
impl<S> ActionExecutor for BrowserActionExecutor<S>
where
    S: BrowserSession + Send + Sync,
{
    async fn execute(&mut self, action: &BrowserAction) -> ActionResult {
        match execute_browser_action(&self.session, action).await {
            Ok(result) => result,
            Err(error) => ActionResult::error(error.to_string()),
        }
    }
}

async fn execute_browser_action<S>(
    session: &S,
    action: &BrowserAction,
) -> Result<ActionResult, BrowserError>
where
    S: BrowserSession + Send + Sync,
{
    match action {
        BrowserAction::Search(params) => {
            let url = search_url(&params.engine, &params.query);
            session.navigate(&url, false).await?;
            Ok(ActionResult::extracted(format!(
                "Searched {:?} for '{}'",
                params.engine, params.query
            )))
        }
        BrowserAction::Navigate(params) => {
            session.navigate(&params.url, params.new_tab).await?;
            Ok(ActionResult::extracted(format!(
                "Navigated to {}",
                params.url
            )))
        }
        BrowserAction::GoBack(_) => {
            session.go_back().await?;
            Ok(ActionResult::extracted("Navigated back"))
        }
        BrowserAction::SwitchTab(params) => {
            session.switch_tab(&params.tab_id).await?;
            Ok(ActionResult::extracted(format!(
                "Switched to tab {}",
                params.tab_id
            )))
        }
        BrowserAction::CloseTab(params) => {
            session.close_tab(&params.tab_id).await?;
            Ok(ActionResult::extracted(format!(
                "Closed tab {}",
                params.tab_id
            )))
        }
        BrowserAction::Click(params) => {
            match (params.index, params.coordinate_x, params.coordinate_y) {
                (Some(0), _, _) => Ok(ActionResult::error(
                    "Cannot click on element with index 0. Use a positive browser_state index.",
                )),
                (Some(index), _, _) => match session.click(index).await {
                    Ok(()) => Ok(ActionResult::extracted(format!("Clicked element {index}"))),
                    Err(error) if is_select_click_validation_error(&error) => {
                        match session.dropdown_options(index).await {
                            Ok(options) => Ok(ActionResult::extracted(format!(
                                "Dropdown options: {}",
                                options.join(", ")
                            ))),
                            Err(_) => Ok(ActionResult::error(error.to_string())),
                        }
                    }
                    Err(error) if is_file_input_click_validation_error(&error) => {
                        Ok(ActionResult::error(
                            "Cannot click on file input elements. Use upload_file with the same index instead.",
                        ))
                    }
                    Err(error) => Err(error),
                },
                (None, Some(x), Some(y)) => {
                    session.click_coordinates(x, y).await?;
                    Ok(ActionResult::extracted(format!(
                        "Clicked coordinates ({x}, {y})"
                    )))
                }
                _ => Ok(ActionResult::error(
                    "click requires either an element index or both coordinate_x and coordinate_y",
                )),
            }
        }
        BrowserAction::Input(params) => {
            session
                .input_text(params.index, &params.text, params.clear)
                .await?;
            Ok(ActionResult::extracted(format!(
                "Typed text into element {}",
                params.index
            )))
        }
        BrowserAction::Scroll(params) => {
            let index = params.index.filter(|index| *index != 0);
            session.scroll(index, params.down, params.pages).await?;
            Ok(ActionResult::extracted("Scrolled page"))
        }
        BrowserAction::FindText(params) => {
            if session.find_text(&params.text).await? {
                Ok(ActionResult::extracted(format!(
                    "Scrolled to text: {}",
                    params.text
                )))
            } else {
                Ok(ActionResult {
                    extracted_content: Some(format!(
                        "Text '{}' not found or not visible on page",
                        params.text
                    )),
                    error: None,
                    long_term_memory: Some(format!(
                        "Tried scrolling to text '{}' but it was not found",
                        params.text
                    )),
                    include_extracted_content_only_once: false,
                    include_in_memory: false,
                    is_done: false,
                    success: None,
                    metadata: None,
                })
            }
        }
        BrowserAction::Evaluate(params) => match session.evaluate(&params.code).await {
            Ok(result_text) => {
                let result_text = truncate_evaluate_result(result_text);
                let long_term_memory = if result_text.chars().count() < 10_000 {
                    result_text.clone()
                } else {
                    format!(
                        "JavaScript executed successfully, result length: {} characters.",
                        result_text.chars().count()
                    )
                };
                Ok(ActionResult {
                    include_extracted_content_only_once: long_term_memory != result_text,
                    extracted_content: Some(result_text),
                    error: None,
                    long_term_memory: Some(long_term_memory),
                    include_in_memory: true,
                    is_done: false,
                    success: None,
                    metadata: None,
                })
            }
            Err(error) => Ok(ActionResult::error(format!(
                "Failed to execute JavaScript: {error}"
            ))),
        },
        BrowserAction::Wait(params) => {
            let actual_seconds = params.seconds.saturating_sub(1).clamp(0, 30) as u64;
            sleep(Duration::from_secs(actual_seconds)).await;
            Ok(ActionResult::extracted(format!(
                "Waited for {} seconds",
                params.seconds
            )))
        }
        BrowserAction::Screenshot(params) => {
            if let Some(file_name) = params.file_name.as_deref() {
                let screenshot = session.screenshot().await?;
                let output_path = screenshot_output_path(file_name);
                if let Some(parent) = output_path.parent()
                    && !parent.as_os_str().is_empty()
                {
                    std::fs::create_dir_all(parent)
                        .map_err(|error| BrowserError::ActionFailed(error.to_string()))?;
                }
                let bytes = base64::engine::general_purpose::STANDARD
                    .decode(&screenshot.base64_png)
                    .map_err(|error| BrowserError::ActionFailed(error.to_string()))?;
                std::fs::write(&output_path, bytes)
                    .map_err(|error| BrowserError::ActionFailed(error.to_string()))?;
                let file_name = output_path.display().to_string();
                Ok(ActionResult {
                    extracted_content: Some(format!("Screenshot saved to {file_name}")),
                    error: None,
                    long_term_memory: Some(format!("Screenshot saved to {file_name}")),
                    include_extracted_content_only_once: true,
                    include_in_memory: true,
                    is_done: false,
                    success: None,
                    metadata: None,
                })
            } else {
                Ok(ActionResult {
                    extracted_content: Some("Requested screenshot for next observation".to_owned()),
                    error: None,
                    long_term_memory: None,
                    include_extracted_content_only_once: false,
                    include_in_memory: false,
                    is_done: false,
                    success: None,
                    metadata: None,
                })
            }
        }
        BrowserAction::Done(params) => Ok(ActionResult::done(params.text.clone(), params.success)),
        BrowserAction::Extract(params) => {
            let text = session.page_text().await?;
            let source_url = session.state(false).await.ok().map(|state| state.url);
            let extract_images = should_extract_images(&params.query, params.extract_images);
            let links = if params.extract_links {
                Some(
                    session
                        .find_elements(
                            "a[href]",
                            &extract_link_attributes(),
                            MAX_EXTRACT_RELATED_ELEMENTS,
                            true,
                        )
                        .await?,
                )
            } else {
                None
            };
            let images = if extract_images {
                Some(
                    session
                        .find_elements(
                            "img[src], img[data-src], picture source[srcset]",
                            &extract_image_attributes(),
                            MAX_EXTRACT_RELATED_ELEMENTS,
                            false,
                        )
                        .await?,
                )
            } else {
                None
            };
            Ok(extract_action_result(
                params,
                &text,
                source_url.as_deref(),
                extract_images,
                links.as_deref(),
                images.as_deref(),
            ))
        }
        BrowserAction::SearchPage(params) => {
            let text = if let Some(scope) = &params.css_scope {
                session
                    .find_elements(scope, &[], params.max_results.max(1), true)
                    .await?
                    .into_iter()
                    .filter_map(|element| element.text)
                    .collect::<Vec<_>>()
                    .join("\n")
            } else {
                session.page_text().await?
            };
            let matches = search_text_matches(
                &text,
                &params.pattern,
                params.regex,
                params.case_sensitive,
                params.context_chars,
                params.max_results,
            )
            .map_err(BrowserError::ActionFailed)?;

            Ok(ActionResult::extracted(format_search_page_results(
                &params.pattern,
                &matches,
            )))
        }
        BrowserAction::FindElements(params) => {
            let attributes = params
                .attributes
                .clone()
                .unwrap_or_else(default_find_element_attributes);
            let elements = session
                .find_elements(
                    &params.selector,
                    &attributes,
                    params.max_results,
                    params.include_text,
                )
                .await?;
            Ok(ActionResult::extracted(format_find_elements_results(
                &params.selector,
                &elements,
            )))
        }
        BrowserAction::GetDropdownOptions(params) => {
            let options = session.dropdown_options(params.index).await?;
            Ok(ActionResult::extracted(format!(
                "Dropdown options: {}",
                options.join(", ")
            )))
        }
        BrowserAction::SelectDropdownOption(params) => {
            session
                .select_dropdown_option(params.index, &params.text)
                .await?;
            Ok(ActionResult::extracted(format!(
                "Selected dropdown option '{}' on element {}",
                params.text, params.index
            )))
        }
        BrowserAction::SendKeys(params) => {
            session.send_keys(&params.keys).await?;
            Ok(ActionResult::extracted(format!(
                "Sent keys '{}'",
                params.keys
            )))
        }
        BrowserAction::SaveAsPdf(params) => {
            let pdf = session
                .save_pdf(
                    params.print_background,
                    params.landscape,
                    params.scale,
                    &params.paper_format,
                )
                .await?;

            let page_title = if params.file_name.is_none() {
                session.state(false).await.ok().map(|state| state.title)
            } else {
                None
            };
            let output_path = pdf_output_path(params.file_name.as_deref(), page_title.as_deref());
            if let Some(parent) = output_path.parent()
                && !parent.as_os_str().is_empty()
            {
                std::fs::create_dir_all(parent)
                    .map_err(|error| BrowserError::ActionFailed(error.to_string()))?;
            }
            let output_path = next_available_pdf_path(output_path);
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(&pdf.base64_pdf)
                .map_err(|error| BrowserError::ActionFailed(error.to_string()))?;
            std::fs::write(&output_path, bytes)
                .map_err(|error| BrowserError::ActionFailed(error.to_string()))?;
            let file_name = output_path.display().to_string();
            Ok(ActionResult {
                extracted_content: Some(format!("Saved PDF to {file_name}")),
                error: None,
                long_term_memory: Some(format!("Saved PDF to {file_name}")),
                include_extracted_content_only_once: true,
                include_in_memory: true,
                is_done: false,
                success: None,
                metadata: None,
            })
        }
        BrowserAction::UploadFile(params) => {
            session
                .upload_file(params.index, std::path::Path::new(&params.path))
                .await?;
            Ok(ActionResult::extracted(format!(
                "Uploaded {} to element {}",
                params.path, params.index
            )))
        }
        BrowserAction::WriteFile(params) => write_file_action(params),
        BrowserAction::ReadFile(params) => read_file_action(&params.file_name),
        BrowserAction::ReplaceFile(params) => {
            replace_file_action(&params.file_name, &params.old_str, &params.new_str)
        }
    }
}

fn is_select_click_validation_error(error: &BrowserError) -> bool {
    error
        .to_string()
        .contains("Cannot click on <select> elements.")
}

fn is_file_input_click_validation_error(error: &BrowserError) -> bool {
    error
        .to_string()
        .contains("Cannot click on file input elements.")
}

const MAX_EXTRACT_CHAR_LIMIT: usize = 100_000;
const MAX_EXTRACT_RELATED_ELEMENTS: usize = 200;
const IMAGE_QUERY_KEYWORDS: &[&str] = &[
    "image",
    "photo",
    "picture",
    "thumbnail",
    "img url",
    "image url",
    "photo url",
    "product image",
];

fn should_extract_images(query: &str, requested: bool) -> bool {
    let query = query.to_ascii_lowercase();
    requested
        || IMAGE_QUERY_KEYWORDS
            .iter()
            .any(|keyword| query.contains(keyword))
}

fn extract_action_result(
    params: &browser_use_tools::ExtractAction,
    page_text: &str,
    source_url: Option<&str>,
    extract_images: bool,
    links: Option<&[FoundElement]>,
    images: Option<&[FoundElement]>,
) -> ActionResult {
    let total_chars = page_text.chars().count();
    if params.start_from_char > total_chars {
        return ActionResult::error(format!(
            "start_from_char ({}) exceeds content length {total_chars} characters.",
            params.start_from_char
        ));
    }

    let available_chars = total_chars.saturating_sub(params.start_from_char);
    let truncated = available_chars > MAX_EXTRACT_CHAR_LIMIT;
    let content: String = page_text
        .chars()
        .skip(params.start_from_char)
        .take(MAX_EXTRACT_CHAR_LIMIT)
        .collect();
    let next_start_char = params.start_from_char + content.chars().count();
    let content_stats = extract_content_stats(
        total_chars,
        params.start_from_char,
        content.chars().count(),
        truncated,
        next_start_char,
        params.extract_links,
        extract_images,
    );
    let rendered =
        render_extract_envelope(params, source_url, &content, &content_stats, links, images);
    let memory = if rendered.chars().count() < 10_000 {
        rendered.clone()
    } else {
        format!(
            "Query: {}\nContent prepared for extraction, length: {} characters.",
            params.query,
            content.chars().count()
        )
    };

    ActionResult {
        extracted_content: Some(rendered),
        error: None,
        long_term_memory: Some(memory),
        include_extracted_content_only_once: true,
        include_in_memory: true,
        is_done: false,
        success: None,
        metadata: extract_metadata(
            params,
            source_url,
            total_chars,
            params.start_from_char,
            content.chars().count(),
            truncated,
            next_start_char,
            extract_images,
            links.map_or(0, <[FoundElement]>::len),
            images.map_or(0, <[FoundElement]>::len),
        ),
    }
}

fn extract_content_stats(
    total_chars: usize,
    start_from_char: usize,
    content_chars: usize,
    truncated: bool,
    next_start_char: usize,
    extract_links: bool,
    extract_images: bool,
) -> String {
    let mut stats =
        format!("Content processed: {total_chars} text chars -> {total_chars} filtered text chars");
    if start_from_char > 0 {
        stats.push_str(&format!(" (started from char {start_from_char})"));
    }
    if truncated {
        stats.push_str(&format!(
            " -> {content_chars} final chars (use start_from_char={next_start_char} to continue)"
        ));
    }
    if extract_links || extract_images {
        stats.push_str(&format!(
            "\nExtraction options: extract_links={extract_links}, extract_images={extract_images}"
        ));
    }
    stats
}

#[allow(clippy::too_many_arguments)]
fn extract_metadata(
    params: &browser_use_tools::ExtractAction,
    source_url: Option<&str>,
    original_chars: usize,
    start_from_char: usize,
    content_chars: usize,
    truncated: bool,
    next_start_char: usize,
    extract_images: bool,
    links_count: usize,
    images_count: usize,
) -> Option<Value> {
    let schema = params.output_schema.as_ref()?;
    Some(serde_json::json!({
        "structured_extraction": true,
        "schema_used": schema,
        "is_partial": truncated,
        "source_url": source_url,
        "content_stats": {
            "method": "page_text",
            "original_text_chars": original_chars,
            "final_filtered_chars": original_chars,
            "started_from_char": start_from_char,
            "returned_chars": content_chars,
            "next_start_char": if truncated { Some(next_start_char) } else { None::<usize> },
        },
        "options": {
            "extract_links": params.extract_links,
            "extract_images": extract_images,
            "links_count": links_count,
            "images_count": images_count,
            "already_collected_count": params.already_collected.len(),
        }
    }))
}

fn render_extract_envelope(
    params: &browser_use_tools::ExtractAction,
    source_url: Option<&str>,
    content: &str,
    content_stats: &str,
    links: Option<&[FoundElement]>,
    images: Option<&[FoundElement]>,
) -> String {
    let mut rendered = source_url
        .map(|url| format!("<url>\n{url}\n</url>\n"))
        .unwrap_or_default();
    rendered.push_str(&format!("<query>\n{}\n</query>\n\n", params.query));

    if let Some(schema) = &params.output_schema {
        let schema = serde_json::to_string_pretty(schema).unwrap_or_else(|_| schema.to_string());
        rendered.push_str(&format!("<output_schema>\n{schema}\n</output_schema>\n\n"));
    }

    rendered.push_str(&format!(
        "<content_stats>\n{content_stats}\n</content_stats>\n\n<webpage_content>\n{content}\n</webpage_content>"
    ));

    if let Some(links) = links.and_then(render_link_appendix) {
        rendered.push_str(&format!("\n\n<links>\n{links}\n</links>"));
    }

    if let Some(images) = images.and_then(render_image_appendix) {
        rendered.push_str(&format!("\n\n<images>\n{images}\n</images>"));
    }

    if !params.already_collected.is_empty() {
        let items = params
            .already_collected
            .iter()
            .take(100)
            .map(|item| format!("- {item}"))
            .collect::<Vec<_>>()
            .join("\n");
        rendered.push_str(&format!(
            "\n\n<already_collected>\nSkip items whose name/title/URL matches any of these already-collected identifiers:\n{items}\n</already_collected>"
        ));
    }

    rendered
}

fn render_link_appendix(elements: &[FoundElement]) -> Option<String> {
    let lines = elements
        .iter()
        .filter_map(|element| {
            let href = element.attributes.get("href")?.trim();
            if href.is_empty() {
                return None;
            }
            let label = element_label(element)
                .filter(|label| !label.is_empty())
                .unwrap_or(href);
            Some(format!("- {label}: {href}"))
        })
        .collect::<Vec<_>>();

    (!lines.is_empty()).then(|| lines.join("\n"))
}

fn render_image_appendix(elements: &[FoundElement]) -> Option<String> {
    let lines = elements
        .iter()
        .filter_map(|element| {
            let src = element
                .attributes
                .get("src")
                .or_else(|| element.attributes.get("data-src"))
                .or_else(|| element.attributes.get("srcset"))?
                .trim();
            if src.is_empty() {
                return None;
            }
            let label = element_label(element)
                .filter(|label| !label.is_empty())
                .unwrap_or("image");
            Some(format!("- {label}: {src}"))
        })
        .collect::<Vec<_>>();

    (!lines.is_empty()).then(|| lines.join("\n"))
}

fn element_label(element: &FoundElement) -> Option<&str> {
    element
        .text
        .as_deref()
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .or_else(|| attr_label(element, "alt"))
        .or_else(|| attr_label(element, "title"))
        .or_else(|| attr_label(element, "aria-label"))
}

fn attr_label<'a>(element: &'a FoundElement, name: &str) -> Option<&'a str> {
    element
        .attributes
        .get(name)
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
}

fn extract_link_attributes() -> Vec<String> {
    ["href", "title", "aria-label", "rel"]
        .into_iter()
        .map(str::to_owned)
        .collect()
}

fn extract_image_attributes() -> Vec<String> {
    ["src", "data-src", "srcset", "alt", "title", "aria-label"]
        .into_iter()
        .map(str::to_owned)
        .collect()
}

fn pdf_output_path(file_name: Option<&str>, page_title: Option<&str>) -> std::path::PathBuf {
    let raw_name = file_name
        .filter(|name| !name.trim().is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| {
            page_title
                .map(sanitize_pdf_title)
                .filter(|title| !title.is_empty())
                .unwrap_or_else(|| "page".to_owned())
        });
    let path = std::path::PathBuf::from(raw_name);
    ensure_pdf_extension(path)
}

fn sanitize_pdf_title(title: &str) -> String {
    title
        .chars()
        .filter(|character| {
            character.is_alphanumeric()
                || *character == '_'
                || *character == ' '
                || *character == '-'
        })
        .collect::<String>()
        .trim()
        .chars()
        .take(50)
        .collect()
}

fn ensure_pdf_extension(mut path: std::path::PathBuf) -> std::path::PathBuf {
    let has_pdf_extension = path
        .file_name()
        .and_then(std::ffi::OsStr::to_str)
        .is_some_and(|file_name| file_name.to_ascii_lowercase().ends_with(".pdf"));
    if has_pdf_extension {
        return path;
    }

    let Some(file_name) = path
        .file_name()
        .and_then(std::ffi::OsStr::to_str)
        .map(str::to_owned)
    else {
        return std::path::PathBuf::from("page.pdf");
    };
    path.set_file_name(format!("{file_name}.pdf"));
    path
}

fn next_available_pdf_path(path: std::path::PathBuf) -> std::path::PathBuf {
    if !path.exists() {
        return path;
    }

    let parent = path.parent().map(std::path::Path::to_path_buf);
    let stem = path
        .file_stem()
        .and_then(std::ffi::OsStr::to_str)
        .unwrap_or("page");
    let extension = path
        .extension()
        .and_then(std::ffi::OsStr::to_str)
        .unwrap_or("pdf");

    for counter in 1.. {
        let candidate_name = format!("{stem} ({counter}).{extension}");
        let candidate = parent.as_ref().map_or_else(
            || std::path::PathBuf::from(&candidate_name),
            |parent| parent.join(&candidate_name),
        );
        if !candidate.exists() {
            return candidate;
        }
    }
    unreachable!("unbounded PDF filename counter should always return")
}

fn screenshot_output_path(file_name: &str) -> std::path::PathBuf {
    let path = std::path::PathBuf::from(if file_name.trim().is_empty() {
        "screenshot".to_owned()
    } else {
        file_name.to_owned()
    });
    ensure_png_extension(path)
}

fn ensure_png_extension(mut path: std::path::PathBuf) -> std::path::PathBuf {
    let has_png_extension = path
        .file_name()
        .and_then(std::ffi::OsStr::to_str)
        .is_some_and(|file_name| file_name.to_ascii_lowercase().ends_with(".png"));
    if has_png_extension {
        return path;
    }

    let Some(file_name) = path
        .file_name()
        .and_then(std::ffi::OsStr::to_str)
        .map(str::to_owned)
    else {
        return std::path::PathBuf::from("screenshot.png");
    };
    path.set_file_name(format!("{file_name}.png"));
    path
}

fn write_file_action(
    params: &browser_use_tools::WriteFileAction,
) -> Result<ActionResult, BrowserError> {
    if let Err(result) = validate_text_file_name(&params.file_name) {
        return Ok(result);
    }
    let path = std::path::Path::new(&params.file_name);
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent)
            .map_err(|error| BrowserError::ActionFailed(error.to_string()))?;
    }

    let mut content = params.content.clone();
    if params.trailing_newline {
        content.push('\n');
    }
    if params.leading_newline {
        content.insert(0, '\n');
    }

    if params.append {
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .map_err(|error| BrowserError::ActionFailed(error.to_string()))?;
        file.write_all(content.as_bytes())
            .map_err(|error| BrowserError::ActionFailed(error.to_string()))?;
        Ok(ActionResult::extracted(format!(
            "Appended to file {}",
            params.file_name
        )))
    } else {
        std::fs::write(path, content)
            .map_err(|error| BrowserError::ActionFailed(error.to_string()))?;
        Ok(ActionResult::extracted(format!(
            "Wrote file {}",
            params.file_name
        )))
    }
}

fn read_file_action(file_name: &str) -> Result<ActionResult, BrowserError> {
    if let Err(result) = validate_text_file_name(file_name) {
        return Ok(result);
    }
    let content = std::fs::read_to_string(file_name)
        .map_err(|error| BrowserError::ActionFailed(error.to_string()))?;
    let memory = read_file_memory(&content);
    Ok(ActionResult {
        extracted_content: Some(format!("Read file {file_name}:\n{content}")),
        error: None,
        long_term_memory: Some(memory),
        include_extracted_content_only_once: true,
        include_in_memory: true,
        is_done: false,
        success: None,
        metadata: None,
    })
}

fn replace_file_action(
    file_name: &str,
    old_str: &str,
    new_str: &str,
) -> Result<ActionResult, BrowserError> {
    if let Err(result) = validate_text_file_name(file_name) {
        return Ok(result);
    }
    if old_str.is_empty() {
        return Ok(ActionResult::error(
            "Cannot replace empty string. Please provide a non-empty string to replace.",
        ));
    }
    let content = std::fs::read_to_string(file_name)
        .map_err(|error| BrowserError::ActionFailed(error.to_string()))?;
    if !content.contains(old_str) {
        return Ok(ActionResult::error(format!(
            "Could not find text to replace in {file_name}"
        )));
    }
    let updated = content.replace(old_str, new_str);
    std::fs::write(file_name, updated)
        .map_err(|error| BrowserError::ActionFailed(error.to_string()))?;
    Ok(ActionResult::extracted(format!(
        "Replaced text in file {file_name}"
    )))
}

fn validate_text_file_name(file_name: &str) -> Result<(), ActionResult> {
    let path = std::path::Path::new(file_name);
    let base_name = path
        .file_name()
        .and_then(std::ffi::OsStr::to_str)
        .unwrap_or(file_name);
    let Some(extension) = path.extension().and_then(std::ffi::OsStr::to_str) else {
        return Err(ActionResult::error(format!(
            "Filename '{base_name}' has no extension. Supported extensions: {}.",
            supported_text_extensions_message()
        )));
    };
    let extension = extension.to_ascii_lowercase();

    if unsupported_binary_extensions().contains(&extension.as_str()) {
        return Err(ActionResult::error(format!(
            "Cannot write binary/image file '{base_name}'. The file actions only support text-based files. Supported extensions: {}.",
            supported_text_extensions_message()
        )));
    }

    if !supported_text_extensions().contains(&extension.as_str()) {
        return Err(ActionResult::error(format!(
            "Unsupported file extension '.{extension}' in '{base_name}'. Supported extensions: {}.",
            supported_text_extensions_message()
        )));
    }

    Ok(())
}

fn supported_text_extensions() -> &'static [&'static str] {
    &["txt", "md", "json", "jsonl", "csv", "html", "xml"]
}

fn unsupported_binary_extensions() -> &'static [&'static str] {
    &[
        "png", "jpg", "jpeg", "gif", "webp", "bmp", "ico", "mp4", "mov", "avi", "zip", "gz", "tar",
        "exe", "bin",
    ]
}

fn supported_text_extensions_message() -> String {
    supported_text_extensions()
        .iter()
        .map(|extension| format!(".{extension}"))
        .collect::<Vec<_>>()
        .join(", ")
}

fn read_file_memory(content: &str) -> String {
    const MAX_MEMORY_SIZE: usize = 1_000;
    if content.len() <= MAX_MEMORY_SIZE {
        return content.to_owned();
    }

    let mut display = String::new();
    let mut lines_count = 0;
    let lines: Vec<&str> = content.lines().collect();
    for line in &lines {
        if display.len() + line.len() + 1 < MAX_MEMORY_SIZE {
            display.push_str(line);
            display.push('\n');
            lines_count += 1;
        } else {
            break;
        }
    }
    let remaining_lines = lines.len().saturating_sub(lines_count);
    if remaining_lines > 0 {
        format!("{display}{remaining_lines} more lines...")
    } else {
        display
    }
}

fn default_find_element_attributes() -> Vec<String> {
    [
        "id",
        "class",
        "name",
        "type",
        "placeholder",
        "href",
        "aria-label",
        "role",
        "title",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect()
}

fn truncate_evaluate_result(result: String) -> String {
    const MAX_CHARS: usize = 20_000;
    const PREFIX_CHARS: usize = 19_950;

    if result.chars().count() <= MAX_CHARS {
        return result;
    }

    let mut truncated: String = result.chars().take(PREFIX_CHARS).collect();
    truncated.push_str("\n... [Truncated after 20000 characters]");
    truncated
}

#[must_use]
pub fn search_url(engine: &SearchEngine, query: &str) -> String {
    let encoded: String = form_urlencoded::byte_serialize(query.as_bytes()).collect();
    match engine {
        SearchEngine::DuckDuckGo => format!("https://duckduckgo.com/?q={encoded}"),
        SearchEngine::Google => format!("https://www.google.com/search?q={encoded}&udm=14"),
        SearchEngine::Bing => format!("https://www.bing.com/search?q={encoded}"),
    }
}

struct SearchTextResult {
    matches: Vec<String>,
    total: usize,
    has_more: bool,
}

fn search_text_matches(
    text: &str,
    pattern: &str,
    regex: bool,
    case_sensitive: bool,
    context_chars: usize,
    max_results: usize,
) -> Result<SearchTextResult, String> {
    if pattern.is_empty() || max_results == 0 {
        return Ok(SearchTextResult {
            matches: Vec::new(),
            total: 0,
            has_more: false,
        });
    }

    let pattern = if regex {
        pattern.to_owned()
    } else {
        regex::escape(pattern)
    };
    let matcher = regex::RegexBuilder::new(&pattern)
        .case_insensitive(!case_sensitive)
        .build()
        .map_err(|error| error.to_string())?;

    let mut matches = Vec::new();
    let mut total = 0;
    for hit in matcher.find_iter(text) {
        total += 1;
        if matches.len() < max_results {
            matches.push(context_snippet(text, hit.start(), hit.end(), context_chars));
        }
    }

    Ok(SearchTextResult {
        has_more: total > matches.len(),
        matches,
        total,
    })
}

fn format_search_page_results(pattern: &str, result: &SearchTextResult) -> String {
    if result.total == 0 {
        return format!("No matches found for \"{pattern}\" on page.");
    }

    let mut lines = vec![format!(
        "Found {} {} for \"{pattern}\" on page:",
        result.total,
        if result.total == 1 {
            "match"
        } else {
            "matches"
        }
    )];
    lines.push(String::new());
    lines.extend(
        result
            .matches
            .iter()
            .enumerate()
            .map(|(index, context)| format!("[{}] {context}", index + 1)),
    );

    if result.has_more {
        lines.push(format!(
            "\n... showing {} of {} total matches. Increase max_results to see more.",
            result.matches.len(),
            result.total
        ));
    }

    lines.join("\n")
}

fn format_find_elements_results(selector: &str, elements: &[FoundElement]) -> String {
    if elements.is_empty() {
        return format!("No elements found matching \"{selector}\".");
    }

    let mut lines = vec![format!(
        "Found {} {} matching \"{selector}\":",
        elements.len(),
        if elements.len() == 1 {
            "element"
        } else {
            "elements"
        }
    )];
    lines.push(String::new());
    lines.extend(
        elements
            .iter()
            .enumerate()
            .map(|(index, element)| format_found_element(index, element)),
    );
    lines.join("\n")
}

fn format_found_element(index: usize, element: &FoundElement) -> String {
    let mut parts = vec![format!("[{index}] <{}>", element.tag_name)];
    if let Some(text) = element
        .text
        .as_deref()
        .map(collapse_whitespace)
        .filter(|text| !text.is_empty())
    {
        parts.push(format!("\"{}\"", truncate_chars(&text, 120)));
    }
    if !element.attributes.is_empty() {
        let attrs = element
            .attributes
            .iter()
            .map(|(key, value)| format!("{key}=\"{}\"", truncate_chars(value, 500)))
            .collect::<Vec<_>>()
            .join(", ");
        parts.push(format!("{{{attrs}}}"));
    }
    parts.join(" ")
}

fn collapse_whitespace(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn truncate_chars(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_owned();
    }
    let mut truncated = text.chars().take(max_chars).collect::<String>();
    truncated.push_str("...");
    truncated
}

fn context_snippet(text: &str, start: usize, end: usize, context_chars: usize) -> String {
    let has_prefix = text[..start].chars().count() > context_chars;
    let has_suffix = text[end..].chars().count() > context_chars;
    let prefix: String = text[..start]
        .chars()
        .rev()
        .take(context_chars)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    let suffix: String = text[end..].chars().take(context_chars).collect();
    format!(
        "{}{prefix}{}{suffix}{}",
        if has_prefix { "..." } else { "" },
        &text[start..end],
        if has_suffix { "..." } else { "" }
    )
}

#[derive(Debug, Error)]
pub enum AgentRunError {
    #[error(transparent)]
    Browser(#[from] BrowserError),
    #[error(transparent)]
    Llm(#[from] LlmError),
    #[error("invalid agent output: {0}")]
    InvalidOutput(String),
    #[error("LLM call timed out after {seconds} seconds")]
    LlmTimedOut { seconds: u64 },
    #[error("agent step timed out after {seconds} seconds")]
    StepTimedOut { seconds: u64 },
    #[error("agent reached max steps ({max_steps}) without completing")]
    StepLimitReached { max_steps: usize },
    #[error("agent stopped after {failures} consecutive failures")]
    MaxFailuresExceeded { failures: u32 },
    #[error("agent repeated the same action sequence for {window} steps")]
    LoopDetected { window: usize },
}

pub struct Agent<M, S> {
    task: String,
    settings: AgentSettings,
    llm: M,
    executor: BrowserActionExecutor<S>,
    history: AgentHistory,
}

impl<M, S> Agent<M, S>
where
    M: ChatModel,
    S: BrowserSession + Send + Sync,
{
    #[must_use]
    pub fn new(task: impl Into<String>, llm: M, session: S) -> Self {
        Self::with_settings(task, AgentSettings::default(), llm, session)
    }

    #[must_use]
    pub fn with_settings(
        task: impl Into<String>,
        settings: AgentSettings,
        llm: M,
        session: S,
    ) -> Self {
        Self {
            task: task.into(),
            settings,
            llm,
            executor: BrowserActionExecutor::new(session),
            history: AgentHistory::default(),
        }
    }

    pub fn history(&self) -> &AgentHistory {
        &self.history
    }

    pub async fn run(&mut self, max_steps: usize) -> Result<&AgentHistory, AgentRunError> {
        let mut consecutive_failures = 0;

        for _ in 0..max_steps {
            let (is_done, has_error) = {
                let seconds = self.settings.step_timeout_seconds;
                let item = timeout(
                    Duration::from_secs(seconds),
                    self.step_recovering_model_errors(),
                )
                .await
                .map_err(|_| AgentRunError::StepTimedOut { seconds })??;
                (
                    item.result.iter().any(|result| result.is_done),
                    item.result.iter().any(|result| result.error.is_some()),
                )
            };

            if is_done {
                return Ok(&self.history);
            }

            if self.settings.loop_detection_enabled
                && repeated_action_loop(&self.history, self.settings.loop_detection_window)
            {
                return Err(AgentRunError::LoopDetected {
                    window: self.settings.loop_detection_window,
                });
            }

            if has_error {
                consecutive_failures += 1;
                if consecutive_failures >= self.settings.max_failures {
                    return Err(AgentRunError::MaxFailuresExceeded {
                        failures: consecutive_failures,
                    });
                }
            } else {
                consecutive_failures = 0;
            }
        }

        Err(AgentRunError::StepLimitReached { max_steps })
    }

    pub async fn step(&mut self) -> Result<&AgentHistoryItem, AgentRunError> {
        let seconds = self.settings.step_timeout_seconds;
        timeout(Duration::from_secs(seconds), self.step_inner())
            .await
            .map_err(|_| AgentRunError::StepTimedOut { seconds })?
    }

    async fn step_inner(&mut self) -> Result<&AgentHistoryItem, AgentRunError> {
        let step_start_time = now_seconds();
        let state = self
            .executor
            .session()
            .state(self.settings.use_vision)
            .await?;
        let request = build_step_request(&self.task, &state, &self.history, &self.settings)?;
        let completion = timeout(
            Duration::from_secs(self.settings.llm_timeout_seconds),
            self.llm.invoke_json(request),
        )
        .await
        .map_err(|_| AgentRunError::LlmTimedOut {
            seconds: self.settings.llm_timeout_seconds,
        })??;
        let model_output: AgentOutput = serde_json::from_value(completion.content)
            .map_err(|error| AgentRunError::InvalidOutput(error.to_string()))?;
        self.record_model_output(state, model_output, Some(step_start_time))
            .await
    }

    async fn step_recovering_model_errors(&mut self) -> Result<&AgentHistoryItem, AgentRunError> {
        let step_start_time = now_seconds();
        let state = self
            .executor
            .session()
            .state(self.settings.use_vision)
            .await?;
        let request = build_step_request(&self.task, &state, &self.history, &self.settings)?;
        let completion = match timeout(
            Duration::from_secs(self.settings.llm_timeout_seconds),
            self.llm.invoke_json(request),
        )
        .await
        {
            Ok(Ok(completion)) => completion,
            Ok(Err(error)) => {
                return self.record_model_error(
                    state,
                    format!("LLM provider error: {error}"),
                    Some(step_start_time),
                );
            }
            Err(_) => {
                return self.record_model_error(
                    state,
                    format!(
                        "LLM call timed out after {} seconds",
                        self.settings.llm_timeout_seconds
                    ),
                    Some(step_start_time),
                );
            }
        };
        let model_output: AgentOutput = match serde_json::from_value(completion.content) {
            Ok(model_output) => model_output,
            Err(error) => {
                return self.record_model_error(
                    state,
                    format!("invalid agent output: {error}"),
                    Some(step_start_time),
                );
            }
        };
        self.record_model_output(state, model_output, Some(step_start_time))
            .await
    }

    async fn record_model_output(
        &mut self,
        state: BrowserStateSummary,
        model_output: AgentOutput,
        step_start_time: Option<f64>,
    ) -> Result<&AgentHistoryItem, AgentRunError> {
        let result = if model_output.action.len() > self.settings.max_actions_per_step {
            vec![ActionResult::error(format!(
                "model returned {} actions, exceeding max_actions_per_step {}",
                model_output.action.len(),
                self.settings.max_actions_per_step
            ))]
        } else {
            self.executor.execute_sequence(&model_output.action).await
        };
        let metadata = step_start_time.map(|start| self.step_metadata(start, now_seconds()));

        self.history.items.push(AgentHistoryItem {
            model_output: Some(model_output),
            result,
            state,
            metadata,
        });

        self.history
            .items
            .last()
            .ok_or_else(|| AgentRunError::InvalidOutput("history item was not recorded".to_owned()))
    }

    fn record_model_error(
        &mut self,
        state: BrowserStateSummary,
        error: String,
        step_start_time: Option<f64>,
    ) -> Result<&AgentHistoryItem, AgentRunError> {
        let metadata = step_start_time.map(|start| self.step_metadata(start, now_seconds()));
        self.history.items.push(AgentHistoryItem {
            model_output: None,
            result: vec![ActionResult::error(error)],
            state,
            metadata,
        });

        self.history
            .items
            .last()
            .ok_or_else(|| AgentRunError::InvalidOutput("history item was not recorded".to_owned()))
    }

    fn step_metadata(&self, step_start_time: f64, step_end_time: f64) -> StepMetadata {
        let step_interval = self
            .history
            .items
            .last()
            .and_then(|item| item.metadata.as_ref())
            .map(|metadata| metadata.duration_seconds().max(0.0));

        StepMetadata {
            step_start_time,
            step_end_time,
            step_number: self.history.items.len() + 1,
            step_interval,
        }
    }
}

fn now_seconds() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
}

pub fn build_step_request(
    task: &str,
    state: &BrowserStateSummary,
    history: &AgentHistory,
    settings: &AgentSettings,
) -> Result<ChatRequest, AgentRunError> {
    let mut state_for_text = state.clone();
    state_for_text.screenshot = None;
    if !settings.include_attributes.is_empty() {
        state_for_text.dom_state.text = state
            .dom_state
            .llm_representation_with_attributes(&settings.include_attributes);
    }
    let state_json = serde_json::to_string_pretty(&state_for_text)
        .map_err(|error| AgentRunError::InvalidOutput(error.to_string()))?;
    let previous_results = render_previous_results(history, settings.max_history_items);
    let page_stats = render_page_stats(state);
    let loop_awareness = render_loop_awareness(history, state, settings)
        .map(|message| format!("\n\nLoop awareness:\n{message}"))
        .unwrap_or_default();
    let mut user_content = vec![ContentPart::Text {
        text: format!(
            "Task:\n{task}\n\nPrevious action results:\n{previous_results}\n\nPage stats:\n{page_stats}{loop_awareness}\n\nBrowser state:\n{state_json}"
        ),
    }];
    if settings.use_vision
        && let Some(screenshot) = state.screenshot.as_deref()
    {
        user_content.push(ContentPart::ImageUrl {
            image_url: screenshot_data_url(screenshot),
        });
    }
    Ok(ChatRequest {
        messages: vec![
            ChatMessage::text(
                MessageRole::System,
                format!(
                    "You are controlling a browser. Return a JSON object matching AgentOutput. \
	                     Use at most {} actions in this step. Avoid repeating the same action \
	                     sequence; if the browser is not changing, choose a different strategy \
	                     or finish with done.",
                    settings.max_actions_per_step
                ),
            ),
            ChatMessage {
                role: MessageRole::User,
                content: user_content,
            },
        ],
        output_schema: Some(schema_for_agent_output()),
    })
}

fn screenshot_data_url(screenshot: &str) -> String {
    if screenshot.starts_with("data:image/") {
        screenshot.to_owned()
    } else {
        format!("data:image/png;base64,{screenshot}")
    }
}

fn render_page_stats(state: &BrowserStateSummary) -> String {
    let indexed_elements = state.dom_state.selector_map.values();
    let total_indexed = state.dom_state.selector_map.len();
    let interactive = indexed_elements
        .clone()
        .filter(|element| element.is_interactive)
        .count();
    let links = indexed_elements
        .clone()
        .filter(|element| element.tag_name == "a")
        .count();
    let iframes = indexed_elements
        .clone()
        .filter(|element| matches!(element.tag_name.as_str(), "iframe" | "frame"))
        .count();
    let scroll_containers = indexed_elements
        .clone()
        .filter(|element| element.is_scrollable)
        .count();
    let text_chars = state.dom_state.text.chars().count();

    let mut stats = format!(
        "<page_stats>{links} links, {interactive} interactive, {iframes} iframes, {scroll_containers} scroll containers, {total_indexed} indexed elements, {text_chars} text chars"
    );

    if let Some(page_info) = state.page_info {
        stats.push_str(&format!(
            ", {}px above, {}px below",
            page_info.pixels_above, page_info.pixels_below
        ));
    }

    stats.push_str("</page_stats>");
    stats
}

fn schema_for_agent_output() -> Value {
    serde_json::to_value(schemars::schema_for!(AgentOutput)).unwrap_or(Value::Null)
}

fn render_previous_results(history: &AgentHistory, max_history_items: Option<usize>) -> String {
    let max_history_items = max_history_items.unwrap_or(5);
    let recent_items: Vec<&AgentHistoryItem> = history
        .items
        .iter()
        .rev()
        .take(max_history_items)
        .rev()
        .collect();
    let latest_recent_index = recent_items.len().saturating_sub(1);
    let rendered: Vec<String> = recent_items
        .into_iter()
        .enumerate()
        .flat_map(|(step_index, item)| {
            item.result
                .iter()
                .enumerate()
                .map(move |(result_index, result)| {
                    let text = render_result_for_prompt(result, step_index == latest_recent_index);
                    format!("{}.{} {}", step_index + 1, result_index + 1, text)
                })
        })
        .collect();

    if rendered.is_empty() {
        "None".to_owned()
    } else {
        rendered.join("\n")
    }
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

fn render_result_for_prompt(result: &ActionResult, is_latest_step: bool) -> &str {
    if let Some(error) = result.error.as_deref() {
        return error;
    }

    if result.include_extracted_content_only_once && !is_latest_step {
        return result
            .long_term_memory
            .as_deref()
            .unwrap_or("[extracted content omitted after first prompt]");
    }

    result
        .extracted_content
        .as_deref()
        .or(result.long_term_memory.as_deref())
        .unwrap_or("no content")
}

fn repeated_action_loop(history: &AgentHistory, window: usize) -> bool {
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
                .and_then(|output| serde_json::to_string(&output.action).ok())
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

/// Execute actions with the same high-level guard shape as browser-use:
/// stop on errors, stop on `done`, stop after sequence-terminating actions,
/// and refuse `done` when it is queued after another action.
pub async fn execute_action_sequence<E>(
    executor: &mut E,
    actions: &[BrowserAction],
) -> Vec<ActionResult>
where
    E: ActionExecutor + Send,
{
    let mut results = Vec::new();

    for (index, action) in actions.iter().enumerate() {
        if index > 0 && matches!(action, BrowserAction::Done(_)) {
            break;
        }

        let result = executor.execute(action).await;
        let should_stop = result.is_done || result.error.is_some() || action.terminates_sequence();
        results.push(result);

        if should_stop {
            break;
        }
    }

    results
}

#[cfg(test)]
mod tests {
    use super::*;
    use browser_use_cdp::{FoundElement, Pdf, Screenshot};
    use browser_use_dom::SerializedDomState;
    use browser_use_tools::{
        ClickElementAction, CloseTabAction, DoneAction, EvaluateAction, ExtractAction,
        FindElementsAction, FindTextAction, GetDropdownOptionsAction, InputTextAction,
        NavigateAction, NoParamsAction, ReadFileAction, ReplaceFileAction, SaveAsPdfAction,
        ScreenshotAction, ScrollAction, SearchPageAction, SelectDropdownOptionAction,
        SendKeysAction, SwitchTabAction, UploadFileAction, WaitAction, WriteFileAction,
    };
    use std::{collections::BTreeMap, collections::VecDeque, sync::Mutex};

    #[test]
    fn target_commit_is_pinned() {
        assert_eq!(INITIAL_UPSTREAM_COMMIT.len(), 40);
    }

    #[test]
    fn settings_defaults_match_browser_use_shape() {
        let settings = AgentSettings::default();

        assert_eq!(settings.max_failures, 5);
        assert_eq!(settings.max_actions_per_step, 5);
        assert_eq!(settings.llm_timeout_seconds, 60);
        assert_eq!(settings.step_timeout_seconds, 180);
        assert_eq!(settings.loop_detection_window, 20);
        assert!(settings.loop_detection_enabled);
        assert_eq!(settings.max_history_items, None);
        assert!(settings.include_attributes.is_empty());
    }

    #[test]
    fn history_returns_latest_done_result() {
        let history = AgentHistory {
            items: vec![AgentHistoryItem {
                model_output: None,
                result: vec![ActionResult::done("finished", true)],
                state: blank_state(),
                metadata: None,
            }],
        };

        assert_eq!(history.final_result(), Some("finished"));
        assert!(history.is_done());
        assert_eq!(history.is_successful(), Some(true));
        assert!(history.errors().is_empty());
    }

    #[test]
    fn history_collects_errors_and_failed_done_status() {
        let history = AgentHistory {
            items: vec![
                AgentHistoryItem {
                    model_output: None,
                    result: vec![ActionResult::error("first failure")],
                    state: blank_state(),
                    metadata: None,
                },
                AgentHistoryItem {
                    model_output: None,
                    result: vec![ActionResult::done("could not finish", false)],
                    state: blank_state(),
                    metadata: None,
                },
            ],
        };

        assert_eq!(history.final_result(), Some("could not finish"));
        assert!(history.is_done());
        assert_eq!(history.is_successful(), Some(false));
        assert_eq!(history.errors(), vec!["first failure"]);
    }

    #[test]
    fn step_metadata_matches_browser_use_shape() {
        let metadata = StepMetadata {
            step_number: 2,
            step_start_time: 10.0,
            step_end_time: 13.5,
            step_interval: Some(2.0),
        };

        assert_eq!(metadata.duration_seconds(), 3.5);

        let serialized = serde_json::to_value(&metadata).expect("serialize metadata");
        assert_eq!(serialized["step_number"], 2);
        assert_eq!(serialized["step_start_time"], 10.0);
        assert_eq!(serialized["step_end_time"], 13.5);
        assert_eq!(serialized["step_interval"], 2.0);

        let without_interval: StepMetadata = serde_json::from_value(serde_json::json!({
            "step_number": 1,
            "step_start_time": 1.0,
            "step_end_time": 2.0
        }))
        .expect("deserialize old metadata");
        assert_eq!(without_interval.step_interval, None);

        let serialized_without_interval =
            serde_json::to_value(&without_interval).expect("serialize metadata");
        assert!(serialized_without_interval.get("step_interval").is_some());
        assert_eq!(serialized_without_interval["step_interval"], Value::Null);
    }

    #[test]
    fn history_helpers_match_browser_use_accessors() {
        let mut first_state = blank_state();
        first_state.url = "https://example.test/start".to_owned();
        let mut second_state = blank_state();
        second_state.url = "https://example.test/done".to_owned();
        let current_state = AgentCurrentState {
            thinking: None,
            evaluation_previous_goal: None,
            memory: None,
            next_goal: None,
        };
        let history = AgentHistory {
            items: vec![
                AgentHistoryItem {
                    model_output: Some(AgentOutput {
                        current_state: current_state.clone(),
                        action: vec![BrowserAction::Click(ClickElementAction {
                            index: Some(1),
                            coordinate_x: None,
                            coordinate_y: None,
                        })],
                    }),
                    result: vec![ActionResult::extracted("Clicked element 1")],
                    state: first_state,
                    metadata: Some(StepMetadata {
                        step_number: 1,
                        step_start_time: 10.0,
                        step_end_time: 11.5,
                        step_interval: None,
                    }),
                },
                AgentHistoryItem {
                    model_output: Some(AgentOutput {
                        current_state,
                        action: vec![BrowserAction::Done(DoneAction {
                            text: "finished".to_owned(),
                            success: true,
                            files_to_display: Vec::new(),
                        })],
                    }),
                    result: vec![ActionResult::done("finished", true)],
                    state: second_state,
                    metadata: Some(StepMetadata {
                        step_number: 2,
                        step_start_time: 20.0,
                        step_end_time: 22.0,
                        step_interval: Some(1.5),
                    }),
                },
            ],
        };

        assert_eq!(history.number_of_steps(), 2);
        assert_eq!(history.action_results().len(), 2);
        assert_eq!(
            history.extracted_content(),
            vec!["Clicked element 1", "finished"]
        );
        assert_eq!(
            history.urls(),
            vec!["https://example.test/start", "https://example.test/done"]
        );
        assert_eq!(history.action_names(), vec!["click", "done"]);
        assert!((history.total_duration_seconds() - 3.5).abs() < f64::EPSILON);
    }

    fn blank_state() -> BrowserStateSummary {
        BrowserStateSummary {
            dom_state: Default::default(),
            url: "about:blank".to_owned(),
            title: "Blank".to_owned(),
            tabs: vec![],
            screenshot: None,
            page_info: None,
            pixels_above: 0,
            pixels_below: 0,
            browser_errors: vec![],
            is_pdf_viewer: false,
            recent_events: None,
            pending_network_requests: vec![],
            pagination_buttons: vec![],
            closed_popup_messages: vec![],
        }
    }

    struct RecordingExecutor {
        seen: Vec<&'static str>,
    }

    #[async_trait]
    impl ActionExecutor for RecordingExecutor {
        async fn execute(&mut self, action: &BrowserAction) -> ActionResult {
            self.seen.push(action.name());
            ActionResult {
                extracted_content: Some(action.name().to_owned()),
                error: None,
                long_term_memory: None,
                include_extracted_content_only_once: false,
                include_in_memory: false,
                is_done: matches!(action, BrowserAction::Done(_)),
                success: None,
                metadata: None,
            }
        }
    }

    #[tokio::test]
    async fn action_sequence_stops_after_navigation() {
        let actions = vec![
            BrowserAction::Navigate(NavigateAction {
                url: "https://example.com".to_owned(),
                new_tab: false,
            }),
            BrowserAction::Click(ClickElementAction {
                index: Some(1),
                coordinate_x: None,
                coordinate_y: None,
            }),
        ];
        let mut executor = RecordingExecutor { seen: Vec::new() };

        let results = execute_action_sequence(&mut executor, &actions).await;

        assert_eq!(executor.seen, vec!["navigate"]);
        assert_eq!(results.len(), 1);
    }

    #[tokio::test]
    async fn done_is_only_executed_when_first_action() {
        let actions = vec![
            BrowserAction::Click(ClickElementAction {
                index: Some(1),
                coordinate_x: None,
                coordinate_y: None,
            }),
            BrowserAction::Done(DoneAction {
                text: "finished".to_owned(),
                success: true,
                files_to_display: vec![],
            }),
        ];
        let mut executor = RecordingExecutor { seen: Vec::new() };

        let results = execute_action_sequence(&mut executor, &actions).await;

        assert_eq!(executor.seen, vec!["click"]);
        assert_eq!(results.len(), 1);
    }

    struct MockSession {
        events: Mutex<Vec<String>>,
        states: Mutex<VecDeque<BrowserStateSummary>>,
        state_screenshot_requests: Mutex<Vec<bool>>,
        state_error: Mutex<Option<String>>,
        click_error: Mutex<Option<String>>,
    }

    impl MockSession {
        fn new() -> Self {
            Self {
                events: Mutex::new(Vec::new()),
                states: Mutex::new(VecDeque::new()),
                state_screenshot_requests: Mutex::new(Vec::new()),
                state_error: Mutex::new(None),
                click_error: Mutex::new(None),
            }
        }

        fn with_states(states: Vec<BrowserStateSummary>) -> Self {
            Self {
                events: Mutex::new(Vec::new()),
                states: Mutex::new(VecDeque::from(states)),
                state_screenshot_requests: Mutex::new(Vec::new()),
                state_error: Mutex::new(None),
                click_error: Mutex::new(None),
            }
        }

        fn with_state_error(error: impl Into<String>) -> Self {
            Self {
                events: Mutex::new(Vec::new()),
                states: Mutex::new(VecDeque::new()),
                state_screenshot_requests: Mutex::new(Vec::new()),
                state_error: Mutex::new(Some(error.into())),
                click_error: Mutex::new(None),
            }
        }

        fn with_click_error(error: impl Into<String>) -> Self {
            Self {
                events: Mutex::new(Vec::new()),
                states: Mutex::new(VecDeque::new()),
                state_screenshot_requests: Mutex::new(Vec::new()),
                state_error: Mutex::new(None),
                click_error: Mutex::new(Some(error.into())),
            }
        }

        fn events(&self) -> Vec<String> {
            self.events.lock().expect("events lock").clone()
        }

        fn state_screenshot_requests(&self) -> Vec<bool> {
            self.state_screenshot_requests
                .lock()
                .expect("state requests lock")
                .clone()
        }
    }

    #[async_trait]
    impl BrowserSession for MockSession {
        async fn state(
            &self,
            include_screenshot: bool,
        ) -> Result<BrowserStateSummary, BrowserError> {
            self.state_screenshot_requests
                .lock()
                .expect("state requests lock")
                .push(include_screenshot);
            if let Some(error) = self.state_error.lock().expect("state error lock").clone() {
                return Err(BrowserError::StateUnavailable(error));
            }
            if let Some(state) = self.states.lock().expect("states lock").pop_front() {
                return Ok(state);
            }
            Ok(BrowserStateSummary {
                dom_state: SerializedDomState::default(),
                ..blank_state()
            })
        }

        async fn navigate(&self, url: &str, new_tab: bool) -> Result<(), BrowserError> {
            self.events
                .lock()
                .expect("events lock")
                .push(format!("navigate:{url}:{new_tab}"));
            Ok(())
        }

        async fn go_back(&self) -> Result<(), BrowserError> {
            self.events
                .lock()
                .expect("events lock")
                .push("go_back".to_owned());
            Ok(())
        }

        async fn switch_tab(&self, target_id: &str) -> Result<(), BrowserError> {
            self.events
                .lock()
                .expect("events lock")
                .push(format!("switch_tab:{target_id}"));
            Ok(())
        }

        async fn close_tab(&self, target_id: &str) -> Result<(), BrowserError> {
            self.events
                .lock()
                .expect("events lock")
                .push(format!("close_tab:{target_id}"));
            Ok(())
        }

        async fn click(&self, index: u32) -> Result<(), BrowserError> {
            self.events
                .lock()
                .expect("events lock")
                .push(format!("click:{index}"));
            if let Some(error) = self.click_error.lock().expect("click error lock").take() {
                return Err(BrowserError::ActionFailed(error));
            }
            Ok(())
        }

        async fn click_coordinates(&self, x: i32, y: i32) -> Result<(), BrowserError> {
            self.events
                .lock()
                .expect("events lock")
                .push(format!("click_coordinates:{x}:{y}"));
            Ok(())
        }

        async fn input_text(
            &self,
            index: u32,
            text: &str,
            clear: bool,
        ) -> Result<(), BrowserError> {
            self.events
                .lock()
                .expect("events lock")
                .push(format!("input:{index}:{text}:{clear}"));
            Ok(())
        }

        async fn scroll(
            &self,
            index: Option<u32>,
            down: bool,
            pages: f64,
        ) -> Result<(), BrowserError> {
            self.events
                .lock()
                .expect("events lock")
                .push(format!("scroll:{index:?}:{down}:{pages}"));
            Ok(())
        }

        async fn find_text(&self, text: &str) -> Result<bool, BrowserError> {
            self.events
                .lock()
                .expect("events lock")
                .push(format!("find_text:{text}"));
            Ok(true)
        }

        async fn evaluate(&self, code: &str) -> Result<String, BrowserError> {
            self.events
                .lock()
                .expect("events lock")
                .push(format!("evaluate:{code}"));
            Ok("EvalOps JS result".to_owned())
        }

        async fn dropdown_options(&self, index: u32) -> Result<Vec<String>, BrowserError> {
            self.events
                .lock()
                .expect("events lock")
                .push(format!("dropdown_options:{index}"));
            Ok(vec!["One".to_owned(), "Two".to_owned()])
        }

        async fn select_dropdown_option(&self, index: u32, text: &str) -> Result<(), BrowserError> {
            self.events
                .lock()
                .expect("events lock")
                .push(format!("select_dropdown_option:{index}:{text}"));
            Ok(())
        }

        async fn page_text(&self) -> Result<String, BrowserError> {
            self.events
                .lock()
                .expect("events lock")
                .push("page_text".to_owned());
            Ok("Alpha EvalOps Beta\nSecond EvalOps line".to_owned())
        }

        async fn find_elements(
            &self,
            selector: &str,
            attributes: &[String],
            max_results: usize,
            include_text: bool,
        ) -> Result<Vec<FoundElement>, BrowserError> {
            self.events.lock().expect("events lock").push(format!(
                "find_elements:{selector}:{}:{max_results}:{include_text}",
                attributes.join("|")
            ));
            if selector == "a[href]" {
                let mut attrs = BTreeMap::new();
                attrs.insert("href".to_owned(), "https://evalops.dev/run".to_owned());
                attrs.insert("title".to_owned(), "Run EvalOps".to_owned());
                return Ok(vec![FoundElement {
                    tag_name: "a".to_owned(),
                    text: include_text.then(|| "Run EvalOps".to_owned()),
                    attributes: attrs,
                }]);
            }
            if selector == "img[src], img[data-src], picture source[srcset]" {
                let mut attrs = BTreeMap::new();
                attrs.insert("src".to_owned(), "https://evalops.dev/hero.png".to_owned());
                attrs.insert("alt".to_owned(), "Hero shot".to_owned());
                return Ok(vec![FoundElement {
                    tag_name: "img".to_owned(),
                    text: include_text.then(|| "ignored".to_owned()),
                    attributes: attrs,
                }]);
            }
            let mut attrs = BTreeMap::new();
            attrs.insert("id".to_owned(), "run".to_owned());
            Ok(vec![FoundElement {
                tag_name: "button".to_owned(),
                text: include_text.then(|| "Run EvalOps".to_owned()),
                attributes: attrs,
            }])
        }

        async fn send_keys(&self, keys: &str) -> Result<(), BrowserError> {
            self.events
                .lock()
                .expect("events lock")
                .push(format!("send_keys:{keys}"));
            Ok(())
        }

        async fn upload_file(
            &self,
            index: u32,
            path: &std::path::Path,
        ) -> Result<(), BrowserError> {
            self.events
                .lock()
                .expect("events lock")
                .push(format!("upload_file:{index}:{}", path.display()));
            Ok(())
        }

        async fn screenshot(&self) -> Result<Screenshot, BrowserError> {
            self.events
                .lock()
                .expect("events lock")
                .push("screenshot".to_owned());
            Ok(Screenshot {
                base64_png: base64::engine::general_purpose::STANDARD.encode("PNGDATA"),
            })
        }

        async fn save_pdf(
            &self,
            print_background: bool,
            landscape: bool,
            scale: f64,
            paper_format: &str,
        ) -> Result<Pdf, BrowserError> {
            self.events.lock().expect("events lock").push(format!(
                "save_pdf:{print_background}:{landscape}:{scale}:{paper_format}"
            ));
            Ok(Pdf {
                base64_pdf: base64::engine::general_purpose::STANDARD.encode("%PDF-1.7"),
            })
        }
    }

    #[tokio::test]
    async fn browser_executor_maps_navigate_to_session() {
        let session = MockSession::new();
        let mut executor = BrowserActionExecutor::new(session);

        let result = executor
            .execute(&BrowserAction::Navigate(NavigateAction {
                url: "https://example.com".to_owned(),
                new_tab: false,
            }))
            .await;

        assert!(result.error.is_none());
        assert_eq!(
            executor.session().events(),
            vec!["navigate:https://example.com:false"]
        );
    }

    #[tokio::test]
    async fn terminating_sequence_action_does_not_need_pre_state() {
        let session = MockSession::with_state_error("state unavailable");
        let mut executor = BrowserActionExecutor::new(session);

        let results = executor
            .execute_sequence(&[BrowserAction::Navigate(NavigateAction {
                url: "https://example.com".to_owned(),
                new_tab: false,
            })])
            .await;

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].error, None);
        assert_eq!(
            executor.session().events(),
            vec!["navigate:https://example.com:false"]
        );
        assert!(executor.session().state_screenshot_requests().is_empty());
    }

    #[tokio::test]
    async fn browser_executor_stops_sequence_when_url_changes() {
        let mut first_state = blank_state();
        first_state.url = "https://example.com/one".to_owned();
        let mut second_state = blank_state();
        second_state.url = "https://example.com/two".to_owned();
        let session = MockSession::with_states(vec![first_state, second_state]);
        let mut executor = BrowserActionExecutor::new(session);

        let results = executor
            .execute_sequence(&[
                BrowserAction::Click(ClickElementAction {
                    index: Some(1),
                    coordinate_x: None,
                    coordinate_y: None,
                }),
                BrowserAction::Input(InputTextAction {
                    index: 2,
                    text: "should not type".to_owned(),
                    clear: true,
                }),
            ])
            .await;

        assert_eq!(results.len(), 1);
        assert_eq!(executor.session().events(), vec!["click:1"]);
    }

    #[tokio::test]
    async fn browser_executor_rejects_zero_click_index() {
        let session = MockSession::new();
        let mut executor = BrowserActionExecutor::new(session);

        let result = executor
            .execute(&BrowserAction::Click(ClickElementAction {
                index: Some(0),
                coordinate_x: None,
                coordinate_y: None,
            }))
            .await;

        assert!(
            result
                .error
                .as_deref()
                .expect("click error")
                .contains("index 0")
        );
        assert_eq!(executor.session().events(), Vec::<String>::new());
    }

    #[tokio::test]
    async fn browser_executor_click_select_returns_dropdown_options() {
        let session = MockSession::with_click_error(
            "Cannot click on <select> elements. Use get_dropdown_options and select_dropdown_option instead.",
        );
        let mut executor = BrowserActionExecutor::new(session);

        let result = executor
            .execute(&BrowserAction::Click(ClickElementAction {
                index: Some(1),
                coordinate_x: None,
                coordinate_y: None,
            }))
            .await;

        assert_eq!(
            result.extracted_content.as_deref(),
            Some("Dropdown options: One, Two")
        );
        assert_eq!(
            executor.session().events(),
            vec!["click:1", "dropdown_options:1"]
        );
    }

    #[tokio::test]
    async fn browser_executor_click_file_input_points_to_upload_file() {
        let session = MockSession::with_click_error(
            "Cannot click on file input elements. Use upload_file instead.",
        );
        let mut executor = BrowserActionExecutor::new(session);

        let result = executor
            .execute(&BrowserAction::Click(ClickElementAction {
                index: Some(2),
                coordinate_x: None,
                coordinate_y: None,
            }))
            .await;

        assert!(
            result
                .error
                .as_deref()
                .expect("file input click error")
                .contains("upload_file")
        );
        assert_eq!(executor.session().events(), vec!["click:2"]);
    }

    #[tokio::test]
    async fn browser_executor_treats_zero_scroll_index_as_page_scroll() {
        let session = MockSession::new();
        let mut executor = BrowserActionExecutor::new(session);

        let result = executor
            .execute(&BrowserAction::Scroll(ScrollAction {
                down: true,
                pages: 1.0,
                index: Some(0),
            }))
            .await;

        assert_eq!(result.error, None);
        assert_eq!(executor.session().events(), vec!["scroll:None:true:1"]);
    }

    #[tokio::test]
    async fn browser_executor_maps_tab_actions_to_session() {
        let session = MockSession::new();
        let mut executor = BrowserActionExecutor::new(session);

        let switch_result = executor
            .execute(&BrowserAction::SwitchTab(SwitchTabAction {
                tab_id: "target-2".to_owned(),
            }))
            .await;
        let close_result = executor
            .execute(&BrowserAction::CloseTab(CloseTabAction {
                tab_id: "target-1".to_owned(),
            }))
            .await;

        assert_eq!(switch_result.error, None);
        assert_eq!(close_result.error, None);
        assert_eq!(
            executor.session().events(),
            vec!["switch_tab:target-2", "close_tab:target-1"]
        );
    }

    #[tokio::test]
    async fn browser_executor_maps_go_back_to_session() {
        let session = MockSession::new();
        let mut executor = BrowserActionExecutor::new(session);

        let result = executor
            .execute(&BrowserAction::GoBack(NoParamsAction { description: None }))
            .await;

        assert_eq!(result.error, None);
        assert_eq!(result.extracted_content.as_deref(), Some("Navigated back"));
        assert_eq!(executor.session().events(), vec!["go_back"]);
    }

    #[tokio::test]
    async fn browser_executor_maps_find_text_to_session() {
        let session = MockSession::new();
        let mut executor = BrowserActionExecutor::new(session);

        let result = executor
            .execute(&BrowserAction::FindText(FindTextAction {
                text: "Needle".to_owned(),
            }))
            .await;

        assert_eq!(result.error, None);
        assert_eq!(
            result.extracted_content.as_deref(),
            Some("Scrolled to text: Needle")
        );
        assert_eq!(executor.session().events(), vec!["find_text:Needle"]);
    }

    #[tokio::test]
    async fn browser_executor_maps_evaluate_to_session() {
        let session = MockSession::new();
        let mut executor = BrowserActionExecutor::new(session);

        let result = executor
            .execute(&BrowserAction::Evaluate(EvaluateAction {
                code: "document.title".to_owned(),
            }))
            .await;

        assert_eq!(result.error, None);
        assert_eq!(
            result.extracted_content.as_deref(),
            Some("EvalOps JS result")
        );
        assert_eq!(executor.session().events(), vec!["evaluate:document.title"]);
    }

    #[tokio::test]
    async fn browser_executor_handles_text_file_actions() {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let path = temp_dir.path().join("notes.md");
        let file_name = path.display().to_string();
        let session = MockSession::new();
        let mut executor = BrowserActionExecutor::new(session);

        let write_result = executor
            .execute(&BrowserAction::WriteFile(WriteFileAction {
                file_name: file_name.clone(),
                content: "hello world".to_owned(),
                append: false,
                trailing_newline: true,
                leading_newline: false,
            }))
            .await;
        let append_result = executor
            .execute(&BrowserAction::WriteFile(WriteFileAction {
                file_name: file_name.clone(),
                content: "world".to_owned(),
                append: true,
                trailing_newline: false,
                leading_newline: true,
            }))
            .await;
        let replace_result = executor
            .execute(&BrowserAction::ReplaceFile(ReplaceFileAction {
                file_name: file_name.clone(),
                old_str: "world".to_owned(),
                new_str: "EvalOps".to_owned(),
            }))
            .await;
        let read_result = executor
            .execute(&BrowserAction::ReadFile(ReadFileAction {
                file_name: file_name.clone(),
            }))
            .await;

        assert_eq!(write_result.error, None);
        assert_eq!(append_result.error, None);
        assert_eq!(replace_result.error, None);
        assert_eq!(
            std::fs::read_to_string(&path).expect("file content"),
            "hello EvalOps\n\nEvalOps"
        );
        assert!(
            read_result
                .extracted_content
                .as_deref()
                .expect("read content")
                .contains("hello EvalOps\n\nEvalOps")
        );
        assert!(read_result.include_extracted_content_only_once);
    }

    #[tokio::test]
    async fn browser_executor_rejects_unsupported_text_file_actions() {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let session = MockSession::new();
        let mut executor = BrowserActionExecutor::new(session);

        let binary_result = executor
            .execute(&BrowserAction::WriteFile(WriteFileAction {
                file_name: temp_dir.path().join("image.png").display().to_string(),
                content: "not really an image".to_owned(),
                append: false,
                trailing_newline: true,
                leading_newline: false,
            }))
            .await;
        let extensionless_result = executor
            .execute(&BrowserAction::ReadFile(ReadFileAction {
                file_name: temp_dir.path().join("notes").display().to_string(),
            }))
            .await;

        let editable_path = temp_dir.path().join("notes.md");
        std::fs::write(&editable_path, "hello").expect("seed editable file");
        let empty_replace_result = executor
            .execute(&BrowserAction::ReplaceFile(ReplaceFileAction {
                file_name: editable_path.display().to_string(),
                old_str: String::new(),
                new_str: "EvalOps".to_owned(),
            }))
            .await;

        assert!(
            binary_result
                .error
                .as_deref()
                .expect("binary error")
                .contains("binary/image file")
        );
        assert!(
            extensionless_result
                .error
                .as_deref()
                .expect("extension error")
                .contains("has no extension")
        );
        assert!(
            empty_replace_result
                .error
                .as_deref()
                .expect("empty replace error")
                .contains("Cannot replace empty string")
        );
        assert_eq!(
            std::fs::read_to_string(editable_path).expect("editable content"),
            "hello"
        );
    }

    #[tokio::test]
    async fn browser_executor_maps_dropdown_actions_to_session() {
        let session = MockSession::new();
        let mut executor = BrowserActionExecutor::new(session);

        let options_result = executor
            .execute(&BrowserAction::GetDropdownOptions(
                GetDropdownOptionsAction { index: 1 },
            ))
            .await;
        let select_result = executor
            .execute(&BrowserAction::SelectDropdownOption(
                SelectDropdownOptionAction {
                    index: 1,
                    text: "Two".to_owned(),
                },
            ))
            .await;

        assert!(
            options_result
                .extracted_content
                .expect("options content")
                .contains("One, Two")
        );
        assert!(select_result.error.is_none());
        assert_eq!(
            executor.session().events(),
            vec!["dropdown_options:1", "select_dropdown_option:1:Two"]
        );
    }

    #[tokio::test]
    async fn browser_executor_maps_send_keys_to_session() {
        let session = MockSession::new();
        let mut executor = BrowserActionExecutor::new(session);

        let result = executor
            .execute(&BrowserAction::SendKeys(SendKeysAction {
                keys: "EvalOps".to_owned(),
            }))
            .await;

        assert_eq!(result.error, None);
        assert_eq!(executor.session().events(), vec!["send_keys:EvalOps"]);
    }

    #[tokio::test]
    async fn browser_executor_maps_wait_without_session_event() {
        let session = MockSession::new();
        let mut executor = BrowserActionExecutor::new(session);

        let result = executor
            .execute(&BrowserAction::Wait(WaitAction { seconds: 0 }))
            .await;

        assert_eq!(result.error, None);
        assert_eq!(
            result.extracted_content.as_deref(),
            Some("Waited for 0 seconds")
        );
        assert_eq!(executor.session().events(), Vec::<String>::new());
    }

    #[tokio::test]
    async fn browser_executor_requests_next_screenshot_observation() {
        let session = MockSession::new();
        let mut executor = BrowserActionExecutor::new(session);

        let result = executor
            .execute(&BrowserAction::Screenshot(ScreenshotAction {
                file_name: None,
            }))
            .await;

        assert_eq!(result.error, None);
        assert_eq!(
            result.extracted_content.as_deref(),
            Some("Requested screenshot for next observation")
        );
        assert_eq!(executor.session().events(), Vec::<String>::new());
    }

    #[tokio::test]
    async fn browser_executor_saves_screenshot_when_file_name_is_present() {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let requested_output_path = temp_dir.path().join("shot");
        let output_path = temp_dir.path().join("shot.png");
        let session = MockSession::new();
        let mut executor = BrowserActionExecutor::new(session);

        let result = executor
            .execute(&BrowserAction::Screenshot(ScreenshotAction {
                file_name: Some(requested_output_path.display().to_string()),
            }))
            .await;

        assert_eq!(result.error, None);
        assert!(
            result
                .extracted_content
                .expect("screenshot content")
                .contains(&output_path.display().to_string())
        );
        assert_eq!(
            std::fs::read(&output_path).expect("screenshot file"),
            b"PNGDATA"
        );
        assert_eq!(executor.session().events(), vec!["screenshot"]);
    }

    #[test]
    fn screenshot_output_path_appends_png_extension() {
        assert_eq!(
            screenshot_output_path("/tmp/shot"),
            std::path::PathBuf::from("/tmp/shot.png")
        );
        assert_eq!(
            screenshot_output_path("/tmp/shot.PNG"),
            std::path::PathBuf::from("/tmp/shot.PNG")
        );
        assert_eq!(
            screenshot_output_path(""),
            std::path::PathBuf::from("screenshot.png")
        );
    }

    #[tokio::test]
    async fn browser_executor_maps_upload_file_to_session() {
        let session = MockSession::new();
        let mut executor = BrowserActionExecutor::new(session);

        let result = executor
            .execute(&BrowserAction::UploadFile(UploadFileAction {
                index: 3,
                path: "/tmp/evalops-upload.txt".to_owned(),
            }))
            .await;

        assert_eq!(result.error, None);
        assert_eq!(
            result.extracted_content.as_deref(),
            Some("Uploaded /tmp/evalops-upload.txt to element 3")
        );
        assert_eq!(
            executor.session().events(),
            vec!["upload_file:3:/tmp/evalops-upload.txt"]
        );
    }

    #[tokio::test]
    async fn browser_executor_saves_pdf_when_file_name_is_present() {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let requested_output_path = temp_dir.path().join("out");
        let output_path = temp_dir.path().join("out.pdf");
        let output = requested_output_path.display().to_string();
        let session = MockSession::new();
        let mut executor = BrowserActionExecutor::new(session);

        let result = executor
            .execute(&BrowserAction::SaveAsPdf(SaveAsPdfAction {
                file_name: Some(output.clone()),
                print_background: true,
                landscape: false,
                scale: 1.0,
                paper_format: "Letter".to_owned(),
            }))
            .await;

        assert_eq!(result.error, None);
        assert!(
            result
                .extracted_content
                .expect("pdf content")
                .contains(&output_path.display().to_string())
        );
        assert!(
            std::fs::read(&output_path)
                .expect("pdf file")
                .starts_with(b"%PDF")
        );
        assert_eq!(
            executor.session().events(),
            vec!["save_pdf:true:false:1:Letter"]
        );
    }

    #[test]
    fn pdf_output_path_uses_sanitized_title_and_deduplicates() {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let existing = temp_dir.path().join("Quarterly Plan  2026.pdf");
        std::fs::write(&existing, b"existing").expect("existing pdf");

        let output = pdf_output_path(None, Some("Quarterly: Plan / 2026"));
        assert_eq!(output, std::path::PathBuf::from("Quarterly Plan  2026.pdf"));

        let duplicate = next_available_pdf_path(existing);
        assert_eq!(
            duplicate.file_name().and_then(std::ffi::OsStr::to_str),
            Some("Quarterly Plan  2026 (1).pdf")
        );
        assert_eq!(
            pdf_output_path(Some("/tmp/report"), None),
            std::path::PathBuf::from("/tmp/report.pdf")
        );
        assert_eq!(
            pdf_output_path(Some("/tmp/report.PDF"), None),
            std::path::PathBuf::from("/tmp/report.PDF")
        );
    }

    #[tokio::test]
    async fn browser_executor_extracts_page_text() {
        let session = MockSession::new();
        let mut executor = BrowserActionExecutor::new(session);

        let result = executor
            .execute(&BrowserAction::Extract(ExtractAction {
                query: "company".to_owned(),
                extract_links: false,
                extract_images: false,
                start_from_char: 6,
                output_schema: None,
                already_collected: vec![],
            }))
            .await;

        let content = result.extracted_content.expect("extracted content");
        assert!(content.contains("<url>\nabout:blank\n</url>"));
        assert!(content.contains("<query>\ncompany\n</query>"));
        assert!(content.contains("<content_stats>"));
        assert!(content.contains("started from char 6"));
        assert!(content.contains("<webpage_content>"));
        assert!(content.contains("EvalOps Beta"));
        assert_eq!(executor.session().events(), vec!["page_text"]);
    }

    #[tokio::test]
    async fn browser_executor_extracts_links_images_schema_and_dedupe_hints() {
        let session = MockSession::new();
        let mut executor = BrowserActionExecutor::new(session);

        let result = executor
            .execute(&BrowserAction::Extract(ExtractAction {
                query: "product image URLs".to_owned(),
                extract_links: true,
                extract_images: false,
                start_from_char: 0,
                output_schema: Some(serde_json::json!({
                    "type": "object",
                    "properties": {
                        "products": { "type": "array" }
                    }
                })),
                already_collected: vec!["Existing product".to_owned()],
            }))
            .await;

        let content = result.extracted_content.expect("extracted content");
        assert!(content.contains("<output_schema>"));
        assert!(content.contains(r#""type": "object""#));
        assert!(content.contains("extract_links=true, extract_images=true"));
        assert!(content.contains("<links>\n- Run EvalOps: https://evalops.dev/run\n</links>"));
        assert!(content.contains("<images>\n- Hero shot: https://evalops.dev/hero.png\n</images>"));
        assert!(content.contains("<already_collected>"));
        assert!(content.contains("- Existing product"));
        let metadata = result.metadata.expect("extract metadata");
        assert_eq!(metadata["structured_extraction"], true);
        assert_eq!(metadata["source_url"], "about:blank");
        assert_eq!(metadata["schema_used"]["type"], "object");
        assert_eq!(metadata["is_partial"], false);
        assert_eq!(metadata["content_stats"]["method"], "page_text");
        assert_eq!(metadata["options"]["extract_links"], true);
        assert_eq!(metadata["options"]["extract_images"], true);
        assert_eq!(metadata["options"]["links_count"], 1);
        assert_eq!(metadata["options"]["images_count"], 1);
        assert_eq!(
            executor.session().events(),
            vec![
                "page_text",
                "find_elements:a[href]:href|title|aria-label|rel:200:true",
                "find_elements:img[src], img[data-src], picture source[srcset]:src|data-src|srcset|alt|title|aria-label:200:false"
            ]
        );
    }

    #[tokio::test]
    async fn browser_executor_rejects_extract_start_beyond_content() {
        let session = MockSession::new();
        let mut executor = BrowserActionExecutor::new(session);

        let result = executor
            .execute(&BrowserAction::Extract(ExtractAction {
                query: "company".to_owned(),
                extract_links: false,
                extract_images: false,
                start_from_char: 999,
                output_schema: None,
                already_collected: vec![],
            }))
            .await;

        assert!(
            result
                .error
                .as_deref()
                .expect("extract error")
                .contains("exceeds content length")
        );
        assert_eq!(executor.session().events(), vec!["page_text"]);
    }

    #[tokio::test]
    async fn browser_executor_searches_page_text() {
        let session = MockSession::new();
        let mut executor = BrowserActionExecutor::new(session);

        let result = executor
            .execute(&BrowserAction::SearchPage(SearchPageAction {
                pattern: "evalops".to_owned(),
                regex: false,
                case_sensitive: false,
                context_chars: 5,
                css_scope: None,
                max_results: 2,
            }))
            .await;

        let content = result.extracted_content.expect("search content");
        assert!(content.contains("Found 2 matches for \"evalops\" on page:"));
        assert!(content.contains("[1] ...lpha EvalOps Beta..."));
        assert!(content.contains("[2] ...cond EvalOps line"));
        assert!(content.contains("EvalOps"));
        assert_eq!(content.matches("EvalOps").count(), 2);
    }

    #[tokio::test]
    async fn browser_executor_finds_css_elements() {
        let session = MockSession::new();
        let mut executor = BrowserActionExecutor::new(session);

        let result = executor
            .execute(&BrowserAction::FindElements(FindElementsAction {
                selector: "button".to_owned(),
                attributes: Some(vec!["id".to_owned()]),
                max_results: 3,
                include_text: true,
            }))
            .await;

        let content = result.extracted_content.expect("find content");
        assert!(content.contains("Found 1 element matching \"button\":"));
        assert!(content.contains("[0] <button> \"Run EvalOps\" {id=\"run\"}"));
        assert!(content.contains("Run EvalOps"));
        assert_eq!(
            executor.session().events(),
            vec!["find_elements:button:id:3:true"]
        );
    }

    struct QueueModel {
        outputs: Mutex<VecDeque<Value>>,
        requests: Mutex<Vec<ChatRequest>>,
    }

    impl QueueModel {
        fn new(outputs: Vec<Value>) -> Self {
            Self {
                outputs: Mutex::new(outputs.into()),
                requests: Mutex::new(Vec::new()),
            }
        }
    }

    #[async_trait]
    impl ChatModel for QueueModel {
        fn provider(&self) -> &str {
            "test"
        }

        fn model(&self) -> &str {
            "static"
        }

        async fn invoke_json(
            &self,
            request: ChatRequest,
        ) -> Result<ChatCompletion<Value>, LlmError> {
            assert!(
                request
                    .messages
                    .iter()
                    .any(|message| message.role == MessageRole::User)
            );
            assert!(request.output_schema.is_some());
            self.requests.lock().expect("requests lock").push(request);
            let content = self
                .outputs
                .lock()
                .expect("outputs lock")
                .pop_front()
                .ok_or_else(|| LlmError::Provider("no queued model output".to_owned()))?;
            Ok(ChatCompletion {
                model: self.model().to_owned(),
                content,
                raw_response: None,
            })
        }
    }

    #[tokio::test]
    async fn agent_step_records_done_history() {
        let output = serde_json::json!({
            "current_state": {
                "thinking": "done"
            },
            "action": [
                {
                    "done": {
                        "text": "finished",
                        "success": true,
                        "files_to_display": []
                    }
                }
            ]
        });
        let mut agent = Agent::new(
            "finish the task",
            QueueModel::new(vec![output]),
            MockSession::new(),
        );

        let item = agent.step().await.expect("agent step");

        assert_eq!(item.result, vec![ActionResult::done("finished", true)]);
        let metadata = item.metadata.as_ref().expect("step metadata");
        assert_eq!(metadata.step_number, 1);
        assert_eq!(metadata.step_interval, None);
        assert!(metadata.step_end_time >= metadata.step_start_time);
        assert!(metadata.duration_seconds() >= 0.0);
        assert_eq!(agent.history().final_result(), Some("finished"));
    }

    struct SlowModel;

    #[async_trait]
    impl ChatModel for SlowModel {
        fn provider(&self) -> &str {
            "test"
        }

        fn model(&self) -> &str {
            "slow"
        }

        async fn invoke_json(
            &self,
            _request: ChatRequest,
        ) -> Result<ChatCompletion<Value>, LlmError> {
            std::future::pending::<()>().await;
            unreachable!("pending model should be cancelled by timeout")
        }
    }

    #[tokio::test]
    async fn agent_step_enforces_llm_timeout() {
        let settings = AgentSettings {
            llm_timeout_seconds: 0,
            ..AgentSettings::default()
        };
        let mut agent = Agent::with_settings("timeout", settings, SlowModel, MockSession::new());

        let error = agent.step().await.expect_err("LLM timeout");

        assert!(matches!(error, AgentRunError::LlmTimedOut { seconds: 0 }));
    }

    #[tokio::test]
    async fn agent_step_enforces_step_timeout() {
        let settings = AgentSettings {
            step_timeout_seconds: 0,
            ..AgentSettings::default()
        };
        let mut agent =
            Agent::with_settings("step timeout", settings, SlowModel, MockSession::new());

        let error = agent.step().await.expect_err("step timeout");

        assert!(matches!(error, AgentRunError::StepTimedOut { seconds: 0 }));
        assert!(agent.history().items.is_empty());
    }

    #[tokio::test]
    async fn agent_step_respects_use_vision_setting() {
        let output = serde_json::json!({
            "current_state": {},
            "action": [
                {
                    "done": {
                        "text": "complete",
                        "success": true,
                        "files_to_display": []
                    }
                }
            ]
        });
        let settings = AgentSettings {
            use_vision: false,
            ..AgentSettings::default()
        };
        let mut agent = Agent::with_settings(
            "finish without screenshot",
            settings,
            QueueModel::new(vec![output]),
            MockSession::new(),
        );

        agent.step().await.expect("agent step");

        let state_requests = agent.executor.session().state_screenshot_requests();
        assert_eq!(state_requests.first(), Some(&false));
        assert!(state_requests.iter().all(|include| !include));
    }

    #[tokio::test]
    async fn agent_step_rejects_too_many_actions_before_side_effects() {
        let output = serde_json::json!({
            "current_state": {},
            "action": [
                {
                    "click": {
                        "index": 1
                    }
                },
                {
                    "click": {
                        "index": 2
                    }
                }
            ]
        });
        let settings = AgentSettings {
            max_actions_per_step: 1,
            ..AgentSettings::default()
        };
        let mut agent = Agent::with_settings(
            "click only once",
            settings,
            QueueModel::new(vec![output]),
            MockSession::new(),
        );

        let item = agent.step().await.expect("agent step");

        assert_eq!(item.result.len(), 1);
        assert!(
            item.result[0]
                .error
                .as_deref()
                .expect("error")
                .contains("max_actions_per_step")
        );
        assert!(agent.executor.session().events().is_empty());
    }

    #[tokio::test]
    async fn agent_run_stops_on_done() {
        let output = serde_json::json!({
            "current_state": {},
            "action": [
                {
                    "done": {
                        "text": "complete",
                        "success": true,
                        "files_to_display": []
                    }
                }
            ]
        });
        let mut agent = Agent::new(
            "complete",
            QueueModel::new(vec![output]),
            MockSession::new(),
        );

        let history = agent.run(3).await.expect("agent run");

        assert_eq!(history.items.len(), 1);
        assert_eq!(history.final_result(), Some("complete"));
    }

    #[tokio::test]
    async fn agent_run_recovers_from_invalid_model_output() {
        let invalid_output = serde_json::json!({
            "not_agent_output": true
        });
        let done_output = serde_json::json!({
            "current_state": {},
            "action": [
                {
                    "done": {
                        "text": "recovered",
                        "success": true,
                        "files_to_display": []
                    }
                }
            ]
        });
        let settings = AgentSettings {
            max_failures: 2,
            ..AgentSettings::default()
        };
        let mut agent = Agent::with_settings(
            "recover",
            settings,
            QueueModel::new(vec![invalid_output, done_output]),
            MockSession::new(),
        );

        let history = agent.run(3).await.expect("agent run");

        assert_eq!(history.items.len(), 2);
        let first_metadata = history.items[0].metadata.as_ref().expect("first metadata");
        assert_eq!(first_metadata.step_number, 1);
        assert_eq!(first_metadata.step_interval, None);
        let second_metadata = history.items[1].metadata.as_ref().expect("second metadata");
        assert_eq!(second_metadata.step_number, 2);
        assert!(second_metadata.step_interval.unwrap_or(-1.0) >= 0.0);
        assert_eq!(history.final_result(), Some("recovered"));
        assert_eq!(history.is_successful(), Some(true));
        assert_eq!(history.errors().len(), 1);
        assert!(
            history.errors()[0].contains("invalid agent output"),
            "unexpected error: {}",
            history.errors()[0]
        );
    }

    #[tokio::test]
    async fn agent_run_enforces_max_failures() {
        let output = serde_json::json!({
            "current_state": {},
            "action": [
                {
                    "click": {
                        "coordinate_x": 10
                    }
                }
            ]
        });
        let settings = AgentSettings {
            max_failures: 2,
            ..AgentSettings::default()
        };
        let mut agent = Agent::with_settings(
            "find buttons",
            settings,
            QueueModel::new(vec![output.clone(), output]),
            MockSession::new(),
        );

        let error = agent.run(5).await.expect_err("max failures");

        assert!(matches!(
            error,
            AgentRunError::MaxFailuresExceeded { failures: 2 }
        ));
        assert_eq!(agent.history().items.len(), 2);
    }

    #[tokio::test]
    async fn agent_run_reports_step_limit() {
        let output = serde_json::json!({
            "current_state": {},
            "action": [
                {
                    "click": {
                        "index": 1
                    }
                }
            ]
        });
        let mut agent = Agent::new(
            "click once",
            QueueModel::new(vec![output]),
            MockSession::new(),
        );

        let error = agent.run(1).await.expect_err("step limit");

        assert!(matches!(
            error,
            AgentRunError::StepLimitReached { max_steps: 1 }
        ));
        assert_eq!(agent.history().items.len(), 1);
    }

    #[tokio::test]
    async fn agent_run_detects_repeated_action_loop() {
        let output = serde_json::json!({
            "current_state": {},
            "action": [
                {
                    "click": {
                        "index": 1
                    }
                }
            ]
        });
        let settings = AgentSettings {
            loop_detection_window: 2,
            ..AgentSettings::default()
        };
        let mut agent = Agent::with_settings(
            "do not loop",
            settings,
            QueueModel::new(vec![output.clone(), output]),
            MockSession::new(),
        );

        let error = agent.run(5).await.expect_err("loop detected");

        assert!(matches!(error, AgentRunError::LoopDetected { window: 2 }));
        assert_eq!(agent.history().items.len(), 2);
    }

    #[tokio::test]
    async fn agent_loop_detection_can_be_disabled() {
        let output = serde_json::json!({
            "current_state": {},
            "action": [
                {
                    "click": {
                        "index": 1
                    }
                }
            ]
        });
        let settings = AgentSettings {
            loop_detection_window: 2,
            loop_detection_enabled: false,
            ..AgentSettings::default()
        };
        let mut agent = Agent::with_settings(
            "loop if allowed",
            settings,
            QueueModel::new(vec![output.clone(), output]),
            MockSession::new(),
        );

        let error = agent.run(2).await.expect_err("step limit");

        assert!(matches!(
            error,
            AgentRunError::StepLimitReached { max_steps: 2 }
        ));
        assert_eq!(agent.history().items.len(), 2);
    }

    #[test]
    fn step_request_includes_previous_results() {
        let mut state = blank_state();
        state.dom_state = SerializedDomState::from_elements(vec![
            browser_use_dom::DomElementRef {
                index: 1,
                target_id: "target".to_owned(),
                backend_node_id: 1,
                node_id: None,
                tag_name: "a".to_owned(),
                role: Some("link".to_owned()),
                name: Some("Docs".to_owned()),
                text: Some("Docs".to_owned()),
                attributes: BTreeMap::from([("href".to_owned(), "/docs".to_owned())]),
                bounds: None,
                is_visible: true,
                is_interactive: true,
                is_scrollable: false,
            },
            browser_use_dom::DomElementRef {
                index: 2,
                target_id: "target".to_owned(),
                backend_node_id: 2,
                node_id: None,
                tag_name: "div".to_owned(),
                role: None,
                name: Some("Results".to_owned()),
                text: None,
                attributes: BTreeMap::new(),
                bounds: None,
                is_visible: true,
                is_interactive: true,
                is_scrollable: true,
            },
        ]);
        state.tabs = vec![browser_use_dom::TabInfo {
            url: "https://example.com/docs".to_owned(),
            title: "Docs".to_owned(),
            tab_id: browser_use_dom::TabInfo::tab_id_for_target("target-1234abcd"),
            target_id: "target-1234abcd".to_owned(),
            parent_target_id: None,
        }];
        let history = AgentHistory {
            items: vec![AgentHistoryItem {
                model_output: None,
                result: vec![ActionResult::extracted("Clicked element 1")],
                state: blank_state(),
                metadata: None,
            }],
        };

        let request = build_step_request("keep going", &state, &history, &AgentSettings::default())
            .expect("step request");
        let request_text = serde_json::to_string(&request.messages).expect("messages json");
        let user_message = request
            .messages
            .iter()
            .find(|message| message.role == MessageRole::User)
            .expect("user message");
        let user_text = match &user_message.content[0] {
            ContentPart::Text { text } => text,
            other => panic!("unexpected first content part: {other:?}"),
        };

        assert!(request_text.contains("Previous action results"));
        assert!(request_text.contains("Clicked element 1"));
        assert!(request_text.contains("Page stats"));
        assert!(request_text.contains("1 links, 2 interactive"));
        assert!(request_text.contains("1 scroll containers"));
        assert!(user_text.contains(r#""tab_id": "abcd""#));
        assert!(request_text.contains("Avoid repeating the same action sequence"));
    }

    #[test]
    fn step_request_includes_loop_awareness_nudge() {
        let mut state = blank_state();
        state.dom_state.text = "[1] <button> Refresh".to_owned();
        let history = AgentHistory {
            items: (0..5)
                .map(|_| AgentHistoryItem {
                    model_output: Some(AgentOutput {
                        current_state: AgentCurrentState {
                            thinking: None,
                            evaluation_previous_goal: None,
                            memory: None,
                            next_goal: None,
                        },
                        action: vec![BrowserAction::Click(ClickElementAction {
                            index: Some(1),
                            coordinate_x: None,
                            coordinate_y: None,
                        })],
                    }),
                    result: vec![ActionResult::extracted("Clicked element 1")],
                    state: state.clone(),
                    metadata: None,
                })
                .collect(),
        };

        let request = build_step_request("unstick", &state, &history, &AgentSettings::default())
            .expect("step request");
        let request_text = serde_json::to_string(&request.messages).expect("messages json");

        assert!(request_text.contains("Loop awareness"));
        assert!(request_text.contains("repeated a similar action 5 times"));
        assert!(request_text.contains("page content has not changed across 5"));

        let disabled_settings = AgentSettings {
            loop_detection_enabled: false,
            ..AgentSettings::default()
        };
        let request = build_step_request("unstick", &state, &history, &disabled_settings)
            .expect("step request");
        let request_text = serde_json::to_string(&request.messages).expect("messages json");
        assert!(!request_text.contains("Loop awareness"));
    }

    #[test]
    fn step_request_attaches_screenshot_as_image_part() {
        let mut state = blank_state();
        state.screenshot = Some("abc123".to_owned());

        let request = build_step_request(
            "inspect screenshot",
            &state,
            &AgentHistory::default(),
            &AgentSettings::default(),
        )
        .expect("step request");
        let user_message = request
            .messages
            .iter()
            .find(|message| message.role == MessageRole::User)
            .expect("user message");

        assert!(matches!(
            &user_message.content[1],
            ContentPart::ImageUrl { image_url } if image_url == "data:image/png;base64,abc123"
        ));
        let text = match &user_message.content[0] {
            ContentPart::Text { text } => text,
            other => panic!("unexpected first content part: {other:?}"),
        };
        assert!(text.contains("Browser state"));
        assert!(!text.contains("abc123"));
    }

    #[test]
    fn step_request_uses_custom_dom_include_attributes() {
        let mut state = blank_state();
        state.dom_state = SerializedDomState::from_elements(vec![browser_use_dom::DomElementRef {
            index: 1,
            target_id: "target".to_owned(),
            backend_node_id: 1,
            node_id: None,
            tag_name: "button".to_owned(),
            role: None,
            name: Some("Run".to_owned()),
            text: None,
            attributes: BTreeMap::from([
                ("data-testid".to_owned(), "run-action".to_owned()),
                ("id".to_owned(), "run".to_owned()),
            ]),
            bounds: None,
            is_visible: true,
            is_interactive: true,
            is_scrollable: false,
        }]);
        let settings = AgentSettings {
            include_attributes: vec!["data-testid".to_owned()],
            ..AgentSettings::default()
        };

        let request = build_step_request("click run", &state, &AgentHistory::default(), &settings)
            .expect("step request");
        let user_message = request
            .messages
            .iter()
            .find(|message| message.role == MessageRole::User)
            .expect("user message");
        let text = match &user_message.content[0] {
            ContentPart::Text { text } => text,
            other => panic!("unexpected first content part: {other:?}"),
        };

        assert!(text.contains(r#""text": "[1] <button data-testid=run-action> Run""#));
        assert!(!text.contains(r#""text": "[1] <button id=run> Run""#));
    }

    #[test]
    fn step_request_omits_screenshot_when_vision_disabled() {
        let mut state = blank_state();
        state.screenshot = Some("abc123".to_owned());
        let settings = AgentSettings {
            use_vision: false,
            ..AgentSettings::default()
        };

        let request =
            build_step_request("no screenshot", &state, &AgentHistory::default(), &settings)
                .expect("step request");
        let user_message = request
            .messages
            .iter()
            .find(|message| message.role == MessageRole::User)
            .expect("user message");

        assert_eq!(user_message.content.len(), 1);
        let request_text = serde_json::to_string(user_message).expect("message json");
        assert!(!request_text.contains("abc123"));
    }

    #[test]
    fn previous_results_only_include_one_time_extractions_once() {
        let history = AgentHistory {
            items: vec![
                AgentHistoryItem {
                    model_output: None,
                    result: vec![ActionResult {
                        extracted_content: Some("very large extracted payload".to_owned()),
                        error: None,
                        long_term_memory: Some("Large extraction saved to file".to_owned()),
                        include_extracted_content_only_once: true,
                        include_in_memory: true,
                        is_done: false,
                        success: None,
                        metadata: None,
                    }],
                    state: blank_state(),
                    metadata: None,
                },
                AgentHistoryItem {
                    model_output: None,
                    result: vec![ActionResult::extracted("Clicked element 1")],
                    state: blank_state(),
                    metadata: None,
                },
            ],
        };

        let rendered = render_previous_results(&history, None);

        assert!(!rendered.contains("very large extracted payload"));
        assert!(rendered.contains("Large extraction saved to file"));
        assert!(rendered.contains("Clicked element 1"));
    }

    #[test]
    fn previous_results_include_latest_one_time_extraction() {
        let history = AgentHistory {
            items: vec![AgentHistoryItem {
                model_output: None,
                result: vec![ActionResult {
                    extracted_content: Some("fresh extracted payload".to_owned()),
                    error: None,
                    long_term_memory: Some("Extraction saved to file".to_owned()),
                    include_extracted_content_only_once: true,
                    include_in_memory: true,
                    is_done: false,
                    success: None,
                    metadata: None,
                }],
                state: blank_state(),
                metadata: None,
            }],
        };

        let rendered = render_previous_results(&history, None);

        assert!(rendered.contains("fresh extracted payload"));
        assert!(!rendered.contains("Extraction saved to file"));
    }

    #[test]
    fn previous_results_can_limit_history_items_from_settings() {
        let history = AgentHistory {
            items: ["first", "second", "third"]
                .into_iter()
                .map(|text| AgentHistoryItem {
                    model_output: None,
                    result: vec![ActionResult::extracted(text)],
                    state: blank_state(),
                    metadata: None,
                })
                .collect(),
        };

        let rendered = render_previous_results(&history, Some(2));

        assert!(!rendered.contains("first"));
        assert!(rendered.contains("second"));
        assert!(rendered.contains("third"));
    }

    #[test]
    fn search_url_matches_browser_use_engines() {
        assert_eq!(
            search_url(&SearchEngine::Google, "browser use rust"),
            "https://www.google.com/search?q=browser+use+rust&udm=14"
        );
        assert_eq!(
            search_url(&SearchEngine::DuckDuckGo, "browser use rust"),
            "https://duckduckgo.com/?q=browser+use+rust"
        );
    }
}
