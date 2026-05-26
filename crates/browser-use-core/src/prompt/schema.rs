use crate::{AgentOutput, AgentSettings, BrowserAction, JudgementResult, MessageCompactionOutput};
use serde_json::Value;
use std::collections::BTreeSet;

pub(crate) fn schema_for_agent_output() -> Value {
    schema_to_compat_value(schemars::schema_for!(AgentOutput))
}

pub(super) fn schema_for_judgement_result() -> Value {
    schema_to_compat_value(schemars::schema_for!(JudgementResult))
}

pub(super) fn schema_for_message_compaction_output() -> Value {
    schema_to_compat_value(schemars::schema_for!(MessageCompactionOutput))
}

/// Converts a generated JSON Schema into the compact browser-use wire shape.
///
/// Rust doc comments are source documentation first, but `schemars` also copies
/// them into JSON Schema `description` metadata. Browser-use-rs compares these
/// schemas against upstream-compatible fixtures, so this helper preserves the
/// small set of descriptions that were already part of that contract and
/// removes newly introduced doc metadata before schemas reach LLM prompts or
/// MCP manifests.
pub fn schema_to_compat_value<T>(schema: T) -> Value
where
    T: serde::Serialize,
{
    let mut value = serde_json::to_value(schema).unwrap_or(Value::Null);
    normalize_compat_schema(&mut value, false);
    value
}

fn normalize_compat_schema(value: &mut Value, inside_properties_map: bool) {
    match value {
        Value::Object(entries) => {
            let removed_metadata_only_description = if inside_properties_map {
                // A key named `description` inside `properties` is a modeled
                // JSON field, not schema metadata. Removing it would break the
                // `NoParamsAction.description` compatibility fixture.
                false
            } else {
                remove_non_contract_description(entries)
            };

            for (key, entry) in entries.iter_mut() {
                normalize_compat_schema(entry, key == "properties");
            }

            if let Some(ref_schema) = single_ref_all_of(entries) {
                // `schemars` wraps `$ref` in `allOf` when a field has doc
                // metadata. After metadata stripping, the wrapper is just noise
                // and would drift the upstream-compatible schema.
                *value = ref_schema;
                return;
            }

            fold_doc_only_unit_enum(entries);

            if removed_metadata_only_description && entries.is_empty() {
                // `serde_json::Value` intentionally has an unconstrained schema.
                // If its only generated content was doc metadata, the compact
                // JSON Schema representation is boolean `true`.
                *value = Value::Bool(true);
            }
        }
        Value::Array(items) => {
            for item in items {
                normalize_compat_schema(item, false);
            }
        }
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {}
    }
}

fn remove_non_contract_description(entries: &mut serde_json::Map<String, Value>) -> bool {
    let should_remove = matches!(
        entries.get("description"),
        Some(Value::String(description)) if !is_contract_schema_description(description)
    );
    if should_remove {
        entries.remove("description");
    }
    should_remove
}

fn single_ref_all_of(entries: &serde_json::Map<String, Value>) -> Option<Value> {
    if entries.len() != 1 {
        return None;
    }
    let Some(Value::Array(items)) = entries.get("allOf") else {
        return None;
    };
    let [Value::Object(item)] = items.as_slice() else {
        return None;
    };
    if item.len() == 1 && item.contains_key("$ref") {
        return Some(Value::Object(item.clone()));
    }
    None
}

fn is_contract_schema_description(description: &str) -> bool {
    matches!(
        description,
        "Browser-use action model: each serialized action is a one-key object."
            | "Free-text or schema-guided page extraction."
            | "CSS selector lookup against the page."
            | "Text or regex search against page content."
            | "Browser state summary compatible with the browser-use agent step contract."
            | "A compact node reference addressable by an action index."
            | "Tree-shaped DOM node used for browser-use's evaluation/judge representation."
            | "Node kind used by the evaluation-focused DOM tree serializer."
            | "Compact page-shape statistics rendered into the agent prompt."
            | "Viewport-relative integer bounds for an indexed element."
            | "A network request that is still in flight when browser state is captured."
            | "Viewport and scroll metrics used to help the agent reason about page shape."
            | "Pagination affordance detected from the current page."
            | "Serialized DOM state in the form the agent consumes."
            | "Information about an open tab or page target."
    )
}

fn fold_doc_only_unit_enum(entries: &mut serde_json::Map<String, Value>) {
    let Some(Value::Array(variants)) = entries.get("oneOf") else {
        return;
    };

    // Documented unit enum variants serialize as `oneOf` with one single-value
    // enum per variant. The pre-doc contract used one flat enum list, so fold
    // only that exact doc-only pattern.
    let mut enum_values = Vec::with_capacity(variants.len());
    for variant in variants {
        let Value::Object(variant) = variant else {
            return;
        };
        if variant.len() != 2 || variant.get("type") != Some(&Value::String("string".to_owned())) {
            return;
        }
        let Some(Value::Array(values)) = variant.get("enum") else {
            return;
        };
        let [Value::String(value)] = values.as_slice() else {
            return;
        };
        enum_values.push(Value::String(value.clone()));
    }

    entries.remove("oneOf");
    entries.insert("enum".to_owned(), Value::Array(enum_values));
    entries.insert("type".to_owned(), Value::String("string".to_owned()));
}

pub(crate) fn schema_for_agent_output_with_settings(settings: &AgentSettings) -> Value {
    let mut schema = schema_for_agent_output();
    let mut remove_fields = vec!["current_state"];

    if !settings.use_thinking || settings.flash_mode {
        remove_fields.push("thinking");
    }
    if settings.flash_mode {
        remove_fields.extend([
            "current_state",
            "evaluation_previous_goal",
            "next_goal",
            "current_plan_item",
            "plan_update",
        ]);
    }

    if !remove_fields.is_empty() {
        prune_schema_properties(&mut schema, &remove_fields);
    }

    let excluded_actions = normalized_schema_excluded_actions(settings);
    if !excluded_actions.is_empty() {
        exclude_schema_actions(&mut schema, &excluded_actions);
    }

    if settings.flash_mode {
        require_schema_properties(&mut schema, &["memory", "action"]);
    } else {
        require_schema_properties(
            &mut schema,
            &["evaluation_previous_goal", "memory", "next_goal", "action"],
        );
    }
    require_non_empty_actions(&mut schema);

    schema
}

pub(crate) fn schema_for_final_response_after_failure(settings: &AgentSettings) -> Value {
    let mut schema = schema_for_agent_output_with_settings(settings);
    restrict_schema_actions_to_done(&mut schema);
    schema
}

fn normalized_excluded_actions(actions: &[String]) -> BTreeSet<String> {
    actions
        .iter()
        .map(|action| action.trim().replace('-', "_").to_ascii_lowercase())
        .filter(|action| !action.is_empty() && action != "done")
        .collect()
}

fn normalized_schema_excluded_actions(settings: &AgentSettings) -> BTreeSet<String> {
    let mut excluded_actions = normalized_excluded_actions(&settings.excluded_actions);
    if !settings.use_vision.allows_screenshot_action() {
        excluded_actions.insert("screenshot".to_owned());
    }
    excluded_actions
}

pub(crate) fn excluded_action_error(
    actions: &[BrowserAction],
    settings: &AgentSettings,
) -> Option<String> {
    if !settings.use_vision.allows_screenshot_action()
        && actions
            .iter()
            .any(|action| matches!(action, BrowserAction::Screenshot(_)))
    {
        return Some(
            "model output requested screenshot action, but AgentSettings.use_vision must be \"auto\""
                .to_owned(),
        );
    }

    let excluded_actions = normalized_excluded_actions(&settings.excluded_actions);
    if excluded_actions.is_empty() {
        return None;
    }

    actions
        .iter()
        .map(BrowserAction::name)
        .find(|name| excluded_actions.contains(*name))
        .map(|name| {
            format!(
                "model output requested excluded action `{name}`; remove it from the action list or update AgentSettings.excluded_actions"
            )
        })
}

fn exclude_schema_actions(schema: &mut Value, excluded_actions: &BTreeSet<String>) {
    for pointer in [
        "/$defs/BrowserAction/oneOf",
        "/$defs/BrowserAction/anyOf",
        "/definitions/BrowserAction/oneOf",
        "/definitions/BrowserAction/anyOf",
    ] {
        if let Some(actions) = schema.pointer_mut(pointer).and_then(Value::as_array_mut) {
            actions.retain(|action| {
                schema_variant_action_name(action)
                    .is_none_or(|name| !excluded_actions.contains(name))
            });
        }
    }
}

fn restrict_schema_actions_to_done(schema: &mut Value) {
    for pointer in [
        "/$defs/BrowserAction/oneOf",
        "/$defs/BrowserAction/anyOf",
        "/definitions/BrowserAction/oneOf",
        "/definitions/BrowserAction/anyOf",
    ] {
        if let Some(actions) = schema.pointer_mut(pointer).and_then(Value::as_array_mut) {
            actions.retain(schema_variant_is_done_action);
        }
    }
}

fn schema_variant_is_done_action(value: &Value) -> bool {
    schema_variant_action_name(value) == Some("done")
}

pub(crate) fn schema_variant_action_name(value: &Value) -> Option<&str> {
    let required_action_name = value
        .get("required")
        .and_then(Value::as_array)
        .and_then(|fields| fields.iter().find_map(Value::as_str));
    required_action_name.or_else(|| {
        value
            .get("properties")
            .and_then(Value::as_object)
            .and_then(|properties| properties.keys().next().map(String::as_str))
    })
}

fn prune_schema_properties(schema: &mut Value, remove_fields: &[&str]) {
    if let Some(properties) = schema.get_mut("properties").and_then(Value::as_object_mut) {
        for field in remove_fields {
            properties.remove(*field);
        }
    }

    if let Some(required) = schema.get_mut("required").and_then(Value::as_array_mut) {
        required.retain(|value| {
            value
                .as_str()
                .is_none_or(|field| !remove_fields.contains(&field))
        });
    }
}

fn require_schema_properties(schema: &mut Value, fields: &[&str]) {
    schema["required"] = Value::Array(
        fields
            .iter()
            .map(|field| Value::String((*field).to_owned()))
            .collect(),
    );
}

fn require_non_empty_actions(schema: &mut Value) {
    if let Some(action) = schema
        .get_mut("properties")
        .and_then(Value::as_object_mut)
        .and_then(|properties| properties.get_mut("action"))
        .and_then(Value::as_object_mut)
    {
        action.insert("minItems".to_owned(), Value::from(1));
    }
}
