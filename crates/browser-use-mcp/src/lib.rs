//! MCP bridge contracts for browser-use-rs.
//!
//! This crate contains only protocol-facing data contracts and JSON builders.
//! The CLI owns the actual stdio/HTTP daemon, while these structs define the
//! tool inputs, outputs, session records, and JSON-RPC envelopes both sides
//! agree on.
//!
//! ```mermaid
//! sequenceDiagram
//!     participant Client as MCP client
//!     participant CLI as browser-use-rs daemon
//!     participant Contracts as browser-use-mcp
//!     participant Core as browser-use-core
//!     Client->>CLI: tools/list
//!     CLI->>Contracts: tool_manifest_json()
//!     Contracts-->>Client: names + input/output schemas
//!     Client->>CLI: tools/call(arguments)
//!     CLI->>Core: run state/actions/replay/agent work
//!     Core-->>CLI: typed result
//!     CLI->>Contracts: tool_success_result()
//!     Contracts-->>Client: text + structuredContent
//! ```

use std::path::PathBuf;

use browser_use_cdp::DevToolsEndpoint;
use browser_use_core::{
    ActionResult, AgentHistory, AgentHistoryReplayRun, AgentSettings, schema_to_compat_value,
};
use browser_use_dom::BrowserStateSummary;
use browser_use_tools::BrowserAction;
use schemars::{JsonSchema, schema_for};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// MCP protocol version advertised by this server.
pub const MCP_PROTOCOL_VERSION: &str = "2025-06-18";
/// Tool name for capturing browser state.
pub const STATE_TOOL_NAME: &str = "browser_use_state";
/// Tool name for executing browser actions.
pub const ACTIONS_TOOL_NAME: &str = "browser_use_actions";
/// Tool name for replaying saved agent history.
pub const REPLAY_TOOL_NAME: &str = "browser_use_replay";
/// Tool name for running a bounded agent task.
pub const AGENT_TOOL_NAME: &str = "browser_use_agent";
/// Tool name for persistent session lifecycle operations.
pub const SESSION_TOOL_NAME: &str = "browser_use_session";

/// MCP tool descriptor exposed from `tools/list`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct McpToolContract {
    /// Tool name.
    pub name: String,
    /// Human-readable tool description.
    pub description: String,
    /// JSON Schema for tool input.
    #[serde(rename = "inputSchema")]
    pub input_schema: Value,
    /// Optional JSON Schema for structured output.
    #[serde(
        rename = "outputSchema",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub output_schema: Option<Value>,
}

/// Minimal JSON-RPC request envelope used by MCP transports.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JsonRpcRequest {
    /// JSON-RPC version, normally `2.0`.
    pub jsonrpc: String,
    /// Request id, absent for notifications.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<Value>,
    /// JSON-RPC method name.
    pub method: String,
    /// Optional method parameters.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub params: Option<Value>,
}

/// Parameters for an MCP `tools/call` request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CallToolParams {
    /// Tool name to call.
    pub name: String,
    /// Tool-specific arguments.
    #[serde(default)]
    pub arguments: Value,
}

/// Input for `browser_use_state`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct StateToolInput {
    /// Existing persistent session id, if reusing one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    /// Optional URL to navigate to before capturing state.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// Whether to include a screenshot in returned state.
    #[serde(default = "default_true")]
    pub screenshot: bool,
}

/// Input for `browser_use_actions`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ActionsToolInput {
    /// Existing persistent session id, if reusing one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    /// Optional URL to navigate to before executing actions.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// Browser actions to execute.
    #[serde(default)]
    pub actions: Vec<BrowserAction>,
    /// Whether to include a screenshot in final returned state.
    #[serde(default = "default_true")]
    pub screenshot: bool,
}

/// Input for `browser_use_replay`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ReplayToolInput {
    /// Existing persistent session id, if reusing one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    /// Optional URL to navigate to before replay.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// Saved agent history to replay.
    pub history: AgentHistory,
}

/// Input for `browser_use_agent`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct AgentToolInput {
    /// Existing persistent session id, if reusing one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    /// Optional URL to navigate to before running the agent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// Natural-language task.
    pub task: String,
    /// LLM provider selector.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<AgentProvider>,
    /// Model id override.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Provider base URL override.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
    /// Structured-output mode override.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub structured_output_mode: Option<AgentStructuredOutputMode>,
    /// Maximum agent steps.
    #[serde(default = "default_max_steps")]
    pub max_steps: usize,
    /// Agent settings.
    #[serde(default, skip_serializing_if = "is_default_agent_settings")]
    pub settings: AgentSettings,
}

/// Structured-output mode exposed through MCP.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum AgentStructuredOutputMode {
    /// Use native JSON Schema response format.
    #[serde(rename = "json-schema", alias = "json_schema")]
    JsonSchema,
    /// Use JSON object response format.
    #[serde(rename = "json-object", alias = "json_object")]
    JsonObject,
    /// Prompt the model to emit JSON without API-level enforcement.
    #[serde(rename = "prompt-only", alias = "prompt_only")]
    PromptOnly,
    /// Request JSON through a tool-call argument payload.
    #[serde(rename = "tool-call", alias = "tool_call")]
    ToolCall,
}

/// LLM provider selector exposed through MCP.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum AgentProvider {
    /// Generic OpenAI-compatible chat-completions provider.
    #[serde(rename = "openai-compatible", alias = "openai")]
    OpenAiCompatible,
    /// DeepSeek OpenAI-compatible provider.
    #[serde(rename = "deepseek", alias = "deep-seek")]
    DeepSeek,
    /// Groq OpenAI-compatible provider.
    #[serde(rename = "groq")]
    Groq,
    /// Cerebras OpenAI-compatible provider.
    #[serde(rename = "cerebras")]
    Cerebras,
    /// Mistral OpenAI-compatible provider.
    #[serde(rename = "mistral")]
    Mistral,
    /// OpenRouter OpenAI-compatible provider.
    #[serde(rename = "openrouter", alias = "open-router")]
    OpenRouter,
    /// Vercel AI Gateway OpenAI-compatible provider.
    #[serde(rename = "vercel", alias = "ai-gateway", alias = "vercel-ai-gateway")]
    Vercel,
    /// Anthropic Messages provider.
    #[serde(rename = "anthropic")]
    Anthropic,
    /// Google Gemini provider.
    #[serde(rename = "gemini", alias = "google")]
    Gemini,
    /// Ollama local chat provider.
    #[serde(rename = "ollama", alias = "local")]
    Ollama,
}

/// Persistent session operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum SessionOperation {
    /// Start a persistent session.
    #[serde(rename = "start")]
    Start,
    /// Stop a persistent session.
    #[serde(rename = "stop")]
    Stop,
    /// List known persistent sessions.
    #[serde(rename = "list")]
    List,
    /// Remove stale stopped session records.
    #[serde(rename = "cleanup")]
    Cleanup,
}

/// Input for `browser_use_session`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SessionToolInput {
    /// Operation to perform.
    pub operation: SessionOperation,
    /// Session id used by stop/state operations.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    /// Optional start URL for session creation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// Whether session state should include a screenshot when returned.
    #[serde(default = "default_true")]
    pub screenshot: bool,
    /// Forces cleanup/stop behavior when supported.
    #[serde(default)]
    pub force: bool,
}

/// Runtime status for a persistent session record.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum SessionStatus {
    /// Process appears to still be running.
    #[serde(rename = "running")]
    Running,
    /// Record exists but process/session appears stale.
    #[serde(rename = "stale")]
    Stale,
    /// Session is stopped.
    #[serde(rename = "stopped")]
    Stopped,
    /// Status has not been checked.
    #[serde(rename = "unknown")]
    Unknown,
}

/// Cleanup action applied to a persistent session record.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum SessionCleanupAction {
    /// Record/directory was removed.
    #[serde(rename = "removed")]
    Removed,
    /// Running session was stopped.
    #[serde(rename = "stopped")]
    Stopped,
}

/// One cleanup action result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SessionCleanupRecord {
    /// Cleanup action performed.
    pub action: SessionCleanupAction,
    /// Session affected by the action.
    pub session: SessionRecord,
}

/// Persistent session metadata stored by the CLI daemon.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SessionRecord {
    /// Stable session id.
    pub id: String,
    /// DevTools endpoint for the session.
    pub endpoint: DevToolsEndpoint,
    /// User data directory used by Chrome.
    pub user_data_dir: PathBuf,
    /// Process id when the session was launched locally.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub process_id: Option<u32>,
    /// Last observed status.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<SessionStatus>,
}

/// Output for `browser_use_state`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct StateToolOutput {
    /// Captured browser state.
    pub state: BrowserStateSummary,
}

/// Output for `browser_use_actions`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ActionsToolOutput {
    /// Action execution results.
    pub results: Vec<ActionResult>,
    /// Browser state after actions.
    pub state: BrowserStateSummary,
}

/// Output for `browser_use_replay`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ReplayToolOutput {
    /// Replay run result.
    pub replay: AgentHistoryReplayRun,
}

/// Output for `browser_use_agent`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct AgentToolOutput {
    /// Agent history.
    pub history: AgentHistory,
}

/// Output for `browser_use_session`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SessionToolOutput {
    /// Session for start/stop operations.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<SessionRecord>,
    /// Sessions for list operations.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sessions: Vec<SessionRecord>,
    /// Cleanup results.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub cleaned_sessions: Vec<SessionCleanupRecord>,
    /// Optional browser state for session operations that return state.
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
/// Returns all MCP tool contracts.
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
/// Returns the tool manifest as JSON.
pub fn tool_manifest_json() -> Value {
    serde_json::to_value(tool_manifest()).unwrap_or(Value::Null)
}

#[must_use]
/// Builds the MCP `initialize` result payload.
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
/// Builds the MCP `tools/list` result payload.
pub fn tools_list_result() -> Value {
    json!({ "tools": tool_manifest() })
}

#[must_use]
/// Wraps structured tool output in an MCP successful tool result.
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
/// Builds an MCP tool error result.
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
/// Builds a JSON-RPC success response.
pub fn json_rpc_success(id: Value, result: Value) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": result
    })
}

#[must_use]
/// Builds a JSON-RPC error response.
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
    // MCP clients treat schemas as part of the protocol contract. Reuse the
    // same compatibility normalizer as agent prompts so adding Rust docs cannot
    // silently change tool input/output shapes.
    McpToolContract {
        name: name.to_owned(),
        description: description.to_owned(),
        input_schema: schema_to_compat_value(schema_for!(Input)),
        output_schema: Some(schema_to_compat_value(schema_for!(Output))),
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
