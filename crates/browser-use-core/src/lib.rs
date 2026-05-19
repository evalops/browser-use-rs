//! Core agent contracts for browser-use-rs.

#[cfg(test)]
use serde_json::Value;
#[cfg(test)]
use std::time::Duration;
#[cfg(test)]
use tokio::time::{sleep, timeout};
#[cfg(test)]
use uuid::Uuid;

#[cfg(test)]
use async_trait::async_trait;
#[cfg(test)]
use base64::Engine;
#[cfg(test)]
use browser_use_cdp::{BrowserError, BrowserSession};

pub use browser_use_dom::{
    BrowserStateSummary, DomInteractedElement, DomInteractedElementMatch,
    DomInteractedElementMatchFailure, DomInteractedElementMatchFailureReason,
    DomInteractedElementMatchLevel, SerializedDomState,
};
pub use browser_use_llm::{
    AnthropicChatModel, ChatCompletion, ChatMessage, ChatModel, ChatRequest, ChatUsage,
    ContentPart, GeminiChatModel, ImageDetailLevel, LlmError, MessageRole, OllamaChatModel,
    OpenAiCompatibleChatModel,
};
pub use browser_use_tools::{BrowserAction, SearchEngine};
mod agent;
mod executor;
mod file_system;
mod history;
mod prompt;
mod settings;
mod urls;
mod usage;

pub(crate) use agent::now_seconds;
pub use agent::{
    Agent, AgentCallbackFuture, AgentCheckpoint, AgentDoneCallback, AgentExternalStatusCallback,
    AgentRunError, AgentShouldStopCallback, AgentStepCallback, AgentTask,
};
#[cfg(test)]
pub(crate) use agent::{encode_conversation_snapshot, expand_user_path, generate_gif_output_path};
pub use executor::{
    ActionExecutor, BrowserActionExecutor, execute_action_sequence, execute_history_replay_plan,
};
pub(crate) use executor::{build_extract_llm_request, complete_llm_extract_result, truncate_chars};
#[cfg(test)]
pub(crate) use executor::{
    extract_action_result, next_available_pdf_path, pdf_output_path, screenshot_output_path,
};
pub(crate) use file_system::display_done_file;
pub use file_system::{
    DEFAULT_FILE_SYSTEM_PATH, FileSystemFileData, FileSystemState, FileSystemStoredFile,
    ManagedFileSystem,
};
#[cfg(test)]
pub(crate) use file_system::{
    PDF_READ_MAX_CHARS, pdf_document_bytes, read_docx_text, read_file_action,
    render_pdf_read_envelope, replace_file_action, resolve_file_action_path,
    supported_text_extensions, write_file_action,
};
pub(crate) use history::historical_replay_actions;
pub use history::{
    ActionReplayRematch, ActionResult, AgentCurrentState, AgentHistory, AgentHistoryItem,
    AgentHistoryReplayExecution, AgentHistoryReplayExecutionItem, AgentHistoryReplayPlan,
    AgentHistoryReplayPlanError, AgentHistoryReplayPlanItem, AgentHistoryReplayRun,
    AgentHistoryReplayRunError, AgentHistoryReplayStop, AgentHistoryReplayStopReason, AgentOutput,
    JudgementResult, MessageCompactionOutput, ModelUsageStats, StepMetadata, UsageSummary,
    build_history_replay_plan, rematch_action_for_replay,
};
#[cfg(test)]
pub(crate) use prompt::{
    MAX_PROMPT_CONTENT_CHARS, match_url_with_domain_pattern, render_previous_results,
    render_read_state_description, schema_for_agent_output, schema_for_agent_output_with_settings,
    schema_for_final_response_after_failure, schema_variant_action_name, totp_code_at,
};
pub(crate) use prompt::{
    actions_for_execution, build_final_response_after_failure_request,
    build_final_response_after_step_limit_request, build_judge_request,
    build_message_compaction_request, build_step_request_with_budget_warning,
    excluded_action_error, latest_history_step_number, render_history_items_for_compaction,
    repeated_action_loop, retain_first_and_recent_history_items,
    scale_coordinate_click_actions_for_prompt, should_inject_step_budget_warning,
};
pub use prompt::{build_step_request, build_step_request_with_file_system, schema_to_compat_value};
pub use settings::{
    AgentSettings, GenerateGif, LlmScreenshotSize, MessageCompaction, MessageCompactionSettings,
    SensitiveDataValue, VisionMode,
};
pub(crate) use settings::{
    action_timeout_duration, coerce_valid_action_timeout_seconds, default_action_timeout_seconds,
    is_zero, timed_out_action_result, wait_between_actions_duration,
};
#[cfg(test)]
pub(crate) use settings::{
    coerce_valid_wait_between_actions_seconds, default_wait_between_actions_seconds,
    parse_action_timeout_seconds,
};
pub use urls::search_url;
#[cfg(test)]
pub(crate) use urls::shorten_urls_in_text;
pub(crate) use urls::{
    extract_start_url_from_task, request_with_shortened_urls,
    restore_shortened_urls_in_agent_output,
};
pub(crate) use usage::TokenUsageTracker;

/// Version of the upstream browser-use source that this crate initially targets.
pub const INITIAL_UPSTREAM_COMMIT: &str = "157779338afdcc03023010ec3c24ad63d820453c";

#[cfg(test)]
mod tests;
