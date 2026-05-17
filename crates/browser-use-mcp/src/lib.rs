//! MCP bridge contracts for browser-use-rs.

use browser_use_core::{ActionResult, AgentHistory};
use browser_use_dom::BrowserStateSummary;
use browser_use_tools::BrowserAction;
use schemars::{JsonSchema, schema_for};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

pub const MCP_PROTOCOL_VERSION: &str = "2025-06-18";
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JsonRpcRequest {
    pub jsonrpc: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<Value>,
    pub method: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub params: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CallToolParams {
    pub name: String,
    #[serde(default)]
    pub arguments: Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct StateToolInput {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default = "default_true")]
    pub screenshot: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ActionsToolInput {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default)]
    pub actions: Vec<BrowserAction>,
    #[serde(default = "default_true")]
    pub screenshot: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct AgentToolInput {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
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

#[must_use]
pub fn initialize_result() -> Value {
    json!({
        "protocolVersion": MCP_PROTOCOL_VERSION,
        "capabilities": {
            "tools": {
                "listChanged": false
            }
        },
        "serverInfo": {
            "name": "browser-use-rs",
            "title": "browser-use-rs",
            "version": env!("CARGO_PKG_VERSION")
        },
        "instructions": "Use browser_use_state for page state, browser_use_actions for deterministic browser actions, and browser_use_agent for bounded agent runs."
    })
}

#[must_use]
pub fn tools_list_result() -> Value {
    json!({ "tools": tool_manifest() })
}

#[must_use]
pub fn tool_success_result(structured_content: Value) -> Value {
    let text = serde_json::to_string_pretty(&structured_content)
        .unwrap_or_else(|_| structured_content.to_string());
    json!({
        "content": [
            {
                "type": "text",
                "text": text
            }
        ],
        "structuredContent": structured_content,
        "isError": false
    })
}

#[must_use]
pub fn tool_error_result(message: impl Into<String>) -> Value {
    json!({
        "content": [
            {
                "type": "text",
                "text": message.into()
            }
        ],
        "isError": true
    })
}

#[must_use]
pub fn json_rpc_success(id: Value, result: Value) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": result
    })
}

#[must_use]
pub fn json_rpc_error(id: Option<Value>, code: i64, message: impl Into<String>) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id.unwrap_or(Value::Null),
        "error": {
            "code": code,
            "message": message.into()
        }
    })
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

    #[test]
    fn initialize_result_declares_tools_capability() {
        let result = initialize_result();

        assert_eq!(
            result.get("protocolVersion").and_then(Value::as_str),
            Some(MCP_PROTOCOL_VERSION)
        );
        assert!(result.pointer("/capabilities/tools").is_some());
    }

    #[test]
    fn tool_success_includes_structured_and_text_content() {
        let result = tool_success_result(json!({ "state": { "title": "EvalOps" } }));

        assert_eq!(result.get("isError").and_then(Value::as_bool), Some(false));
        assert_eq!(
            result
                .pointer("/structuredContent/state/title")
                .and_then(Value::as_str),
            Some("EvalOps")
        );
        assert!(
            result
                .pointer("/content/0/text")
                .and_then(Value::as_str)
                .expect("text")
                .contains("EvalOps")
        );
    }
}
