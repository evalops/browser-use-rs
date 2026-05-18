//! MCP bridge contracts for browser-use-rs.

use std::path::PathBuf;

use browser_use_cdp::DevToolsEndpoint;
use browser_use_core::{ActionResult, AgentHistory, AgentHistoryReplayRun, AgentSettings};
use browser_use_dom::BrowserStateSummary;
use browser_use_tools::BrowserAction;
use schemars::{JsonSchema, schema_for};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

pub const MCP_PROTOCOL_VERSION: &str = "2025-06-18";
pub const STATE_TOOL_NAME: &str = "browser_use_state";
pub const ACTIONS_TOOL_NAME: &str = "browser_use_actions";
pub const REPLAY_TOOL_NAME: &str = "browser_use_replay";
pub const AGENT_TOOL_NAME: &str = "browser_use_agent";
pub const SESSION_TOOL_NAME: &str = "browser_use_session";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct McpToolContract {
    pub name: String,
    pub description: String,
    #[serde(rename = "inputSchema")]
    pub input_schema: Value,
    #[serde(
        rename = "outputSchema",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub output_schema: Option<Value>,
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ReplayToolInput {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    pub history: AgentHistory,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct AgentToolInput {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    pub task: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<AgentProvider>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub structured_output_mode: Option<AgentStructuredOutputMode>,
    #[serde(default = "default_max_steps")]
    pub max_steps: usize,
    #[serde(default, skip_serializing_if = "is_default_agent_settings")]
    pub settings: AgentSettings,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum AgentStructuredOutputMode {
    #[serde(rename = "json-schema", alias = "json_schema")]
    JsonSchema,
    #[serde(rename = "json-object", alias = "json_object")]
    JsonObject,
    #[serde(rename = "prompt-only", alias = "prompt_only")]
    PromptOnly,
    #[serde(rename = "tool-call", alias = "tool_call")]
    ToolCall,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum AgentProvider {
    #[serde(rename = "openai-compatible", alias = "openai")]
    OpenAiCompatible,
    #[serde(rename = "deepseek", alias = "deep-seek")]
    DeepSeek,
    #[serde(rename = "groq")]
    Groq,
    #[serde(rename = "cerebras")]
    Cerebras,
    #[serde(rename = "mistral")]
    Mistral,
    #[serde(rename = "openrouter", alias = "open-router")]
    OpenRouter,
    #[serde(rename = "vercel", alias = "ai-gateway", alias = "vercel-ai-gateway")]
    Vercel,
    #[serde(rename = "anthropic")]
    Anthropic,
    #[serde(rename = "gemini", alias = "google")]
    Gemini,
    #[serde(rename = "ollama", alias = "local")]
    Ollama,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum SessionOperation {
    #[serde(rename = "start")]
    Start,
    #[serde(rename = "stop")]
    Stop,
    #[serde(rename = "list")]
    List,
    #[serde(rename = "cleanup")]
    Cleanup,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SessionToolInput {
    pub operation: SessionOperation,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default = "default_true")]
    pub screenshot: bool,
    #[serde(default)]
    pub force: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum SessionStatus {
    #[serde(rename = "running")]
    Running,
    #[serde(rename = "stale")]
    Stale,
    #[serde(rename = "stopped")]
    Stopped,
    #[serde(rename = "unknown")]
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum SessionCleanupAction {
    #[serde(rename = "removed")]
    Removed,
    #[serde(rename = "stopped")]
    Stopped,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SessionCleanupRecord {
    pub action: SessionCleanupAction,
    pub session: SessionRecord,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SessionRecord {
    pub id: String,
    pub endpoint: DevToolsEndpoint,
    pub user_data_dir: PathBuf,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub process_id: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<SessionStatus>,
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
pub struct ReplayToolOutput {
    pub replay: AgentHistoryReplayRun,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct AgentToolOutput {
    pub history: AgentHistory,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SessionToolOutput {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<SessionRecord>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sessions: Vec<SessionRecord>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub cleaned_sessions: Vec<SessionCleanupRecord>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<BrowserStateSummary>,
}

fn default_true() -> bool {
    true
}

fn default_max_steps() -> usize {
    10
}

fn is_default_agent_settings(settings: &AgentSettings) -> bool {
    settings == &AgentSettings::default()
}

#[must_use]
pub fn tool_manifest() -> Vec<McpToolContract> {
    vec![
        tool_contract::<StateToolInput, StateToolOutput>(
            STATE_TOOL_NAME,
            "Launch a browser, navigate to a URL, and return browser-use state.",
        ),
        tool_contract::<ActionsToolInput, ActionsToolOutput>(
            ACTIONS_TOOL_NAME,
            "Launch a browser, run browser-use actions, and return action results plus final state.",
        ),
        tool_contract::<ReplayToolInput, ReplayToolOutput>(
            REPLAY_TOOL_NAME,
            "Replay saved browser-use AgentHistory against current browser state.",
        ),
        tool_contract::<AgentToolInput, AgentToolOutput>(
            AGENT_TOOL_NAME,
            "Launch a browser, run a bounded browser-use agent task, and return agent history.",
        ),
        tool_contract::<SessionToolInput, SessionToolOutput>(
            SESSION_TOOL_NAME,
            "Start, stop, list, or clean up persistent browser-use sessions.",
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
        "instructions": "Use browser_use_state for page state, browser_use_actions for deterministic browser actions, browser_use_replay for saved AgentHistory replay, browser_use_agent for bounded agent runs, and browser_use_session for persistent session lifecycle."
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

fn tool_contract<Input, Output>(name: &str, description: &str) -> McpToolContract
where
    Input: JsonSchema,
    Output: JsonSchema,
{
    McpToolContract {
        name: name.to_owned(),
        description: description.to_owned(),
        input_schema: serde_json::to_value(schema_for!(Input)).unwrap_or(Value::Null),
        output_schema: Some(serde_json::to_value(schema_for!(Output)).unwrap_or(Value::Null)),
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
            vec![
                STATE_TOOL_NAME,
                ACTIONS_TOOL_NAME,
                REPLAY_TOOL_NAME,
                AGENT_TOOL_NAME,
                SESSION_TOOL_NAME,
            ]
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
    fn replay_tool_schema_exposes_history() {
        let manifest = tool_manifest();
        let replay_tool = manifest
            .iter()
            .find(|tool| tool.name == REPLAY_TOOL_NAME)
            .expect("replay tool");
        let schema_text = serde_json::to_string(&replay_tool.input_schema).expect("schema text");

        assert!(schema_text.contains("history"));
        assert!(schema_text.contains("session_id"));
        assert!(schema_text.contains("url"));
    }

    #[test]
    fn manifest_exposes_output_schemas() {
        let manifest = tool_manifest();

        for tool_name in [
            STATE_TOOL_NAME,
            ACTIONS_TOOL_NAME,
            REPLAY_TOOL_NAME,
            AGENT_TOOL_NAME,
            SESSION_TOOL_NAME,
        ] {
            let tool = manifest
                .iter()
                .find(|tool| tool.name == tool_name)
                .expect("tool contract");
            assert!(
                tool.output_schema.is_some(),
                "{tool_name} missing outputSchema"
            );
        }

        let replay_tool = manifest
            .iter()
            .find(|tool| tool.name == REPLAY_TOOL_NAME)
            .expect("replay tool");
        let output_schema_text =
            serde_json::to_string(&replay_tool.output_schema).expect("output schema text");

        assert!(output_schema_text.contains("replay"));
        assert!(output_schema_text.contains("AgentHistoryReplayRun"));
    }

    #[test]
    fn agent_tool_keeps_provider_secret_out_of_tool_input() {
        let schema = serde_json::to_value(schema_for!(AgentToolInput)).expect("schema");
        let schema_text = serde_json::to_string(&schema).expect("schema text");

        assert!(!schema_text.contains("api_key"));
        assert!(schema_text.contains("provider"));
        assert!(schema_text.contains("openai-compatible"));
        assert!(schema_text.contains("deepseek"));
        assert!(schema_text.contains("groq"));
        assert!(schema_text.contains("cerebras"));
        assert!(schema_text.contains("mistral"));
        assert!(schema_text.contains("openrouter"));
        assert!(schema_text.contains("vercel"));
        assert!(schema_text.contains("anthropic"));
        assert!(schema_text.contains("gemini"));
        assert!(schema_text.contains("ollama"));
        assert!(schema_text.contains("model"));
        assert!(schema_text.contains("base_url"));
        assert!(schema_text.contains("structured_output_mode"));
        assert!(schema_text.contains("json-schema"));
        assert!(schema_text.contains("json-object"));
        assert!(schema_text.contains("prompt-only"));
        assert!(schema_text.contains("tool-call"));
        assert!(schema_text.contains("settings"));
        assert!(schema_text.contains("use_vision"));
        assert!(schema_text.contains("max_actions_per_step"));
        assert!(schema_text.contains("generate_gif"));
        assert!(schema_text.contains("calculate_cost"));
        assert!(schema_text.contains("include_tool_call_examples"));
        assert!(schema_text.contains("vision_detail_level"));
        assert!(schema_text.contains("flash_mode"));
        assert!(schema_text.contains("use_judge"));
        assert!(schema_text.contains("ground_truth"));
        assert!(schema_text.contains("extraction_schema"));
        assert!(schema_text.contains("message_compaction"));
        assert!(schema_text.contains("compact_every_n_steps"));
        assert!(schema_text.contains("trigger_char_count"));
        assert!(schema_text.contains("keep_last_items"));
        assert!(schema_text.contains("save_conversation_path"));
        assert!(schema_text.contains("save_conversation_path_encoding"));
        assert!(schema_text.contains("file_system_path"));
        assert!(schema_text.contains("max_clickable_elements_length"));
        assert!(schema_text.contains("include_recent_events"));
        assert!(schema_text.contains("sample_images"));
        assert!(schema_text.contains("llm_screenshot_size"));
        assert!(schema_text.contains("url_shortening_limit"));
        assert!(schema_text.contains("display_files_in_done_text"));
        assert!(schema_text.contains("available_file_paths"));
        assert!(schema_text.contains("initial_actions"));
        assert!(schema_text.contains("directly_open_url"));
        assert!(schema_text.contains("excluded_actions"));
        assert!(schema_text.contains("sensitive_data"));
        assert!(schema_text.contains("override_system_message"));
        assert!(schema_text.contains("extend_system_message"));
    }

    #[test]
    fn agent_tool_accepts_structured_output_mode_aliases() {
        let input: AgentToolInput = serde_json::from_value(json!({
            "url": "https://example.com",
            "task": "extract",
            "structured_output_mode": "tool_call"
        }))
        .expect("agent input");

        assert_eq!(
            input.structured_output_mode,
            Some(AgentStructuredOutputMode::ToolCall)
        );
    }

    #[test]
    fn agent_tool_preserves_excluded_action_settings() {
        let input: AgentToolInput = serde_json::from_value(json!({
            "url": "https://example.com",
            "task": "extract",
            "settings": {
                "use_vision": "auto",
                "vision_detail_level": "high",
                "excluded_actions": ["search", "scroll"],
                "available_file_paths": ["/tmp/report.pdf"],
                "include_recent_events": true,
                "display_files_in_done_text": false,
                "generate_gif": "/tmp/trace.gif",
                "calculate_cost": true,
                "include_tool_call_examples": true,
                "use_judge": false,
                "ground_truth": "Must include a receipt.",
                "extraction_schema": {
                    "type": "object",
                    "properties": {
                        "company": { "type": "string" }
                    }
                },
                "message_compaction": {
                    "compact_every_n_steps": 3,
                    "trigger_char_count": 1024,
                    "keep_last_items": 2,
                    "summary_max_chars": 1200,
                    "include_read_state": true
                },
                "save_conversation_path": "/tmp/conversations",
                "save_conversation_path_encoding": "utf-8",
                "file_system_path": "/tmp/browser-use-agent-files"
            }
        }))
        .expect("agent input");

        assert_eq!(
            input.settings.vision_detail_level,
            browser_use_core::ImageDetailLevel::High
        );
        assert_eq!(
            input.settings.use_vision,
            browser_use_core::VisionMode::Auto
        );
        assert_eq!(input.settings.excluded_actions, ["search", "scroll"]);
        assert_eq!(input.settings.available_file_paths, ["/tmp/report.pdf"]);
        assert!(input.settings.include_recent_events);
        assert!(!input.settings.display_files_in_done_text);
        assert_eq!(
            input.settings.generate_gif,
            browser_use_core::GenerateGif::Path("/tmp/trace.gif".to_owned())
        );
        assert!(input.settings.calculate_cost);
        assert!(input.settings.include_tool_call_examples);
        assert!(!input.settings.use_judge);
        assert_eq!(
            input.settings.ground_truth.as_deref(),
            Some("Must include a receipt.")
        );
        assert_eq!(
            input
                .settings
                .extraction_schema
                .as_ref()
                .and_then(|schema| { schema["properties"]["company"]["type"].as_str() }),
            Some("string")
        );
        let browser_use_core::MessageCompaction::Settings(message_compaction) =
            &input.settings.message_compaction
        else {
            panic!("expected message compaction settings");
        };
        assert_eq!(message_compaction.compact_every_n_steps, 3);
        assert_eq!(message_compaction.trigger_char_count, Some(1024));
        assert_eq!(message_compaction.keep_last_items, 2);
        assert_eq!(message_compaction.summary_max_chars, 1200);
        assert!(message_compaction.include_read_state);
        assert_eq!(
            input.settings.save_conversation_path.as_deref(),
            Some("/tmp/conversations")
        );
        assert_eq!(
            input.settings.save_conversation_path_encoding.as_deref(),
            Some("utf-8")
        );
        assert_eq!(
            input.settings.file_system_path.as_deref(),
            Some("/tmp/browser-use-agent-files")
        );
    }

    #[test]
    fn session_tool_schema_exposes_lifecycle_operations() {
        let manifest = tool_manifest();
        let session_tool = manifest
            .iter()
            .find(|tool| tool.name == SESSION_TOOL_NAME)
            .expect("session tool");
        let schema_text = serde_json::to_string(&session_tool.input_schema).expect("schema text");

        assert!(schema_text.contains("operation"));
        assert!(schema_text.contains("start"));
        assert!(schema_text.contains("stop"));
        assert!(schema_text.contains("list"));
        assert!(schema_text.contains("cleanup"));
        assert!(schema_text.contains("force"));
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
