//! Browser action execution and history replay execution.
//!
//! The agent produces [`BrowserAction`] values; this module turns those action
//! structs into calls on a [`BrowserSession`]. It also owns action-batch stop
//! rules, managed-file actions, per-action timeout handling, and replay
//! execution against remapped history plans.
//!
//! ```mermaid
//! flowchart LR
//!     Action["BrowserAction"] --> Executor["BrowserActionExecutor"]
//!     Executor --> Session["BrowserSession methods"]
//!     Executor --> Files["ManagedFileSystem"]
//!     Session --> Result["ActionResult"]
//!     Files --> Result
//!     Result --> History["AgentHistoryItem.result"]
//! ```

mod extract;
mod files;
mod page_results;

use crate::{
    ActionResult, AgentHistory, AgentHistoryReplayExecution, AgentHistoryReplayExecutionItem,
    AgentHistoryReplayPlan, AgentHistoryReplayPlanError, AgentHistoryReplayPlanItem,
    AgentHistoryReplayRun, AgentHistoryReplayRunError, AgentHistoryReplayStop,
    AgentHistoryReplayStopReason, ManagedFileSystem, action_timeout_duration,
    coerce_valid_action_timeout_seconds, default_action_timeout_seconds, historical_replay_actions,
    rematch_action_for_replay, search_url, timed_out_action_result,
};
use async_trait::async_trait;
use base64::Engine;
use browser_use_cdp::{BrowserError, BrowserSession};
use browser_use_dom::BrowserStateSummary;
use browser_use_tools::BrowserAction;
use extract::{
    MAX_EXTRACT_RELATED_ELEMENTS, extract_image_attributes, extract_link_attributes,
    should_extract_images,
};
pub(crate) use extract::{
    build_extract_llm_request, complete_llm_extract_result, extract_action_result,
};
use files::{done_action_result, upload_file_action_path};
pub(crate) use files::{next_available_pdf_path, pdf_output_path, screenshot_output_path};
pub(crate) use page_results::truncate_chars;
use page_results::{
    default_find_element_attributes, format_find_elements_results, format_search_page_results,
    search_text_matches, truncate_evaluate_result,
};
use std::collections::BTreeSet;
use std::time::Duration;
use tokio::time::{sleep, timeout};

#[async_trait]
/// Minimal async interface for something that can execute one browser action.
pub trait ActionExecutor {
    /// Executes one action and returns a browser-use action result.
    async fn execute(&mut self, action: &BrowserAction) -> ActionResult;
}

/// Default action executor backed by a [`BrowserSession`] and managed files.
pub struct BrowserActionExecutor<S> {
    session: S,
    file_system: ManagedFileSystem,
    display_files_in_done_text: bool,
    action_timeout_seconds: f64,
    enforce_upload_file_availability: bool,
    available_file_paths: BTreeSet<String>,
}

impl<S> BrowserActionExecutor<S> {
    /// Creates an executor with a temporary managed file system.
    #[must_use]
    pub fn new(session: S) -> Self {
        Self::with_file_system(
            session,
            ManagedFileSystem::new_in_temp().expect("create managed file system"),
        )
    }

    /// Creates an executor with an explicit managed file system.
    #[must_use]
    pub fn with_file_system(session: S, file_system: ManagedFileSystem) -> Self {
        Self {
            session,
            file_system,
            display_files_in_done_text: true,
            action_timeout_seconds: default_action_timeout_seconds(),
            enforce_upload_file_availability: false,
            available_file_paths: BTreeSet::new(),
        }
    }

    /// Returns the underlying browser session.
    #[must_use]
    pub fn session(&self) -> &S {
        &self.session
    }

    /// Returns the managed file system.
    #[must_use]
    pub fn file_system(&self) -> &ManagedFileSystem {
        &self.file_system
    }

    /// Returns mutable access to the managed file system.
    pub fn file_system_mut(&mut self) -> &mut ManagedFileSystem {
        &mut self.file_system
    }

    /// Controls whether `done` includes requested managed file text.
    pub fn set_display_files_in_done_text(&mut self, display_files_in_done_text: bool) {
        self.display_files_in_done_text = display_files_in_done_text;
    }

    /// Sets the per-action timeout, coercing invalid values to the default.
    pub fn set_action_timeout_seconds(&mut self, action_timeout_seconds: f64) {
        self.action_timeout_seconds = coerce_valid_action_timeout_seconds(action_timeout_seconds);
    }

    /// Returns the effective per-action timeout.
    #[must_use]
    pub fn action_timeout_seconds(&self) -> f64 {
        coerce_valid_action_timeout_seconds(self.action_timeout_seconds)
    }

    /// Sets upload-file allow-list enforcement.
    pub fn set_upload_file_availability(
        &mut self,
        enforce_upload_file_availability: bool,
        available_file_paths: Vec<String>,
    ) {
        self.enforce_upload_file_availability = enforce_upload_file_availability;
        self.available_file_paths = available_file_paths.into_iter().collect();
    }
}

impl<S> BrowserActionExecutor<S>
where
    S: BrowserSession + Send + Sync,
{
    pub(crate) async fn execute_for_agent(&mut self, action: &BrowserAction) -> ActionResult {
        let timeout_seconds = self.action_timeout_seconds();
        match timeout(
            action_timeout_duration(timeout_seconds),
            execute_browser_action(
                &self.session,
                &mut self.file_system,
                action,
                self.display_files_in_done_text,
                self.enforce_upload_file_availability,
                &self.available_file_paths,
                false,
            ),
        )
        .await
        {
            Ok(Ok(result)) => result,
            Ok(Err(error)) => ActionResult::error(error.to_string()),
            Err(_) => timed_out_action_result(action, timeout_seconds),
        }
    }

    /// Executes a model-provided action batch using browser-use stop rules.
    ///
    /// Batches stop after errors, terminal actions, navigation-like actions, a
    /// `done` action after any prior action, or when a non-terminating action
    /// unexpectedly changes the page URL.
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

    /// Executes a precomputed history replay plan.
    pub async fn execute_replay_plan(
        &mut self,
        plan: &AgentHistoryReplayPlan,
    ) -> AgentHistoryReplayExecution {
        let mut items = Vec::new();
        let mut stop = None;

        for (plan_index, item) in plan.actions.iter().enumerate() {
            let action = &item.remapped_action;
            if plan_index > 0 && matches!(action, BrowserAction::Done(_)) {
                stop = Some(AgentHistoryReplayStop {
                    step_index: item.step_index,
                    action_index: item.action_index,
                    reason: AgentHistoryReplayStopReason::DoneAfterPriorAction,
                    diagnostic: None,
                });
                break;
            }

            let needs_page_change_guard = !action.terminates_sequence();
            let before = if needs_page_change_guard {
                match self.session.state(false).await {
                    Ok(state) => Some(state),
                    Err(error) => {
                        let result = ActionResult::error(error.to_string());
                        stop = Some(AgentHistoryReplayStop {
                            step_index: item.step_index,
                            action_index: item.action_index,
                            reason: AgentHistoryReplayStopReason::Error,
                            diagnostic: result.error.clone(),
                        });
                        items.push(AgentHistoryReplayExecutionItem {
                            step_index: item.step_index,
                            action_index: item.action_index,
                            original_action: item.original_action.clone(),
                            executed_action: action.clone(),
                            rematch: item.rematch.clone(),
                            result,
                        });
                        break;
                    }
                }
            } else {
                None
            };

            let result = self.execute(action).await;
            let mut stop_reason = replay_stop_reason(action, &result);
            let mut stop_diagnostic = result.error.clone();
            let mut page_changed = false;
            if stop_reason.is_none() {
                if let Some(before) = before {
                    match self.session.state(false).await {
                        Ok(after) => page_changed = after.url != before.url,
                        Err(error) => {
                            stop_reason = Some(AgentHistoryReplayStopReason::Error);
                            stop_diagnostic = Some(error.to_string());
                        }
                    }
                }
            }

            items.push(AgentHistoryReplayExecutionItem {
                step_index: item.step_index,
                action_index: item.action_index,
                original_action: item.original_action.clone(),
                executed_action: action.clone(),
                rematch: item.rematch.clone(),
                result,
            });

            if let Some(reason) = stop_reason {
                stop = Some(AgentHistoryReplayStop {
                    step_index: item.step_index,
                    action_index: item.action_index,
                    reason,
                    diagnostic: stop_diagnostic,
                });
                break;
            }

            if page_changed {
                stop = Some(AgentHistoryReplayStop {
                    step_index: item.step_index,
                    action_index: item.action_index,
                    reason: AgentHistoryReplayStopReason::PageChanged,
                    diagnostic: None,
                });
                break;
            }
        }

        AgentHistoryReplayExecution { items, stop }
    }

    /// Captures current browser state, builds a replay plan, and executes it.
    pub async fn replay_history(
        &mut self,
        history: &AgentHistory,
    ) -> Result<AgentHistoryReplayRun, AgentHistoryReplayRunError> {
        let current_state = self.session.state(false).await.map_err(|error| {
            AgentHistoryReplayRunError::CurrentState {
                message: error.to_string(),
            }
        })?;
        let (plan, execution) = self
            .execute_history_replay_with_recapture(history, current_state.clone())
            .await?;
        Ok(AgentHistoryReplayRun {
            current_state,
            plan,
            execution,
        })
    }

    async fn execute_history_replay_with_recapture(
        &mut self,
        history: &AgentHistory,
        mut latest_state: BrowserStateSummary,
    ) -> Result<(AgentHistoryReplayPlan, AgentHistoryReplayExecution), AgentHistoryReplayRunError>
    {
        let mut plan_items = Vec::new();
        let mut execution_items = Vec::new();
        let mut stop = None;

        for historical in historical_replay_actions(history) {
            if !plan_items.is_empty() && matches!(historical.action, BrowserAction::Done(_)) {
                stop = Some(AgentHistoryReplayStop {
                    step_index: historical.step_index,
                    action_index: historical.action_index,
                    reason: AgentHistoryReplayStopReason::DoneAfterPriorAction,
                    diagnostic: None,
                });
                break;
            }

            let rematch = rematch_action_for_replay(
                &historical.action,
                historical.interacted_element.as_ref(),
                &latest_state.dom_state,
            )
            .map_err(|failure| AgentHistoryReplayRunError::Plan {
                error: Box::new(AgentHistoryReplayPlanError {
                    step_index: historical.step_index,
                    action_index: historical.action_index,
                    original_action: Box::new(historical.action.clone()),
                    original_index: historical.action.interacted_element_index(),
                    failure: Box::new(failure),
                }),
            })?;
            let action = rematch.action.clone();
            let plan_item = AgentHistoryReplayPlanItem {
                step_index: historical.step_index,
                action_index: historical.action_index,
                original_action: historical.action.clone(),
                remapped_action: action.clone(),
                rematch,
            };
            plan_items.push(plan_item.clone());

            let needs_recapture = !action.terminates_sequence();
            let result = self.execute(&action).await;
            let mut stop_reason = replay_stop_reason(&action, &result);
            let mut stop_diagnostic = result.error.clone();
            let mut recaptured_state = None;
            if stop_reason.is_none() && needs_recapture {
                match self.session.state(false).await {
                    Ok(state) => recaptured_state = Some(state),
                    Err(error) => {
                        stop_reason = Some(AgentHistoryReplayStopReason::Error);
                        stop_diagnostic = Some(error.to_string());
                    }
                }
            }

            execution_items.push(AgentHistoryReplayExecutionItem {
                step_index: historical.step_index,
                action_index: historical.action_index,
                original_action: historical.action,
                executed_action: action,
                rematch: plan_item.rematch,
                result,
            });

            if let Some(reason) = stop_reason {
                stop = Some(AgentHistoryReplayStop {
                    step_index: historical.step_index,
                    action_index: historical.action_index,
                    reason,
                    diagnostic: stop_diagnostic,
                });
                break;
            }

            if let Some(state) = recaptured_state {
                latest_state = state;
            }
        }

        Ok((
            AgentHistoryReplayPlan {
                actions: plan_items,
            },
            AgentHistoryReplayExecution {
                items: execution_items,
                stop,
            },
        ))
    }
}

#[async_trait]
impl<S> ActionExecutor for BrowserActionExecutor<S>
where
    S: BrowserSession + Send + Sync,
{
    async fn execute(&mut self, action: &BrowserAction) -> ActionResult {
        let timeout_seconds = self.action_timeout_seconds();
        match timeout(
            action_timeout_duration(timeout_seconds),
            execute_browser_action(
                &self.session,
                &mut self.file_system,
                action,
                self.display_files_in_done_text,
                self.enforce_upload_file_availability,
                &self.available_file_paths,
                true,
            ),
        )
        .await
        {
            Ok(Ok(result)) => result,
            Ok(Err(error)) => ActionResult::error(error.to_string()),
            Err(_) => timed_out_action_result(action, timeout_seconds),
        }
    }
}

async fn execute_browser_action<S>(
    session: &S,
    file_system: &mut ManagedFileSystem,
    action: &BrowserAction,
    display_files_in_done_text: bool,
    enforce_upload_file_availability: bool,
    available_file_paths: &BTreeSet<String>,
    save_extract_envelope_to_file_system: bool,
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
                    judgement: None,
                    long_term_memory: Some(format!(
                        "Tried scrolling to text '{}' but it was not found",
                        params.text
                    )),
                    include_extracted_content_only_once: false,
                    include_in_memory: false,
                    is_done: false,
                    success: None,
                    attachments: Vec::new(),
                    images: Vec::new(),
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
                    judgement: None,
                    long_term_memory: Some(long_term_memory),
                    include_in_memory: true,
                    is_done: false,
                    success: None,
                    attachments: Vec::new(),
                    images: Vec::new(),
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
                    judgement: None,
                    long_term_memory: Some(format!("Screenshot saved to {file_name}")),
                    include_extracted_content_only_once: true,
                    include_in_memory: true,
                    is_done: false,
                    success: None,
                    attachments: vec![file_name],
                    images: Vec::new(),
                    metadata: None,
                })
            } else {
                Ok(ActionResult {
                    extracted_content: Some("Requested screenshot for next observation".to_owned()),
                    error: None,
                    judgement: None,
                    long_term_memory: None,
                    include_extracted_content_only_once: false,
                    include_in_memory: false,
                    is_done: false,
                    success: None,
                    attachments: Vec::new(),
                    images: Vec::new(),
                    metadata: Some(serde_json::json!({ "include_screenshot": true })),
                })
            }
        }
        BrowserAction::Done(params) => Ok(done_action_result(
            params,
            Some(file_system),
            display_files_in_done_text,
        )),
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
                save_extract_envelope_to_file_system.then_some(file_system),
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
                judgement: None,
                long_term_memory: Some(format!("Saved PDF to {file_name}")),
                include_extracted_content_only_once: true,
                include_in_memory: true,
                is_done: false,
                success: None,
                attachments: vec![file_name],
                images: Vec::new(),
                metadata: None,
            })
        }
        BrowserAction::UploadFile(params) => {
            let upload_path = match upload_file_action_path(
                params,
                file_system,
                enforce_upload_file_availability,
                available_file_paths,
            ) {
                Ok(upload_path) => upload_path,
                Err(error) => return Ok(ActionResult::error(error)),
            };
            session.upload_file(params.index, &upload_path).await?;
            Ok(ActionResult::extracted(format!(
                "Uploaded {} to element {}",
                upload_path.display(),
                params.index
            )))
        }
        BrowserAction::WriteFile(params) => file_system.write_file(params),
        BrowserAction::ReadFile(params) => file_system.read_file(&params.file_name),
        BrowserAction::ReplaceFile(params) => {
            file_system.replace_file(&params.file_name, &params.old_str, &params.new_str)
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

/// Execute a rematched replay plan while preserving history coordinates and
/// the same generic stop rules used by action sequence execution.
pub async fn execute_history_replay_plan<E>(
    executor: &mut E,
    plan: &AgentHistoryReplayPlan,
) -> AgentHistoryReplayExecution
where
    E: ActionExecutor + Send,
{
    let mut items = Vec::new();
    let mut stop = None;

    for (plan_index, item) in plan.actions.iter().enumerate() {
        let action = &item.remapped_action;
        // Replay follows the same done-after-prior-action rule as normal agent
        // execution: a terminal answer is only valid as the first action in a
        // sequence, after the browser has already been observed.
        if plan_index > 0 && matches!(action, BrowserAction::Done(_)) {
            stop = Some(AgentHistoryReplayStop {
                step_index: item.step_index,
                action_index: item.action_index,
                reason: AgentHistoryReplayStopReason::DoneAfterPriorAction,
                diagnostic: None,
            });
            break;
        }

        let result = executor.execute(action).await;
        let stop_reason = replay_stop_reason(action, &result);
        let stop_diagnostic = result.error.clone();
        // Each execution row keeps both the original and remapped action. That
        // makes replay auditable when an element index was adjusted before the
        // browser side effect ran.
        items.push(AgentHistoryReplayExecutionItem {
            step_index: item.step_index,
            action_index: item.action_index,
            original_action: item.original_action.clone(),
            executed_action: action.clone(),
            rematch: item.rematch.clone(),
            result,
        });

        if let Some(reason) = stop_reason {
            stop = Some(AgentHistoryReplayStop {
                step_index: item.step_index,
                action_index: item.action_index,
                reason,
                diagnostic: stop_diagnostic,
            });
            break;
        }
    }

    AgentHistoryReplayExecution { items, stop }
}

fn replay_stop_reason(
    action: &BrowserAction,
    result: &ActionResult,
) -> Option<AgentHistoryReplayStopReason> {
    if result.error.is_some() {
        Some(AgentHistoryReplayStopReason::Error)
    } else if result.is_done {
        Some(AgentHistoryReplayStopReason::Done)
    } else if action.terminates_sequence() {
        Some(AgentHistoryReplayStopReason::TerminatingAction)
    } else {
        None
    }
}
