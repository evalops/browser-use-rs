use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use base64::Engine;
use browser_use_cdp::{BrowserProfile, BrowserSession, CdpBrowserSession};
use browser_use_core::BrowserActionExecutor;
use browser_use_llm::OpenAiCompatibleChatModel;
use clap::Parser;
use schemars::schema_for;
use serde_json::Value;
use tokio::io::{self, AsyncBufReadExt, AsyncWriteExt, BufReader};
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
enum Command {
    /// Print the frozen upstream browser-use commit target.
    VersionTarget,
    /// Print a JSON Schema for a compatibility contract.
    Schema { contract: SchemaContract },
    /// Print the MCP tool manifest JSON exposed by browser-use-mcp.
    McpTools,
    /// Run a stdio MCP server exposing browser-use tools.
    McpStdio,
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
    /// Run a bounded browser agent task through an OpenAI-compatible chat model.
    Agent {
        url: String,
        task: String,
        #[arg(long, env = "OPENAI_API_KEY")]
        api_key: String,
        #[arg(long, env = "OPENAI_MODEL")]
        model: String,
        #[arg(
            long,
            env = "OPENAI_BASE_URL",
            default_value = "https://api.openai.com/v1"
        )]
        base_url: String,
        #[arg(long, default_value_t = 10)]
        max_steps: usize,
    },
}

#[derive(Debug, Clone, Copy, clap::ValueEnum)]
enum SchemaContract {
    Action,
    AgentOutput,
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
            api_key,
            model,
            base_url,
            max_steps,
        }) => {
            let session = launch_and_navigate(&url).await?;
            let llm = OpenAiCompatibleChatModel::new(api_key, model).with_base_url(base_url);
            let mut agent = browser_use_core::Agent::new(task, llm, session);
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
            let api_key = std::env::var("OPENAI_API_KEY")?;
            let model = input
                .model
                .or_else(|| std::env::var("OPENAI_MODEL").ok())
                .ok_or_else(|| anyhow::anyhow!("OPENAI_MODEL or model input is required"))?;
            let base_url = input
                .base_url
                .or_else(|| std::env::var("OPENAI_BASE_URL").ok())
                .unwrap_or_else(|| "https://api.openai.com/v1".to_owned());
            let session = if let Some(session_id) = input.session_id {
                runtime.session(&session_id, input.url).await?
            } else {
                Arc::new(launch_and_navigate(&require_mcp_url(input.url)?).await?)
            };
            let llm = OpenAiCompatibleChatModel::new(api_key, model).with_base_url(base_url);
            let mut agent = browser_use_core::Agent::new(input.task, llm, session);
            let history = agent.run(input.max_steps).await?;
            let output = browser_use_mcp::AgentToolOutput {
                history: history.clone(),
            };
            Ok(browser_use_mcp::tool_success_result(serde_json::to_value(
                output,
            )?))
        }
        _ => unreachable!("tool name was validated before execution"),
    }
}

fn require_mcp_url(url: Option<String>) -> anyhow::Result<String> {
    url.ok_or_else(|| anyhow::anyhow!("url is required when session_id is not provided"))
}
