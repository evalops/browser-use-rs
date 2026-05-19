//! Agent output, step history, usage summaries, and replay planning.
//!
//! The agent stores each step as model output plus browser action results and
//! state. This module defines that durable shape, helpers that mirror upstream
//! browser-use accessors, and replay planning that remaps historical DOM
//! indexes to a current page.

use std::collections::BTreeMap;

use browser_use_dom::{
    BrowserStateSummary, DomInteractedElement, DomInteractedElementMatch,
    DomInteractedElementMatchFailure, SerializedDomState,
};
use browser_use_tools::BrowserAction;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::is_zero;

/// Structured model response for one agent step.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct AgentOutput {
    /// Nested browser-use state block, retained for upstream compatibility.
    #[serde(default, skip_serializing_if = "AgentCurrentState::is_empty")]
    pub current_state: AgentCurrentState,
    /// Optional top-level thinking text accepted from flattened providers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thinking: Option<String>,
    /// Optional evaluation of the previous goal.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evaluation_previous_goal: Option<String>,
    /// Optional memory note the model wants carried forward.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memory: Option<String>,
    /// Optional next-goal description.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_goal: Option<String>,
    /// Current plan item index when planning is enabled.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_plan_item: Option<usize>,
    /// Updated plan list when the model replans.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan_update: Option<Vec<String>>,
    /// Browser actions to execute for this step.
    pub action: Vec<BrowserAction>,
}

impl AgentOutput {
    /// Returns the effective brain/current-state fields across nested and flattened shapes.
    #[must_use]
    pub fn current_brain(&self) -> AgentCurrentState {
        AgentCurrentState {
            thinking: self
                .thinking
                .clone()
                .or_else(|| self.current_state.thinking.clone()),
            evaluation_previous_goal: self
                .evaluation_previous_goal
                .clone()
                .or_else(|| self.current_state.evaluation_previous_goal.clone()),
            memory: self
                .memory
                .clone()
                .or_else(|| self.current_state.memory.clone()),
            next_goal: self
                .next_goal
                .clone()
                .or_else(|| self.current_state.next_goal.clone()),
        }
    }
}

/// Current reasoning/memory fields emitted by the model.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct AgentCurrentState {
    /// Model reasoning or brief scratchpad.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thinking: Option<String>,
    /// Model evaluation of the prior step.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evaluation_previous_goal: Option<String>,
    /// Memory to carry into later steps.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memory: Option<String>,
    /// Next goal for the agent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_goal: Option<String>,
}

impl AgentCurrentState {
    /// Returns true when no current-state fields are present.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.thinking.is_none()
            && self.evaluation_previous_goal.is_none()
            && self.memory.is_none()
            && self.next_goal.is_none()
    }
}

/// Result of a judge/validation call.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct JudgementResult {
    /// Optional judge reasoning.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<String>,
    /// True when the judge validates the task result.
    pub verdict: bool,
    /// Optional failure reason when `verdict` is false.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure_reason: Option<String>,
    /// True when the task could not be completed as stated.
    #[serde(default)]
    pub impossible_task: bool,
    /// True when a CAPTCHA blocked completion.
    #[serde(default)]
    pub reached_captcha: bool,
}

/// Structured result from a message-compaction model call.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct MessageCompactionOutput {
    /// Compacted history summary.
    pub summary: String,
}

/// Result of executing one browser action.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct ActionResult {
    /// Text returned to the agent for the action.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extracted_content: Option<String>,
    /// Error text when the action failed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Optional judge result associated with the action.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub judgement: Option<JudgementResult>,
    /// Memory text retained in future prompts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub long_term_memory: Option<String>,
    /// Include extracted content only in the next prompt once.
    #[serde(default)]
    pub include_extracted_content_only_once: bool,
    /// Include this result in memory/history prompts.
    #[serde(default)]
    pub include_in_memory: bool,
    /// True when this action terminates the task.
    #[serde(default)]
    pub is_done: bool,
    /// Task success value, valid only when `is_done` is true.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub success: Option<bool>,
    /// Local artifact paths attached to the result.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attachments: Vec<String>,
    /// Image payloads returned by file/screenshot actions.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub images: Vec<Value>,
    /// Additional structured metadata for control flow.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<Value>,
}

#[derive(Deserialize)]
struct ActionResultWire {
    #[serde(default)]
    extracted_content: Option<String>,
    #[serde(default)]
    error: Option<String>,
    #[serde(default)]
    judgement: Option<JudgementResult>,
    #[serde(default)]
    long_term_memory: Option<String>,
    #[serde(default)]
    include_extracted_content_only_once: bool,
    #[serde(default)]
    include_in_memory: bool,
    #[serde(default)]
    is_done: bool,
    #[serde(default)]
    success: Option<bool>,
    #[serde(default)]
    attachments: Vec<String>,
    #[serde(default)]
    images: Vec<Value>,
    #[serde(default)]
    metadata: Option<Value>,
}

impl<'de> Deserialize<'de> for ActionResult {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let wire = ActionResultWire::deserialize(deserializer)?;
        if wire.success == Some(true) && !wire.is_done {
            return Err(serde::de::Error::custom(
                "success=true can only be set when is_done=true",
            ));
        }

        Ok(Self {
            extracted_content: wire.extracted_content,
            error: wire.error,
            judgement: wire.judgement,
            long_term_memory: wire.long_term_memory,
            include_extracted_content_only_once: wire.include_extracted_content_only_once,
            include_in_memory: wire.include_in_memory,
            is_done: wire.is_done,
            success: wire.success,
            attachments: wire.attachments,
            images: wire.images,
            metadata: wire.metadata,
        })
    }
}

impl ActionResult {
    /// Creates a successful non-terminal text result.
    #[must_use]
    pub fn extracted(text: impl Into<String>) -> Self {
        let text = text.into();
        Self {
            extracted_content: Some(text.clone()),
            error: None,
            judgement: None,
            long_term_memory: Some(text),
            include_extracted_content_only_once: false,
            include_in_memory: false,
            is_done: false,
            success: None,
            attachments: Vec::new(),
            images: Vec::new(),
            metadata: None,
        }
    }

    /// Creates an error result that is retained in memory.
    #[must_use]
    pub fn error(error: impl Into<String>) -> Self {
        Self {
            extracted_content: None,
            error: Some(error.into()),
            judgement: None,
            long_term_memory: None,
            include_extracted_content_only_once: false,
            include_in_memory: true,
            is_done: false,
            success: None,
            attachments: Vec::new(),
            images: Vec::new(),
            metadata: None,
        }
    }

    /// Creates a terminal done result.
    #[must_use]
    pub fn done(text: impl Into<String>, success: bool) -> Self {
        Self {
            extracted_content: Some(text.into()),
            error: None,
            judgement: None,
            long_term_memory: None,
            include_extracted_content_only_once: false,
            include_in_memory: true,
            is_done: true,
            success: Some(success),
            attachments: Vec::new(),
            images: Vec::new(),
            metadata: None,
        }
    }

    /// Creates a terminal done result with artifact attachments.
    #[must_use]
    pub fn done_with_attachments(
        text: impl Into<String>,
        success: bool,
        attachments: Vec<String>,
    ) -> Self {
        Self {
            attachments,
            ..Self::done(text, success)
        }
    }
}

/// Complete record for one agent step.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct AgentHistoryItem {
    /// Model output for the step, absent for synthetic/history-only entries.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_output: Option<AgentOutput>,
    /// Browser action results for the step.
    pub result: Vec<ActionResult>,
    /// Browser state observed before the step's model call.
    pub state: BrowserStateSummary,
    /// Timing and ordinal metadata.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<StepMetadata>,
}

/// Timing and sequence metadata for one step.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct StepMetadata {
    /// Step start time in seconds since epoch.
    pub step_start_time: f64,
    /// Step end time in seconds since epoch.
    pub step_end_time: f64,
    /// One-based step number.
    pub step_number: usize,
    /// Seconds since the previous step, when known.
    #[serde(default)]
    pub step_interval: Option<f64>,
}

impl StepMetadata {
    /// Returns `step_end_time - step_start_time`.
    #[must_use]
    pub fn duration_seconds(&self) -> f64 {
        self.step_end_time - self.step_start_time
    }
}

/// Aggregate usage and cost summary across agent model calls.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct UsageSummary {
    /// Total prompt tokens.
    pub total_prompt_tokens: u64,
    /// Estimated prompt cost.
    pub total_prompt_cost: f64,
    /// Prompt tokens served from cache.
    pub total_prompt_cached_tokens: u64,
    /// Estimated cached prompt cost.
    pub total_prompt_cached_cost: f64,
    /// Total completion tokens.
    pub total_completion_tokens: u64,
    /// Estimated completion cost.
    pub total_completion_cost: f64,
    /// Total prompt plus completion tokens.
    pub total_tokens: u64,
    /// Estimated total cost.
    pub total_cost: f64,
    /// Number of completions included.
    pub entry_count: usize,
    /// Per-model usage breakdown.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub by_model: BTreeMap<String, ModelUsageStats>,
}

/// Usage summary for one model id.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ModelUsageStats {
    /// Model id.
    pub model: String,
    /// Prompt tokens for this model.
    pub prompt_tokens: u64,
    /// Completion tokens for this model.
    pub completion_tokens: u64,
    /// Total tokens for this model.
    pub total_tokens: u64,
    /// Estimated cost for this model.
    pub cost: f64,
    /// Number of invocations for this model.
    pub invocations: usize,
    /// Average total tokens per invocation.
    pub average_tokens_per_invocation: f64,
}

/// Complete durable history for an agent run.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct AgentHistory {
    /// Step records in chronological order.
    #[serde(default)]
    pub items: Vec<AgentHistoryItem>,
    /// Optional usage summary.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<UsageSummary>,
    /// Latest compacted memory summary.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compacted_memory: Option<String>,
    /// Number of compactions performed.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub compaction_count: usize,
    /// Step number of the last compaction.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_compaction_step: Option<usize>,
}

/// Replay rematch result for one historical action.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ActionReplayRematch {
    /// Action after any index remapping.
    pub action: BrowserAction,
    /// Original element index from history, when the action had one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub original_index: Option<u32>,
    /// Current element index after rematching, when available.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rematched_index: Option<u32>,
    /// DOM rematch details.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub match_result: Option<DomInteractedElementMatch>,
    /// True when replay changed the action's element index.
    #[serde(default)]
    pub changed: bool,
}

/// Ordered plan of historical actions remapped for the current DOM.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct AgentHistoryReplayPlan {
    /// Planned replay actions.
    #[serde(default)]
    pub actions: Vec<AgentHistoryReplayPlanItem>,
}

/// One replay-plan item.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct AgentHistoryReplayPlanItem {
    /// Original history step index.
    pub step_index: usize,
    /// Original action index within that step.
    pub action_index: usize,
    /// Historical action before remapping.
    pub original_action: BrowserAction,
    /// Action that should be executed now.
    pub remapped_action: BrowserAction,
    /// Rematch details for the action.
    pub rematch: ActionReplayRematch,
}

/// Error produced while building a replay plan.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct AgentHistoryReplayPlanError {
    /// History step whose action failed to rematch.
    pub step_index: usize,
    /// Action index within the step.
    pub action_index: usize,
    /// Original action that could not be planned.
    pub original_action: Box<BrowserAction>,
    /// Historical element index, when the action had one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub original_index: Option<u32>,
    /// DOM rematch failure details.
    pub failure: Box<DomInteractedElementMatchFailure>,
}

impl std::fmt::Display for AgentHistoryReplayPlanError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "failed to rematch replay action {} in step {}: {}",
            self.action_index, self.step_index, self.failure.message
        )
    }
}

impl std::error::Error for AgentHistoryReplayPlanError {}

/// Result of executing a replay plan.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct AgentHistoryReplayExecution {
    /// Executed replay items.
    pub items: Vec<AgentHistoryReplayExecutionItem>,
    /// Stop reason, if replay ended before all actions ran.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stop: Option<AgentHistoryReplayStop>,
}

/// One executed replay action.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct AgentHistoryReplayExecutionItem {
    /// Original history step index.
    pub step_index: usize,
    /// Original action index within the step.
    pub action_index: usize,
    /// Historical action before remapping.
    pub original_action: BrowserAction,
    /// Action actually executed.
    pub executed_action: BrowserAction,
    /// Rematch details.
    pub rematch: ActionReplayRematch,
    /// Browser result from executing the action.
    pub result: ActionResult,
}

/// Full replay run with current state, plan, and execution result.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct AgentHistoryReplayRun {
    /// Browser state used as the initial replay target.
    pub current_state: BrowserStateSummary,
    /// Replay plan built from history.
    pub plan: AgentHistoryReplayPlan,
    /// Replay execution result.
    pub execution: AgentHistoryReplayExecution,
}

/// Error returned by the higher-level replay flow.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AgentHistoryReplayRunError {
    /// Current state capture failed before planning.
    CurrentState {
        /// Error message from state capture.
        message: String,
    },
    /// Replay planning failed.
    Plan {
        /// Detailed replay plan error.
        error: Box<AgentHistoryReplayPlanError>,
    },
}

impl std::fmt::Display for AgentHistoryReplayRunError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::CurrentState { message } => {
                write!(
                    formatter,
                    "failed to capture current browser state: {message}"
                )
            }
            Self::Plan { error } => write!(formatter, "{error}"),
        }
    }
}

impl std::error::Error for AgentHistoryReplayRunError {}

/// Reason replay stopped before exhausting the plan.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct AgentHistoryReplayStop {
    /// Step index associated with the stop.
    pub step_index: usize,
    /// Action index associated with the stop.
    pub action_index: usize,
    /// Stop reason.
    pub reason: AgentHistoryReplayStopReason,
    /// Optional diagnostic message.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diagnostic: Option<String>,
}

/// Categories of replay termination.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AgentHistoryReplayStopReason {
    /// A `done` action appeared after a prior action in the replay sequence.
    DoneAfterPriorAction,
    /// A `done` action executed.
    Done,
    /// An executed action returned an error.
    Error,
    /// The page URL changed after a non-terminating action.
    PageChanged,
    /// A navigation-like action executed.
    TerminatingAction,
}

#[derive(Debug, Clone)]
pub(crate) struct HistoricalReplayAction {
    pub(crate) step_index: usize,
    pub(crate) action_index: usize,
    pub(crate) action: BrowserAction,
    pub(crate) interacted_element: Option<DomInteractedElement>,
}

pub(crate) fn historical_replay_actions(history: &AgentHistory) -> Vec<HistoricalReplayAction> {
    let mut actions = Vec::new();
    for (step_index, item) in history.items.iter().enumerate() {
        let Some(output) = item.model_output.as_ref() else {
            continue;
        };
        for (action_index, action) in output.action.iter().enumerate() {
            let interacted_element = action
                .interacted_element_index()
                .and_then(|index| item.state.dom_state.selector_map.get(&index))
                .map(DomInteractedElement::from_element);
            actions.push(HistoricalReplayAction {
                step_index,
                action_index,
                action: action.clone(),
                interacted_element,
            });
        }
    }
    actions
}

/// Builds a replay plan by rematching history actions against the current DOM.
pub fn build_history_replay_plan(
    history: &AgentHistory,
    current_dom: &SerializedDomState,
) -> Result<AgentHistoryReplayPlan, AgentHistoryReplayPlanError> {
    let mut actions = Vec::new();
    for historical in historical_replay_actions(history) {
        let rematch = rematch_action_for_replay(
            &historical.action,
            historical.interacted_element.as_ref(),
            current_dom,
        )
        .map_err(|failure| AgentHistoryReplayPlanError {
            step_index: historical.step_index,
            action_index: historical.action_index,
            original_action: Box::new(historical.action.clone()),
            original_index: historical.action.interacted_element_index(),
            failure: Box::new(failure),
        })?;
        actions.push(AgentHistoryReplayPlanItem {
            step_index: historical.step_index,
            action_index: historical.action_index,
            original_action: historical.action,
            remapped_action: rematch.action.clone(),
            rematch,
        });
    }

    Ok(AgentHistoryReplayPlan { actions })
}

/// Rematches a single action's historical element index against the current DOM.
pub fn rematch_action_for_replay(
    action: &BrowserAction,
    interacted_element: Option<&DomInteractedElement>,
    current_dom: &SerializedDomState,
) -> Result<ActionReplayRematch, DomInteractedElementMatchFailure> {
    let Some(original_index) = action.interacted_element_index() else {
        return Ok(ActionReplayRematch {
            action: action.clone(),
            original_index: None,
            rematched_index: None,
            match_result: None,
            changed: false,
        });
    };
    let Some(interacted_element) = interacted_element else {
        return Ok(ActionReplayRematch {
            action: action.clone(),
            original_index: Some(original_index),
            rematched_index: None,
            match_result: None,
            changed: false,
        });
    };

    let match_result = interacted_element.rematch(current_dom)?;
    let rematched_index = match_result.index;
    let changed = rematched_index != original_index;
    Ok(ActionReplayRematch {
        action: action_with_interacted_index(action, rematched_index),
        original_index: Some(original_index),
        rematched_index: Some(rematched_index),
        match_result: Some(match_result),
        changed,
    })
}

impl AgentHistory {
    /// Returns final extracted content from the latest result, if any.
    #[must_use]
    pub fn final_result(&self) -> Option<&str> {
        self.last_result()
            .and_then(|result| result.extracted_content.as_deref())
    }

    /// Returns true when the latest action result is terminal.
    #[must_use]
    pub fn is_done(&self) -> bool {
        self.last_result().is_some_and(|result| result.is_done)
    }

    /// Returns final success when the latest result is terminal.
    #[must_use]
    pub fn is_successful(&self) -> Option<bool> {
        self.last_result()
            .filter(|result| result.is_done)
            .and_then(|result| result.success)
    }

    /// Returns one optional error per step.
    #[must_use]
    pub fn errors(&self) -> Vec<Option<&str>> {
        self.items
            .iter()
            .map(|item| {
                item.result
                    .iter()
                    .find_map(|result| result.error.as_deref())
            })
            .collect()
    }

    /// Returns true if any step contains an error.
    #[must_use]
    pub fn has_errors(&self) -> bool {
        self.errors().iter().any(Option::is_some)
    }

    #[must_use]
    /// Returns the latest judgement result, if any.
    pub fn judgement(&self) -> Option<&JudgementResult> {
        self.last_result()
            .and_then(|result| result.judgement.as_ref())
    }

    #[must_use]
    /// Returns true when the latest result has a judgement.
    pub fn is_judged(&self) -> bool {
        self.judgement().is_some()
    }

    #[must_use]
    /// Returns the latest judgement verdict.
    pub fn is_validated(&self) -> Option<bool> {
        self.judgement().map(|judgement| judgement.verdict)
    }

    #[must_use]
    /// Sums step durations from metadata.
    pub fn total_duration_seconds(&self) -> f64 {
        self.items
            .iter()
            .filter_map(|item| item.metadata.as_ref())
            .map(StepMetadata::duration_seconds)
            .sum()
    }

    #[must_use]
    /// Returns the number of history steps.
    pub fn number_of_steps(&self) -> usize {
        self.items.len()
    }

    #[must_use]
    /// Returns URLs observed at each step.
    pub fn urls(&self) -> Vec<&str> {
        self.items
            .iter()
            .map(|item| item.state.url.as_str())
            .collect()
    }

    #[must_use]
    /// Returns screenshots from all or the last `n_last` steps.
    pub fn screenshots(
        &self,
        n_last: Option<usize>,
        return_none_if_not_screenshot: bool,
    ) -> Vec<Option<&str>> {
        if n_last == Some(0) {
            return Vec::new();
        }

        let items = if let Some(n_last) = n_last {
            let start = self.items.len().saturating_sub(n_last);
            &self.items[start..]
        } else {
            &self.items
        };

        items
            .iter()
            .filter_map(|item| match item.state.screenshot.as_deref() {
                Some(screenshot) if !screenshot.is_empty() => Some(Some(screenshot)),
                _ if return_none_if_not_screenshot => Some(None),
                _ => None,
            })
            .collect()
    }

    #[must_use]
    /// Returns all action results across all steps.
    pub fn action_results(&self) -> Vec<&ActionResult> {
        self.items
            .iter()
            .flat_map(|item| item.result.iter())
            .collect()
    }

    #[must_use]
    /// Returns extracted content from all action results.
    pub fn extracted_content(&self) -> Vec<&str> {
        self.action_results()
            .into_iter()
            .filter_map(|result| result.extracted_content.as_deref())
            .collect()
    }

    #[must_use]
    /// Returns the latest model action serialized as JSON.
    pub fn last_action(&self) -> Option<Value> {
        self.items
            .last()
            .and_then(|item| item.model_output.as_ref())
            .and_then(|output| output.action.last())
            .and_then(|action| serde_json::to_value(action).ok())
    }

    #[must_use]
    /// Returns all model outputs in history order.
    pub fn model_outputs(&self) -> Vec<&AgentOutput> {
        self.items
            .iter()
            .filter_map(|item| item.model_output.as_ref())
            .collect()
    }

    #[must_use]
    /// Returns effective model thinking/current-state blocks.
    pub fn model_thoughts(&self) -> Vec<AgentCurrentState> {
        self.model_outputs()
            .into_iter()
            .map(AgentOutput::current_brain)
            .collect()
    }

    #[must_use]
    /// Returns model actions serialized with interacted-element snapshots.
    pub fn model_actions(&self) -> Vec<Value> {
        self.items
            .iter()
            .flat_map(|item| {
                item.model_output
                    .as_ref()
                    .map(|output| {
                        output
                            .action
                            .iter()
                            .filter_map(|action| {
                                action_value_with_interacted_element(action, &item.state)
                            })
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default()
            })
            .collect()
    }

    #[must_use]
    /// Returns per-step action history with action results embedded.
    pub fn action_history(&self) -> Vec<Vec<Value>> {
        self.items
            .iter()
            .map(|item| {
                let Some(output) = item.model_output.as_ref() else {
                    return Vec::new();
                };

                output
                    .action
                    .iter()
                    .zip(item.result.iter())
                    .filter_map(|(action, result)| {
                        let mut action_output =
                            action_value_with_interacted_element(action, &item.state)?;
                        if let Value::Object(attributes) = &mut action_output {
                            attributes.insert(
                                "result".to_owned(),
                                result
                                    .long_term_memory
                                    .as_ref()
                                    .map(|memory| Value::String(memory.clone()))
                                    .unwrap_or(Value::Null),
                            );
                        }
                        Some(action_output)
                    })
                    .collect()
            })
            .collect()
    }

    /// Builds a replay plan from this history against a current DOM snapshot.
    pub fn replay_plan(
        &self,
        current_dom: &SerializedDomState,
    ) -> Result<AgentHistoryReplayPlan, AgentHistoryReplayPlanError> {
        build_history_replay_plan(self, current_dom)
    }

    #[must_use]
    /// Returns serialized model actions whose names are in `include`.
    pub fn model_actions_filtered(&self, include: &[&str]) -> Vec<Value> {
        self.items
            .iter()
            .filter_map(|item| item.model_output.as_ref())
            .flat_map(|output| output.action.iter())
            .filter(|action| include.contains(&action.name()))
            .filter_map(|action| serde_json::to_value(action).ok())
            .collect()
    }

    #[must_use]
    /// Returns action names in history order.
    pub fn action_names(&self) -> Vec<&'static str> {
        self.items
            .iter()
            .filter_map(|item| item.model_output.as_ref())
            .flat_map(|output| output.action.iter())
            .map(BrowserAction::name)
            .collect()
    }

    fn last_result(&self) -> Option<&ActionResult> {
        self.items.last().and_then(|item| item.result.last())
    }
}

fn action_value_with_interacted_element(
    action: &BrowserAction,
    state: &BrowserStateSummary,
) -> Option<Value> {
    let mut action_output = serde_json::to_value(action).ok()?;
    if let Value::Object(attributes) = &mut action_output {
        let interacted_element = action
            .interacted_element_index()
            .and_then(|index| state.dom_state.selector_map.get(&index))
            .map(DomInteractedElement::from_element)
            .and_then(|element| serde_json::to_value(element).ok())
            .unwrap_or(Value::Null);
        attributes.insert("interacted_element".to_owned(), interacted_element);
    }
    Some(action_output)
}

fn action_with_interacted_index(action: &BrowserAction, index: u32) -> BrowserAction {
    action
        .with_interacted_element_index(index)
        .unwrap_or_else(|| action.clone())
}
