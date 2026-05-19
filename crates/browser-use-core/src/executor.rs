use crate::{
    ActionResult, AgentHistory, AgentHistoryReplayExecution, AgentHistoryReplayExecutionItem,
    AgentHistoryReplayPlan, AgentHistoryReplayPlanError, AgentHistoryReplayPlanItem,
    AgentHistoryReplayRun, AgentHistoryReplayRunError, AgentHistoryReplayStop,
    AgentHistoryReplayStopReason, ManagedFileSystem, action_timeout_duration,
    coerce_valid_action_timeout_seconds, default_action_timeout_seconds, display_done_file,
    historical_replay_actions, rematch_action_for_replay, search_url, timed_out_action_result,
};
use async_trait::async_trait;
use base64::Engine;
use browser_use_cdp::{BrowserError, BrowserSession, FoundElement};
use browser_use_dom::BrowserStateSummary;
use browser_use_llm::{ChatMessage, ChatRequest, MessageRole};
use browser_use_tools::BrowserAction;
use serde_json::Value;
use std::collections::BTreeSet;
use std::time::Duration;
use tokio::time::{sleep, timeout};

#[async_trait]
pub trait ActionExecutor {
    async fn execute(&mut self, action: &BrowserAction) -> ActionResult;
}

pub struct BrowserActionExecutor<S> {
    session: S,
    file_system: ManagedFileSystem,
    display_files_in_done_text: bool,
    action_timeout_seconds: f64,
    enforce_upload_file_availability: bool,
    available_file_paths: BTreeSet<String>,
}

impl<S> BrowserActionExecutor<S> {
    #[must_use]
    pub fn new(session: S) -> Self {
        Self::with_file_system(
            session,
            ManagedFileSystem::new_in_temp().expect("create managed file system"),
        )
    }

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

    #[must_use]
    pub fn session(&self) -> &S {
        &self.session
    }

    #[must_use]
    pub fn file_system(&self) -> &ManagedFileSystem {
        &self.file_system
    }

    pub fn file_system_mut(&mut self) -> &mut ManagedFileSystem {
        &mut self.file_system
    }

    pub fn set_display_files_in_done_text(&mut self, display_files_in_done_text: bool) {
        self.display_files_in_done_text = display_files_in_done_text;
    }

    pub fn set_action_timeout_seconds(&mut self, action_timeout_seconds: f64) {
        self.action_timeout_seconds = coerce_valid_action_timeout_seconds(action_timeout_seconds);
    }

    #[must_use]
    pub fn action_timeout_seconds(&self) -> f64 {
        coerce_valid_action_timeout_seconds(self.action_timeout_seconds)
    }

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

fn done_action_result(
    params: &browser_use_tools::DoneAction,
    file_system: Option<&ManagedFileSystem>,
    display_files_in_done_text: bool,
) -> ActionResult {
    let mut user_message = params.text.clone();
    let mut file_sections = Vec::new();
    let mut attachments = Vec::new();

    for file_name in &params.files_to_display {
        let displayed_file = file_system
            .and_then(|file_system| file_system.display_done_file(file_name))
            .or_else(|| display_done_file(file_name));
        if let Some((section, attachment)) = displayed_file {
            if display_files_in_done_text {
                file_sections.push(section);
            }
            attachments.push(attachment);
        }
    }

    if !file_sections.is_empty() {
        user_message.push_str("\n\nAttachments:");
        for section in file_sections {
            user_message.push_str("\n\n");
            user_message.push_str(&section);
        }
    }

    ActionResult::done_with_attachments(user_message, params.success, attachments)
}

fn upload_file_action_path(
    params: &browser_use_tools::UploadFileAction,
    file_system: &ManagedFileSystem,
    enforce_upload_file_availability: bool,
    available_file_paths: &BTreeSet<String>,
) -> Result<std::path::PathBuf, String> {
    if !enforce_upload_file_availability {
        return Ok(std::path::PathBuf::from(&params.path));
    }

    let path = if available_file_paths.contains(&params.path) {
        std::path::PathBuf::from(&params.path)
    } else if let Some(path) = file_system.upload_file_path(&params.path) {
        path
    } else {
        return Err(format!(
            "File path {} is not available. Add it to AgentSettings.available_file_paths before using upload_file.",
            params.path
        ));
    };

    if !path.exists() {
        return Err(format!("File {} does not exist", path.display()));
    }
    if path.metadata().map(|metadata| metadata.len()).unwrap_or(0) == 0 {
        return Err(format!(
            "File {} is empty (0 bytes). The file may not have been saved correctly.",
            path.display()
        ));
    }

    Ok(path)
}

const MAX_EXTRACT_CHAR_LIMIT: usize = 100_000;
const MAX_EXTRACT_RELATED_ELEMENTS: usize = 200;
const MAX_EXTRACT_MEMORY_LENGTH: usize = 10_000;
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

pub(crate) fn extract_action_result(
    params: &browser_use_tools::ExtractAction,
    page_text: &str,
    source_url: Option<&str>,
    extract_images: bool,
    links: Option<&[FoundElement]>,
    images: Option<&[FoundElement]>,
    file_system: Option<&mut ManagedFileSystem>,
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
    } else if let Some(file_name) =
        file_system.and_then(|file_system| file_system.save_extracted_content(&rendered).ok())
    {
        format!(
            "Query: {}\nContent in {file_name} and once in <read_state>.",
            params.query
        )
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
        judgement: None,
        long_term_memory: Some(memory),
        include_extracted_content_only_once: true,
        include_in_memory: true,
        is_done: false,
        success: None,
        attachments: Vec::new(),
        images: Vec::new(),
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

pub(crate) fn build_extract_llm_request(
    params: &browser_use_tools::ExtractAction,
    raw_envelope: &str,
) -> ChatRequest {
    let structured_schema = params.output_schema.clone();
    let system_prompt = if structured_schema.is_some() {
        "You are an expert at extracting structured data from webpage markdown. Extract only information present in the webpage. Return data that conforms exactly to the provided JSON Schema."
    } else {
        "You are an expert at extracting data from webpage markdown. Extract only information relevant to the query. Do not guess or use outside knowledge."
    };
    let user_prompt = format!(
        "Use the prepared extraction envelope below. It contains the query, content statistics, webpage markdown, and any link/image/already-collected context.\n\n{raw_envelope}"
    );

    ChatRequest {
        messages: vec![
            ChatMessage::text(MessageRole::System, system_prompt),
            ChatMessage::text(MessageRole::User, user_prompt),
        ],
        output_schema: Some(structured_schema.unwrap_or_else(schema_for_extract_text_result)),
    }
}

fn schema_for_extract_text_result() -> Value {
    serde_json::json!({
        "type": "object",
        "properties": {
            "result": { "type": "string" }
        },
        "required": ["result"],
        "additionalProperties": false
    })
}

pub(crate) fn complete_llm_extract_result(
    params: &browser_use_tools::ExtractAction,
    raw_envelope: &str,
    raw_metadata: Option<&Value>,
    completion: Value,
    file_system: &mut ManagedFileSystem,
) -> ActionResult {
    let source_url = tagged_section(raw_envelope, "url").unwrap_or("about:blank");
    let metadata = params
        .output_schema
        .as_ref()
        .map(|schema| structured_extract_metadata(schema, raw_metadata, source_url, &completion));
    let extracted_content = if params.output_schema.is_some() {
        format!(
            "<url>\n{source_url}\n</url>\n<query>\n{}\n</query>\n<structured_result>\n{}\n</structured_result>",
            params.query,
            serde_json::to_string(&completion).unwrap_or_else(|_| completion.to_string())
        )
    } else {
        format!(
            "<url>\n{source_url}\n</url>\n<query>\n{}\n</query>\n<result>\n{}\n</result>",
            params.query,
            extract_text_completion(completion)
        )
    };
    let (long_term_memory, include_extracted_content_only_once) =
        extract_memory_fields(&params.query, &extracted_content, file_system);

    ActionResult {
        extracted_content: Some(extracted_content),
        error: None,
        judgement: None,
        long_term_memory: Some(long_term_memory),
        include_extracted_content_only_once,
        include_in_memory: true,
        is_done: false,
        success: None,
        attachments: Vec::new(),
        images: Vec::new(),
        metadata,
    }
}

fn extract_text_completion(completion: Value) -> String {
    completion
        .get("result")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
        .or_else(|| completion.as_str().map(ToOwned::to_owned))
        .unwrap_or_else(|| completion.to_string())
}

fn extract_memory_fields(
    query: &str,
    extracted_content: &str,
    file_system: &mut ManagedFileSystem,
) -> (String, bool) {
    if extracted_content.chars().count() < MAX_EXTRACT_MEMORY_LENGTH {
        return (extracted_content.to_owned(), false);
    }

    if let Ok(file_name) = file_system.save_extracted_content(extracted_content) {
        (
            format!("Query: {query}\nContent in {file_name} and once in <read_state>."),
            true,
        )
    } else {
        (
            format!(
                "Query: {query}\nExtracted result length: {} characters.",
                extracted_content.chars().count()
            ),
            true,
        )
    }
}

fn structured_extract_metadata(
    schema: &Value,
    raw_metadata: Option<&Value>,
    source_url: &str,
    data: &Value,
) -> Value {
    serde_json::json!({
        "structured_extraction": true,
        "extraction_result": {
            "data": data,
            "schema_used": schema,
            "is_partial": raw_metadata
                .and_then(|metadata| metadata.get("is_partial"))
                .cloned()
                .unwrap_or(Value::Bool(false)),
            "source_url": source_url,
            "content_stats": raw_metadata
                .and_then(|metadata| metadata.get("content_stats"))
                .cloned()
                .unwrap_or(Value::Null),
        }
    })
}

fn tagged_section<'a>(text: &'a str, tag: &str) -> Option<&'a str> {
    let start_tag = format!("<{tag}>\n");
    let end_tag = format!("\n</{tag}>");
    let start = text.find(&start_tag)? + start_tag.len();
    let end = text[start..].find(&end_tag)? + start;
    Some(&text[start..end])
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

pub(crate) fn pdf_output_path(
    file_name: Option<&str>,
    page_title: Option<&str>,
) -> std::path::PathBuf {
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

pub(crate) fn next_available_pdf_path(path: std::path::PathBuf) -> std::path::PathBuf {
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

pub(crate) fn screenshot_output_path(file_name: &str) -> std::path::PathBuf {
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

pub(crate) fn truncate_chars(text: &str, max_chars: usize) -> String {
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
