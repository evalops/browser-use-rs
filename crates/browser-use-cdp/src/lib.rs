//! Chrome DevTools Protocol browser-session layer.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use browser_use_dom::{
    BrowserStateSummary, DomElementRef, ElementBounds, PageInfo, SerializedDomState, TabInfo,
};
use futures_util::{SinkExt, StreamExt};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tempfile::TempDir;
use thiserror::Error;
use tokio::net::TcpStream;
use tokio::process::{Child, Command};
use tokio::sync::Mutex;
use tokio::time::sleep;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, connect_async};

const INTERACTIVE_ELEMENTS_JS: &str = r#"
(() => {
  const selector = [
    'a[href]',
    'button',
    'input',
    'textarea',
    'select',
    '[role="button"]',
    '[role="link"]',
    '[onclick]',
    '[tabindex]:not([tabindex="-1"])'
  ].join(',');
  const all = Array.from(document.querySelectorAll(selector));
  const visible = all.filter((el) => {
    const rect = el.getBoundingClientRect();
    const style = window.getComputedStyle(el);
    return rect.width > 0 && rect.height > 0 && style.display !== 'none' && style.visibility !== 'hidden';
  });
  return visible.slice(0, 400).map((el, offset) => {
    const rect = el.getBoundingClientRect();
    const attrs = {};
    for (const name of ['id', 'class', 'name', 'type', 'placeholder', 'href', 'aria-label', 'role', 'title']) {
      const value = el.getAttribute(name);
      if (value) attrs[name] = value;
    }
    const text = (el.innerText || el.value || '').trim().slice(0, 200);
    const name = (el.getAttribute('aria-label') || el.getAttribute('title') || el.getAttribute('placeholder') || text || '').trim();
    return {
      index: offset + 1,
      tag_name: el.tagName.toLowerCase(),
      role: el.getAttribute('role'),
      name,
      text,
      attributes: attrs,
      bounds: {
        x: Math.round(rect.x),
        y: Math.round(rect.y),
        width: Math.round(rect.width),
        height: Math.round(rect.height)
      },
      is_visible: true,
      is_interactive: true
    };
  });
})()
"#;

const PAGE_INFO_JS: &str = r#"
JSON.stringify((() => {
  const documentElement = document.documentElement;
  const body = document.body || documentElement;
  const viewportWidth = Math.round(window.innerWidth || documentElement.clientWidth || 0);
  const viewportHeight = Math.round(window.innerHeight || documentElement.clientHeight || 0);
  const pageWidth = Math.round(Math.max(
    body.scrollWidth,
    body.offsetWidth,
    documentElement.clientWidth,
    documentElement.scrollWidth,
    documentElement.offsetWidth
  ));
  const pageHeight = Math.round(Math.max(
    body.scrollHeight,
    body.offsetHeight,
    documentElement.clientHeight,
    documentElement.scrollHeight,
    documentElement.offsetHeight
  ));
  const scrollX = Math.round(window.scrollX || window.pageXOffset || 0);
  const scrollY = Math.round(window.scrollY || window.pageYOffset || 0);
  return {
    viewport_width: viewportWidth,
    viewport_height: viewportHeight,
    page_width: pageWidth,
    page_height: pageHeight,
    scroll_x: scrollX,
    scroll_y: scrollY,
    pixels_above: Math.max(0, scrollY),
    pixels_below: Math.max(0, pageHeight - viewportHeight - scrollY),
    pixels_left: Math.max(0, scrollX),
    pixels_right: Math.max(0, pageWidth - viewportWidth - scrollX)
  };
})())
"#;

fn element_action_js(index: u32, action: &str) -> String {
    format!(
        r#"
(() => {{
  const selector = [
    'a[href]',
    'button',
    'input',
    'textarea',
    'select',
    '[role="button"]',
    '[role="link"]',
    '[onclick]',
    '[tabindex]:not([tabindex="-1"])'
  ].join(',');
  const elements = Array.from(document.querySelectorAll(selector)).filter((el) => {{
    const rect = el.getBoundingClientRect();
    const style = window.getComputedStyle(el);
    return rect.width > 0 && rect.height > 0 && style.display !== 'none' && style.visibility !== 'hidden';
  }});
  const el = elements[{zero_based}];
  if (!el) throw new Error('No interactive element found for index {index}');
  el.scrollIntoView({{ block: 'center', inline: 'center' }});
  {action}
  return true;
}})()
"#,
        zero_based = index.saturating_sub(1),
        index = index,
        action = action
    )
}

#[derive(Debug, Error)]
pub enum BrowserError {
    #[error("browser is not connected")]
    NotConnected,
    #[error("Chrome/Chromium executable not found; checked: {0:?}")]
    ExecutableNotFound(Vec<PathBuf>),
    #[error("browser launch failed: {0}")]
    LaunchFailed(String),
    #[error("timed out waiting for DevToolsActivePort at {0}")]
    DevToolsEndpointTimedOut(PathBuf),
    #[error("CDP transport error: {0}")]
    Transport(String),
    #[error("CDP command {method} failed: {message}")]
    CommandFailed { method: String, message: String },
    #[error("CDP response for {0} was missing expected data")]
    MissingResponseData(String),
    #[error("navigation failed: {0}")]
    NavigationFailed(String),
    #[error("action failed: {0}")]
    ActionFailed(String),
    #[error("browser state unavailable: {0}")]
    StateUnavailable(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Screenshot {
    pub base64_png: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct BrowserViewport {
    pub width: u32,
    pub height: u32,
}

impl Default for BrowserViewport {
    fn default() -> Self {
        Self {
            width: 1280,
            height: 720,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ProxySettings {
    pub server: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub password: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct BrowserProfile {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cdp_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub executable_path: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote_debugging_port: Option<u16>,
    #[serde(default = "default_headless")]
    pub headless: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_data_dir: Option<PathBuf>,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub allowed_domains: Vec<String>,
    #[serde(default)]
    pub prohibited_domains: Vec<String>,
    #[serde(default)]
    pub viewport: BrowserViewport,
    #[serde(default = "default_browser_start_timeout_ms")]
    pub browser_start_timeout_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proxy: Option<ProxySettings>,
}

impl Default for BrowserProfile {
    fn default() -> Self {
        Self {
            cdp_url: None,
            executable_path: None,
            remote_debugging_port: None,
            headless: default_headless(),
            user_data_dir: None,
            args: Vec::new(),
            allowed_domains: Vec::new(),
            prohibited_domains: Vec::new(),
            viewport: BrowserViewport::default(),
            browser_start_timeout_ms: default_browser_start_timeout_ms(),
            proxy: None,
        }
    }
}

fn default_headless() -> bool {
    true
}

fn default_browser_start_timeout_ms() -> u64 {
    10_000
}

impl BrowserProfile {
    pub fn resolve_executable(&self) -> Result<PathBuf, BrowserError> {
        resolve_chrome_executable(
            self.executable_path.as_deref(),
            std::env::var_os("BROWSER_USE_CHROME").map(PathBuf::from),
            default_chrome_candidates(),
        )
    }

    #[must_use]
    pub fn launch_plan(&self) -> BrowserLaunchPlan {
        let remote_debugging_port = self.remote_debugging_port.unwrap_or(0);
        let mut args = vec![
            format!("--remote-debugging-port={remote_debugging_port}"),
            "--no-first-run".to_owned(),
            "--no-default-browser-check".to_owned(),
            format!(
                "--window-size={},{}",
                self.viewport.width, self.viewport.height
            ),
        ];

        if self.headless {
            args.push("--headless=new".to_owned());
        }

        if let Some(user_data_dir) = &self.user_data_dir {
            args.push(format!("--user-data-dir={}", user_data_dir.display()));
        }

        if let Some(proxy) = &self.proxy {
            args.push(format!("--proxy-server={}", proxy.server));
        }

        args.extend(self.args.iter().cloned());

        BrowserLaunchPlan {
            executable_path: self.executable_path.clone(),
            args,
        }
    }

    pub async fn launch_local(&self) -> Result<LaunchedBrowser, BrowserError> {
        let executable_path = self.resolve_executable()?;
        let (user_data_dir, owned_user_data_dir) = match &self.user_data_dir {
            Some(path) => (path.clone(), None),
            None => {
                let temp_dir = tempfile::Builder::new()
                    .prefix("browser-use-rs-")
                    .tempdir()
                    .map_err(|error| BrowserError::LaunchFailed(error.to_string()))?;
                (temp_dir.path().to_path_buf(), Some(temp_dir))
            }
        };

        let mut launch_profile = self.clone();
        launch_profile.executable_path = Some(executable_path.clone());
        launch_profile.user_data_dir = Some(user_data_dir.clone());
        let plan = launch_profile.launch_plan();

        let mut command = Command::new(&executable_path);
        command
            .args(&plan.args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());

        let mut child = command
            .spawn()
            .map_err(|error| BrowserError::LaunchFailed(error.to_string()))?;

        match wait_for_devtools_endpoint(&user_data_dir, self.browser_start_timeout_ms).await {
            Ok(endpoint) => Ok(LaunchedBrowser {
                child,
                endpoint,
                _user_data_dir: owned_user_data_dir,
            }),
            Err(error) => {
                let _ = child.start_kill();
                Err(error)
            }
        }
    }
}

pub fn resolve_chrome_executable<I>(
    explicit_path: Option<&Path>,
    env_override: Option<PathBuf>,
    candidates: I,
) -> Result<PathBuf, BrowserError>
where
    I: IntoIterator<Item = PathBuf>,
{
    let mut checked = Vec::new();

    if let Some(path) = explicit_path {
        checked.push(path.to_path_buf());
        if path.exists() {
            return Ok(path.to_path_buf());
        }
    }

    if let Some(path) = env_override {
        checked.push(path.clone());
        if path.exists() {
            return Ok(path);
        }
    }

    for path in candidates {
        checked.push(path.clone());
        if path.exists() {
            return Ok(path);
        }
    }

    Err(BrowserError::ExecutableNotFound(checked))
}

#[must_use]
pub fn default_chrome_candidates() -> Vec<PathBuf> {
    let mut candidates = Vec::new();

    #[cfg(target_os = "macos")]
    {
        candidates.push(PathBuf::from(
            "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
        ));
        candidates.push(PathBuf::from(
            "/Applications/Chromium.app/Contents/MacOS/Chromium",
        ));
        candidates.push(PathBuf::from(
            "/Applications/Google Chrome Canary.app/Contents/MacOS/Google Chrome Canary",
        ));
    }

    #[cfg(target_os = "linux")]
    {
        candidates.push(PathBuf::from("/usr/bin/google-chrome"));
        candidates.push(PathBuf::from("/usr/bin/google-chrome-stable"));
        candidates.push(PathBuf::from("/usr/bin/chromium"));
        candidates.push(PathBuf::from("/usr/bin/chromium-browser"));
    }

    #[cfg(target_os = "windows")]
    {
        if let Some(program_files) = std::env::var_os("PROGRAMFILES") {
            candidates
                .push(PathBuf::from(program_files).join("Google/Chrome/Application/chrome.exe"));
        }
        if let Some(program_files_x86) = std::env::var_os("PROGRAMFILES(X86)") {
            candidates.push(
                PathBuf::from(program_files_x86).join("Google/Chrome/Application/chrome.exe"),
            );
        }
        if let Some(local_app_data) = std::env::var_os("LOCALAPPDATA") {
            candidates
                .push(PathBuf::from(local_app_data).join("Google/Chrome/Application/chrome.exe"));
        }
    }

    candidates
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct BrowserLaunchPlan {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub executable_path: Option<PathBuf>,
    pub args: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct DevToolsEndpoint {
    pub http_url: String,
    pub websocket_url: String,
}

impl DevToolsEndpoint {
    pub fn from_active_port_file(
        host: &str,
        active_port_contents: &str,
    ) -> Result<Self, BrowserError> {
        let mut lines = active_port_contents.lines();
        let port = lines
            .next()
            .ok_or_else(|| {
                BrowserError::StateUnavailable("DevToolsActivePort missing port".to_owned())
            })?
            .trim();
        let browser_path = lines
            .next()
            .ok_or_else(|| {
                BrowserError::StateUnavailable("DevToolsActivePort missing browser path".to_owned())
            })?
            .trim();

        if port.is_empty() || browser_path.is_empty() {
            return Err(BrowserError::StateUnavailable(
                "DevToolsActivePort contains empty endpoint fields".to_owned(),
            ));
        }

        Ok(Self {
            http_url: format!("http://{host}:{port}"),
            websocket_url: format!("ws://{host}:{port}{browser_path}"),
        })
    }
}

pub struct LaunchedBrowser {
    child: Child,
    endpoint: DevToolsEndpoint,
    _user_data_dir: Option<TempDir>,
}

impl LaunchedBrowser {
    #[must_use]
    pub fn endpoint(&self) -> &DevToolsEndpoint {
        &self.endpoint
    }

    #[must_use]
    pub fn process_id(&self) -> Option<u32> {
        self.child.id()
    }
}

impl Drop for LaunchedBrowser {
    fn drop(&mut self) {
        let _ = self.child.start_kill();
    }
}

#[must_use]
pub fn devtools_active_port_path(user_data_dir: &Path) -> PathBuf {
    user_data_dir.join("DevToolsActivePort")
}

pub async fn wait_for_devtools_endpoint(
    user_data_dir: &Path,
    timeout_ms: u64,
) -> Result<DevToolsEndpoint, BrowserError> {
    let active_port_path = devtools_active_port_path(user_data_dir);
    let deadline = Instant::now() + Duration::from_millis(timeout_ms);

    loop {
        match tokio::fs::read_to_string(&active_port_path).await {
            Ok(contents) => match DevToolsEndpoint::from_active_port_file("127.0.0.1", &contents) {
                Ok(endpoint) => return Ok(endpoint),
                Err(error @ BrowserError::StateUnavailable(_)) => {
                    if Instant::now() >= deadline {
                        return Err(error);
                    }
                    sleep(Duration::from_millis(50)).await;
                }
                Err(error) => return Err(error),
            },
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                if Instant::now() >= deadline {
                    return Err(BrowserError::DevToolsEndpointTimedOut(active_port_path));
                }
                sleep(Duration::from_millis(50)).await;
            }
            Err(error) => return Err(BrowserError::StateUnavailable(error.to_string())),
        }
    }
}

type CdpSocket = WebSocketStream<MaybeTlsStream<TcpStream>>;

pub struct CdpConnection {
    socket: Mutex<CdpSocket>,
    next_id: AtomicU64,
}

impl CdpConnection {
    pub async fn connect(endpoint: &DevToolsEndpoint) -> Result<Self, BrowserError> {
        let (socket, _) = connect_async(&endpoint.websocket_url)
            .await
            .map_err(|error| BrowserError::Transport(error.to_string()))?;

        Ok(Self {
            socket: Mutex::new(socket),
            next_id: AtomicU64::new(1),
        })
    }

    pub async fn command(
        &self,
        method: &str,
        params: Value,
        session_id: Option<&str>,
    ) -> Result<Value, BrowserError> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let mut request = json!({
            "id": id,
            "method": method,
            "params": params,
        });

        if let Some(session_id) = session_id {
            request["sessionId"] = Value::String(session_id.to_owned());
        }

        let mut socket = self.socket.lock().await;
        socket
            .send(Message::Text(request.to_string().into()))
            .await
            .map_err(|error| BrowserError::Transport(error.to_string()))?;

        while let Some(message) = socket.next().await {
            let message = message.map_err(|error| BrowserError::Transport(error.to_string()))?;
            let Message::Text(text) = message else {
                continue;
            };
            let payload: Value = serde_json::from_str(&text)
                .map_err(|error| BrowserError::Transport(error.to_string()))?;

            if payload.get("id").and_then(Value::as_u64) != Some(id) {
                continue;
            }

            if let Some(error) = payload.get("error") {
                let message = error
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown CDP error")
                    .to_owned();
                return Err(BrowserError::CommandFailed {
                    method: method.to_owned(),
                    message,
                });
            }

            return payload
                .get("result")
                .cloned()
                .ok_or_else(|| BrowserError::MissingResponseData(format!("{method} result")));
        }

        Err(BrowserError::Transport(
            "CDP websocket closed while waiting for response".to_owned(),
        ))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttachedPage {
    pub target_id: String,
    pub session_id: String,
}

pub struct CdpBrowserSession {
    connection: CdpConnection,
    page: AttachedPage,
    _launched_browser: Option<LaunchedBrowser>,
}

impl CdpBrowserSession {
    pub async fn connect(endpoint: DevToolsEndpoint) -> Result<Self, BrowserError> {
        let connection = CdpConnection::connect(&endpoint).await?;
        let page = attach_or_create_page(&connection).await?;

        Ok(Self {
            connection,
            page,
            _launched_browser: None,
        })
    }

    pub async fn launch(profile: &BrowserProfile) -> Result<Self, BrowserError> {
        let launched_browser = profile.launch_local().await?;
        let connection = CdpConnection::connect(launched_browser.endpoint()).await?;
        let page = attach_or_create_page(&connection).await?;

        Ok(Self {
            connection,
            page,
            _launched_browser: Some(launched_browser),
        })
    }

    async fn evaluate_json(&self, expression: &str) -> Result<Value, BrowserError> {
        let result = self
            .connection
            .command(
                "Runtime.evaluate",
                json!({
                    "expression": expression,
                    "returnByValue": true,
                    "awaitPromise": true,
                }),
                Some(&self.page.session_id),
            )
            .await?;

        runtime_evaluate_value(result)
    }

    async fn evaluate_effect(&self, expression: String) -> Result<(), BrowserError> {
        let result = self
            .connection
            .command(
                "Runtime.evaluate",
                json!({
                    "expression": expression,
                    "awaitPromise": true,
                    "returnByValue": true,
                }),
                Some(&self.page.session_id),
            )
            .await?;
        let _ = runtime_evaluate_value(result)?;
        Ok(())
    }

    async fn page_location(&self) -> Result<(String, String), BrowserError> {
        let value = self
            .evaluate_json("JSON.stringify({ url: location.href, title: document.title })")
            .await?;
        let encoded = value.as_str().ok_or_else(|| {
            BrowserError::MissingResponseData("Runtime.evaluate string value".to_owned())
        })?;
        let page: Value = serde_json::from_str(encoded)
            .map_err(|error| BrowserError::Transport(error.to_string()))?;

        Ok((
            page.get("url")
                .and_then(Value::as_str)
                .unwrap_or("about:blank")
                .to_owned(),
            page.get("title")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_owned(),
        ))
    }

    async fn page_info(&self) -> Result<PageInfo, BrowserError> {
        let value = self.evaluate_json(PAGE_INFO_JS).await?;
        let encoded = value.as_str().ok_or_else(|| {
            BrowserError::MissingResponseData("Runtime.evaluate page info".to_owned())
        })?;
        let page_info: Value = serde_json::from_str(encoded)
            .map_err(|error| BrowserError::Transport(error.to_string()))?;

        page_info_from_value(&page_info)
            .ok_or_else(|| BrowserError::MissingResponseData("page info fields".to_owned()))
    }

    async fn dom_state(&self) -> Result<SerializedDomState, BrowserError> {
        let value = self.evaluate_json(INTERACTIVE_ELEMENTS_JS).await?;
        let elements = value
            .as_array()
            .ok_or_else(|| {
                BrowserError::MissingResponseData("interactive element array".to_owned())
            })?
            .iter()
            .map(|element| self.dom_element_from_value(element))
            .collect::<Result<Vec<_>, _>>()?;

        Ok(SerializedDomState::from_elements(elements))
    }

    fn dom_element_from_value(&self, value: &Value) -> Result<DomElementRef, BrowserError> {
        let index = value
            .get("index")
            .and_then(Value::as_u64)
            .and_then(|index| u32::try_from(index).ok())
            .ok_or_else(|| BrowserError::MissingResponseData("element index".to_owned()))?;
        let attributes = value
            .get("attributes")
            .and_then(Value::as_object)
            .map(|attrs| {
                attrs
                    .iter()
                    .filter_map(|(key, value)| {
                        value.as_str().map(|value| (key.clone(), value.to_owned()))
                    })
                    .collect()
            })
            .unwrap_or_default();

        Ok(DomElementRef {
            index,
            target_id: self.page.target_id.clone(),
            backend_node_id: 0,
            node_id: None,
            tag_name: value
                .get("tag_name")
                .and_then(Value::as_str)
                .unwrap_or("element")
                .to_owned(),
            role: value.get("role").and_then(Value::as_str).map(str::to_owned),
            name: value.get("name").and_then(Value::as_str).map(str::to_owned),
            text: value.get("text").and_then(Value::as_str).map(str::to_owned),
            attributes,
            bounds: element_bounds_from_value(value),
            is_visible: value
                .get("is_visible")
                .and_then(Value::as_bool)
                .unwrap_or(true),
            is_interactive: value
                .get("is_interactive")
                .and_then(Value::as_bool)
                .unwrap_or(true),
        })
    }
}

fn element_bounds_from_value(value: &Value) -> Option<ElementBounds> {
    let bounds = value.get("bounds")?;
    Some(ElementBounds {
        x: bounds
            .get("x")?
            .as_i64()
            .and_then(|x| i32::try_from(x).ok())?,
        y: bounds
            .get("y")?
            .as_i64()
            .and_then(|y| i32::try_from(y).ok())?,
        width: bounds
            .get("width")?
            .as_u64()
            .and_then(|width| u32::try_from(width).ok())?,
        height: bounds
            .get("height")?
            .as_u64()
            .and_then(|height| u32::try_from(height).ok())?,
    })
}

fn page_info_from_value(value: &Value) -> Option<PageInfo> {
    Some(PageInfo {
        viewport_width: u32_field(value, "viewport_width")?,
        viewport_height: u32_field(value, "viewport_height")?,
        page_width: u32_field(value, "page_width")?,
        page_height: u32_field(value, "page_height")?,
        scroll_x: i32_field(value, "scroll_x")?,
        scroll_y: i32_field(value, "scroll_y")?,
        pixels_above: u32_field(value, "pixels_above")?,
        pixels_below: u32_field(value, "pixels_below")?,
        pixels_left: u32_field(value, "pixels_left")?,
        pixels_right: u32_field(value, "pixels_right")?,
    })
}

fn u32_field(value: &Value, field: &str) -> Option<u32> {
    value
        .get(field)?
        .as_u64()
        .and_then(|number| u32::try_from(number).ok())
}

fn i32_field(value: &Value, field: &str) -> Option<i32> {
    value
        .get(field)?
        .as_i64()
        .and_then(|number| i32::try_from(number).ok())
}

fn runtime_evaluate_value(result: Value) -> Result<Value, BrowserError> {
    if let Some(exception) = result.get("exceptionDetails") {
        let message = exception
            .get("text")
            .and_then(Value::as_str)
            .unwrap_or("Runtime.evaluate exception")
            .to_owned();
        return Err(BrowserError::CommandFailed {
            method: "Runtime.evaluate".to_owned(),
            message,
        });
    }

    result
        .get("result")
        .and_then(|result| result.get("value"))
        .cloned()
        .ok_or_else(|| BrowserError::MissingResponseData("Runtime.evaluate value".to_owned()))
}

async fn attach_or_create_page(connection: &CdpConnection) -> Result<AttachedPage, BrowserError> {
    let targets = connection
        .command("Target.getTargets", json!({}), None)
        .await?;
    let page_target = targets
        .get("targetInfos")
        .and_then(Value::as_array)
        .and_then(|targets| {
            targets
                .iter()
                .find(|target| {
                    target.get("type").and_then(Value::as_str) == Some("page")
                        && target.get("url").and_then(Value::as_str) != Some("chrome://newtab/")
                })
                .or_else(|| {
                    targets
                        .iter()
                        .find(|target| target.get("type").and_then(Value::as_str) == Some("page"))
                })
        })
        .and_then(|target| {
            target
                .get("targetId")
                .and_then(Value::as_str)
                .map(str::to_owned)
        });

    let target_id = match page_target {
        Some(target_id) => target_id,
        None => connection
            .command("Target.createTarget", json!({ "url": "about:blank" }), None)
            .await?
            .get("targetId")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .ok_or_else(|| {
                BrowserError::MissingResponseData("Target.createTarget targetId".to_owned())
            })?,
    };

    let session_id = connection
        .command(
            "Target.attachToTarget",
            json!({
                "targetId": target_id,
                "flatten": true,
            }),
            None,
        )
        .await?
        .get("sessionId")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| {
            BrowserError::MissingResponseData("Target.attachToTarget sessionId".to_owned())
        })?;

    Ok(AttachedPage {
        target_id,
        session_id,
    })
}

#[async_trait]
impl BrowserSession for CdpBrowserSession {
    async fn state(&self, include_screenshot: bool) -> Result<BrowserStateSummary, BrowserError> {
        let (url, title) = self.page_location().await?;
        let page_info = self.page_info().await?;
        let dom_state = self.dom_state().await?;
        let screenshot = if include_screenshot {
            Some(self.screenshot().await?.base64_png)
        } else {
            None
        };

        Ok(BrowserStateSummary {
            dom_state,
            url: url.clone(),
            title: title.clone(),
            tabs: vec![TabInfo {
                url,
                title,
                target_id: self.page.target_id.clone(),
                parent_target_id: None,
            }],
            screenshot,
            page_info: Some(page_info),
            pixels_above: page_info.pixels_above,
            pixels_below: page_info.pixels_below,
            browser_errors: vec![],
            is_pdf_viewer: false,
            recent_events: None,
            pending_network_requests: vec![],
            pagination_buttons: vec![],
            closed_popup_messages: vec![],
        })
    }

    async fn navigate(&self, url: &str, _new_tab: bool) -> Result<(), BrowserError> {
        self.connection
            .command(
                "Page.navigate",
                json!({
                    "url": url,
                }),
                Some(&self.page.session_id),
            )
            .await?;
        Ok(())
    }

    async fn click(&self, index: u32) -> Result<(), BrowserError> {
        self.evaluate_effect(element_action_js(index, "el.click();"))
            .await
    }

    async fn click_coordinates(&self, x: i32, y: i32) -> Result<(), BrowserError> {
        for event_type in ["mousePressed", "mouseReleased"] {
            self.connection
                .command(
                    "Input.dispatchMouseEvent",
                    json!({
                        "type": event_type,
                        "x": x,
                        "y": y,
                        "button": "left",
                        "clickCount": 1,
                    }),
                    Some(&self.page.session_id),
                )
                .await?;
        }
        Ok(())
    }

    async fn input_text(&self, index: u32, text: &str, clear: bool) -> Result<(), BrowserError> {
        let text_json = serde_json::to_string(text)
            .map_err(|error| BrowserError::Transport(error.to_string()))?;
        let action = if clear {
            format!(
                "el.focus(); el.value = {text_json}; el.dispatchEvent(new Event('input', {{ bubbles: true }})); el.dispatchEvent(new Event('change', {{ bubbles: true }}));"
            )
        } else {
            format!(
                "el.focus(); el.value = (el.value || '') + {text_json}; el.dispatchEvent(new Event('input', {{ bubbles: true }})); el.dispatchEvent(new Event('change', {{ bubbles: true }}));"
            )
        };
        self.evaluate_effect(element_action_js(index, &action))
            .await
    }

    async fn scroll(
        &self,
        _index: Option<u32>,
        down: bool,
        pages: f64,
    ) -> Result<(), BrowserError> {
        let direction = if down { 1.0 } else { -1.0 };
        self.evaluate_effect(format!(
            "window.scrollBy(0, window.innerHeight * {}); true;",
            pages * direction
        ))
        .await
    }

    async fn screenshot(&self) -> Result<Screenshot, BrowserError> {
        let result = self
            .connection
            .command(
                "Page.captureScreenshot",
                json!({
                    "format": "png",
                    "fromSurface": true,
                }),
                Some(&self.page.session_id),
            )
            .await?;

        let base64_png = result
            .get("data")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .ok_or_else(|| {
                BrowserError::MissingResponseData("Page.captureScreenshot data".to_owned())
            })?;

        Ok(Screenshot { base64_png })
    }
}

#[async_trait]
pub trait BrowserSession: Send + Sync {
    async fn state(&self, include_screenshot: bool) -> Result<BrowserStateSummary, BrowserError>;

    async fn navigate(&self, url: &str, new_tab: bool) -> Result<(), BrowserError>;

    async fn click(&self, index: u32) -> Result<(), BrowserError>;

    async fn click_coordinates(&self, x: i32, y: i32) -> Result<(), BrowserError>;

    async fn input_text(&self, index: u32, text: &str, clear: bool) -> Result<(), BrowserError>;

    async fn scroll(&self, index: Option<u32>, down: bool, pages: f64) -> Result<(), BrowserError>;

    async fn screenshot(&self) -> Result<Screenshot, BrowserError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_profile_uses_headless_chrome_args() {
        let plan = BrowserProfile::default().launch_plan();

        assert!(plan.args.contains(&"--headless=new".to_owned()));
        assert!(plan.args.contains(&"--remote-debugging-port=0".to_owned()));
        assert!(plan.args.contains(&"--window-size=1280,720".to_owned()));
    }

    #[test]
    fn profile_can_pin_remote_debugging_port() {
        let profile = BrowserProfile {
            remote_debugging_port: Some(9222),
            ..BrowserProfile::default()
        };
        let plan = profile.launch_plan();

        assert!(
            plan.args
                .contains(&"--remote-debugging-port=9222".to_owned())
        );
    }

    #[test]
    fn launch_plan_preserves_profile_and_custom_args_order() {
        let profile = BrowserProfile {
            headless: false,
            user_data_dir: Some(PathBuf::from("/tmp/browser-use-rs-profile")),
            args: vec!["--disable-gpu".to_owned()],
            proxy: Some(ProxySettings {
                server: "http://127.0.0.1:8080".to_owned(),
                username: None,
                password: None,
            }),
            ..BrowserProfile::default()
        };

        let plan = profile.launch_plan();

        assert!(!plan.args.contains(&"--headless=new".to_owned()));
        assert!(
            plan.args
                .contains(&"--user-data-dir=/tmp/browser-use-rs-profile".to_owned())
        );
        assert!(
            plan.args
                .contains(&"--proxy-server=http://127.0.0.1:8080".to_owned())
        );
        assert_eq!(plan.args.last(), Some(&"--disable-gpu".to_owned()));
    }

    #[test]
    fn parses_devtools_active_port_endpoint() {
        let endpoint = DevToolsEndpoint::from_active_port_file(
            "127.0.0.1",
            "38119\n/devtools/browser/abc123\n",
        )
        .expect("parse endpoint");

        assert_eq!(endpoint.http_url, "http://127.0.0.1:38119");
        assert_eq!(
            endpoint.websocket_url,
            "ws://127.0.0.1:38119/devtools/browser/abc123"
        );
    }

    #[test]
    fn active_port_path_lives_under_user_data_dir() {
        assert_eq!(
            devtools_active_port_path(Path::new("/tmp/profile")),
            PathBuf::from("/tmp/profile/DevToolsActivePort")
        );
    }

    #[test]
    fn parses_page_info_metrics() {
        let page_info = page_info_from_value(&json!({
            "viewport_width": 1280,
            "viewport_height": 720,
            "page_width": 1280,
            "page_height": 2000,
            "scroll_x": 0,
            "scroll_y": 300,
            "pixels_above": 300,
            "pixels_below": 980,
            "pixels_left": 0,
            "pixels_right": 0
        }))
        .expect("page info");

        assert_eq!(page_info.scroll_y, 300);
        assert_eq!(page_info.pixels_below, 980);
    }

    #[test]
    fn executable_resolution_prefers_explicit_path() {
        let current_exe = std::env::current_exe().expect("current exe");
        let resolved = resolve_chrome_executable(Some(&current_exe), None, Vec::<PathBuf>::new())
            .expect("resolve executable");

        assert_eq!(resolved, current_exe);
    }

    #[test]
    fn executable_resolution_reports_checked_paths() {
        let missing = PathBuf::from("/definitely/not/a/chrome");
        let error = resolve_chrome_executable(None, None, vec![missing.clone()])
            .expect_err("missing executable");

        match error {
            BrowserError::ExecutableNotFound(checked) => assert_eq!(checked, vec![missing]),
            other => panic!("unexpected error: {other}"),
        }
    }

    #[tokio::test]
    async fn waits_for_devtools_endpoint_file() {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let active_port_path = devtools_active_port_path(temp_dir.path());
        tokio::fs::write(&active_port_path, "38119\n/devtools/browser/abc123\n")
            .await
            .expect("write endpoint");

        let endpoint = wait_for_devtools_endpoint(temp_dir.path(), 100)
            .await
            .expect("endpoint");

        assert_eq!(endpoint.http_url, "http://127.0.0.1:38119");
    }

    #[tokio::test]
    #[ignore = "requires Chrome/Chromium installed on the local machine"]
    async fn launches_local_chrome_when_available() {
        let profile = BrowserProfile::default();
        let browser = profile.launch_local().await.expect("launch local browser");

        assert!(browser.process_id().is_some());
        assert!(browser.endpoint().http_url.starts_with("http://127.0.0.1:"));
        assert!(
            browser
                .endpoint()
                .websocket_url
                .starts_with("ws://127.0.0.1:")
        );
    }

    #[tokio::test]
    #[ignore = "requires Chrome/Chromium installed on the local machine"]
    async fn cdp_session_can_navigate_read_state_and_capture_screenshot() {
        let profile = BrowserProfile::default();
        let session = CdpBrowserSession::launch(&profile)
            .await
            .expect("launch CDP session");

        session
            .navigate(
                "data:text/html,<html><head><title>browser-use-rs smoke</title></head><body><button onclick=\"document.title='clicked'\">Click me</button><input placeholder='Name'><div style='height:2000px'>Scroll target</div></body></html>",
                false,
            )
            .await
            .expect("navigate");
        sleep(Duration::from_millis(100)).await;

        let initial_state = session.state(false).await.expect("initial state");
        assert_eq!(initial_state.dom_state.element_count(), 2);
        assert!(
            initial_state
                .dom_state
                .llm_representation()
                .contains("Click me")
        );

        session.click(1).await.expect("click by index");
        sleep(Duration::from_millis(100)).await;
        session
            .input_text(2, "EvalOps", true)
            .await
            .expect("input text");
        session
            .click_coordinates(20, 20)
            .await
            .expect("coordinate click");
        session.scroll(None, true, 0.25).await.expect("scroll");

        let state = session.state(true).await.expect("state");

        assert!(state.url.starts_with("data:text/html"));
        assert_eq!(state.title, "clicked");
        assert!(
            state.dom_state.llm_representation().contains("EvalOps"),
            "DOM state did not include typed input value: {}",
            state.dom_state.llm_representation()
        );
        assert!(state.screenshot.expect("screenshot").len() > 100);
    }
}
