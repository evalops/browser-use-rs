use crate::{
    ActionResult, AgentHistory, AgentHistoryItem, AgentSettings, BrowserAction, BrowserStateSummary,
};

pub(crate) const MAX_PROMPT_CONTENT_CHARS: usize = 60_000;
const MAX_PROMPT_ERROR_CHARS: usize = 200;
const PROMPT_ERROR_EDGE_CHARS: usize = 100;

pub(crate) fn render_previous_results(
    history: &AgentHistory,
    max_history_items: Option<usize>,
) -> String {
    enum HistoryPromptEntry<'a> {
        Item(&'a AgentHistoryItem),
        Omitted(usize),
    }

    let total_items = history.items.len();
    let entries: Vec<HistoryPromptEntry<'_>> = match max_history_items {
        None => history.items.iter().map(HistoryPromptEntry::Item).collect(),
        Some(max_history_items) if total_items <= max_history_items => {
            history.items.iter().map(HistoryPromptEntry::Item).collect()
        }
        Some(0) => vec![HistoryPromptEntry::Omitted(total_items)],
        Some(max_history_items) => {
            let omitted_count = total_items - max_history_items;
            let recent_items_count = max_history_items - 1;
            let recent_start = total_items.saturating_sub(recent_items_count);
            let mut entries = vec![
                HistoryPromptEntry::Item(&history.items[0]),
                HistoryPromptEntry::Omitted(omitted_count),
            ];
            entries.extend(
                history
                    .items
                    .iter()
                    .skip(recent_start)
                    .map(HistoryPromptEntry::Item),
            );
            entries
        }
    };

    let mut rendered = Vec::new();
    if let Some(memory) = non_empty_prompt_text(history.compacted_memory.as_deref()) {
        rendered.push(render_compacted_memory(memory));
    }
    if history.items.is_empty() {
        rendered.push("Agent initialized".to_owned());
    }
    for entry in entries {
        match entry {
            HistoryPromptEntry::Item(item) => {
                if let Some(item_text) = render_history_item_for_prompt(item) {
                    rendered.push(item_text);
                }
            }
            HistoryPromptEntry::Omitted(omitted_count) if omitted_count > 0 => {
                rendered.push(format!(
                    "<sys>[... {omitted_count} previous steps omitted...]</sys>"
                ));
            }
            HistoryPromptEntry::Omitted(_) => {}
        }
    }

    truncate_prompt_content(rendered.join("\n"))
}

fn render_compacted_memory(memory: &str) -> String {
    format!(
        "<compacted_memory>\n\
         <!-- Summary of prior steps. Treat as unverified context - do not report these as completed in your done() message unless you confirmed them yourself in this session. -->\n\
         {memory}\n\
         </compacted_memory>"
    )
}

pub(crate) fn render_history_items_for_compaction(history: &AgentHistory) -> String {
    let mut rendered = if history.items.is_empty() {
        vec!["Agent initialized".to_owned()]
    } else {
        Vec::new()
    };
    for item in &history.items {
        if let Some(item_text) = render_history_item_for_prompt(item) {
            rendered.push(item_text);
        }
    }
    truncate_prompt_content(rendered.join("\n"))
}

fn render_history_item_for_prompt(item: &AgentHistoryItem) -> Option<String> {
    let mut content_parts = Vec::new();
    if let Some(output) = item.model_output.as_ref() {
        let brain = output.current_brain();
        if let Some(evaluation) = non_empty_prompt_text(brain.evaluation_previous_goal.as_deref()) {
            content_parts.push(evaluation.to_owned());
        }
        if let Some(memory) = non_empty_prompt_text(brain.memory.as_deref()) {
            content_parts.push(memory.to_owned());
        }
        if let Some(next_goal) = non_empty_prompt_text(brain.next_goal.as_deref()) {
            content_parts.push(next_goal.to_owned());
        }
    }
    if let Some(action_results) = render_action_results_for_prompt(&item.result) {
        content_parts.push(action_results);
    }

    (!content_parts.is_empty()).then(|| format!("<step>\n{}", content_parts.join("\n")))
}

fn render_action_results_for_prompt(results: &[ActionResult]) -> Option<String> {
    let mut lines = Vec::new();
    for result in results {
        if let Some(memory) = non_empty_prompt_text(result.long_term_memory.as_deref()) {
            lines.push(memory.to_owned());
        } else if !result.include_extracted_content_only_once
            && let Some(content) = non_empty_prompt_text(result.extracted_content.as_deref())
        {
            lines.push(content.to_owned());
        }

        if let Some(error) = non_empty_prompt_text(result.error.as_deref()) {
            lines.push(truncate_error_for_prompt(error));
        }
    }

    if lines.is_empty() {
        None
    } else {
        Some(truncate_prompt_content(format!(
            "Result\n{}",
            lines.join("\n")
        )))
    }
}

pub(crate) fn render_read_state_description(history: &AgentHistory) -> Option<String> {
    let latest = history.items.last()?;
    let mut blocks = Vec::new();
    for result in &latest.result {
        if result.include_extracted_content_only_once
            && let Some(extracted_content) =
                non_empty_prompt_text(result.extracted_content.as_deref())
        {
            let index = blocks.len();
            blocks.push(format!(
                "<read_state_{index}>\n{extracted_content}\n</read_state_{index}>"
            ));
        }
    }

    if blocks.is_empty() {
        None
    } else {
        Some(truncate_prompt_content(blocks.join("\n")))
    }
}

pub(super) fn non_empty_prompt_text(text: Option<&str>) -> Option<&str> {
    text.filter(|value| !value.is_empty())
}

pub(super) fn render_planning_context(
    history: &AgentHistory,
    settings: &AgentSettings,
) -> Option<String> {
    if !settings.enable_planning || settings.flash_mode {
        return None;
    }

    let steps_without_plan_update = history
        .items
        .iter()
        .rev()
        .take_while(|item| {
            item.model_output
                .as_ref()
                .and_then(|output| output.plan_update.as_ref())
                .is_none()
        })
        .count();
    let recent_failures = history
        .items
        .iter()
        .rev()
        .take_while(|item| item.result.iter().any(|result| result.error.is_some()))
        .count();

    let mut message = format!(
        "When useful, include `current_plan_item` and `plan_update` to keep multi-step work explicit. Replan after {} stalled/error steps; avoid exploring for more than {} steps without a plan update.",
        settings.planning_replan_on_stall, settings.planning_exploration_limit
    );

    if settings.planning_replan_on_stall > 0 && recent_failures >= settings.planning_replan_on_stall
    {
        message.push_str(
            " Recent steps have failed or stalled, so revise the plan before continuing.",
        );
    } else if settings.planning_exploration_limit > 0
        && steps_without_plan_update >= settings.planning_exploration_limit
    {
        message.push_str(" You have explored for several steps without updating the plan; provide a concise plan_update.");
    }

    Some(message)
}

pub(super) fn render_loop_awareness(
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

fn truncate_prompt_content(content: String) -> String {
    if content.chars().count() <= MAX_PROMPT_CONTENT_CHARS {
        return content;
    }

    let truncated = content
        .chars()
        .take(MAX_PROMPT_CONTENT_CHARS)
        .collect::<String>();
    format!("{truncated}\n... [Content truncated at 60k characters]")
}

fn truncate_error_for_prompt(error: &str) -> String {
    if error.chars().count() <= MAX_PROMPT_ERROR_CHARS {
        return error.to_owned();
    }

    let prefix = error
        .chars()
        .take(PROMPT_ERROR_EDGE_CHARS)
        .collect::<String>();
    let suffix = error
        .chars()
        .rev()
        .take(PROMPT_ERROR_EDGE_CHARS)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<String>();
    format!("{prefix}......{suffix}")
}

pub(crate) fn repeated_action_loop(history: &AgentHistory, window: usize) -> bool {
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
                .and_then(|output| action_sequence_similarity_signature(&output.action))
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

fn action_sequence_similarity_signature(actions: &[BrowserAction]) -> Option<String> {
    let signatures = actions
        .iter()
        .filter(|action| !matches!(action.name(), "wait" | "done" | "go_back"))
        .filter_map(action_similarity_signature)
        .collect::<Vec<_>>();
    if signatures.is_empty() {
        None
    } else {
        Some(signatures.join("||"))
    }
}
