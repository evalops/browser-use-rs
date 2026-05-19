use crate::{
    BrowserError, BrowserViewport, CloudBrowserClient, CloudBrowserCreateRequest, ProxySettings,
    deserialize_env_map, deserialize_non_negative_f64, deserialize_non_negative_f64_option,
};
use schemars::JsonSchema;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::collections::{BTreeMap, BTreeSet};
use std::mem::ManuallyDrop;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, Instant};
use tempfile::TempDir;
use tokio::process::{Child, Command};
use tokio::time::sleep;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum IgnoreDefaultArgs {
    All(bool),
    List(Vec<String>),
}

impl Default for IgnoreDefaultArgs {
    fn default() -> Self {
        Self::List(default_ignore_default_args())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum BrowserChannel {
    #[serde(rename = "chromium")]
    Chromium,
    #[serde(rename = "chrome")]
    Chrome,
    #[serde(rename = "chrome-beta")]
    ChromeBeta,
    #[serde(rename = "chrome-dev")]
    ChromeDev,
    #[serde(rename = "chrome-canary")]
    ChromeCanary,
    #[serde(rename = "msedge")]
    MsEdge,
    #[serde(rename = "msedge-beta")]
    MsEdgeBeta,
    #[serde(rename = "msedge-dev")]
    MsEdgeDev,
    #[serde(rename = "msedge-canary")]
    MsEdgeCanary,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum RecordHarContent {
    Omit,
    #[default]
    Embed,
    Attach,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum RecordHarMode {
    #[default]
    Full,
    Minimal,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct BrowserProfile {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cdp_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub headers: Option<BTreeMap<String, String>>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub use_cloud: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cloud_browser_params: Option<CloudBrowserCreateRequest>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cloud_api_base_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cloud_api_key: Option<String>,
    #[serde(default, deserialize_with = "deserialize_env_map")]
    pub env: BTreeMap<String, String>,
    #[serde(
        default,
        alias = "browser_binary_path",
        alias = "chrome_binary_path",
        skip_serializing_if = "Option::is_none"
    )]
    pub executable_path: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channel: Option<BrowserChannel>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote_debugging_port: Option<u16>,
    #[serde(default = "default_headless")]
    pub headless: bool,
    #[serde(default)]
    pub devtools: bool,
    #[serde(default = "default_chromium_sandbox")]
    pub chromium_sandbox: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_data_dir: Option<PathBuf>,
    #[serde(default = "default_profile_directory")]
    pub profile_directory: String,
    #[serde(
        default,
        alias = "downloads_dir",
        alias = "save_downloads_path",
        skip_serializing_if = "Option::is_none"
    )]
    pub downloads_path: Option<PathBuf>,
    #[serde(default = "default_accept_downloads")]
    pub accept_downloads: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub storage_state_path: Option<PathBuf>,
    #[serde(default = "default_auto_download_pdfs")]
    pub auto_download_pdfs: bool,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub ignore_default_args: IgnoreDefaultArgs,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_agent: Option<String>,
    #[serde(default = "default_browser_permissions")]
    pub permissions: Vec<String>,
    #[serde(default)]
    pub allowed_domains: Vec<String>,
    #[serde(default)]
    pub prohibited_domains: Vec<String>,
    #[serde(default)]
    pub block_ip_addresses: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keep_alive: Option<bool>,
    #[serde(default)]
    pub disable_security: bool,
    #[serde(default)]
    pub deterministic_rendering: bool,
    #[serde(default = "default_cross_origin_iframes")]
    pub cross_origin_iframes: bool,
    #[serde(default = "default_max_iframes")]
    pub max_iframes: usize,
    #[serde(default = "default_max_iframe_depth")]
    pub max_iframe_depth: usize,
    #[serde(default = "default_paint_order_filtering")]
    pub paint_order_filtering: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub screen: Option<BrowserViewport>,
    #[serde(default)]
    pub viewport: BrowserViewport,
    #[serde(default, skip_serializing_if = "is_false")]
    pub no_viewport: bool,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "deserialize_non_negative_f64_option"
    )]
    pub device_scale_factor: Option<f64>,
    #[serde(default)]
    pub record_har_content: RecordHarContent,
    #[serde(default)]
    pub record_har_mode: RecordHarMode,
    #[serde(
        default,
        alias = "save_har_path",
        skip_serializing_if = "Option::is_none"
    )]
    pub record_har_path: Option<PathBuf>,
    #[serde(
        default,
        alias = "save_recording_path",
        skip_serializing_if = "Option::is_none"
    )]
    pub record_video_dir: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub record_video_size: Option<BrowserViewport>,
    #[serde(default = "default_record_video_framerate")]
    pub record_video_framerate: u32,
    #[serde(default, skip_serializing_if = "is_default_video_recording_format")]
    pub record_video_format: VideoRecordingFormat,
    #[serde(default, alias = "trace_path", skip_serializing_if = "Option::is_none")]
    pub traces_dir: Option<PathBuf>,
    #[serde(
        default = "default_minimum_wait_page_load_time",
        deserialize_with = "deserialize_non_negative_f64"
    )]
    pub minimum_wait_page_load_time: f64,
    #[serde(
        default = "default_wait_for_network_idle_page_load_time",
        deserialize_with = "deserialize_non_negative_f64"
    )]
    pub wait_for_network_idle_page_load_time: f64,
    #[serde(default = "default_highlight_elements")]
    pub highlight_elements: bool,
    #[serde(default)]
    pub dom_highlight_elements: bool,
    #[serde(default = "default_filter_highlight_ids")]
    pub filter_highlight_ids: bool,
    #[serde(default = "default_interaction_highlight_color")]
    pub interaction_highlight_color: String,
    #[serde(default = "default_interaction_highlight_duration")]
    pub interaction_highlight_duration: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window_size: Option<BrowserViewport>,
    #[serde(
        default = "default_window_position",
        skip_serializing_if = "Option::is_none"
    )]
    pub window_position: Option<BrowserViewport>,
    #[serde(default = "default_browser_start_timeout_ms")]
    pub browser_start_timeout_ms: u64,
    #[serde(default = "default_navigation_timeout_ms")]
    pub navigation_timeout_ms: u64,
    #[serde(default = "default_network_request_timeout_ms")]
    pub network_request_timeout_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proxy: Option<ProxySettings>,
}

impl Default for BrowserProfile {
    fn default() -> Self {
        Self {
            cdp_url: None,
            headers: None,
            use_cloud: false,
            cloud_browser_params: None,
            cloud_api_base_url: None,
            cloud_api_key: None,
            env: BTreeMap::new(),
            executable_path: None,
            channel: None,
            remote_debugging_port: None,
            headless: default_headless(),
            devtools: false,
            chromium_sandbox: default_chromium_sandbox(),
            user_data_dir: None,
            profile_directory: default_profile_directory(),
            downloads_path: None,
            accept_downloads: default_accept_downloads(),
            storage_state_path: None,
            auto_download_pdfs: default_auto_download_pdfs(),
            args: Vec::new(),
            ignore_default_args: IgnoreDefaultArgs::default(),
            user_agent: None,
            permissions: default_browser_permissions(),
            allowed_domains: Vec::new(),
            prohibited_domains: Vec::new(),
            block_ip_addresses: false,
            keep_alive: None,
            disable_security: false,
            deterministic_rendering: false,
            cross_origin_iframes: default_cross_origin_iframes(),
            max_iframes: default_max_iframes(),
            max_iframe_depth: default_max_iframe_depth(),
            paint_order_filtering: default_paint_order_filtering(),
            screen: None,
            viewport: BrowserViewport::default(),
            no_viewport: false,
            device_scale_factor: None,
            record_har_content: RecordHarContent::default(),
            record_har_mode: RecordHarMode::default(),
            record_har_path: None,
            record_video_dir: None,
            record_video_size: None,
            record_video_framerate: default_record_video_framerate(),
            record_video_format: VideoRecordingFormat::default(),
            traces_dir: None,
            minimum_wait_page_load_time: default_minimum_wait_page_load_time(),
            wait_for_network_idle_page_load_time: default_wait_for_network_idle_page_load_time(),
            highlight_elements: default_highlight_elements(),
            dom_highlight_elements: false,
            filter_highlight_ids: default_filter_highlight_ids(),
            interaction_highlight_color: default_interaction_highlight_color(),
            interaction_highlight_duration: default_interaction_highlight_duration(),
            window_size: None,
            window_position: default_window_position(),
            browser_start_timeout_ms: default_browser_start_timeout_ms(),
            navigation_timeout_ms: default_navigation_timeout_ms(),
            network_request_timeout_ms: default_network_request_timeout_ms(),
            proxy: None,
        }
    }
}

pub(crate) fn is_false(value: &bool) -> bool {
    !*value
}

fn default_headless() -> bool {
    true
}

fn default_chromium_sandbox() -> bool {
    true
}

fn default_record_video_framerate() -> u32 {
    30
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, JsonSchema)]
pub enum VideoRecordingFormat {
    #[default]
    Mp4,
    Webm,
    Gif,
}

impl VideoRecordingFormat {
    fn parse(value: &str) -> Result<Self, String> {
        match value
            .trim()
            .trim_start_matches('.')
            .to_ascii_lowercase()
            .as_str()
        {
            "mp4" => Ok(Self::Mp4),
            "webm" => Ok(Self::Webm),
            "gif" => Ok(Self::Gif),
            other => Err(format!(
                "record_video_format must be mp4, webm, or gif; got {other:?}"
            )),
        }
    }

    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Mp4 => "mp4",
            Self::Webm => "webm",
            Self::Gif => "gif",
        }
    }
}

impl Serialize for VideoRecordingFormat {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for VideoRecordingFormat {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::parse(&value).map_err(serde::de::Error::custom)
    }
}

fn is_default_video_recording_format(value: &VideoRecordingFormat) -> bool {
    *value == VideoRecordingFormat::default()
}

fn default_profile_directory() -> String {
    "Default".to_owned()
}

fn default_browser_permissions() -> Vec<String> {
    ["clipboardReadWrite", "notifications"]
        .into_iter()
        .map(ToOwned::to_owned)
        .collect()
}

fn default_auto_download_pdfs() -> bool {
    true
}

fn default_accept_downloads() -> bool {
    true
}

fn default_highlight_elements() -> bool {
    true
}

fn default_filter_highlight_ids() -> bool {
    true
}

fn default_interaction_highlight_color() -> String {
    "rgb(255, 127, 39)".to_owned()
}

fn default_interaction_highlight_duration() -> f64 {
    1.0
}

fn default_window_position() -> Option<BrowserViewport> {
    Some(BrowserViewport {
        width: 0,
        height: 0,
    })
}

fn default_cross_origin_iframes() -> bool {
    true
}

fn default_max_iframes() -> usize {
    100
}

fn default_max_iframe_depth() -> usize {
    5
}

pub(crate) fn default_paint_order_filtering() -> bool {
    true
}

pub(crate) fn default_ignore_default_args() -> Vec<String> {
    DEFAULT_IGNORE_DEFAULT_ARGS
        .iter()
        .map(|arg| (*arg).to_owned())
        .collect()
}

fn default_browser_start_timeout_ms() -> u64 {
    30_000
}

pub(crate) fn default_navigation_timeout_ms() -> u64 {
    20_000
}

fn default_network_request_timeout_ms() -> u64 {
    10_000
}

fn default_minimum_wait_page_load_time() -> f64 {
    0.25
}

fn default_wait_for_network_idle_page_load_time() -> f64 {
    0.5
}

pub(crate) fn profile_keeps_launched_browser_alive(profile: &BrowserProfile) -> bool {
    profile.keep_alive == Some(true)
}

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

impl BrowserProfile {
    #[must_use]
    pub fn uses_cloud(&self) -> bool {
        self.use_cloud || self.cloud_browser_params.is_some()
    }

    #[must_use]
    pub fn cloud_create_request(&self) -> Option<CloudBrowserCreateRequest> {
        self.uses_cloud()
            .then(|| self.cloud_browser_params.clone().unwrap_or_default())
    }

    #[must_use]
    pub fn cloud_browser_request(&self) -> CloudBrowserCreateRequest {
        self.cloud_browser_params.clone().unwrap_or_default()
    }

    pub async fn create_cloud_endpoint(&self) -> Result<Option<DevToolsEndpoint>, BrowserError> {
        let client = self.cloud_browser_client();
        self.create_cloud_endpoint_with_client(&client).await
    }

    pub async fn create_cloud_endpoint_with_client(
        &self,
        client: &CloudBrowserClient,
    ) -> Result<Option<DevToolsEndpoint>, BrowserError> {
        let Some(request) = self.cloud_create_request() else {
            return Ok(None);
        };
        client
            .create_browser(&request)
            .await?
            .devtools_endpoint()
            .map(Some)
    }

    pub async fn create_cloud_devtools_endpoint(&self) -> Result<DevToolsEndpoint, BrowserError> {
        self.create_cloud_endpoint()
            .await?
            .ok_or_else(|| BrowserError::Cloud("cloud browser is not enabled".to_owned()))
    }

    fn cloud_browser_client(&self) -> CloudBrowserClient {
        let mut client = match &self.cloud_api_key {
            Some(api_key) => CloudBrowserClient::with_api_key(api_key.clone()),
            None => CloudBrowserClient::new(),
        };
        if let Some(api_base_url) = &self.cloud_api_base_url {
            client = client.with_base_url(api_base_url.clone());
        }
        client
    }

    pub fn resolve_executable(&self) -> Result<PathBuf, BrowserError> {
        resolve_chrome_executable(
            self.executable_path.as_deref(),
            std::env::var_os("BROWSER_USE_CHROME").map(PathBuf::from),
            browser_executable_candidates(self.channel),
        )
    }

    #[must_use]
    pub fn launch_plan(&self) -> BrowserLaunchPlan {
        self.try_launch_plan()
            .expect("invalid BrowserProfile launch plan")
    }

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
pub fn browser_executable_candidates(channel: Option<BrowserChannel>) -> Vec<PathBuf> {
    match channel {
        Some(channel) => browser_channel_candidates(channel),
        None => default_chrome_candidates(),
    }
}

#[must_use]
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
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub env: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct DevToolsEndpoint {
    pub http_url: String,
    pub websocket_url: String,
}

impl DevToolsEndpoint {
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
