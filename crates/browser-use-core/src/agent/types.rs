use crate::{AgentHistory, AgentOutput, AgentSettings, FileSystemState};
use browser_use_cdp::BrowserError;
use browser_use_dom::BrowserStateSummary;
use browser_use_llm::LlmError;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::future::Future;
use std::pin::Pin;
use thiserror::Error;
use uuid::Uuid;

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

pub(crate) fn new_agent_id() -> Uuid {
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
