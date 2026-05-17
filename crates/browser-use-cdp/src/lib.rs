//! Chrome DevTools Protocol browser-session layer.

use std::path::{Path, PathBuf};

use async_trait::async_trait;
use browser_use_dom::BrowserStateSummary;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum BrowserError {
    #[error("browser is not connected")]
    NotConnected,
    #[error("Chrome/Chromium executable not found; checked: {0:?}")]
    ExecutableNotFound(Vec<PathBuf>),
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
            proxy: None,
        }
    }
}

fn default_headless() -> bool {
    true
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
}
