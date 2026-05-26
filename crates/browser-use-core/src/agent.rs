//! Agent run loop, checkpoints, callbacks, and model/browser orchestration.
//!
//! [`Agent`] is generic over a chat model and a browser session. That generic
//! shape keeps the run loop independent from any specific LLM provider or
//! browser backend while still allowing strongly typed tests and adapters.
//!
//! The run loop has three layers:
//!
//! 1. [`Agent::run`] owns stop conditions, retry accounting, and step budget
//!    policy.
//! 2. `step_recovering_model_errors_with_kind` turns browser state into an LLM
//!    request and records provider or schema failures as history items.
//! 3. `record_model_output` validates the accepted model output and delegates
//!    browser side effects to [`BrowserActionExecutor`].
//!
//! ```mermaid
//! flowchart TD
//!     Start["Agent::run(max_steps)"] --> Initial["execute_initial_actions"]
//!     Initial --> Loop{"steps remaining?"}
//!     Loop -->|yes| State["capture BrowserStateSummary"]
//!     State --> Prompt["build ChatRequest + AgentOutput schema"]
//!     Prompt --> LLM["ChatModel::invoke_json"]
//!     LLM --> Parse["parse AgentOutput"]
//!     Parse --> Validate["callbacks, excluded actions, limits"]
//!     Validate --> Execute["BrowserActionExecutor sequence"]
//!     Execute --> Record["append AgentHistoryItem"]
//!     Record --> Done{"done / loop / failures?"}
//!     Done -->|continue| Loop
//!     Done -->|terminal| Finish["return AgentHistory or AgentRunError"]
//! ```

use crate::{
    ActionResult, AgentHistory, AgentHistoryItem, AgentOutput, AgentSettings, BrowserAction,
    BrowserActionExecutor, FileSystemState, JudgementResult, ManagedFileSystem,
    MessageCompactionOutput, StepMetadata, TokenUsageTracker, action_timeout_duration,
    actions_for_execution, build_extract_llm_request, build_final_response_after_failure_request,
    build_final_response_after_step_limit_request, build_judge_request,
    build_message_compaction_request, build_step_request_with_budget_warning,
    build_step_request_with_file_system, complete_llm_extract_result, excluded_action_error,
    latest_history_step_number, render_history_items_for_compaction, repeated_action_loop,
    request_with_shortened_urls, restore_shortened_urls_in_agent_output,
    retain_first_and_recent_history_items, scale_coordinate_click_actions_for_prompt,
    should_inject_step_budget_warning, timed_out_action_result, truncate_chars,
    wait_between_actions_duration,
};
use browser_use_cdp::{BrowserError, BrowserSession};
use browser_use_dom::BrowserStateSummary;
use browser_use_llm::{ChatCompletion, ChatModel, ChatRequest, LlmError};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::future::Future;
use std::pin::Pin;
use std::time::Duration;
use thiserror::Error;
use tokio::time::{sleep, timeout};
use uuid::Uuid;

mod artifacts;

pub(crate) use artifacts::{
    encode_conversation_snapshot, expand_user_path, generate_gif_output_path, now_seconds,
};
use artifacts::{
    format_conversation_snapshot, initial_actions_model_output, initial_actions_state_history,
    is_single_done_output, managed_file_system_for_settings, result_requests_screenshot,
    settings_with_direct_start_url, settings_with_llm_screenshot_default, write_history_gif,
};

/// Serializable task envelope with settings and generated id.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct AgentTask {
    /// Task/run id.
    pub id: Uuid,
    /// Natural-language task instructions.
    pub task: String,
    /// Settings used for this task.
    #[serde(default)]
    pub settings: AgentSettings,
}

impl AgentTask {
    /// Creates a task with default settings and a fresh id.
    #[must_use]
    pub fn new(task: impl Into<String>) -> Self {
        Self {
            id: new_agent_id(),
            task: task.into(),
            settings: AgentSettings::default(),
        }
    }
}

/// Error returned by agent construction, stepping, and running.
#[derive(Debug, Error)]
pub enum AgentRunError {
    /// Browser/session error.
    #[error(transparent)]
    Browser(#[from] BrowserError),
    /// LLM/provider error.
    #[error(transparent)]
    Llm(#[from] LlmError),
    /// Model output did not match the expected schema.
    #[error("invalid agent output: {0}")]
    InvalidOutput(String),
    /// LLM call exceeded its timeout.
    #[error("LLM call timed out after {seconds} seconds")]
    LlmTimedOut {
        /// Timeout seconds.
        seconds: u64,
    },
    /// Complete agent step exceeded its timeout.
    #[error("agent step timed out after {seconds} seconds")]
    StepTimedOut {
        /// Timeout seconds.
        seconds: u64,
    },
    /// Run exhausted the configured step budget.
    #[error("agent reached max steps ({max_steps}) without completing")]
    StepLimitReached {
        /// Maximum steps requested by the caller.
        max_steps: usize,
    },
    /// Too many consecutive step/action failures occurred.
    #[error("agent stopped after {failures} consecutive failures")]
    MaxFailuresExceeded {
        /// Consecutive failure count.
        failures: u32,
    },
    /// Repeated-action loop detection fired.
    #[error("agent repeated the same action sequence for {window} steps")]
    LoopDetected {
        /// Detection window size.
        window: usize,
    },
    /// Stop was requested before a step.
    #[error("agent stopped before the next step: {reason}")]
    Stopped {
        /// Stop reason.
        reason: String,
    },
    /// Agent is paused.
    #[error("agent paused before the next step")]
    Paused,
    /// External status callback interrupted the run.
    #[error("agent interrupted by external status callback")]
    ExternalStatusInterrupted,
    /// User-provided callback failed.
    #[error("agent callback {callback} failed: {message}")]
    Callback {
        /// Callback name.
        callback: &'static str,
        /// Callback error message.
        message: String,
    },
    /// Conversation transcript could not be saved.
    #[error("failed to save conversation to {path}: {source}")]
    ConversationSave {
        /// Output path.
        path: String,
        /// I/O source error.
        #[source]
        source: std::io::Error,
    },
    /// Requested transcript encoding is unknown.
    #[error("unsupported conversation transcript encoding {encoding:?}")]
    ConversationEncoding {
        /// Requested encoding.
        encoding: String,
    },
    /// Transcript text cannot be represented in the requested encoding.
    #[error("conversation transcript encoding {encoding:?} cannot represent the transcript text")]
    ConversationEncodingLossy {
        /// Requested encoding.
        encoding: String,
    },
    /// GIF history artifact could not be written.
    #[error("failed to save agent GIF at {path}: {message}")]
    GifSave {
        /// Output path.
        path: String,
        /// Error message.
        message: String,
    },
}

/// Serializable snapshot for pausing and resuming an agent run.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct AgentCheckpoint {
    /// Agent id.
    #[serde(default = "new_agent_id")]
    pub id: Uuid,
    /// Current task text, including follow-up requests.
    pub task: String,
    /// Runtime settings.
    pub settings: AgentSettings,
    /// Durable history so far.
    pub history: AgentHistory,
    /// Whether initial actions have already run.
    pub initial_actions_executed: bool,
    /// Whether stop has been requested.
    #[serde(default, skip_serializing_if = "is_false")]
    pub stopped: bool,
    /// Whether the agent is paused.
    #[serde(default, skip_serializing_if = "is_false")]
    pub paused: bool,
    /// Managed file-system snapshot.
    pub file_system_state: FileSystemState,
}

fn is_false(value: &bool) -> bool {
    !*value
}

fn new_agent_id() -> Uuid {
    Uuid::now_v7()
}

/// Boxed future returned by async agent callbacks.
pub type AgentCallbackFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T, String>> + Send + 'a>>;
/// Callback invoked after a new step model output is accepted.
pub type AgentStepCallback = Box<
    dyn for<'a> FnMut(
            &'a BrowserStateSummary,
            &'a AgentOutput,
            usize,
        ) -> AgentCallbackFuture<'a, ()>
        + Send
        + 'static,
>;
/// Callback invoked after the agent completes successfully.
pub type AgentDoneCallback =
    Box<dyn for<'a> FnMut(&'a AgentHistory) -> AgentCallbackFuture<'a, ()> + Send + 'static>;
/// Callback polled before steps to request a graceful stop.
pub type AgentShouldStopCallback =
    Box<dyn FnMut() -> AgentCallbackFuture<'static, bool> + Send + 'static>;
/// Callback polled before steps to report external interruption status.
pub type AgentExternalStatusCallback =
    Box<dyn FnMut() -> AgentCallbackFuture<'static, bool> + Send + 'static>;

/// Browser-use agent run loop.
pub struct Agent<M, S> {
    pub(crate) id: Uuid,
    pub(crate) task: String,
    pub(crate) settings: AgentSettings,
    pub(crate) llm: M,
    pub(crate) page_extraction_llm: Option<M>,
    pub(crate) judge_llm: Option<M>,
    pub(crate) fallback_llm: Option<M>,
    pub(crate) using_fallback_llm: bool,
    pub(crate) executor: BrowserActionExecutor<S>,
    pub(crate) history: AgentHistory,
    pub(crate) token_usage: TokenUsageTracker,
    pub(crate) initial_actions_executed: bool,
    pub(crate) stopped: bool,
    pub(crate) paused: bool,
    pub(crate) step_callbacks: Vec<AgentStepCallback>,
    pub(crate) done_callbacks: Vec<AgentDoneCallback>,
    pub(crate) should_stop_callback: Option<AgentShouldStopCallback>,
    pub(crate) external_status_callback: Option<AgentExternalStatusCallback>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StepRequestKind {
    Normal,
    BudgetWarning { steps_used: usize, max_steps: usize },
    FinalStep { max_steps: usize },
}

#[derive(Debug)]
enum AgentLlmCallError {
    TimedOut { seconds: u64 },
    Provider(LlmError),
}

fn is_fallback_eligible_llm_error(error: &LlmError) -> bool {
    matches!(error, LlmError::Provider(_) | LlmError::RateLimited(_))
}

fn agent_llm_call_error_to_run_error(error: AgentLlmCallError) -> AgentRunError {
    match error {
        AgentLlmCallError::TimedOut { seconds } => AgentRunError::LlmTimedOut { seconds },
        AgentLlmCallError::Provider(error) => AgentRunError::Llm(error),
    }
}

impl<M, S> Agent<M, S>
where
    M: ChatModel,
    S: BrowserSession + Send + Sync,
{
    /// Creates an agent with default settings.
    #[must_use]
    pub fn new(task: impl Into<String>, llm: M, session: S) -> Self {
        Self::with_settings(task, AgentSettings::default(), llm, session)
    }

    /// Creates an agent with explicit settings.
    #[must_use]
    pub fn with_settings(
        task: impl Into<String>,
        settings: AgentSettings,
        llm: M,
        session: S,
    ) -> Self {
        let file_system =
            managed_file_system_for_settings(&settings).expect("create managed file system");
        Self::with_settings_and_file_system(task, settings, llm, session, file_system)
    }

    /// Creates an agent with explicit settings and managed file system.
    #[must_use]
    pub fn with_settings_and_file_system(
        task: impl Into<String>,
        settings: AgentSettings,
        llm: M,
        session: S,
        file_system: ManagedFileSystem,
    ) -> Self {
        let task = task.into();
        let settings = settings_with_direct_start_url(&task, settings);
        let settings = settings_with_llm_screenshot_default(settings, llm.model());
        let mut executor = BrowserActionExecutor::with_file_system(session, file_system);
        executor.set_display_files_in_done_text(settings.display_files_in_done_text);
        executor.set_action_timeout_seconds(settings.action_timeout_seconds);
        executor.set_upload_file_availability(true, settings.available_file_paths.clone());
        let token_usage = TokenUsageTracker::for_settings(&settings);
        Self {
            id: new_agent_id(),
            task,
            settings,
            llm,
            page_extraction_llm: None,
            judge_llm: None,
            fallback_llm: None,
            using_fallback_llm: false,
            executor,
            history: AgentHistory::default(),
            token_usage,
            initial_actions_executed: false,
            stopped: false,
            paused: false,
            step_callbacks: Vec::new(),
            done_callbacks: Vec::new(),
            should_stop_callback: None,
            external_status_callback: None,
        }
    }

    /// Restores an agent from a checkpoint with new live model/session handles.
    pub fn from_checkpoint(
        checkpoint: AgentCheckpoint,
        llm: M,
        session: S,
    ) -> Result<Self, BrowserError> {
        let file_system = ManagedFileSystem::from_state(checkpoint.file_system_state)?;
        let mut executor = BrowserActionExecutor::with_file_system(session, file_system);
        executor.set_display_files_in_done_text(checkpoint.settings.display_files_in_done_text);
        executor.set_action_timeout_seconds(checkpoint.settings.action_timeout_seconds);
        executor
            .set_upload_file_availability(true, checkpoint.settings.available_file_paths.clone());
        let token_usage = TokenUsageTracker::for_settings(&checkpoint.settings)
            .with_base_summary(checkpoint.history.usage.clone());
        Ok(Self {
            id: checkpoint.id,
            task: checkpoint.task,
            settings: checkpoint.settings,
            llm,
            page_extraction_llm: None,
            judge_llm: None,
            fallback_llm: None,
            using_fallback_llm: false,
            executor,
            history: checkpoint.history,
            token_usage,
            initial_actions_executed: checkpoint.initial_actions_executed,
            stopped: checkpoint.stopped,
            paused: checkpoint.paused,
            step_callbacks: Vec::new(),
            done_callbacks: Vec::new(),
            should_stop_callback: None,
            external_status_callback: None,
        })
    }

    /// Returns immutable access to the run history.
    pub fn history(&self) -> &AgentHistory {
        &self.history
    }

    /// Returns the agent id.
    pub fn id(&self) -> Uuid {
        self.id
    }

    /// Backward-compatible alias for [`Agent::id`].
    pub fn task_id(&self) -> Uuid {
        self.id
    }

    #[must_use]
    /// Returns this agent with a replacement id.
    pub fn with_task_id(mut self, task_id: Uuid) -> Self {
        self.set_task_id(task_id);
        self
    }

    /// Replaces the agent id.
    pub fn set_task_id(&mut self, task_id: Uuid) {
        self.id = task_id;
    }

    /// Returns this agent with a dedicated page-extraction model.
    pub fn with_page_extraction_llm(mut self, page_extraction_llm: M) -> Self {
        self.set_page_extraction_llm(page_extraction_llm);
        self
    }

    /// Sets a dedicated model for extract actions.
    pub fn set_page_extraction_llm(&mut self, page_extraction_llm: M) {
        self.page_extraction_llm = Some(page_extraction_llm);
    }

    /// Clears the dedicated extraction model.
    pub fn clear_page_extraction_llm(&mut self) {
        self.page_extraction_llm = None;
    }

    /// Returns this agent with a dedicated judge model.
    pub fn with_judge_llm(mut self, judge_llm: M) -> Self {
        self.set_judge_llm(judge_llm);
        self
    }

    /// Sets a dedicated judge model.
    pub fn set_judge_llm(&mut self, judge_llm: M) {
        self.judge_llm = Some(judge_llm);
    }

    /// Clears the dedicated judge model.
    pub fn clear_judge_llm(&mut self) {
        self.judge_llm = None;
    }

    /// Returns this agent with a fallback model for provider/rate-limit errors.
    pub fn with_fallback_llm(mut self, fallback_llm: M) -> Self {
        self.set_fallback_llm(fallback_llm);
        self
    }

    /// Sets a fallback model for provider/rate-limit errors.
    pub fn set_fallback_llm(&mut self, fallback_llm: M) {
        self.fallback_llm = Some(fallback_llm);
    }

    /// Clears the fallback model.
    pub fn clear_fallback_llm(&mut self) {
        self.fallback_llm = None;
    }

    #[must_use]
    /// Returns true after the agent has switched to its fallback model.
    pub fn is_using_fallback_llm(&self) -> bool {
        self.using_fallback_llm
    }

    /// Serializes current resumable state.
    pub fn checkpoint(&self) -> AgentCheckpoint {
        AgentCheckpoint {
            id: self.id,
            task: self.task.clone(),
            settings: self.settings.clone(),
            history: self.history.clone(),
            initial_actions_executed: self.initial_actions_executed,
            stopped: self.stopped,
            paused: self.paused,
            file_system_state: self.file_system_state(),
        }
    }

    /// Returns the managed file system used by file actions.
    pub fn file_system(&self) -> &ManagedFileSystem {
        self.executor.file_system()
    }

    /// Returns mutable access to the managed file system.
    pub fn file_system_mut(&mut self) -> &mut ManagedFileSystem {
        self.executor.file_system_mut()
    }

    /// Returns a serializable managed file-system snapshot.
    pub fn file_system_state(&self) -> FileSystemState {
        self.executor.file_system().get_state()
    }

    /// Registers a synchronous callback invoked after each accepted model output.
    pub fn register_new_step_callback<F, E>(&mut self, mut callback: F)
    where
        F: FnMut(&BrowserStateSummary, &AgentOutput, usize) -> Result<(), E> + Send + 'static,
        E: ToString + 'static,
    {
        self.register_new_step_callback_async(move |state, output, step| {
            let result = callback(state, output, step).map_err(|error| error.to_string());
            Box::pin(async move { result })
        });
    }

    /// Registers an async callback invoked after each accepted model output.
    pub fn register_new_step_callback_async<F>(&mut self, callback: F)
    where
        F: for<'a> FnMut(
                &'a BrowserStateSummary,
                &'a AgentOutput,
                usize,
            ) -> AgentCallbackFuture<'a, ()>
            + Send
            + 'static,
    {
        self.step_callbacks.push(Box::new(callback));
    }

    /// Removes all step callbacks.
    pub fn clear_new_step_callbacks(&mut self) {
        self.step_callbacks.clear();
    }

    /// Registers a synchronous callback invoked after successful completion.
    pub fn register_done_callback<F, E>(&mut self, mut callback: F)
    where
        F: FnMut(&AgentHistory) -> Result<(), E> + Send + 'static,
        E: ToString + 'static,
    {
        self.register_done_callback_async(move |history| {
            let result = callback(history).map_err(|error| error.to_string());
            Box::pin(async move { result })
        });
    }

    /// Registers an async callback invoked after successful completion.
    pub fn register_done_callback_async<F>(&mut self, callback: F)
    where
        F: for<'a> FnMut(&'a AgentHistory) -> AgentCallbackFuture<'a, ()> + Send + 'static,
    {
        self.done_callbacks.push(Box::new(callback));
    }

    /// Removes all done callbacks.
    pub fn clear_done_callbacks(&mut self) {
        self.done_callbacks.clear();
    }

    /// Registers a synchronous callback that can request a graceful stop.
    pub fn register_should_stop_callback<F, E>(&mut self, mut callback: F)
    where
        F: FnMut() -> Result<bool, E> + Send + 'static,
        E: ToString + 'static,
    {
        self.register_should_stop_callback_async(move || {
            let result = callback().map_err(|error| error.to_string());
            Box::pin(async move { result })
        });
    }

    /// Registers an async callback that can request a graceful stop.
    pub fn register_should_stop_callback_async<F>(&mut self, callback: F)
    where
        F: FnMut() -> AgentCallbackFuture<'static, bool> + Send + 'static,
    {
        self.should_stop_callback = Some(Box::new(callback));
    }

    /// Clears the stop callback.
    pub fn clear_should_stop_callback(&mut self) {
        self.should_stop_callback = None;
    }

    /// Registers a synchronous callback that reports external interruption.
    pub fn register_external_agent_status_callback<F, E>(&mut self, mut callback: F)
    where
        F: FnMut() -> Result<bool, E> + Send + 'static,
        E: ToString + 'static,
    {
        self.register_external_agent_status_callback_async(move || {
            let result = callback().map_err(|error| error.to_string());
            Box::pin(async move { result })
        });
    }

    /// Registers an async callback that reports external interruption.
    pub fn register_external_agent_status_callback_async<F>(&mut self, callback: F)
    where
        F: FnMut() -> AgentCallbackFuture<'static, bool> + Send + 'static,
    {
        self.external_status_callback = Some(Box::new(callback));
    }

    /// Compatibility alias for registering an external status callback.
    pub fn register_external_agent_status_raise_error_callback<F, E>(&mut self, callback: F)
    where
        F: FnMut() -> Result<bool, E> + Send + 'static,
        E: ToString + 'static,
    {
        self.register_external_agent_status_callback(callback);
    }

    /// Async compatibility alias for registering an external status callback.
    pub fn register_external_agent_status_raise_error_callback_async<F>(&mut self, callback: F)
    where
        F: FnMut() -> AgentCallbackFuture<'static, bool> + Send + 'static,
    {
        self.register_external_agent_status_callback_async(callback);
    }

    /// Clears the external status callback.
    pub fn clear_external_agent_status_callback(&mut self) {
        self.external_status_callback = None;
    }

    /// Requests that the agent stop before its next step.
    pub fn stop(&mut self) {
        self.stopped = true;
    }

    /// Returns true when stop has been requested.
    pub fn is_stopped(&self) -> bool {
        self.stopped
    }

    /// Appends a follow-up user request and clears stopped/paused state.
    pub fn add_new_task(&mut self, new_task: impl AsRef<str>) {
        if !self.task.contains("<initial_user_request>") {
            self.task = format!("<initial_user_request>{}</initial_user_request>", self.task);
        }
        self.task.push('\n');
        self.task.push_str(&format!(
            "<follow_up_user_request> {} </follow_up_user_request>",
            new_task.as_ref().trim()
        ));
        self.stopped = false;
        self.paused = false;
    }

    /// Pauses the agent before the next step.
    pub fn pause(&mut self) {
        self.paused = true;
    }

    /// Clears paused state.
    pub fn resume(&mut self) {
        self.paused = false;
    }

    /// Returns true when the agent is paused.
    pub fn is_paused(&self) -> bool {
        self.paused
    }

    async fn check_stop_requested(&mut self) -> Result<(), AgentRunError> {
        if let Some(callback) = self.should_stop_callback.as_mut() {
            let should_stop = callback()
                .await
                .map_err(|message| AgentRunError::Callback {
                    callback: "should_stop",
                    message,
                })?;
            if should_stop {
                self.stopped = true;
                return Err(AgentRunError::Stopped {
                    reason: "should_stop callback requested stop".to_owned(),
                });
            }
        }

        if let Some(callback) = self.external_status_callback.as_mut() {
            let interrupted = callback()
                .await
                .map_err(|message| AgentRunError::Callback {
                    callback: "external_agent_status",
                    message,
                })?;
            if interrupted {
                return Err(AgentRunError::ExternalStatusInterrupted);
            }
        }

        if self.stopped {
            return Err(AgentRunError::Stopped {
                reason: "stop requested".to_owned(),
            });
        }
        if self.paused {
            return Err(AgentRunError::Paused);
        }

        Ok(())
    }

    async fn invoke_step_callbacks(
        &mut self,
        state: &BrowserStateSummary,
        model_output: &AgentOutput,
        step_number: usize,
    ) -> Result<(), AgentRunError> {
        for callback in &mut self.step_callbacks {
            callback(state, model_output, step_number)
                .await
                .map_err(|message| AgentRunError::Callback {
                    callback: "new_step",
                    message,
                })?;
        }
        Ok(())
    }

    async fn invoke_done_callbacks(&mut self) -> Result<(), AgentRunError> {
        for callback in &mut self.done_callbacks {
            callback(&self.history)
                .await
                .map_err(|message| AgentRunError::Callback {
                    callback: "done",
                    message,
                })?;
        }
        Ok(())
    }

    async fn invoke_json_with_fallback(
        &mut self,
        request: ChatRequest,
    ) -> Result<ChatCompletion<Value>, AgentLlmCallError> {
        let seconds = self.settings.llm_timeout_seconds;
        let first = Self::invoke_json_once(&self.llm, seconds, request.clone()).await;
        match first {
            Err(AgentLlmCallError::Provider(error)) if self.try_switch_to_fallback_llm(&error) => {
                let completion = Self::invoke_json_once(&self.llm, seconds, request).await?;
                self.record_completion_usage(&completion).await;
                Ok(completion)
            }
            Ok(completion) => {
                self.record_completion_usage(&completion).await;
                Ok(completion)
            }
            Err(error) => Err(error),
        }
    }

    async fn invoke_json_once(
        llm: &M,
        seconds: u64,
        request: ChatRequest,
    ) -> Result<ChatCompletion<Value>, AgentLlmCallError> {
        timeout(Duration::from_secs(seconds), llm.invoke_json(request))
            .await
            .map_err(|_| AgentLlmCallError::TimedOut { seconds })?
            .map_err(AgentLlmCallError::Provider)
    }

    fn try_switch_to_fallback_llm(&mut self, error: &LlmError) -> bool {
        if self.using_fallback_llm || !is_fallback_eligible_llm_error(error) {
            return false;
        }
        let Some(fallback_llm) = self.fallback_llm.take() else {
            return false;
        };
        self.llm = fallback_llm;
        self.using_fallback_llm = true;
        true
    }

    async fn record_completion_usage(&mut self, completion: &ChatCompletion<Value>) {
        self.token_usage.add_completion(completion);
        self.refresh_usage_summary().await;
    }

    async fn refresh_usage_summary(&mut self) {
        self.history.usage = Some(self.token_usage.summary().await);
    }

    /// Runs the agent until completion, failure, stop, pause, or step budget exhaustion.
    pub async fn run(&mut self, max_steps: usize) -> Result<&AgentHistory, AgentRunError> {
        let mut consecutive_failures = 0;

        // Stop/pause checks happen before synthetic step-zero actions so callers
        // can safely construct an agent and immediately pause or cancel it.
        self.check_stop_requested().await?;
        self.execute_initial_actions().await?;
        if self
            .history
            .items
            .last()
            .is_some_and(|item| item.result.iter().any(|result| result.is_done))
        {
            return self.finish_successful_run().await;
        }

        for step_index in 0..max_steps {
            self.check_stop_requested().await?;
            let (is_done, has_error) = {
                let seconds = self.settings.step_timeout_seconds;
                let steps_used = step_index + 1;
                // The final-step and budget-warning prompts are still normal
                // steps from the browser executor's point of view. Only the
                // model instruction/schema is narrowed so history shape stays
                // compatible with upstream browser-use.
                let step_kind = if steps_used == max_steps {
                    StepRequestKind::FinalStep { max_steps }
                } else if should_inject_step_budget_warning(steps_used, max_steps) {
                    StepRequestKind::BudgetWarning {
                        steps_used,
                        max_steps,
                    }
                } else {
                    StepRequestKind::Normal
                };
                let item = timeout(
                    Duration::from_secs(seconds),
                    self.step_recovering_model_errors_with_kind(step_kind),
                )
                .await
                .map_err(|_| AgentRunError::StepTimedOut { seconds })??;
                (
                    item.result.iter().any(|result| result.is_done),
                    item.result.iter().any(|result| result.error.is_some()),
                )
            };

            if is_done {
                return self.finish_successful_run().await;
            }
            // Re-check after side effects so a stop request raised by a callback
            // or surrounding runtime wins before loop detection/compaction does
            // any extra work.
            self.check_stop_requested().await?;

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
                    // browser-use asks for one last model-authored done message
                    // after repeated failures. That path is opt-in because some
                    // integrations prefer a hard error over another model call.
                    if self.settings.final_response_after_failure {
                        let final_item = self
                            .record_final_response_after_failure(consecutive_failures)
                            .await?;
                        let final_is_done = final_item.result.iter().any(|result| result.is_done);
                        if final_is_done {
                            return self.finish_successful_run().await;
                        }
                    }
                    return Err(AgentRunError::MaxFailuresExceeded {
                        failures: consecutive_failures,
                    });
                }
            } else {
                consecutive_failures = 0;
            }

            self.maybe_compact_history().await;
        }

        Err(AgentRunError::StepLimitReached { max_steps })
    }

    /// Executes exactly one model-observe/action step.
    pub async fn step(&mut self) -> Result<&AgentHistoryItem, AgentRunError> {
        self.check_stop_requested().await?;
        let seconds = self.settings.step_timeout_seconds;
        timeout(Duration::from_secs(seconds), self.step_inner())
            .await
            .map_err(|_| AgentRunError::StepTimedOut { seconds })?
    }

    async fn step_inner(&mut self) -> Result<&AgentHistoryItem, AgentRunError> {
        let step_start_time = now_seconds();
        let include_screenshot = self.should_include_screenshot();
        let state = self.executor.session().state(include_screenshot).await?;
        let request = build_step_request_with_file_system(
            &self.task,
            &state,
            &self.history,
            &self.settings,
            Some(self.executor.file_system()),
        )?;
        let request_for_transcript = request.clone();
        // URL shortening is prompt-only. The transcript keeps the full request,
        // and model output is restored before any browser action executes.
        let (request, url_replacements) =
            request_with_shortened_urls(request, self.settings.url_shortening_limit);
        let completion = self
            .invoke_json_with_fallback(request)
            .await
            .map_err(agent_llm_call_error_to_run_error)?;
        let model_output: AgentOutput = serde_json::from_value(completion.content)
            .map_err(|error| AgentRunError::InvalidOutput(error.to_string()))?;
        let model_output = restore_shortened_urls_in_agent_output(model_output, &url_replacements)
            .map_err(|error| {
                AgentRunError::InvalidOutput(format!("restore shortened URLs: {error}"))
            })?;
        self.save_conversation_snapshot(&request_for_transcript, &model_output)?;
        self.record_model_output(state, model_output, Some(step_start_time))
            .await
    }

    async fn step_recovering_model_errors_with_kind(
        &mut self,
        step_kind: StepRequestKind,
    ) -> Result<&AgentHistoryItem, AgentRunError> {
        self.check_stop_requested().await?;
        let step_start_time = now_seconds();
        let include_screenshot = self.should_include_screenshot();
        let state = self.executor.session().state(include_screenshot).await?;
        let request = match step_kind {
            StepRequestKind::Normal => build_step_request_with_file_system(
                &self.task,
                &state,
                &self.history,
                &self.settings,
                Some(self.executor.file_system()),
            )?,
            StepRequestKind::BudgetWarning {
                steps_used,
                max_steps,
            } => build_step_request_with_budget_warning(
                &self.task,
                &state,
                &self.history,
                &self.settings,
                Some(self.executor.file_system()),
                steps_used,
                max_steps,
            )?,
            StepRequestKind::FinalStep { max_steps } => {
                build_final_response_after_step_limit_request(
                    &self.task,
                    &state,
                    &self.history,
                    &self.settings,
                    Some(self.executor.file_system()),
                    max_steps,
                )?
            }
        };
        let request_for_transcript = request.clone();
        // This recovery path records LLM/schema problems as ordinary history
        // errors, which lets `run` apply browser-use failure accounting instead
        // of aborting the entire run on the first malformed provider response.
        let (request, url_replacements) =
            request_with_shortened_urls(request, self.settings.url_shortening_limit);
        let completion = match self.invoke_json_with_fallback(request).await {
            Ok(completion) => completion,
            Err(AgentLlmCallError::Provider(error)) => {
                return self.record_model_error(
                    state,
                    format!("LLM provider error: {error}"),
                    Some(step_start_time),
                );
            }
            Err(AgentLlmCallError::TimedOut { seconds }) => {
                return self.record_model_error(
                    state,
                    format!("LLM call timed out after {seconds} seconds"),
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
        let model_output =
            match restore_shortened_urls_in_agent_output(model_output, &url_replacements) {
                Ok(model_output) => model_output,
                Err(error) => {
                    return self.record_model_error(
                        state,
                        format!("invalid agent output after URL restoration: {error}"),
                        Some(step_start_time),
                    );
                }
            };
        self.save_conversation_snapshot(&request_for_transcript, &model_output)?;
        if matches!(step_kind, StepRequestKind::FinalStep { .. })
            && !is_single_done_output(&model_output)
        {
            return self.record_model_error(
                state,
                "final response at step limit must return exactly one done action".to_owned(),
                Some(step_start_time),
            );
        }
        self.record_model_output(state, model_output, Some(step_start_time))
            .await
    }

    async fn record_model_output(
        &mut self,
        state: BrowserStateSummary,
        mut model_output: AgentOutput,
        step_start_time: Option<f64>,
    ) -> Result<&AgentHistoryItem, AgentRunError> {
        if model_output.action.len() > self.settings.max_actions_per_step {
            // Truncation mirrors upstream behavior: keep the model output shape,
            // but execute only the configured prefix so the browser never sees
            // more actions than the caller allowed.
            model_output
                .action
                .truncate(self.settings.max_actions_per_step);
        }
        let step_number = latest_history_step_number(&self.history)
            .map(|step| step + 1)
            .unwrap_or(1);
        self.invoke_step_callbacks(&state, &model_output, step_number)
            .await?;
        self.check_stop_requested().await?;
        if let Some(error) = excluded_action_error(&model_output.action, &self.settings) {
            return self.record_model_error(state, error, step_start_time);
        }
        // The stored history keeps exactly what the model said. The execution
        // copy may have secrets filled, extraction defaults injected, or prompt
        // screenshot coordinates scaled back to the real viewport.
        let actions = actions_for_execution(&model_output.action, &self.settings, &state.url);
        let actions = scale_coordinate_click_actions_for_prompt(&actions, &self.settings, &state);
        let result = self.execute_agent_sequence(&actions).await?;
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

    pub(crate) async fn execute_agent_sequence(
        &mut self,
        actions: &[BrowserAction],
    ) -> Result<Vec<ActionResult>, AgentRunError> {
        let mut results = Vec::new();

        for (index, action) in actions.iter().enumerate() {
            self.check_stop_requested().await?;

            // A done action after another action is ignored because the first
            // action may navigate or mutate the page. The next model step should
            // observe that new state before deciding the task is done.
            if index > 0 && matches!(action, BrowserAction::Done(_)) {
                break;
            }
            if index > 0 {
                let wait_seconds = self.settings.effective_wait_between_actions_seconds();
                if wait_seconds > 0.0 {
                    sleep(wait_between_actions_duration(wait_seconds)).await;
                    self.check_stop_requested().await?;
                }
            }

            let needs_page_change_guard = !action.terminates_sequence();
            let before = if needs_page_change_guard {
                // Non-terminating actions are guarded by a cheap pre/post state
                // check. If the URL changes, the current action sequence stops
                // so the next model call can reason about the new page.
                match self.executor.session().state(false).await {
                    Ok(state) => Some(state),
                    Err(error) => {
                        results.push(ActionResult::error(error.to_string()));
                        break;
                    }
                }
            } else {
                None
            };

            let timeout_seconds = self.settings.effective_action_timeout_seconds();
            let result = match timeout(action_timeout_duration(timeout_seconds), async {
                let raw_result = self.executor.execute_for_agent(action).await;
                self.resolve_agent_action_result(action, raw_result).await
            })
            .await
            {
                Ok(result) => result,
                Err(_) => timed_out_action_result(action, timeout_seconds),
            };
            let should_stop =
                result.is_done || result.error.is_some() || action.terminates_sequence();
            let page_changed = if should_stop {
                false
            } else if let Some(before) = before {
                match self.executor.session().state(false).await {
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

        Ok(results)
    }

    async fn resolve_agent_action_result(
        &mut self,
        action: &BrowserAction,
        raw_result: ActionResult,
    ) -> ActionResult {
        let BrowserAction::Extract(params) = action else {
            return raw_result;
        };
        if raw_result.error.is_some() {
            return raw_result;
        }
        let Some(raw_content) = raw_result.extracted_content.as_deref() else {
            return raw_result;
        };

        let request = build_extract_llm_request(params, raw_content);
        let completion = match Self::invoke_json_once(
            self.page_extraction_llm.as_ref().unwrap_or(&self.llm),
            self.settings.llm_timeout_seconds,
            request,
        )
        .await
        {
            Ok(completion) => completion,
            Err(AgentLlmCallError::Provider(error)) => {
                return ActionResult::error(format!("LLM-backed extract failed: {error}"));
            }
            Err(AgentLlmCallError::TimedOut { seconds }) => {
                return ActionResult::error(format!(
                    "LLM-backed extract timed out after {seconds} seconds"
                ));
            }
        };
        self.record_completion_usage(&completion).await;

        complete_llm_extract_result(
            params,
            raw_content,
            raw_result.metadata.as_ref(),
            completion.content,
            self.executor.file_system_mut(),
        )
    }

    pub(crate) async fn execute_initial_actions(&mut self) -> Result<(), AgentRunError> {
        if self.initial_actions_executed || self.settings.initial_actions.is_empty() {
            return Ok(());
        }
        self.initial_actions_executed = true;

        let step_start_time = now_seconds();
        let initial_actions = self.settings.initial_actions.clone();
        let state = initial_actions_state_history(&initial_actions);
        let current_url = state.url.clone();
        let execution_actions =
            actions_for_execution(&initial_actions, &self.settings, &current_url);
        let result = self.execute_agent_sequence(&execution_actions).await?;
        let step_end_time = now_seconds();

        self.history.items.push(AgentHistoryItem {
            model_output: Some(initial_actions_model_output(
                initial_actions,
                self.settings.flash_mode,
            )),
            result,
            state,
            metadata: Some(StepMetadata {
                step_start_time,
                step_end_time,
                step_number: 0,
                step_interval: None,
            }),
        });

        Ok(())
    }

    async fn record_final_response_after_failure(
        &mut self,
        failures: u32,
    ) -> Result<&AgentHistoryItem, AgentRunError> {
        self.check_stop_requested().await?;
        let step_start_time = now_seconds();
        let include_screenshot = self.should_include_screenshot();
        let state = self.executor.session().state(include_screenshot).await?;
        let request = build_final_response_after_failure_request(
            &self.task,
            &state,
            &self.history,
            &self.settings,
            Some(self.executor.file_system()),
            failures,
        )?;
        let request_for_transcript = request.clone();
        let (request, url_replacements) =
            request_with_shortened_urls(request, self.settings.url_shortening_limit);
        let completion = match self.invoke_json_with_fallback(request).await {
            Ok(completion) => completion,
            Err(AgentLlmCallError::Provider(error)) => {
                return self.record_model_error(
                    state,
                    format!("LLM provider error during final response after failure: {error}"),
                    Some(step_start_time),
                );
            }
            Err(AgentLlmCallError::TimedOut { seconds }) => {
                return self.record_model_error(
                    state,
                    format!(
                        "LLM call timed out after {seconds} seconds during final response after failure"
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
                    format!("invalid final response after failure: {error}"),
                    Some(step_start_time),
                );
            }
        };
        let model_output =
            match restore_shortened_urls_in_agent_output(model_output, &url_replacements) {
                Ok(model_output) => model_output,
                Err(error) => {
                    return self.record_model_error(
                        state,
                        format!("invalid final response after URL restoration: {error}"),
                        Some(step_start_time),
                    );
                }
            };
        self.save_conversation_snapshot(&request_for_transcript, &model_output)?;

        if !is_single_done_output(&model_output) {
            return self.record_model_error(
                state,
                "final response after failure must return exactly one done action".to_owned(),
                Some(step_start_time),
            );
        }

        self.record_model_output(state, model_output, Some(step_start_time))
            .await
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

    fn save_conversation_snapshot(
        &self,
        request: &ChatRequest,
        model_output: &AgentOutput,
    ) -> Result<(), AgentRunError> {
        let Some(directory) = self.settings.save_conversation_path.as_deref() else {
            return Ok(());
        };
        let directory = expand_user_path(directory);
        std::fs::create_dir_all(&directory).map_err(|source| AgentRunError::ConversationSave {
            path: directory.display().to_string(),
            source,
        })?;
        let target = directory.join(format!(
            "conversation_{}_{}.txt",
            self.id,
            self.next_step_number()
        ));
        let snapshot = format_conversation_snapshot(request, model_output);
        let bytes = encode_conversation_snapshot(
            &snapshot,
            self.settings.save_conversation_path_encoding.as_deref(),
        )?;
        std::fs::write(&target, bytes).map_err(|source| AgentRunError::ConversationSave {
            path: target.display().to_string(),
            source,
        })
    }

    fn save_history_gif(&self) -> Result<(), AgentRunError> {
        let Some(path) = generate_gif_output_path(&self.settings.generate_gif) else {
            return Ok(());
        };
        write_history_gif(&self.history, &path).map_err(|message| AgentRunError::GifSave {
            path: path.display().to_string(),
            message,
        })
    }

    async fn maybe_judge_done_result(&mut self) {
        if !self.settings.use_judge || !self.history.is_done() {
            return;
        }

        let request = build_judge_request(&self.task, &self.history, &self.settings);
        let Ok(completion) = Self::invoke_json_once(
            self.judge_llm.as_ref().unwrap_or(&self.llm),
            self.settings.llm_timeout_seconds,
            request,
        )
        .await
        else {
            return;
        };
        self.record_completion_usage(&completion).await;
        let Ok(judgement) = serde_json::from_value::<JudgementResult>(completion.content) else {
            return;
        };

        if let Some(result) = self
            .history
            .items
            .last_mut()
            .and_then(|item| item.result.last_mut())
            .filter(|result| result.is_done)
        {
            result.judgement = Some(judgement);
        }
    }

    async fn finish_successful_run(&mut self) -> Result<&AgentHistory, AgentRunError> {
        self.maybe_judge_done_result().await;
        self.refresh_usage_summary().await;
        self.save_history_gif()?;
        self.invoke_done_callbacks().await?;
        Ok(&self.history)
    }

    async fn maybe_compact_history(&mut self) -> bool {
        let Some(compaction_settings) = self.settings.message_compaction.resolved_settings() else {
            return false;
        };
        let Some(step_number) = latest_history_step_number(&self.history) else {
            return false;
        };
        let steps_since =
            step_number.saturating_sub(self.history.last_compaction_step.unwrap_or(0));
        if steps_since < compaction_settings.compact_every_n_steps {
            return false;
        }

        let full_history_text = render_history_items_for_compaction(&self.history);
        if full_history_text.chars().count() < compaction_settings.effective_trigger_char_count() {
            return false;
        }

        let request = build_message_compaction_request(
            &self.history,
            &compaction_settings,
            &self.settings.sensitive_data,
        );
        let Ok(completion) = timeout(
            Duration::from_secs(self.settings.llm_timeout_seconds),
            self.llm.invoke_json(request),
        )
        .await
        else {
            return false;
        };
        let Ok(completion) = completion else {
            return false;
        };
        self.record_completion_usage(&completion).await;
        let Ok(output) = serde_json::from_value::<MessageCompactionOutput>(completion.content)
        else {
            return false;
        };
        let mut summary = output.summary.trim().to_owned();
        if summary.is_empty() {
            return false;
        }
        if compaction_settings.summary_max_chars > 0
            && summary.chars().count() > compaction_settings.summary_max_chars
        {
            summary = truncate_chars(&summary, compaction_settings.summary_max_chars);
        }

        self.history.compacted_memory = Some(summary);
        self.history.compaction_count += 1;
        self.history.last_compaction_step = Some(step_number);
        retain_first_and_recent_history_items(
            &mut self.history,
            compaction_settings.keep_last_items,
        );
        true
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
            step_number: self.next_step_number(),
            step_interval,
        }
    }

    fn next_step_number(&self) -> usize {
        self.history
            .items
            .iter()
            .filter_map(|item| item.metadata.as_ref().map(|metadata| metadata.step_number))
            .max()
            .unwrap_or(0)
            + 1
    }

    fn should_include_screenshot(&self) -> bool {
        let action_requested_screenshot = self
            .history
            .items
            .last()
            .is_some_and(|item| item.result.iter().any(result_requests_screenshot));
        self.settings
            .use_vision
            .should_include_screenshot(action_requested_screenshot)
    }
}
