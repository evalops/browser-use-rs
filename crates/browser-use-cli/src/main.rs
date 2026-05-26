//! Command-line and daemon entry point for browser-use-rs.
//!
//! The binary wires together the public crates: it parses CLI options, launches
//! or connects to CDP browser sessions, configures LLM providers, runs bounded
//! agents, manages persistent session records, and exposes the MCP JSON-RPC
//! bridge over stdio, TCP, or HTTP.

use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use base64::Engine;
use browser_use_cdp::{BrowserProfile, BrowserSession, CdpBrowserSession};
use browser_use_core::{
    AgentSettings, BrowserActionExecutor, GenerateGif, ImageDetailLevel, MessageCompaction,
    MessageCompactionSettings, SensitiveDataValue, VisionMode,
};
use browser_use_llm::{
    AnthropicChatModel, ChatModel, GeminiChatModel, OllamaChatModel, OpenAiCompatibleChatModel,
    OpenAiSchemaTransform, OpenAiStructuredOutputMode,
};
use clap::Parser;
use schemars::schema_for;
use serde_json::Value;
use tokio::io::{self, AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio::time::sleep;

mod mcp_daemon;
mod session;

#[cfg(test)]
use mcp_daemon::{
    DaemonLifecycleFiles, HttpRequest, McpRuntime, McpSessionPlan, handle_http_request,
    handle_mcp_message, http_request_authorized, mcp_session_plan, read_http_request,
};
use mcp_daemon::{DaemonLifecycleOptions, run_daemon, run_mcp_stdio};
#[cfg(test)]
use session::{
    SessionCleanupDecision, StoredSession, annotate_session_status, session_cleanup_decision,
    session_status_with_checker,
};
use session::{
    SessionCommand, cleanup_persistent_sessions, list_session_records, read_agent_history,
    read_session_record, run_session_command, session_record_path, start_persistent_session,
    stop_persistent_session,
};

#[derive(Debug, Parser)]
#[command(name = "browser-use-rs")]
#[command(about = "Rust behavioral conformance port of browser-use")]
struct Cli {
    #[arg(long, default_value_t = false)]
    version_target: bool,
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Debug, clap::Subcommand)]
#[allow(clippy::large_enum_variant)]
enum Command {
    /// Print the frozen upstream browser-use commit target.
    VersionTarget,
    /// Print a JSON Schema for a compatibility contract.
    Schema { contract: SchemaContract },
    /// Print the MCP tool manifest JSON exposed by browser-use-mcp.
    McpTools,
    /// Run a stdio MCP server exposing browser-use tools.
    McpStdio,
    /// Run a TCP or HTTP JSON-RPC daemon exposing the same tools as mcp-stdio.
    Daemon {
        #[arg(long, default_value = "127.0.0.1:8765")]
        addr: String,
        #[arg(long, value_enum, default_value = "tcp")]
        transport: DaemonTransport,
        #[arg(long, env = "BROWSER_USE_RS_DAEMON_TOKEN")]
        auth_token: Option<String>,
        #[arg(long)]
        pid_file: Option<PathBuf>,
        #[arg(long)]
        ready_file: Option<PathBuf>,
    },
    /// Create, reuse, and stop local Chrome sessions across CLI invocations.
    Session {
        #[command(subcommand)]
        command: SessionCommand,
    },
    /// Launch Chrome, navigate to a URL, print state JSON, then exit.
    Open { url: String },
    /// Launch Chrome, navigate to a URL, and print browser state JSON.
    State {
        url: String,
        #[arg(long, default_value_t = false)]
        screenshot: bool,
    },
    /// Launch Chrome, navigate to a URL, and write a PNG screenshot.
    Screenshot { url: String, output: PathBuf },
    /// Launch Chrome, navigate to a URL, click an indexed element, and print state JSON.
    Click { url: String, index: u32 },
    /// Launch Chrome, navigate to a URL, type into an indexed element, and print state JSON.
    Type {
        url: String,
        index: u32,
        text: String,
        #[arg(long, default_value_t = true)]
        clear: bool,
    },
    /// Launch Chrome, navigate to a URL, scroll, and print state JSON.
    Scroll {
        url: String,
        #[arg(long, default_value_t = 1.0)]
        pages: f64,
        #[arg(long, default_value_t = true)]
        down: bool,
    },
    /// Launch Chrome, run a JSON action list in one session, and print results plus final state.
    Actions {
        url: String,
        actions: PathBuf,
        #[arg(long, default_value_t = true)]
        screenshot: bool,
    },
    /// Launch Chrome, replay saved AgentHistory against current state, and print the replay run.
    Replay { url: String, history: PathBuf },
    /// Run a bounded browser agent task through a schema-guided chat model.
    Agent {
        url: String,
        task: String,
        #[arg(long, value_enum, default_value = "openai-compatible")]
        provider: LlmProvider,
        #[arg(long)]
        api_key: Option<String>,
        #[arg(long)]
        model: Option<String>,
        #[arg(long)]
        base_url: Option<String>,
        #[arg(long = "structured-output-mode", value_enum)]
        structured_output_mode: Option<StructuredOutputMode>,
        #[arg(long = "allowed-domain")]
        allowed_domains: Vec<String>,
        #[arg(long = "prohibited-domain")]
        prohibited_domains: Vec<String>,
        #[arg(long, default_value_t = false)]
        block_ip_addresses: bool,
        #[arg(long, default_value_t = 10)]
        max_steps: usize,
        #[arg(long, default_value_t = false)]
        no_vision: bool,
        #[arg(long = "vision-mode", value_enum, conflicts_with = "no_vision")]
        vision_mode: Option<CliVisionMode>,
        #[arg(long = "vision-detail-level", value_enum)]
        vision_detail_level: Option<CliVisionDetailLevel>,
        #[arg(long)]
        max_failures: Option<u32>,
        #[arg(long = "generate-gif", num_args = 0..=1, default_missing_value = "true", value_name = "PATH")]
        generate_gif: Option<String>,
        #[arg(long)]
        max_actions_per_step: Option<usize>,
        #[arg(long)]
        llm_timeout_seconds: Option<u64>,
        #[arg(long)]
        step_timeout_seconds: Option<u64>,
        #[arg(long)]
        action_timeout_seconds: Option<f64>,
        #[arg(long)]
        wait_between_actions_seconds: Option<f64>,
        #[arg(long, default_value_t = false)]
        no_directly_open_url: bool,
        #[arg(long, default_value_t = false)]
        no_final_response_after_failure: bool,
        #[arg(long, default_value_t = false)]
        no_display_files_in_done_text: bool,
        #[arg(long, default_value_t = false)]
        no_loop_detection: bool,
        #[arg(long)]
        loop_detection_window: Option<usize>,
        #[arg(long, default_value_t = false)]
        no_thinking: bool,
        #[arg(long, default_value_t = false)]
        flash_mode: bool,
        #[arg(long, default_value_t = false)]
        no_judge: bool,
        #[arg(long = "ground-truth")]
        ground_truth: Option<String>,
        #[arg(long = "extraction-schema", value_parser = parse_json_value)]
        extraction_schema: Option<Value>,
        #[arg(long, default_value_t = false)]
        calculate_cost: bool,
        #[arg(long, default_value_t = false)]
        include_tool_call_examples: bool,
        #[arg(long = "save-conversation-path")]
        save_conversation_path: Option<String>,
        #[arg(long = "save-conversation-path-encoding")]
        save_conversation_path_encoding: Option<String>,
        #[arg(long = "file-system-path")]
        file_system_path: Option<String>,
        #[arg(long, default_value_t = false)]
        no_planning: bool,
        #[arg(long)]
        planning_replan_on_stall: Option<usize>,
        #[arg(long)]
        planning_exploration_limit: Option<usize>,
        #[arg(long)]
        max_history_items: Option<usize>,
        #[arg(long = "no-message-compaction", default_value_t = false)]
        no_message_compaction: bool,
        #[arg(long = "message-compaction-compact-every-n-steps")]
        message_compaction_compact_every_n_steps: Option<usize>,
        #[arg(
            long = "message-compaction-trigger-char-count",
            conflicts_with = "message_compaction_trigger_token_count"
        )]
        message_compaction_trigger_char_count: Option<usize>,
        #[arg(
            long = "message-compaction-trigger-token-count",
            conflicts_with = "message_compaction_trigger_char_count"
        )]
        message_compaction_trigger_token_count: Option<usize>,
        #[arg(long = "message-compaction-chars-per-token")]
        message_compaction_chars_per_token: Option<f64>,
        #[arg(long = "message-compaction-keep-last-items")]
        message_compaction_keep_last_items: Option<usize>,
        #[arg(long = "message-compaction-summary-max-chars")]
        message_compaction_summary_max_chars: Option<usize>,
        #[arg(
            long = "message-compaction-include-read-state",
            default_value_t = false
        )]
        message_compaction_include_read_state: bool,
        #[arg(long)]
        max_clickable_elements_length: Option<usize>,
        #[arg(long, default_value_t = false)]
        include_recent_events: bool,
        #[arg(long = "include-attribute")]
        include_attributes: Vec<String>,
        #[arg(long = "available-file-path")]
        available_file_paths: Vec<String>,
        #[arg(long = "exclude-action")]
        excluded_actions: Vec<String>,
        #[arg(long = "sensitive-data", value_parser = parse_sensitive_data_entry)]
        sensitive_data: Vec<SensitiveDataEntry>,
        #[arg(long = "sensitive-data-domain", value_parser = parse_domain_sensitive_data_entry)]
        sensitive_data_domains: Vec<DomainSensitiveDataEntry>,
        #[arg(long)]
        override_system_message: Option<String>,
        #[arg(long)]
        extend_system_message: Option<String>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
enum LlmProvider {
    #[value(name = "openai-compatible", alias = "openai")]
    OpenAiCompatible,
    #[value(name = "deepseek", alias = "deep-seek")]
    DeepSeek,
    Groq,
    Cerebras,
    Mistral,
    #[value(name = "openrouter", alias = "open-router")]
    OpenRouter,
    #[value(name = "vercel", alias = "ai-gateway", alias = "vercel-ai-gateway")]
    Vercel,
    Anthropic,
    #[value(alias = "google")]
    Gemini,
    #[value(alias = "local")]
    Ollama,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
enum CliVisionDetailLevel {
    Auto,
    Low,
    High,
}

impl From<CliVisionDetailLevel> for ImageDetailLevel {
    fn from(value: CliVisionDetailLevel) -> Self {
        match value {
            CliVisionDetailLevel::Auto => Self::Auto,
            CliVisionDetailLevel::Low => Self::Low,
            CliVisionDetailLevel::High => Self::High,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
enum CliVisionMode {
    #[value(alias = "true")]
    Always,
    Auto,
    #[value(alias = "false")]
    Never,
}

impl From<CliVisionMode> for VisionMode {
    fn from(value: CliVisionMode) -> Self {
        match value {
            CliVisionMode::Always => Self::Always,
            CliVisionMode::Auto => Self::Auto,
            CliVisionMode::Never => Self::Never,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
enum DaemonTransport {
    Tcp,
    Http,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
enum StructuredOutputMode {
    JsonSchema,
    JsonObject,
    PromptOnly,
    ToolCall,
}

impl StructuredOutputMode {
    fn into_openai_mode(self) -> OpenAiStructuredOutputMode {
        match self {
            Self::JsonSchema => OpenAiStructuredOutputMode::JsonSchema,
            Self::JsonObject => OpenAiStructuredOutputMode::JsonObject,
            Self::PromptOnly => OpenAiStructuredOutputMode::PromptOnly,
            Self::ToolCall => OpenAiStructuredOutputMode::ToolCall,
        }
    }

    fn from_mcp(mode: browser_use_mcp::AgentStructuredOutputMode) -> Self {
        match mode {
            browser_use_mcp::AgentStructuredOutputMode::JsonSchema => Self::JsonSchema,
            browser_use_mcp::AgentStructuredOutputMode::JsonObject => Self::JsonObject,
            browser_use_mcp::AgentStructuredOutputMode::PromptOnly => Self::PromptOnly,
            browser_use_mcp::AgentStructuredOutputMode::ToolCall => Self::ToolCall,
        }
    }
}

impl DaemonTransport {
    fn as_str(self) -> &'static str {
        match self {
            Self::Tcp => "tcp",
            Self::Http => "http",
        }
    }
}

impl LlmProvider {
    fn from_mcp(provider: Option<browser_use_mcp::AgentProvider>) -> Self {
        match provider.unwrap_or(browser_use_mcp::AgentProvider::OpenAiCompatible) {
            browser_use_mcp::AgentProvider::OpenAiCompatible => Self::OpenAiCompatible,
            browser_use_mcp::AgentProvider::DeepSeek => Self::DeepSeek,
            browser_use_mcp::AgentProvider::Groq => Self::Groq,
            browser_use_mcp::AgentProvider::Cerebras => Self::Cerebras,
            browser_use_mcp::AgentProvider::Mistral => Self::Mistral,
            browser_use_mcp::AgentProvider::OpenRouter => Self::OpenRouter,
            browser_use_mcp::AgentProvider::Vercel => Self::Vercel,
            browser_use_mcp::AgentProvider::Anthropic => Self::Anthropic,
            browser_use_mcp::AgentProvider::Gemini => Self::Gemini,
            browser_use_mcp::AgentProvider::Ollama => Self::Ollama,
        }
    }
}

#[derive(Debug, Clone, Copy, clap::ValueEnum)]
enum SchemaContract {
    Action,
    AgentOutput,
    ReplayRun,
    BrowserState,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Some(Command::VersionTarget) => {
            println!("{}", browser_use_core::INITIAL_UPSTREAM_COMMIT);
        }
        Some(Command::Schema { contract }) => {
            let schema = match contract {
                SchemaContract::Action => schema_for!(browser_use_tools::BrowserAction),
                SchemaContract::AgentOutput => schema_for!(browser_use_core::AgentOutput),
                SchemaContract::ReplayRun => schema_for!(browser_use_core::AgentHistoryReplayRun),
                SchemaContract::BrowserState => schema_for!(browser_use_dom::BrowserStateSummary),
            };
            println!("{}", serde_json::to_string_pretty(&schema)?);
        }
        Some(Command::McpTools) => {
            println!(
                "{}",
                serde_json::to_string_pretty(&browser_use_mcp::tool_manifest_json())?
            );
        }
        Some(Command::McpStdio) => {
            run_mcp_stdio().await?;
        }
        Some(Command::Daemon {
            addr,
            transport,
            auth_token,
            pid_file,
            ready_file,
        }) => {
            run_daemon(
                &addr,
                transport,
                auth_token,
                DaemonLifecycleOptions {
                    pid_file,
                    ready_file,
                },
            )
            .await?;
        }
        Some(Command::Session { command }) => {
            run_session_command(command).await?;
        }
        Some(Command::Open { url }) => {
            let session = launch_and_navigate(&url).await?;
            print_state(&session, true).await?;
        }
        Some(Command::State { url, screenshot }) => {
            let session = launch_and_navigate(&url).await?;
            print_state(&session, screenshot).await?;
        }
        Some(Command::Screenshot { url, output }) => {
            let session = launch_and_navigate(&url).await?;
            let screenshot = session.screenshot().await?;
            let png = base64::engine::general_purpose::STANDARD.decode(screenshot.base64_png)?;
            std::fs::write(&output, png)?;
            println!("{}", output.display());
        }
        Some(Command::Click { url, index }) => {
            let session = launch_and_navigate(&url).await?;
            session.click(index).await?;
            sleep(Duration::from_millis(100)).await;
            print_state(&session, true).await?;
        }
        Some(Command::Type {
            url,
            index,
            text,
            clear,
        }) => {
            let session = launch_and_navigate(&url).await?;
            session.input_text(index, &text, clear).await?;
            print_state(&session, true).await?;
        }
        Some(Command::Scroll { url, pages, down }) => {
            let session = launch_and_navigate(&url).await?;
            session.scroll(None, down, pages).await?;
            print_state(&session, true).await?;
        }
        Some(Command::Actions {
            url,
            actions,
            screenshot,
        }) => {
            let session = launch_and_navigate(&url).await?;
            let actions = std::fs::read_to_string(&actions)?;
            let actions: Vec<browser_use_tools::BrowserAction> = serde_json::from_str(&actions)?;
            let mut executor = BrowserActionExecutor::new(session);
            let results = executor.execute_sequence(&actions).await;
            let state = executor.session().state(screenshot).await?;
            println!(
                "{}",
                serde_json::to_string_pretty(&serde_json::json!({
                    "results": results,
                    "state": state,
                }))?
            );
        }
        Some(Command::Replay { url, history }) => {
            let session = launch_and_navigate(&url).await?;
            let history = read_agent_history(&history)?;
            let mut executor = BrowserActionExecutor::new(session);
            let replay = executor.replay_history(&history).await?;
            println!("{}", serde_json::to_string_pretty(&replay)?);
        }
        Some(Command::Agent {
            url,
            task,
            provider,
            api_key,
            model,
            base_url,
            structured_output_mode,
            allowed_domains,
            prohibited_domains,
            block_ip_addresses,
            max_steps,
            no_vision,
            vision_mode,
            vision_detail_level,
            max_failures,
            generate_gif,
            max_actions_per_step,
            llm_timeout_seconds,
            step_timeout_seconds,
            action_timeout_seconds,
            wait_between_actions_seconds,
            no_directly_open_url,
            no_final_response_after_failure,
            no_display_files_in_done_text,
            no_loop_detection,
            loop_detection_window,
            no_thinking,
            flash_mode,
            no_judge,
            ground_truth,
            extraction_schema,
            calculate_cost,
            include_tool_call_examples,
            save_conversation_path,
            save_conversation_path_encoding,
            file_system_path,
            no_planning,
            planning_replan_on_stall,
            planning_exploration_limit,
            max_history_items,
            no_message_compaction,
            message_compaction_compact_every_n_steps,
            message_compaction_trigger_char_count,
            message_compaction_trigger_token_count,
            message_compaction_chars_per_token,
            message_compaction_keep_last_items,
            message_compaction_summary_max_chars,
            message_compaction_include_read_state,
            max_clickable_elements_length,
            include_recent_events,
            include_attributes,
            available_file_paths,
            excluded_actions,
            sensitive_data,
            sensitive_data_domains,
            override_system_message,
            extend_system_message,
        }) => {
            let llm = configured_chat_model(
                provider,
                api_key,
                model,
                base_url,
                structured_output_mode.map(StructuredOutputMode::into_openai_mode),
            )?;
            let session = launch_and_navigate_with_profile(
                &url,
                BrowserProfile {
                    allowed_domains,
                    prohibited_domains,
                    block_ip_addresses,
                    ..BrowserProfile::default()
                },
            )
            .await?;
            let settings = cli_agent_settings(CliAgentSettingsArgs {
                no_vision,
                vision_mode,
                vision_detail_level,
                max_failures,
                generate_gif,
                max_actions_per_step,
                llm_timeout_seconds,
                step_timeout_seconds,
                action_timeout_seconds,
                wait_between_actions_seconds,
                no_directly_open_url,
                no_final_response_after_failure,
                no_display_files_in_done_text,
                no_loop_detection,
                loop_detection_window,
                no_thinking,
                flash_mode,
                no_judge,
                ground_truth,
                extraction_schema,
                calculate_cost,
                include_tool_call_examples,
                save_conversation_path,
                save_conversation_path_encoding,
                file_system_path,
                no_planning,
                planning_replan_on_stall,
                planning_exploration_limit,
                max_history_items,
                no_message_compaction,
                message_compaction_compact_every_n_steps,
                message_compaction_trigger_char_count,
                message_compaction_trigger_token_count,
                message_compaction_chars_per_token,
                message_compaction_keep_last_items,
                message_compaction_summary_max_chars,
                message_compaction_include_read_state,
                max_clickable_elements_length,
                include_recent_events,
                include_attributes,
                available_file_paths,
                excluded_actions,
                sensitive_data,
                sensitive_data_domains,
                override_system_message,
                extend_system_message,
            });
            let mut agent = browser_use_core::Agent::with_settings(task, settings, llm, session);
            let history = agent.run(max_steps).await?;
            println!("{}", serde_json::to_string_pretty(history)?);
        }
        None if cli.version_target => {
            println!("{}", browser_use_core::INITIAL_UPSTREAM_COMMIT);
        }
        None => {}
    }

    Ok(())
}

async fn launch_and_navigate(url: &str) -> anyhow::Result<CdpBrowserSession> {
    launch_and_navigate_with_profile(url, BrowserProfile::default()).await
}

async fn launch_and_navigate_with_profile(
    url: &str,
    profile: BrowserProfile,
) -> anyhow::Result<CdpBrowserSession> {
    let session = CdpBrowserSession::launch(&profile).await?;
    session.navigate(url, false).await?;
    sleep(Duration::from_millis(150)).await;
    Ok(session)
}

async fn print_state(session: &CdpBrowserSession, include_screenshot: bool) -> anyhow::Result<()> {
    let state = session.state(include_screenshot).await?;
    println!("{}", serde_json::to_string_pretty(&state)?);
    Ok(())
}

#[derive(Debug, Default)]
struct CliAgentSettingsArgs {
    no_vision: bool,
    vision_mode: Option<CliVisionMode>,
    vision_detail_level: Option<CliVisionDetailLevel>,
    max_failures: Option<u32>,
    generate_gif: Option<String>,
    max_actions_per_step: Option<usize>,
    llm_timeout_seconds: Option<u64>,
    step_timeout_seconds: Option<u64>,
    action_timeout_seconds: Option<f64>,
    wait_between_actions_seconds: Option<f64>,
    no_directly_open_url: bool,
    no_final_response_after_failure: bool,
    no_display_files_in_done_text: bool,
    no_loop_detection: bool,
    loop_detection_window: Option<usize>,
    no_thinking: bool,
    flash_mode: bool,
    no_judge: bool,
    ground_truth: Option<String>,
    extraction_schema: Option<Value>,
    calculate_cost: bool,
    include_tool_call_examples: bool,
    save_conversation_path: Option<String>,
    save_conversation_path_encoding: Option<String>,
    file_system_path: Option<String>,
    no_planning: bool,
    planning_replan_on_stall: Option<usize>,
    planning_exploration_limit: Option<usize>,
    max_history_items: Option<usize>,
    no_message_compaction: bool,
    message_compaction_compact_every_n_steps: Option<usize>,
    message_compaction_trigger_char_count: Option<usize>,
    message_compaction_trigger_token_count: Option<usize>,
    message_compaction_chars_per_token: Option<f64>,
    message_compaction_keep_last_items: Option<usize>,
    message_compaction_summary_max_chars: Option<usize>,
    message_compaction_include_read_state: bool,
    max_clickable_elements_length: Option<usize>,
    include_recent_events: bool,
    include_attributes: Vec<String>,
    available_file_paths: Vec<String>,
    excluded_actions: Vec<String>,
    sensitive_data: Vec<SensitiveDataEntry>,
    sensitive_data_domains: Vec<DomainSensitiveDataEntry>,
    override_system_message: Option<String>,
    extend_system_message: Option<String>,
}

fn cli_agent_settings(args: CliAgentSettingsArgs) -> AgentSettings {
    let mut settings = AgentSettings::default();
    let custom_message_compaction = cli_message_compaction_settings(&args);

    if args.no_vision {
        settings.use_vision = VisionMode::Never;
    }
    if let Some(value) = args.vision_mode {
        settings.use_vision = value.into();
    }
    if let Some(value) = args.vision_detail_level {
        settings.vision_detail_level = value.into();
    }
    if let Some(value) = args.max_failures {
        settings.max_failures = value;
    }
    if let Some(value) = args.generate_gif {
        settings.generate_gif = cli_generate_gif(value);
    }
    if let Some(value) = args.max_actions_per_step {
        settings.max_actions_per_step = value;
    }
    if let Some(value) = args.llm_timeout_seconds {
        settings.llm_timeout_seconds = value;
    }
    if let Some(value) = args.step_timeout_seconds {
        settings.step_timeout_seconds = value;
    }
    if let Some(value) = args.action_timeout_seconds {
        settings.action_timeout_seconds = value;
    }
    if let Some(value) = args.wait_between_actions_seconds {
        settings.wait_between_actions_seconds = value;
    }
    if args.no_directly_open_url {
        settings.directly_open_url = false;
    }
    if args.no_final_response_after_failure {
        settings.final_response_after_failure = false;
    }
    if args.no_display_files_in_done_text {
        settings.display_files_in_done_text = false;
    }
    if args.no_loop_detection {
        settings.loop_detection_enabled = false;
    }
    if let Some(value) = args.loop_detection_window {
        settings.loop_detection_window = value;
    }
    if args.no_thinking {
        settings.use_thinking = false;
    }
    if args.flash_mode {
        settings.flash_mode = true;
    }
    if args.no_judge {
        settings.use_judge = false;
    }
    settings.ground_truth = args.ground_truth;
    settings.extraction_schema = args.extraction_schema;
    settings.calculate_cost = args.calculate_cost;
    settings.include_tool_call_examples = args.include_tool_call_examples;
    settings.save_conversation_path = args.save_conversation_path;
    if let Some(value) = args.save_conversation_path_encoding {
        settings.save_conversation_path_encoding = Some(value);
    }
    settings.file_system_path = args.file_system_path;
    if args.no_planning {
        settings.enable_planning = false;
    }
    if let Some(value) = args.planning_replan_on_stall {
        settings.planning_replan_on_stall = value;
    }
    if let Some(value) = args.planning_exploration_limit {
        settings.planning_exploration_limit = value;
    }
    settings.max_history_items = args.max_history_items;
    if args.no_message_compaction {
        settings.message_compaction = MessageCompaction::Disabled;
    } else if let Some(message_compaction) = custom_message_compaction {
        settings.message_compaction = MessageCompaction::Settings(message_compaction);
    }
    if let Some(value) = args.max_clickable_elements_length {
        settings.max_clickable_elements_length = value;
    }
    settings.include_recent_events = args.include_recent_events;
    settings.include_attributes = args.include_attributes;
    settings.available_file_paths = args.available_file_paths;
    settings.excluded_actions = args.excluded_actions;
    settings.sensitive_data = cli_sensitive_data(args.sensitive_data, args.sensitive_data_domains);
    settings.override_system_message = args.override_system_message;
    settings.extend_system_message = args.extend_system_message;

    settings
}

fn cli_message_compaction_settings(
    args: &CliAgentSettingsArgs,
) -> Option<MessageCompactionSettings> {
    let has_custom_message_compaction = args.message_compaction_compact_every_n_steps.is_some()
        || args.message_compaction_trigger_char_count.is_some()
        || args.message_compaction_trigger_token_count.is_some()
        || args.message_compaction_chars_per_token.is_some()
        || args.message_compaction_keep_last_items.is_some()
        || args.message_compaction_summary_max_chars.is_some()
        || args.message_compaction_include_read_state;
    if !has_custom_message_compaction {
        return None;
    }

    let mut settings = MessageCompactionSettings::default();
    if let Some(value) = args.message_compaction_compact_every_n_steps {
        settings.compact_every_n_steps = value;
    }
    if let Some(value) = args.message_compaction_trigger_char_count {
        settings.trigger_char_count = Some(value);
        settings.trigger_token_count = None;
    }
    if let Some(value) = args.message_compaction_trigger_token_count {
        settings.trigger_token_count = Some(value);
        settings.trigger_char_count = Some(
            (value as f64
                * args
                    .message_compaction_chars_per_token
                    .unwrap_or(settings.chars_per_token))
            .floor() as usize,
        );
    }
    if let Some(value) = args.message_compaction_chars_per_token {
        settings.chars_per_token = value;
        if let Some(tokens) = settings.trigger_token_count {
            settings.trigger_char_count = Some((tokens as f64 * value).floor() as usize);
        }
    }
    if let Some(value) = args.message_compaction_keep_last_items {
        settings.keep_last_items = value;
    }
    if let Some(value) = args.message_compaction_summary_max_chars {
        settings.summary_max_chars = value;
    }
    if args.message_compaction_include_read_state {
        settings.include_read_state = true;
    }
    Some(settings)
}

fn cli_generate_gif(value: String) -> GenerateGif {
    match value.trim().to_ascii_lowercase().as_str() {
        "true" => GenerateGif::Enabled,
        "false" => GenerateGif::Disabled,
        _ => GenerateGif::Path(value),
    }
}

fn parse_json_value(value: &str) -> Result<Value, String> {
    serde_json::from_str(value).map_err(|error| format!("expected JSON value: {error}"))
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SensitiveDataEntry {
    placeholder: String,
    value: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DomainSensitiveDataEntry {
    domain_pattern: String,
    placeholder: String,
    value: String,
}

fn parse_sensitive_data_entry(value: &str) -> Result<SensitiveDataEntry, String> {
    let (placeholder, secret) = value
        .split_once('=')
        .ok_or_else(|| "expected placeholder=value".to_owned())?;
    let placeholder = placeholder.trim();
    if placeholder.is_empty() {
        return Err("placeholder cannot be empty".to_owned());
    }

    Ok(SensitiveDataEntry {
        placeholder: placeholder.to_owned(),
        value: secret.to_owned(),
    })
}

fn parse_domain_sensitive_data_entry(value: &str) -> Result<DomainSensitiveDataEntry, String> {
    let (domain_pattern, rest) = value
        .split_once('=')
        .ok_or_else(|| "expected domain-pattern=placeholder=value".to_owned())?;
    let domain_pattern = domain_pattern.trim();
    if domain_pattern.is_empty() {
        return Err("domain pattern cannot be empty".to_owned());
    }
    let (placeholder, secret) = rest
        .split_once('=')
        .ok_or_else(|| "expected domain-pattern=placeholder=value".to_owned())?;
    let placeholder = placeholder.trim();
    if placeholder.is_empty() {
        return Err("placeholder cannot be empty".to_owned());
    }

    Ok(DomainSensitiveDataEntry {
        domain_pattern: domain_pattern.to_owned(),
        placeholder: placeholder.to_owned(),
        value: secret.to_owned(),
    })
}

fn cli_sensitive_data(
    entries: Vec<SensitiveDataEntry>,
    domain_entries: Vec<DomainSensitiveDataEntry>,
) -> BTreeMap<String, SensitiveDataValue> {
    let mut sensitive_data = BTreeMap::new();
    for entry in entries {
        sensitive_data.insert(entry.placeholder, SensitiveDataValue::Value(entry.value));
    }
    for entry in domain_entries {
        let value = sensitive_data
            .entry(entry.domain_pattern)
            .or_insert_with(|| SensitiveDataValue::Domain(BTreeMap::new()));
        let SensitiveDataValue::Domain(values) = value else {
            *value = SensitiveDataValue::Domain(BTreeMap::new());
            let SensitiveDataValue::Domain(values) = value else {
                unreachable!("domain value was just inserted");
            };
            values.insert(entry.placeholder, entry.value);
            continue;
        };
        values.insert(entry.placeholder, entry.value);
    }

    sensitive_data
}

fn configured_chat_model(
    provider: LlmProvider,
    api_key: Option<String>,
    model: Option<String>,
    base_url: Option<String>,
    structured_output_mode_override: Option<OpenAiStructuredOutputMode>,
) -> anyhow::Result<Box<dyn ChatModel>> {
    match provider {
        LlmProvider::OpenAiCompatible
        | LlmProvider::DeepSeek
        | LlmProvider::Groq
        | LlmProvider::Cerebras
        | LlmProvider::Mistral
        | LlmProvider::OpenRouter
        | LlmProvider::Vercel => configured_openai_wire_chat_model(
            openai_wire_provider_config(provider),
            api_key,
            model,
            base_url,
            structured_output_mode_override,
        ),
        LlmProvider::Anthropic => {
            let api_key = api_key
                .or_else(|| nonempty_env("ANTHROPIC_API_KEY"))
                .ok_or_else(|| anyhow::anyhow!("ANTHROPIC_API_KEY or --api-key is required"))?;
            let model = model
                .or_else(|| nonempty_env("ANTHROPIC_MODEL"))
                .ok_or_else(|| anyhow::anyhow!("ANTHROPIC_MODEL or --model is required"))?;
            let base_url = base_url
                .or_else(|| nonempty_env("ANTHROPIC_BASE_URL"))
                .unwrap_or_else(|| "https://api.anthropic.com/v1".to_owned());
            let mut llm = AnthropicChatModel::new(api_key, model).with_base_url(base_url);
            if let Some(version) = nonempty_env("ANTHROPIC_VERSION") {
                llm = llm.with_anthropic_version(version);
            }
            if let Some(max_tokens) = nonempty_env("ANTHROPIC_MAX_TOKENS") {
                llm = llm.with_max_tokens(max_tokens.parse()?);
            }
            Ok(Box::new(llm))
        }
        LlmProvider::Gemini => {
            let api_key = api_key
                .or_else(|| nonempty_env("GEMINI_API_KEY"))
                .ok_or_else(|| anyhow::anyhow!("GEMINI_API_KEY or --api-key is required"))?;
            let model = model
                .or_else(|| nonempty_env("GEMINI_MODEL"))
                .ok_or_else(|| anyhow::anyhow!("GEMINI_MODEL or --model is required"))?;
            let base_url = base_url
                .or_else(|| nonempty_env("GEMINI_BASE_URL"))
                .unwrap_or_else(|| "https://generativelanguage.googleapis.com/v1beta".to_owned());
            Ok(Box::new(
                GeminiChatModel::new(api_key, model).with_base_url(base_url),
            ))
        }
        LlmProvider::Ollama => {
            let model = model
                .or_else(|| nonempty_env("OLLAMA_MODEL"))
                .ok_or_else(|| anyhow::anyhow!("OLLAMA_MODEL or --model is required"))?;
            let base_url = base_url
                .or_else(|| nonempty_env("OLLAMA_BASE_URL"))
                .or_else(|| nonempty_env("OLLAMA_HOST"))
                .unwrap_or_else(|| "http://localhost:11434".to_owned());
            Ok(Box::new(
                OllamaChatModel::new(model).with_base_url(base_url),
            ))
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct OpenAiWireProviderConfig {
    provider_name: &'static str,
    api_key_env: &'static [&'static str],
    model_env: &'static [&'static str],
    base_url_env: &'static [&'static str],
    default_headers: &'static [OpenAiWireDefaultHeader],
    default_model: Option<&'static str>,
    default_base_url: &'static str,
    structured_output_mode: OpenAiStructuredOutputMode,
    schema_transform: OpenAiSchemaTransform,
}

#[derive(Debug, Clone, Copy)]
struct OpenAiWireDefaultHeader {
    name: &'static str,
    value_env: &'static [&'static str],
}

fn configured_openai_wire_chat_model(
    config: OpenAiWireProviderConfig,
    api_key: Option<String>,
    model: Option<String>,
    base_url: Option<String>,
    structured_output_mode_override: Option<OpenAiStructuredOutputMode>,
) -> anyhow::Result<Box<dyn ChatModel>> {
    let api_key = api_key
        .or_else(|| first_nonempty_env(config.api_key_env))
        .ok_or_else(|| {
            anyhow::anyhow!(
                "{} or --api-key is required",
                provider_env_list(config.api_key_env)
            )
        })?;
    let model = model
        .or_else(|| first_nonempty_env(config.model_env))
        .or_else(|| config.default_model.map(ToOwned::to_owned))
        .ok_or_else(|| {
            anyhow::anyhow!(
                "{} or --model is required",
                provider_env_list(config.model_env)
            )
        })?;
    let base_url = base_url
        .or_else(|| first_nonempty_env(config.base_url_env))
        .unwrap_or_else(|| config.default_base_url.to_owned());
    let structured_output_mode =
        default_structured_output_mode(config, &model, structured_output_mode_override);

    let mut llm = OpenAiCompatibleChatModel::new(api_key, model)
        .with_base_url(base_url)
        .with_provider_name(config.provider_name)
        .with_structured_output_mode(structured_output_mode)
        .with_schema_transform(config.schema_transform);
    for (name, value) in openai_wire_default_headers(config, first_nonempty_env) {
        llm = llm.try_with_default_header(name, value)?;
    }

    Ok(Box::new(llm))
}

fn default_structured_output_mode(
    config: OpenAiWireProviderConfig,
    model: &str,
    override_mode: Option<OpenAiStructuredOutputMode>,
) -> OpenAiStructuredOutputMode {
    if let Some(mode) = override_mode {
        return mode;
    }

    match config.provider_name {
        "groq" if model == "moonshotai/kimi-k2-instruct" => OpenAiStructuredOutputMode::ToolCall,
        "vercel" if vercel_prompt_fallback_model(model) => OpenAiStructuredOutputMode::PromptOnly,
        _ => config.structured_output_mode,
    }
}

fn vercel_prompt_fallback_model(model: &str) -> bool {
    let lower = model.to_ascii_lowercase();
    lower.starts_with("google/")
        || lower.starts_with("anthropic/")
        || [
            "o1",
            "o3",
            "o4",
            "gpt-oss",
            "gpt-5.2-pro",
            "gpt-5.4-pro",
            "deepseek-r1",
            "-thinking",
            "perplexity/sonar-reasoning",
        ]
        .iter()
        .any(|pattern| lower.contains(pattern))
}

fn openai_wire_default_headers<F>(
    config: OpenAiWireProviderConfig,
    lookup: F,
) -> Vec<(&'static str, String)>
where
    F: Fn(&[&str]) -> Option<String>,
{
    config
        .default_headers
        .iter()
        .filter_map(|header| lookup(header.value_env).map(|value| (header.name, value)))
        .collect()
}

fn openai_wire_provider_config(provider: LlmProvider) -> OpenAiWireProviderConfig {
    match provider {
        LlmProvider::OpenAiCompatible => OpenAiWireProviderConfig {
            provider_name: "openai-compatible",
            api_key_env: &["OPENAI_API_KEY"],
            model_env: &["OPENAI_MODEL"],
            base_url_env: &["OPENAI_BASE_URL"],
            default_headers: &[],
            default_model: None,
            default_base_url: "https://api.openai.com/v1",
            structured_output_mode: OpenAiStructuredOutputMode::JsonSchema,
            schema_transform: OpenAiSchemaTransform::Default,
        },
        LlmProvider::DeepSeek => OpenAiWireProviderConfig {
            provider_name: "deepseek",
            api_key_env: &["DEEPSEEK_API_KEY"],
            model_env: &["DEEPSEEK_MODEL"],
            base_url_env: &["DEEPSEEK_BASE_URL"],
            default_headers: &[],
            default_model: Some("deepseek-chat"),
            default_base_url: "https://api.deepseek.com/v1",
            structured_output_mode: OpenAiStructuredOutputMode::ToolCall,
            schema_transform: OpenAiSchemaTransform::Default,
        },
        LlmProvider::Groq => OpenAiWireProviderConfig {
            provider_name: "groq",
            api_key_env: &["GROQ_API_KEY"],
            model_env: &["GROQ_MODEL"],
            base_url_env: &["GROQ_BASE_URL"],
            default_headers: &[],
            default_model: None,
            default_base_url: "https://api.groq.com/openai/v1",
            structured_output_mode: OpenAiStructuredOutputMode::JsonSchema,
            schema_transform: OpenAiSchemaTransform::Default,
        },
        LlmProvider::Cerebras => OpenAiWireProviderConfig {
            provider_name: "cerebras",
            api_key_env: &["CEREBRAS_API_KEY"],
            model_env: &["CEREBRAS_MODEL"],
            base_url_env: &["CEREBRAS_BASE_URL"],
            default_headers: &[],
            default_model: Some("llama3.1-8b"),
            default_base_url: "https://api.cerebras.ai/v1",
            structured_output_mode: OpenAiStructuredOutputMode::PromptOnly,
            schema_transform: OpenAiSchemaTransform::Default,
        },
        LlmProvider::Mistral => OpenAiWireProviderConfig {
            provider_name: "mistral",
            api_key_env: &["MISTRAL_API_KEY"],
            model_env: &["MISTRAL_MODEL"],
            base_url_env: &["MISTRAL_BASE_URL"],
            default_headers: &[],
            default_model: Some("mistral-medium-latest"),
            default_base_url: "https://api.mistral.ai/v1",
            structured_output_mode: OpenAiStructuredOutputMode::JsonSchema,
            schema_transform: OpenAiSchemaTransform::MistralCompatible,
        },
        LlmProvider::OpenRouter => OpenAiWireProviderConfig {
            provider_name: "openrouter",
            api_key_env: &["OPENROUTER_API_KEY"],
            model_env: &["OPENROUTER_MODEL"],
            base_url_env: &["OPENROUTER_BASE_URL"],
            default_headers: &[
                OpenAiWireDefaultHeader {
                    name: "HTTP-Referer",
                    value_env: &["OPENROUTER_HTTP_REFERER", "OPENROUTER_APP_URL"],
                },
                OpenAiWireDefaultHeader {
                    name: "X-Title",
                    value_env: &["OPENROUTER_X_TITLE", "OPENROUTER_APP_TITLE"],
                },
                OpenAiWireDefaultHeader {
                    name: "X-OpenRouter-Title",
                    value_env: &["OPENROUTER_X_TITLE", "OPENROUTER_APP_TITLE"],
                },
            ],
            default_model: None,
            default_base_url: "https://openrouter.ai/api/v1",
            structured_output_mode: OpenAiStructuredOutputMode::JsonSchema,
            schema_transform: OpenAiSchemaTransform::Default,
        },
        LlmProvider::Vercel => OpenAiWireProviderConfig {
            provider_name: "vercel",
            api_key_env: &["AI_GATEWAY_API_KEY", "VERCEL_OIDC_TOKEN"],
            model_env: &["AI_GATEWAY_MODEL", "VERCEL_MODEL"],
            base_url_env: &["AI_GATEWAY_BASE_URL"],
            default_headers: &[],
            default_model: None,
            default_base_url: "https://ai-gateway.vercel.sh/v1",
            structured_output_mode: OpenAiStructuredOutputMode::JsonSchema,
            schema_transform: OpenAiSchemaTransform::Default,
        },
        LlmProvider::Anthropic | LlmProvider::Gemini | LlmProvider::Ollama => {
            unreachable!("non-OpenAI-wire provider")
        }
    }
}

fn first_nonempty_env(names: &[&str]) -> Option<String> {
    names.iter().find_map(|name| nonempty_env(name))
}

fn provider_env_list(names: &[&str]) -> String {
    names.join(", ")
}

fn nonempty_env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|value| !value.is_empty())
}

#[cfg(test)]
mod tests;
