//! Browser launch and connection profile handling.
//!
//! A [`BrowserProfile`] is the main configuration object for CDP sessions. It
//! can describe an already-running CDP endpoint, a local Chrome launch, or a
//! Browser Use Cloud browser. The rest of this module turns that profile into
//! concrete Chrome arguments, executable paths, storage-state actions, and
//! DevTools endpoints.

use crate::{
    BrowserError, BrowserViewport, CloudBrowserClient, CloudBrowserCreateRequest, ProxySettings,
    deserialize_env_map, deserialize_non_negative_f64, deserialize_non_negative_f64_option,
};
use schemars::JsonSchema;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::collections::BTreeMap;
use std::path::PathBuf;

mod launch;

pub(crate) use launch::default_ignore_default_args;
pub use launch::{
    BrowserLaunchPlan, DevToolsEndpoint, LaunchedBrowser, browser_channel_candidates,
    browser_executable_candidates, default_chrome_candidates, devtools_active_port_path,
    resolve_chrome_executable, wait_for_devtools_endpoint,
};
#[cfg(test)]
pub(crate) use launch::{
    CHROME_DETERMINISTIC_RENDERING_ARGS, CHROME_DISABLE_SECURITY_ARGS, CHROME_DOCKER_ARGS,
};

/// Controls whether browser-use default Chrome arguments are ignored.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum IgnoreDefaultArgs {
    /// Ignore all defaults when true, or keep all defaults when false.
    All(bool),
    /// Ignore only the listed default arguments.
    List(Vec<String>),
}

impl Default for IgnoreDefaultArgs {
    fn default() -> Self {
        Self::List(default_ignore_default_args())
    }
}

/// Browser channel used when resolving an executable path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum BrowserChannel {
    /// Open-source Chromium.
    #[serde(rename = "chromium")]
    Chromium,
    /// Stable Google Chrome.
    #[serde(rename = "chrome")]
    Chrome,
    /// Google Chrome Beta.
    #[serde(rename = "chrome-beta")]
    ChromeBeta,
    /// Google Chrome Dev.
    #[serde(rename = "chrome-dev")]
    ChromeDev,
    /// Google Chrome Canary.
    #[serde(rename = "chrome-canary")]
    ChromeCanary,
    /// Stable Microsoft Edge.
    #[serde(rename = "msedge")]
    MsEdge,
    /// Microsoft Edge Beta.
    #[serde(rename = "msedge-beta")]
    MsEdgeBeta,
    /// Microsoft Edge Dev.
    #[serde(rename = "msedge-dev")]
    MsEdgeDev,
    /// Microsoft Edge Canary.
    #[serde(rename = "msedge-canary")]
    MsEdgeCanary,
}

/// How much response body data should be included in HAR recording.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum RecordHarContent {
    /// Omit response bodies.
    Omit,
    /// Embed response bodies in the HAR file.
    #[default]
    Embed,
    /// Store response bodies as separate attachments when supported.
    Attach,
}

/// HAR recording detail level.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum RecordHarMode {
    /// Capture full HAR entries.
    #[default]
    Full,
    /// Capture a smaller HAR payload.
    Minimal,
}

/// Configuration for connecting to, launching, or creating a browser.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct BrowserProfile {
    /// Existing CDP websocket URL to connect to instead of launching Chrome.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cdp_url: Option<String>,
    /// Extra headers for CDP websocket connections.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub headers: Option<BTreeMap<String, String>>,
    /// Whether to create a Browser Use Cloud session.
    #[serde(default, skip_serializing_if = "is_false")]
    pub use_cloud: bool,
    /// Browser Use Cloud create request parameters.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cloud_browser_params: Option<CloudBrowserCreateRequest>,
    /// Optional Browser Use Cloud API base URL override.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cloud_api_base_url: Option<String>,
    /// Optional Browser Use Cloud API key override.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cloud_api_key: Option<String>,
    /// Environment variables passed to a locally launched Chrome process.
    #[serde(default, deserialize_with = "deserialize_env_map")]
    pub env: BTreeMap<String, String>,
    /// Explicit Chrome executable path.
    #[serde(
        default,
        alias = "browser_binary_path",
        alias = "chrome_binary_path",
        skip_serializing_if = "Option::is_none"
    )]
    pub executable_path: Option<PathBuf>,
    /// Browser channel used to find an executable when no explicit path is set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channel: Option<BrowserChannel>,
    /// Fixed remote debugging port; `None` lets Chrome choose.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote_debugging_port: Option<u16>,
    /// Launch Chrome in headless mode.
    #[serde(default = "default_headless")]
    pub headless: bool,
    /// Auto-open DevTools for tabs.
    #[serde(default)]
    pub devtools: bool,
    /// Keep Chromium sandboxing enabled.
    #[serde(default = "default_chromium_sandbox")]
    pub chromium_sandbox: bool,
    /// User data directory for the browser profile.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_data_dir: Option<PathBuf>,
    /// Chrome profile directory inside `user_data_dir`.
    #[serde(default = "default_profile_directory")]
    pub profile_directory: String,
    /// Directory for accepted downloads.
    #[serde(
        default,
        alias = "downloads_dir",
        alias = "save_downloads_path",
        skip_serializing_if = "Option::is_none"
    )]
    pub downloads_path: Option<PathBuf>,
    /// Whether downloads are accepted and routed to a managed directory.
    #[serde(default = "default_accept_downloads")]
    pub accept_downloads: bool,
    /// Path used to load/save browser storage state.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub storage_state_path: Option<PathBuf>,
    /// Whether PDF viewer URLs should be auto-downloaded when possible.
    #[serde(default = "default_auto_download_pdfs")]
    pub auto_download_pdfs: bool,
    /// Additional raw Chrome command-line arguments.
    #[serde(default)]
    pub args: Vec<String>,
    /// Default Chrome arguments to suppress.
    #[serde(default)]
    pub ignore_default_args: IgnoreDefaultArgs,
    /// Optional user-agent override.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_agent: Option<String>,
    /// Browser permissions to grant via CDP.
    #[serde(default = "default_browser_permissions")]
    pub permissions: Vec<String>,
    /// URL allow-list enforced by navigation policy.
    #[serde(default)]
    pub allowed_domains: Vec<String>,
    /// URL deny-list enforced by navigation policy.
    #[serde(default)]
    pub prohibited_domains: Vec<String>,
    /// Blocks navigation to literal IP addresses when true.
    #[serde(default)]
    pub block_ip_addresses: bool,
    /// Keeps a launched browser alive after session detach when set to `Some(true)`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keep_alive: Option<bool>,
    /// Adds Chrome flags that relax web security checks.
    #[serde(default)]
    pub disable_security: bool,
    /// Adds deterministic rendering flags for screenshot/video reproducibility.
    #[serde(default)]
    pub deterministic_rendering: bool,
    /// Traverses cross-origin iframes when collecting DOM state.
    #[serde(default = "default_cross_origin_iframes")]
    pub cross_origin_iframes: bool,
    /// Maximum number of iframes to inspect.
    #[serde(default = "default_max_iframes")]
    pub max_iframes: usize,
    /// Maximum iframe nesting depth to inspect.
    #[serde(default = "default_max_iframe_depth")]
    pub max_iframe_depth: usize,
    /// Filters DOM elements hidden behind later paint-order elements.
    #[serde(default = "default_paint_order_filtering")]
    pub paint_order_filtering: bool,
    /// Optional screen size passed to browser contexts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub screen: Option<BrowserViewport>,
    /// Viewport size used for local launch and CDP emulation.
    #[serde(default)]
    pub viewport: BrowserViewport,
    /// Disables viewport emulation.
    #[serde(default, skip_serializing_if = "is_false")]
    pub no_viewport: bool,
    /// Device scale factor for viewport emulation.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "deserialize_non_negative_f64_option"
    )]
    pub device_scale_factor: Option<f64>,
    /// HAR content capture policy.
    #[serde(default)]
    pub record_har_content: RecordHarContent,
    /// HAR detail mode.
    #[serde(default)]
    pub record_har_mode: RecordHarMode,
    /// Path where HAR output should be written.
    #[serde(
        default,
        alias = "save_har_path",
        skip_serializing_if = "Option::is_none"
    )]
    pub record_har_path: Option<PathBuf>,
    /// Directory where video output should be written.
    #[serde(
        default,
        alias = "save_recording_path",
        skip_serializing_if = "Option::is_none"
    )]
    pub record_video_dir: Option<PathBuf>,
    /// Optional video dimensions.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub record_video_size: Option<BrowserViewport>,
    /// Video capture framerate.
    #[serde(default = "default_record_video_framerate")]
    pub record_video_framerate: u32,
    /// Video artifact format.
    #[serde(default, skip_serializing_if = "is_default_video_recording_format")]
    pub record_video_format: VideoRecordingFormat,
    /// Directory where trace artifacts should be written.
    #[serde(default, alias = "trace_path", skip_serializing_if = "Option::is_none")]
    pub traces_dir: Option<PathBuf>,
    /// Minimum seconds to wait after page load before state capture continues.
    #[serde(
        default = "default_minimum_wait_page_load_time",
        deserialize_with = "deserialize_non_negative_f64"
    )]
    pub minimum_wait_page_load_time: f64,
    /// Seconds to wait for network idle after the minimum load wait.
    #[serde(
        default = "default_wait_for_network_idle_page_load_time",
        deserialize_with = "deserialize_non_negative_f64"
    )]
    pub wait_for_network_idle_page_load_time: f64,
    /// Draws a highlight overlay around interacted elements.
    #[serde(default = "default_highlight_elements")]
    pub highlight_elements: bool,
    /// Draws overlays for DOM-highlight capture paths.
    #[serde(default)]
    pub dom_highlight_elements: bool,
    /// Removes internal highlight ids from prompt-facing DOM output.
    #[serde(default = "default_filter_highlight_ids")]
    pub filter_highlight_ids: bool,
    /// CSS color used for interaction highlights.
    #[serde(default = "default_interaction_highlight_color")]
    pub interaction_highlight_color: String,
    /// Seconds that interaction highlights remain visible.
    #[serde(default = "default_interaction_highlight_duration")]
    pub interaction_highlight_duration: f64,
    /// Native browser window size.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window_size: Option<BrowserViewport>,
    /// Native browser window position, represented as x/y in a viewport-shaped pair.
    #[serde(
        default = "default_window_position",
        skip_serializing_if = "Option::is_none"
    )]
    pub window_position: Option<BrowserViewport>,
    /// Milliseconds to wait for a launched browser to expose DevTools.
    #[serde(default = "default_browser_start_timeout_ms")]
    pub browser_start_timeout_ms: u64,
    /// Milliseconds to wait for navigation CDP operations.
    #[serde(default = "default_navigation_timeout_ms")]
    pub navigation_timeout_ms: u64,
    /// Milliseconds before a request is considered timed out for lifecycle events.
    #[serde(default = "default_network_request_timeout_ms")]
    pub network_request_timeout_ms: u64,
    /// Optional proxy settings translated into Chrome launch arguments.
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

/// Video artifact format produced by the CDP recorder.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, JsonSchema)]
pub enum VideoRecordingFormat {
    /// MPEG-4 output.
    #[default]
    Mp4,
    /// WebM output.
    Webm,
    /// Animated GIF output.
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

impl BrowserProfile {
    /// Returns true when this profile should create a cloud browser.
    #[must_use]
    pub fn uses_cloud(&self) -> bool {
        self.use_cloud || self.cloud_browser_params.is_some()
    }

    /// Returns the cloud create request when cloud mode is enabled.
    #[must_use]
    pub fn cloud_create_request(&self) -> Option<CloudBrowserCreateRequest> {
        self.uses_cloud()
            .then(|| self.cloud_browser_params.clone().unwrap_or_default())
    }

    /// Returns the configured cloud request, or a default request.
    #[must_use]
    pub fn cloud_browser_request(&self) -> CloudBrowserCreateRequest {
        self.cloud_browser_params.clone().unwrap_or_default()
    }

    /// Creates a cloud endpoint if this profile uses Browser Use Cloud.
    pub async fn create_cloud_endpoint(&self) -> Result<Option<DevToolsEndpoint>, BrowserError> {
        let client = self.cloud_browser_client();
        self.create_cloud_endpoint_with_client(&client).await
    }

    /// Creates a cloud endpoint using an injected client.
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

    /// Creates a cloud endpoint and errors if cloud mode is not enabled.
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
}
