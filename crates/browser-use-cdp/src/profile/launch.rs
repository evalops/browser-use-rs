//! Local browser launch planning and DevTools endpoint helpers.

use super::{BrowserChannel, BrowserProfile, IgnoreDefaultArgs};
use crate::BrowserError;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::mem::ManuallyDrop;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, Instant};
use tempfile::TempDir;
use tokio::process::{Child, Command};
use tokio::time::sleep;

const CHROME_DEFAULT_ARGS: &[&str] = &[
    "--disable-field-trial-config",
    "--disable-background-networking",
    "--disable-background-timer-throttling",
    "--disable-backgrounding-occluded-windows",
    "--disable-back-forward-cache",
    "--disable-breakpad",
    "--disable-client-side-phishing-detection",
    "--disable-component-update",
    "--no-default-browser-check",
    "--disable-dev-shm-usage",
    "--disable-hang-monitor",
    "--disable-ipc-flooding-protection",
    "--disable-popup-blocking",
    "--disable-prompt-on-repost",
    "--disable-renderer-backgrounding",
    "--metrics-recording-only",
    "--no-first-run",
    "--no-service-autorun",
    "--export-tagged-pdf",
    "--disable-search-engine-choice-screen",
    "--unsafely-disable-devtools-self-xss-warnings",
    "--enable-features=NetworkService,NetworkServiceInProcess",
    "--enable-network-information-downlink-max",
    "--disable-sync",
];

const DEFAULT_IGNORE_DEFAULT_ARGS: &[&str] = &[
    "--enable-automation",
    "--disable-extensions",
    "--hide-scrollbars",
    "--disable-features=AcceptCHFrame,AutoExpandDetailsElement,AvoidUnnecessaryBeforeUnloadCheckSync,CertificateTransparencyComponentUpdater,DeferRendererTasksAfterInput,DestroyProfileOnBrowserClose,DialMediaRouteProvider,ExtensionManifestV2Disabled,GlobalMediaControls,HttpsUpgrades,ImprovedCookieControls,LazyFrameLoading,LensOverlay,MediaRouter,PaintHolding,ThirdPartyStoragePartitioning,Translate",
];

pub(crate) const CHROME_DISABLE_SECURITY_ARGS: &[&str] = &[
    "--disable-site-isolation-trials",
    "--disable-web-security",
    "--disable-features=IsolateOrigins,site-per-process",
    "--allow-running-insecure-content",
    "--ignore-certificate-errors",
    "--ignore-ssl-errors",
    "--ignore-certificate-errors-spki-list",
];

pub(crate) const CHROME_DOCKER_ARGS: &[&str] = &[
    "--no-sandbox",
    "--disable-gpu-sandbox",
    "--disable-setuid-sandbox",
    "--disable-dev-shm-usage",
    "--no-xshm",
    "--no-zygote",
    "--disable-site-isolation-trials",
];

pub(crate) const CHROME_DETERMINISTIC_RENDERING_ARGS: &[&str] = &[
    "--deterministic-mode",
    "--js-flags=--random-seed=1157259159",
    "--force-device-scale-factor=2",
    "--enable-webgl",
    "--font-render-hinting=none",
    "--force-color-profile=srgb",
];

pub(crate) fn default_ignore_default_args() -> Vec<String> {
    DEFAULT_IGNORE_DEFAULT_ARGS
        .iter()
        .map(|arg| (*arg).to_owned())
        .collect()
}

impl BrowserProfile {
    /// Resolves the Chrome/Chromium executable for this profile.
    pub fn resolve_executable(&self) -> Result<PathBuf, BrowserError> {
        resolve_chrome_executable(
            self.executable_path.as_deref(),
            std::env::var_os("BROWSER_USE_CHROME").map(PathBuf::from),
            browser_executable_candidates(self.channel),
        )
    }

    /// Builds a launch plan and panics if the profile is internally invalid.
    ///
    /// This is convenient for tests and callers that already validated the
    /// profile. Use [`BrowserProfile::try_launch_plan`] for fallible paths.
    #[must_use]
    pub fn launch_plan(&self) -> BrowserLaunchPlan {
        self.try_launch_plan()
            .expect("invalid BrowserProfile launch plan")
    }

    /// Builds the local Chrome command plan without spawning the process.
    pub fn try_launch_plan(&self) -> Result<BrowserLaunchPlan, BrowserError> {
        if self.headless && self.devtools {
            return Err(BrowserError::LaunchFailed(
                "headless=True and devtools=True cannot both be set at the same time".to_owned(),
            ));
        }
        if self.headless && self.no_viewport {
            return Err(BrowserError::LaunchFailed(
                "headless=True and no_viewport=True cannot both be set at the same time".to_owned(),
            ));
        }
        Ok(self.build_launch_plan())
    }

    fn build_launch_plan(&self) -> BrowserLaunchPlan {
        let remote_debugging_port = self.remote_debugging_port.unwrap_or(0);
        let window_size = self
            .window_size
            .as_ref()
            .or(self.screen.as_ref())
            .unwrap_or(&self.viewport);
        let mut args = self.default_chrome_args();
        args.push(format!("--remote-debugging-port={remote_debugging_port}"));
        args.push(format!(
            "--window-size={},{}",
            window_size.width, window_size.height
        ));

        if let Some(window_position) = &self.window_position {
            args.push(format!(
                "--window-position={},{}",
                window_position.width, window_position.height
            ));
        }

        if self.headless {
            args.push("--headless=new".to_owned());
        }

        if self.devtools {
            args.push("--auto-open-devtools-for-tabs".to_owned());
        }

        if let Some(user_data_dir) = &self.user_data_dir {
            args.push(format!("--user-data-dir={}", user_data_dir.display()));
            if !self.profile_directory.is_empty() {
                args.push(format!("--profile-directory={}", self.profile_directory));
            }
        }

        if !self.chromium_sandbox {
            args.extend(CHROME_DOCKER_ARGS.iter().map(|arg| (*arg).to_owned()));
        }

        if self.disable_security {
            args.extend(
                CHROME_DISABLE_SECURITY_ARGS
                    .iter()
                    .map(|arg| (*arg).to_owned()),
            );
        }

        if self.deterministic_rendering {
            args.extend(
                CHROME_DETERMINISTIC_RENDERING_ARGS
                    .iter()
                    .map(|arg| (*arg).to_owned()),
            );
        }

        if let Some(proxy) = &self.proxy {
            let proxy_server = proxy.server.as_str();
            if !proxy_server.is_empty() {
                args.push(format!("--proxy-server={proxy_server}"));
                if let Some(proxy_bypass) = proxy.bypass.as_deref() {
                    if !proxy_bypass.is_empty() {
                        args.push(format!("--proxy-bypass-list={proxy_bypass}"));
                    }
                }
            }
        }

        if let Some(user_agent) = self.user_agent.as_deref().filter(|value| !value.is_empty()) {
            args.push(format!("--user-agent={user_agent}"));
        }

        args.extend(self.args.iter().cloned());
        let args = normalize_launch_args(args);

        BrowserLaunchPlan {
            executable_path: self.executable_path.clone(),
            args,
            env: self.env.clone(),
        }
    }

    fn default_chrome_args(&self) -> Vec<String> {
        match &self.ignore_default_args {
            IgnoreDefaultArgs::All(true) => Vec::new(),
            IgnoreDefaultArgs::All(false) => CHROME_DEFAULT_ARGS
                .iter()
                .map(|arg| (*arg).to_owned())
                .collect(),
            IgnoreDefaultArgs::List(ignored_args) => CHROME_DEFAULT_ARGS
                .iter()
                .filter(|arg| !ignored_args.iter().any(|ignored| ignored == **arg))
                .map(|arg| (*arg).to_owned())
                .collect(),
        }
    }

    /// Spawns a local Chrome process and waits for its DevTools endpoint.
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
        let plan = launch_profile.try_launch_plan()?;

        let mut command = Command::new(&executable_path);
        command
            .args(&plan.args)
            .envs(&plan.env)
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

fn normalize_launch_args(args: Vec<String>) -> Vec<String> {
    dedupe_launch_args_by_switch(merge_disable_features_args(args))
}

fn merge_disable_features_args(args: Vec<String>) -> Vec<String> {
    let mut feature_values = Vec::new();
    let mut last_disable_features_index = None;

    for (index, arg) in args.iter().enumerate() {
        let Some(value) = disable_features_value(arg) else {
            continue;
        };
        last_disable_features_index = Some(index);
        feature_values.extend(value.split(',').map(str::to_owned));
    }

    let Some(last_disable_features_index) = last_disable_features_index else {
        return args;
    };
    let Some(merged_features) = merged_disable_features_value(&feature_values) else {
        return args
            .into_iter()
            .filter(|arg| disable_features_value(arg).is_none())
            .collect();
    };
    let merged_arg = format!("--disable-features={merged_features}");

    args.into_iter()
        .enumerate()
        .filter_map(|(index, arg)| {
            if disable_features_value(&arg).is_none() {
                return Some(arg);
            }
            (index == last_disable_features_index).then(|| merged_arg.clone())
        })
        .collect()
}

fn disable_features_value(arg: &str) -> Option<&str> {
    arg.strip_prefix("--disable-features=")
}

fn merged_disable_features_value(values: &[String]) -> Option<String> {
    let mut seen = BTreeSet::new();
    let mut unique_features = Vec::new();
    for value in values {
        let feature = value.trim();
        if feature.is_empty() || !seen.insert(feature.to_owned()) {
            continue;
        }
        unique_features.push(feature.to_owned());
    }
    (!unique_features.is_empty()).then(|| unique_features.join(","))
}

fn dedupe_launch_args_by_switch(args: Vec<String>) -> Vec<String> {
    let mut seen = BTreeSet::new();
    let mut deduped = Vec::new();
    for arg in args.into_iter().rev() {
        if seen.insert(launch_arg_key(&arg).to_owned()) {
            deduped.push(arg);
        }
    }
    deduped.reverse();
    deduped
}

fn launch_arg_key(arg: &str) -> &str {
    arg.split_once('=').map_or(arg, |(key, _)| key)
}

/// Resolves a Chrome executable from explicit path, env override, and candidates.
///
/// The returned error includes every checked path so CLI users can see exactly
/// why launch failed.
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
/// Returns executable candidates for the requested channel or default set.
pub fn browser_executable_candidates(channel: Option<BrowserChannel>) -> Vec<PathBuf> {
    match channel {
        Some(channel) => browser_channel_candidates(channel),
        None => default_chrome_candidates(),
    }
}

#[must_use]
/// Returns platform-specific executable candidates for one browser channel.
pub fn browser_channel_candidates(channel: BrowserChannel) -> Vec<PathBuf> {
    let mut candidates = Vec::new();

    #[cfg(target_os = "macos")]
    {
        match channel {
            BrowserChannel::Chromium => candidates.push(PathBuf::from(
                "/Applications/Chromium.app/Contents/MacOS/Chromium",
            )),
            BrowserChannel::Chrome => candidates.push(PathBuf::from(
                "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
            )),
            BrowserChannel::ChromeBeta => candidates.push(PathBuf::from(
                "/Applications/Google Chrome Beta.app/Contents/MacOS/Google Chrome Beta",
            )),
            BrowserChannel::ChromeDev => candidates.push(PathBuf::from(
                "/Applications/Google Chrome Dev.app/Contents/MacOS/Google Chrome Dev",
            )),
            BrowserChannel::ChromeCanary => candidates.push(PathBuf::from(
                "/Applications/Google Chrome Canary.app/Contents/MacOS/Google Chrome Canary",
            )),
            BrowserChannel::MsEdge => candidates.push(PathBuf::from(
                "/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge",
            )),
            BrowserChannel::MsEdgeBeta => candidates.push(PathBuf::from(
                "/Applications/Microsoft Edge Beta.app/Contents/MacOS/Microsoft Edge Beta",
            )),
            BrowserChannel::MsEdgeDev => candidates.push(PathBuf::from(
                "/Applications/Microsoft Edge Dev.app/Contents/MacOS/Microsoft Edge Dev",
            )),
            BrowserChannel::MsEdgeCanary => candidates.push(PathBuf::from(
                "/Applications/Microsoft Edge Canary.app/Contents/MacOS/Microsoft Edge Canary",
            )),
        }
    }

    #[cfg(target_os = "linux")]
    {
        match channel {
            BrowserChannel::Chromium => {
                candidates.push(PathBuf::from("/usr/bin/chromium"));
                candidates.push(PathBuf::from("/usr/bin/chromium-browser"));
            }
            BrowserChannel::Chrome => {
                candidates.push(PathBuf::from("/usr/bin/google-chrome"));
                candidates.push(PathBuf::from("/usr/bin/google-chrome-stable"));
            }
            BrowserChannel::ChromeBeta => {
                candidates.push(PathBuf::from("/usr/bin/google-chrome-beta"))
            }
            BrowserChannel::ChromeDev => {
                candidates.push(PathBuf::from("/usr/bin/google-chrome-unstable"))
            }
            BrowserChannel::ChromeCanary => {
                candidates.push(PathBuf::from("/usr/bin/google-chrome-canary"))
            }
            BrowserChannel::MsEdge => {
                candidates.push(PathBuf::from("/usr/bin/microsoft-edge"));
                candidates.push(PathBuf::from("/usr/bin/microsoft-edge-stable"));
            }
            BrowserChannel::MsEdgeBeta => {
                candidates.push(PathBuf::from("/usr/bin/microsoft-edge-beta"))
            }
            BrowserChannel::MsEdgeDev => {
                candidates.push(PathBuf::from("/usr/bin/microsoft-edge-dev"))
            }
            BrowserChannel::MsEdgeCanary => {
                candidates.push(PathBuf::from("/usr/bin/microsoft-edge-canary"))
            }
        }
    }

    #[cfg(target_os = "windows")]
    {
        let program_files = std::env::var_os("PROGRAMFILES").map(PathBuf::from);
        let program_files_x86 = std::env::var_os("PROGRAMFILES(X86)").map(PathBuf::from);
        let local_app_data = std::env::var_os("LOCALAPPDATA").map(PathBuf::from);
        match channel {
            BrowserChannel::Chromium => {
                if let Some(local_app_data) = &local_app_data {
                    candidates.push(local_app_data.join("Chromium/Application/chrome.exe"));
                }
            }
            BrowserChannel::Chrome => {
                if let Some(program_files) = &program_files {
                    candidates.push(program_files.join("Google/Chrome/Application/chrome.exe"));
                }
                if let Some(program_files_x86) = &program_files_x86 {
                    candidates.push(program_files_x86.join("Google/Chrome/Application/chrome.exe"));
                }
                if let Some(local_app_data) = &local_app_data {
                    candidates.push(local_app_data.join("Google/Chrome/Application/chrome.exe"));
                }
            }
            BrowserChannel::ChromeBeta => {
                if let Some(program_files) = &program_files {
                    candidates
                        .push(program_files.join("Google/Chrome Beta/Application/chrome.exe"));
                }
                if let Some(program_files_x86) = &program_files_x86 {
                    candidates
                        .push(program_files_x86.join("Google/Chrome Beta/Application/chrome.exe"));
                }
            }
            BrowserChannel::ChromeDev => {
                if let Some(program_files) = &program_files {
                    candidates.push(program_files.join("Google/Chrome Dev/Application/chrome.exe"));
                }
                if let Some(program_files_x86) = &program_files_x86 {
                    candidates
                        .push(program_files_x86.join("Google/Chrome Dev/Application/chrome.exe"));
                }
            }
            BrowserChannel::ChromeCanary => {
                if let Some(local_app_data) = &local_app_data {
                    candidates
                        .push(local_app_data.join("Google/Chrome SxS/Application/chrome.exe"));
                }
            }
            BrowserChannel::MsEdge => {
                if let Some(program_files) = &program_files {
                    candidates.push(program_files.join("Microsoft/Edge/Application/msedge.exe"));
                }
                if let Some(program_files_x86) = &program_files_x86 {
                    candidates
                        .push(program_files_x86.join("Microsoft/Edge/Application/msedge.exe"));
                }
            }
            BrowserChannel::MsEdgeBeta => {
                if let Some(program_files) = &program_files {
                    candidates
                        .push(program_files.join("Microsoft/Edge Beta/Application/msedge.exe"));
                }
                if let Some(program_files_x86) = &program_files_x86 {
                    candidates
                        .push(program_files_x86.join("Microsoft/Edge Beta/Application/msedge.exe"));
                }
            }
            BrowserChannel::MsEdgeDev => {
                if let Some(program_files) = &program_files {
                    candidates
                        .push(program_files.join("Microsoft/Edge Dev/Application/msedge.exe"));
                }
                if let Some(program_files_x86) = &program_files_x86 {
                    candidates
                        .push(program_files_x86.join("Microsoft/Edge Dev/Application/msedge.exe"));
                }
            }
            BrowserChannel::MsEdgeCanary => {
                if let Some(local_app_data) = &local_app_data {
                    candidates
                        .push(local_app_data.join("Microsoft/Edge SxS/Application/msedge.exe"));
                }
            }
        }
    }

    candidates
}

#[must_use]
/// Returns platform-specific default Chrome/Chromium executable candidates.
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

/// Fully normalized local Chrome launch command plan.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct BrowserLaunchPlan {
    /// Executable path, if known before launch.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub executable_path: Option<PathBuf>,
    /// Chrome command-line arguments after defaults, overrides, and de-duplication.
    pub args: Vec<String>,
    /// Environment variables supplied to the Chrome process.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub env: BTreeMap<String, String>,
}

/// HTTP and websocket URLs needed to talk to Chrome DevTools.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct DevToolsEndpoint {
    /// HTTP base URL used for discovery endpoints.
    pub http_url: String,
    /// Websocket URL used for the CDP transport.
    pub websocket_url: String,
}

impl DevToolsEndpoint {
    /// Builds an endpoint from a websocket CDP URL.
    pub fn from_cdp_url(cdp_url: &str) -> Result<Self, BrowserError> {
        let parsed = url::Url::parse(cdp_url)
            .map_err(|error| BrowserError::StateUnavailable(error.to_string()))?;
        let websocket_url = match parsed.scheme() {
            "ws" | "wss" => cdp_url.to_owned(),
            scheme => {
                return Err(BrowserError::StateUnavailable(format!(
                    "unsupported CDP URL scheme {scheme:?}; expected ws or wss"
                )));
            }
        };
        let mut http_url = parsed;
        let http_scheme = if http_url.scheme() == "wss" {
            "https"
        } else {
            "http"
        };
        http_url.set_scheme(http_scheme).map_err(|_| {
            BrowserError::StateUnavailable(format!(
                "could not convert CDP URL scheme to {http_scheme}"
            ))
        })?;
        http_url.set_path("");
        http_url.set_query(None);
        http_url.set_fragment(None);
        Ok(Self {
            http_url: http_url.to_string().trim_end_matches('/').to_owned(),
            websocket_url,
        })
    }

    /// Parses Chrome's `DevToolsActivePort` file into a DevTools endpoint.
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

/// Handle for a Chrome process launched by [`BrowserProfile::launch_local`].
pub struct LaunchedBrowser {
    child: Child,
    endpoint: DevToolsEndpoint,
    _user_data_dir: Option<TempDir>,
}

impl LaunchedBrowser {
    /// Returns the DevTools endpoint exposed by the launched browser.
    #[must_use]
    pub fn endpoint(&self) -> &DevToolsEndpoint {
        &self.endpoint
    }

    /// Returns the child process id when available.
    #[must_use]
    pub fn process_id(&self) -> Option<u32> {
        self.child.id()
    }

    /// Prevents `Drop` from killing the browser and returns its endpoint.
    #[must_use]
    pub fn detach(self) -> DevToolsEndpoint {
        let this = ManuallyDrop::new(self);
        this.endpoint.clone()
    }
}

impl Drop for LaunchedBrowser {
    fn drop(&mut self) {
        let _ = self.child.start_kill();
    }
}

#[must_use]
/// Returns the path to Chrome's `DevToolsActivePort` file for a user-data dir.
pub fn devtools_active_port_path(user_data_dir: &Path) -> PathBuf {
    user_data_dir.join("DevToolsActivePort")
}

/// Waits until Chrome writes a readable DevTools endpoint file.
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
