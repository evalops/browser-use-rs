use super::*;

pub(crate) async fn run_mcp_stdio() -> anyhow::Result<()> {
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

pub(crate) async fn run_daemon(
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
pub(crate) struct DaemonLifecycleOptions {
    pub(crate) pid_file: Option<PathBuf>,
    pub(crate) ready_file: Option<PathBuf>,
}

pub(crate) struct DaemonLifecycleFiles {
    paths: Vec<PathBuf>,
}

impl DaemonLifecycleFiles {
    pub(crate) fn write(
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

pub(crate) struct HttpRequest {
    pub(crate) method: String,
    pub(crate) path: String,
    pub(crate) headers: HashMap<String, String>,
    pub(crate) body: Vec<u8>,
}

pub(crate) struct HttpResponse {
    pub(crate) status: u16,
    pub(crate) body: Vec<u8>,
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

pub(crate) async fn read_http_request(stream: &mut TcpStream) -> anyhow::Result<HttpRequest> {
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

pub(crate) async fn handle_http_request(
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

pub(crate) fn http_request_authorized(request: &HttpRequest, auth_token: Option<&str>) -> bool {
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
pub(crate) struct McpRuntime {
    sessions: HashMap<String, Arc<CdpBrowserSession>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum McpSessionPlan {
    ReuseInMemory,
    ReconnectPersistentRecord,
    CreatePersistentRecord,
}

pub(crate) fn mcp_session_plan(
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

pub(crate) async fn handle_mcp_message(raw: &str, runtime: &mut McpRuntime) -> Option<Value> {
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
