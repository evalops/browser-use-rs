//! Core agent contracts for browser-use-rs.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::time::Duration;
use thiserror::Error;
use tokio::time::timeout;
use uuid::Uuid;

use async_trait::async_trait;
use base64::Engine;
use browser_use_cdp::{BrowserError, BrowserSession};
use url::form_urlencoded;

pub use browser_use_dom::BrowserStateSummary;
pub use browser_use_llm::{
    AnthropicChatModel, ChatCompletion, ChatMessage, ChatModel, ChatRequest, GeminiChatModel,
    LlmError, MessageRole, OpenAiCompatibleChatModel,
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
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct AgentHistoryItem {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_output: Option<AgentOutput>,
    pub result: Vec<ActionResult>,
    pub state: BrowserStateSummary,
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

            let before = match self.session.state(false).await {
                Ok(state) => state,
                Err(error) => {
                    results.push(ActionResult::error(error.to_string()));
                    break;
                }
            };
            let result = self.execute(action).await;
            let should_stop =
                result.is_done || result.error.is_some() || action.terminates_sequence();
            let page_changed = if should_stop {
                false
            } else {
                match self.session.state(false).await {
                    Ok(after) => after.url != before.url,
                    Err(error) => {
                        results.push(result);
                        results.push(ActionResult::error(error.to_string()));
                        break;
                    }
                }
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
                (Some(index), _, _) => {
                    session.click(index).await?;
                    Ok(ActionResult::extracted(format!("Clicked element {index}")))
                }
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
            session
                .scroll(params.index, params.down, params.pages)
                .await?;
            Ok(ActionResult::extracted("Scrolled page"))
        }
        BrowserAction::Screenshot(_) => {
            let screenshot = session.screenshot().await?;
            Ok(ActionResult {
                extracted_content: Some(format!(
                    "Captured screenshot ({} base64 chars)",
                    screenshot.base64_png.len()
                )),
                error: None,
                long_term_memory: None,
                include_extracted_content_only_once: true,
                include_in_memory: false,
                is_done: false,
                success: None,
            })
        }
        BrowserAction::Done(params) => Ok(ActionResult::done(params.text.clone(), params.success)),
        BrowserAction::Extract(params) => {
            let text = session.page_text().await?;
            let extracted: String = text
                .chars()
                .skip(params.start_from_char)
                .take(4_000)
                .collect();
            Ok(ActionResult {
                extracted_content: Some(format!(
                    "Extracted page content for '{}':\n{}",
                    params.query, extracted
                )),
                error: None,
                long_term_memory: Some(extracted),
                include_extracted_content_only_once: true,
                include_in_memory: true,
                is_done: false,
                success: None,
            })
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

            if matches.is_empty() {
                Ok(ActionResult::extracted(format!(
                    "No page matches for '{}'",
                    params.pattern
                )))
            } else {
                Ok(ActionResult::extracted(format!(
                    "Page matches for '{}':\n{}",
                    params.pattern,
                    matches.join("\n")
                )))
            }
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
            let rendered = serde_json::to_string_pretty(&elements)
                .map_err(|error| BrowserError::ActionFailed(error.to_string()))?;
            Ok(ActionResult::extracted(format!(
                "Found {} elements for selector '{}':\n{}",
                elements.len(),
                params.selector,
                rendered
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

            if let Some(file_name) = &params.file_name {
                let bytes = base64::engine::general_purpose::STANDARD
                    .decode(&pdf.base64_pdf)
                    .map_err(|error| BrowserError::ActionFailed(error.to_string()))?;
                std::fs::write(file_name, bytes)
                    .map_err(|error| BrowserError::ActionFailed(error.to_string()))?;
                Ok(ActionResult {
                    extracted_content: Some(format!("Saved PDF to {file_name}")),
                    error: None,
                    long_term_memory: Some(format!("Saved PDF to {file_name}")),
                    include_extracted_content_only_once: true,
                    include_in_memory: true,
                    is_done: false,
                    success: None,
                })
            } else {
                Ok(ActionResult {
                    extracted_content: Some(format!(
                        "Captured PDF ({} base64 chars)",
                        pdf.base64_pdf.len()
                    )),
                    error: None,
                    long_term_memory: None,
                    include_extracted_content_only_once: true,
                    include_in_memory: false,
                    is_done: false,
                    success: None,
                })
            }
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

#[must_use]
pub fn search_url(engine: &SearchEngine, query: &str) -> String {
    let encoded: String = form_urlencoded::byte_serialize(query.as_bytes()).collect();
    match engine {
        SearchEngine::DuckDuckGo => format!("https://duckduckgo.com/?q={encoded}"),
        SearchEngine::Google => format!("https://www.google.com/search?q={encoded}&udm=14"),
        SearchEngine::Bing => format!("https://www.bing.com/search?q={encoded}"),
    }
}

fn search_text_matches(
    text: &str,
    pattern: &str,
    regex: bool,
    case_sensitive: bool,
    context_chars: usize,
    max_results: usize,
) -> Result<Vec<String>, String> {
    if pattern.is_empty() || max_results == 0 {
        return Ok(Vec::new());
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

    Ok(matcher
        .find_iter(text)
        .take(max_results)
        .map(|hit| context_snippet(text, hit.start(), hit.end(), context_chars))
        .collect())
}

fn context_snippet(text: &str, start: usize, end: usize, context_chars: usize) -> String {
    let prefix: String = text[..start]
        .chars()
        .rev()
        .take(context_chars)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    let suffix: String = text[end..].chars().take(context_chars).collect();
    format!("{prefix}{}{suffix}", &text[start..end])
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
        let state = self.executor.session().state(true).await?;
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
        self.record_model_output(state, model_output).await
    }

    async fn step_recovering_model_errors(&mut self) -> Result<&AgentHistoryItem, AgentRunError> {
        let state = self.executor.session().state(true).await?;
        let request = build_step_request(&self.task, &state, &self.history, &self.settings)?;
        let completion = match timeout(
            Duration::from_secs(self.settings.llm_timeout_seconds),
            self.llm.invoke_json(request),
        )
        .await
        {
            Ok(Ok(completion)) => completion,
            Ok(Err(error)) => {
                return self.record_model_error(state, format!("LLM provider error: {error}"));
            }
            Err(_) => {
                return self.record_model_error(
                    state,
                    format!(
                        "LLM call timed out after {} seconds",
                        self.settings.llm_timeout_seconds
                    ),
                );
            }
        };
        let model_output: AgentOutput = match serde_json::from_value(completion.content) {
            Ok(model_output) => model_output,
            Err(error) => {
                return self.record_model_error(state, format!("invalid agent output: {error}"));
            }
        };
        self.record_model_output(state, model_output).await
    }

    async fn record_model_output(
        &mut self,
        state: BrowserStateSummary,
        model_output: AgentOutput,
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

        self.history.items.push(AgentHistoryItem {
            model_output: Some(model_output),
            result,
            state,
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
    ) -> Result<&AgentHistoryItem, AgentRunError> {
        self.history.items.push(AgentHistoryItem {
            model_output: None,
            result: vec![ActionResult::error(error)],
            state,
        });

        self.history
            .items
            .last()
            .ok_or_else(|| AgentRunError::InvalidOutput("history item was not recorded".to_owned()))
    }
}

pub fn build_step_request(
    task: &str,
    state: &BrowserStateSummary,
    history: &AgentHistory,
    settings: &AgentSettings,
) -> Result<ChatRequest, AgentRunError> {
    let state_json = serde_json::to_string_pretty(state)
        .map_err(|error| AgentRunError::InvalidOutput(error.to_string()))?;
    let previous_results = render_previous_results(history);
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
            ChatMessage::text(
                MessageRole::User,
                format!(
                    "Task:\n{task}\n\nPrevious action results:\n{previous_results}\n\nBrowser state:\n{state_json}"
                ),
            ),
        ],
        output_schema: Some(schema_for_agent_output()),
    })
}

fn schema_for_agent_output() -> Value {
    serde_json::to_value(schemars::schema_for!(AgentOutput)).unwrap_or(Value::Null)
}

fn render_previous_results(history: &AgentHistory) -> String {
    let rendered: Vec<String> = history
        .items
        .iter()
        .rev()
        .take(5)
        .rev()
        .enumerate()
        .flat_map(|(step_index, item)| {
            item.result
                .iter()
                .enumerate()
                .map(move |(result_index, result)| {
                    let text = result
                        .error
                        .as_deref()
                        .or(result.extracted_content.as_deref())
                        .unwrap_or("no content");
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
        ClickElementAction, CloseTabAction, DoneAction, ExtractAction, FindElementsAction,
        GetDropdownOptionsAction, InputTextAction, NavigateAction, SaveAsPdfAction,
        SearchPageAction, SelectDropdownOptionAction, SendKeysAction, SwitchTabAction,
        UploadFileAction,
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
    }

    #[test]
    fn history_returns_latest_done_result() {
        let history = AgentHistory {
            items: vec![AgentHistoryItem {
                model_output: None,
                result: vec![ActionResult::done("finished", true)],
                state: blank_state(),
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
                },
                AgentHistoryItem {
                    model_output: None,
                    result: vec![ActionResult::done("could not finish", false)],
                    state: blank_state(),
                },
            ],
        };

        assert_eq!(history.final_result(), Some("could not finish"));
        assert!(history.is_done());
        assert_eq!(history.is_successful(), Some(false));
        assert_eq!(history.errors(), vec!["first failure"]);
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
    }

    impl MockSession {
        fn new() -> Self {
            Self {
                events: Mutex::new(Vec::new()),
                states: Mutex::new(VecDeque::new()),
            }
        }

        fn with_states(states: Vec<BrowserStateSummary>) -> Self {
            Self {
                events: Mutex::new(Vec::new()),
                states: Mutex::new(VecDeque::from(states)),
            }
        }

        fn events(&self) -> Vec<String> {
            self.events.lock().expect("events lock").clone()
        }
    }

    #[async_trait]
    impl BrowserSession for MockSession {
        async fn state(
            &self,
            _include_screenshot: bool,
        ) -> Result<BrowserStateSummary, BrowserError> {
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
                base64_png: "abc".to_owned(),
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
        let output_path = temp_dir.path().join("out.pdf");
        let output = output_path.display().to_string();
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
                .contains(&output)
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
        assert!(content.contains("EvalOps Beta"));
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
        let history = AgentHistory {
            items: vec![AgentHistoryItem {
                model_output: None,
                result: vec![ActionResult::extracted("Clicked element 1")],
                state: blank_state(),
            }],
        };

        let request = build_step_request(
            "keep going",
            &blank_state(),
            &history,
            &AgentSettings::default(),
        )
        .expect("step request");
        let request_text = serde_json::to_string(&request.messages).expect("messages json");

        assert!(request_text.contains("Previous action results"));
        assert!(request_text.contains("Clicked element 1"));
        assert!(request_text.contains("Avoid repeating the same action sequence"));
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
