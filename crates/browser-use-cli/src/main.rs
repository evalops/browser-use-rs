use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::process::Command as StdCommand;
use std::sync::Arc;
use std::time::{Duration, Instant};

use base64::Engine;
use browser_use_cdp::{BrowserProfile, BrowserSession, CdpBrowserSession};
use browser_use_core::{
    AgentHistory, AgentSettings, BrowserActionExecutor, ImageDetailLevel, SensitiveDataValue,
    VisionMode,
};
use browser_use_llm::{
    AnthropicChatModel, ChatModel, GeminiChatModel, OllamaChatModel, OpenAiCompatibleChatModel,
    OpenAiStructuredOutputMode,
};
use clap::Parser;
use schemars::schema_for;
use serde_json::Value;
use tokio::io::{self, AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio::time::sleep;

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
        #[arg(long)]
        max_actions_per_step: Option<usize>,
        #[arg(long)]
        llm_timeout_seconds: Option<u64>,
        #[arg(long)]
        step_timeout_seconds: Option<u64>,
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
        #[arg(long = "save-conversation-path")]
        save_conversation_path: Option<String>,
        #[arg(long = "save-conversation-path-encoding")]
        save_conversation_path_encoding: Option<String>,
        #[arg(long, default_value_t = false)]
        no_planning: bool,
        #[arg(long)]
        planning_replan_on_stall: Option<usize>,
        #[arg(long)]
        planning_exploration_limit: Option<usize>,
        #[arg(long)]
        max_history_items: Option<usize>,
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

#[derive(Debug, clap::Subcommand)]
enum SessionCommand {
    /// Launch a persistent Chrome session and navigate it to a URL.
    Start {
        id: String,
        url: String,
        #[arg(long, default_value_t = false)]
        screenshot: bool,
    },
    /// Print state for an existing persistent session.
    State {
        id: String,
        #[arg(long, default_value_t = false)]
        screenshot: bool,
    },
    /// Run a JSON action list against an existing persistent session.
    Actions {
        id: String,
        actions: PathBuf,
        #[arg(long, default_value_t = true)]
        screenshot: bool,
    },
    /// Replay serialized AgentHistory against an existing persistent session.
    Replay { id: String, history: PathBuf },
    /// Stop an existing persistent session.
    Stop { id: String },
    /// List recorded persistent sessions.
    List,
    /// Remove stale persistent session records, or force-clean a specific record.
    Cleanup {
        id: Option<String>,
        #[arg(long, default_value_t = false)]
        force: bool,
    },
}

type StoredSession = browser_use_mcp::SessionRecord;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SessionCleanupDecision {
    RemoveRecord,
    StopRunning,
    SkipRunning,
    SkipUnknown,
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
            max_actions_per_step,
            llm_timeout_seconds,
            step_timeout_seconds,
            no_final_response_after_failure,
            no_display_files_in_done_text,
            no_loop_detection,
            loop_detection_window,
            no_thinking,
            flash_mode,
            no_judge,
            ground_truth,
            save_conversation_path,
            save_conversation_path_encoding,
            no_planning,
            planning_replan_on_stall,
            planning_exploration_limit,
            max_history_items,
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
                max_actions_per_step,
                llm_timeout_seconds,
                step_timeout_seconds,
                no_final_response_after_failure,
                no_display_files_in_done_text,
                no_loop_detection,
                loop_detection_window,
                no_thinking,
                flash_mode,
                no_judge,
                ground_truth,
                save_conversation_path,
                save_conversation_path_encoding,
                no_planning,
                planning_replan_on_stall,
                planning_exploration_limit,
                max_history_items,
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

fn read_agent_history(path: &PathBuf) -> anyhow::Result<AgentHistory> {
    let history = std::fs::read_to_string(path)?;
    Ok(serde_json::from_str(&history)?)
}

#[derive(Debug, Default)]
struct CliAgentSettingsArgs {
    no_vision: bool,
    vision_mode: Option<CliVisionMode>,
    vision_detail_level: Option<CliVisionDetailLevel>,
    max_failures: Option<u32>,
    max_actions_per_step: Option<usize>,
    llm_timeout_seconds: Option<u64>,
    step_timeout_seconds: Option<u64>,
    no_final_response_after_failure: bool,
    no_display_files_in_done_text: bool,
    no_loop_detection: bool,
    loop_detection_window: Option<usize>,
    no_thinking: bool,
    flash_mode: bool,
    no_judge: bool,
    ground_truth: Option<String>,
    save_conversation_path: Option<String>,
    save_conversation_path_encoding: Option<String>,
    no_planning: bool,
    planning_replan_on_stall: Option<usize>,
    planning_exploration_limit: Option<usize>,
    max_history_items: Option<usize>,
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
    if let Some(value) = args.max_actions_per_step {
        settings.max_actions_per_step = value;
    }
    if let Some(value) = args.llm_timeout_seconds {
        settings.llm_timeout_seconds = value;
    }
    if let Some(value) = args.step_timeout_seconds {
        settings.step_timeout_seconds = value;
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
    settings.save_conversation_path = args.save_conversation_path;
    if let Some(value) = args.save_conversation_path_encoding {
        settings.save_conversation_path_encoding = Some(value);
    }
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

async fn start_persistent_session(
    id: &str,
    url: &str,
    screenshot: bool,
) -> anyhow::Result<(
    StoredSession,
    CdpBrowserSession,
    browser_use_dom::BrowserStateSummary,
)> {
    validate_session_id(id)?;
    let path = session_record_path(id)?;
    if path.exists() {
        anyhow::bail!("session already exists: {id}");
    }
    let user_data_dir = session_user_data_dir(id)?;
    std::fs::create_dir_all(&user_data_dir)?;
    let profile = BrowserProfile {
        user_data_dir: Some(user_data_dir.clone()),
        ..BrowserProfile::default()
    };
    let launched = profile.launch_local().await?;
    let endpoint = launched.endpoint().clone();
    let process_id = launched.process_id();
    let session = CdpBrowserSession::connect(endpoint.clone()).await?;
    session.navigate(url, false).await?;
    sleep(Duration::from_millis(150)).await;
    let state = session.state(screenshot).await?;
    let record = StoredSession {
        id: id.to_owned(),
        endpoint,
        user_data_dir,
        process_id,
        status: None,
    };
    write_session_record(&record)?;
    let _ = launched.detach();
    Ok((annotate_session_status(record), session, state))
}

async fn stop_persistent_session(id: &str) -> anyhow::Result<StoredSession> {
    let mut record = read_session_record(id)?;
    if let Ok(session) = CdpBrowserSession::connect(record.endpoint.clone()).await {
        let _ = session.close_browser().await;
    }
    wait_for_process_exit(record.process_id, Duration::from_secs(2)).await;
    remove_session_dir(id)?;
    record.status = Some(browser_use_mcp::SessionStatus::Stopped);
    Ok(record)
}

async fn cleanup_persistent_sessions(
    id: Option<&str>,
    force: bool,
) -> anyhow::Result<Vec<browser_use_mcp::SessionCleanupRecord>> {
    let records = if let Some(id) = id {
        vec![annotate_session_status(read_session_record(id)?)]
    } else {
        list_session_records()?
    };
    let mut cleaned = Vec::new();

    for record in records {
        match session_cleanup_decision(&record, force, process_is_running) {
            SessionCleanupDecision::RemoveRecord => {
                remove_session_dir(&record.id)?;
                cleaned.push(browser_use_mcp::SessionCleanupRecord {
                    action: browser_use_mcp::SessionCleanupAction::Removed,
                    session: record,
                });
            }
            SessionCleanupDecision::StopRunning => {
                let record = stop_persistent_session(&record.id).await?;
                cleaned.push(browser_use_mcp::SessionCleanupRecord {
                    action: browser_use_mcp::SessionCleanupAction::Stopped,
                    session: record,
                });
            }
            SessionCleanupDecision::SkipRunning if id.is_some() => {
                anyhow::bail!(
                    "session {} is running; use session stop or pass --force",
                    record.id
                );
            }
            SessionCleanupDecision::SkipUnknown if id.is_some() => {
                anyhow::bail!(
                    "session {} has unknown liveness; pass --force to remove the record",
                    record.id
                );
            }
            SessionCleanupDecision::SkipRunning | SessionCleanupDecision::SkipUnknown => {}
        }
    }

    Ok(cleaned)
}

fn session_cleanup_decision(
    record: &StoredSession,
    force: bool,
    is_running: impl Fn(u32) -> bool,
) -> SessionCleanupDecision {
    match session_status_with_checker(record, is_running) {
        browser_use_mcp::SessionStatus::Running if force => SessionCleanupDecision::StopRunning,
        browser_use_mcp::SessionStatus::Running => SessionCleanupDecision::SkipRunning,
        browser_use_mcp::SessionStatus::Stale | browser_use_mcp::SessionStatus::Stopped => {
            SessionCleanupDecision::RemoveRecord
        }
        browser_use_mcp::SessionStatus::Unknown if force => SessionCleanupDecision::RemoveRecord,
        browser_use_mcp::SessionStatus::Unknown => SessionCleanupDecision::SkipUnknown,
    }
}

async fn run_session_command(command: SessionCommand) -> anyhow::Result<()> {
    match command {
        SessionCommand::Start {
            id,
            url,
            screenshot,
        } => {
            let (record, _session, state) = start_persistent_session(&id, &url, screenshot).await?;
            println!(
                "{}",
                serde_json::to_string_pretty(&serde_json::json!({
                    "session": record,
                    "state": state
                }))?
            );
        }
        SessionCommand::State { id, screenshot } => {
            let record = annotate_session_status(read_session_record(&id)?);
            let session = CdpBrowserSession::connect(record.endpoint).await?;
            print_state(&session, screenshot).await?;
        }
        SessionCommand::Actions {
            id,
            actions,
            screenshot,
        } => {
            let record = annotate_session_status(read_session_record(&id)?);
            let session = CdpBrowserSession::connect(record.endpoint.clone()).await?;
            let actions = std::fs::read_to_string(&actions)?;
            let actions: Vec<browser_use_tools::BrowserAction> = serde_json::from_str(&actions)?;
            let mut executor = BrowserActionExecutor::new(session);
            let results = executor.execute_sequence(&actions).await;
            let state = executor.session().state(screenshot).await?;
            println!(
                "{}",
                serde_json::to_string_pretty(&serde_json::json!({
                    "session": record,
                    "results": results,
                    "state": state,
                }))?
            );
        }
        SessionCommand::Replay { id, history } => {
            let record = annotate_session_status(read_session_record(&id)?);
            let session = CdpBrowserSession::connect(record.endpoint.clone()).await?;
            let history = read_agent_history(&history)?;
            let mut executor = BrowserActionExecutor::new(session);
            let replay = executor.replay_history(&history).await?;
            println!(
                "{}",
                serde_json::to_string_pretty(&serde_json::json!({
                    "session": record,
                    "replay": replay,
                }))?
            );
        }
        SessionCommand::Stop { id } => {
            let record = stop_persistent_session(&id).await?;
            println!("{}", serde_json::to_string_pretty(&record)?);
        }
        SessionCommand::List => {
            println!(
                "{}",
                serde_json::to_string_pretty(&list_session_records()?)?
            );
        }
        SessionCommand::Cleanup { id, force } => {
            let cleaned = cleanup_persistent_sessions(id.as_deref(), force).await?;
            let remaining = list_session_records()?;
            println!(
                "{}",
                serde_json::to_string_pretty(&serde_json::json!({
                    "cleaned_sessions": cleaned,
                    "sessions": remaining,
                }))?
            );
        }
    }

    Ok(())
}

fn validate_session_id(id: &str) -> anyhow::Result<()> {
    if id.is_empty()
        || !id
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_')
    {
        anyhow::bail!("session id must contain only ASCII letters, digits, '-' or '_'");
    }
    Ok(())
}

fn state_dir() -> anyhow::Result<PathBuf> {
    if let Some(path) = std::env::var_os("BROWSER_USE_RS_STATE_DIR") {
        return Ok(PathBuf::from(path));
    }
    let home = std::env::var_os("HOME").ok_or_else(|| anyhow::anyhow!("HOME is not set"))?;
    Ok(PathBuf::from(home).join(".browser-use-rs"))
}

fn sessions_dir() -> anyhow::Result<PathBuf> {
    Ok(state_dir()?.join("sessions"))
}

fn session_dir(id: &str) -> anyhow::Result<PathBuf> {
    validate_session_id(id)?;
    Ok(sessions_dir()?.join(id))
}

fn session_user_data_dir(id: &str) -> anyhow::Result<PathBuf> {
    Ok(session_dir(id)?.join("profile"))
}

fn session_record_path(id: &str) -> anyhow::Result<PathBuf> {
    Ok(session_dir(id)?.join("session.json"))
}

fn write_session_record(record: &StoredSession) -> anyhow::Result<()> {
    let path = session_record_path(&record.id)?;
    let parent = path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("session path has no parent"))?;
    let mut stored_record = record.clone();
    stored_record.status = None;
    std::fs::create_dir_all(parent)?;
    std::fs::write(path, serde_json::to_vec_pretty(&stored_record)?)?;
    Ok(())
}

fn read_session_record(id: &str) -> anyhow::Result<StoredSession> {
    let path = session_record_path(id)?;
    let contents = std::fs::read_to_string(&path)?;
    Ok(serde_json::from_str(&contents)?)
}

fn remove_session_dir(id: &str) -> anyhow::Result<()> {
    let path = session_dir(id)?;
    if path.exists() {
        std::fs::remove_dir_all(path)?;
    }
    Ok(())
}

fn list_session_records() -> anyhow::Result<Vec<StoredSession>> {
    let dir = sessions_dir()?;
    if !dir.exists() {
        return Ok(Vec::new());
    }

    let mut records = Vec::new();
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            continue;
        }
        let id = entry.file_name().to_string_lossy().to_string();
        if let Ok(record) = read_session_record(&id) {
            records.push(annotate_session_status(record));
        }
    }
    records.sort_by(|left, right| left.id.cmp(&right.id));
    Ok(records)
}

fn annotate_session_status(mut record: StoredSession) -> StoredSession {
    record.status = Some(session_status(&record));
    record
}

fn session_status(record: &StoredSession) -> browser_use_mcp::SessionStatus {
    session_status_with_checker(record, process_is_running)
}

fn session_status_with_checker(
    record: &StoredSession,
    is_running: impl Fn(u32) -> bool,
) -> browser_use_mcp::SessionStatus {
    match record.process_id {
        Some(process_id) if is_running(process_id) => browser_use_mcp::SessionStatus::Running,
        Some(_) => browser_use_mcp::SessionStatus::Stale,
        None => browser_use_mcp::SessionStatus::Unknown,
    }
}

async fn wait_for_process_exit(process_id: Option<u32>, timeout: Duration) {
    let Some(process_id) = process_id else {
        return;
    };
    let deadline = Instant::now() + timeout;
    while process_is_running(process_id) && Instant::now() < deadline {
        sleep(Duration::from_millis(50)).await;
    }
}

#[cfg(unix)]
fn process_is_running(process_id: u32) -> bool {
    StdCommand::new("kill")
        .arg("-0")
        .arg(process_id.to_string())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

#[cfg(not(unix))]
fn process_is_running(_process_id: u32) -> bool {
    false
}

async fn run_mcp_stdio() -> anyhow::Result<()> {
    let stdin = BufReader::new(io::stdin());
    let mut lines = stdin.lines();
    let mut stdout = io::stdout();
    let mut runtime = McpRuntime::default();

    while let Some(line) = lines.next_line().await? {
        if line.trim().is_empty() {
            continue;
        }
        if let Some(response) = handle_mcp_message(&line, &mut runtime).await {
            let mut encoded = serde_json::to_vec(&response)?;
            encoded.push(b'\n');
            stdout.write_all(&encoded).await?;
            stdout.flush().await?;
        }
    }

    Ok(())
}

async fn run_daemon(
    addr: &str,
    transport: DaemonTransport,
    auth_token: Option<String>,
    lifecycle_options: DaemonLifecycleOptions,
) -> anyhow::Result<()> {
    match transport {
        DaemonTransport::Tcp => run_tcp_daemon(addr, lifecycle_options).await,
        DaemonTransport::Http => run_http_daemon(addr, auth_token, lifecycle_options).await,
    }
}

#[derive(Debug, Clone, Default)]
struct DaemonLifecycleOptions {
    pid_file: Option<PathBuf>,
    ready_file: Option<PathBuf>,
}

struct DaemonLifecycleFiles {
    paths: Vec<PathBuf>,
}

impl DaemonLifecycleFiles {
    fn write(
        options: DaemonLifecycleOptions,
        transport: DaemonTransport,
        addr: &str,
    ) -> anyhow::Result<Self> {
        let mut paths = Vec::new();
        let pid = std::process::id();
        if let Some(path) = options.pid_file {
            write_supervisor_file(&path, format!("{pid}\n").as_bytes())?;
            paths.push(path);
        }
        if let Some(path) = options.ready_file {
            let ready = serde_json::json!({
                "ready": true,
                "pid": pid,
                "addr": addr,
                "transport": transport.as_str(),
            });
            write_supervisor_file(&path, serde_json::to_vec_pretty(&ready)?.as_slice())?;
            paths.push(path);
        }

        Ok(Self { paths })
    }
}

impl Drop for DaemonLifecycleFiles {
    fn drop(&mut self) {
        for path in &self.paths {
            let _ = std::fs::remove_file(path);
        }
    }
}

fn write_supervisor_file(path: &PathBuf, contents: &[u8]) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, contents)?;
    Ok(())
}

async fn run_tcp_daemon(
    addr: &str,
    lifecycle_options: DaemonLifecycleOptions,
) -> anyhow::Result<()> {
    let listener = TcpListener::bind(addr).await?;
    let local_addr = listener.local_addr()?.to_string();
    println!("{local_addr}");
    let _lifecycle =
        DaemonLifecycleFiles::write(lifecycle_options, DaemonTransport::Tcp, &local_addr)?;
    let runtime = Arc::new(tokio::sync::Mutex::new(McpRuntime::default()));
    let shutdown = shutdown_signal();
    tokio::pin!(shutdown);

    loop {
        tokio::select! {
            () = &mut shutdown => return Ok(()),
            accepted = listener.accept() => {
                let (stream, _) = accepted?;
                let runtime = Arc::clone(&runtime);
                tokio::spawn(async move {
                    let _ = handle_daemon_connection(stream, runtime).await;
                });
            }
        }
    }
}

async fn handle_daemon_connection(
    stream: TcpStream,
    runtime: Arc<tokio::sync::Mutex<McpRuntime>>,
) -> anyhow::Result<()> {
    let (reader, mut writer) = stream.into_split();
    let mut lines = BufReader::new(reader).lines();

    while let Some(line) = lines.next_line().await? {
        if line.trim().is_empty() {
            continue;
        }
        let response = {
            let mut runtime = runtime.lock().await;
            handle_mcp_message(&line, &mut runtime).await
        };
        if let Some(response) = response {
            let mut encoded = serde_json::to_vec(&response)?;
            encoded.push(b'\n');
            writer.write_all(&encoded).await?;
            writer.flush().await?;
        }
    }

    Ok(())
}

async fn run_http_daemon(
    addr: &str,
    auth_token: Option<String>,
    lifecycle_options: DaemonLifecycleOptions,
) -> anyhow::Result<()> {
    let listener = TcpListener::bind(addr).await?;
    let local_addr = listener.local_addr()?.to_string();
    println!("{local_addr}");
    let _lifecycle =
        DaemonLifecycleFiles::write(lifecycle_options, DaemonTransport::Http, &local_addr)?;
    let runtime = Arc::new(tokio::sync::Mutex::new(McpRuntime::default()));
    let auth_token = auth_token.map(Arc::new);
    let shutdown = shutdown_signal();
    tokio::pin!(shutdown);

    loop {
        tokio::select! {
            () = &mut shutdown => return Ok(()),
            accepted = listener.accept() => {
                let (stream, _) = accepted?;
                let runtime = Arc::clone(&runtime);
                let auth_token = auth_token.clone();
                tokio::spawn(async move {
                    let _ = handle_http_daemon_connection(stream, runtime, auth_token).await;
                });
            }
        }
    }
}

async fn shutdown_signal() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};

        let mut terminate = signal(SignalKind::terminate()).ok();
        let mut interrupt = signal(SignalKind::interrupt()).ok();

        tokio::select! {
            _ = tokio::signal::ctrl_c() => {}
            _ = async {
                if let Some(signal) = terminate.as_mut() {
                    signal.recv().await;
                } else {
                    std::future::pending::<()>().await;
                }
            } => {}
            _ = async {
                if let Some(signal) = interrupt.as_mut() {
                    signal.recv().await;
                } else {
                    std::future::pending::<()>().await;
                }
            } => {}
        }
    }

    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}

async fn handle_http_daemon_connection(
    mut stream: TcpStream,
    runtime: Arc<tokio::sync::Mutex<McpRuntime>>,
    auth_token: Option<Arc<String>>,
) -> anyhow::Result<()> {
    let request = read_http_request(&mut stream).await?;
    let response = {
        let mut runtime = runtime.lock().await;
        handle_http_request(
            request,
            &mut runtime,
            auth_token.as_deref().map(String::as_str),
        )
        .await
    };
    stream.write_all(&response.to_bytes()).await?;
    stream.flush().await?;
    Ok(())
}

struct HttpRequest {
    method: String,
    path: String,
    headers: HashMap<String, String>,
    body: Vec<u8>,
}

struct HttpResponse {
    status: u16,
    body: Vec<u8>,
}

impl HttpResponse {
    fn json(status: u16, value: Value) -> Self {
        Self {
            status,
            body: serde_json::to_vec(&value).unwrap_or_else(|_| b"{}".to_vec()),
        }
    }

    fn to_bytes(&self) -> Vec<u8> {
        let reason = http_reason(self.status);
        let mut response = format!(
            "HTTP/1.1 {} {reason}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
            self.status,
            self.body.len()
        )
        .into_bytes();
        response.extend_from_slice(&self.body);
        response
    }
}

async fn read_http_request(stream: &mut TcpStream) -> anyhow::Result<HttpRequest> {
    const MAX_HEADER_BYTES: usize = 16 * 1024;
    let mut buffer = Vec::new();
    let mut chunk = [0_u8; 1024];

    let header_end = loop {
        if let Some(index) = find_http_header_end(&buffer) {
            break index;
        }
        if buffer.len() >= MAX_HEADER_BYTES {
            anyhow::bail!("HTTP headers exceeded {MAX_HEADER_BYTES} bytes");
        }
        let read = stream.read(&mut chunk).await?;
        if read == 0 {
            anyhow::bail!("connection closed before complete HTTP headers");
        }
        buffer.extend_from_slice(&chunk[..read]);
    };

    let header_text = std::str::from_utf8(&buffer[..header_end])?;
    let mut lines = header_text.split("\r\n");
    let request_line = lines
        .next()
        .ok_or_else(|| anyhow::anyhow!("missing HTTP request line"))?;
    let mut request_parts = request_line.split_whitespace();
    let method = request_parts
        .next()
        .ok_or_else(|| anyhow::anyhow!("missing HTTP method"))?
        .to_owned();
    let path = request_parts
        .next()
        .ok_or_else(|| anyhow::anyhow!("missing HTTP path"))?
        .to_owned();

    let mut headers = HashMap::new();
    for line in lines {
        if let Some((name, value)) = line.split_once(':') {
            headers.insert(name.trim().to_ascii_lowercase(), value.trim().to_owned());
        }
    }

    let content_length = headers
        .get("content-length")
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or_default();
    let body_start = header_end + 4;
    let mut body = buffer.get(body_start..).unwrap_or_default().to_vec();
    while body.len() < content_length {
        let read = stream.read(&mut chunk).await?;
        if read == 0 {
            anyhow::bail!("connection closed before complete HTTP body");
        }
        body.extend_from_slice(&chunk[..read]);
    }
    body.truncate(content_length);

    Ok(HttpRequest {
        method,
        path,
        headers,
        body,
    })
}

fn find_http_header_end(buffer: &[u8]) -> Option<usize> {
    buffer.windows(4).position(|window| window == b"\r\n\r\n")
}

async fn handle_http_request(
    request: HttpRequest,
    runtime: &mut McpRuntime,
    auth_token: Option<&str>,
) -> HttpResponse {
    if request.method == "GET" && request.path == "/healthz" {
        return HttpResponse::json(200, serde_json::json!({ "ok": true }));
    }

    if request.method != "POST" || request.path != "/rpc" {
        return HttpResponse::json(
            404,
            serde_json::json!({ "error": "not_found", "message": "use POST /rpc" }),
        );
    }

    if !http_request_authorized(&request, auth_token) {
        return HttpResponse::json(
            401,
            serde_json::json!({ "error": "unauthorized", "message": "missing or invalid daemon token" }),
        );
    }

    let raw = match std::str::from_utf8(&request.body) {
        Ok(raw) => raw,
        Err(error) => {
            return HttpResponse::json(
                400,
                serde_json::json!({ "error": "invalid_utf8", "message": error.to_string() }),
            );
        }
    };

    match handle_mcp_message(raw, runtime).await {
        Some(response) => HttpResponse::json(200, response),
        None => HttpResponse::json(202, serde_json::json!({ "accepted": true })),
    }
}

fn http_request_authorized(request: &HttpRequest, auth_token: Option<&str>) -> bool {
    let Some(auth_token) = auth_token else {
        return true;
    };
    let bearer = request
        .headers
        .get("authorization")
        .and_then(|value| value.strip_prefix("Bearer "))
        .is_some_and(|token| token == auth_token);
    let token_header = request
        .headers
        .get("x-browser-use-rs-token")
        .is_some_and(|token| token == auth_token);
    bearer || token_header
}

fn http_reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        202 => "Accepted",
        400 => "Bad Request",
        401 => "Unauthorized",
        404 => "Not Found",
        _ => "OK",
    }
}

#[derive(Default)]
struct McpRuntime {
    sessions: HashMap<String, Arc<CdpBrowserSession>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum McpSessionPlan {
    ReuseInMemory,
    ReconnectPersistentRecord,
    CreatePersistentRecord,
}

fn mcp_session_plan(
    has_in_memory_session: bool,
    has_persistent_record: bool,
    has_url: bool,
    session_id: &str,
) -> anyhow::Result<McpSessionPlan> {
    if has_in_memory_session {
        return Ok(McpSessionPlan::ReuseInMemory);
    }
    if has_persistent_record {
        return Ok(McpSessionPlan::ReconnectPersistentRecord);
    }
    if has_url {
        return Ok(McpSessionPlan::CreatePersistentRecord);
    }
    anyhow::bail!("url is required to create MCP session {session_id}")
}

impl McpRuntime {
    async fn session(
        &mut self,
        session_id: &str,
        url: Option<String>,
    ) -> anyhow::Result<Arc<CdpBrowserSession>> {
        let record_path = session_record_path(session_id)?;
        match mcp_session_plan(
            self.sessions.contains_key(session_id),
            record_path.exists(),
            url.is_some(),
            session_id,
        )? {
            McpSessionPlan::ReuseInMemory => {
                let session = self
                    .sessions
                    .get(session_id)
                    .cloned()
                    .expect("session plan confirmed in-memory session");
                if let Some(url) = url {
                    session.navigate(&url, false).await?;
                    sleep(Duration::from_millis(150)).await;
                }
                Ok(session)
            }
            McpSessionPlan::ReconnectPersistentRecord => {
                let record = read_session_record(session_id)?;
                let session = Arc::new(CdpBrowserSession::connect(record.endpoint).await?);
                if let Some(url) = url {
                    session.navigate(&url, false).await?;
                    sleep(Duration::from_millis(150)).await;
                }
                self.sessions
                    .insert(session_id.to_owned(), Arc::clone(&session));
                Ok(session)
            }
            McpSessionPlan::CreatePersistentRecord => {
                let url = url.expect("session plan confirmed URL is present");
                let (_record, session, _state) =
                    start_persistent_session(session_id, &url, false).await?;
                let session = Arc::new(session);
                self.sessions
                    .insert(session_id.to_owned(), Arc::clone(&session));
                Ok(session)
            }
        }
    }
}

async fn handle_mcp_message(raw: &str, runtime: &mut McpRuntime) -> Option<Value> {
    let request = match serde_json::from_str::<browser_use_mcp::JsonRpcRequest>(raw) {
        Ok(request) => request,
        Err(error) => {
            return Some(browser_use_mcp::json_rpc_error(
                None,
                -32700,
                format!("Parse error: {error}"),
            ));
        }
    };

    let id = request.id.clone()?;

    if request.jsonrpc != "2.0" {
        return Some(browser_use_mcp::json_rpc_error(
            Some(id),
            -32600,
            "Invalid JSON-RPC version",
        ));
    }

    match request.method.as_str() {
        "initialize" => Some(browser_use_mcp::json_rpc_success(
            id,
            browser_use_mcp::initialize_result(),
        )),
        "ping" => Some(browser_use_mcp::json_rpc_success(id, serde_json::json!({}))),
        "tools/list" => Some(browser_use_mcp::json_rpc_success(
            id,
            browser_use_mcp::tools_list_result(),
        )),
        "tools/call" => Some(handle_mcp_tool_call(id, request.params, runtime).await),
        method => Some(browser_use_mcp::json_rpc_error(
            Some(id),
            -32601,
            format!("Method not found: {method}"),
        )),
    }
}

async fn handle_mcp_tool_call(id: Value, params: Option<Value>, runtime: &mut McpRuntime) -> Value {
    let params = match serde_json::from_value::<browser_use_mcp::CallToolParams>(
        params.unwrap_or(Value::Null),
    ) {
        Ok(params) => params,
        Err(error) => {
            return browser_use_mcp::json_rpc_error(
                Some(id),
                -32602,
                format!("Invalid tools/call params: {error}"),
            );
        }
    };

    if !matches!(
        params.name.as_str(),
        browser_use_mcp::STATE_TOOL_NAME
            | browser_use_mcp::ACTIONS_TOOL_NAME
            | browser_use_mcp::REPLAY_TOOL_NAME
            | browser_use_mcp::AGENT_TOOL_NAME
            | browser_use_mcp::SESSION_TOOL_NAME
    ) {
        return browser_use_mcp::json_rpc_error(
            Some(id),
            -32602,
            format!("Unknown tool: {}", params.name),
        );
    }

    let result = execute_mcp_tool(&params.name, params.arguments, runtime)
        .await
        .unwrap_or_else(|error| browser_use_mcp::tool_error_result(error.to_string()));
    browser_use_mcp::json_rpc_success(id, result)
}

async fn execute_mcp_tool(
    name: &str,
    arguments: Value,
    runtime: &mut McpRuntime,
) -> anyhow::Result<Value> {
    match name {
        browser_use_mcp::STATE_TOOL_NAME => {
            let input: browser_use_mcp::StateToolInput = serde_json::from_value(arguments)?;
            let state = if let Some(session_id) = input.session_id {
                let session = runtime.session(&session_id, input.url).await?;
                session.state(input.screenshot).await?
            } else {
                let url = require_mcp_url(input.url)?;
                let session = launch_and_navigate(&url).await?;
                session.state(input.screenshot).await?
            };
            let output = browser_use_mcp::StateToolOutput { state };
            Ok(browser_use_mcp::tool_success_result(serde_json::to_value(
                output,
            )?))
        }
        browser_use_mcp::ACTIONS_TOOL_NAME => {
            let input: browser_use_mcp::ActionsToolInput = serde_json::from_value(arguments)?;
            let session = if let Some(session_id) = input.session_id {
                runtime.session(&session_id, input.url).await?
            } else {
                Arc::new(launch_and_navigate(&require_mcp_url(input.url)?).await?)
            };
            let mut executor = BrowserActionExecutor::new(session);
            let results = executor.execute_sequence(&input.actions).await;
            let state = executor.session().state(input.screenshot).await?;
            let output = browser_use_mcp::ActionsToolOutput { results, state };
            Ok(browser_use_mcp::tool_success_result(serde_json::to_value(
                output,
            )?))
        }
        browser_use_mcp::REPLAY_TOOL_NAME => {
            let input: browser_use_mcp::ReplayToolInput = serde_json::from_value(arguments)?;
            let session = if let Some(session_id) = input.session_id {
                runtime.session(&session_id, input.url).await?
            } else {
                Arc::new(launch_and_navigate(&require_mcp_url(input.url)?).await?)
            };
            let mut executor = BrowserActionExecutor::new(session);
            let replay = executor.replay_history(&input.history).await?;
            let output = browser_use_mcp::ReplayToolOutput { replay };
            Ok(browser_use_mcp::tool_success_result(serde_json::to_value(
                output,
            )?))
        }
        browser_use_mcp::AGENT_TOOL_NAME => {
            let input: browser_use_mcp::AgentToolInput = serde_json::from_value(arguments)?;
            let provider = LlmProvider::from_mcp(input.provider);
            let structured_output_mode = input
                .structured_output_mode
                .map(StructuredOutputMode::from_mcp)
                .map(StructuredOutputMode::into_openai_mode);
            let llm = configured_chat_model(
                provider,
                None,
                input.model,
                input.base_url,
                structured_output_mode,
            )?;
            let session = if let Some(session_id) = input.session_id {
                runtime.session(&session_id, input.url).await?
            } else {
                Arc::new(launch_and_navigate(&require_mcp_url(input.url)?).await?)
            };
            let mut agent =
                browser_use_core::Agent::with_settings(input.task, input.settings, llm, session);
            let history = agent.run(input.max_steps).await?;
            let output = browser_use_mcp::AgentToolOutput {
                history: history.clone(),
            };
            Ok(browser_use_mcp::tool_success_result(serde_json::to_value(
                output,
            )?))
        }
        browser_use_mcp::SESSION_TOOL_NAME => {
            let input: browser_use_mcp::SessionToolInput = serde_json::from_value(arguments)?;
            let output = match input.operation {
                browser_use_mcp::SessionOperation::Start => {
                    let session_id = require_mcp_session_id(input.session_id)?;
                    let url = require_mcp_url(input.url)?;
                    let (record, session, state) =
                        start_persistent_session(&session_id, &url, input.screenshot).await?;
                    runtime.sessions.insert(session_id, Arc::new(session));
                    browser_use_mcp::SessionToolOutput {
                        session: Some(record),
                        sessions: Vec::new(),
                        cleaned_sessions: Vec::new(),
                        state: Some(state),
                    }
                }
                browser_use_mcp::SessionOperation::Stop => {
                    let session_id = require_mcp_session_id(input.session_id)?;
                    runtime.sessions.remove(&session_id);
                    let record = stop_persistent_session(&session_id).await?;
                    browser_use_mcp::SessionToolOutput {
                        session: Some(record),
                        sessions: Vec::new(),
                        cleaned_sessions: Vec::new(),
                        state: None,
                    }
                }
                browser_use_mcp::SessionOperation::List => browser_use_mcp::SessionToolOutput {
                    session: None,
                    sessions: list_session_records()?,
                    cleaned_sessions: Vec::new(),
                    state: None,
                },
                browser_use_mcp::SessionOperation::Cleanup => {
                    let cleaned_sessions =
                        cleanup_persistent_sessions(input.session_id.as_deref(), input.force)
                            .await?;
                    browser_use_mcp::SessionToolOutput {
                        session: None,
                        sessions: list_session_records()?,
                        cleaned_sessions,
                        state: None,
                    }
                }
            };
            Ok(browser_use_mcp::tool_success_result(serde_json::to_value(
                output,
            )?))
        }
        _ => unreachable!("tool name was validated before execution"),
    }
}

fn require_mcp_session_id(session_id: Option<String>) -> anyhow::Result<String> {
    session_id.ok_or_else(|| anyhow::anyhow!("session_id is required for this operation"))
}

fn require_mcp_url(url: Option<String>) -> anyhow::Result<String> {
    url.ok_or_else(|| anyhow::anyhow!("url is required when session_id is not provided"))
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

    let mut llm = OpenAiCompatibleChatModel::new(api_key, model)
        .with_base_url(base_url)
        .with_provider_name(config.provider_name)
        .with_structured_output_mode(
            structured_output_mode_override.unwrap_or(config.structured_output_mode),
        );
    for (name, value) in openai_wire_default_headers(config, first_nonempty_env) {
        llm = llm.try_with_default_header(name, value)?;
    }

    Ok(Box::new(llm))
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
mod tests {
    use super::*;

    #[test]
    fn parses_agent_settings_flags() {
        let cli = Cli::try_parse_from([
            "browser-use-rs",
            "agent",
            "https://example.com",
            "test task",
            "--provider",
            "openai",
            "--max-steps",
            "3",
            "--no-vision",
            "--vision-detail-level",
            "high",
            "--max-failures",
            "2",
            "--max-actions-per-step",
            "1",
            "--llm-timeout-seconds",
            "11",
            "--step-timeout-seconds",
            "22",
            "--no-final-response-after-failure",
            "--no-display-files-in-done-text",
            "--no-loop-detection",
            "--loop-detection-window",
            "4",
            "--no-thinking",
            "--flash-mode",
            "--no-judge",
            "--ground-truth",
            "Must include a receipt.",
            "--save-conversation-path",
            "/tmp/browser-use-conversations",
            "--save-conversation-path-encoding",
            "utf-8",
            "--no-planning",
            "--planning-replan-on-stall",
            "5",
            "--planning-exploration-limit",
            "6",
            "--max-history-items",
            "7",
            "--max-clickable-elements-length",
            "8000",
            "--include-recent-events",
            "--include-attribute",
            "data-testid",
            "--include-attribute",
            "aria-label",
            "--available-file-path",
            "/tmp/report.pdf",
            "--available-file-path",
            "/tmp/chart.png",
            "--exclude-action",
            "search",
            "--exclude-action",
            "scroll",
            "--sensitive-data",
            "username=evalops@example.test",
            "--sensitive-data",
            "api_key=sk=value",
            "--sensitive-data-domain",
            "*.example.test=password=super-secret",
            "--override-system-message",
            "Custom system prompt.",
            "--extend-system-message",
            "Add selector guidance.",
            "--allowed-domain",
            "*.example.test",
            "--prohibited-domain",
            "tracker.example.test",
            "--block-ip-addresses",
        ])
        .expect("agent settings flags should parse");

        match cli.command.expect("agent command") {
            Command::Agent {
                provider,
                max_steps,
                no_vision,
                vision_detail_level,
                max_failures,
                max_actions_per_step,
                llm_timeout_seconds,
                step_timeout_seconds,
                no_final_response_after_failure,
                no_display_files_in_done_text,
                no_loop_detection,
                loop_detection_window,
                no_thinking,
                flash_mode,
                no_judge,
                ground_truth,
                save_conversation_path,
                save_conversation_path_encoding,
                no_planning,
                planning_replan_on_stall,
                planning_exploration_limit,
                max_history_items,
                max_clickable_elements_length,
                include_recent_events,
                include_attributes,
                available_file_paths,
                excluded_actions,
                sensitive_data,
                sensitive_data_domains,
                override_system_message,
                extend_system_message,
                allowed_domains,
                prohibited_domains,
                block_ip_addresses,
                ..
            } => {
                assert_eq!(provider, LlmProvider::OpenAiCompatible);
                assert_eq!(max_steps, 3);
                assert!(no_vision);
                assert_eq!(vision_detail_level, Some(CliVisionDetailLevel::High));
                assert_eq!(max_failures, Some(2));
                assert_eq!(max_actions_per_step, Some(1));
                assert_eq!(llm_timeout_seconds, Some(11));
                assert_eq!(step_timeout_seconds, Some(22));
                assert!(no_final_response_after_failure);
                assert!(no_display_files_in_done_text);
                assert!(no_loop_detection);
                assert_eq!(loop_detection_window, Some(4));
                assert!(no_thinking);
                assert!(flash_mode);
                assert!(no_judge);
                assert_eq!(ground_truth.as_deref(), Some("Must include a receipt."));
                assert_eq!(
                    save_conversation_path.as_deref(),
                    Some("/tmp/browser-use-conversations")
                );
                assert_eq!(save_conversation_path_encoding.as_deref(), Some("utf-8"));
                assert!(no_planning);
                assert_eq!(planning_replan_on_stall, Some(5));
                assert_eq!(planning_exploration_limit, Some(6));
                assert_eq!(max_history_items, Some(7));
                assert_eq!(max_clickable_elements_length, Some(8000));
                assert!(include_recent_events);
                assert_eq!(include_attributes, ["data-testid", "aria-label"]);
                assert_eq!(available_file_paths, ["/tmp/report.pdf", "/tmp/chart.png"]);
                assert_eq!(excluded_actions, ["search", "scroll"]);
                assert_eq!(
                    sensitive_data,
                    [
                        SensitiveDataEntry {
                            placeholder: "username".to_owned(),
                            value: "evalops@example.test".to_owned()
                        },
                        SensitiveDataEntry {
                            placeholder: "api_key".to_owned(),
                            value: "sk=value".to_owned()
                        }
                    ]
                );
                assert_eq!(
                    sensitive_data_domains,
                    [DomainSensitiveDataEntry {
                        domain_pattern: "*.example.test".to_owned(),
                        placeholder: "password".to_owned(),
                        value: "super-secret".to_owned()
                    }]
                );
                assert_eq!(
                    override_system_message.as_deref(),
                    Some("Custom system prompt.")
                );
                assert_eq!(
                    extend_system_message.as_deref(),
                    Some("Add selector guidance.")
                );
                assert_eq!(allowed_domains, ["*.example.test"]);
                assert_eq!(prohibited_domains, ["tracker.example.test"]);
                assert!(block_ip_addresses);
            }
            _ => panic!("expected agent command"),
        }
    }

    #[test]
    fn parses_replay_command() {
        let cli = Cli::try_parse_from([
            "browser-use-rs",
            "replay",
            "https://example.com",
            "history.json",
        ])
        .expect("replay command should parse");

        match cli.command.expect("replay command") {
            Command::Replay { url, history } => {
                assert_eq!(url, "https://example.com");
                assert_eq!(history, PathBuf::from("history.json"));
            }
            _ => panic!("expected replay command"),
        }
    }

    #[test]
    fn parses_replay_run_schema_contract() {
        let cli = Cli::try_parse_from(["browser-use-rs", "schema", "replay-run"])
            .expect("schema command should parse");

        match cli.command.expect("schema command") {
            Command::Schema { contract } => assert!(matches!(contract, SchemaContract::ReplayRun)),
            _ => panic!("expected schema command"),
        }
    }

    #[test]
    fn parses_upstream_openai_wire_provider_aliases() {
        for (provider_name, expected_provider) in [
            ("deepseek", LlmProvider::DeepSeek),
            ("deep-seek", LlmProvider::DeepSeek),
            ("groq", LlmProvider::Groq),
            ("cerebras", LlmProvider::Cerebras),
            ("mistral", LlmProvider::Mistral),
            ("openrouter", LlmProvider::OpenRouter),
            ("open-router", LlmProvider::OpenRouter),
            ("vercel", LlmProvider::Vercel),
            ("ai-gateway", LlmProvider::Vercel),
        ] {
            let cli = Cli::try_parse_from([
                "browser-use-rs",
                "agent",
                "https://example.com",
                "test task",
                "--provider",
                provider_name,
            ])
            .expect("agent provider should parse");

            match cli.command.expect("agent command") {
                Command::Agent { provider, .. } => assert_eq!(provider, expected_provider),
                _ => panic!("expected agent command"),
            }
        }
    }

    #[test]
    fn configures_openai_wire_provider_aliases_without_env() {
        for (provider, expected_name, expected_model) in [
            (LlmProvider::DeepSeek, "deepseek", "deepseek-chat"),
            (LlmProvider::Cerebras, "cerebras", "llama3.1-8b"),
            (LlmProvider::Mistral, "mistral", "mistral-medium-latest"),
        ] {
            let llm =
                configured_chat_model(provider, Some("test-key".to_owned()), None, None, None)
                    .expect("provider should use default model");

            assert_eq!(llm.provider(), expected_name);
            assert_eq!(llm.model(), expected_model);
        }

        let openrouter = configured_chat_model(
            LlmProvider::OpenRouter,
            Some("test-key".to_owned()),
            Some("openai/gpt-4o-mini".to_owned()),
            None,
            None,
        )
        .expect("openrouter with explicit model");
        assert_eq!(openrouter.provider(), "openrouter");
        assert_eq!(openrouter.model(), "openai/gpt-4o-mini");

        assert_eq!(
            openai_wire_provider_config(LlmProvider::DeepSeek).structured_output_mode,
            OpenAiStructuredOutputMode::ToolCall
        );
        assert_eq!(
            openai_wire_provider_config(LlmProvider::Cerebras).structured_output_mode,
            OpenAiStructuredOutputMode::PromptOnly
        );
        assert!(
            openai_wire_provider_config(LlmProvider::DeepSeek)
                .default_headers
                .is_empty()
        );
    }

    #[test]
    fn openrouter_default_headers_read_app_attribution_env_names() {
        let config = openai_wire_provider_config(LlmProvider::OpenRouter);
        let headers = openai_wire_default_headers(config, |names| {
            if names == ["OPENROUTER_HTTP_REFERER", "OPENROUTER_APP_URL"] {
                Some("https://evalops.dev".to_owned())
            } else if names == ["OPENROUTER_X_TITLE", "OPENROUTER_APP_TITLE"] {
                Some("EvalOps browser-use-rs".to_owned())
            } else {
                None
            }
        });

        assert_eq!(
            headers,
            [
                ("HTTP-Referer", "https://evalops.dev".to_owned()),
                ("X-Title", "EvalOps browser-use-rs".to_owned()),
                ("X-OpenRouter-Title", "EvalOps browser-use-rs".to_owned())
            ]
        );
    }

    #[test]
    fn parses_structured_output_mode_override() {
        let cli = Cli::try_parse_from([
            "browser-use-rs",
            "agent",
            "https://example.com",
            "test task",
            "--provider",
            "openrouter",
            "--structured-output-mode",
            "tool-call",
        ])
        .expect("structured output mode should parse");

        match cli.command.expect("agent command") {
            Command::Agent {
                structured_output_mode,
                ..
            } => assert_eq!(structured_output_mode, Some(StructuredOutputMode::ToolCall)),
            _ => panic!("expected agent command"),
        }
    }

    #[test]
    fn maps_mcp_structured_output_mode_override() {
        let mode =
            StructuredOutputMode::from_mcp(browser_use_mcp::AgentStructuredOutputMode::ToolCall)
                .into_openai_mode();

        assert_eq!(mode, OpenAiStructuredOutputMode::ToolCall);
    }

    #[test]
    fn rejects_malformed_sensitive_data_flags() {
        assert!(
            Cli::try_parse_from([
                "browser-use-rs",
                "agent",
                "https://example.com",
                "test task",
                "--sensitive-data",
                "username",
            ])
            .is_err()
        );
        assert!(
            Cli::try_parse_from([
                "browser-use-rs",
                "agent",
                "https://example.com",
                "test task",
                "--sensitive-data-domain",
                "*.example.test=password",
            ])
            .is_err()
        );
    }

    #[test]
    fn parses_agent_auto_vision_mode_flag() {
        let cli = Cli::try_parse_from([
            "browser-use-rs",
            "agent",
            "https://example.com",
            "test task",
            "--vision-mode",
            "auto",
        ])
        .expect("auto vision mode should parse");

        match cli.command.expect("agent command") {
            Command::Agent { vision_mode, .. } => {
                assert_eq!(vision_mode, Some(CliVisionMode::Auto));
            }
            _ => panic!("expected agent command"),
        }

        assert!(
            Cli::try_parse_from([
                "browser-use-rs",
                "agent",
                "https://example.com",
                "test task",
                "--no-vision",
                "--vision-mode",
                "auto",
            ])
            .is_err()
        );
    }

    #[test]
    fn builds_agent_settings_from_cli_flags() {
        let settings = cli_agent_settings(CliAgentSettingsArgs {
            no_vision: true,
            vision_mode: None,
            vision_detail_level: Some(CliVisionDetailLevel::High),
            max_failures: Some(2),
            max_actions_per_step: Some(1),
            llm_timeout_seconds: Some(11),
            step_timeout_seconds: Some(22),
            no_final_response_after_failure: true,
            no_display_files_in_done_text: true,
            no_loop_detection: true,
            loop_detection_window: Some(4),
            no_thinking: true,
            flash_mode: true,
            no_judge: true,
            ground_truth: Some("Must include a receipt.".to_owned()),
            save_conversation_path: Some("/tmp/browser-use-conversations".to_owned()),
            save_conversation_path_encoding: Some("utf-8".to_owned()),
            no_planning: true,
            planning_replan_on_stall: Some(5),
            planning_exploration_limit: Some(6),
            max_history_items: Some(7),
            max_clickable_elements_length: Some(8000),
            include_recent_events: true,
            include_attributes: vec!["data-testid".to_owned(), "aria-label".to_owned()],
            available_file_paths: vec!["/tmp/report.pdf".to_owned(), "/tmp/chart.png".to_owned()],
            excluded_actions: vec!["search".to_owned(), "scroll".to_owned()],
            sensitive_data: vec![SensitiveDataEntry {
                placeholder: "username".to_owned(),
                value: "evalops@example.test".to_owned(),
            }],
            sensitive_data_domains: vec![
                DomainSensitiveDataEntry {
                    domain_pattern: "*.example.test".to_owned(),
                    placeholder: "password".to_owned(),
                    value: "super-secret".to_owned(),
                },
                DomainSensitiveDataEntry {
                    domain_pattern: "*.example.test".to_owned(),
                    placeholder: "otp".to_owned(),
                    value: "123456".to_owned(),
                },
            ],
            override_system_message: Some("Custom system prompt.".to_owned()),
            extend_system_message: Some("Add selector guidance.".to_owned()),
        });

        assert_eq!(settings.use_vision, VisionMode::Never);
        assert_eq!(settings.vision_detail_level, ImageDetailLevel::High);
        assert_eq!(settings.max_failures, 2);
        assert_eq!(settings.max_actions_per_step, 1);
        assert_eq!(settings.llm_timeout_seconds, 11);
        assert_eq!(settings.step_timeout_seconds, 22);
        assert!(!settings.final_response_after_failure);
        assert!(!settings.display_files_in_done_text);
        assert!(!settings.loop_detection_enabled);
        assert_eq!(settings.loop_detection_window, 4);
        assert!(!settings.use_thinking);
        assert!(settings.flash_mode);
        assert!(!settings.use_judge);
        assert_eq!(
            settings.ground_truth.as_deref(),
            Some("Must include a receipt.")
        );
        assert_eq!(
            settings.save_conversation_path.as_deref(),
            Some("/tmp/browser-use-conversations")
        );
        assert_eq!(
            settings.save_conversation_path_encoding.as_deref(),
            Some("utf-8")
        );
        assert!(!settings.enable_planning);
        assert_eq!(settings.planning_replan_on_stall, 5);
        assert_eq!(settings.planning_exploration_limit, 6);
        assert_eq!(settings.max_history_items, Some(7));
        assert_eq!(settings.max_clickable_elements_length, 8000);
        assert!(settings.include_recent_events);
        assert_eq!(settings.include_attributes, ["data-testid", "aria-label"]);
        assert_eq!(
            settings.available_file_paths,
            ["/tmp/report.pdf", "/tmp/chart.png"]
        );
        assert_eq!(settings.excluded_actions, ["search", "scroll"]);
        assert_eq!(
            settings.sensitive_data.get("username"),
            Some(&SensitiveDataValue::Value(
                "evalops@example.test".to_owned()
            ))
        );
        assert_eq!(
            settings.sensitive_data.get("*.example.test"),
            Some(&SensitiveDataValue::Domain(BTreeMap::from([
                ("otp".to_owned(), "123456".to_owned()),
                ("password".to_owned(), "super-secret".to_owned())
            ])))
        );
        assert_eq!(
            settings.override_system_message.as_deref(),
            Some("Custom system prompt.")
        );
        assert_eq!(
            settings.extend_system_message.as_deref(),
            Some("Add selector guidance.")
        );
    }

    #[test]
    fn builds_agent_settings_with_auto_vision_mode() {
        let settings = cli_agent_settings(CliAgentSettingsArgs {
            vision_mode: Some(CliVisionMode::Auto),
            ..CliAgentSettingsArgs::default()
        });

        assert_eq!(settings.use_vision, VisionMode::Auto);
    }

    #[test]
    fn parses_http_daemon_flags() {
        let cli = Cli::try_parse_from([
            "browser-use-rs",
            "daemon",
            "--addr",
            "127.0.0.1:0",
            "--transport",
            "http",
            "--auth-token",
            "secret",
            "--pid-file",
            "/tmp/browser-use-rs.pid",
            "--ready-file",
            "/tmp/browser-use-rs.ready.json",
        ])
        .expect("daemon flags should parse");

        match cli.command.expect("daemon command") {
            Command::Daemon {
                addr,
                transport,
                auth_token,
                pid_file,
                ready_file,
            } => {
                assert_eq!(addr, "127.0.0.1:0");
                assert_eq!(transport, DaemonTransport::Http);
                assert_eq!(auth_token.as_deref(), Some("secret"));
                assert_eq!(pid_file, Some(PathBuf::from("/tmp/browser-use-rs.pid")));
                assert_eq!(
                    ready_file,
                    Some(PathBuf::from("/tmp/browser-use-rs.ready.json"))
                );
            }
            _ => panic!("expected daemon command"),
        }
    }

    #[test]
    fn mcp_session_plan_persists_implicit_session_ids() {
        assert_eq!(
            mcp_session_plan(true, false, false, "existing").expect("in memory"),
            McpSessionPlan::ReuseInMemory
        );
        assert_eq!(
            mcp_session_plan(false, true, false, "recorded").expect("record"),
            McpSessionPlan::ReconnectPersistentRecord
        );
        assert_eq!(
            mcp_session_plan(false, false, true, "implicit").expect("create"),
            McpSessionPlan::CreatePersistentRecord
        );

        let error = mcp_session_plan(false, false, false, "missing-url")
            .expect_err("missing url should fail");
        assert_eq!(
            error.to_string(),
            "url is required to create MCP session missing-url"
        );
    }

    #[test]
    fn session_record_status_is_backward_compatible() {
        let record: StoredSession = serde_json::from_value(serde_json::json!({
            "id": "legacy",
            "endpoint": {
                "http_url": "http://127.0.0.1:9222",
                "websocket_url": "ws://127.0.0.1:9222/devtools/browser/legacy"
            },
            "user_data_dir": "/tmp/browser-use-rs-legacy",
            "process_id": 4294967295_u32
        }))
        .expect("legacy record");

        assert_eq!(record.status, None);
        assert_eq!(
            session_status_with_checker(&record, |_| false),
            browser_use_mcp::SessionStatus::Stale
        );
        assert_eq!(
            session_status_with_checker(&record, |_| true),
            browser_use_mcp::SessionStatus::Running
        );
        assert_eq!(
            annotate_session_status(StoredSession {
                process_id: None,
                ..record
            })
            .status,
            Some(browser_use_mcp::SessionStatus::Unknown)
        );
    }

    #[test]
    fn session_cleanup_decision_is_conservative() {
        let record: StoredSession = serde_json::from_value(serde_json::json!({
            "id": "cleanup-target",
            "endpoint": {
                "http_url": "http://127.0.0.1:9222",
                "websocket_url": "ws://127.0.0.1:9222/devtools/browser/cleanup"
            },
            "user_data_dir": "/tmp/browser-use-rs-cleanup",
            "process_id": 1234_u32
        }))
        .expect("cleanup record");

        assert_eq!(
            session_cleanup_decision(&record, false, |_| true),
            SessionCleanupDecision::SkipRunning
        );
        assert_eq!(
            session_cleanup_decision(&record, true, |_| true),
            SessionCleanupDecision::StopRunning
        );
        assert_eq!(
            session_cleanup_decision(&record, false, |_| false),
            SessionCleanupDecision::RemoveRecord
        );

        let unknown = StoredSession {
            process_id: None,
            ..record
        };
        assert_eq!(
            session_cleanup_decision(&unknown, false, |_| false),
            SessionCleanupDecision::SkipUnknown
        );
        assert_eq!(
            session_cleanup_decision(&unknown, true, |_| false),
            SessionCleanupDecision::RemoveRecord
        );
    }

    #[test]
    fn parses_session_cleanup_flags() {
        let cli = Cli::try_parse_from([
            "browser-use-rs",
            "session",
            "cleanup",
            "stale-one",
            "--force",
        ])
        .expect("cleanup flags should parse");

        match cli.command.expect("command") {
            Command::Session {
                command: SessionCommand::Cleanup { id, force },
            } => {
                assert_eq!(id.as_deref(), Some("stale-one"));
                assert!(force);
            }
            _ => panic!("expected session cleanup command"),
        }
    }

    #[test]
    fn parses_session_replay_command() {
        let cli = Cli::try_parse_from([
            "browser-use-rs",
            "session",
            "replay",
            "existing",
            "history.json",
        ])
        .expect("session replay command should parse");

        match cli.command.expect("command") {
            Command::Session {
                command: SessionCommand::Replay { id, history },
            } => {
                assert_eq!(id, "existing");
                assert_eq!(history, PathBuf::from("history.json"));
            }
            _ => panic!("expected session replay command"),
        }
    }

    #[test]
    fn daemon_lifecycle_files_write_supervisor_artifacts() {
        let dir = std::env::temp_dir().join(format!(
            "browser-use-rs-daemon-lifecycle-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir");
        let pid_file = dir.join("daemon.pid");
        let ready_file = dir.join("daemon.ready.json");

        {
            let files = DaemonLifecycleFiles::write(
                DaemonLifecycleOptions {
                    pid_file: Some(pid_file.clone()),
                    ready_file: Some(ready_file.clone()),
                },
                DaemonTransport::Http,
                "127.0.0.1:8765",
            )
            .expect("write lifecycle files");
            let pid = std::fs::read_to_string(&pid_file).expect("pid file");
            assert_eq!(pid.trim(), std::process::id().to_string());
            let ready: Value =
                serde_json::from_slice(&std::fs::read(&ready_file).expect("ready file"))
                    .expect("ready json");
            assert_eq!(ready["ready"], true);
            assert_eq!(ready["transport"], "http");
            assert_eq!(ready["addr"], "127.0.0.1:8765");
            assert_eq!(ready["pid"], std::process::id());
            drop(files);
        }

        assert!(!pid_file.exists());
        assert!(!ready_file.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn authorizes_http_daemon_requests() {
        let request = http_request(
            "POST",
            "/rpc",
            [
                ("authorization", "Bearer secret"),
                ("x-browser-use-rs-token", "wrong"),
            ],
            b"{}",
        );
        assert!(http_request_authorized(&request, Some("secret")));
        assert!(http_request_authorized(&request, None));

        let request = http_request(
            "POST",
            "/rpc",
            [("x-browser-use-rs-token", "secret")],
            b"{}",
        );
        assert!(http_request_authorized(&request, Some("secret")));

        let request = http_request("POST", "/rpc", [("authorization", "Bearer nope")], b"{}");
        assert!(!http_request_authorized(&request, Some("secret")));
    }

    #[tokio::test]
    async fn mcp_replay_tool_dispatches_to_schema_errors() {
        let mut runtime = McpRuntime::default();
        let response = handle_mcp_message(
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"browser_use_replay","arguments":{}}}"#,
            &mut runtime,
        )
        .await
        .expect("response");

        assert_eq!(response["jsonrpc"], "2.0");
        assert_eq!(response["id"], 1);
        assert_eq!(response["result"]["isError"], true);
        let text = response["result"]["content"][0]["text"]
            .as_str()
            .expect("text content");
        assert!(text.contains("history"));
    }

    #[tokio::test]
    async fn http_daemon_healthz_does_not_require_auth() {
        let mut runtime = McpRuntime::default();
        let response = handle_http_request(
            http_request("GET", "/healthz", [], b""),
            &mut runtime,
            Some("secret"),
        )
        .await;

        assert_eq!(response.status, 200);
        let body: Value = serde_json::from_slice(&response.body).expect("json body");
        assert_eq!(body["ok"], true);
    }

    #[tokio::test]
    async fn http_daemon_rejects_missing_token() {
        let mut runtime = McpRuntime::default();
        let response = handle_http_request(
            http_request(
                "POST",
                "/rpc",
                [],
                br#"{"jsonrpc":"2.0","id":1,"method":"ping"}"#,
            ),
            &mut runtime,
            Some("secret"),
        )
        .await;

        assert_eq!(response.status, 401);
        let body: Value = serde_json::from_slice(&response.body).expect("json body");
        assert_eq!(body["error"], "unauthorized");
    }

    #[tokio::test]
    async fn http_daemon_dispatches_json_rpc_with_auth() {
        let mut runtime = McpRuntime::default();
        let response = handle_http_request(
            http_request(
                "POST",
                "/rpc",
                [("authorization", "Bearer secret")],
                br#"{"jsonrpc":"2.0","id":1,"method":"tools/list","params":{}}"#,
            ),
            &mut runtime,
            Some("secret"),
        )
        .await;

        assert_eq!(response.status, 200);
        let body: Value = serde_json::from_slice(&response.body).expect("json body");
        assert_eq!(body["jsonrpc"], "2.0");
        assert_eq!(body["id"], 1);
        assert!(body["result"]["tools"].as_array().is_some_and(|tools| {
            tools
                .iter()
                .any(|tool| tool["name"] == browser_use_mcp::STATE_TOOL_NAME)
        }));
    }

    #[tokio::test]
    async fn reads_http_request_with_split_body() {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let addr = listener.local_addr().expect("local addr");

        let writer = tokio::spawn(async move {
            let mut stream = TcpStream::connect(addr).await.expect("connect");
            stream
                .write_all(
                    b"POST /rpc HTTP/1.1\r\nHost: localhost\r\nContent-Length: 11\r\n\r\nhello",
                )
                .await
                .expect("write headers");
            stream.write_all(b" world").await.expect("write body");
        });

        let (mut stream, _) = listener.accept().await.expect("accept");
        let request = read_http_request(&mut stream).await.expect("read request");
        writer.await.expect("writer task");

        assert_eq!(request.method, "POST");
        assert_eq!(request.path, "/rpc");
        assert_eq!(request.headers["host"], "localhost");
        assert_eq!(request.body, b"hello world");
    }

    fn http_request<const N: usize>(
        method: &str,
        path: &str,
        headers: [(&str, &str); N],
        body: &[u8],
    ) -> HttpRequest {
        HttpRequest {
            method: method.to_owned(),
            path: path.to_owned(),
            headers: headers
                .into_iter()
                .map(|(name, value)| (name.to_ascii_lowercase(), value.to_owned()))
                .collect(),
            body: body.to_vec(),
        }
    }
}
