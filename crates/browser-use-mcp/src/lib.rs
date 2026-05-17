//! MCP bridge contracts for browser-use-rs.

use browser_use_core::{ActionResult, AgentHistory};
use browser_use_dom::BrowserStateSummary;
use browser_use_tools::BrowserAction;
use schemars::{JsonSchema, schema_for};
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const STATE_TOOL_NAME: &str = "browser_use_state";
pub const ACTIONS_TOOL_NAME: &str = "browser_use_actions";
pub const AGENT_TOOL_NAME: &str = "browser_use_agent";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct McpToolContract {
    pub name: String,
    pub description: String,
    #[serde(rename = "inputSchema")]
    pub input_schema: Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct StateToolInput {
    pub url: String,
    #[serde(default = "default_true")]
    pub screenshot: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ActionsToolInput {
    pub url: String,
    #[serde(default)]
    pub actions: Vec<BrowserAction>,
    #[serde(default = "default_true")]
    pub screenshot: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct AgentToolInput {
    pub url: String,
    pub task: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
    #[serde(default = "default_max_steps")]
    pub max_steps: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct StateToolOutput {
    pub state: BrowserStateSummary,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ActionsToolOutput {
    pub results: Vec<ActionResult>,
    pub state: BrowserStateSummary,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct AgentToolOutput {
    pub history: AgentHistory,
}

fn default_true() -> bool {
    true
}

fn default_max_steps() -> usize {
    10
}

#[must_use]
pub fn tool_manifest() -> Vec<McpToolContract> {
    vec![
        tool_contract::<StateToolInput>(
            STATE_TOOL_NAME,
            "Launch a browser, navigate to a URL, and return browser-use state.",
        ),
        tool_contract::<ActionsToolInput>(
            ACTIONS_TOOL_NAME,
            "Launch a browser, run browser-use actions, and return action results plus final state.",
        ),
        tool_contract::<AgentToolInput>(
            AGENT_TOOL_NAME,
            "Launch a browser, run a bounded browser-use agent task, and return agent history.",
        ),
    ]
}

#[must_use]
pub fn tool_manifest_json() -> Value {
    serde_json::to_value(tool_manifest()).unwrap_or(Value::Null)
}

fn tool_contract<T>(name: &str, description: &str) -> McpToolContract
where
    T: JsonSchema,
{
    McpToolContract {
        name: name.to_owned(),
        description: description.to_owned(),
        input_schema: serde_json::to_value(schema_for!(T)).unwrap_or(Value::Null),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_exposes_expected_tools() {
        let names: Vec<String> = tool_manifest().into_iter().map(|tool| tool.name).collect();

        assert_eq!(
            names,
            vec![STATE_TOOL_NAME, ACTIONS_TOOL_NAME, AGENT_TOOL_NAME]
        );
    }

    #[test]
    fn actions_tool_schema_exposes_action_array() {
        let manifest = tool_manifest();
        let actions_tool = manifest
            .iter()
            .find(|tool| tool.name == ACTIONS_TOOL_NAME)
            .expect("actions tool");
        let schema_text = serde_json::to_string(&actions_tool.input_schema).expect("schema text");

        assert!(schema_text.contains("actions"));
        assert!(schema_text.contains("array"));
    }

    #[test]
    fn agent_tool_keeps_provider_secret_out_of_tool_input() {
        let schema = serde_json::to_value(schema_for!(AgentToolInput)).expect("schema");
        let schema_text = serde_json::to_string(&schema).expect("schema text");

        assert!(!schema_text.contains("api_key"));
        assert!(schema_text.contains("model"));
        assert!(schema_text.contains("base_url"));
    }
}
