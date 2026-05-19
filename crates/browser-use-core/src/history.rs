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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct AgentOutput {
    #[serde(default, skip_serializing_if = "AgentCurrentState::is_empty")]
    pub current_state: AgentCurrentState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thinking: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evaluation_previous_goal: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memory: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_goal: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_plan_item: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan_update: Option<Vec<String>>,
    pub action: Vec<BrowserAction>,
}

impl AgentOutput {
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

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
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

impl AgentCurrentState {
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.thinking.is_none()
            && self.evaluation_previous_goal.is_none()
            && self.memory.is_none()
            && self.next_goal.is_none()
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct JudgementResult {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<String>,
    pub verdict: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure_reason: Option<String>,
    #[serde(default)]
    pub impossible_task: bool,
    #[serde(default)]
    pub reached_captcha: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct MessageCompactionOutput {
    pub summary: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct ActionResult {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extracted_content: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub judgement: Option<JudgementResult>,
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
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attachments: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub images: Vec<Value>,
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
pub struct UsageSummary {
    pub total_prompt_tokens: u64,
    pub total_prompt_cost: f64,
    pub total_prompt_cached_tokens: u64,
    pub total_prompt_cached_cost: f64,
    pub total_completion_tokens: u64,
    pub total_completion_cost: f64,
    pub total_tokens: u64,
    pub total_cost: f64,
    pub entry_count: usize,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub by_model: BTreeMap<String, ModelUsageStats>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ModelUsageStats {
    pub model: String,
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub total_tokens: u64,
    pub cost: f64,
    pub invocations: usize,
    pub average_tokens_per_invocation: f64,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct AgentHistory {
    #[serde(default)]
    pub items: Vec<AgentHistoryItem>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<UsageSummary>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compacted_memory: Option<String>,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub compaction_count: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_compaction_step: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ActionReplayRematch {
    pub action: BrowserAction,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub original_index: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rematched_index: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub match_result: Option<DomInteractedElementMatch>,
    #[serde(default)]
    pub changed: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct AgentHistoryReplayPlan {
    #[serde(default)]
    pub actions: Vec<AgentHistoryReplayPlanItem>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct AgentHistoryReplayPlanItem {
    pub step_index: usize,
    pub action_index: usize,
    pub original_action: BrowserAction,
    pub remapped_action: BrowserAction,
    pub rematch: ActionReplayRematch,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct AgentHistoryReplayPlanError {
    pub step_index: usize,
    pub action_index: usize,
    pub original_action: Box<BrowserAction>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub original_index: Option<u32>,
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct AgentHistoryReplayExecution {
    pub items: Vec<AgentHistoryReplayExecutionItem>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stop: Option<AgentHistoryReplayStop>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct AgentHistoryReplayExecutionItem {
    pub step_index: usize,
    pub action_index: usize,
    pub original_action: BrowserAction,
    pub executed_action: BrowserAction,
    pub rematch: ActionReplayRematch,
    pub result: ActionResult,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct AgentHistoryReplayRun {
    pub current_state: BrowserStateSummary,
    pub plan: AgentHistoryReplayPlan,
    pub execution: AgentHistoryReplayExecution,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AgentHistoryReplayRunError {
    CurrentState {
        message: String,
    },
    Plan {
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct AgentHistoryReplayStop {
    pub step_index: usize,
    pub action_index: usize,
    pub reason: AgentHistoryReplayStopReason,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diagnostic: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AgentHistoryReplayStopReason {
    DoneAfterPriorAction,
    Done,
    Error,
    PageChanged,
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
    #[must_use]
    pub fn final_result(&self) -> Option<&str> {
        self.last_result()
            .and_then(|result| result.extracted_content.as_deref())
    }

    #[must_use]
    pub fn is_done(&self) -> bool {
        self.last_result().is_some_and(|result| result.is_done)
    }

    #[must_use]
    pub fn is_successful(&self) -> Option<bool> {
        self.last_result()
            .filter(|result| result.is_done)
            .and_then(|result| result.success)
    }

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

    #[must_use]
    pub fn has_errors(&self) -> bool {
        self.errors().iter().any(Option::is_some)
    }

    #[must_use]
    pub fn judgement(&self) -> Option<&JudgementResult> {
        self.last_result()
            .and_then(|result| result.judgement.as_ref())
    }

    #[must_use]
    pub fn is_judged(&self) -> bool {
        self.judgement().is_some()
    }

    #[must_use]
    pub fn is_validated(&self) -> Option<bool> {
        self.judgement().map(|judgement| judgement.verdict)
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
    pub fn last_action(&self) -> Option<Value> {
        self.items
            .last()
            .and_then(|item| item.model_output.as_ref())
            .and_then(|output| output.action.last())
            .and_then(|action| serde_json::to_value(action).ok())
    }

    #[must_use]
    pub fn model_outputs(&self) -> Vec<&AgentOutput> {
        self.items
            .iter()
            .filter_map(|item| item.model_output.as_ref())
            .collect()
    }

    #[must_use]
    pub fn model_thoughts(&self) -> Vec<AgentCurrentState> {
        self.model_outputs()
            .into_iter()
            .map(AgentOutput::current_brain)
            .collect()
    }

    #[must_use]
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

    pub fn replay_plan(
        &self,
        current_dom: &SerializedDomState,
    ) -> Result<AgentHistoryReplayPlan, AgentHistoryReplayPlanError> {
        build_history_replay_plan(self, current_dom)
    }

    #[must_use]
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
