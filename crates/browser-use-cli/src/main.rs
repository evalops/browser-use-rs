use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Command as StdCommand;
use std::sync::Arc;
use std::time::{Duration, Instant};

use base64::Engine;
use browser_use_cdp::{BrowserProfile, BrowserSession, CdpBrowserSession};
use browser_use_core::{AgentSettings, BrowserActionExecutor};
use browser_use_llm::{
    AnthropicChatModel, ChatModel, GeminiChatModel, OllamaChatModel, OpenAiCompatibleChatModel,
};
use clap::Parser;
use schemars::schema_for;
use serde_json::Value;
use tokio::io::{self, AsyncBufReadExt, AsyncWriteExt, BufReader};
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
    /// Run a TCP JSON-RPC daemon exposing the same tools as mcp-stdio.
    Daemon {
        #[arg(long, default_value = "127.0.0.1:8765")]
        addr: String,
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
        #[arg(long, default_value_t = 10)]
        max_steps: usize,
        #[arg(long, default_value_t = false)]
        no_vision: bool,
        #[arg(long)]
        max_failures: Option<u32>,
        #[arg(long)]
        max_actions_per_step: Option<usize>,
        #[arg(long)]
        llm_timeout_seconds: Option<u64>,
        #[arg(long)]
        step_timeout_seconds: Option<u64>,
        #[arg(long, default_value_t = false)]
        no_loop_detection: bool,
        #[arg(long)]
        loop_detection_window: Option<usize>,
        #[arg(long, default_value_t = false)]
        no_thinking: bool,
        #[arg(long, default_value_t = false)]
        flash_mode: bool,
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
        #[arg(long = "include-attribute")]
        include_attributes: Vec<String>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
enum LlmProvider {
    #[value(name = "openai-compatible", alias = "openai")]
    OpenAiCompatible,
    Anthropic,
    #[value(alias = "google")]
    Gemini,
    #[value(alias = "local")]
    Ollama,
}

impl LlmProvider {
    fn from_mcp(provider: Option<browser_use_mcp::AgentProvider>) -> Self {
        match provider.unwrap_or(browser_use_mcp::AgentProvider::OpenAiCompatible) {
            browser_use_mcp::AgentProvider::OpenAiCompatible => Self::OpenAiCompatible,
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
    /// Stop an existing persistent session.
    Stop { id: String },
    /// List recorded persistent sessions.
    List,
}

type StoredSession = browser_use_mcp::SessionRecord;

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
        Some(Command::Daemon { addr }) => {
            run_daemon(&addr).await?;
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
        Some(Command::Agent {
            url,
            task,
            provider,
            api_key,
            model,
            base_url,
            max_steps,
            no_vision,
            max_failures,
            max_actions_per_step,
            llm_timeout_seconds,
            step_timeout_seconds,
            no_loop_detection,
            loop_detection_window,
            no_thinking,
            flash_mode,
            no_planning,
            planning_replan_on_stall,
            planning_exploration_limit,
            max_history_items,
            max_clickable_elements_length,
            include_attributes,
        }) => {
            let llm = configured_chat_model(provider, api_key, model, base_url)?;
            let session = launch_and_navigate(&url).await?;
            let settings = cli_agent_settings(CliAgentSettingsArgs {
                no_vision,
                max_failures,
                max_actions_per_step,
                llm_timeout_seconds,
                step_timeout_seconds,
                no_loop_detection,
                loop_detection_window,
                no_thinking,
                flash_mode,
                no_planning,
                planning_replan_on_stall,
                planning_exploration_limit,
                max_history_items,
                max_clickable_elements_length,
                include_attributes,
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
    let session = CdpBrowserSession::launch(&BrowserProfile::default()).await?;
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
    max_failures: Option<u32>,
    max_actions_per_step: Option<usize>,
    llm_timeout_seconds: Option<u64>,
    step_timeout_seconds: Option<u64>,
    no_loop_detection: bool,
    loop_detection_window: Option<usize>,
    no_thinking: bool,
    flash_mode: bool,
    no_planning: bool,
    planning_replan_on_stall: Option<usize>,
    planning_exploration_limit: Option<usize>,
    max_history_items: Option<usize>,
    max_clickable_elements_length: Option<usize>,
    include_attributes: Vec<String>,
}

fn cli_agent_settings(args: CliAgentSettingsArgs) -> AgentSettings {
    let mut settings = AgentSettings::default();

    if args.no_vision {
        settings.use_vision = false;
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
    settings.include_attributes = args.include_attributes;

    settings
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
    };
    write_session_record(&record)?;
    let _ = launched.detach();
    Ok((record, session, state))
}

async fn stop_persistent_session(id: &str) -> anyhow::Result<StoredSession> {
    let record = read_session_record(id)?;
    if let Ok(session) = CdpBrowserSession::connect(record.endpoint.clone()).await {
        let _ = session.close_browser().await;
    }
    wait_for_process_exit(record.process_id, Duration::from_secs(2)).await;
    remove_session_dir(id)?;
    Ok(record)
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
            let record = read_session_record(&id)?;
            let session = CdpBrowserSession::connect(record.endpoint).await?;
            print_state(&session, screenshot).await?;
        }
        SessionCommand::Actions {
            id,
            actions,
            screenshot,
        } => {
            let record = read_session_record(&id)?;
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
    std::fs::create_dir_all(parent)?;
    std::fs::write(path, serde_json::to_vec_pretty(record)?)?;
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
            records.push(record);
        }
    }
    records.sort_by(|left, right| left.id.cmp(&right.id));
    Ok(records)
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

async fn run_daemon(addr: &str) -> anyhow::Result<()> {
    let listener = TcpListener::bind(addr).await?;
    println!("{}", listener.local_addr()?);
    let runtime = Arc::new(tokio::sync::Mutex::new(McpRuntime::default()));

    loop {
        let (stream, _) = listener.accept().await?;
        let runtime = Arc::clone(&runtime);
        tokio::spawn(async move {
            let _ = handle_daemon_connection(stream, runtime).await;
        });
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

#[derive(Default)]
struct McpRuntime {
    sessions: HashMap<String, Arc<CdpBrowserSession>>,
}

impl McpRuntime {
    async fn session(
        &mut self,
        session_id: &str,
        url: Option<String>,
    ) -> anyhow::Result<Arc<CdpBrowserSession>> {
        if let Some(session) = self.sessions.get(session_id).cloned() {
            if let Some(url) = url {
                session.navigate(&url, false).await?;
                sleep(Duration::from_millis(150)).await;
            }
            return Ok(session);
        }

        let record_path = session_record_path(session_id)?;
        if record_path.exists() {
            let record = read_session_record(session_id)?;
            let session = Arc::new(CdpBrowserSession::connect(record.endpoint).await?);
            if let Some(url) = url {
                session.navigate(&url, false).await?;
                sleep(Duration::from_millis(150)).await;
            }
            self.sessions
                .insert(session_id.to_owned(), Arc::clone(&session));
            return Ok(session);
        }

        let url = url
            .ok_or_else(|| anyhow::anyhow!("url is required to create MCP session {session_id}"))?;
        let session = Arc::new(launch_and_navigate(&url).await?);
        self.sessions
            .insert(session_id.to_owned(), Arc::clone(&session));
        Ok(session)
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
        browser_use_mcp::AGENT_TOOL_NAME => {
            let input: browser_use_mcp::AgentToolInput = serde_json::from_value(arguments)?;
            let provider = LlmProvider::from_mcp(input.provider);
            let llm = configured_chat_model(provider, None, input.model, input.base_url)?;
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
                        state: None,
                    }
                }
                browser_use_mcp::SessionOperation::List => browser_use_mcp::SessionToolOutput {
                    session: None,
                    sessions: list_session_records()?,
                    state: None,
                },
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
) -> anyhow::Result<Box<dyn ChatModel>> {
    match provider {
        LlmProvider::OpenAiCompatible => {
            let api_key = api_key
                .or_else(|| nonempty_env("OPENAI_API_KEY"))
                .ok_or_else(|| anyhow::anyhow!("OPENAI_API_KEY or --api-key is required"))?;
            let model = model
                .or_else(|| nonempty_env("OPENAI_MODEL"))
                .ok_or_else(|| anyhow::anyhow!("OPENAI_MODEL or --model is required"))?;
            let base_url = base_url
                .or_else(|| nonempty_env("OPENAI_BASE_URL"))
                .unwrap_or_else(|| "https://api.openai.com/v1".to_owned());
            Ok(Box::new(
                OpenAiCompatibleChatModel::new(api_key, model).with_base_url(base_url),
            ))
        }
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
            "--max-failures",
            "2",
            "--max-actions-per-step",
            "1",
            "--llm-timeout-seconds",
            "11",
            "--step-timeout-seconds",
            "22",
            "--no-loop-detection",
            "--loop-detection-window",
            "4",
            "--no-thinking",
            "--flash-mode",
            "--no-planning",
            "--planning-replan-on-stall",
            "5",
            "--planning-exploration-limit",
            "6",
            "--max-history-items",
            "7",
            "--max-clickable-elements-length",
            "8000",
            "--include-attribute",
            "data-testid",
            "--include-attribute",
            "aria-label",
        ])
        .expect("agent settings flags should parse");

        match cli.command.expect("agent command") {
            Command::Agent {
                provider,
                max_steps,
                no_vision,
                max_failures,
                max_actions_per_step,
                llm_timeout_seconds,
                step_timeout_seconds,
                no_loop_detection,
                loop_detection_window,
                no_thinking,
                flash_mode,
                no_planning,
                planning_replan_on_stall,
                planning_exploration_limit,
                max_history_items,
                max_clickable_elements_length,
                include_attributes,
                ..
            } => {
                assert_eq!(provider, LlmProvider::OpenAiCompatible);
                assert_eq!(max_steps, 3);
                assert!(no_vision);
                assert_eq!(max_failures, Some(2));
                assert_eq!(max_actions_per_step, Some(1));
                assert_eq!(llm_timeout_seconds, Some(11));
                assert_eq!(step_timeout_seconds, Some(22));
                assert!(no_loop_detection);
                assert_eq!(loop_detection_window, Some(4));
                assert!(no_thinking);
                assert!(flash_mode);
                assert!(no_planning);
                assert_eq!(planning_replan_on_stall, Some(5));
                assert_eq!(planning_exploration_limit, Some(6));
                assert_eq!(max_history_items, Some(7));
                assert_eq!(max_clickable_elements_length, Some(8000));
                assert_eq!(include_attributes, ["data-testid", "aria-label"]);
            }
            _ => panic!("expected agent command"),
        }
    }

    #[test]
    fn builds_agent_settings_from_cli_flags() {
        let settings = cli_agent_settings(CliAgentSettingsArgs {
            no_vision: true,
            max_failures: Some(2),
            max_actions_per_step: Some(1),
            llm_timeout_seconds: Some(11),
            step_timeout_seconds: Some(22),
            no_loop_detection: true,
            loop_detection_window: Some(4),
            no_thinking: true,
            flash_mode: true,
            no_planning: true,
            planning_replan_on_stall: Some(5),
            planning_exploration_limit: Some(6),
            max_history_items: Some(7),
            max_clickable_elements_length: Some(8000),
            include_attributes: vec!["data-testid".to_owned(), "aria-label".to_owned()],
        });

        assert!(!settings.use_vision);
        assert_eq!(settings.max_failures, 2);
        assert_eq!(settings.max_actions_per_step, 1);
        assert_eq!(settings.llm_timeout_seconds, 11);
        assert_eq!(settings.step_timeout_seconds, 22);
        assert!(!settings.loop_detection_enabled);
        assert_eq!(settings.loop_detection_window, 4);
        assert!(!settings.use_thinking);
        assert!(settings.flash_mode);
        assert!(!settings.enable_planning);
        assert_eq!(settings.planning_replan_on_stall, 5);
        assert_eq!(settings.planning_exploration_limit, 6);
        assert_eq!(settings.max_history_items, Some(7));
        assert_eq!(settings.max_clickable_elements_length, 8000);
        assert_eq!(settings.include_attributes, ["data-testid", "aria-label"]);
    }
}
