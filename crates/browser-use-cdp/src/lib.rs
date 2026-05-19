//! Chrome DevTools Protocol browser-session layer.

#[cfg(test)]
use std::collections::HashMap;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::path::{Path, PathBuf};
#[cfg(test)]
use std::process::Stdio;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use async_trait::async_trait;
#[cfg(test)]
use base64::Engine;
use browser_use_dom::{
    BrowserStateSummary, DomElementRef, ElementBounds, PageInfo, SerializedDomState, TabInfo,
};
#[cfg(test)]
use browser_use_dom::{DomEvalNode, DomPageStats, PaginationButtonType};
#[cfg(test)]
use futures_util::{SinkExt, StreamExt};
use schemars::JsonSchema;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::{Value, json};
#[cfg(test)]
use std::sync::atomic::{AtomicBool, AtomicU64};
use tempfile::TempDir;
use thiserror::Error;
#[cfg(test)]
use tokio::sync::mpsc;
use tokio::sync::{Mutex, broadcast};
use tokio::time::sleep;
#[cfg(test)]
use tokio_tungstenite::tungstenite::Message;

mod cloud;
mod dom;
mod input;
mod lifecycle;
mod policy;
mod profile;
mod recording;
mod runtime;
mod storage;
mod transport;
mod watchdog;

pub(crate) use cloud::download_http_client;
pub use cloud::{
    CloudBrowserClient, CloudBrowserCreateRequest, CloudBrowserResponse, CreateCloudBrowserRequest,
};
#[cfg(test)]
pub(crate) use cloud::{cloud_auth_config_path, load_cloud_auth_api_token, resolve_cloud_api_key};
#[cfg(test)]
pub(crate) use dom::{AX_REF_ATTRIBUTE, INTERACTIVE_ELEMENTS_JS, dom_element_from_value};
pub(crate) use dom::{
    AccessibilityNodeInfo, CLEANUP_AX_REFS_JS, CLICK_ELEMENT_ACTION_JS, DROPDOWN_OPTIONS_BODY_JS,
    FRAME_ELEMENTS_JS, PAGE_INFO_JS, accessibility_nodes_by_backend_id, click_element_js,
    detect_pagination_buttons, dom_highlight_overlay_elements, dom_highlight_overlay_script,
    dom_state_from_interactive_value, dropdown_options_js, element_action_function_js,
    element_action_js, element_eval_js, element_function_js, frame_element_infos_from_value,
    iframe_target_infos_from_targets, index_fallback_target_id,
    interaction_coordinate_highlight_script, interaction_element_highlight_script,
    interactive_elements_js, is_missing_target_error, merge_dom_states, offset_dom_state_bounds,
    page_info_from_value, parse_dropdown_options_value, scroll_to_text_js,
    select_dropdown_option_body_js, select_dropdown_option_js, should_fallback_to_index_traversal,
    snapshot_backend_ids_by_ax_ref, target_local_index_for_global_index, u32_field,
};
pub(crate) use input::{is_special_key, key_event_params, modifier_mask, normalize_send_keys};
pub use lifecycle::{
    BrowserLifecycleAdapterEvent, BrowserLifecycleAdapterEventKind,
    BrowserLifecycleAdapterEventSubscription, BrowserLifecycleEvent, BrowserLifecycleEventKind,
    BrowserLifecycleEventStreamError, BrowserLifecycleEventSubscription,
    browser_lifecycle_adapter_events,
};
pub(crate) use policy::UrlAccessPolicy;
#[cfg(test)]
pub(crate) use policy::is_ip_address;
#[cfg(test)]
pub(crate) use profile::default_ignore_default_args;
pub use profile::{
    BrowserChannel, BrowserLaunchPlan, BrowserProfile, DevToolsEndpoint, IgnoreDefaultArgs,
    LaunchedBrowser, RecordHarContent, RecordHarMode, VideoRecordingFormat,
    browser_channel_candidates, browser_executable_candidates, default_chrome_candidates,
    devtools_active_port_path, resolve_chrome_executable, wait_for_devtools_endpoint,
};
#[cfg(test)]
pub(crate) use profile::{
    CHROME_DETERMINISTIC_RENDERING_ARGS, CHROME_DISABLE_SECURITY_ARGS, CHROME_DOCKER_ARGS,
};
#[cfg(test)]
pub(crate) use profile::{default_navigation_timeout_ms, default_paint_order_filtering};
pub(crate) use profile::{is_false, profile_keeps_launched_browser_alive};
#[cfg(test)]
pub(crate) use recording::CdpVideoState;
pub(crate) use recording::{
    CdpHarRecorder, CdpTraceRecorder, CdpVideoRecorder, TRACE_ARTIFACT_KIND,
    TRACE_ARTIFACT_SCHEMA_VERSION, trace_epoch_millis, trace_recording_failed_event,
    trace_security_event_json, trace_timestamp, video_recording_failed_event,
};
pub(crate) use runtime::{
    render_runtime_evaluate_result, runtime_command_value, runtime_evaluate_params,
    runtime_evaluate_value,
};
pub(crate) use storage::{
    apply_origin_storage_state, browser_storage_state, load_browser_storage_state, page_tabs,
    storage_state_counts, write_storage_state,
};
#[cfg(test)]
pub(crate) use storage::{
    dom_storage_entries_to_items, frame_security_origins_from_result, origin_storage_apply_script,
    upsert_origin_storage_state,
};
pub use transport::CdpConnection;

pub(crate) use watchdog::{
    BrowserLifecycleWatchdog, BrowserLifecycleWatchdogRecorders, BrowserSecurityEvent,
    BrowserSecurityWatchdog, CdpAutoPdfDownloadState, LifecycleEventSink, cdp_request_key,
    is_pdf_viewer_url, pdf_download_filename_from_url, push_lifecycle_event_and_publish,
    push_security_event, security_event_state_fields, unique_download_path,
};
#[cfg(test)]
pub(crate) use watchdog::{
    MAX_LIFECYCLE_EVENTS, MAX_SECURITY_EVENTS, UrlPolicyWatchdogAction,
    cdp_auto_pdf_candidate_from_response, cdp_auto_pdf_lifecycle_event, is_path_contained,
    lifecycle_event_for_download_start, lifecycle_event_for_websocket_closed,
    lifecycle_event_for_websocket_reconnect_failed, lifecycle_event_for_websocket_reconnected,
    lifecycle_event_for_websocket_reconnecting, lifecycle_events_for_download_progress,
    lifecycle_events_for_target_crash, lifecycle_events_for_timed_out_network_requests,
    push_lifecycle_event, sanitize_download_filename, track_network_request,
    url_policy_watchdog_action_for_event,
};

pub(crate) use transport::CdpEvent;
#[cfg(test)]
pub(crate) use transport::{
    cdp_reconnect_delay_for_attempt, cdp_websocket_reconnect_failed_event,
    cdp_websocket_reconnected_event, cdp_websocket_reconnecting_event, cdp_websocket_request,
    should_reconnect_after_websocket_event,
};

const URL_POLICY_SETTLE_MS: u64 = 200;
const CLOUD_HTTP_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug, Clone, Error)]
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
    #[error("navigation blocked by browser profile policy: {url} ({reason})")]
    NavigationBlocked { url: String, reason: String },
    #[error("action failed: {0}")]
    ActionFailed(String),
    #[error("browser state unavailable: {0}")]
    StateUnavailable(String),
    #[error("Browser Use Cloud authentication failed: {0}")]
    CloudAuth(String),
    #[error("Browser Use Cloud request failed: {0}")]
    Cloud(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Screenshot {
    pub base64_png: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pdf {
    pub base64_pdf: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct FoundElement {
    pub tag_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(default)]
    pub attributes: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
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
    pub bypass: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub password: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum CloudProxyCountryCode {
    #[default]
    Unset,
    Disabled,
    Country(String),
}

impl CloudProxyCountryCode {
    #[must_use]
    pub fn disabled() -> Self {
        Self::Disabled
    }

    #[must_use]
    pub fn country(country_code: impl Into<String>) -> Self {
        Self::Country(country_code.into())
    }

    fn is_unset(&self) -> bool {
        matches!(self, Self::Unset)
    }
}

impl JsonSchema for CloudProxyCountryCode {
    fn schema_name() -> String {
        "CloudProxyCountryCode".to_owned()
    }

    fn json_schema(_gen: &mut schemars::r#gen::SchemaGenerator) -> schemars::schema::Schema {
        serde_json::from_value(serde_json::json!({
            "oneOf": [
                { "type": "string" },
                { "type": "null" }
            ]
        }))
        .expect("valid CloudProxyCountryCode JSON schema")
    }
}

fn serialize_cloud_proxy_country_code<S>(
    value: &CloudProxyCountryCode,
    serializer: S,
) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    match value {
        CloudProxyCountryCode::Unset => serializer.serialize_none(),
        CloudProxyCountryCode::Disabled => serializer.serialize_none(),
        CloudProxyCountryCode::Country(country_code) => serializer.serialize_str(country_code),
    }
}

fn deserialize_cloud_proxy_country_code<'de, D>(
    deserializer: D,
) -> Result<CloudProxyCountryCode, D::Error>
where
    D: Deserializer<'de>,
{
    Ok(match Option::<String>::deserialize(deserializer)? {
        Some(country_code) => CloudProxyCountryCode::Country(country_code),
        None => CloudProxyCountryCode::Disabled,
    })
}

fn deserialize_env_map<'de, D>(deserializer: D) -> Result<BTreeMap<String, String>, D::Error>
where
    D: Deserializer<'de>,
{
    let Some(values) = Option::<BTreeMap<String, Value>>::deserialize(deserializer)? else {
        return Ok(BTreeMap::new());
    };
    values
        .into_iter()
        .map(|(key, value)| env_value_to_string(value).map(|value| (key, value)))
        .collect()
}

fn env_value_to_string<E>(value: Value) -> Result<String, E>
where
    E: serde::de::Error,
{
    match value {
        Value::String(value) => Ok(value),
        Value::Bool(value) => Ok(value.to_string()),
        Value::Number(value) => Ok(value.to_string()),
        other => Err(E::custom(format!(
            "browser env values must be strings, numbers, or booleans; got {other}"
        ))),
    }
}

fn deserialize_non_negative_f64_option<'de, D>(deserializer: D) -> Result<Option<f64>, D::Error>
where
    D: Deserializer<'de>,
{
    let value = Option::<f64>::deserialize(deserializer)?;
    match value {
        Some(value) if value.is_finite() && value >= 0.0 => Ok(Some(value)),
        Some(value) => Err(serde::de::Error::custom(format!(
            "device_scale_factor must be a finite non-negative number; got {value}"
        ))),
        None => Ok(None),
    }
}

fn deserialize_non_negative_f64<'de, D>(deserializer: D) -> Result<f64, D::Error>
where
    D: Deserializer<'de>,
{
    let value = f64::deserialize(deserializer)?;
    if value.is_finite() && value >= 0.0 {
        Ok(value)
    } else {
        Err(serde::de::Error::custom(format!(
            "page-load wait seconds must be a finite non-negative number; got {value}"
        )))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttachedPage {
    pub target_id: String,
    pub session_id: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct IframeTraversalConfig {
    cross_origin_iframes: bool,
    max_iframes: usize,
    max_iframe_depth: usize,
}

impl IframeTraversalConfig {
    fn from_profile(profile: &BrowserProfile) -> Self {
        Self {
            cross_origin_iframes: profile.cross_origin_iframes,
            max_iframes: profile.max_iframes,
            max_iframe_depth: profile.max_iframe_depth,
        }
    }

    fn max_iframe_depth_for_same_origin(self) -> usize {
        self.max_iframe_depth
    }

    fn remaining_same_origin_depth(self, current_depth: usize) -> usize {
        self.max_iframe_depth.saturating_sub(current_depth)
    }

    fn allows_cross_origin_depth(self, depth: usize) -> bool {
        self.cross_origin_iframes && depth <= self.max_iframe_depth && self.max_iframes > 0
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct ViewportEmulationConfig {
    viewport: Option<BrowserViewport>,
    device_scale_factor: f64,
}

impl ViewportEmulationConfig {
    fn from_profile(profile: &BrowserProfile) -> Self {
        let viewport = (!profile.no_viewport).then_some(profile.viewport);
        Self {
            viewport,
            device_scale_factor: profile.device_scale_factor.unwrap_or(1.0),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PageLoadWaitConfig {
    minimum_wait: Duration,
    network_idle_wait: Duration,
}

impl PageLoadWaitConfig {
    fn from_profile(profile: &BrowserProfile) -> Self {
        Self {
            minimum_wait: Duration::from_secs_f64(profile.minimum_wait_page_load_time),
            network_idle_wait: Duration::from_secs_f64(
                profile.wait_for_network_idle_page_load_time,
            ),
        }
    }

    fn is_disabled(self) -> bool {
        self.minimum_wait.is_zero() && self.network_idle_wait.is_zero()
    }
}

#[derive(Debug, Clone, PartialEq)]
struct InteractionHighlightConfig {
    enabled: bool,
    color: String,
    duration_seconds: f64,
}

impl InteractionHighlightConfig {
    fn from_profile(profile: &BrowserProfile) -> Self {
        Self {
            enabled: profile.highlight_elements,
            color: profile.interaction_highlight_color.clone(),
            duration_seconds: profile.interaction_highlight_duration,
        }
    }

    fn element_script(&self, bounds: Option<ElementBounds>) -> Option<String> {
        if !self.enabled {
            return None;
        }
        let bounds = bounds?;
        if bounds.width == 0 || bounds.height == 0 {
            return None;
        }
        Some(interaction_element_highlight_script(
            bounds,
            &self.color,
            self.duration_seconds,
        ))
    }

    fn coordinate_script(&self, x: i32, y: i32) -> Option<String> {
        if !self.enabled {
            return None;
        }
        Some(interaction_coordinate_highlight_script(
            x,
            y,
            &self.color,
            self.duration_seconds,
        ))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct DomHighlightConfig {
    enabled: bool,
    filter_highlight_ids: bool,
}

impl DomHighlightConfig {
    fn from_profile(profile: &BrowserProfile) -> Self {
        Self {
            enabled: profile.dom_highlight_elements,
            filter_highlight_ids: profile.filter_highlight_ids,
        }
    }

    fn overlay_script(&self, selector_map: &BTreeMap<u32, DomElementRef>) -> Option<String> {
        if !self.enabled {
            return None;
        }
        let elements = dom_highlight_overlay_elements(selector_map, self.filter_highlight_ids);
        Some(dom_highlight_overlay_script(&elements))
    }
}

#[derive(Debug)]
struct NetworkActivityState {
    active_request_ids: BTreeSet<String>,
    last_activity_at: Instant,
}

impl NetworkActivityState {
    fn new(now: Instant) -> Self {
        Self {
            active_request_ids: BTreeSet::new(),
            last_activity_at: now,
        }
    }

    fn observe_request_started(&mut self, request_id: &str, now: Instant) {
        self.active_request_ids.insert(request_id.to_owned());
        self.last_activity_at = now;
    }

    fn observe_request_finished(&mut self, request_id: &str, now: Instant) {
        self.active_request_ids.remove(request_id);
        self.last_activity_at = now;
    }

    fn idle_remaining(&self, now: Instant, idle_for: Duration) -> Option<Duration> {
        if !self.active_request_ids.is_empty() {
            return Some(idle_for);
        }
        let elapsed = now.saturating_duration_since(self.last_activity_at);
        if elapsed >= idle_for {
            None
        } else {
            Some(idle_for - elapsed)
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct FrameOffset {
    x: i32,
    y: i32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct FrameElementInfo {
    url: String,
    offset: FrameOffset,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct IframeTargetInfo {
    target_id: String,
    offset: FrameOffset,
    depth: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct AttachedFramePage {
    page: AttachedPage,
    offset: FrameOffset,
    depth: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CachedDomElementRef {
    element: DomElementRef,
    target_local_index: u32,
}

pub struct CdpBrowserSession {
    connection: Arc<CdpConnection>,
    page: Arc<Mutex<AttachedPage>>,
    last_dom_state: Arc<Mutex<Option<SerializedDomState>>>,
    pending_url_policy_error: Arc<Mutex<Option<BrowserError>>>,
    security_events: Arc<Mutex<VecDeque<BrowserSecurityEvent>>>,
    lifecycle_events: Arc<Mutex<VecDeque<BrowserLifecycleEvent>>>,
    lifecycle_event_tx: broadcast::Sender<BrowserLifecycleEvent>,
    url_policy: UrlAccessPolicy,
    iframe_traversal: IframeTraversalConfig,
    paint_order_filtering: bool,
    viewport_emulation: ViewportEmulationConfig,
    page_load_wait: PageLoadWaitConfig,
    interaction_highlight: InteractionHighlightConfig,
    dom_highlight: DomHighlightConfig,
    network_activity: Arc<Mutex<NetworkActivityState>>,
    har_recorder: Option<Arc<CdpHarRecorder>>,
    video_recorder: Option<Arc<CdpVideoRecorder>>,
    trace_recorder: Option<CdpTraceRecorder>,
    downloads_path: Option<PathBuf>,
    auto_download_pdfs: bool,
    auto_pdf_downloads: Arc<Mutex<BTreeMap<String, PathBuf>>>,
    storage_state_path: Option<PathBuf>,
    navigation_timeout_ms: u64,
    _lifecycle_watchdog: BrowserLifecycleWatchdog,
    _security_watchdog: Option<BrowserSecurityWatchdog>,
    _launched_browser: Option<LaunchedBrowser>,
    _downloads_dir: Option<TempDir>,
}

struct SessionDownloads {
    path: Option<PathBuf>,
    temp_dir: Option<TempDir>,
}

impl SessionDownloads {
    fn from_profile(profile: &BrowserProfile) -> Result<Self, BrowserError> {
        if !profile.accept_downloads {
            return Ok(Self {
                path: None,
                temp_dir: None,
            });
        }
        if let Some(downloads_path) = &profile.downloads_path {
            return Ok(Self {
                path: Some(downloads_path.clone()),
                temp_dir: None,
            });
        }
        let temp_dir = tempfile::Builder::new()
            .prefix("browser-use-downloads-")
            .tempdir()
            .map_err(|error| BrowserError::StateUnavailable(error.to_string()))?;
        Ok(Self {
            path: Some(temp_dir.path().to_path_buf()),
            temp_dir: Some(temp_dir),
        })
    }
}

impl CdpBrowserSession {
    pub async fn connect(endpoint: DevToolsEndpoint) -> Result<Self, BrowserError> {
        Self::connect_with_profile(endpoint, &BrowserProfile::default()).await
    }

    pub async fn connect_with_profile(
        endpoint: DevToolsEndpoint,
        profile: &BrowserProfile,
    ) -> Result<Self, BrowserError> {
        let downloads = SessionDownloads::from_profile(profile)?;
        let connection =
            CdpConnection::connect_with_headers(&endpoint, profile.headers.as_ref()).await?;
        let permission_grant_event =
            grant_browser_permissions(&connection, &profile.permissions).await;
        if let Some(downloads_path) = &downloads.path {
            enable_browser_download_events(&connection, downloads_path).await?;
        }
        let page = attach_or_create_page(&connection).await?;
        let initial_page = page.clone();
        let viewport_emulation = ViewportEmulationConfig::from_profile(profile);
        apply_viewport_emulation_for_page(&connection, &page, viewport_emulation).await?;
        let page = Arc::new(Mutex::new(page));
        let last_dom_state = Arc::new(Mutex::new(None));
        let pending_url_policy_error = Arc::new(Mutex::new(None));
        let security_events = Arc::new(Mutex::new(VecDeque::new()));
        let lifecycle_events = Arc::new(Mutex::new(VecDeque::new()));
        let network_activity = Arc::new(Mutex::new(NetworkActivityState::new(Instant::now())));
        let har_recorder = CdpHarRecorder::from_profile(profile);
        let video_recorder = CdpVideoRecorder::from_profile(profile);
        let trace_recorder = CdpTraceRecorder::from_profile(profile);
        let auto_pdf_downloads = Arc::new(Mutex::new(BTreeMap::new()));
        let cdp_auto_pdf_download = CdpAutoPdfDownloadState::from_downloads(
            profile.auto_download_pdfs,
            downloads.path.as_deref(),
            auto_pdf_downloads.clone(),
        );
        let (lifecycle_event_tx, _) = broadcast::channel(256);
        {
            let mut events = lifecycle_events.lock().await;
            push_lifecycle_event_and_publish(
                &mut events,
                &lifecycle_event_tx,
                BrowserLifecycleEvent::browser_connected(endpoint.http_url.clone()),
            );
            if let Some(event) = permission_grant_event {
                push_lifecycle_event_and_publish(&mut events, &lifecycle_event_tx, event);
            }
        }
        let lifecycle_watchdog = BrowserLifecycleWatchdog::start(
            connection.clone(),
            lifecycle_events.clone(),
            lifecycle_event_tx.clone(),
            profile.network_request_timeout_ms,
            network_activity.clone(),
            BrowserLifecycleWatchdogRecorders {
                cdp_auto_pdf_download,
                har_recorder: har_recorder.clone(),
                video_recorder: video_recorder.clone(),
            },
        );
        let page_load_wait = PageLoadWaitConfig::from_profile(profile);

        let session = Self {
            connection,
            page,
            last_dom_state,
            pending_url_policy_error,
            security_events,
            lifecycle_events,
            lifecycle_event_tx,
            url_policy: UrlAccessPolicy::from_profile(profile),
            iframe_traversal: IframeTraversalConfig::from_profile(profile),
            paint_order_filtering: profile.paint_order_filtering,
            viewport_emulation,
            page_load_wait,
            interaction_highlight: InteractionHighlightConfig::from_profile(profile),
            dom_highlight: DomHighlightConfig::from_profile(profile),
            network_activity,
            har_recorder,
            video_recorder,
            trace_recorder,
            downloads_path: downloads.path,
            auto_download_pdfs: profile.auto_download_pdfs,
            auto_pdf_downloads,
            storage_state_path: None,
            navigation_timeout_ms: profile.navigation_timeout_ms,
            _lifecycle_watchdog: lifecycle_watchdog,
            _security_watchdog: None,
            _launched_browser: None,
            _downloads_dir: downloads.temp_dir,
        };
        session.start_video_recording_for_page(&initial_page).await;
        Ok(session)
    }

    pub async fn launch(profile: &BrowserProfile) -> Result<Self, BrowserError> {
        let downloads = SessionDownloads::from_profile(profile)?;
        let url_policy = UrlAccessPolicy::from_profile(profile);
        let (endpoint, launched_browser) = if profile.uses_cloud() {
            (profile.create_cloud_devtools_endpoint().await?, None)
        } else {
            let launched_browser = profile.launch_local().await?;
            (launched_browser.endpoint().clone(), Some(launched_browser))
        };
        let launched_browser = launched_browser.and_then(|browser| {
            if profile_keeps_launched_browser_alive(profile) {
                let _ = browser.detach();
                None
            } else {
                Some(browser)
            }
        });
        let connection =
            CdpConnection::connect_with_headers(&endpoint, profile.headers.as_ref()).await?;
        let permission_grant_event =
            grant_browser_permissions(&connection, &profile.permissions).await;
        if let Some(downloads_path) = &downloads.path {
            enable_browser_download_events(&connection, downloads_path).await?;
        }
        let page = attach_or_create_page(&connection).await?;
        let initial_page = page.clone();
        let viewport_emulation = ViewportEmulationConfig::from_profile(profile);
        apply_viewport_emulation_for_page(&connection, &page, viewport_emulation).await?;
        let storage_state_loaded_event = if let Some(storage_state_path) =
            &profile.storage_state_path
        {
            let storage_state = load_browser_storage_state(&connection, storage_state_path).await?;
            apply_origin_storage_state(&connection, &page, &storage_state).await?;
            let (cookies_count, origins_count) = storage_state_counts(&storage_state);
            Some(BrowserLifecycleEvent::storage_state_loaded(
                storage_state_path.display().to_string(),
                cookies_count,
                origins_count,
            ))
        } else {
            None
        };
        let page = Arc::new(Mutex::new(page));
        let last_dom_state = Arc::new(Mutex::new(None));
        let pending_url_policy_error = Arc::new(Mutex::new(None));
        let security_events = Arc::new(Mutex::new(VecDeque::new()));
        let lifecycle_events = Arc::new(Mutex::new(VecDeque::new()));
        let network_activity = Arc::new(Mutex::new(NetworkActivityState::new(Instant::now())));
        let har_recorder = CdpHarRecorder::from_profile(profile);
        let video_recorder = CdpVideoRecorder::from_profile(profile);
        let trace_recorder = CdpTraceRecorder::from_profile(profile);
        let auto_pdf_downloads = Arc::new(Mutex::new(BTreeMap::new()));
        let cdp_auto_pdf_download = CdpAutoPdfDownloadState::from_downloads(
            profile.auto_download_pdfs,
            downloads.path.as_deref(),
            auto_pdf_downloads.clone(),
        );
        let (lifecycle_event_tx, _) = broadcast::channel(256);
        {
            let mut events = lifecycle_events.lock().await;
            push_lifecycle_event_and_publish(
                &mut events,
                &lifecycle_event_tx,
                BrowserLifecycleEvent::browser_connected(endpoint.http_url.clone()),
            );
            if let Some(event) = permission_grant_event {
                push_lifecycle_event_and_publish(&mut events, &lifecycle_event_tx, event);
            }
            if let Some(event) = storage_state_loaded_event {
                push_lifecycle_event_and_publish(&mut events, &lifecycle_event_tx, event);
            }
        }
        let lifecycle_watchdog = BrowserLifecycleWatchdog::start(
            connection.clone(),
            lifecycle_events.clone(),
            lifecycle_event_tx.clone(),
            profile.network_request_timeout_ms,
            network_activity.clone(),
            BrowserLifecycleWatchdogRecorders {
                cdp_auto_pdf_download,
                har_recorder: har_recorder.clone(),
                video_recorder: video_recorder.clone(),
            },
        );
        let security_watchdog = BrowserSecurityWatchdog::start(
            connection.clone(),
            page.clone(),
            last_dom_state.clone(),
            pending_url_policy_error.clone(),
            security_events.clone(),
            LifecycleEventSink {
                events: lifecycle_events.clone(),
                event_tx: lifecycle_event_tx.clone(),
            },
            url_policy.clone(),
        )
        .await?;

        let session = Self {
            connection,
            page,
            last_dom_state,
            pending_url_policy_error,
            security_events,
            lifecycle_events,
            lifecycle_event_tx,
            url_policy,
            iframe_traversal: IframeTraversalConfig::from_profile(profile),
            paint_order_filtering: profile.paint_order_filtering,
            viewport_emulation,
            page_load_wait: PageLoadWaitConfig::from_profile(profile),
            interaction_highlight: InteractionHighlightConfig::from_profile(profile),
            dom_highlight: DomHighlightConfig::from_profile(profile),
            network_activity,
            har_recorder,
            video_recorder,
            trace_recorder,
            downloads_path: downloads.path,
            auto_download_pdfs: profile.auto_download_pdfs,
            auto_pdf_downloads,
            storage_state_path: profile.storage_state_path.clone(),
            navigation_timeout_ms: profile.navigation_timeout_ms,
            _lifecycle_watchdog: lifecycle_watchdog,
            _security_watchdog: security_watchdog,
            _launched_browser: launched_browser,
            _downloads_dir: downloads.temp_dir,
        };
        session.start_video_recording_for_page(&initial_page).await;
        Ok(session)
    }

    pub async fn close_browser(&self) -> Result<(), BrowserError> {
        self.record_lifecycle_event(BrowserLifecycleEvent::browser_close_requested())
            .await;
        if let Some(path) = &self.storage_state_path {
            self.save_storage_state(path).await?;
        }
        if let Some(har_recorder) = &self.har_recorder {
            let _ = har_recorder.write_har().await;
        }
        if let Some(video_recorder) = &self.video_recorder {
            match video_recorder.stop_and_write(&self.connection).await {
                Ok((_path, Some(error))) => {
                    self.record_lifecycle_event(video_recording_failed_event("encode", &error))
                        .await;
                }
                Ok((_path, None)) => {}
                Err(error) => {
                    self.record_lifecycle_event(video_recording_failed_event("stop", &error))
                        .await;
                }
            }
        }
        if let Err(error) = self.write_trace_artifact().await {
            self.record_lifecycle_event(trace_recording_failed_event("write", &error))
                .await;
        }
        self.connection.mark_intentional_stop();
        self.connection
            .command("Browser.close", json!({}), None)
            .await
            .map(|_| ())
    }

    pub async fn save_storage_state(&self, path: &Path) -> Result<(), BrowserError> {
        let page = self.current_page().await;
        let storage_state = browser_storage_state(&self.connection, Some(&page)).await?;
        let (cookies_count, origins_count) = storage_state_counts(&storage_state);
        write_storage_state(path, &storage_state).await?;
        self.record_lifecycle_event(BrowserLifecycleEvent::storage_state_saved(
            path.display().to_string(),
            cookies_count,
            origins_count,
        ))
        .await;
        Ok(())
    }

    pub async fn load_storage_state(&self, path: &Path) -> Result<(), BrowserError> {
        let storage_state = load_browser_storage_state(&self.connection, path).await?;
        let page = self.current_page().await;
        apply_origin_storage_state(&self.connection, &page, &storage_state).await?;
        let (cookies_count, origins_count) = storage_state_counts(&storage_state);
        self.record_lifecycle_event(BrowserLifecycleEvent::storage_state_loaded(
            path.display().to_string(),
            cookies_count,
            origins_count,
        ))
        .await;
        Ok(())
    }

    async fn write_trace_artifact(&self) -> Result<Option<PathBuf>, BrowserError> {
        let Some(trace_recorder) = &self.trace_recorder else {
            return Ok(None);
        };
        let generated_at_millis = trace_epoch_millis();
        let current_page = self.page.lock().await.clone();
        let lifecycle_events = self
            .lifecycle_events
            .lock()
            .await
            .iter()
            .cloned()
            .collect::<Vec<_>>();
        let security_events = self
            .security_events
            .lock()
            .await
            .iter()
            .map(trace_security_event_json)
            .collect::<Vec<_>>();
        let last_dom_state = self.last_dom_state.lock().await.clone();
        let artifact = json!({
            "schema_version": TRACE_ARTIFACT_SCHEMA_VERSION,
            "artifact": {
                "kind": TRACE_ARTIFACT_KIND,
                "format": "json",
                "runtime": "direct_cdp",
                "playwright_trace_zip": false,
            },
            "generated_at": trace_timestamp(generated_at_millis),
            "current_page": {
                "target_id": current_page.target_id,
                "session_id": current_page.session_id,
            },
            "lifecycle_events": lifecycle_events,
            "security_events": security_events,
            "last_dom_state": last_dom_state,
        });

        trace_recorder
            .write_trace_artifact(artifact)
            .await
            .map(Some)
    }

    async fn current_page(&self) -> AttachedPage {
        let page = self.page.lock().await.clone();
        if self
            .connection
            .is_registered_session_stale(&page.session_id)
            .await
        {
            return self
                .reattach_current_page(page.clone())
                .await
                .unwrap_or(page);
        }
        page
    }

    async fn set_current_page(&self, page: AttachedPage) {
        *self.page.lock().await = page.clone();
        self.start_video_recording_for_page(&page).await;
    }

    async fn start_video_recording_for_page(&self, page: &AttachedPage) {
        let Some(video_recorder) = &self.video_recorder else {
            return;
        };
        if let Err(error) = video_recorder
            .start_screencast_for_page(&self.connection, page)
            .await
        {
            self.record_lifecycle_event(video_recording_failed_event("start", &error))
                .await;
        }
    }

    async fn apply_viewport_emulation(&self, page: &AttachedPage) -> Result<(), BrowserError> {
        apply_viewport_emulation_for_page(&self.connection, page, self.viewport_emulation).await
    }

    async fn wait_for_page_load_settle(&self) {
        if self.page_load_wait.is_disabled() {
            return;
        }
        if !self.page_load_wait.minimum_wait.is_zero() {
            sleep(self.page_load_wait.minimum_wait).await;
        }
        if !self.page_load_wait.network_idle_wait.is_zero() {
            self.wait_for_network_idle(self.page_load_wait.network_idle_wait)
                .await;
        }
    }

    async fn wait_for_network_idle(&self, idle_for: Duration) {
        let deadline = Instant::now() + idle_for;
        loop {
            let now = Instant::now();
            if now >= deadline {
                return;
            }
            let remaining = {
                self.network_activity
                    .lock()
                    .await
                    .idle_remaining(now, idle_for)
            };
            let Some(remaining) = remaining else {
                return;
            };
            let until_deadline = deadline.saturating_duration_since(now);
            let sleep_for = remaining.min(until_deadline).min(Duration::from_millis(50));
            if sleep_for.is_zero() {
                return;
            }
            sleep(sleep_for).await;
        }
    }

    async fn auto_download_pdf_if_needed(&self, url: &str) {
        if !self.auto_download_pdfs || !is_pdf_viewer_url(url) {
            return;
        }
        let Some(downloads_path) = &self.downloads_path else {
            return;
        };

        match self.auto_download_pdf(url, downloads_path).await {
            Ok(Some(event)) => self.record_lifecycle_event(event).await,
            Ok(None) => {}
            Err(error) => {
                self.record_lifecycle_event(BrowserLifecycleEvent::pdf_auto_download_failed(
                    url,
                    error.to_string(),
                ))
                .await;
            }
        }
    }

    async fn auto_download_pdf(
        &self,
        url: &str,
        downloads_path: &Path,
    ) -> Result<Option<BrowserLifecycleEvent>, BrowserError> {
        if let Some(path) = self.cached_auto_pdf_download(url).await {
            if tokio::fs::metadata(&path).await.is_ok() {
                return Ok(None);
            }
        }

        let response = download_http_client()
            .get(url)
            .send()
            .await
            .map_err(|error| BrowserError::StateUnavailable(error.to_string()))?;
        if !response.status().is_success() {
            return Err(BrowserError::StateUnavailable(format!(
                "PDF download returned HTTP {}",
                response.status()
            )));
        }
        let bytes = response
            .bytes()
            .await
            .map_err(|error| BrowserError::StateUnavailable(error.to_string()))?;
        tokio::fs::create_dir_all(downloads_path)
            .await
            .map_err(|error| BrowserError::StateUnavailable(error.to_string()))?;
        let file_name = pdf_download_filename_from_url(url);
        let path = unique_download_path(downloads_path, &file_name).await?;
        tokio::fs::write(&path, &bytes)
            .await
            .map_err(|error| BrowserError::StateUnavailable(error.to_string()))?;
        self.auto_pdf_downloads
            .lock()
            .await
            .insert(url.to_owned(), path.clone());

        Ok(Some(BrowserLifecycleEvent::pdf_auto_downloaded(
            url,
            path.display().to_string(),
            path.file_name()
                .and_then(|name| name.to_str())
                .map(str::to_owned)
                .unwrap_or(file_name),
            u64::try_from(bytes.len()).unwrap_or(u64::MAX),
        )))
    }

    async fn cached_auto_pdf_download(&self, url: &str) -> Option<PathBuf> {
        self.auto_pdf_downloads.lock().await.get(url).cloned()
    }

    async fn reattach_current_page(
        &self,
        stale_page: AttachedPage,
    ) -> Result<AttachedPage, BrowserError> {
        let page = match attach_to_target(&self.connection, stale_page.target_id.clone()).await {
            Ok(page) => page,
            Err(error) if is_missing_target_error(&error) => {
                attach_or_create_page(&self.connection).await?
            }
            Err(error) => return Err(error),
        };
        self.apply_viewport_emulation(&page).await?;
        let target_id = page.target_id.clone();
        self.set_current_page(page.clone()).await;
        self.clear_cached_dom_state().await;
        self.record_lifecycle_event(BrowserLifecycleEvent::target_switched(target_id))
            .await;
        Ok(page)
    }

    async fn set_cached_dom_state(&self, dom_state: SerializedDomState) {
        *self.last_dom_state.lock().await = Some(dom_state);
    }

    async fn clear_cached_dom_state(&self) {
        *self.last_dom_state.lock().await = None;
    }

    async fn take_pending_url_policy_error(&self) -> Result<(), BrowserError> {
        if let Some(error) = self.pending_url_policy_error.lock().await.take() {
            return Err(error);
        }
        Ok(())
    }

    async fn clear_matching_pending_url_policy_errors(&self, handled: &[(String, String)]) {
        let mut pending = self.pending_url_policy_error.lock().await;
        let Some(BrowserError::NavigationBlocked { url, reason }) = pending.as_ref() else {
            return;
        };
        if handled
            .iter()
            .any(|(handled_url, handled_reason)| handled_url == url && handled_reason == reason)
        {
            *pending = None;
        }
    }

    async fn validate_url_policy_before_navigation(&self, url: &str) -> Result<(), BrowserError> {
        match self.url_policy.validate(url) {
            Ok(()) => Ok(()),
            Err(BrowserError::NavigationBlocked { url, reason }) => {
                self.record_security_event(BrowserSecurityEvent::prevented_navigation(
                    url.clone(),
                    reason.clone(),
                ))
                .await;
                Err(BrowserError::NavigationBlocked { url, reason })
            }
            Err(error) => Err(error),
        }
    }

    async fn record_security_event(&self, event: BrowserSecurityEvent) {
        let lifecycle_event = event.lifecycle_event.clone();
        let mut events = self.security_events.lock().await;
        push_security_event(&mut events, event);
        drop(events);
        self.record_lifecycle_event(lifecycle_event).await;
    }

    async fn record_lifecycle_event(&self, event: BrowserLifecycleEvent) {
        let mut events = self.lifecycle_events.lock().await;
        push_lifecycle_event_and_publish(&mut events, &self.lifecycle_event_tx, event);
    }

    pub async fn lifecycle_events(&self) -> Vec<BrowserLifecycleEvent> {
        self.lifecycle_events.lock().await.iter().cloned().collect()
    }

    pub async fn lifecycle_adapter_events(&self) -> Vec<BrowserLifecycleAdapterEvent> {
        browser_lifecycle_adapter_events(&self.lifecycle_events().await)
    }

    pub fn subscribe_lifecycle_events(&self) -> BrowserLifecycleEventSubscription {
        BrowserLifecycleEventSubscription::new(self.lifecycle_event_tx.subscribe())
    }

    pub fn subscribe_lifecycle_adapter_events(&self) -> BrowserLifecycleAdapterEventSubscription {
        BrowserLifecycleAdapterEventSubscription::new(self.subscribe_lifecycle_events())
    }

    async fn cached_element(&self, index: u32) -> Option<CachedDomElementRef> {
        let state = self.last_dom_state.lock().await;
        let state = state.as_ref()?;
        let element = state.selector_map.get(&index)?.clone();
        let target_local_index =
            target_local_index_for_global_index(&state.selector_map, index, &element.target_id);

        Some(CachedDomElementRef {
            element,
            target_local_index,
        })
    }

    async fn evaluate_json(&self, expression: &str) -> Result<Value, BrowserError> {
        self.evaluate_json_with_options(expression, false).await
    }

    async fn evaluate_json_with_options(
        &self,
        expression: &str,
        include_command_line_api: bool,
    ) -> Result<Value, BrowserError> {
        let page = self.current_page().await;
        self.evaluate_json_for_page(&page, expression, include_command_line_api)
            .await
    }

    async fn evaluate_json_for_page(
        &self,
        page: &AttachedPage,
        expression: &str,
        include_command_line_api: bool,
    ) -> Result<Value, BrowserError> {
        let result = self
            .connection
            .command(
                "Runtime.evaluate",
                runtime_evaluate_params(expression, include_command_line_api),
                Some(&page.session_id),
            )
            .await?;

        runtime_evaluate_value(result)
    }

    async fn evaluate_effect(&self, expression: String) -> Result<(), BrowserError> {
        let page = self.current_page().await;
        self.evaluate_effect_for_page(&page, expression).await
    }

    async fn evaluate_effect_for_page(
        &self,
        page: &AttachedPage,
        expression: String,
    ) -> Result<(), BrowserError> {
        let result = self
            .connection
            .command(
                "Runtime.evaluate",
                json!({
                    "expression": expression,
                    "awaitPromise": true,
                    "returnByValue": true,
                }),
                Some(&page.session_id),
            )
            .await?;
        let _ = runtime_evaluate_value(result)?;
        Ok(())
    }

    async fn highlight_element_if_enabled(&self, element: &DomElementRef) {
        let Some(script) = self.interaction_highlight.element_script(element.bounds) else {
            return;
        };
        let Ok(page) = self.page_for_element(element).await else {
            return;
        };
        let _ = self.evaluate_effect_for_page(&page, script).await;
    }

    async fn highlight_coordinates_if_enabled(&self, x: i32, y: i32) {
        let Some(script) = self.interaction_highlight.coordinate_script(x, y) else {
            return;
        };
        let page = self.current_page().await;
        let _ = self.evaluate_effect_for_page(&page, script).await;
    }

    async fn apply_dom_highlights_if_enabled(&self, dom_state: &SerializedDomState) {
        let Some(script) = self.dom_highlight.overlay_script(&dom_state.selector_map) else {
            return;
        };
        let page = self.current_page().await;
        let _ = self.evaluate_effect_for_page(&page, script).await;
    }

    async fn page_for_element(
        &self,
        element: &DomElementRef,
    ) -> Result<AttachedPage, BrowserError> {
        let page = self.current_page().await;
        if element.target_id == page.target_id {
            return Ok(page);
        }

        attach_to_target(&self.connection, element.target_id.clone()).await
    }

    async fn page_for_index_fallback(
        &self,
        cached_element: Option<&CachedDomElementRef>,
    ) -> Result<AttachedPage, BrowserError> {
        let page = self.current_page().await;
        let target_id = index_fallback_target_id(&page, cached_element).to_owned();
        if target_id == page.target_id {
            return Ok(page);
        }

        attach_to_target(&self.connection, target_id).await
    }

    async fn resolve_element_object_id(
        &self,
        page: &AttachedPage,
        element: &DomElementRef,
    ) -> Result<String, BrowserError> {
        let params = if element.backend_node_id != 0 {
            json!({ "backendNodeId": element.backend_node_id })
        } else if let Some(node_id) = element.node_id {
            json!({ "nodeId": node_id })
        } else {
            return Err(BrowserError::MissingResponseData(
                "cached element node id".to_owned(),
            ));
        };

        self.connection
            .command("DOM.resolveNode", params, Some(&page.session_id))
            .await?
            .get("object")
            .and_then(|object| object.get("objectId"))
            .and_then(Value::as_str)
            .map(str::to_owned)
            .ok_or_else(|| BrowserError::MissingResponseData("DOM.resolveNode objectId".to_owned()))
    }

    async fn call_element_function(
        &self,
        element: &DomElementRef,
        function_declaration: String,
    ) -> Result<(), BrowserError> {
        let _ = self
            .call_element_function_value(element, function_declaration)
            .await?;
        Ok(())
    }

    async fn call_element_function_value(
        &self,
        element: &DomElementRef,
        function_declaration: String,
    ) -> Result<Value, BrowserError> {
        let page = self.page_for_element(element).await?;
        let object_id = self.resolve_element_object_id(&page, element).await?;
        let result = self
            .connection
            .command(
                "Runtime.callFunctionOn",
                json!({
                    "objectId": object_id,
                    "functionDeclaration": function_declaration,
                    "awaitPromise": true,
                    "returnByValue": true,
                }),
                Some(&page.session_id),
            )
            .await?;
        runtime_command_value(result, "Runtime.callFunctionOn")
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
        let page = self.current_page().await;
        let root_interactive_js =
            interactive_elements_js(self.iframe_traversal, self.paint_order_filtering);
        let value = self
            .evaluate_json_for_page(&page, &root_interactive_js, true)
            .await?;
        let accessibility = self
            .accessibility_enrichment(&page)
            .await
            .unwrap_or_default();
        let _ = self
            .evaluate_effect_for_page(&page, CLEANUP_AX_REFS_JS.to_owned())
            .await;
        let root_state = dom_state_from_interactive_value(&page.target_id, &value, &accessibility)?;
        let frame_infos = self.frame_element_infos(&page).await.unwrap_or_default();
        let child_pages = self
            .iframe_target_pages(&page, &frame_infos)
            .await
            .unwrap_or_default();
        let mut child_states = Vec::new();

        for child_page in child_pages {
            let child_interactive_js = interactive_elements_js(
                IframeTraversalConfig {
                    max_iframe_depth: self
                        .iframe_traversal
                        .remaining_same_origin_depth(child_page.depth),
                    ..self.iframe_traversal
                },
                self.paint_order_filtering,
            );
            let Ok(value) = self
                .evaluate_json_for_page(&child_page.page, &child_interactive_js, true)
                .await
            else {
                continue;
            };
            let accessibility = self
                .accessibility_enrichment(&child_page.page)
                .await
                .unwrap_or_default();
            let _ = self
                .evaluate_effect_for_page(&child_page.page, CLEANUP_AX_REFS_JS.to_owned())
                .await;
            let Ok(mut child_state) = dom_state_from_interactive_value(
                &child_page.page.target_id,
                &value,
                &accessibility,
            ) else {
                continue;
            };
            offset_dom_state_bounds(&mut child_state, child_page.offset);
            child_states.push(child_state);
        }

        Ok(merge_dom_states(root_state, child_states))
    }

    async fn accessibility_enrichment(
        &self,
        page: &AttachedPage,
    ) -> Result<BTreeMap<String, AccessibilityNodeInfo>, BrowserError> {
        let snapshot = self
            .connection
            .command(
                "DOMSnapshot.captureSnapshot",
                json!({ "computedStyles": [] }),
                Some(&page.session_id),
            )
            .await?;
        let backend_by_ref = snapshot_backend_ids_by_ax_ref(&snapshot);
        if backend_by_ref.is_empty() {
            return Ok(BTreeMap::new());
        }
        let backend_node_ids = backend_by_ref.values().copied().collect::<Vec<_>>();
        let node_ids_by_backend = self
            .node_ids_by_backend_ids(page, &backend_node_ids)
            .await
            .unwrap_or_default();

        let ax_by_backend = self
            .connection
            .command(
                "Accessibility.getFullAXTree",
                json!({}),
                Some(&page.session_id),
            )
            .await
            .map(|tree| accessibility_nodes_by_backend_id(&tree))
            .unwrap_or_default();

        Ok(backend_by_ref
            .into_iter()
            .map(|(ax_ref, backend_node_id)| {
                let mut info = ax_by_backend
                    .get(&backend_node_id)
                    .cloned()
                    .unwrap_or_default();
                info.backend_node_id = backend_node_id;
                info.node_id = node_ids_by_backend.get(&backend_node_id).copied();
                (ax_ref, info)
            })
            .collect())
    }

    async fn frame_element_infos(
        &self,
        page: &AttachedPage,
    ) -> Result<Vec<FrameElementInfo>, BrowserError> {
        let value = self
            .evaluate_json_for_page(page, FRAME_ELEMENTS_JS, false)
            .await?;
        frame_element_infos_from_value(&value)
    }

    async fn iframe_target_pages(
        &self,
        page: &AttachedPage,
        frame_infos: &[FrameElementInfo],
    ) -> Result<Vec<AttachedFramePage>, BrowserError> {
        if !self.iframe_traversal.allows_cross_origin_depth(1) {
            return Ok(Vec::new());
        }
        let targets = self
            .connection
            .command("Target.getTargets", json!({}), None)
            .await?;
        let target_infos = iframe_target_infos_from_targets(
            &targets,
            &page.target_id,
            frame_infos,
            self.iframe_traversal,
        );
        let mut pages = Vec::new();

        for target_info in target_infos {
            match attach_to_target(&self.connection, target_info.target_id).await {
                Ok(page) => pages.push(AttachedFramePage {
                    page,
                    offset: target_info.offset,
                    depth: target_info.depth,
                }),
                Err(error) if is_missing_target_error(&error) => {}
                Err(error) => return Err(error),
            }
        }

        Ok(pages)
    }

    async fn page_text_for_page(&self, page: &AttachedPage) -> Result<String, BrowserError> {
        let value = self
            .evaluate_json_for_page(
                page,
                "(document.body ? document.body.innerText : document.documentElement.innerText || '')",
                false,
            )
            .await?;
        value
            .as_str()
            .map(str::to_owned)
            .ok_or_else(|| BrowserError::MissingResponseData("page text".to_owned()))
    }

    async fn find_elements_for_page(
        &self,
        page: &AttachedPage,
        selector: &str,
        attributes: &[String],
        max_results: usize,
        include_text: bool,
    ) -> Result<Vec<FoundElement>, BrowserError> {
        let selector_json = serde_json::to_string(selector)
            .map_err(|error| BrowserError::Transport(error.to_string()))?;
        let attributes_json = serde_json::to_string(attributes)
            .map_err(|error| BrowserError::Transport(error.to_string()))?;
        let value = self
            .evaluate_json_for_page(
                page,
                &format!(
                    r#"
JSON.stringify((() => {{
  const selector = {selector_json};
  const attributeNames = {attributes_json};
  return Array.from(document.querySelectorAll(selector)).slice(0, {max_results}).map((el) => {{
    const attrs = {{}};
    for (const name of attributeNames) {{
      const value = el.getAttribute(name);
      if (value !== null && value !== '') attrs[name] = value;
    }}
    return {{
      tag_name: el.tagName.toLowerCase(),
      text: {text_expr},
      attributes: attrs
    }};
  }});
}})())
"#,
                    selector_json = selector_json,
                    attributes_json = attributes_json,
                    max_results = max_results,
                    text_expr = if include_text {
                        "(el.innerText || el.value || '').trim().slice(0, 500)"
                    } else {
                        "null"
                    }
                ),
                false,
            )
            .await?;
        let encoded = value.as_str().ok_or_else(|| {
            BrowserError::MissingResponseData("find elements result string".to_owned())
        })?;
        serde_json::from_str(encoded).map_err(|error| BrowserError::Transport(error.to_string()))
    }

    async fn node_ids_by_backend_ids(
        &self,
        page: &AttachedPage,
        backend_node_ids: &[u64],
    ) -> Result<BTreeMap<u64, u64>, BrowserError> {
        if backend_node_ids.is_empty() {
            return Ok(BTreeMap::new());
        }

        let _ = self
            .connection
            .command(
                "DOM.getDocument",
                json!({ "depth": -1, "pierce": true }),
                Some(&page.session_id),
            )
            .await;
        let result = self
            .connection
            .command(
                "DOM.pushNodesByBackendIdsToFrontend",
                json!({ "backendNodeIds": backend_node_ids }),
                Some(&page.session_id),
            )
            .await?;
        let node_ids = result
            .get("nodeIds")
            .and_then(Value::as_array)
            .ok_or_else(|| BrowserError::MissingResponseData("DOM nodeIds".to_owned()))?;

        Ok(backend_node_ids
            .iter()
            .zip(node_ids)
            .filter_map(|(backend_node_id, node_id)| {
                let node_id = node_id.as_u64()?;
                (node_id != 0).then_some((*backend_node_id, node_id))
            })
            .collect())
    }

    async fn enforce_url_policy_after_settle(&self) -> Result<(), BrowserError> {
        if self.url_policy.is_unrestricted() {
            return Ok(());
        }

        sleep(Duration::from_millis(URL_POLICY_SETTLE_MS)).await;
        self.enforce_open_tab_url_policy().await
    }

    async fn enforce_url_policy_after_navigation_settle(&self) -> Result<(), BrowserError> {
        self.enforce_url_policy_after_settle().await?;
        self.wait_for_page_load_settle().await;
        self.enforce_open_tab_url_policy().await
    }

    async fn enforce_open_tab_url_policy(&self) -> Result<(), BrowserError> {
        if self.url_policy.is_unrestricted() {
            return Ok(());
        }
        self.take_pending_url_policy_error().await?;

        let tabs = page_tabs(&self.connection).await?;
        let current_page = self.current_page().await;
        let mut blocked: Option<BrowserError> = None;
        let mut handled_blocks = Vec::new();

        for tab in tabs {
            if self.url_policy.is_allowed(&tab.url) {
                continue;
            }

            let blocked_url = tab.url.clone();
            let reason = self.url_policy.block_reason(&tab.url).to_owned();
            if tab.target_id == current_page.target_id {
                let outcome = self
                    .connection
                    .command(
                        "Page.navigate",
                        json!({ "url": "about:blank" }),
                        Some(&current_page.session_id),
                    )
                    .await;
                let event = match outcome {
                    Ok(_) => BrowserSecurityEvent::reset_current(tab.url.clone(), reason.clone()),
                    Err(error) => BrowserSecurityEvent::reset_current_failed(
                        tab.url.clone(),
                        reason.clone(),
                        error.to_string(),
                    ),
                };
                self.record_security_event(event).await;
            } else {
                let outcome = self
                    .connection
                    .command(
                        "Target.closeTarget",
                        json!({ "targetId": &tab.target_id }),
                        None,
                    )
                    .await;
                match outcome {
                    Ok(_) => {
                        self.record_security_event(BrowserSecurityEvent::closed_popup(
                            tab.url.clone(),
                            reason.clone(),
                        ))
                        .await;
                    }
                    Err(error) => {
                        self.record_security_event(BrowserSecurityEvent::close_popup_failed(
                            tab.url.clone(),
                            reason.clone(),
                            error.to_string(),
                        ))
                        .await;
                        return Err(error);
                    }
                }
            }
            self.clear_cached_dom_state().await;
            handled_blocks.push((blocked_url.clone(), reason.clone()));

            if blocked.is_none() {
                blocked = Some(BrowserError::NavigationBlocked {
                    url: blocked_url,
                    reason,
                });
            }
        }

        if let Some(error) = blocked {
            // The watchdog observes the same CDP event stream and can report a current-tab
            // block just after synchronous enforcement already reset it. Keep the first
            // boundary error, but do not make the next state/action boundary fail again.
            sleep(Duration::from_millis(URL_POLICY_SETTLE_MS)).await;
            self.clear_matching_pending_url_policy_errors(&handled_blocks)
                .await;
            return Err(error);
        }

        Ok(())
    }
}

async fn attach_or_create_page(connection: &CdpConnection) -> Result<AttachedPage, BrowserError> {
    let targets = connection
        .command("Target.getTargets", json!({}), None)
        .await?;
    let target_infos = targets
        .get("targetInfos")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut page_targets: Vec<String> = target_infos
        .iter()
        .filter(|target| {
            target.get("type").and_then(Value::as_str) == Some("page")
                && target.get("url").and_then(Value::as_str) != Some("chrome://newtab/")
        })
        .filter_map(|target| {
            target
                .get("targetId")
                .and_then(Value::as_str)
                .map(str::to_owned)
        })
        .collect();
    page_targets.extend(
        target_infos
            .iter()
            .filter(|target| target.get("type").and_then(Value::as_str) == Some("page"))
            .filter(|target| target.get("url").and_then(Value::as_str) == Some("chrome://newtab/"))
            .filter_map(|target| {
                target
                    .get("targetId")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
            }),
    );

    for target_id in page_targets {
        match attach_to_target(connection, target_id).await {
            Ok(page) => return Ok(page),
            Err(BrowserError::CommandFailed { method, message })
                if method == "Target.attachToTarget"
                    && message.contains("No target with given id found") =>
            {
                continue;
            }
            Err(error) => return Err(error),
        }
    }

    let target_id = create_target(connection, "about:blank").await?;
    attach_to_target(connection, target_id).await
}

async fn create_target(connection: &CdpConnection, url: &str) -> Result<String, BrowserError> {
    connection
        .command("Target.createTarget", json!({ "url": url }), None)
        .await?
        .get("targetId")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| BrowserError::MissingResponseData("Target.createTarget targetId".to_owned()))
}

async fn attach_to_target(
    connection: &CdpConnection,
    target_id: String,
) -> Result<AttachedPage, BrowserError> {
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

    connection.register_attached_session(&session_id).await;
    connection
        .command("Page.enable", json!({}), Some(&session_id))
        .await?;
    connection
        .command("Network.enable", json!({}), Some(&session_id))
        .await?;

    Ok(AttachedPage {
        target_id,
        session_id,
    })
}

fn viewport_emulation_params(config: ViewportEmulationConfig) -> Option<Value> {
    config.viewport.map(|viewport| {
        json!({
            "width": viewport.width,
            "height": viewport.height,
            "deviceScaleFactor": config.device_scale_factor,
            "mobile": false,
        })
    })
}

async fn apply_viewport_emulation_for_page(
    connection: &CdpConnection,
    page: &AttachedPage,
    config: ViewportEmulationConfig,
) -> Result<(), BrowserError> {
    let Some(params) = viewport_emulation_params(config) else {
        return Ok(());
    };
    connection
        .command(
            "Emulation.setDeviceMetricsOverride",
            params,
            Some(&page.session_id),
        )
        .await
        .map(|_| ())
}

fn browser_permission_grant_params(permissions: &[String]) -> Option<Value> {
    (!permissions.is_empty()).then(|| json!({ "permissions": permissions }))
}

async fn grant_browser_permissions(
    connection: &CdpConnection,
    permissions: &[String],
) -> Option<BrowserLifecycleEvent> {
    let params = browser_permission_grant_params(permissions)?;
    match connection
        .command("Browser.grantPermissions", params, None)
        .await
    {
        Ok(_) => None,
        Err(error) => Some(BrowserLifecycleEvent::permissions_grant_failed(
            permissions,
            error.to_string(),
        )),
    }
}

async fn enable_browser_download_events(
    connection: &CdpConnection,
    downloads_path: &Path,
) -> Result<(), BrowserError> {
    tokio::fs::create_dir_all(downloads_path)
        .await
        .map_err(|error| BrowserError::StateUnavailable(error.to_string()))?;
    let downloads_path = downloads_path.display().to_string();
    connection
        .command(
            "Browser.setDownloadBehavior",
            json!({
                "behavior": "allow",
                "downloadPath": downloads_path,
                "eventsEnabled": true,
            }),
            None,
        )
        .await
        .map(|_| ())
}

fn resolve_page_target_id_from_tabs(
    tabs: &[TabInfo],
    tab_id_or_target_id: &str,
) -> Result<String, BrowserError> {
    if let Some(tab) = tabs.iter().find(|tab| tab.target_id == tab_id_or_target_id) {
        return Ok(tab.target_id.clone());
    }

    if tab_id_or_target_id.len() == 4 {
        let matches = tabs
            .iter()
            .filter(|tab| tab.short_target_id() == tab_id_or_target_id)
            .collect::<Vec<_>>();
        return match matches.as_slice() {
            [tab] => Ok(tab.target_id.clone()),
            [] => Err(BrowserError::ActionFailed(format!(
                "No open tab found for short tab id {tab_id_or_target_id}"
            ))),
            _ => Err(BrowserError::ActionFailed(format!(
                "Short tab id {tab_id_or_target_id} matched multiple open tabs"
            ))),
        };
    }

    Err(BrowserError::ActionFailed(format!(
        "No open tab found for target id {tab_id_or_target_id}"
    )))
}

async fn resolve_page_target_id(
    connection: &CdpConnection,
    tab_id_or_target_id: &str,
) -> Result<String, BrowserError> {
    let tabs = page_tabs(connection).await?;
    resolve_page_target_id_from_tabs(&tabs, tab_id_or_target_id)
}

#[async_trait]
impl BrowserSession for CdpBrowserSession {
    fn subscribe_lifecycle_events(&self) -> BrowserLifecycleEventSubscription {
        BrowserLifecycleEventSubscription::new(self.lifecycle_event_tx.subscribe())
    }

    async fn state(&self, include_screenshot: bool) -> Result<BrowserStateSummary, BrowserError> {
        self.enforce_open_tab_url_policy().await?;
        self.wait_for_page_load_settle().await;
        let (url, title) = self.page_location().await?;
        let is_pdf_viewer = is_pdf_viewer_url(&url);
        if is_pdf_viewer {
            self.auto_download_pdf_if_needed(&url).await;
        }
        let page_info = self.page_info().await?;
        let dom_state = self.dom_state().await?;
        self.set_cached_dom_state(dom_state.clone()).await;
        self.apply_dom_highlights_if_enabled(&dom_state).await;
        let pagination_buttons = detect_pagination_buttons(&dom_state);
        let current_page = self.current_page().await;
        let tabs = page_tabs(&self.connection).await?;
        let (recent_events, closed_popup_messages, browser_errors) = {
            let events = self.security_events.lock().await;
            security_event_state_fields(&events)
        };
        let screenshot = if include_screenshot {
            Some(self.screenshot().await?.base64_png)
        } else {
            None
        };

        Ok(BrowserStateSummary {
            dom_state,
            url: url.clone(),
            title: title.clone(),
            tabs: if tabs.is_empty() {
                vec![TabInfo {
                    url,
                    title,
                    tab_id: TabInfo::tab_id_for_target(&current_page.target_id),
                    target_id: current_page.target_id,
                    parent_target_id: None,
                }]
            } else {
                tabs
            },
            screenshot,
            page_info: Some(page_info),
            pixels_above: page_info.pixels_above,
            pixels_below: page_info.pixels_below,
            browser_errors,
            is_pdf_viewer,
            recent_events,
            pending_network_requests: vec![],
            pagination_buttons,
            closed_popup_messages,
        })
    }

    async fn navigate(&self, url: &str, new_tab: bool) -> Result<(), BrowserError> {
        self.validate_url_policy_before_navigation(url).await?;
        if new_tab {
            let target_id = create_target(&self.connection, url).await?;
            self.record_lifecycle_event(BrowserLifecycleEvent::target_created(
                target_id.clone(),
                url.to_owned(),
            ))
            .await;
            self.record_lifecycle_event(BrowserLifecycleEvent::navigation_started(
                target_id.clone(),
                url.to_owned(),
            ))
            .await;
            let page = match attach_to_target(&self.connection, target_id.clone()).await {
                Ok(page) => page,
                Err(error) => {
                    self.record_lifecycle_event(BrowserLifecycleEvent::navigation_failed(
                        target_id.clone(),
                        url.to_owned(),
                        error.to_string(),
                    ))
                    .await;
                    return Err(error);
                }
            };
            self.apply_viewport_emulation(&page).await?;
            let target_id = page.target_id.clone();
            self.set_current_page(page).await;
            self.record_lifecycle_event(BrowserLifecycleEvent::target_switched(target_id.clone()))
                .await;
            self.clear_cached_dom_state().await;
            let result = self.enforce_url_policy_after_navigation_settle().await;
            match &result {
                Ok(()) => {
                    self.record_lifecycle_event(BrowserLifecycleEvent::navigation_completed(
                        target_id,
                        url.to_owned(),
                    ))
                    .await;
                }
                Err(error) => {
                    self.record_lifecycle_event(BrowserLifecycleEvent::navigation_failed(
                        target_id,
                        url.to_owned(),
                        error.to_string(),
                    ))
                    .await;
                }
            }
            return result;
        }

        let page = self.current_page().await;
        self.record_lifecycle_event(BrowserLifecycleEvent::navigation_started(
            page.target_id.clone(),
            url.to_owned(),
        ))
        .await;
        let navigate = self.connection.command(
            "Page.navigate",
            json!({
                "url": url,
            }),
            Some(&page.session_id),
        );
        let navigate_result = if self.navigation_timeout_ms == 0 {
            navigate.await
        } else {
            match tokio::time::timeout(Duration::from_millis(self.navigation_timeout_ms), navigate)
                .await
            {
                Ok(result) => result,
                Err(_) => {
                    let timeout_seconds =
                        format!("{:.3}", self.navigation_timeout_ms as f64 / 1000.0);
                    self.record_lifecycle_event(BrowserLifecycleEvent::network_timeout(
                        page.target_id.clone(),
                        url.to_owned(),
                        timeout_seconds,
                    ))
                    .await;
                    return Err(BrowserError::NavigationFailed(format!(
                        "Page.navigate timed out after {}ms for {url}",
                        self.navigation_timeout_ms
                    )));
                }
            }
        };
        if let Err(error) = navigate_result {
            self.record_lifecycle_event(BrowserLifecycleEvent::navigation_failed(
                page.target_id.clone(),
                url.to_owned(),
                error.to_string(),
            ))
            .await;
            return Err(error);
        }
        self.clear_cached_dom_state().await;
        let result = self.enforce_url_policy_after_navigation_settle().await;
        match &result {
            Ok(()) => {
                self.record_lifecycle_event(BrowserLifecycleEvent::navigation_completed(
                    page.target_id,
                    url.to_owned(),
                ))
                .await;
            }
            Err(error) => {
                self.record_lifecycle_event(BrowserLifecycleEvent::navigation_failed(
                    page.target_id,
                    url.to_owned(),
                    error.to_string(),
                ))
                .await;
            }
        }
        result
    }

    async fn go_back(&self) -> Result<(), BrowserError> {
        let page = self.current_page().await;
        let history = self
            .connection
            .command(
                "Page.getNavigationHistory",
                json!({}),
                Some(&page.session_id),
            )
            .await?;
        let entry_id = previous_navigation_entry_id(&history)?;
        self.connection
            .command(
                "Page.navigateToHistoryEntry",
                json!({ "entryId": entry_id }),
                Some(&page.session_id),
            )
            .await?;
        self.clear_cached_dom_state().await;
        self.enforce_url_policy_after_navigation_settle().await
    }

    async fn switch_tab(&self, target_id: &str) -> Result<(), BrowserError> {
        let target_id = resolve_page_target_id(&self.connection, target_id).await?;
        let page = attach_to_target(&self.connection, target_id).await?;
        self.apply_viewport_emulation(&page).await?;
        let target_id = page.target_id.clone();
        self.set_current_page(page).await;
        self.record_lifecycle_event(BrowserLifecycleEvent::target_switched(target_id))
            .await;
        self.clear_cached_dom_state().await;
        self.enforce_open_tab_url_policy().await
    }

    async fn close_tab(&self, target_id: &str) -> Result<(), BrowserError> {
        let target_id = resolve_page_target_id(&self.connection, target_id).await?;
        self.connection
            .command(
                "Target.closeTarget",
                json!({ "targetId": &target_id }),
                None,
            )
            .await?;
        self.record_lifecycle_event(BrowserLifecycleEvent::target_closed(target_id.clone()))
            .await;

        if self.current_page().await.target_id == target_id {
            let page = attach_or_create_page(&self.connection).await?;
            self.apply_viewport_emulation(&page).await?;
            let target_id = page.target_id.clone();
            self.set_current_page(page).await;
            self.record_lifecycle_event(BrowserLifecycleEvent::target_switched(target_id))
                .await;
        }
        self.clear_cached_dom_state().await;

        self.enforce_open_tab_url_policy().await
    }

    async fn click(&self, index: u32) -> Result<(), BrowserError> {
        let cached_element = self.cached_element(index).await;
        if let Some(cached) = cached_element.as_ref() {
            self.highlight_element_if_enabled(&cached.element).await;
            match self
                .call_element_function(
                    &cached.element,
                    element_action_function_js(CLICK_ELEMENT_ACTION_JS),
                )
                .await
            {
                Ok(()) => return self.enforce_url_policy_after_navigation_settle().await,
                Err(error) if should_fallback_to_index_traversal(&error) => {}
                Err(error) => return Err(error),
            }
        }
        let page = self
            .page_for_index_fallback(cached_element.as_ref())
            .await?;
        let fallback_index = cached_element
            .as_ref()
            .map(|cached| cached.target_local_index)
            .unwrap_or(index);
        self.evaluate_effect_for_page(&page, click_element_js(fallback_index))
            .await?;
        self.enforce_url_policy_after_navigation_settle().await
    }

    async fn click_coordinates(&self, x: i32, y: i32) -> Result<(), BrowserError> {
        let page = self.current_page().await;
        self.highlight_coordinates_if_enabled(x, y).await;
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
                    Some(&page.session_id),
                )
                .await?;
        }
        self.enforce_url_policy_after_navigation_settle().await
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
        let cached_element = self.cached_element(index).await;
        if let Some(cached) = cached_element.as_ref() {
            self.highlight_element_if_enabled(&cached.element).await;
            match self
                .call_element_function(&cached.element, element_action_function_js(&action))
                .await
            {
                Ok(()) => return self.enforce_url_policy_after_settle().await,
                Err(error) if should_fallback_to_index_traversal(&error) => {}
                Err(error) => return Err(error),
            }
        }
        let page = self
            .page_for_index_fallback(cached_element.as_ref())
            .await?;
        let fallback_index = cached_element
            .as_ref()
            .map(|cached| cached.target_local_index)
            .unwrap_or(index);
        self.evaluate_effect_for_page(&page, element_action_js(fallback_index, &action))
            .await?;
        self.enforce_url_policy_after_settle().await
    }

    async fn scroll(&self, index: Option<u32>, down: bool, pages: f64) -> Result<(), BrowserError> {
        let direction = if down { 1.0 } else { -1.0 };
        if let Some(index) = index {
            let action = format!(
                "el.scrollBy(0, (el.clientHeight || window.innerHeight) * {});",
                pages * direction
            );
            let cached_element = self.cached_element(index).await;
            if let Some(cached) = cached_element.as_ref() {
                match self
                    .call_element_function(&cached.element, element_action_function_js(&action))
                    .await
                {
                    Ok(()) => return self.enforce_url_policy_after_settle().await,
                    Err(error) if should_fallback_to_index_traversal(&error) => {}
                    Err(error) => return Err(error),
                }
            }
            let page = self
                .page_for_index_fallback(cached_element.as_ref())
                .await?;
            let fallback_index = cached_element
                .as_ref()
                .map(|cached| cached.target_local_index)
                .unwrap_or(index);
            self.evaluate_effect_for_page(&page, element_action_js(fallback_index, &action))
                .await?;
            return self.enforce_url_policy_after_settle().await;
        }
        self.evaluate_effect(format!(
            "window.scrollBy(0, window.innerHeight * {}); true;",
            pages * direction
        ))
        .await?;
        self.enforce_url_policy_after_settle().await
    }

    async fn find_text(&self, text: &str) -> Result<bool, BrowserError> {
        let found = self
            .evaluate_json(&scroll_to_text_js(text)?)
            .await?
            .as_bool()
            .ok_or_else(|| BrowserError::MissingResponseData("scroll-to-text result".to_owned()))?;
        self.enforce_url_policy_after_settle().await?;
        Ok(found)
    }

    async fn evaluate(&self, code: &str) -> Result<String, BrowserError> {
        let page = self.current_page().await;
        let result = self
            .connection
            .command(
                "Runtime.evaluate",
                json!({
                    "expression": code,
                    "awaitPromise": true,
                    "returnByValue": true,
                }),
                Some(&page.session_id),
            )
            .await?;
        let rendered = render_runtime_evaluate_result(&result)?;
        self.enforce_url_policy_after_settle().await?;
        Ok(rendered)
    }

    async fn dropdown_options(&self, index: u32) -> Result<Vec<String>, BrowserError> {
        let cached_element = self.cached_element(index).await;
        if let Some(cached) = cached_element.as_ref() {
            match self
                .call_element_function_value(
                    &cached.element,
                    element_function_js(DROPDOWN_OPTIONS_BODY_JS),
                )
                .await
            {
                Ok(value) => return parse_dropdown_options_value(value),
                Err(error) if should_fallback_to_index_traversal(&error) => {}
                Err(error) => return Err(error),
            }
        }
        let page = self
            .page_for_index_fallback(cached_element.as_ref())
            .await?;
        let fallback_index = cached_element
            .as_ref()
            .map(|cached| cached.target_local_index)
            .unwrap_or(index);
        let value = self
            .evaluate_json_for_page(&page, &dropdown_options_js(fallback_index), false)
            .await?;
        parse_dropdown_options_value(value)
    }

    async fn select_dropdown_option(&self, index: u32, text: &str) -> Result<(), BrowserError> {
        let body = select_dropdown_option_body_js(text)?;
        let cached_element = self.cached_element(index).await;
        if let Some(cached) = cached_element.as_ref() {
            match self
                .call_element_function(&cached.element, element_function_js(&body))
                .await
            {
                Ok(()) => return self.enforce_url_policy_after_settle().await,
                Err(error) if should_fallback_to_index_traversal(&error) => {}
                Err(error) => return Err(error),
            }
        }
        let page = self
            .page_for_index_fallback(cached_element.as_ref())
            .await?;
        let fallback_index = cached_element
            .as_ref()
            .map(|cached| cached.target_local_index)
            .unwrap_or(index);
        self.evaluate_effect_for_page(&page, select_dropdown_option_js(fallback_index, text)?)
            .await?;
        self.enforce_url_policy_after_settle().await
    }

    async fn page_text(&self) -> Result<String, BrowserError> {
        let page = self.current_page().await;
        let mut texts = Vec::new();
        let root_text = self.page_text_for_page(&page).await?;
        if !root_text.trim().is_empty() {
            texts.push(root_text);
        }
        let frame_infos = self.frame_element_infos(&page).await.unwrap_or_default();
        let child_pages = self
            .iframe_target_pages(&page, &frame_infos)
            .await
            .unwrap_or_default();
        for child_page in child_pages {
            let Ok(text) = self.page_text_for_page(&child_page.page).await else {
                continue;
            };
            if !text.trim().is_empty() {
                texts.push(text);
            }
        }
        Ok(texts.join("\n"))
    }

    async fn find_elements(
        &self,
        selector: &str,
        attributes: &[String],
        max_results: usize,
        include_text: bool,
    ) -> Result<Vec<FoundElement>, BrowserError> {
        let page = self.current_page().await;
        let mut elements = self
            .find_elements_for_page(&page, selector, attributes, max_results, include_text)
            .await?;
        if elements.len() >= max_results {
            return Ok(elements);
        }

        let frame_infos = self.frame_element_infos(&page).await.unwrap_or_default();
        let child_pages = self
            .iframe_target_pages(&page, &frame_infos)
            .await
            .unwrap_or_default();
        for child_page in child_pages {
            let remaining = max_results.saturating_sub(elements.len());
            if remaining == 0 {
                break;
            }
            let Ok(mut child_elements) = self
                .find_elements_for_page(
                    &child_page.page,
                    selector,
                    attributes,
                    remaining,
                    include_text,
                )
                .await
            else {
                continue;
            };
            elements.append(&mut child_elements);
        }

        Ok(elements)
    }

    async fn send_keys(&self, keys: &str) -> Result<(), BrowserError> {
        let page = self.current_page().await;
        let normalized_keys = normalize_send_keys(keys);
        if normalized_keys.contains('+') {
            let parts = normalized_keys
                .split('+')
                .map(str::to_owned)
                .collect::<Vec<_>>();
            if let Some((main_key, modifiers)) = parts.split_last() {
                let modifier_value = modifier_mask(modifiers);
                for modifier in modifiers {
                    self.connection
                        .command(
                            "Input.dispatchKeyEvent",
                            key_event_params("keyDown", modifier, 0),
                            Some(&page.session_id),
                        )
                        .await?;
                }
                self.connection
                    .command(
                        "Input.dispatchKeyEvent",
                        key_event_params("keyDown", main_key, modifier_value),
                        Some(&page.session_id),
                    )
                    .await?;
                self.connection
                    .command(
                        "Input.dispatchKeyEvent",
                        key_event_params("keyUp", main_key, modifier_value),
                        Some(&page.session_id),
                    )
                    .await?;
                for modifier in modifiers.iter().rev() {
                    self.connection
                        .command(
                            "Input.dispatchKeyEvent",
                            key_event_params("keyUp", modifier, 0),
                            Some(&page.session_id),
                        )
                        .await?;
                }
            }
            return self.enforce_url_policy_after_settle().await;
        }

        if is_special_key(&normalized_keys) {
            self.connection
                .command(
                    "Input.dispatchKeyEvent",
                    key_event_params("keyDown", &normalized_keys, 0),
                    Some(&page.session_id),
                )
                .await?;
            if normalized_keys == "Enter" {
                self.connection
                    .command(
                        "Input.dispatchKeyEvent",
                        json!({
                            "type": "char",
                            "text": "\r",
                            "key": "Enter",
                        }),
                        Some(&page.session_id),
                    )
                    .await?;
            }
            self.connection
                .command(
                    "Input.dispatchKeyEvent",
                    key_event_params("keyUp", &normalized_keys, 0),
                    Some(&page.session_id),
                )
                .await?;
            return self.enforce_url_policy_after_settle().await;
        }

        self.connection
            .command(
                "Input.insertText",
                json!({
                    "text": normalized_keys,
                }),
                Some(&page.session_id),
            )
            .await?;
        self.enforce_url_policy_after_settle().await
    }

    async fn upload_file(&self, index: u32, path: &Path) -> Result<(), BrowserError> {
        let canonical_path = std::fs::canonicalize(path).map_err(|error| {
            BrowserError::ActionFailed(format!(
                "failed to resolve upload file '{}': {error}",
                path.display()
            ))
        })?;
        if !canonical_path.is_file() {
            return Err(BrowserError::ActionFailed(format!(
                "upload path is not a file: {}",
                canonical_path.display()
            )));
        }
        let path_string = canonical_path.to_str().ok_or_else(|| {
            BrowserError::ActionFailed(format!(
                "upload path is not valid UTF-8: {}",
                canonical_path.display()
            ))
        })?;

        let token = format!(
            "browser-use-rs-upload-{}",
            self.connection.next_id.fetch_add(1, Ordering::Relaxed)
        );
        let token_json = serde_json::to_string(&token)
            .map_err(|error| BrowserError::Transport(error.to_string()))?;
        let mark_upload_body = format!(
            r#"
  if (el.tagName.toLowerCase() !== 'input' || el.type !== 'file') {{
    throw new Error('Element is not a file input');
  }}
  el.setAttribute('data-browser-use-rs-upload-token', {token_json});
  return true;
"#
        );
        let cached_element = self.cached_element(index).await;
        let mut marked_cached_element = None;
        if let Some(cached) = cached_element.as_ref() {
            match self
                .call_element_function(&cached.element, element_function_js(&mark_upload_body))
                .await
            {
                Ok(()) => marked_cached_element = Some(cached.clone()),
                Err(error) if should_fallback_to_index_traversal(&error) => {}
                Err(error) => return Err(error),
            }
        }
        let page = if let Some(cached) = marked_cached_element.as_ref() {
            self.page_for_element(&cached.element).await?
        } else {
            self.page_for_index_fallback(cached_element.as_ref())
                .await?
        };
        let fallback_index = cached_element
            .as_ref()
            .map(|cached| cached.target_local_index)
            .unwrap_or(index);
        if marked_cached_element.is_none() {
            self.evaluate_effect_for_page(
                &page,
                element_eval_js(fallback_index, &mark_upload_body),
            )
            .await?;
        }

        let document = self
            .connection
            .command(
                "DOM.getDocument",
                json!({ "depth": -1, "pierce": true }),
                Some(&page.session_id),
            )
            .await?;
        let root_node_id = document
            .get("root")
            .and_then(|root| u32_field(root, "nodeId"))
            .ok_or_else(|| {
                BrowserError::MissingResponseData("DOM.getDocument root nodeId".to_owned())
            })?;
        let selector = format!(r#"[data-browser-use-rs-upload-token="{token}"]"#);
        let query_result = self
            .connection
            .command(
                "DOM.querySelector",
                json!({
                    "nodeId": root_node_id,
                    "selector": selector,
                }),
                Some(&page.session_id),
            )
            .await?;
        let node_id = u32_field(&query_result, "nodeId")
            .filter(|node_id| *node_id != 0)
            .ok_or_else(|| {
                BrowserError::MissingResponseData("DOM.querySelector nodeId".to_owned())
            })?;

        self.connection
            .command(
                "DOM.setFileInputFiles",
                json!({
                    "nodeId": node_id,
                    "files": [path_string],
                }),
                Some(&page.session_id),
            )
            .await?;

        let finish_upload_body = r#"
  el.removeAttribute('data-browser-use-rs-upload-token');
  el.dispatchEvent(new Event('input', { bubbles: true }));
  el.dispatchEvent(new Event('change', { bubbles: true }));
  return true;
"#;
        if let Some(cached) = marked_cached_element.as_ref() {
            match self
                .call_element_function(&cached.element, element_function_js(finish_upload_body))
                .await
            {
                Ok(()) => return self.enforce_url_policy_after_settle().await,
                Err(error) if should_fallback_to_index_traversal(&error) => {}
                Err(error) => return Err(error),
            }
        }
        self.evaluate_effect_for_page(&page, element_eval_js(fallback_index, finish_upload_body))
            .await?;
        self.enforce_url_policy_after_settle().await
    }

    async fn screenshot(&self) -> Result<Screenshot, BrowserError> {
        let page = self.current_page().await;
        let result = self
            .connection
            .command(
                "Page.captureScreenshot",
                json!({
                    "format": "png",
                    "fromSurface": true,
                }),
                Some(&page.session_id),
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

    async fn save_pdf(
        &self,
        print_background: bool,
        landscape: bool,
        scale: f64,
        paper_format: &str,
    ) -> Result<Pdf, BrowserError> {
        let page = self.current_page().await;
        let (paper_width, paper_height) = paper_size_inches(paper_format);
        let result = self
            .connection
            .command(
                "Page.printToPDF",
                json!({
                    "printBackground": print_background,
                    "landscape": landscape,
                    "scale": scale,
                    "paperWidth": paper_width,
                    "paperHeight": paper_height,
                }),
                Some(&page.session_id),
            )
            .await?;

        let base64_pdf = result
            .get("data")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .ok_or_else(|| BrowserError::MissingResponseData("Page.printToPDF data".to_owned()))?;

        Ok(Pdf { base64_pdf })
    }
}

fn paper_size_inches(format: &str) -> (f64, f64) {
    match format.to_ascii_lowercase().as_str() {
        "a4" => (8.27, 11.69),
        "legal" => (8.5, 14.0),
        "tabloid" => (11.0, 17.0),
        _ => (8.5, 11.0),
    }
}

fn previous_navigation_entry_id(history: &Value) -> Result<i64, BrowserError> {
    let current_index = history
        .get("currentIndex")
        .and_then(Value::as_i64)
        .ok_or_else(|| {
            BrowserError::MissingResponseData("Page.getNavigationHistory currentIndex".to_owned())
        })?;

    if current_index <= 0 {
        return Err(BrowserError::ActionFailed(
            "No previous browser history entry".to_owned(),
        ));
    }

    history
        .get("entries")
        .and_then(Value::as_array)
        .and_then(|entries| entries.get((current_index - 1) as usize))
        .and_then(|entry| entry.get("id"))
        .and_then(Value::as_i64)
        .ok_or_else(|| {
            BrowserError::MissingResponseData("Page.getNavigationHistory entries".to_owned())
        })
}

#[async_trait]
pub trait BrowserSession: Send + Sync {
    fn subscribe_lifecycle_events(&self) -> BrowserLifecycleEventSubscription {
        BrowserLifecycleEventSubscription::closed()
    }

    fn subscribe_lifecycle_adapter_events(&self) -> BrowserLifecycleAdapterEventSubscription {
        BrowserLifecycleAdapterEventSubscription::new(self.subscribe_lifecycle_events())
    }

    async fn state(&self, include_screenshot: bool) -> Result<BrowserStateSummary, BrowserError>;

    async fn navigate(&self, url: &str, new_tab: bool) -> Result<(), BrowserError>;

    async fn go_back(&self) -> Result<(), BrowserError>;

    async fn switch_tab(&self, target_id: &str) -> Result<(), BrowserError>;

    async fn close_tab(&self, target_id: &str) -> Result<(), BrowserError>;

    async fn click(&self, index: u32) -> Result<(), BrowserError>;

    async fn click_coordinates(&self, x: i32, y: i32) -> Result<(), BrowserError>;

    async fn input_text(&self, index: u32, text: &str, clear: bool) -> Result<(), BrowserError>;

    async fn scroll(&self, index: Option<u32>, down: bool, pages: f64) -> Result<(), BrowserError>;

    async fn find_text(&self, text: &str) -> Result<bool, BrowserError>;

    async fn evaluate(&self, code: &str) -> Result<String, BrowserError>;

    async fn dropdown_options(&self, index: u32) -> Result<Vec<String>, BrowserError>;

    async fn select_dropdown_option(&self, index: u32, text: &str) -> Result<(), BrowserError>;

    async fn page_text(&self) -> Result<String, BrowserError>;

    async fn find_elements(
        &self,
        selector: &str,
        attributes: &[String],
        max_results: usize,
        include_text: bool,
    ) -> Result<Vec<FoundElement>, BrowserError>;

    async fn send_keys(&self, keys: &str) -> Result<(), BrowserError>;

    async fn upload_file(&self, index: u32, path: &Path) -> Result<(), BrowserError>;

    async fn screenshot(&self) -> Result<Screenshot, BrowserError>;

    async fn save_pdf(
        &self,
        print_background: bool,
        landscape: bool,
        scale: f64,
        paper_format: &str,
    ) -> Result<Pdf, BrowserError>;
}

#[async_trait]
impl<T> BrowserSession for Arc<T>
where
    T: BrowserSession + ?Sized,
{
    fn subscribe_lifecycle_events(&self) -> BrowserLifecycleEventSubscription {
        self.as_ref().subscribe_lifecycle_events()
    }

    fn subscribe_lifecycle_adapter_events(&self) -> BrowserLifecycleAdapterEventSubscription {
        self.as_ref().subscribe_lifecycle_adapter_events()
    }

    async fn state(&self, include_screenshot: bool) -> Result<BrowserStateSummary, BrowserError> {
        self.as_ref().state(include_screenshot).await
    }

    async fn navigate(&self, url: &str, new_tab: bool) -> Result<(), BrowserError> {
        self.as_ref().navigate(url, new_tab).await
    }

    async fn go_back(&self) -> Result<(), BrowserError> {
        self.as_ref().go_back().await
    }

    async fn switch_tab(&self, target_id: &str) -> Result<(), BrowserError> {
        self.as_ref().switch_tab(target_id).await
    }

    async fn close_tab(&self, target_id: &str) -> Result<(), BrowserError> {
        self.as_ref().close_tab(target_id).await
    }

    async fn click(&self, index: u32) -> Result<(), BrowserError> {
        self.as_ref().click(index).await
    }

    async fn click_coordinates(&self, x: i32, y: i32) -> Result<(), BrowserError> {
        self.as_ref().click_coordinates(x, y).await
    }

    async fn input_text(&self, index: u32, text: &str, clear: bool) -> Result<(), BrowserError> {
        self.as_ref().input_text(index, text, clear).await
    }

    async fn scroll(&self, index: Option<u32>, down: bool, pages: f64) -> Result<(), BrowserError> {
        self.as_ref().scroll(index, down, pages).await
    }

    async fn find_text(&self, text: &str) -> Result<bool, BrowserError> {
        self.as_ref().find_text(text).await
    }

    async fn evaluate(&self, code: &str) -> Result<String, BrowserError> {
        self.as_ref().evaluate(code).await
    }

    async fn dropdown_options(&self, index: u32) -> Result<Vec<String>, BrowserError> {
        self.as_ref().dropdown_options(index).await
    }

    async fn select_dropdown_option(&self, index: u32, text: &str) -> Result<(), BrowserError> {
        self.as_ref().select_dropdown_option(index, text).await
    }

    async fn page_text(&self) -> Result<String, BrowserError> {
        self.as_ref().page_text().await
    }

    async fn find_elements(
        &self,
        selector: &str,
        attributes: &[String],
        max_results: usize,
        include_text: bool,
    ) -> Result<Vec<FoundElement>, BrowserError> {
        self.as_ref()
            .find_elements(selector, attributes, max_results, include_text)
            .await
    }

    async fn send_keys(&self, keys: &str) -> Result<(), BrowserError> {
        self.as_ref().send_keys(keys).await
    }

    async fn upload_file(&self, index: u32, path: &Path) -> Result<(), BrowserError> {
        self.as_ref().upload_file(index, path).await
    }

    async fn screenshot(&self) -> Result<Screenshot, BrowserError> {
        self.as_ref().screenshot().await
    }

    async fn save_pdf(
        &self,
        print_background: bool,
        landscape: bool,
        scale: f64,
        paper_format: &str,
    ) -> Result<Pdf, BrowserError> {
        self.as_ref()
            .save_pdf(print_background, landscape, scale, paper_format)
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cloud_browser_response_json(id: &str, status: &str) -> Value {
        json!({
            "id": id,
            "status": status,
            "liveUrl": format!("https://cloud.browser-use.com/live/{id}"),
            "cdpUrl": format!("wss://cdp.browser-use.com/devtools/browser/{id}"),
            "timeoutAt": "2026-05-18T20:00:00Z",
            "startedAt": "2026-05-18T19:00:00Z",
            "finishedAt": null
        })
    }

    async fn cloud_test_server(
        responses: Vec<(u16, Value)>,
    ) -> (String, tokio::task::JoinHandle<Vec<String>>) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind cloud test server");
        let addr = listener.local_addr().expect("cloud test server addr");
        let handle = tokio::spawn(async move {
            let mut requests = Vec::new();
            for (status, body) in responses {
                let (mut stream, _) = listener.accept().await.expect("accept cloud request");
                let mut buffer = Vec::new();
                let mut chunk = [0_u8; 1024];
                loop {
                    let read = stream.read(&mut chunk).await.expect("read cloud request");
                    if read == 0 {
                        break;
                    }
                    buffer.extend_from_slice(&chunk[..read]);
                    if http_request_complete(&buffer) {
                        break;
                    }
                }
                let request = String::from_utf8_lossy(&buffer).to_string();
                requests.push(request);
                let body = body.to_string();
                let reason = match status {
                    200 => "OK",
                    401 => "Unauthorized",
                    404 => "Not Found",
                    _ => "Error",
                };
                let response = format!(
                    "HTTP/1.1 {status} {reason}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                    body.len()
                );
                stream
                    .write_all(response.as_bytes())
                    .await
                    .expect("write cloud response");
            }
            requests
        });
        (format!("http://{addr}"), handle)
    }

    async fn pdf_download_test_server(
        body: &'static [u8],
    ) -> (String, tokio::task::JoinHandle<usize>) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind PDF test server");
        let addr = listener.local_addr().expect("PDF test server addr");
        let handle = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.expect("accept PDF request");
            let mut buffer = [0_u8; 1024];
            let _ = stream.read(&mut buffer).await.expect("read PDF request");
            let response = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: application/pdf\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
                body.len()
            );
            stream
                .write_all(response.as_bytes())
                .await
                .expect("write PDF response headers");
            stream
                .write_all(body)
                .await
                .expect("write PDF response body");
            1
        });
        (format!("http://{addr}/docs/report.pdf"), handle)
    }

    fn test_session_for_pdf_downloads(
        downloads_path: Option<PathBuf>,
        auto_download_pdfs: bool,
    ) -> CdpBrowserSession {
        let (request_tx, _request_rx) = mpsc::channel(1);
        let (event_tx, _) = broadcast::channel(16);
        let connection = Arc::new(CdpConnection {
            request_tx,
            event_tx,
            next_id: AtomicU64::new(1),
            intentional_stop: Arc::new(AtomicBool::new(false)),
            connection_generation: Arc::new(AtomicU64::new(0)),
            session_generations: Arc::new(Mutex::new(HashMap::new())),
        });
        let (lifecycle_event_tx, _) = broadcast::channel(16);
        CdpBrowserSession {
            connection,
            page: Arc::new(Mutex::new(AttachedPage {
                target_id: "target-1".to_owned(),
                session_id: "session-1".to_owned(),
            })),
            last_dom_state: Arc::new(Mutex::new(None)),
            pending_url_policy_error: Arc::new(Mutex::new(None)),
            security_events: Arc::new(Mutex::new(VecDeque::new())),
            lifecycle_events: Arc::new(Mutex::new(VecDeque::new())),
            lifecycle_event_tx,
            url_policy: UrlAccessPolicy::from_profile(&BrowserProfile::default()),
            iframe_traversal: IframeTraversalConfig::from_profile(&BrowserProfile::default()),
            paint_order_filtering: default_paint_order_filtering(),
            viewport_emulation: ViewportEmulationConfig::from_profile(&BrowserProfile::default()),
            page_load_wait: PageLoadWaitConfig::from_profile(&BrowserProfile::default()),
            interaction_highlight: InteractionHighlightConfig::from_profile(
                &BrowserProfile::default(),
            ),
            dom_highlight: DomHighlightConfig::from_profile(&BrowserProfile::default()),
            network_activity: Arc::new(Mutex::new(NetworkActivityState::new(Instant::now()))),
            har_recorder: None,
            video_recorder: None,
            trace_recorder: None,
            downloads_path,
            auto_download_pdfs,
            auto_pdf_downloads: Arc::new(Mutex::new(BTreeMap::new())),
            storage_state_path: None,
            navigation_timeout_ms: default_navigation_timeout_ms(),
            _lifecycle_watchdog: BrowserLifecycleWatchdog {
                handle: tokio::spawn(async {}),
            },
            _security_watchdog: None,
            _launched_browser: None,
            _downloads_dir: None,
        }
    }

    fn har_request_event(
        request_id: &str,
        url: &str,
        frame_id: &str,
        resource_type: &str,
        timestamp: f64,
        wall_time: f64,
    ) -> CdpEvent {
        CdpEvent {
            method: "Network.requestWillBeSent".to_owned(),
            params: json!({
                "requestId": request_id,
                "frameId": frame_id,
                "documentURL": url,
                "type": resource_type,
                "timestamp": timestamp,
                "wallTime": wall_time,
                "request": {
                    "url": url,
                    "method": "GET",
                    "headers": {
                        "Accept": "*/*"
                    }
                }
            }),
            session_id: None,
        }
    }

    fn har_response_event(
        request_id: &str,
        url: &str,
        status: u64,
        mime_type: &str,
        timestamp: f64,
    ) -> CdpEvent {
        CdpEvent {
            method: "Network.responseReceived".to_owned(),
            params: json!({
                "requestId": request_id,
                "timestamp": timestamp,
                "response": {
                    "url": url,
                    "status": status,
                    "statusText": "OK",
                    "mimeType": mime_type,
                    "protocol": "h2",
                    "remoteIPAddress": "203.0.113.10",
                    "remotePort": 443,
                    "headers": {
                        "Content-Type": mime_type,
                        "Content-Length": "5"
                    },
                    "securityDetails": {
                        "protocol": "TLS 1.3",
                        "subjectName": "example.test",
                        "sanList": ["example.test"]
                    }
                }
            }),
            session_id: None,
        }
    }

    async fn seed_har_entry(
        recorder: &CdpHarRecorder,
        request_id: &str,
        url: &str,
        frame_id: &str,
        resource_type: &str,
        body: &[u8],
    ) {
        recorder
            .observe_request_will_be_sent(&har_request_event(
                request_id,
                url,
                frame_id,
                resource_type,
                10.0,
                1_700_000_000.0,
            ))
            .await;
        recorder
            .observe_response_received(&har_response_event(
                request_id,
                url,
                200,
                "text/plain",
                10.1,
            ))
            .await;
        let request_key = format!("root:{request_id}");
        let mut state = recorder.state.lock().await;
        let entry = state
            .entries
            .get_mut(&request_key)
            .expect("seeded HAR entry");
        entry.response_body = Some(body.to_vec());
        entry.ts_finished = Some(10.3);
        entry.encoded_data_length = Some(i64::try_from(body.len()).expect("body len"));
        entry.transfer_size = entry.encoded_data_length;
    }

    fn http_request_complete(buffer: &[u8]) -> bool {
        let Some(header_end) = buffer.windows(4).position(|window| window == b"\r\n\r\n") else {
            return false;
        };
        let header_end = header_end + 4;
        let headers = String::from_utf8_lossy(&buffer[..header_end]);
        let content_length = headers
            .lines()
            .find_map(|line| {
                line.split_once(':').and_then(|(name, value)| {
                    name.eq_ignore_ascii_case("content-length")
                        .then(|| value.trim().parse::<usize>().ok())
                        .flatten()
                })
            })
            .unwrap_or(0);
        buffer.len() >= header_end + content_length
    }

    fn request_body(request: &str) -> Value {
        let body = request
            .split("\r\n\r\n")
            .nth(1)
            .expect("request body separator");
        serde_json::from_str(body).expect("request body json")
    }

    fn request_header<'a>(request: &'a str, name: &str) -> Option<&'a str> {
        request.lines().find_map(|line| {
            line.split_once(':').and_then(|(header_name, value)| {
                header_name.eq_ignore_ascii_case(name).then(|| value.trim())
            })
        })
    }

    #[derive(Debug, Clone)]
    struct RecordedCdpCommand {
        method: String,
        params: Value,
        session_id: Option<String>,
    }

    async fn cdp_command_test_server(
        grant_error: Option<&'static str>,
        expected_requests: usize,
    ) -> (
        DevToolsEndpoint,
        tokio::task::JoinHandle<Vec<RecordedCdpCommand>>,
    ) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind cdp test server");
        let addr = listener.local_addr().expect("cdp test server addr");
        let endpoint = DevToolsEndpoint {
            http_url: format!("http://{addr}"),
            websocket_url: format!("ws://{addr}/devtools/browser/test"),
        };
        let handle = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.expect("accept cdp websocket");
            let mut websocket = tokio_tungstenite::accept_async(stream)
                .await
                .expect("accept websocket handshake");
            let mut commands = Vec::new();

            for _ in 0..expected_requests {
                let Some(message) = websocket.next().await else {
                    break;
                };
                let message = message.expect("cdp websocket message");
                let text = match message {
                    Message::Text(text) => text.to_string(),
                    Message::Binary(bytes) => String::from_utf8(bytes.to_vec()).expect("utf8 cdp"),
                    _ => continue,
                };
                let payload: Value = serde_json::from_str(&text).expect("cdp request json");
                let id = payload.get("id").and_then(Value::as_u64).expect("cdp id");
                let method = payload
                    .get("method")
                    .and_then(Value::as_str)
                    .expect("cdp method");
                let params = payload.get("params").cloned().unwrap_or_else(|| json!({}));
                commands.push(RecordedCdpCommand {
                    method: method.to_owned(),
                    params,
                    session_id: payload
                        .get("sessionId")
                        .and_then(Value::as_str)
                        .map(str::to_owned),
                });

                let response = cdp_command_test_response(id, method, grant_error);
                websocket
                    .send(Message::Text(response.to_string().into()))
                    .await
                    .expect("send cdp response");
            }

            commands
        });
        (endpoint, handle)
    }

    #[allow(clippy::result_large_err)]
    async fn cdp_command_header_test_server(
        expected_requests: usize,
    ) -> (
        DevToolsEndpoint,
        tokio::task::JoinHandle<(Vec<RecordedCdpCommand>, BTreeMap<String, String>)>,
    ) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind cdp header test server");
        let addr = listener.local_addr().expect("cdp header test server addr");
        let endpoint = DevToolsEndpoint {
            http_url: format!("http://{addr}"),
            websocket_url: format!("ws://{addr}/devtools/browser/test"),
        };
        let handle = tokio::spawn(async move {
            let (stream, _) = listener
                .accept()
                .await
                .expect("accept cdp header websocket");
            let handshake_headers =
                Arc::new(std::sync::Mutex::new(BTreeMap::<String, String>::new()));
            let captured_headers = handshake_headers.clone();
            let mut websocket = tokio_tungstenite::accept_hdr_async(
                stream,
                move |request: &tokio_tungstenite::tungstenite::handshake::server::Request,
                      response: tokio_tungstenite::tungstenite::handshake::server::Response| {
                    let mut headers = captured_headers
                        .lock()
                        .expect("capture websocket handshake headers");
                    for (name, value) in request.headers() {
                        if let Ok(value) = value.to_str() {
                            headers.insert(name.as_str().to_ascii_lowercase(), value.to_owned());
                        }
                    }
                    Ok(response)
                },
            )
            .await
            .expect("accept websocket handshake");
            let mut commands = Vec::new();

            for _ in 0..expected_requests {
                let Some(message) = websocket.next().await else {
                    break;
                };
                let message = message.expect("cdp websocket message");
                let text = match message {
                    Message::Text(text) => text.to_string(),
                    Message::Binary(bytes) => String::from_utf8(bytes.to_vec()).expect("utf8 cdp"),
                    _ => continue,
                };
                let payload: Value = serde_json::from_str(&text).expect("cdp request json");
                let id = payload.get("id").and_then(Value::as_u64).expect("cdp id");
                let method = payload
                    .get("method")
                    .and_then(Value::as_str)
                    .expect("cdp method");
                let params = payload.get("params").cloned().unwrap_or_else(|| json!({}));
                commands.push(RecordedCdpCommand {
                    method: method.to_owned(),
                    params,
                    session_id: payload
                        .get("sessionId")
                        .and_then(Value::as_str)
                        .map(str::to_owned),
                });

                let response = cdp_command_test_response(id, method, None);
                websocket
                    .send(Message::Text(response.to_string().into()))
                    .await
                    .expect("send cdp response");
            }

            let headers = handshake_headers
                .lock()
                .expect("read websocket handshake headers")
                .clone();
            (commands, headers)
        });
        (endpoint, handle)
    }

    fn cdp_command_test_response(
        id: u64,
        method: &str,
        grant_error: Option<&'static str>,
    ) -> Value {
        if method == "Browser.grantPermissions" {
            if let Some(message) = grant_error {
                return json!({
                    "id": id,
                    "error": {
                        "message": message
                    }
                });
            }
        }

        let result = match method {
            "Target.getTargets" => json!({
                "targetInfos": [{
                    "targetId": "target-1",
                    "type": "page",
                    "url": "about:blank"
                }]
            }),
            "Target.attachToTarget" => json!({
                "sessionId": "session-1"
            }),
            "Browser.grantPermissions"
            | "Browser.close"
            | "Browser.setDownloadBehavior"
            | "Page.screencastFrameAck"
            | "Page.startScreencast"
            | "Page.stopScreencast"
            | "Page.enable"
            | "Network.enable"
            | "Emulation.setDeviceMetricsOverride" => json!({}),
            "Network.getResponseBody" => json!({
                "body": base64::engine::general_purpose::STANDARD.encode(b"%PDF-1.7 cdp body"),
                "base64Encoded": true
            }),
            other => panic!("unexpected CDP method {other}"),
        };
        json!({
            "id": id,
            "result": result
        })
    }

    fn test_png_frame_base64(width: u32, height: u32) -> String {
        let image = image::RgbaImage::from_pixel(width, height, image::Rgba([32, 64, 96, 255]));
        let mut cursor = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(image)
            .write_to(&mut cursor, image::ImageFormat::Png)
            .expect("encode png frame");
        base64::engine::general_purpose::STANDARD.encode(cursor.into_inner())
    }

    fn arg_index(args: &[String], expected: &str) -> usize {
        args.iter()
            .position(|arg| arg == expected)
            .unwrap_or_else(|| panic!("missing launch arg {expected} in {args:?}"))
    }

    #[test]
    fn default_profile_uses_headless_chrome_args() {
        let profile = BrowserProfile::default();
        let plan = profile.launch_plan();

        assert!(plan.args.contains(&"--headless=new".to_owned()));
        assert!(plan.args.contains(&"--remote-debugging-port=0".to_owned()));
        assert!(plan.args.contains(&"--window-size=1280,720".to_owned()));
        assert!(plan.args.contains(&"--window-position=0,0".to_owned()));
        assert!(!profile.devtools);
        assert!(
            !plan
                .args
                .iter()
                .any(|arg| arg == "--auto-open-devtools-for-tabs")
        );
        assert_eq!(profile.window_size, None);
        assert_eq!(
            profile.window_position,
            Some(BrowserViewport {
                width: 0,
                height: 0
            })
        );
        assert!(profile.chromium_sandbox);
        assert!(!profile.devtools);
        assert!(
            !plan
                .args
                .contains(&"--auto-open-devtools-for-tabs".to_owned())
        );
        assert!(
            ![
                "--no-sandbox",
                "--disable-gpu-sandbox",
                "--disable-setuid-sandbox",
                "--no-xshm"
            ]
            .iter()
            .any(|arg| plan.args.iter().any(|plan_arg| plan_arg.as_str() == *arg))
        );
        assert_eq!(profile.profile_directory, "Default");
        assert!(
            !plan
                .args
                .iter()
                .any(|arg| arg.starts_with("--profile-directory="))
        );
        assert_eq!(profile.user_agent, None);
        assert!(!plan.args.iter().any(|arg| arg.starts_with("--user-agent=")));
        assert!(!profile.disable_security);
        assert!(!profile.deterministic_rendering);
        assert!(
            !CHROME_DISABLE_SECURITY_ARGS
                .iter()
                .any(|arg| plan.args.iter().any(|plan_arg| plan_arg.as_str() == *arg))
        );
        assert!(
            !CHROME_DETERMINISTIC_RENDERING_ARGS
                .iter()
                .any(|arg| plan.args.iter().any(|plan_arg| plan_arg.as_str() == *arg))
        );
        assert_eq!(profile.browser_start_timeout_ms, 30_000);
        assert_eq!(profile.navigation_timeout_ms, 20_000);
        assert!(!profile.uses_cloud());
        assert_eq!(profile.cloud_create_request(), None);
    }

    #[test]
    fn default_profile_uses_upstream_browser_permissions() {
        let profile = BrowserProfile::default();
        assert_eq!(
            profile.permissions,
            vec!["clipboardReadWrite".to_owned(), "notifications".to_owned()]
        );

        let deserialized: BrowserProfile =
            serde_json::from_value(json!({})).expect("deserialize default profile");
        assert_eq!(deserialized.permissions, profile.permissions);

        let serialized = serde_json::to_value(&profile).expect("serialize profile");
        assert_eq!(
            serialized["permissions"],
            json!(["clipboardReadWrite", "notifications"])
        );

        let explicit_empty: BrowserProfile =
            serde_json::from_value(json!({ "permissions": [] })).expect("empty permissions");
        assert!(explicit_empty.permissions.is_empty());
    }

    #[test]
    fn browser_profile_iframe_traversal_defaults_match_upstream() {
        let profile = BrowserProfile::default();
        assert!(profile.cross_origin_iframes);
        assert_eq!(profile.max_iframes, 100);
        assert_eq!(profile.max_iframe_depth, 5);

        let deserialized: BrowserProfile =
            serde_json::from_value(json!({})).expect("deserialize default profile");
        assert!(deserialized.cross_origin_iframes);
        assert_eq!(deserialized.max_iframes, 100);
        assert_eq!(deserialized.max_iframe_depth, 5);

        let serialized = serde_json::to_value(&profile).expect("serialize profile");
        assert_eq!(serialized["cross_origin_iframes"], json!(true));
        assert_eq!(serialized["max_iframes"], json!(100));
        assert_eq!(serialized["max_iframe_depth"], json!(5));

        let configured: BrowserProfile = serde_json::from_value(json!({
            "cross_origin_iframes": false,
            "max_iframes": 2,
            "max_iframe_depth": 0
        }))
        .expect("configured profile");
        assert!(!configured.cross_origin_iframes);
        assert_eq!(configured.max_iframes, 2);
        assert_eq!(configured.max_iframe_depth, 0);
    }

    #[test]
    fn browser_profile_paint_order_filtering_defaults_true_in_json() {
        let decoded: BrowserProfile = serde_json::from_value(json!({})).expect("empty profile");
        assert!(decoded.paint_order_filtering);

        let encoded = serde_json::to_value(BrowserProfile::default()).expect("profile json");
        assert_eq!(encoded["paint_order_filtering"], json!(true));

        let disabled: BrowserProfile = serde_json::from_value(json!({
            "paint_order_filtering": false
        }))
        .expect("disabled paint-order filtering profile");
        assert!(!disabled.paint_order_filtering);
        assert_eq!(
            serde_json::to_value(disabled).expect("disabled profile json")["paint_order_filtering"],
            json!(false)
        );
    }

    #[test]
    fn browser_profile_interaction_highlight_defaults_match_upstream() {
        let decoded: BrowserProfile = serde_json::from_value(json!({})).expect("empty profile");
        assert!(decoded.highlight_elements);
        assert!(!decoded.dom_highlight_elements);
        assert!(decoded.filter_highlight_ids);
        assert_eq!(decoded.interaction_highlight_color, "rgb(255, 127, 39)");
        assert_eq!(decoded.interaction_highlight_duration, 1.0);

        let encoded = serde_json::to_value(BrowserProfile::default()).expect("profile json");
        assert_eq!(encoded["highlight_elements"], json!(true));
        assert_eq!(encoded["dom_highlight_elements"], json!(false));
        assert_eq!(encoded["filter_highlight_ids"], json!(true));
        assert_eq!(
            encoded["interaction_highlight_color"],
            json!("rgb(255, 127, 39)")
        );
        assert_eq!(encoded["interaction_highlight_duration"], json!(1.0));

        let disabled: BrowserProfile = serde_json::from_value(json!({
            "highlight_elements": false,
            "dom_highlight_elements": true,
            "filter_highlight_ids": false,
            "interaction_highlight_color": "lime",
            "interaction_highlight_duration": 0.25
        }))
        .expect("highlight profile");
        assert!(!disabled.highlight_elements);
        assert!(disabled.dom_highlight_elements);
        assert!(!disabled.filter_highlight_ids);
        assert_eq!(disabled.interaction_highlight_color, "lime");
        assert_eq!(disabled.interaction_highlight_duration, 0.25);
    }

    #[test]
    fn default_profile_uses_upstream_ignore_default_args_shape() {
        let profile = BrowserProfile::default();
        let IgnoreDefaultArgs::List(ignored_args) = &profile.ignore_default_args else {
            panic!("default ignore_default_args should be a list");
        };
        assert_eq!(ignored_args, &default_ignore_default_args());

        let deserialized: BrowserProfile =
            serde_json::from_value(json!({})).expect("deserialize default profile");
        assert_eq!(
            deserialized.ignore_default_args,
            profile.ignore_default_args
        );

        let serialized = serde_json::to_value(&profile).expect("serialize profile");
        assert_eq!(serialized["ignore_default_args"], json!(ignored_args));

        let ignored_list: BrowserProfile = serde_json::from_value(json!({
            "ignore_default_args": ["--disable-sync"]
        }))
        .expect("ignore list profile");
        assert_eq!(
            ignored_list.ignore_default_args,
            IgnoreDefaultArgs::List(vec!["--disable-sync".to_owned()])
        );

        let ignored_all: BrowserProfile =
            serde_json::from_value(json!({ "ignore_default_args": true }))
                .expect("ignore all profile");
        assert_eq!(
            ignored_all.ignore_default_args,
            IgnoreDefaultArgs::All(true)
        );
    }

    #[test]
    fn browser_profile_env_defaults_and_coerces_upstream_wire_values() {
        let profile = BrowserProfile::default();
        assert!(profile.env.is_empty());

        let deserialized: BrowserProfile =
            serde_json::from_value(json!({})).expect("deserialize default profile");
        assert!(deserialized.env.is_empty());

        let profile_with_env: BrowserProfile = serde_json::from_value(json!({
            "env": {
                "BROWSER_USE_HEADLESS": true,
                "BROWSER_USE_SCALE": 2.5,
                "BROWSER_USE_TOKEN": "secret"
            }
        }))
        .expect("deserialize env profile");
        assert_eq!(
            profile_with_env.env,
            BTreeMap::from([
                ("BROWSER_USE_HEADLESS".to_owned(), "true".to_owned()),
                ("BROWSER_USE_SCALE".to_owned(), "2.5".to_owned()),
                ("BROWSER_USE_TOKEN".to_owned(), "secret".to_owned()),
            ])
        );
        assert_eq!(
            serde_json::to_value(&profile_with_env.env).expect("serialize env"),
            json!({
                "BROWSER_USE_HEADLESS": "true",
                "BROWSER_USE_SCALE": "2.5",
                "BROWSER_USE_TOKEN": "secret"
            })
        );
    }

    #[test]
    fn browser_profile_headers_default_omitted_and_round_trip() {
        let decoded: BrowserProfile = serde_json::from_value(json!({})).expect("empty profile");
        assert_eq!(decoded.headers, None);

        let encoded = serde_json::to_value(BrowserProfile::default()).expect("profile json");
        assert!(encoded.get("headers").is_none());

        let configured: BrowserProfile = serde_json::from_value(json!({
            "headers": {
                "Authorization": "Bearer test-token",
                "X-Browser-Use-Test": "yes"
            }
        }))
        .expect("headers profile");
        assert_eq!(
            configured.headers,
            Some(BTreeMap::from([
                ("Authorization".to_owned(), "Bearer test-token".to_owned()),
                ("X-Browser-Use-Test".to_owned(), "yes".to_owned()),
            ]))
        );
        assert_eq!(
            serde_json::to_value(configured).expect("headers profile json")["headers"],
            json!({
                "Authorization": "Bearer test-token",
                "X-Browser-Use-Test": "yes"
            })
        );
    }

    #[test]
    fn cdp_websocket_request_rejects_invalid_profile_headers() {
        let invalid_name = BTreeMap::from([("Bad Header".to_owned(), "value".to_owned())]);
        let error = cdp_websocket_request("ws://127.0.0.1/devtools/browser/test", &invalid_name)
            .expect_err("invalid header name");
        assert!(
            error
                .to_string()
                .contains("invalid CDP websocket header name")
        );

        let invalid_value = BTreeMap::from([("X-Test".to_owned(), "bad\nvalue".to_owned())]);
        let error = cdp_websocket_request("ws://127.0.0.1/devtools/browser/test", &invalid_value)
            .expect_err("invalid header value");
        assert!(
            error
                .to_string()
                .contains("invalid CDP websocket header value")
        );
    }

    #[test]
    fn browser_permission_grant_params_skip_empty_lists() {
        assert_eq!(browser_permission_grant_params(&[]), None);

        let permissions = vec!["clipboardReadWrite".to_owned(), "notifications".to_owned()];
        assert_eq!(
            browser_permission_grant_params(&permissions),
            Some(json!({
                "permissions": ["clipboardReadWrite", "notifications"]
            }))
        );
    }

    #[test]
    fn permission_grant_failure_lifecycle_event_is_inspectable() {
        let permissions = vec!["clipboardReadWrite".to_owned(), "notifications".to_owned()];
        let event = BrowserLifecycleEvent::permissions_grant_failed(
            &permissions,
            "Browser denied permission grant",
        );

        assert_eq!(event.kind, BrowserLifecycleEventKind::BrowserDiagnostic);
        assert_eq!(event.reason.as_deref(), Some("permissions_grant_failed"));
        assert_eq!(
            event.details.get("permissions").map(String::as_str),
            Some("clipboardReadWrite,notifications")
        );
        assert_eq!(
            event.details.get("permissions_count").map(String::as_str),
            Some("2")
        );
        assert_eq!(
            event.error.as_deref(),
            Some("Browser denied permission grant")
        );
        assert_eq!(
            BrowserLifecycleAdapterEvent::from_lifecycle_event(&event).kind,
            BrowserLifecycleAdapterEventKind::BrowserDiagnostic
        );
    }

    #[tokio::test]
    async fn direct_connect_grants_default_permissions_before_target_attach() {
        let (endpoint, command_log) = cdp_command_test_server(None, 7).await;
        let session = CdpBrowserSession::connect(endpoint)
            .await
            .expect("connect session");

        let commands = command_log.await.expect("cdp command log");
        assert_eq!(
            commands
                .iter()
                .map(|command| command.method.as_str())
                .collect::<Vec<_>>(),
            vec![
                "Browser.grantPermissions",
                "Browser.setDownloadBehavior",
                "Target.getTargets",
                "Target.attachToTarget",
                "Page.enable",
                "Network.enable",
                "Emulation.setDeviceMetricsOverride",
            ]
        );
        assert_eq!(
            commands[0].params,
            json!({
                "permissions": ["clipboardReadWrite", "notifications"]
            })
        );
        assert_eq!(commands[1].params["behavior"], "allow");
        assert_eq!(commands[1].params["eventsEnabled"], true);
        assert!(
            commands[1].params["downloadPath"]
                .as_str()
                .is_some_and(|path| path.contains("browser-use-downloads-"))
        );
        assert!(
            session
                .downloads_path
                .as_ref()
                .is_some_and(|path| path.exists())
        );
        assert!(session._downloads_dir.is_some());
        assert_eq!(commands[4].session_id.as_deref(), Some("session-1"));
        assert_eq!(commands[5].session_id.as_deref(), Some("session-1"));
        assert_eq!(commands[6].session_id.as_deref(), Some("session-1"));
        assert_eq!(
            commands[6].params,
            json!({
                "width": 1280,
                "height": 720,
                "deviceScaleFactor": 1.0,
                "mobile": false
            })
        );

        let lifecycle_events = session.lifecycle_events().await;
        assert_eq!(
            lifecycle_events
                .iter()
                .filter(|event| event.reason.as_deref() == Some("permissions_grant_failed"))
                .count(),
            0
        );
    }

    #[tokio::test]
    async fn direct_connect_skips_empty_permissions_before_target_attach() {
        let (endpoint, command_log) = cdp_command_test_server(None, 5).await;
        let profile = BrowserProfile {
            permissions: Vec::new(),
            accept_downloads: false,
            ..BrowserProfile::default()
        };
        let _session = CdpBrowserSession::connect_with_profile(endpoint, &profile)
            .await
            .expect("connect session");

        let commands = command_log.await.expect("cdp command log");
        assert_eq!(
            commands
                .iter()
                .map(|command| command.method.as_str())
                .collect::<Vec<_>>(),
            vec![
                "Target.getTargets",
                "Target.attachToTarget",
                "Page.enable",
                "Network.enable",
                "Emulation.setDeviceMetricsOverride",
            ]
        );
    }

    #[tokio::test]
    async fn direct_connect_keeps_download_behavior_when_auto_pdf_disabled() {
        let downloads_dir = TempDir::new().expect("downloads temp dir");
        let (endpoint, command_log) = cdp_command_test_server(None, 6).await;
        let profile = BrowserProfile {
            permissions: Vec::new(),
            downloads_path: Some(downloads_dir.path().to_path_buf()),
            auto_download_pdfs: false,
            ..BrowserProfile::default()
        };
        let _session = CdpBrowserSession::connect_with_profile(endpoint, &profile)
            .await
            .expect("connect session");

        let commands = command_log.await.expect("cdp command log");
        assert_eq!(commands[0].method, "Browser.setDownloadBehavior");
        assert_eq!(
            commands[0].params,
            json!({
                "behavior": "allow",
                "downloadPath": downloads_dir.path().display().to_string(),
                "eventsEnabled": true
            })
        );
        assert!(
            commands
                .iter()
                .any(|command| command.method == "Network.enable")
        );
    }

    #[tokio::test]
    async fn direct_connect_uses_downloads_path_alias_for_download_behavior() {
        let downloads_dir = TempDir::new().expect("downloads temp dir");
        let (endpoint, command_log) = cdp_command_test_server(None, 6).await;
        let profile: BrowserProfile = serde_json::from_value(json!({
            "permissions": [],
            "save_downloads_path": downloads_dir.path().display().to_string()
        }))
        .expect("deserialize alias profile");
        let session = CdpBrowserSession::connect_with_profile(endpoint, &profile)
            .await
            .expect("connect session");

        assert_eq!(
            session.downloads_path.as_deref(),
            Some(downloads_dir.path())
        );

        let commands = command_log.await.expect("cdp command log");
        assert_eq!(commands[0].method, "Browser.setDownloadBehavior");
        assert_eq!(
            commands[0].params,
            json!({
                "behavior": "allow",
                "downloadPath": downloads_dir.path().display().to_string(),
                "eventsEnabled": true
            })
        );
    }

    #[tokio::test]
    async fn direct_connect_generates_session_owned_downloads_path_when_accepted() {
        let (endpoint, command_log) = cdp_command_test_server(None, 6).await;
        let profile = BrowserProfile {
            permissions: Vec::new(),
            ..BrowserProfile::default()
        };
        let session = CdpBrowserSession::connect_with_profile(endpoint, &profile)
            .await
            .expect("connect session");

        let downloads_path = session
            .downloads_path
            .clone()
            .expect("generated downloads path");
        assert!(downloads_path.exists());
        assert!(session._downloads_dir.is_some());

        let commands = command_log.await.expect("cdp command log");
        assert_eq!(commands[0].method, "Browser.setDownloadBehavior");
        assert_eq!(
            commands[0].params,
            json!({
                "behavior": "allow",
                "downloadPath": downloads_path.display().to_string(),
                "eventsEnabled": true
            })
        );

        drop(session);
        assert!(!downloads_path.exists());
    }

    #[tokio::test]
    async fn direct_connect_accept_downloads_false_disables_download_path() {
        let downloads_dir = TempDir::new().expect("downloads temp dir");
        let (endpoint, command_log) = cdp_command_test_server(None, 5).await;
        let profile = BrowserProfile {
            permissions: Vec::new(),
            downloads_path: Some(downloads_dir.path().to_path_buf()),
            accept_downloads: false,
            ..BrowserProfile::default()
        };
        let session = CdpBrowserSession::connect_with_profile(endpoint, &profile)
            .await
            .expect("connect session");

        let commands = command_log.await.expect("cdp command log");
        assert!(
            !commands
                .iter()
                .any(|command| command.method == "Browser.setDownloadBehavior")
        );
        assert!(session.downloads_path.is_none());
        assert!(session._downloads_dir.is_none());

        session
            .auto_download_pdf_if_needed("https://example.test/report.pdf")
            .await;
        assert!(
            std::fs::read_dir(downloads_dir.path())
                .expect("downloads dir entries")
                .next()
                .is_none()
        );
        assert!(
            session
                .lifecycle_events()
                .await
                .iter()
                .all(|event| event.reason.as_deref() != Some("pdf_auto_download"))
        );
    }

    #[tokio::test]
    async fn direct_connect_sends_profile_headers_in_websocket_handshake() {
        let (endpoint, command_log) = cdp_command_header_test_server(5).await;
        let profile = BrowserProfile {
            headers: Some(BTreeMap::from([
                ("Authorization".to_owned(), "Bearer cdp-token".to_owned()),
                ("X-Browser-Use-Test".to_owned(), "handshake".to_owned()),
            ])),
            permissions: Vec::new(),
            accept_downloads: false,
            ..BrowserProfile::default()
        };
        let _session = CdpBrowserSession::connect_with_profile(endpoint, &profile)
            .await
            .expect("connect with profile headers");

        let (commands, headers) = command_log.await.expect("cdp command log");
        assert_eq!(
            headers.get("authorization").map(String::as_str),
            Some("Bearer cdp-token")
        );
        assert_eq!(
            headers.get("x-browser-use-test").map(String::as_str),
            Some("handshake")
        );
        assert_eq!(
            commands
                .iter()
                .map(|command| command.method.as_str())
                .collect::<Vec<_>>(),
            vec![
                "Target.getTargets",
                "Target.attachToTarget",
                "Page.enable",
                "Network.enable",
                "Emulation.setDeviceMetricsOverride",
            ]
        );
    }

    #[tokio::test]
    async fn direct_connect_applies_configured_viewport_emulation() {
        let (endpoint, command_log) = cdp_command_test_server(None, 5).await;
        let profile = BrowserProfile {
            permissions: Vec::new(),
            accept_downloads: false,
            viewport: BrowserViewport {
                width: 1024,
                height: 768,
            },
            device_scale_factor: Some(2.5),
            ..BrowserProfile::default()
        };
        let _session = CdpBrowserSession::connect_with_profile(endpoint, &profile)
            .await
            .expect("connect session");

        let commands = command_log.await.expect("cdp command log");
        let command = commands
            .iter()
            .find(|command| command.method == "Emulation.setDeviceMetricsOverride")
            .expect("viewport emulation command");
        assert_eq!(command.session_id.as_deref(), Some("session-1"));
        assert_eq!(
            command.params,
            json!({
                "width": 1024,
                "height": 768,
                "deviceScaleFactor": 2.5,
                "mobile": false
            })
        );
    }

    #[tokio::test]
    async fn direct_connect_skips_viewport_emulation_when_no_viewport() {
        let (endpoint, command_log) = cdp_command_test_server(None, 5).await;
        let profile = BrowserProfile {
            no_viewport: true,
            accept_downloads: false,
            ..BrowserProfile::default()
        };
        let _session = CdpBrowserSession::connect_with_profile(endpoint, &profile)
            .await
            .expect("connect session");

        let commands = command_log.await.expect("cdp command log");
        assert!(
            !commands
                .iter()
                .any(|command| command.method == "Emulation.setDeviceMetricsOverride")
        );
    }

    #[tokio::test]
    async fn direct_connect_records_permission_grant_failures_without_failing() {
        let (endpoint, command_log) =
            cdp_command_test_server(Some("permission grant denied"), 7).await;
        let session = CdpBrowserSession::connect(endpoint)
            .await
            .expect("connect session despite grant failure");

        let commands = command_log.await.expect("cdp command log");
        assert_eq!(commands[0].method, "Browser.grantPermissions");
        assert_eq!(commands[1].method, "Browser.setDownloadBehavior");
        assert_eq!(commands[2].method, "Target.getTargets");

        let lifecycle_events = session.lifecycle_events().await;
        let event = lifecycle_events
            .iter()
            .find(|event| event.reason.as_deref() == Some("permissions_grant_failed"))
            .expect("permission grant diagnostic");
        assert_eq!(event.kind, BrowserLifecycleEventKind::BrowserDiagnostic);
        assert!(event.error.as_deref().is_some_and(|error| {
            error.contains("Browser.grantPermissions") && error.contains("permission grant denied")
        }));
    }

    #[test]
    fn browser_profile_security_toggles_default_false_in_json() {
        let decoded: BrowserProfile = serde_json::from_value(json!({})).expect("empty profile");
        assert!(!decoded.devtools);
        assert!(!decoded.disable_security);
        assert!(!decoded.deterministic_rendering);

        let encoded = serde_json::to_value(BrowserProfile::default()).expect("profile json");
        assert_eq!(encoded["devtools"], json!(false));
        assert_eq!(encoded["disable_security"], json!(false));
        assert_eq!(encoded["deterministic_rendering"], json!(false));
    }

    #[test]
    fn browser_profile_user_agent_defaults_to_none_in_json() {
        let decoded: BrowserProfile = serde_json::from_value(json!({})).expect("empty profile");
        assert_eq!(decoded.user_agent, None);

        let encoded = serde_json::to_value(BrowserProfile::default()).expect("profile json");
        assert!(encoded.get("user_agent").is_none());
    }

    #[test]
    fn browser_profile_channel_defaults_to_none_in_json() {
        let decoded: BrowserProfile = serde_json::from_value(json!({})).expect("empty profile");
        assert_eq!(decoded.channel, None);

        let encoded = serde_json::to_value(BrowserProfile::default()).expect("profile json");
        assert!(encoded.get("channel").is_none());

        let configured: BrowserProfile =
            serde_json::from_value(json!({ "channel": "chrome-beta" })).expect("channel profile");
        assert_eq!(configured.channel, Some(BrowserChannel::ChromeBeta));
        let configured_json = serde_json::to_value(&configured).expect("configured profile json");
        assert_eq!(configured_json["channel"], json!("chrome-beta"));
    }

    #[test]
    fn browser_profile_executable_path_aliases_match_upstream() {
        let canonical_path = "/tmp/browser-use-rs-chrome";
        let canonical: BrowserProfile = serde_json::from_value(json!({
            "executable_path": canonical_path
        }))
        .expect("canonical executable path profile");
        assert_eq!(
            canonical.executable_path.as_deref(),
            Some(Path::new(canonical_path))
        );

        let browser_binary_path = "/tmp/browser-use-rs-browser-binary";
        let browser_binary: BrowserProfile = serde_json::from_value(json!({
            "browser_binary_path": browser_binary_path
        }))
        .expect("browser_binary_path alias profile");
        assert_eq!(
            browser_binary.executable_path.as_deref(),
            Some(Path::new(browser_binary_path))
        );

        let chrome_binary_path = "/tmp/browser-use-rs-chrome-binary";
        let chrome_binary: BrowserProfile = serde_json::from_value(json!({
            "chrome_binary_path": chrome_binary_path
        }))
        .expect("chrome_binary_path alias profile");
        assert_eq!(
            chrome_binary.executable_path.as_deref(),
            Some(Path::new(chrome_binary_path))
        );

        let encoded = serde_json::to_value(chrome_binary).expect("canonical profile json");
        assert_eq!(encoded["executable_path"], json!(chrome_binary_path));
        assert!(encoded.get("browser_binary_path").is_none());
        assert!(encoded.get("chrome_binary_path").is_none());
    }

    #[test]
    fn browser_profile_profile_directory_defaults_in_json() {
        let decoded: BrowserProfile = serde_json::from_value(json!({})).expect("empty profile");
        assert_eq!(decoded.profile_directory, "Default");

        let encoded = serde_json::to_value(BrowserProfile::default()).expect("profile json");
        assert_eq!(encoded["profile_directory"], json!("Default"));
    }

    #[test]
    fn browser_profile_chromium_sandbox_defaults_true_in_json() {
        let decoded: BrowserProfile = serde_json::from_value(json!({})).expect("empty profile");
        assert!(decoded.chromium_sandbox);

        let encoded = serde_json::to_value(BrowserProfile::default()).expect("profile json");
        assert_eq!(encoded["chromium_sandbox"], json!(true));
    }

    #[test]
    fn browser_profile_devtools_defaults_false_in_json() {
        let decoded: BrowserProfile = serde_json::from_value(json!({})).expect("empty profile");
        assert!(!decoded.devtools);

        let encoded = serde_json::to_value(BrowserProfile::default()).expect("profile json");
        assert_eq!(encoded["devtools"], json!(false));
    }

    #[test]
    fn browser_profile_window_geometry_matches_upstream_defaults_in_json() {
        let decoded: BrowserProfile = serde_json::from_value(json!({})).expect("empty profile");
        assert_eq!(decoded.window_size, None);
        assert_eq!(
            decoded.window_position,
            Some(BrowserViewport {
                width: 0,
                height: 0
            })
        );

        let encoded = serde_json::to_value(BrowserProfile::default()).expect("profile json");
        assert!(encoded.get("window_size").is_none());
        assert_eq!(encoded["window_position"], json!({"width": 0, "height": 0}));
    }

    #[test]
    fn browser_profile_viewport_emulation_defaults_and_validation() {
        let decoded: BrowserProfile = serde_json::from_value(json!({})).expect("empty profile");
        assert_eq!(decoded.screen, None);
        assert_eq!(decoded.viewport, BrowserViewport::default());
        assert!(!decoded.no_viewport);
        assert_eq!(decoded.device_scale_factor, None);

        let encoded = serde_json::to_value(BrowserProfile::default()).expect("profile json");
        assert!(encoded.get("screen").is_none());
        assert_eq!(encoded["viewport"], json!({ "width": 1280, "height": 720 }));
        assert!(encoded.get("no_viewport").is_none());
        assert!(encoded.get("device_scale_factor").is_none());

        let configured: BrowserProfile = serde_json::from_value(json!({
            "screen": { "width": 1920, "height": 1080 },
            "viewport": { "width": 1024, "height": 768 },
            "no_viewport": true,
            "device_scale_factor": 2.5
        }))
        .expect("configured viewport profile");
        assert_eq!(
            configured.screen,
            Some(BrowserViewport {
                width: 1920,
                height: 1080
            })
        );
        assert_eq!(
            configured.viewport,
            BrowserViewport {
                width: 1024,
                height: 768
            }
        );
        assert!(configured.no_viewport);
        assert_eq!(configured.device_scale_factor, Some(2.5));

        let negative = serde_json::from_value::<BrowserProfile>(json!({
            "device_scale_factor": -1.0
        }))
        .expect_err("negative device_scale_factor should be rejected");
        assert!(negative.to_string().contains("device_scale_factor"));
    }

    #[test]
    fn browser_profile_keep_alive_preserves_upstream_wire_shape() {
        let decoded: BrowserProfile = serde_json::from_value(json!({})).expect("empty profile");
        assert_eq!(decoded.keep_alive, None);

        let encoded = serde_json::to_value(BrowserProfile::default()).expect("profile json");
        assert!(encoded.get("keep_alive").is_none());

        let keep_alive: BrowserProfile = serde_json::from_value(json!({
            "keep_alive": true
        }))
        .expect("keep alive profile");
        assert_eq!(keep_alive.keep_alive, Some(true));
        assert!(profile_keeps_launched_browser_alive(&keep_alive));

        let close_on_drop: BrowserProfile = serde_json::from_value(json!({
            "keep_alive": false
        }))
        .expect("close on drop profile");
        assert_eq!(close_on_drop.keep_alive, Some(false));
        assert!(!profile_keeps_launched_browser_alive(&close_on_drop));

        let null_keep_alive: BrowserProfile = serde_json::from_value(json!({
            "keep_alive": null
        }))
        .expect("null keep alive profile");
        assert_eq!(null_keep_alive.keep_alive, None);
        assert!(!profile_keeps_launched_browser_alive(&null_keep_alive));
    }

    #[test]
    fn browser_profile_auto_download_pdfs_defaults_true_in_json() {
        let decoded: BrowserProfile = serde_json::from_value(json!({})).expect("empty profile");
        assert!(decoded.auto_download_pdfs);

        let encoded = serde_json::to_value(BrowserProfile::default()).expect("profile json");
        assert_eq!(encoded["auto_download_pdfs"], json!(true));

        let disabled: BrowserProfile = serde_json::from_value(json!({
            "auto_download_pdfs": false
        }))
        .expect("disabled auto PDF profile");
        assert!(!disabled.auto_download_pdfs);
        assert_eq!(
            serde_json::to_value(disabled).expect("disabled profile json")["auto_download_pdfs"],
            json!(false)
        );
    }

    #[test]
    fn browser_profile_accept_downloads_defaults_true_in_json() {
        let decoded: BrowserProfile = serde_json::from_value(json!({})).expect("empty profile");
        assert!(decoded.accept_downloads);

        let encoded = serde_json::to_value(BrowserProfile::default()).expect("profile json");
        assert_eq!(encoded["accept_downloads"], json!(true));

        let disabled: BrowserProfile = serde_json::from_value(json!({
            "accept_downloads": false,
            "downloads_path": "/tmp/browser-use-rs-disabled-downloads"
        }))
        .expect("disabled downloads profile");
        assert!(!disabled.accept_downloads);
        assert_eq!(
            serde_json::to_value(disabled).expect("disabled profile json")["accept_downloads"],
            json!(false)
        );
    }

    #[test]
    fn browser_profile_downloads_path_aliases_match_upstream() {
        let canonical_path = "/tmp/browser-use-rs-downloads";
        let canonical: BrowserProfile = serde_json::from_value(json!({
            "downloads_path": canonical_path
        }))
        .expect("canonical downloads path profile");
        assert_eq!(
            canonical.downloads_path.as_deref(),
            Some(Path::new(canonical_path))
        );

        let downloads_dir_path = "/tmp/browser-use-rs-downloads-dir";
        let downloads_dir: BrowserProfile = serde_json::from_value(json!({
            "downloads_dir": downloads_dir_path
        }))
        .expect("downloads_dir alias profile");
        assert_eq!(
            downloads_dir.downloads_path.as_deref(),
            Some(Path::new(downloads_dir_path))
        );

        let save_downloads_path = "/tmp/browser-use-rs-save-downloads-path";
        let save_downloads: BrowserProfile = serde_json::from_value(json!({
            "save_downloads_path": save_downloads_path
        }))
        .expect("save_downloads_path alias profile");
        assert_eq!(
            save_downloads.downloads_path.as_deref(),
            Some(Path::new(save_downloads_path))
        );

        let encoded = serde_json::to_value(save_downloads).expect("canonical profile json");
        assert_eq!(encoded["downloads_path"], json!(save_downloads_path));
        assert!(encoded.get("downloads_dir").is_none());
        assert!(encoded.get("save_downloads_path").is_none());
    }

    #[test]
    fn browser_profile_har_recording_defaults_and_alias_match_upstream() {
        let decoded: BrowserProfile = serde_json::from_value(json!({})).expect("empty profile");
        assert_eq!(decoded.record_har_content, RecordHarContent::Embed);
        assert_eq!(decoded.record_har_mode, RecordHarMode::Full);
        assert_eq!(decoded.record_har_path, None);

        let encoded = serde_json::to_value(BrowserProfile::default()).expect("profile json");
        assert_eq!(encoded["record_har_content"], json!("embed"));
        assert_eq!(encoded["record_har_mode"], json!("full"));
        assert!(encoded.get("record_har_path").is_none());

        let save_har_path = "/tmp/browser-use-rs/session.har";
        let alias: BrowserProfile = serde_json::from_value(json!({
            "save_har_path": save_har_path,
            "record_har_content": "attach",
            "record_har_mode": "minimal"
        }))
        .expect("HAR alias profile");
        assert_eq!(alias.record_har_content, RecordHarContent::Attach);
        assert_eq!(alias.record_har_mode, RecordHarMode::Minimal);
        assert_eq!(
            alias.record_har_path.as_deref(),
            Some(Path::new(save_har_path))
        );

        let encoded = serde_json::to_value(alias).expect("canonical HAR profile json");
        assert_eq!(encoded["record_har_path"], json!(save_har_path));
        assert!(encoded.get("save_har_path").is_none());
    }

    #[test]
    fn browser_profile_video_recording_config_matches_upstream_shape() {
        let decoded: BrowserProfile = serde_json::from_value(json!({})).expect("empty profile");
        assert_eq!(decoded.record_video_dir, None);
        assert_eq!(decoded.record_video_size, None);
        assert_eq!(decoded.record_video_framerate, 30);
        assert_eq!(decoded.record_video_format, VideoRecordingFormat::Mp4);

        let encoded = serde_json::to_value(BrowserProfile::default()).expect("profile json");
        assert!(encoded.get("record_video_dir").is_none());
        assert!(encoded.get("record_video_size").is_none());
        assert_eq!(encoded["record_video_framerate"], json!(30));
        assert!(encoded.get("record_video_format").is_none());

        let save_recording_path = "/tmp/browser-use-rs/videos";
        let configured: BrowserProfile = serde_json::from_value(json!({
            "save_recording_path": save_recording_path,
            "record_video_size": {
                "width": 1024,
                "height": 768
            },
            "record_video_framerate": 24,
            "record_video_format": ".webm"
        }))
        .expect("video recording profile");
        assert_eq!(
            configured.record_video_dir.as_deref(),
            Some(Path::new(save_recording_path))
        );
        assert_eq!(
            configured.record_video_size,
            Some(BrowserViewport {
                width: 1024,
                height: 768
            })
        );
        assert_eq!(configured.record_video_framerate, 24);
        assert_eq!(configured.record_video_format, VideoRecordingFormat::Webm);

        let encoded = serde_json::to_value(&configured).expect("canonical video profile json");
        assert_eq!(encoded["record_video_dir"], json!(save_recording_path));
        assert!(encoded.get("save_recording_path").is_none());
        assert_eq!(
            encoded["record_video_size"],
            json!({ "width": 1024, "height": 768 })
        );
        assert_eq!(encoded["record_video_framerate"], json!(24));
        assert_eq!(encoded["record_video_format"], json!("webm"));
        assert_eq!(
            configured.launch_plan().args,
            BrowserProfile::default().launch_plan().args
        );
    }

    fn video_recorder_for_test(
        dir: PathBuf,
        format: VideoRecordingFormat,
        ffmpeg_path: impl Into<PathBuf>,
    ) -> CdpVideoRecorder {
        CdpVideoRecorder {
            dir,
            size: BrowserViewport {
                width: 2,
                height: 2,
            },
            framerate: 12,
            format,
            ffmpeg_path: ffmpeg_path.into(),
            state: Mutex::new(CdpVideoState::default()),
        }
    }

    #[test]
    fn video_recording_format_selection_uses_video_extensions() {
        let temp_dir = TempDir::new().expect("video temp dir");
        let recorder = video_recorder_for_test(
            temp_dir.path().to_path_buf(),
            VideoRecordingFormat::Mp4,
            "ffmpeg",
        );

        let mp4 = recorder.artifact_path(42, 0, VideoRecordingFormat::Mp4);
        assert_eq!(
            mp4.extension().and_then(|extension| extension.to_str()),
            Some("mp4")
        );
        assert!(
            mp4.file_name()
                .and_then(|name| name.to_str())
                .expect("mp4 file name")
                .starts_with("browser-use-rs-video-42-")
        );

        let webm = recorder.artifact_path(42, 1, VideoRecordingFormat::Webm);
        assert_eq!(
            webm.extension().and_then(|extension| extension.to_str()),
            Some("webm")
        );
        assert!(
            webm.file_name()
                .and_then(|name| name.to_str())
                .expect("webm file name")
                .contains("-1.webm")
        );
    }

    #[tokio::test]
    async fn video_recording_encoder_failure_falls_back_to_gif_and_reports_diagnostic() {
        let temp_dir = TempDir::new().expect("video temp dir");
        let recorder = video_recorder_for_test(
            temp_dir.path().to_path_buf(),
            VideoRecordingFormat::Mp4,
            "__browser_use_rs_missing_ffmpeg__",
        );

        let (path, encoder_error) = recorder
            .write_recording_artifact(123, &[test_png_frame_base64(2, 2)])
            .await
            .expect("fallback video artifact");
        assert_eq!(
            path.extension().and_then(|extension| extension.to_str()),
            Some("gif")
        );
        assert!(
            std::fs::read(&path)
                .expect("fallback gif")
                .starts_with(b"GIF")
        );
        let encoder_error = encoder_error.expect("mp4 encoder error");
        assert!(
            encoder_error
                .to_string()
                .contains("ffmpeg video encoder unavailable")
        );

        let event = video_recording_failed_event("encode", &encoder_error);
        assert_eq!(event.kind, BrowserLifecycleEventKind::BrowserDiagnostic);
        assert_eq!(event.reason.as_deref(), Some("video_recording_failed"));
        assert_eq!(
            event.details.get("phase").map(String::as_str),
            Some("encode")
        );
        assert!(event.error.expect("diagnostic error").contains("ffmpeg"));
    }

    fn command_available(command: &str) -> bool {
        std::process::Command::new(command)
            .arg("-version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|status| status.success())
    }

    fn assert_ffprobe_reads_video(path: &Path) {
        let output = std::process::Command::new("ffprobe")
            .arg("-v")
            .arg("error")
            .arg("-select_streams")
            .arg("v:0")
            .arg("-show_entries")
            .arg("stream=codec_type,width,height")
            .arg("-of")
            .arg("default=noprint_wrappers=1")
            .arg(path)
            .output()
            .expect("run ffprobe");
        assert!(
            output.status.success(),
            "ffprobe failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            stdout.contains("codec_type=video"),
            "ffprobe output: {stdout}"
        );
        assert!(stdout.contains("width="), "ffprobe output: {stdout}");
        assert!(stdout.contains("height="), "ffprobe output: {stdout}");
    }

    #[tokio::test]
    async fn video_recording_writes_mp4_with_ffmpeg_when_available() {
        if !command_available("ffmpeg") || !command_available("ffprobe") {
            eprintln!("skipping mp4 encoder smoke; ffmpeg/ffprobe unavailable");
            return;
        }

        let temp_dir = TempDir::new().expect("video temp dir");
        let recorder = video_recorder_for_test(
            temp_dir.path().to_path_buf(),
            VideoRecordingFormat::Mp4,
            "ffmpeg",
        );
        let (path, encoder_error) = recorder
            .write_recording_artifact(456, &[test_png_frame_base64(2, 2)])
            .await
            .expect("mp4 video artifact");
        assert!(encoder_error.is_none());
        assert_eq!(
            path.extension().and_then(|extension| extension.to_str()),
            Some("mp4")
        );
        assert!(std::fs::metadata(&path).expect("mp4 metadata").len() > 0);
        assert_ffprobe_reads_video(&path);
    }

    #[tokio::test]
    async fn video_recording_writes_webm_or_falls_back_when_encoder_is_unavailable() {
        if !command_available("ffmpeg") || !command_available("ffprobe") {
            eprintln!("skipping webm encoder smoke; ffmpeg/ffprobe unavailable");
            return;
        }

        let temp_dir = TempDir::new().expect("video temp dir");
        let recorder = video_recorder_for_test(
            temp_dir.path().to_path_buf(),
            VideoRecordingFormat::Webm,
            "ffmpeg",
        );
        let (path, encoder_error) = recorder
            .write_recording_artifact(789, &[test_png_frame_base64(2, 2)])
            .await
            .expect("webm or fallback video artifact");

        if let Some(error) = encoder_error {
            eprintln!("webm encoder unavailable, verified fallback path: {error}");
            assert_eq!(
                path.extension().and_then(|extension| extension.to_str()),
                Some("gif")
            );
            assert!(
                std::fs::read(&path)
                    .expect("fallback gif")
                    .starts_with(b"GIF")
            );
            return;
        }

        assert_eq!(
            path.extension().and_then(|extension| extension.to_str()),
            Some("webm")
        );
        assert!(std::fs::metadata(&path).expect("webm metadata").len() > 0);
        assert_ffprobe_reads_video(&path);
    }

    #[tokio::test]
    async fn video_recording_captures_screencast_frames_and_writes_gif() {
        let temp_dir = TempDir::new().expect("video temp dir");
        let video_dir = temp_dir.path().join("videos");
        let profile = BrowserProfile {
            record_video_dir: Some(video_dir.clone()),
            record_video_size: Some(BrowserViewport {
                width: 2,
                height: 2,
            }),
            record_video_framerate: 12,
            record_video_format: VideoRecordingFormat::Gif,
            ..BrowserProfile::default()
        };
        let (endpoint, command_log) = cdp_command_test_server(None, 11).await;
        let session = CdpBrowserSession::connect_with_profile(endpoint, &profile)
            .await
            .expect("connect video session");

        session
            .connection
            .event_tx
            .send(CdpEvent {
                method: "Page.screencastFrame".to_owned(),
                params: json!({
                    "data": test_png_frame_base64(2, 2),
                    "sessionId": 7
                }),
                session_id: Some("session-1".to_owned()),
            })
            .expect("send screencast frame");
        sleep(Duration::from_millis(20)).await;

        session.close_browser().await.expect("close video browser");
        let commands = command_log.await.expect("cdp command log");
        assert_eq!(
            commands
                .iter()
                .map(|command| command.method.as_str())
                .collect::<Vec<_>>(),
            vec![
                "Browser.grantPermissions",
                "Browser.setDownloadBehavior",
                "Target.getTargets",
                "Target.attachToTarget",
                "Page.enable",
                "Network.enable",
                "Emulation.setDeviceMetricsOverride",
                "Page.startScreencast",
                "Page.screencastFrameAck",
                "Page.stopScreencast",
                "Browser.close"
            ]
        );
        let start = commands
            .iter()
            .find(|command| command.method == "Page.startScreencast")
            .expect("start screencast command");
        assert_eq!(start.session_id.as_deref(), Some("session-1"));
        assert_eq!(start.params["format"], json!("png"));
        assert_eq!(start.params["maxWidth"], json!(2));
        assert_eq!(start.params["maxHeight"], json!(2));
        let ack = commands
            .iter()
            .find(|command| command.method == "Page.screencastFrameAck")
            .expect("screencast frame ack");
        assert_eq!(ack.session_id.as_deref(), Some("session-1"));
        assert_eq!(ack.params["sessionId"], json!(7));

        let video_path = std::fs::read_dir(&video_dir)
            .expect("video dir entries")
            .map(|entry| entry.expect("video dir entry").path())
            .find(|path| path.extension().and_then(|extension| extension.to_str()) == Some("gif"))
            .expect("gif video artifact");
        let bytes = tokio::fs::read(&video_path)
            .await
            .expect("read video artifact");
        assert!(bytes.starts_with(b"GIF"));

        let lifecycle_json =
            serde_json::to_string(&session.lifecycle_events().await).expect("lifecycle json");
        assert!(!lifecycle_json.contains("browser-use-rs-video"));
        assert!(!lifecycle_json.contains(video_dir.to_str().expect("video dir utf8")));
    }

    #[tokio::test]
    #[ignore = "requires Chrome/Chromium plus ffmpeg and ffprobe on the local machine"]
    async fn cdp_session_record_video_dir_writes_playable_mp4_with_ffmpeg() {
        if !command_available("ffmpeg") || !command_available("ffprobe") {
            eprintln!("skipping browser-backed mp4 smoke; ffmpeg/ffprobe unavailable");
            return;
        }

        let temp_dir = TempDir::new().expect("video temp dir");
        let video_dir = temp_dir.path().join("videos");
        let profile = BrowserProfile {
            record_video_dir: Some(video_dir.clone()),
            record_video_size: Some(BrowserViewport {
                width: 320,
                height: 240,
            }),
            record_video_framerate: 8,
            record_video_format: VideoRecordingFormat::Mp4,
            ..BrowserProfile::default()
        };
        let session = CdpBrowserSession::launch(&profile)
            .await
            .expect("launch video smoke browser");

        session
            .navigate(
                "data:text/html,<html><head><title>video smoke</title></head><body><main id='box' style='width:320px;height:240px;background:#0b7285;color:white;font-size:32px'>Frame</main><script>let i=0;setInterval(()=>{i++;document.getElementById('box').textContent='Frame '+i;document.body.style.background=i%2?'#0b7285':'#c92a2a';},80);</script></body></html>",
                false,
            )
            .await
            .expect("navigate video smoke");
        sleep(Duration::from_millis(700)).await;
        session
            .close_browser()
            .await
            .expect("close video smoke browser");

        let videos = std::fs::read_dir(&video_dir)
            .expect("video dir")
            .map(|entry| entry.expect("video entry").path())
            .filter(|path| path.extension().and_then(|extension| extension.to_str()) == Some("mp4"))
            .collect::<Vec<_>>();
        assert!(
            !videos.is_empty(),
            "expected an mp4 artifact in {video_dir:?}"
        );
        assert!(std::fs::metadata(&videos[0]).expect("mp4 metadata").len() > 0);
        assert_ffprobe_reads_video(&videos[0]);

        let lifecycle_json =
            serde_json::to_string(&session.lifecycle_events().await).expect("lifecycle json");
        assert!(!lifecycle_json.contains("browser-use-rs-video"));
        assert!(!lifecycle_json.contains(video_dir.to_str().expect("video dir utf8")));
    }

    #[test]
    fn browser_profile_trace_path_config_matches_upstream_shape() {
        let decoded: BrowserProfile = serde_json::from_value(json!({})).expect("empty profile");
        assert_eq!(decoded.traces_dir, None);

        let encoded = serde_json::to_value(BrowserProfile::default()).expect("profile json");
        assert!(encoded.get("traces_dir").is_none());

        let trace_path = "/tmp/browser-use-rs/traces";
        let configured: BrowserProfile = serde_json::from_value(json!({
            "trace_path": trace_path
        }))
        .expect("trace path profile");
        assert_eq!(
            configured.traces_dir.as_deref(),
            Some(Path::new(trace_path))
        );

        let encoded = serde_json::to_value(&configured).expect("canonical trace profile json");
        assert_eq!(encoded["traces_dir"], json!(trace_path));
        assert!(encoded.get("trace_path").is_none());
        assert_eq!(
            configured.launch_plan().args,
            BrowserProfile::default().launch_plan().args
        );
    }

    #[tokio::test]
    async fn trace_recording_resolves_unique_artifact_paths_inside_traces_dir() {
        let temp_dir = TempDir::new().expect("trace temp dir");
        let traces_dir = temp_dir.path().join("traces");
        let recorder = CdpTraceRecorder {
            dir: traces_dir.clone(),
        };
        let epoch_millis = 1_700_000_000_123;
        let first = recorder.artifact_path(epoch_millis, 0);
        assert_eq!(first.parent(), Some(traces_dir.as_path()));
        let expected_first = format!(
            "browser-use-rs-trace-{epoch_millis}-{}.json",
            std::process::id()
        );
        assert_eq!(
            first.file_name().and_then(|name| name.to_str()),
            Some(expected_first.as_str())
        );

        tokio::fs::create_dir_all(&traces_dir)
            .await
            .expect("create traces dir");
        tokio::fs::write(&first, b"existing")
            .await
            .expect("seed existing trace");
        let second = recorder
            .unique_artifact_path(epoch_millis)
            .await
            .expect("unique trace path");
        let expected_second = format!(
            "browser-use-rs-trace-{epoch_millis}-{}-1.json",
            std::process::id()
        );
        assert_eq!(
            second.file_name().and_then(|name| name.to_str()),
            Some(expected_second.as_str())
        );
    }

    #[tokio::test]
    async fn trace_recording_close_writes_artifact_without_response_metadata() {
        let temp_dir = TempDir::new().expect("trace temp dir");
        let traces_dir = temp_dir.path().join("traces");
        let profile = BrowserProfile {
            traces_dir: Some(traces_dir.clone()),
            ..BrowserProfile::default()
        };
        let (endpoint, command_log) = cdp_command_test_server(None, 8).await;
        let session = CdpBrowserSession::connect_with_profile(endpoint, &profile)
            .await
            .expect("connect traced session");
        session
            .set_cached_dom_state(SerializedDomState {
                text: "cached page text".to_owned(),
                ..SerializedDomState::default()
            })
            .await;
        session
            .record_security_event(BrowserSecurityEvent::prevented_navigation(
                "https://blocked.test".to_owned(),
                "prohibited_domain".to_owned(),
            ))
            .await;

        session.close_browser().await.expect("close traced browser");
        let commands = command_log.await.expect("cdp command log");
        assert_eq!(
            commands.last().map(|command| command.method.as_str()),
            Some("Browser.close")
        );

        let trace_path = std::fs::read_dir(&traces_dir)
            .expect("trace dir entries")
            .map(|entry| entry.expect("trace dir entry").path())
            .find(|path| path.extension().and_then(|extension| extension.to_str()) == Some("json"))
            .expect("trace json artifact");
        let trace: Value = serde_json::from_slice(
            &tokio::fs::read(&trace_path)
                .await
                .expect("read trace artifact"),
        )
        .expect("trace artifact json");
        assert_eq!(
            trace["schema_version"],
            json!(TRACE_ARTIFACT_SCHEMA_VERSION)
        );
        assert_eq!(trace["artifact"]["kind"], json!(TRACE_ARTIFACT_KIND));
        assert_eq!(trace["artifact"]["format"], json!("json"));
        assert_eq!(trace["artifact"]["runtime"], json!("direct_cdp"));
        assert_eq!(trace["artifact"]["playwright_trace_zip"], json!(false));
        assert!(
            trace["generated_at"]
                .as_str()
                .is_some_and(|value| value.ends_with('Z'))
        );
        assert_eq!(trace["current_page"]["target_id"], json!("target-1"));
        assert_eq!(trace["current_page"]["session_id"], json!("session-1"));
        assert_eq!(trace["last_dom_state"]["text"], json!("cached page text"));
        assert!(
            trace["lifecycle_events"]
                .as_array()
                .expect("lifecycle events")
                .iter()
                .any(|event| event["kind"] == json!("browser_close_requested"))
        );
        assert_eq!(
            trace["security_events"][0]["lifecycle_event"]["kind"],
            json!("navigation_blocked")
        );

        let lifecycle_json =
            serde_json::to_string(&session.lifecycle_events().await).expect("lifecycle json");
        assert!(!lifecycle_json.contains("browser-use-rs-trace"));
        assert!(!lifecycle_json.contains(traces_dir.to_str().expect("trace dir utf8")));
    }

    #[tokio::test]
    async fn trace_recording_failure_records_diagnostic_without_artifact_metadata() {
        let temp_dir = TempDir::new().expect("trace temp dir");
        let traces_dir = temp_dir.path().join("trace-output-is-file");
        tokio::fs::write(&traces_dir, b"not a directory")
            .await
            .expect("seed trace file path");
        let profile = BrowserProfile {
            traces_dir: Some(traces_dir.clone()),
            ..BrowserProfile::default()
        };
        let (endpoint, command_log) = cdp_command_test_server(None, 8).await;
        let session = CdpBrowserSession::connect_with_profile(endpoint, &profile)
            .await
            .expect("connect traced session");

        session.close_browser().await.expect("close traced browser");
        let commands = command_log.await.expect("cdp command log");
        assert_eq!(
            commands.last().map(|command| command.method.as_str()),
            Some("Browser.close")
        );

        let events = session.lifecycle_events().await;
        let diagnostic = events
            .iter()
            .find(|event| event.reason.as_deref() == Some("trace_recording_failed"))
            .expect("trace failure diagnostic");
        assert_eq!(
            diagnostic.kind,
            BrowserLifecycleEventKind::BrowserDiagnostic
        );
        assert_eq!(
            diagnostic.details.get("phase").map(String::as_str),
            Some("write")
        );
        assert!(
            diagnostic
                .error
                .as_deref()
                .is_some_and(|error| error.contains("browser state unavailable"))
        );

        let lifecycle_json = serde_json::to_string(&events).expect("lifecycle json");
        assert!(!lifecycle_json.contains("browser-use-rs-trace"));
        assert!(!lifecycle_json.contains(TRACE_ARTIFACT_KIND));
        assert!(!lifecycle_json.contains(traces_dir.to_str().expect("trace dir utf8")));
    }

    #[tokio::test]
    async fn trace_artifact_write_does_not_mutate_lifecycle_responses() {
        let temp_dir = TempDir::new().expect("trace temp dir");
        let mut session = test_session_for_pdf_downloads(None, false);
        session.trace_recorder = Some(CdpTraceRecorder {
            dir: temp_dir.path().join("traces"),
        });
        session
            .record_lifecycle_event(BrowserLifecycleEvent::target_switched("target-1"))
            .await;
        let before = session.lifecycle_events().await;

        let trace_path = session
            .write_trace_artifact()
            .await
            .expect("write trace")
            .expect("trace path");
        let after = session.lifecycle_events().await;

        assert_eq!(after, before);
        let after_json = serde_json::to_string(&after).expect("lifecycle json");
        assert!(!after_json.contains("browser-use-rs-trace"));
        assert!(
            !after_json.contains(
                trace_path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .expect("trace file name")
            )
        );
    }

    #[tokio::test]
    async fn har_recording_writes_https_har_embed_shape() {
        let temp_dir = TempDir::new().expect("HAR temp dir");
        let profile = BrowserProfile {
            record_har_path: Some(temp_dir.path().join("network.har")),
            ..BrowserProfile::default()
        };
        let recorder = CdpHarRecorder::from_profile(&profile).expect("HAR recorder");

        let url = "https://example.test/index.html";
        recorder
            .observe_request_will_be_sent(&har_request_event(
                "request-1",
                url,
                "frame-1",
                "Document",
                10.0,
                1_700_000_000.0,
            ))
            .await;
        recorder
            .observe_response_received(&har_response_event(
                "request-1",
                url,
                200,
                "text/plain",
                10.1,
            ))
            .await;
        recorder
            .observe_data_received(&CdpEvent {
                method: "Network.dataReceived".to_owned(),
                params: json!({
                    "requestId": "request-1",
                    "data": "hello"
                }),
                session_id: None,
            })
            .await;
        {
            let mut state = recorder.state.lock().await;
            let entry = state.entries.get_mut("root:request-1").expect("HAR entry");
            entry.ts_finished = Some(10.35);
            entry.encoded_data_length = Some(5);
            entry.transfer_size = Some(5);
        }
        recorder
            .observe_page_lifecycle(&CdpEvent {
                method: "Page.lifecycleEvent".to_owned(),
                params: json!({
                    "frameId": "frame-1",
                    "name": "DOMContentLoaded",
                    "timestamp": 10.2
                }),
                session_id: None,
            })
            .await;
        recorder
            .observe_page_lifecycle(&CdpEvent {
                method: "Page.lifecycleEvent".to_owned(),
                params: json!({
                    "frameId": "frame-1",
                    "name": "load",
                    "timestamp": 10.3
                }),
                session_id: None,
            })
            .await;
        recorder
            .observe_frame_navigated(&CdpEvent {
                method: "Page.frameNavigated".to_owned(),
                params: json!({
                    "frame": {
                        "id": "frame-1",
                        "name": "Example",
                        "url": url
                    }
                }),
                session_id: None,
            })
            .await;

        recorder.write_har().await.expect("write HAR");
        let har: Value = serde_json::from_slice(
            &tokio::fs::read(temp_dir.path().join("network.har"))
                .await
                .expect("read HAR"),
        )
        .expect("HAR json");

        assert_eq!(har["log"]["version"], json!("1.2"));
        assert_eq!(har["log"]["creator"]["name"], json!("browser-use-rs"));
        assert_eq!(har["log"]["pages"][0]["id"], json!("page@frame-1"));
        assert_eq!(har["log"]["pages"][0]["title"], json!("Example"));
        assert_eq!(
            har["log"]["pages"][0]["pageTimings"]["onContentLoad"],
            json!(200)
        );
        assert_eq!(har["log"]["pages"][0]["pageTimings"]["onLoad"], json!(300));
        let entry = &har["log"]["entries"][0];
        assert_eq!(entry["startedDateTime"], json!("2023-11-14T22:13:20Z"));
        assert_eq!(entry["time"], json!(350));
        assert_eq!(entry["request"]["method"], json!("GET"));
        assert_eq!(entry["request"]["url"], json!(url));
        assert_eq!(entry["response"]["status"], json!(200));
        assert_eq!(entry["response"]["httpVersion"], json!("HTTP/2.0"));
        assert_eq!(entry["response"]["content"]["text"], json!("hello"));
        assert_eq!(entry["response"]["content"]["size"], json!(5));
        assert_eq!(entry["response"]["_transferSize"], json!(5));
        assert_eq!(entry["pageref"], json!("page@frame-1"));
        assert_eq!(entry["serverIPAddress"], json!("203.0.113.10"));
        assert_eq!(entry["_serverPort"], json!(443));
        assert_eq!(entry["_securityDetails"]["protocol"], json!("TLS 1.3"));
        assert!(entry["_securityDetails"].get("sanList").is_none());
    }

    #[tokio::test]
    async fn har_recording_minimal_filters_to_main_page_origin_and_skips_favicon() {
        let temp_dir = TempDir::new().expect("HAR temp dir");
        let profile = BrowserProfile {
            record_har_path: Some(temp_dir.path().join("minimal.har")),
            record_har_mode: RecordHarMode::Minimal,
            ..BrowserProfile::default()
        };
        let recorder = CdpHarRecorder::from_profile(&profile).expect("HAR recorder");

        seed_har_entry(
            &recorder,
            "main",
            "https://example.test/index.html",
            "frame-1",
            "Document",
            b"main",
        )
        .await;
        seed_har_entry(
            &recorder,
            "same-origin",
            "https://example.test/app.js",
            "frame-1",
            "Script",
            b"same",
        )
        .await;
        seed_har_entry(
            &recorder,
            "cross-origin",
            "https://cdn.test/app.js",
            "frame-1",
            "Script",
            b"cross",
        )
        .await;
        seed_har_entry(
            &recorder,
            "favicon",
            "https://example.test/favicon.ico",
            "frame-1",
            "Image",
            b"icon",
        )
        .await;

        recorder.write_har().await.expect("write HAR");
        let har: Value = serde_json::from_slice(
            &tokio::fs::read(temp_dir.path().join("minimal.har"))
                .await
                .expect("read HAR"),
        )
        .expect("HAR json");
        let urls = har["log"]["entries"]
            .as_array()
            .expect("entries")
            .iter()
            .map(|entry| entry["request"]["url"].as_str().expect("entry url"))
            .collect::<Vec<_>>();
        assert_eq!(
            urls,
            vec![
                "https://example.test/index.html",
                "https://example.test/app.js"
            ]
        );
    }

    #[tokio::test]
    async fn har_recording_content_modes_control_body_representation() {
        let temp_dir = TempDir::new().expect("HAR temp dir");

        let omit_profile = BrowserProfile {
            record_har_path: Some(temp_dir.path().join("omit.har")),
            record_har_content: RecordHarContent::Omit,
            ..BrowserProfile::default()
        };
        let omit = CdpHarRecorder::from_profile(&omit_profile).expect("omit HAR recorder");
        seed_har_entry(
            &omit,
            "omit",
            "https://example.test/omit.json",
            "frame-1",
            "Document",
            br#"{"ok":true}"#,
        )
        .await;
        {
            let mut state = omit.state.lock().await;
            state
                .entries
                .get_mut("root:omit")
                .expect("omit entry")
                .post_data = Some("request-body".to_owned());
        }
        omit.write_har().await.expect("write omit HAR");
        let omit_har: Value = serde_json::from_slice(
            &tokio::fs::read(temp_dir.path().join("omit.har"))
                .await
                .expect("read omit HAR"),
        )
        .expect("omit HAR json");
        let omit_entry = &omit_har["log"]["entries"][0];
        assert!(omit_entry["response"]["content"].get("text").is_none());
        assert!(omit_entry["response"]["content"].get("_file").is_none());
        assert!(omit_entry["request"]["postData"].is_null());

        let attach_profile = BrowserProfile {
            record_har_path: Some(temp_dir.path().join("attach.har")),
            record_har_content: RecordHarContent::Attach,
            ..BrowserProfile::default()
        };
        let attach = CdpHarRecorder::from_profile(&attach_profile).expect("attach HAR recorder");
        seed_har_entry(
            &attach,
            "attach",
            "https://example.test/attach.json",
            "frame-1",
            "Document",
            br#"{"ok":true}"#,
        )
        .await;
        {
            let mut state = attach.state.lock().await;
            let entry = state.entries.get_mut("root:attach").expect("attach entry");
            entry.post_data = Some("request-body".to_owned());
            entry
                .request_headers
                .insert("content-type".to_owned(), "text/plain".to_owned());
        }
        attach.write_har().await.expect("write attach HAR");
        let attach_har: Value = serde_json::from_slice(
            &tokio::fs::read(temp_dir.path().join("attach.har"))
                .await
                .expect("read attach HAR"),
        )
        .expect("attach HAR json");
        let attach_entry = &attach_har["log"]["entries"][0];
        let response_file = attach_entry["response"]["content"]["_file"]
            .as_str()
            .expect("attached response file");
        let request_file = attach_entry["request"]["postData"]["_file"]
            .as_str()
            .expect("attached request file");
        assert!(
            temp_dir
                .path()
                .join("attach_har_parts")
                .join(response_file)
                .exists()
        );
        assert!(
            temp_dir
                .path()
                .join("attach_har_parts")
                .join(request_file)
                .exists()
        );
    }

    #[test]
    fn browser_profile_page_load_wait_defaults_and_validation() {
        let decoded: BrowserProfile = serde_json::from_value(json!({})).expect("empty profile");
        assert_eq!(decoded.minimum_wait_page_load_time, 0.25);
        assert_eq!(decoded.wait_for_network_idle_page_load_time, 0.5);

        let encoded = serde_json::to_value(BrowserProfile::default()).expect("profile json");
        assert_eq!(encoded["minimum_wait_page_load_time"], json!(0.25));
        assert_eq!(encoded["wait_for_network_idle_page_load_time"], json!(0.5));

        let zero_waits: BrowserProfile = serde_json::from_value(json!({
            "minimum_wait_page_load_time": 0.0,
            "wait_for_network_idle_page_load_time": 0.0
        }))
        .expect("zero wait profile");
        assert_eq!(zero_waits.minimum_wait_page_load_time, 0.0);
        assert_eq!(zero_waits.wait_for_network_idle_page_load_time, 0.0);
        assert!(PageLoadWaitConfig::from_profile(&zero_waits).is_disabled());

        let negative = serde_json::from_value::<BrowserProfile>(json!({
            "minimum_wait_page_load_time": -0.1
        }))
        .expect_err("negative page-load wait should be rejected");
        assert!(negative.to_string().contains("page-load wait"));
    }

    #[test]
    fn network_activity_state_reports_idle_remaining_from_requests_and_finish_events() {
        let start = Instant::now();
        let mut state = NetworkActivityState::new(start);
        let idle_for = Duration::from_millis(500);

        assert_eq!(
            state.idle_remaining(start + Duration::from_millis(200), idle_for),
            Some(Duration::from_millis(300))
        );
        assert_eq!(
            state.idle_remaining(start + Duration::from_millis(500), idle_for),
            None
        );

        state.observe_request_started("request-1", start + Duration::from_millis(600));
        assert_eq!(
            state.idle_remaining(start + Duration::from_millis(900), idle_for),
            Some(idle_for)
        );

        state.observe_request_finished("request-1", start + Duration::from_millis(1_000));
        assert_eq!(
            state.idle_remaining(start + Duration::from_millis(1_250), idle_for),
            Some(Duration::from_millis(250))
        );
        assert_eq!(
            state.idle_remaining(start + Duration::from_millis(1_500), idle_for),
            None
        );
    }

    #[test]
    fn cloud_browser_request_preserves_proxy_country_tri_state() {
        let omitted = CloudBrowserCreateRequest::default();
        assert_eq!(
            serde_json::to_value(&omitted).expect("request json"),
            json!({})
        );

        let disabled = CloudBrowserCreateRequest {
            proxy_country_code: CloudProxyCountryCode::disabled(),
            ..CloudBrowserCreateRequest::default()
        };
        assert_eq!(
            serde_json::to_value(&disabled).expect("request json"),
            json!({ "proxy_country_code": null })
        );

        let country = CloudBrowserCreateRequest {
            profile_id: Some("profile-123".to_owned()),
            proxy_country_code: CloudProxyCountryCode::country("jp"),
            timeout: Some(60),
            enable_recording: true,
        };
        assert_eq!(
            serde_json::to_value(&country).expect("request json"),
            json!({
                "profile_id": "profile-123",
                "proxy_country_code": "jp",
                "timeout": 60,
                "enable_recording": true
            })
        );
    }

    #[test]
    fn cloud_browser_request_accepts_upstream_aliases() {
        let request: CloudBrowserCreateRequest = serde_json::from_value(json!({
            "cloud_profile_id": "profile-456",
            "cloud_proxy_country_code": null,
            "cloud_timeout": 45,
            "enableRecording": true
        }))
        .expect("alias request");

        assert_eq!(request.profile_id.as_deref(), Some("profile-456"));
        assert_eq!(request.proxy_country_code, CloudProxyCountryCode::Disabled);
        assert_eq!(request.timeout, Some(45));
        assert!(request.enable_recording);
    }

    #[test]
    fn cloud_browser_params_force_cloud_request_without_local_launch_changes() {
        let profile = BrowserProfile {
            cloud_browser_params: Some(CloudBrowserCreateRequest {
                proxy_country_code: CloudProxyCountryCode::disabled(),
                ..CloudBrowserCreateRequest::default()
            }),
            ..BrowserProfile::default()
        };

        assert!(profile.uses_cloud());
        assert_eq!(
            serde_json::to_value(profile.cloud_create_request().expect("cloud request"))
                .expect("request json"),
            json!({ "proxy_country_code": null })
        );

        let plan = profile.launch_plan();
        assert!(plan.args.contains(&"--headless=new".to_owned()));
        assert!(plan.args.contains(&"--remote-debugging-port=0".to_owned()));
    }

    #[tokio::test]
    async fn cloud_browser_client_tracks_created_session_and_stops_current() {
        let (base_url, server) = cloud_test_server(vec![
            (200, cloud_browser_response_json("browser-123", "running")),
            (200, cloud_browser_response_json("browser-123", "stopped")),
        ])
        .await;
        let client = CloudBrowserClient::with_api_key("test-key").with_base_url(base_url);

        let created = client
            .create_browser(&CloudBrowserCreateRequest {
                proxy_country_code: CloudProxyCountryCode::disabled(),
                ..CloudBrowserCreateRequest::default()
            })
            .await
            .expect("create cloud browser");
        assert_eq!(created.id, "browser-123");
        assert_eq!(
            client.current_session_id().await.as_deref(),
            Some("browser-123")
        );

        let stopped = client
            .stop_browser(None)
            .await
            .expect("stop current cloud browser");
        assert_eq!(stopped.status, "stopped");
        assert_eq!(client.current_session_id().await, None);

        let requests = server.await.expect("cloud server task");
        assert_eq!(requests.len(), 2);
        assert!(requests[0].starts_with("POST /api/v2/browsers "));
        assert_eq!(
            request_header(&requests[0], "x-browser-use-api-key"),
            Some("test-key")
        );
        assert_eq!(
            request_body(&requests[0]),
            json!({ "proxy_country_code": null })
        );
        assert!(requests[1].starts_with("PATCH /api/v2/browsers/browser-123 "));
        assert_eq!(request_body(&requests[1]), json!({ "action": "stop" }));
    }

    #[tokio::test]
    async fn cloud_browser_client_sends_extra_headers_on_create_and_stop() {
        let (base_url, server) = cloud_test_server(vec![
            (200, cloud_browser_response_json("browser-extra", "running")),
            (200, cloud_browser_response_json("browser-extra", "stopped")),
        ])
        .await;
        let client = CloudBrowserClient::with_api_key("test-key").with_base_url(base_url);

        client
            .create_browser_with_headers(
                &CloudBrowserCreateRequest::default(),
                [
                    ("X-Trace-Id", "trace-create"),
                    ("X-Browser-Use-API-Key", "override-key"),
                ],
            )
            .await
            .expect("create cloud browser with extra headers");
        client
            .stop_browser_with_headers(Some("browser-extra"), [("X-Trace-Id", "trace-stop")])
            .await
            .expect("stop cloud browser with extra headers");

        let requests = server.await.expect("cloud server task");
        assert_eq!(requests.len(), 2);
        assert_eq!(
            request_header(&requests[0], "x-trace-id"),
            Some("trace-create")
        );
        assert_eq!(
            request_header(&requests[0], "x-browser-use-api-key"),
            Some("override-key")
        );
        assert_eq!(
            request_header(&requests[1], "x-trace-id"),
            Some("trace-stop")
        );
        assert_eq!(
            request_header(&requests[1], "x-browser-use-api-key"),
            Some("test-key")
        );
    }

    #[tokio::test]
    async fn cloud_browser_client_rejects_invalid_extra_headers_before_request() {
        let error = CloudBrowserClient::with_api_key("test-key")
            .create_browser_with_headers(
                &CloudBrowserCreateRequest::default(),
                [("bad header", "value")],
            )
            .await
            .expect_err("invalid extra header name");
        assert!(
            error
                .to_string()
                .contains("Invalid cloud extra header name")
        );

        let error = CloudBrowserClient::with_api_key("test-key")
            .stop_browser_with_headers(Some("browser-extra"), [("X-Trace-Id", "bad\nvalue")])
            .await
            .expect_err("invalid extra header value");
        assert!(
            error
                .to_string()
                .contains("Invalid cloud extra header value")
        );
    }

    #[tokio::test]
    async fn cloud_browser_client_contextualizes_non_success_errors() {
        let (base_url, server) =
            cloud_test_server(vec![(500, json!({ "detail": "create failed" }))]).await;
        let error = CloudBrowserClient::with_api_key("test-key")
            .with_base_url(base_url)
            .create_browser(&CloudBrowserCreateRequest::default())
            .await
            .expect_err("create failure should include action context");
        assert!(error.to_string().contains(
            "Failed to create cloud browser: HTTP 500 Internal Server Error - create failed"
        ));
        server.await.expect("create failure server task");

        let (base_url, server) =
            cloud_test_server(vec![(503, json!({ "detail": "stop failed" }))]).await;
        let error = CloudBrowserClient::with_api_key("test-key")
            .with_base_url(base_url)
            .stop_browser(Some("browser-failed"))
            .await
            .expect_err("stop failure should include action context");
        assert!(
            error.to_string().contains(
                "Failed to stop cloud browser: HTTP 503 Service Unavailable - stop failed"
            )
        );
        server.await.expect("stop failure server task");
    }

    #[tokio::test]
    async fn cloud_browser_client_stops_explicit_session_id() {
        let (base_url, server) = cloud_test_server(vec![(
            200,
            cloud_browser_response_json("browser-explicit", "stopped"),
        )])
        .await;
        let client = CloudBrowserClient::with_api_key("test-key").with_base_url(base_url);

        let stopped = client
            .stop_browser(Some("browser-explicit"))
            .await
            .expect("stop explicit cloud browser");
        assert_eq!(stopped.id, "browser-explicit");
        assert_eq!(client.current_session_id().await, None);

        let requests = server.await.expect("cloud server task");
        assert_eq!(requests.len(), 1);
        assert!(requests[0].starts_with("PATCH /api/v2/browsers/browser-explicit "));
        assert_eq!(request_body(&requests[0]), json!({ "action": "stop" }));
    }

    #[tokio::test]
    async fn cloud_browser_client_reports_missing_current_session() {
        let error = CloudBrowserClient::with_api_key("test-key")
            .stop_browser(None)
            .await
            .expect_err("missing current session should fail");

        assert!(error.to_string().contains("No session ID provided"));
    }

    #[tokio::test]
    async fn cloud_browser_client_clears_current_session_on_not_found() {
        let (base_url, server) = cloud_test_server(vec![
            (
                200,
                cloud_browser_response_json("browser-missing", "running"),
            ),
            (404, json!({ "detail": "not found" })),
        ])
        .await;
        let client = CloudBrowserClient::with_api_key("test-key").with_base_url(base_url);
        client
            .create_browser(&CloudBrowserCreateRequest::default())
            .await
            .expect("create cloud browser");
        assert_eq!(
            client.current_session_id().await.as_deref(),
            Some("browser-missing")
        );

        let error = client
            .stop_browser(None)
            .await
            .expect_err("404 stop should fail");
        assert!(error.to_string().contains("browser-missing not found"));
        assert_eq!(client.current_session_id().await, None);

        let requests = server.await.expect("cloud server task");
        assert_eq!(requests.len(), 2);
        assert!(requests[1].starts_with("PATCH /api/v2/browsers/browser-missing "));
    }

    #[test]
    fn cloud_api_key_resolution_prefers_explicit_then_env_then_auth_config() {
        let temp_dir = TempDir::new().expect("temp cloud auth dir");
        let auth_config_path = temp_dir.path().join("cloud_auth.json");
        std::fs::write(&auth_config_path, r#"{ "api_token": "config-key" }"#)
            .expect("write cloud auth config");

        assert_eq!(
            resolve_cloud_api_key(
                Some("explicit-key"),
                Some("env-key".to_owned()),
                Some(&auth_config_path)
            )
            .as_deref(),
            Some("explicit-key")
        );
        assert_eq!(
            resolve_cloud_api_key(
                Some("  "),
                Some("env-key".to_owned()),
                Some(&auth_config_path)
            )
            .as_deref(),
            Some("env-key")
        );
        assert_eq!(
            resolve_cloud_api_key(None, Some("  ".to_owned()), Some(&auth_config_path)).as_deref(),
            Some("config-key")
        );
    }

    #[tokio::test]
    async fn cloud_browser_client_uses_auth_config_api_token() {
        let temp_dir = TempDir::new().expect("temp cloud auth dir");
        let auth_config_path = temp_dir.path().join("cloud_auth.json");
        std::fs::write(&auth_config_path, r#"{ "api_token": "config-key" }"#)
            .expect("write cloud auth config");
        let (base_url, server) = cloud_test_server(vec![(
            200,
            cloud_browser_response_json("browser-config-token", "running"),
        )])
        .await;
        let client = CloudBrowserClient::new()
            .with_auth_config_path(auth_config_path)
            .with_base_url(base_url);

        let created = client
            .create_browser(&CloudBrowserCreateRequest::default())
            .await
            .expect("create cloud browser");
        assert_eq!(created.id, "browser-config-token");

        let requests = server.await.expect("cloud server task");
        assert_eq!(requests.len(), 1);
        assert!(
            requests[0]
                .to_ascii_lowercase()
                .contains("x-browser-use-api-key: config-key")
        );
    }

    #[test]
    fn cloud_auth_config_fallback_ignores_missing_empty_and_corrupt_files() {
        let temp_dir = TempDir::new().expect("temp cloud auth dir");
        let missing_path = temp_dir.path().join("missing.json");
        assert_eq!(load_cloud_auth_api_token(Some(&missing_path)), None);

        let corrupt_path = temp_dir.path().join("corrupt.json");
        std::fs::write(&corrupt_path, "{").expect("write corrupt config");
        assert_eq!(load_cloud_auth_api_token(Some(&corrupt_path)), None);

        let empty_path = temp_dir.path().join("empty.json");
        std::fs::write(&empty_path, r#"{ "api_token": "  " }"#).expect("write empty config");
        assert_eq!(load_cloud_auth_api_token(Some(&empty_path)), None);
    }

    #[test]
    fn cloud_auth_config_path_matches_upstream_env_layout() {
        assert_eq!(
            cloud_auth_config_path(
                Some(PathBuf::from("~/browser-use")),
                Some(PathBuf::from("/xdg")),
                Some(PathBuf::from("/home/alice"))
            ),
            PathBuf::from("/home/alice/browser-use/cloud_auth.json")
        );
        assert_eq!(
            cloud_auth_config_path(None, Some(PathBuf::from("/xdg")), None),
            PathBuf::from("/xdg/browseruse/cloud_auth.json")
        );
        assert_eq!(
            cloud_auth_config_path(None, None, Some(PathBuf::from("/home/alice"))),
            PathBuf::from("/home/alice/.config/browseruse/cloud_auth.json")
        );
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
                bypass: None,
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
                .contains(&"--profile-directory=Default".to_owned())
        );
        assert!(
            plan.args
                .contains(&"--proxy-server=http://127.0.0.1:8080".to_owned())
        );
        assert_eq!(plan.args.last(), Some(&"--disable-gpu".to_owned()));
    }

    #[test]
    fn launch_plan_preserves_env_without_changing_args() {
        let profile = BrowserProfile {
            env: BTreeMap::from([
                ("BROWSER_USE_HEADLESS".to_owned(), "false".to_owned()),
                ("BROWSER_USE_TOKEN".to_owned(), "secret".to_owned()),
            ]),
            args: vec!["--custom-last".to_owned()],
            ..BrowserProfile::default()
        };

        let plan = profile.launch_plan();

        assert_eq!(plan.env, profile.env);
        assert_eq!(plan.args.last(), Some(&"--custom-last".to_owned()));
        assert!(plan.args.iter().any(|arg| arg == "--headless=new"));
    }

    #[test]
    fn launch_plan_emits_representative_upstream_default_args() {
        let profile = BrowserProfile::default();
        let plan = profile.launch_plan();

        for arg in [
            "--disable-background-networking",
            "--disable-popup-blocking",
            "--disable-sync",
            "--enable-features=NetworkService,NetworkServiceInProcess",
        ] {
            assert!(
                plan.args.iter().any(|plan_arg| plan_arg == arg),
                "missing upstream default arg {arg}"
            );
        }
    }

    #[test]
    fn launch_plan_suppresses_listed_default_args() {
        let profile = BrowserProfile {
            ignore_default_args: IgnoreDefaultArgs::List(vec![
                "--disable-sync".to_owned(),
                "--disable-popup-blocking".to_owned(),
            ]),
            ..BrowserProfile::default()
        };
        let plan = profile.launch_plan();

        assert!(!plan.args.iter().any(|arg| arg == "--disable-sync"));
        assert!(
            !plan
                .args
                .iter()
                .any(|arg| arg == "--disable-popup-blocking")
        );
        assert!(
            plan.args
                .iter()
                .any(|arg| arg == "--disable-background-networking")
        );
    }

    #[test]
    fn launch_plan_suppresses_all_default_args_when_requested() {
        let profile = BrowserProfile {
            ignore_default_args: IgnoreDefaultArgs::All(true),
            ..BrowserProfile::default()
        };
        let plan = profile.launch_plan();

        for arg in [
            "--disable-background-networking",
            "--disable-popup-blocking",
            "--disable-sync",
            "--no-first-run",
            "--no-default-browser-check",
        ] {
            assert!(
                !plan.args.iter().any(|plan_arg| plan_arg == arg),
                "default arg {arg} should be suppressed"
            );
        }
        assert!(
            plan.args
                .iter()
                .any(|arg| arg == "--remote-debugging-port=0")
        );
        assert!(plan.args.iter().any(|arg| arg == "--window-size=1280,720"));
        assert!(plan.args.iter().any(|arg| arg == "--headless=new"));
    }

    #[test]
    fn launch_plan_merges_disable_features_values_in_order() {
        let profile = BrowserProfile {
            disable_security: true,
            args: vec![
                "--disable-features=MediaRouter,Translate,IsolateOrigins".to_owned(),
                "--custom-last".to_owned(),
            ],
            ..BrowserProfile::default()
        };
        let plan = profile.launch_plan();
        let disable_features_args = plan
            .args
            .iter()
            .filter(|arg| arg.starts_with("--disable-features="))
            .map(String::as_str)
            .collect::<Vec<_>>();

        assert_eq!(
            disable_features_args,
            vec!["--disable-features=IsolateOrigins,site-per-process,MediaRouter,Translate"]
        );
        assert_eq!(plan.args.last(), Some(&"--custom-last".to_owned()));
    }

    #[test]
    fn launch_plan_dedupes_duplicate_switches_with_last_value() {
        let profile = BrowserProfile {
            user_agent: Some("GeneratedAgent/1.0".to_owned()),
            args: vec![
                "--user-agent=CallerAgent/2.0".to_owned(),
                "--remote-debugging-port=9333".to_owned(),
            ],
            ..BrowserProfile::default()
        };
        let plan = profile.launch_plan();

        assert!(
            !plan
                .args
                .iter()
                .any(|arg| arg == "--user-agent=GeneratedAgent/1.0")
        );
        assert!(
            plan.args
                .iter()
                .any(|arg| arg == "--user-agent=CallerAgent/2.0")
        );
        assert!(
            !plan
                .args
                .iter()
                .any(|arg| arg == "--remote-debugging-port=0")
        );
        assert_eq!(
            plan.args.last(),
            Some(&"--remote-debugging-port=9333".to_owned())
        );
    }

    #[test]
    fn launch_plan_emits_default_profile_directory_with_user_data_dir() {
        let profile = BrowserProfile {
            user_data_dir: Some(PathBuf::from("/tmp/browser-use-rs-profile")),
            ..BrowserProfile::default()
        };

        let plan = profile.launch_plan();
        let user_data_index = arg_index(&plan.args, "--user-data-dir=/tmp/browser-use-rs-profile");
        let profile_directory_index = arg_index(&plan.args, "--profile-directory=Default");

        assert_eq!(profile_directory_index, user_data_index + 1);
    }

    #[test]
    fn launch_plan_omits_empty_or_orphan_profile_directory() {
        let no_user_data_dir = BrowserProfile {
            profile_directory: "Profile 2".to_owned(),
            ..BrowserProfile::default()
        }
        .launch_plan();
        assert!(
            !no_user_data_dir
                .args
                .iter()
                .any(|arg| arg.starts_with("--profile-directory="))
        );

        let empty_profile_directory = BrowserProfile {
            user_data_dir: Some(PathBuf::from("/tmp/browser-use-rs-profile")),
            profile_directory: String::new(),
            ..BrowserProfile::default()
        }
        .launch_plan();
        assert!(
            !empty_profile_directory
                .args
                .iter()
                .any(|arg| arg.starts_with("--profile-directory="))
        );
    }

    #[test]
    fn launch_plan_places_custom_profile_directory_before_generated_and_custom_args() {
        let profile = BrowserProfile {
            user_data_dir: Some(PathBuf::from("/tmp/browser-use-rs-profile")),
            profile_directory: "Profile 2".to_owned(),
            disable_security: true,
            args: vec![
                "--profile-directory=Override".to_owned(),
                "--custom-last".to_owned(),
            ],
            ..BrowserProfile::default()
        };

        let plan = profile.launch_plan();
        let user_data_index = arg_index(&plan.args, "--user-data-dir=/tmp/browser-use-rs-profile");
        let security_index = arg_index(&plan.args, "--disable-site-isolation-trials");
        let custom_profile_directory_index = arg_index(&plan.args, "--profile-directory=Override");

        assert!(
            !plan
                .args
                .iter()
                .any(|arg| arg == "--profile-directory=Profile 2")
        );
        assert!(user_data_index < security_index);
        assert!(security_index < custom_profile_directory_index);
        assert_eq!(plan.args.last(), Some(&"--custom-last".to_owned()));
    }

    #[test]
    fn launch_plan_emits_chromium_sandbox_args_when_disabled() {
        let profile = BrowserProfile {
            chromium_sandbox: false,
            ..BrowserProfile::default()
        };

        let plan = profile.launch_plan();

        for arg in CHROME_DOCKER_ARGS {
            assert!(
                plan.args.iter().any(|plan_arg| plan_arg.as_str() == *arg),
                "missing chromium_sandbox=false launch arg {arg}"
            );
        }
    }

    #[test]
    fn launch_plan_keeps_chromium_sandbox_args_before_custom_args() {
        let profile = BrowserProfile {
            chromium_sandbox: false,
            user_data_dir: Some(PathBuf::from("/tmp/browser-use-rs-profile")),
            args: vec!["--no-sandbox=false".to_owned(), "--custom-last".to_owned()],
            ..BrowserProfile::default()
        };

        let plan = profile.launch_plan();
        let profile_directory_index = arg_index(&plan.args, "--profile-directory=Default");
        let first_custom_arg_index = arg_index(&plan.args, "--no-sandbox=false");

        assert!(!plan.args.iter().any(|arg| arg == "--no-sandbox"));
        for arg in CHROME_DOCKER_ARGS {
            if *arg == "--no-sandbox" {
                continue;
            }
            assert!(
                arg_index(&plan.args, arg) < first_custom_arg_index,
                "generated chromium_sandbox=false launch arg {arg} should come before caller args"
            );
        }
        assert!(profile_directory_index < first_custom_arg_index);
        assert_eq!(plan.args.last(), Some(&"--custom-last".to_owned()));
    }

    #[test]
    fn launch_plan_emits_devtools_arg_when_headful() {
        let profile = BrowserProfile {
            headless: false,
            devtools: true,
            ..BrowserProfile::default()
        };

        let plan = profile.try_launch_plan().expect("devtools launch plan");

        assert!(
            plan.args
                .contains(&"--auto-open-devtools-for-tabs".to_owned())
        );
        assert!(!plan.args.contains(&"--headless=new".to_owned()));
    }

    #[test]
    fn launch_plan_rejects_devtools_with_headless() {
        let profile = BrowserProfile {
            headless: true,
            devtools: true,
            ..BrowserProfile::default()
        };

        let error = profile
            .try_launch_plan()
            .expect_err("headless devtools should fail launch planning");

        assert!(
            error
                .to_string()
                .contains("headless=True and devtools=True cannot both be set")
        );
    }

    #[test]
    fn launch_plan_rejects_no_viewport_with_headless() {
        let profile = BrowserProfile {
            headless: true,
            no_viewport: true,
            ..BrowserProfile::default()
        };

        let error = profile
            .try_launch_plan()
            .expect_err("headless no_viewport should fail launch planning");

        assert!(
            error
                .to_string()
                .contains("headless=True and no_viewport=True cannot both be set")
        );
    }

    #[test]
    fn launch_plan_keeps_devtools_arg_before_custom_args() {
        let profile = BrowserProfile {
            headless: false,
            devtools: true,
            args: vec![
                "--auto-open-devtools-for-tabs=false".to_owned(),
                "--custom-last".to_owned(),
            ],
            ..BrowserProfile::default()
        };

        let plan = profile.try_launch_plan().expect("devtools launch plan");
        let custom_devtools_index = arg_index(&plan.args, "--auto-open-devtools-for-tabs=false");

        assert!(
            !plan
                .args
                .iter()
                .any(|arg| arg == "--auto-open-devtools-for-tabs")
        );
        assert!(custom_devtools_index < arg_index(&plan.args, "--custom-last"));
        assert_eq!(plan.args.last(), Some(&"--custom-last".to_owned()));
    }

    #[test]
    fn launch_plan_uses_explicit_window_size_without_mutating_viewport() {
        let profile = BrowserProfile {
            window_size: Some(BrowserViewport {
                width: 1920,
                height: 1400,
            }),
            ..BrowserProfile::default()
        };

        let plan = profile.launch_plan();

        assert_eq!(profile.viewport, BrowserViewport::default());
        assert!(plan.args.contains(&"--window-size=1920,1400".to_owned()));
        assert!(!plan.args.contains(&"--window-size=1280,720".to_owned()));
    }

    #[test]
    fn launch_plan_can_use_screen_as_window_size_fallback() {
        let profile = BrowserProfile {
            screen: Some(BrowserViewport {
                width: 1440,
                height: 900,
            }),
            ..BrowserProfile::default()
        };

        let plan = profile.launch_plan();

        assert!(plan.args.contains(&"--window-size=1440,900".to_owned()));
        assert!(!plan.args.contains(&"--window-size=1280,720".to_owned()));
    }

    #[test]
    fn viewport_emulation_params_match_cdp_shape() {
        let params = viewport_emulation_params(ViewportEmulationConfig {
            viewport: Some(BrowserViewport {
                width: 1024,
                height: 768,
            }),
            device_scale_factor: 2.0,
        })
        .expect("viewport params");

        assert_eq!(
            params,
            json!({
                "width": 1024,
                "height": 768,
                "deviceScaleFactor": 2.0,
                "mobile": false
            })
        );

        assert_eq!(
            viewport_emulation_params(ViewportEmulationConfig {
                viewport: None,
                device_scale_factor: 2.0,
            }),
            None
        );
    }

    #[test]
    fn launch_plan_emits_window_position() {
        let profile = BrowserProfile {
            window_position: Some(BrowserViewport {
                width: 40,
                height: 80,
            }),
            ..BrowserProfile::default()
        };

        let plan = profile.launch_plan();

        assert!(plan.args.contains(&"--window-position=40,80".to_owned()));
    }

    #[test]
    fn launch_plan_keeps_window_geometry_before_custom_args() {
        let profile = BrowserProfile {
            window_size: Some(BrowserViewport {
                width: 1440,
                height: 900,
            }),
            window_position: Some(BrowserViewport {
                width: 10,
                height: 20,
            }),
            args: vec![
                "--window-size=1,1".to_owned(),
                "--window-position=2,2".to_owned(),
                "--custom-last".to_owned(),
            ],
            ..BrowserProfile::default()
        };

        let plan = profile.launch_plan();
        let custom_size_index = arg_index(&plan.args, "--window-size=1,1");
        let custom_position_index = arg_index(&plan.args, "--window-position=2,2");

        assert!(!plan.args.iter().any(|arg| arg == "--window-size=1440,900"));
        assert!(!plan.args.iter().any(|arg| arg == "--window-position=10,20"));
        assert!(custom_size_index < custom_position_index);
        assert_eq!(plan.args.last(), Some(&"--custom-last".to_owned()));
    }

    #[test]
    fn launch_plan_emits_disable_security_args() {
        let profile = BrowserProfile {
            disable_security: true,
            ..BrowserProfile::default()
        };

        let plan = profile.launch_plan();

        for arg in CHROME_DISABLE_SECURITY_ARGS {
            assert!(
                plan.args.iter().any(|plan_arg| plan_arg.as_str() == *arg),
                "missing disable_security launch arg {arg}"
            );
        }
    }

    #[test]
    fn launch_plan_emits_deterministic_rendering_args() {
        let profile = BrowserProfile {
            deterministic_rendering: true,
            ..BrowserProfile::default()
        };

        let plan = profile.launch_plan();

        for arg in CHROME_DETERMINISTIC_RENDERING_ARGS {
            assert!(
                plan.args.iter().any(|plan_arg| plan_arg.as_str() == *arg),
                "missing deterministic_rendering launch arg {arg}"
            );
        }
    }

    #[test]
    fn launch_plan_keeps_security_and_rendering_args_before_custom_args() {
        let profile = BrowserProfile {
            disable_security: true,
            deterministic_rendering: true,
            args: vec![
                "--force-device-scale-factor=1".to_owned(),
                "--custom-last".to_owned(),
            ],
            proxy: Some(ProxySettings {
                server: "http://127.0.0.1:8080".to_owned(),
                bypass: Some("localhost".to_owned()),
                username: None,
                password: None,
            }),
            ..BrowserProfile::default()
        };

        let plan = profile.launch_plan();
        let first_custom_arg_index = arg_index(&plan.args, "--force-device-scale-factor=1");

        assert_eq!(plan.args.last(), Some(&"--custom-last".to_owned()));
        for arg in CHROME_DISABLE_SECURITY_ARGS
            .iter()
            .chain(CHROME_DETERMINISTIC_RENDERING_ARGS.iter())
        {
            if *arg == "--force-device-scale-factor=2" {
                continue;
            }
            assert!(
                arg_index(&plan.args, arg) < first_custom_arg_index,
                "generated launch arg {arg} should come before caller args"
            );
        }
        assert!(
            !plan
                .args
                .iter()
                .any(|arg| arg == "--force-device-scale-factor=2")
        );
        assert!(
            arg_index(&plan.args, "--disable-site-isolation-trials")
                < arg_index(&plan.args, "--deterministic-mode")
        );
        assert!(arg_index(&plan.args, "--proxy-bypass-list=localhost") < first_custom_arg_index);
    }

    #[test]
    fn launch_plan_omits_empty_user_agent() {
        let profile = BrowserProfile {
            user_agent: Some(String::new()),
            args: vec!["--custom-last".to_owned()],
            ..BrowserProfile::default()
        };

        let plan = profile.launch_plan();

        assert!(!plan.args.iter().any(|arg| arg.starts_with("--user-agent=")));
        assert_eq!(plan.args.last(), Some(&"--custom-last".to_owned()));
    }

    #[test]
    fn launch_plan_emits_user_agent_before_custom_args() {
        let profile = BrowserProfile {
            user_agent: Some("BrowserUseRust/0.4".to_owned()),
            args: vec![
                "--user-agent=OverrideAgent/1.0".to_owned(),
                "--custom-last".to_owned(),
            ],
            proxy: Some(ProxySettings {
                server: "http://127.0.0.1:8080".to_owned(),
                bypass: None,
                username: None,
                password: None,
            }),
            ..BrowserProfile::default()
        };

        let plan = profile.launch_plan();
        let proxy_index = arg_index(&plan.args, "--proxy-server=http://127.0.0.1:8080");
        let custom_user_agent_index = arg_index(&plan.args, "--user-agent=OverrideAgent/1.0");

        assert!(
            !plan
                .args
                .iter()
                .any(|arg| arg == "--user-agent=BrowserUseRust/0.4")
        );
        assert!(proxy_index < custom_user_agent_index);
        assert_eq!(plan.args.last(), Some(&"--custom-last".to_owned()));
    }

    #[test]
    fn proxy_settings_serializes_optional_bypass() {
        let without_bypass = ProxySettings {
            server: "socks5://127.0.0.1:1080".to_owned(),
            bypass: None,
            username: None,
            password: None,
        };
        assert_eq!(
            serde_json::to_value(&without_bypass).expect("serialize proxy without bypass"),
            json!({ "server": "socks5://127.0.0.1:1080" })
        );

        let with_bypass = ProxySettings {
            server: "http://proxy.internal:8080".to_owned(),
            bypass: Some("localhost,127.0.0.1,*.internal".to_owned()),
            username: Some("alice".to_owned()),
            password: None,
        };
        assert_eq!(
            serde_json::to_value(&with_bypass).expect("serialize proxy with bypass"),
            json!({
                "server": "http://proxy.internal:8080",
                "bypass": "localhost,127.0.0.1,*.internal",
                "username": "alice"
            })
        );

        let decoded: ProxySettings = serde_json::from_value(json!({
            "server": "http://proxy.internal:8080"
        }))
        .expect("deserialize proxy without bypass");
        assert_eq!(decoded.bypass, None);
    }

    #[test]
    fn launch_plan_emits_proxy_bypass_after_proxy_server() {
        let profile = BrowserProfile {
            args: vec!["--disable-gpu".to_owned()],
            proxy: Some(ProxySettings {
                server: "http://proxy.internal:8080".to_owned(),
                bypass: Some("localhost,127.0.0.1,*.internal".to_owned()),
                username: None,
                password: None,
            }),
            ..BrowserProfile::default()
        };

        let plan = profile.launch_plan();
        let proxy_server_index = plan
            .args
            .iter()
            .position(|arg| arg == "--proxy-server=http://proxy.internal:8080")
            .expect("proxy server arg");
        let proxy_bypass_index = plan
            .args
            .iter()
            .position(|arg| arg == "--proxy-bypass-list=localhost,127.0.0.1,*.internal")
            .expect("proxy bypass arg");

        assert_eq!(proxy_bypass_index, proxy_server_index + 1);
        assert_eq!(plan.args.last(), Some(&"--disable-gpu".to_owned()));
    }

    #[test]
    fn launch_plan_skips_proxy_bypass_without_server_or_value() {
        let bypass_without_server = BrowserProfile {
            proxy: Some(ProxySettings {
                server: String::new(),
                bypass: Some("localhost".to_owned()),
                username: None,
                password: None,
            }),
            ..BrowserProfile::default()
        }
        .launch_plan();

        assert!(
            !bypass_without_server
                .args
                .iter()
                .any(|arg| arg.starts_with("--proxy-server="))
        );
        assert!(
            !bypass_without_server
                .args
                .iter()
                .any(|arg| arg.starts_with("--proxy-bypass-list="))
        );

        let empty_bypass = BrowserProfile {
            proxy: Some(ProxySettings {
                server: "http://proxy.internal:8080".to_owned(),
                bypass: Some(String::new()),
                username: None,
                password: None,
            }),
            ..BrowserProfile::default()
        }
        .launch_plan();

        assert!(
            empty_bypass
                .args
                .contains(&"--proxy-server=http://proxy.internal:8080".to_owned())
        );
        assert!(
            !empty_bypass
                .args
                .iter()
                .any(|arg| arg.starts_with("--proxy-bypass-list="))
        );
    }

    #[test]
    fn iframe_targets_match_parent_and_frame_urls() {
        let targets = json!({
            "targetInfos": [
                {
                    "type": "iframe",
                    "targetId": "child",
                    "parentId": "root",
                    "url": "http://127.0.0.1:8081/child#section"
                },
                {
                    "type": "iframe",
                    "targetId": "unrelated",
                    "parentId": "other-page",
                    "url": "http://127.0.0.1:8081/child"
                },
                {
                    "type": "iframe",
                    "targetId": "fallback",
                    "url": "https://example.test/frame"
                },
                {
                    "type": "page",
                    "targetId": "page",
                    "url": "https://example.test/frame"
                }
            ]
        });
        let frame_infos = vec![
            FrameElementInfo {
                url: "http://127.0.0.1:8081/child".to_owned(),
                offset: FrameOffset { x: 12, y: 34 },
            },
            FrameElementInfo {
                url: "https://example.test/frame".to_owned(),
                offset: FrameOffset { x: 56, y: 78 },
            },
        ];

        let infos = iframe_target_infos_from_targets(
            &targets,
            "root",
            &frame_infos,
            IframeTraversalConfig::from_profile(&BrowserProfile::default()),
        );

        assert_eq!(
            infos,
            vec![
                IframeTargetInfo {
                    target_id: "child".to_owned(),
                    offset: FrameOffset { x: 12, y: 34 },
                    depth: 1,
                },
                IframeTargetInfo {
                    target_id: "fallback".to_owned(),
                    offset: FrameOffset { x: 56, y: 78 },
                    depth: 1,
                },
            ]
        );
    }

    #[test]
    fn iframe_target_limits_honor_profile_controls() {
        let targets = json!({
            "targetInfos": [
                {
                    "type": "iframe",
                    "targetId": "one",
                    "parentId": "root",
                    "url": "https://example.test/one"
                },
                {
                    "type": "iframe",
                    "targetId": "two",
                    "parentId": "root",
                    "url": "https://example.test/two"
                },
                {
                    "type": "iframe",
                    "targetId": "three",
                    "parentId": "root",
                    "url": "https://example.test/three"
                }
            ]
        });
        let frame_infos = vec![
            FrameElementInfo {
                url: "https://example.test/one".to_owned(),
                offset: FrameOffset { x: 1, y: 1 },
            },
            FrameElementInfo {
                url: "https://example.test/two".to_owned(),
                offset: FrameOffset { x: 2, y: 2 },
            },
            FrameElementInfo {
                url: "https://example.test/three".to_owned(),
                offset: FrameOffset { x: 3, y: 3 },
            },
        ];
        let limited = IframeTraversalConfig {
            cross_origin_iframes: true,
            max_iframes: 2,
            max_iframe_depth: 5,
        };

        let infos = iframe_target_infos_from_targets(&targets, "root", &frame_infos, limited);
        assert_eq!(infos.len(), 2);
        assert_eq!(infos[0].target_id, "one");
        assert_eq!(infos[1].target_id, "two");

        let disabled = IframeTraversalConfig {
            cross_origin_iframes: false,
            max_iframes: 100,
            max_iframe_depth: 5,
        };
        assert!(
            iframe_target_infos_from_targets(&targets, "root", &frame_infos, disabled).is_empty()
        );

        let zero_depth = IframeTraversalConfig {
            cross_origin_iframes: true,
            max_iframes: 100,
            max_iframe_depth: 0,
        };
        assert!(
            iframe_target_infos_from_targets(&targets, "root", &frame_infos, zero_depth).is_empty()
        );

        let zero_iframes = IframeTraversalConfig {
            cross_origin_iframes: true,
            max_iframes: 0,
            max_iframe_depth: 5,
        };
        assert!(
            iframe_target_infos_from_targets(&targets, "root", &frame_infos, zero_iframes)
                .is_empty()
        );
    }

    #[test]
    fn interactive_snapshot_script_carries_iframe_traversal_limits() {
        let script = interactive_elements_js(
            IframeTraversalConfig {
                cross_origin_iframes: true,
                max_iframes: 7,
                max_iframe_depth: 2,
            },
            true,
        );

        assert!(script.contains("const maxIframeDepth = 2;"));
        assert!(script.contains("const maxIframeDocuments = 7;"));
        assert!(script.contains("if (depth >= maxIframeDepth) return;"));
        assert!(script.contains("if (visitedIframeDocuments >= maxIframeDocuments) return;"));
        assert!(script.contains("visitChildren(frameDocument, { x: offset.x + rect.x, y: offset.y + rect.y }, depth + 1);"));
    }

    #[test]
    fn cached_iframe_fallback_uses_target_local_index() {
        let state = SerializedDomState::from_elements(vec![
            test_dom_bound_element(1, "root-target", "Root iframe", None),
            test_dom_bound_element(2, "child-target", "Child button", None),
            test_dom_bound_element(3, "child-target", "Child input", None),
        ]);
        let current_page = AttachedPage {
            target_id: "root-target".to_owned(),
            session_id: "root-session".to_owned(),
        };
        let cached = CachedDomElementRef {
            element: state.selector_map[&3].clone(),
            target_local_index: target_local_index_for_global_index(
                &state.selector_map,
                3,
                "child-target",
            ),
        };

        assert_eq!(cached.target_local_index, 2);
        assert_eq!(
            index_fallback_target_id(&current_page, Some(&cached)),
            "child-target"
        );
        assert_eq!(
            target_local_index_for_global_index(&state.selector_map, 1, "root-target"),
            1
        );
    }

    #[test]
    fn merged_dom_states_renumber_elements_and_preserve_targets() {
        let root = SerializedDomState::from_elements(vec![test_dom_bound_element(
            8,
            "root-target",
            "Root button",
            None,
        )])
        .with_page_stats(DomPageStats {
            interactive_elements: 1,
            total_elements: 3,
            ..DomPageStats::default()
        })
        .with_eval_root(DomEvalNode::element("html").with_children(vec![
            DomEvalNode::element("body").with_children(vec![
                DomEvalNode::element("iframe").with_attribute("title", "Child frame"),
            ]),
        ]));
        let mut child = SerializedDomState::from_elements(vec![test_dom_bound_element(
            1,
            "child-target",
            "Child input",
            Some(ElementBounds {
                x: 5,
                y: 7,
                width: 90,
                height: 20,
            }),
        )])
        .with_page_stats(DomPageStats {
            interactive_elements: 1,
            total_elements: 2,
            ..DomPageStats::default()
        })
        .with_eval_root(DomEvalNode::element("html").with_children(vec![
            DomEvalNode::element("body").with_children(vec![
                DomEvalNode::element("button")
                    .with_children(vec![DomEvalNode::text("Child input")])
                    .interactive(88),
            ]),
        ]));
        offset_dom_state_bounds(&mut child, FrameOffset { x: 100, y: 40 });

        let merged = merge_dom_states(root, vec![child]);

        assert_eq!(merged.element_count(), 2);
        assert_eq!(merged.selector_map[&1].target_id, "root-target");
        assert_eq!(merged.selector_map[&2].target_id, "child-target");
        assert_eq!(merged.selector_map[&2].name.as_deref(), Some("Child input"));
        assert_eq!(
            merged.selector_map[&2].bounds,
            Some(ElementBounds {
                x: 105,
                y: 47,
                width: 90,
                height: 20,
            })
        );
        assert_eq!(merged.page_stats.interactive_elements, 2);
        assert_eq!(merged.page_stats.total_elements, 5);
        let eval = merged.eval_representation();
        assert!(
            eval.contains("#iframe-content"),
            "merged eval tree missed iframe content: {eval}"
        );
        assert!(
            eval.contains("[i_88] <button>Child input"),
            "merged eval tree missed child backend marker: {eval}"
        );
    }

    fn test_dom_bound_element(
        index: u32,
        target_id: &str,
        name: &str,
        bounds: Option<ElementBounds>,
    ) -> DomElementRef {
        DomElementRef {
            index,
            target_id: target_id.to_owned(),
            backend_node_id: index.into(),
            node_id: Some(index.into()),
            tag_name: "button".to_owned(),
            role: Some("button".to_owned()),
            name: Some(name.to_owned()),
            text: Some(name.to_owned()),
            attributes: BTreeMap::new(),
            bounds,
            is_visible: true,
            is_interactive: true,
            is_scrollable: false,
        }
    }

    async fn spawn_static_html_server(
        body: String,
    ) -> (std::net::SocketAddr, tokio::task::JoinHandle<()>) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind static html server");
        let addr = listener.local_addr().expect("static html server address");
        let server = tokio::spawn(async move {
            loop {
                let Ok((mut stream, _)) = listener.accept().await else {
                    break;
                };
                let body = body.clone();
                tokio::spawn(async move {
                    let mut buffer = [0_u8; 1024];
                    let _ = stream.read(&mut buffer).await;
                    let response = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        body.len(),
                        body
                    );
                    let _ = stream.write_all(response.as_bytes()).await;
                });
            }
        });
        (addr, server)
    }

    #[test]
    fn url_policy_allows_internal_data_and_default_web_urls() {
        let policy = UrlAccessPolicy::default();

        assert!(policy.is_allowed("about:blank"));
        assert!(policy.is_allowed("chrome://newtab/"));
        assert!(policy.is_allowed("chrome://new-tab-page"));
        assert!(policy.is_allowed("data:text/html,<title>ok</title>"));
        assert!(policy.is_allowed("blob:https://example.com/id"));
        assert!(policy.is_allowed("https://example.com/page"));
    }

    #[test]
    fn url_policy_watchdog_closes_disallowed_new_target_events() {
        let policy = UrlAccessPolicy::from_profile(&BrowserProfile {
            prohibited_domains: vec!["evil.test".to_owned()],
            ..BrowserProfile::default()
        });
        let current_page = AttachedPage {
            target_id: "current-target".to_owned(),
            session_id: "current-session".to_owned(),
        };
        let event = CdpEvent {
            method: "Target.targetCreated".to_owned(),
            params: json!({
                "targetInfo": {
                    "type": "page",
                    "targetId": "popup-target",
                    "url": "https://evil.test/popup"
                }
            }),
            session_id: None,
        };

        assert_eq!(
            url_policy_watchdog_action_for_event(&policy, &current_page, &event),
            Some(UrlPolicyWatchdogAction::CloseTarget {
                target_id: "popup-target".to_owned(),
                url: "https://evil.test/popup".to_owned(),
                reason: "in_prohibited_domains".to_owned(),
            })
        );
    }

    #[test]
    fn url_policy_watchdog_ignores_empty_target_urls() {
        let policy = UrlAccessPolicy::from_profile(&BrowserProfile {
            prohibited_domains: vec!["evil.test".to_owned()],
            ..BrowserProfile::default()
        });
        let current_page = AttachedPage {
            target_id: "current-target".to_owned(),
            session_id: "current-session".to_owned(),
        };
        let target_event = CdpEvent {
            method: "Target.targetCreated".to_owned(),
            params: json!({
                "targetInfo": {
                    "type": "page",
                    "targetId": "popup-target",
                    "url": ""
                }
            }),
            session_id: None,
        };
        let frame_event = CdpEvent {
            method: "Page.frameNavigated".to_owned(),
            params: json!({ "frame": { "id": "frame-1", "url": "" } }),
            session_id: Some("current-session".to_owned()),
        };

        assert_eq!(
            url_policy_watchdog_action_for_event(&policy, &current_page, &target_event),
            None
        );
        assert_eq!(
            url_policy_watchdog_action_for_event(&policy, &current_page, &frame_event),
            None
        );
    }

    #[test]
    fn url_policy_watchdog_resets_current_page_navigation_events() {
        let policy = UrlAccessPolicy::from_profile(&BrowserProfile {
            allowed_domains: vec!["safe.test".to_owned()],
            ..BrowserProfile::default()
        });
        let current_page = AttachedPage {
            target_id: "current-target".to_owned(),
            session_id: "current-session".to_owned(),
        };
        let event = CdpEvent {
            method: "Page.frameNavigated".to_owned(),
            params: json!({
                "frame": {
                    "id": "frame-1",
                    "url": "https://blocked.test/redirect"
                }
            }),
            session_id: Some("current-session".to_owned()),
        };

        assert_eq!(
            url_policy_watchdog_action_for_event(&policy, &current_page, &event),
            Some(UrlPolicyWatchdogAction::ResetCurrent {
                session_id: "current-session".to_owned(),
                url: "https://blocked.test/redirect".to_owned(),
                reason: "not_in_allowed_domains".to_owned(),
            })
        );
    }

    #[tokio::test]
    async fn url_policy_duplicate_pending_blocks_can_be_cleared_after_sync_enforcement() {
        let session = test_session_for_pdf_downloads(None, false);
        *session.pending_url_policy_error.lock().await = Some(BrowserError::NavigationBlocked {
            url: "https://blocked.test/redirect".to_owned(),
            reason: "not_in_allowed_domains".to_owned(),
        });

        session
            .clear_matching_pending_url_policy_errors(&[(
                "https://blocked.test/redirect".to_owned(),
                "not_in_allowed_domains".to_owned(),
            )])
            .await;

        assert!(session.pending_url_policy_error.lock().await.is_none());

        *session.pending_url_policy_error.lock().await = Some(BrowserError::NavigationBlocked {
            url: "https://blocked.test/other".to_owned(),
            reason: "not_in_allowed_domains".to_owned(),
        });
        session
            .clear_matching_pending_url_policy_errors(&[(
                "https://blocked.test/redirect".to_owned(),
                "not_in_allowed_domains".to_owned(),
            )])
            .await;

        assert!(session.pending_url_policy_error.lock().await.is_some());
    }

    #[test]
    fn browser_security_events_format_state_diagnostics() {
        let mut events = VecDeque::new();
        push_security_event(
            &mut events,
            BrowserSecurityEvent::prevented_navigation(
                "https://blocked.test/direct".to_owned(),
                "not_in_allowed_domains".to_owned(),
            ),
        );
        push_security_event(
            &mut events,
            BrowserSecurityEvent::closed_popup(
                "https://evil.test/popup".to_owned(),
                "in_prohibited_domains".to_owned(),
            ),
        );
        push_security_event(
            &mut events,
            BrowserSecurityEvent::reset_current(
                "https://blocked.test/redirect".to_owned(),
                "not_in_allowed_domains".to_owned(),
            ),
        );
        push_security_event(
            &mut events,
            BrowserSecurityEvent::close_popup_failed(
                "https://stuck.test/popup".to_owned(),
                "in_prohibited_domains".to_owned(),
                "CDP target is already detached".to_owned(),
            ),
        );

        let (recent_events, closed_popup_messages, browser_errors) =
            security_event_state_fields(&events);
        let recent_events = recent_events.expect("recent security events");

        assert!(recent_events.contains("no browser navigation was started"));
        assert!(recent_events.contains("Closed popup https://evil.test/popup"));
        assert!(recent_events.contains("reset current tab to about:blank"));
        assert!(recent_events.contains("Failed to close popup https://stuck.test/popup"));
        assert_eq!(
            closed_popup_messages,
            vec!["Closed popup https://evil.test/popup (in_prohibited_domains)"]
        );
        assert_eq!(
            browser_errors,
            vec![
                "Failed to close popup https://stuck.test/popup (in_prohibited_domains): CDP target is already detached"
            ]
        );
        assert_eq!(
            events[0].lifecycle_event.kind,
            BrowserLifecycleEventKind::NavigationBlocked
        );
        assert_eq!(
            events[1].lifecycle_event.kind,
            BrowserLifecycleEventKind::PopupClosed
        );
        assert_eq!(
            events[2].lifecycle_event.kind,
            BrowserLifecycleEventKind::CurrentTargetReset
        );
        assert_eq!(
            events[3].lifecycle_event.kind,
            BrowserLifecycleEventKind::PopupCloseFailed
        );
    }

    #[test]
    fn browser_security_events_are_bounded() {
        let mut events = VecDeque::new();
        for index in 0..(MAX_SECURITY_EVENTS + 2) {
            push_security_event(
                &mut events,
                BrowserSecurityEvent::closed_popup(
                    format!("https://blocked-{index}.test/popup"),
                    "in_prohibited_domains".to_owned(),
                ),
            );
        }

        let (recent_events, closed_popup_messages, browser_errors) =
            security_event_state_fields(&events);
        let recent_events = recent_events.expect("recent security events");

        assert_eq!(events.len(), MAX_SECURITY_EVENTS);
        assert_eq!(closed_popup_messages.len(), MAX_SECURITY_EVENTS);
        assert!(browser_errors.is_empty());
        assert!(!recent_events.contains("blocked-0.test"));
        assert!(!recent_events.contains("blocked-1.test"));
        assert!(recent_events.contains("blocked-2.test"));
        assert!(recent_events.contains("blocked-9.test"));
    }

    #[test]
    fn browser_lifecycle_events_cover_target_and_navigation_transitions() {
        let events = vec![
            BrowserLifecycleEvent::browser_connected("http://127.0.0.1:9222"),
            BrowserLifecycleEvent::target_created("target-1", "https://example.test"),
            BrowserLifecycleEvent::target_switched("target-1"),
            BrowserLifecycleEvent::navigation_started("target-1", "https://example.test"),
            BrowserLifecycleEvent::navigation_completed("target-1", "https://example.test"),
            BrowserLifecycleEvent::target_closed("target-1"),
            BrowserSecurityEvent::reset_current(
                "https://blocked.test".to_owned(),
                "not_in_allowed_domains".to_owned(),
            )
            .lifecycle_event,
            BrowserSecurityEvent::close_popup_failed(
                "https://stuck.test/popup".to_owned(),
                "in_prohibited_domains".to_owned(),
                "No target with given id found".to_owned(),
            )
            .lifecycle_event,
        ];

        assert_eq!(
            events.iter().map(|event| &event.kind).collect::<Vec<_>>(),
            vec![
                &BrowserLifecycleEventKind::BrowserConnected,
                &BrowserLifecycleEventKind::TargetCreated,
                &BrowserLifecycleEventKind::TargetSwitched,
                &BrowserLifecycleEventKind::NavigationStarted,
                &BrowserLifecycleEventKind::NavigationCompleted,
                &BrowserLifecycleEventKind::TargetClosed,
                &BrowserLifecycleEventKind::CurrentTargetReset,
                &BrowserLifecycleEventKind::PopupCloseFailed,
            ]
        );
        assert_eq!(events[1].target_id.as_deref(), Some("target-1"));
        assert_eq!(events[3].url.as_deref(), Some("https://example.test"));
        assert_eq!(events[6].reason.as_deref(), Some("not_in_allowed_domains"));
        assert_eq!(
            events[7].error.as_deref(),
            Some("No target with given id found")
        );

        let json = serde_json::to_value(&events).expect("serialize lifecycle events");
        assert_eq!(json[0]["kind"], "browser_connected");
        assert_eq!(json[4]["kind"], "navigation_completed");
        assert!(json[4].get("details").is_none());
    }

    #[test]
    fn browser_lifecycle_events_cover_remaining_upstream_shapes() {
        let events = vec![
            BrowserLifecycleEvent::browser_reconnecting("http://127.0.0.1:9222", 2, 3),
            BrowserLifecycleEvent::browser_reconnected("http://127.0.0.1:9222", 2, "1.25"),
            BrowserLifecycleEvent::target_crashed("target-1", "Inspector target crashed"),
            BrowserLifecycleEvent::navigation_failed(
                "target-1",
                "https://example.test/slow",
                "net::ERR_FAILED",
            ),
            BrowserLifecycleEvent::network_timeout("target-1", "https://example.test/slow", "8"),
            BrowserLifecycleEvent::javascript_dialog_handled(
                "https://example.test",
                "confirm",
                "Continue?",
                true,
            ),
            BrowserLifecycleEvent::download_started(
                "download-guid",
                "https://example.test/report.pdf",
                "report.pdf",
            ),
            BrowserLifecycleEvent::download_progress(
                "download-guid",
                1024,
                Some(4096),
                "inProgress",
            ),
            BrowserLifecycleEvent::file_downloaded(
                "download-guid",
                "/tmp/report.pdf",
                "report.pdf",
                4096,
            ),
            BrowserLifecycleEvent::storage_state_saved("/tmp/storage.json", 4, 2),
            BrowserLifecycleEvent::storage_state_loaded("/tmp/storage.json", 4, 2),
            BrowserLifecycleEvent::browser_stopped("graceful_stop"),
        ];

        assert_eq!(
            events.iter().map(|event| &event.kind).collect::<Vec<_>>(),
            vec![
                &BrowserLifecycleEventKind::BrowserReconnecting,
                &BrowserLifecycleEventKind::BrowserReconnected,
                &BrowserLifecycleEventKind::TargetCrashed,
                &BrowserLifecycleEventKind::NavigationFailed,
                &BrowserLifecycleEventKind::NetworkTimeout,
                &BrowserLifecycleEventKind::JavaScriptDialogHandled,
                &BrowserLifecycleEventKind::DownloadStarted,
                &BrowserLifecycleEventKind::DownloadProgress,
                &BrowserLifecycleEventKind::FileDownloaded,
                &BrowserLifecycleEventKind::StorageStateSaved,
                &BrowserLifecycleEventKind::StorageStateLoaded,
                &BrowserLifecycleEventKind::BrowserStopped,
            ]
        );

        assert_eq!(events[0].details["attempt"], "2");
        assert_eq!(events[1].details["downtime_seconds"], "1.25");
        assert_eq!(events[5].details["dialog_message"], "Continue?".to_owned());
        assert_eq!(events[7].details["total_bytes"], "4096");
        assert_eq!(events[9].details["cookies_count"], "4");

        let json = serde_json::to_value(&events).expect("serialize lifecycle events");
        assert_eq!(json[2]["kind"], "target_crashed");
        assert_eq!(json[5]["details"]["action"], "accepted");
        assert_eq!(json[8]["details"]["file_name"], "report.pdf");
    }

    #[test]
    fn browser_lifecycle_adapter_events_map_upstream_taxonomy() {
        let events = vec![
            BrowserLifecycleEvent::browser_close_requested(),
            BrowserLifecycleEvent::browser_connected("http://127.0.0.1:9222"),
            BrowserLifecycleEvent::browser_stopped("graceful_stop"),
            BrowserLifecycleEvent::browser_reconnecting("http://127.0.0.1:9222", 1, 3),
            BrowserLifecycleEvent::browser_reconnected("http://127.0.0.1:9222", 1, "0.250"),
            BrowserLifecycleEvent::target_created("target-1", "https://example.test"),
            BrowserLifecycleEvent::target_closed("target-1"),
            BrowserLifecycleEvent::target_switched("target-1"),
            BrowserLifecycleEvent::target_crashed("target-1", "Inspector target crashed"),
            BrowserLifecycleEvent::navigation_started("target-1", "https://example.test"),
            BrowserLifecycleEvent::navigation_completed("target-1", "https://example.test"),
            BrowserLifecycleEvent::navigation_failed(
                "target-1",
                "https://example.test",
                "net::ERR_FAILED",
            ),
            BrowserLifecycleEvent::network_timeout("target-1", "https://example.test", "8"),
            BrowserSecurityEvent::prevented_navigation(
                "https://blocked.test".to_owned(),
                "not_in_allowed_domains".to_owned(),
            )
            .lifecycle_event,
            BrowserSecurityEvent::reset_current(
                "https://blocked.test".to_owned(),
                "not_in_allowed_domains".to_owned(),
            )
            .lifecycle_event,
            BrowserSecurityEvent::reset_current_failed(
                "https://blocked.test".to_owned(),
                "not_in_allowed_domains".to_owned(),
                "reset failed".to_owned(),
            )
            .lifecycle_event,
            BrowserSecurityEvent::closed_popup(
                "https://blocked.test/popup".to_owned(),
                "in_prohibited_domains".to_owned(),
            )
            .lifecycle_event,
            BrowserSecurityEvent::close_popup_failed(
                "https://stuck.test/popup".to_owned(),
                "in_prohibited_domains".to_owned(),
                "No target with given id found".to_owned(),
            )
            .lifecycle_event,
            BrowserLifecycleEvent::javascript_dialog_handled(
                "https://example.test",
                "alert",
                "Hello",
                false,
            ),
            BrowserLifecycleEvent::download_started(
                "download-guid",
                "https://example.test/report.pdf",
                "report.pdf",
            ),
            BrowserLifecycleEvent::download_progress(
                "download-guid",
                1024,
                Some(4096),
                "inProgress",
            ),
            BrowserLifecycleEvent::file_downloaded(
                "download-guid",
                "/tmp/report.pdf",
                "report.pdf",
                4096,
            ),
            BrowserLifecycleEvent::storage_state_saved("/tmp/storage.json", 4, 2),
            BrowserLifecycleEvent::storage_state_loaded("/tmp/storage.json", 4, 2),
        ];

        let adapter_events = browser_lifecycle_adapter_events(&events);

        assert_eq!(
            adapter_events
                .iter()
                .map(|event| &event.kind)
                .collect::<Vec<_>>(),
            vec![
                &BrowserLifecycleAdapterEventKind::BrowserStop,
                &BrowserLifecycleAdapterEventKind::BrowserConnected,
                &BrowserLifecycleAdapterEventKind::BrowserStopped,
                &BrowserLifecycleAdapterEventKind::BrowserReconnecting,
                &BrowserLifecycleAdapterEventKind::BrowserReconnected,
                &BrowserLifecycleAdapterEventKind::TabCreated,
                &BrowserLifecycleAdapterEventKind::TabClosed,
                &BrowserLifecycleAdapterEventKind::AgentFocusChanged,
                &BrowserLifecycleAdapterEventKind::TargetCrashed,
                &BrowserLifecycleAdapterEventKind::NavigationStarted,
                &BrowserLifecycleAdapterEventKind::NavigationComplete,
                &BrowserLifecycleAdapterEventKind::BrowserError,
                &BrowserLifecycleAdapterEventKind::BrowserError,
                &BrowserLifecycleAdapterEventKind::BrowserError,
                &BrowserLifecycleAdapterEventKind::BrowserDiagnostic,
                &BrowserLifecycleAdapterEventKind::BrowserError,
                &BrowserLifecycleAdapterEventKind::BrowserDiagnostic,
                &BrowserLifecycleAdapterEventKind::BrowserError,
                &BrowserLifecycleAdapterEventKind::JavaScriptDialogHandled,
                &BrowserLifecycleAdapterEventKind::DownloadStarted,
                &BrowserLifecycleAdapterEventKind::DownloadProgress,
                &BrowserLifecycleAdapterEventKind::FileDownloaded,
                &BrowserLifecycleAdapterEventKind::StorageState,
                &BrowserLifecycleAdapterEventKind::StorageState,
            ]
        );
        assert_eq!(
            adapter_events[7].source_kind,
            BrowserLifecycleEventKind::TargetSwitched
        );
        assert_eq!(adapter_events[7].target_id.as_deref(), Some("target-1"));
        assert_eq!(
            adapter_events[14].source_kind,
            BrowserLifecycleEventKind::CurrentTargetReset
        );

        let json = serde_json::to_value(&adapter_events).expect("serialize adapter events");
        assert_eq!(json[7]["kind"], "agent_focus_changed");
        assert_eq!(json[10]["kind"], "navigation_complete");
        assert_eq!(json[10]["source_kind"], "navigation_completed");
    }

    #[tokio::test]
    async fn lifecycle_adapter_subscription_maps_facade_events() {
        let (event_tx, event_rx) = broadcast::channel(4);
        let subscription = BrowserLifecycleEventSubscription::new(event_rx);
        let mut adapter_subscription = BrowserLifecycleAdapterEventSubscription::new(subscription);

        assert_eq!(adapter_subscription.try_recv().expect("empty stream"), None);

        event_tx
            .send(BrowserLifecycleEvent::target_switched("target-1"))
            .expect("send lifecycle event");

        let event = adapter_subscription
            .recv()
            .await
            .expect("adapter lifecycle event");
        assert_eq!(
            event.kind,
            BrowserLifecycleAdapterEventKind::AgentFocusChanged
        );
        assert_eq!(event.source_kind, BrowserLifecycleEventKind::TargetSwitched);
        assert_eq!(event.target_id.as_deref(), Some("target-1"));
    }

    #[test]
    fn lifecycle_watchdog_maps_cdp_crash_and_download_events() {
        let mut active_requests = HashMap::new();
        track_network_request(
            &mut active_requests,
            &CdpEvent {
                method: "Network.requestWillBeSent".to_owned(),
                params: json!({
                    "requestId": "request-1",
                    "request": {
                        "url": "https://example.test/api/report",
                        "method": "POST"
                    },
                    "type": "Fetch",
                }),
                session_id: Some("session-1".to_owned()),
            },
        );
        let started_at = active_requests
            .get_mut("request-1")
            .expect("tracked request")
            .started_at;
        active_requests
            .get_mut("request-1")
            .expect("tracked request")
            .started_at = started_at - Duration::from_secs(11);
        let timeout_events = lifecycle_events_for_timed_out_network_requests(
            &mut active_requests,
            started_at,
            Duration::from_secs(10),
        );
        assert_eq!(timeout_events.len(), 1);
        assert_eq!(
            timeout_events[0].kind,
            BrowserLifecycleEventKind::NetworkTimeout
        );
        assert_eq!(timeout_events[0].details["request_id"], "request-1");
        assert!(active_requests.is_empty());

        let websocket_closed = lifecycle_event_for_websocket_closed(&CdpEvent {
            method: "browser-use-rs.websocket-closed".to_owned(),
            params: json!({
                "reason": "websocket_error",
                "error": "connection reset",
            }),
            session_id: None,
        });
        assert_eq!(
            websocket_closed.kind,
            BrowserLifecycleEventKind::BrowserStopped
        );
        assert_eq!(websocket_closed.reason.as_deref(), Some("websocket_error"));
        assert_eq!(websocket_closed.error.as_deref(), Some("connection reset"));
        assert!(should_reconnect_after_websocket_event(
            &CdpEvent {
                method: "browser-use-rs.websocket-closed".to_owned(),
                params: json!({ "reason": "websocket_stream_ended" }),
                session_id: None,
            },
            false,
            false,
        ));
        assert!(!should_reconnect_after_websocket_event(
            &CdpEvent {
                method: "browser-use-rs.websocket-closed".to_owned(),
                params: json!({ "reason": "connection_actor_stopped" }),
                session_id: None,
            },
            false,
            false,
        ));
        assert!(!should_reconnect_after_websocket_event(
            &CdpEvent {
                method: "browser-use-rs.websocket-closed".to_owned(),
                params: json!({ "reason": "websocket_error" }),
                session_id: None,
            },
            true,
            false,
        ));
        assert_eq!(
            cdp_reconnect_delay_for_attempt(4),
            Duration::from_millis(4_000)
        );

        let reconnecting = lifecycle_event_for_websocket_reconnecting(
            &cdp_websocket_reconnecting_event("http://127.0.0.1:9222", 2, 3),
        )
        .expect("reconnecting lifecycle event");
        assert_eq!(
            reconnecting.kind,
            BrowserLifecycleEventKind::BrowserReconnecting
        );
        assert_eq!(reconnecting.details["attempt"], "2");
        assert_eq!(reconnecting.details["max_attempts"], "3");

        let reconnected =
            lifecycle_event_for_websocket_reconnected(&cdp_websocket_reconnected_event(
                "http://127.0.0.1:9222",
                2,
                Duration::from_millis(1_250),
                4,
            ))
            .expect("reconnected lifecycle event");
        assert_eq!(
            reconnected.kind,
            BrowserLifecycleEventKind::BrowserReconnected
        );
        assert_eq!(reconnected.details["downtime_seconds"], "1.250");
        assert_eq!(reconnected.details["connection_generation"], "4");

        let reconnect_failed =
            lifecycle_event_for_websocket_reconnect_failed(&cdp_websocket_reconnect_failed_event(
                "http://127.0.0.1:9222",
                3,
                Duration::from_millis(7_000),
                Some("connection refused".to_owned()),
            ));
        assert_eq!(
            reconnect_failed.kind,
            BrowserLifecycleEventKind::BrowserStopped
        );
        assert_eq!(reconnect_failed.reason.as_deref(), Some("reconnect_failed"));
        assert_eq!(
            reconnect_failed.error.as_deref(),
            Some("connection refused")
        );

        let crash_event = CdpEvent {
            method: "Target.targetCrashed".to_owned(),
            params: json!({
                "targetId": "target-1",
                "status": "crashed",
                "errorCode": 139,
            }),
            session_id: Some("session-1".to_owned()),
        };
        let crash_events = lifecycle_events_for_target_crash(&crash_event);
        assert_eq!(crash_events.len(), 1);
        assert_eq!(
            crash_events[0].kind,
            BrowserLifecycleEventKind::TargetCrashed
        );
        assert_eq!(crash_events[0].target_id.as_deref(), Some("target-1"));
        assert_eq!(crash_events[0].error.as_deref(), Some("crashed (139)"));
        assert_eq!(crash_events[0].details["session_id"], "session-1");

        let download_start = lifecycle_event_for_download_start(&CdpEvent {
            method: "Browser.downloadWillBegin".to_owned(),
            params: json!({
                "guid": "download-guid",
                "url": "https://example.test/report.pdf",
                "suggestedFilename": "report.pdf",
            }),
            session_id: None,
        })
        .expect("download start event");
        assert_eq!(
            download_start.kind,
            BrowserLifecycleEventKind::DownloadStarted
        );
        assert_eq!(download_start.details["suggested_filename"], "report.pdf");

        let sanitized_download_start = lifecycle_event_for_download_start(&CdpEvent {
            method: "Browser.downloadWillBegin".to_owned(),
            params: json!({
                "guid": "download-guid",
                "url": "https://example.test/report.pdf",
                "suggestedFilename": "../../etc/passwd",
            }),
            session_id: None,
        })
        .expect("sanitized download start event");
        assert_eq!(
            sanitized_download_start.details["suggested_filename"],
            "passwd"
        );

        let download_progress = lifecycle_events_for_download_progress(&CdpEvent {
            method: "Browser.downloadProgress".to_owned(),
            params: json!({
                "guid": "download-guid",
                "receivedBytes": 4096,
                "totalBytes": 4096,
                "state": "completed",
                "filePath": "/tmp/report.pdf",
            }),
            session_id: None,
        });
        assert_eq!(
            download_progress
                .iter()
                .map(|event| &event.kind)
                .collect::<Vec<_>>(),
            vec![
                &BrowserLifecycleEventKind::DownloadProgress,
                &BrowserLifecycleEventKind::FileDownloaded,
            ]
        );
        assert_eq!(download_progress[1].details["file_name"], "report.pdf");

        let sanitized_download_progress = lifecycle_events_for_download_progress(&CdpEvent {
            method: "Browser.downloadProgress".to_owned(),
            params: json!({
                "guid": "download-guid",
                "receivedBytes": 4096,
                "totalBytes": 4096,
                "state": "completed",
                "filePath": "/tmp/../../escape.bin",
            }),
            session_id: None,
        });
        assert_eq!(
            sanitized_download_progress[1].details["file_name"],
            "escape.bin"
        );
    }

    #[test]
    fn download_filename_sanitization_matches_upstream_security_boundary() {
        assert_eq!(sanitize_download_filename("../../etc/passwd"), "passwd");
        assert_eq!(sanitize_download_filename("/etc/shadow"), "shadow");
        assert_eq!(
            sanitize_download_filename("..\\..\\Windows\\System32\\config.txt"),
            "config.txt"
        );
        assert_eq!(sanitize_download_filename("a/b\\c/../d.pdf"), "d.pdf");
        for malicious in ["..", ".", "/", "\\", "../", "..\\", "/.", "\\.", "/.."] {
            assert_eq!(
                sanitize_download_filename(malicious),
                "download",
                "{malicious:?} should fall back to default"
            );
        }
        assert_eq!(sanitize_download_filename("file.txt\0.exe"), "file.txt.exe");
        assert_eq!(sanitize_download_filename(""), "download");
        assert_eq!(sanitize_download_filename("report.pdf"), "report.pdf");
        assert_eq!(
            sanitize_download_filename("file with spaces.pdf"),
            "file with spaces.pdf"
        );
        assert_eq!(sanitize_download_filename(".bashrc"), ".bashrc");
        assert_eq!(sanitize_download_filename("résumé.pdf"), "résumé.pdf");
        assert_eq!(sanitize_download_filename("文档.pdf"), "文档.pdf");
    }

    #[test]
    fn pdf_viewer_url_detection_is_conservative() {
        assert!(is_pdf_viewer_url("https://example.test/report.pdf"));
        assert!(is_pdf_viewer_url(
            "https://example.test/report.PDF?download=1#page=2"
        ));
        assert!(is_pdf_viewer_url("https://example.test/viewer/pdf/123"));
        assert!(!is_pdf_viewer_url("https://example.test/report.html"));
        assert!(!is_pdf_viewer_url(
            "https://example.test/report.html?file=report.pdf"
        ));
    }

    #[test]
    fn pdf_download_filename_uses_safe_pdf_basename() {
        assert_eq!(
            pdf_download_filename_from_url("https://example.test/docs/report.pdf?x=1"),
            "report.pdf"
        );
        assert_eq!(
            pdf_download_filename_from_url("https://example.test/pdf/monthly%20report"),
            "monthly report.pdf"
        );
        assert_eq!(
            pdf_download_filename_from_url("https://example.test/docs/..%2Fsecret.pdf"),
            "secret.pdf"
        );
    }

    #[test]
    fn cdp_auto_pdf_candidate_uses_response_metadata() {
        let event = CdpEvent {
            method: "Network.responseReceived".to_owned(),
            params: json!({
                "requestId": "request-1",
                "response": {
                    "url": "https://example.test/download?id=123",
                    "mimeType": "application/pdf",
                    "headers": {
                        "Content-Disposition": "attachment; filename*=UTF-8''report%20final",
                        "Content-Type": "application/pdf; charset=binary"
                    }
                }
            }),
            session_id: Some("session-1".to_owned()),
        };

        let candidate = cdp_auto_pdf_candidate_from_response(&event).expect("pdf candidate");
        assert_eq!(candidate.request_id, "request-1");
        assert_eq!(candidate.request_key, "session-1:request-1");
        assert_eq!(candidate.session_id.as_deref(), Some("session-1"));
        assert_eq!(candidate.url, "https://example.test/download?id=123");
        assert_eq!(candidate.file_name, "report final.pdf");
    }

    #[test]
    fn cdp_auto_pdf_candidate_ignores_non_pdf_responses() {
        let event = CdpEvent {
            method: "Network.responseReceived".to_owned(),
            params: json!({
                "requestId": "request-1",
                "response": {
                    "url": "https://example.test/index.html?file=report.pdf",
                    "mimeType": "text/html",
                    "headers": {
                        "Content-Type": "text/html"
                    }
                }
            }),
            session_id: None,
        };

        assert!(cdp_auto_pdf_candidate_from_response(&event).is_none());
    }

    #[tokio::test]
    async fn cdp_auto_pdf_state_deduplicates_url_cache_and_paths() {
        let temp_dir = TempDir::new().expect("downloads dir");
        let downloaded_urls = Arc::new(Mutex::new(BTreeMap::new()));
        let state = Arc::new(CdpAutoPdfDownloadState {
            downloads_path: temp_dir.path().to_path_buf(),
            downloaded_urls,
            candidates: Mutex::new(BTreeMap::new()),
        });
        let response_event = CdpEvent {
            method: "Network.responseReceived".to_owned(),
            params: json!({
                "requestId": "request-1",
                "response": {
                    "url": "https://example.test/report.pdf",
                    "mimeType": "application/pdf",
                    "headers": {}
                }
            }),
            session_id: Some("session-1".to_owned()),
        };
        let finish_event = CdpEvent {
            method: "Network.loadingFinished".to_owned(),
            params: json!({ "requestId": "request-1" }),
            session_id: Some("session-1".to_owned()),
        };

        state.observe_response(&response_event).await;
        let candidate = state
            .take_finished_candidate(&finish_event)
            .await
            .expect("first candidate");
        let event = state
            .write_candidate(&candidate, b"%PDF-1.7")
            .await
            .expect("write pdf");
        assert_eq!(event.details["file_name"], "report.pdf");
        let first_path = temp_dir.path().join("report.pdf");
        assert!(first_path.exists());

        state.observe_response(&response_event).await;
        assert!(state.take_finished_candidate(&finish_event).await.is_none());

        std::fs::remove_file(&first_path).expect("remove cached pdf");
        state.observe_response(&response_event).await;
        let second_candidate = state
            .take_finished_candidate(&finish_event)
            .await
            .expect("stale cache redownload candidate");
        std::fs::write(&first_path, b"existing").expect("seed duplicate filename");
        let second = state
            .write_candidate(&second_candidate, b"%PDF-1.7")
            .await
            .expect("write deduped pdf");
        assert_eq!(second.details["file_name"], "report-1.pdf");
    }

    #[tokio::test]
    async fn cdp_auto_pdf_lifecycle_downloads_response_body() {
        let temp_dir = TempDir::new().expect("downloads dir");
        let (endpoint, command_log) = cdp_command_test_server(None, 1).await;
        let connection = CdpConnection::connect(&endpoint)
            .await
            .expect("connect cdp");
        let downloaded_urls = Arc::new(Mutex::new(BTreeMap::new()));
        let state = Arc::new(CdpAutoPdfDownloadState {
            downloads_path: temp_dir.path().to_path_buf(),
            downloaded_urls,
            candidates: Mutex::new(BTreeMap::new()),
        });
        let response_event = CdpEvent {
            method: "Network.responseReceived".to_owned(),
            params: json!({
                "requestId": "request-1",
                "response": {
                    "url": "https://example.test/download",
                    "mimeType": "application/octet-stream",
                    "headers": {
                        "Content-Disposition": "attachment; filename=cdp-report.pdf",
                        "Content-Type": "application/pdf"
                    }
                }
            }),
            session_id: Some("session-1".to_owned()),
        };
        let finish_event = CdpEvent {
            method: "Network.loadingFinished".to_owned(),
            params: json!({ "requestId": "request-1" }),
            session_id: Some("session-1".to_owned()),
        };

        state.observe_response(&response_event).await;
        let auto_pdf_download = Some(state);
        let event = cdp_auto_pdf_lifecycle_event(&connection, &auto_pdf_download, &finish_event)
            .await
            .expect("auto PDF lifecycle event");
        assert_eq!(event.kind, BrowserLifecycleEventKind::FileDownloaded);
        assert_eq!(event.reason.as_deref(), Some("pdf_auto_download"));
        assert_eq!(event.details["file_name"], "cdp-report.pdf");
        assert_eq!(event.details["file_size"], "17");
        assert_eq!(
            tokio::fs::read(temp_dir.path().join("cdp-report.pdf"))
                .await
                .expect("downloaded pdf bytes"),
            b"%PDF-1.7 cdp body"
        );

        let commands = command_log.await.expect("cdp command log");
        assert_eq!(commands[0].method, "Network.getResponseBody");
        assert_eq!(commands[0].params, json!({ "requestId": "request-1" }));
        assert_eq!(commands[0].session_id.as_deref(), Some("session-1"));
    }

    #[tokio::test]
    async fn auto_pdf_download_writes_once_and_reuses_session_cache() {
        let temp_dir = TempDir::new().expect("downloads dir");
        let (url, hits) = pdf_download_test_server(b"%PDF-1.4 test").await;
        let session = test_session_for_pdf_downloads(Some(temp_dir.path().to_path_buf()), true);

        let event = session
            .auto_download_pdf(&url, temp_dir.path())
            .await
            .expect("download PDF")
            .expect("first download event");
        assert_eq!(event.kind, BrowserLifecycleEventKind::FileDownloaded);
        assert_eq!(event.reason.as_deref(), Some("pdf_auto_download"));
        assert_eq!(event.details["auto_download"], "true");
        assert_eq!(event.details["file_name"], "report.pdf");
        assert_eq!(hits.await.expect("PDF server hits"), 1);
        let downloaded_path = temp_dir.path().join("report.pdf");
        assert_eq!(
            tokio::fs::read(&downloaded_path)
                .await
                .expect("downloaded PDF bytes"),
            b"%PDF-1.4 test"
        );

        let duplicate = session
            .auto_download_pdf(&url, temp_dir.path())
            .await
            .expect("duplicate cache lookup");
        assert!(duplicate.is_none());
    }

    #[tokio::test]
    async fn disabled_auto_pdf_download_does_not_touch_downloads_path() {
        let temp_dir = TempDir::new().expect("downloads dir");
        let session = test_session_for_pdf_downloads(Some(temp_dir.path().to_path_buf()), false);

        session
            .auto_download_pdf_if_needed("https://example.test/report.pdf")
            .await;

        assert!(
            std::fs::read_dir(temp_dir.path())
                .expect("downloads dir entries")
                .next()
                .is_none()
        );
        assert!(session.lifecycle_events().await.is_empty());
    }

    #[tokio::test]
    async fn unique_download_path_avoids_existing_files_inside_download_dir() {
        let temp_dir = TempDir::new().expect("downloads dir");
        let existing = temp_dir.path().join("report.pdf");
        tokio::fs::write(&existing, b"existing")
            .await
            .expect("write existing PDF");

        let next = unique_download_path(temp_dir.path(), "../../report.pdf")
            .await
            .expect("unique path");
        tokio::fs::write(&next, b"new")
            .await
            .expect("write unique PDF");

        assert_eq!(next, temp_dir.path().join("report-1.pdf"));
        assert!(is_path_contained(&next, temp_dir.path()));
    }

    #[test]
    fn path_containment_rejects_directory_escape() {
        let temp_dir = TempDir::new().expect("temp downloads dir");
        let downloads_dir = temp_dir.path();
        let nested_dir = downloads_dir.join("nested");
        std::fs::create_dir(&nested_dir).expect("nested downloads dir");
        let nested_file = nested_dir.join("report.pdf");
        std::fs::write(&nested_file, b"pdf").expect("nested file");

        assert!(is_path_contained(downloads_dir, downloads_dir));
        assert!(is_path_contained(&nested_file, downloads_dir));
        assert!(!is_path_contained(
            &downloads_dir.join("../escape.bin"),
            downloads_dir
        ));

        let sibling_dir = downloads_dir
            .parent()
            .expect("downloads dir parent")
            .join(format!(
                "{}_sibling",
                downloads_dir
                    .file_name()
                    .and_then(|name| name.to_str())
                    .expect("downloads dir name")
            ));
        std::fs::create_dir(&sibling_dir).expect("sibling dir");
        let sibling_file = sibling_dir.join("report.pdf");
        std::fs::write(&sibling_file, b"pdf").expect("sibling file");
        assert!(!is_path_contained(&sibling_file, downloads_dir));
    }

    #[tokio::test]
    async fn cdp_connection_rejects_stale_registered_sessions() {
        let (request_tx, _request_rx) = mpsc::channel(1);
        let (event_tx, _) = broadcast::channel(1);
        let connection = CdpConnection {
            request_tx,
            event_tx,
            next_id: AtomicU64::new(1),
            intentional_stop: Arc::new(AtomicBool::new(false)),
            connection_generation: Arc::new(AtomicU64::new(0)),
            session_generations: Arc::new(Mutex::new(HashMap::new())),
        };

        connection.register_attached_session("session-1").await;
        connection
            .ensure_session_generation_current(Some("session-1"))
            .await
            .expect("session is current before reconnect");
        connection
            .connection_generation
            .fetch_add(1, Ordering::Relaxed);

        let error = connection
            .ensure_session_generation_current(Some("session-1"))
            .await
            .expect_err("session is stale after reconnect generation advances");
        assert!(matches!(error, BrowserError::Transport(_)));
        assert!(error.to_string().contains("stale after reconnect"));

        connection
            .ensure_session_generation_current(Some("unknown-session"))
            .await
            .expect("unknown sessions are left to Chrome");
    }

    #[test]
    fn storage_state_counts_browser_use_shape() {
        let storage_state = json!({
                "cookies": [
                    { "name": "sid", "value": "1", "domain": ".example.test", "path": "/" },
                    { "name": "pref", "value": "dark", "domain": ".example.test", "path": "/" }
                ],
                "origins": [
                    {
                        "origin": "https://example.test",
                        "localStorage": [{ "name": "theme", "value": "dark" }],
                        "sessionStorage": [{ "name": "tab", "value": "reports" }]
                    }
                ]
        });
        assert_eq!(storage_state_counts(&storage_state), (2, 1));
        assert_eq!(storage_state_counts(&json!({})), (0, 0));

        let script = origin_storage_apply_script(&storage_state["origins"][0])
            .expect("origin storage apply script");
        assert!(script.contains(r#"const expectedOrigin = "https://example.test";"#));
        assert!(script.contains(r#""theme":"dark""#));
        assert!(script.contains(r#""tab":"reports""#));
        assert!(
            origin_storage_apply_script(&json!({
                "origin": "https://empty.test",
                "localStorage": [],
                "sessionStorage": []
            }))
            .is_none()
        );

        let frame_tree = json!({
            "frameTree": {
                "frame": {
                    "id": "root",
                    "url": "https://example.test/dashboard",
                    "securityOrigin": "https://example.test"
                },
                "childFrames": [
                    {
                        "frame": {
                            "id": "child-1",
                            "url": "https://accounts.example.test/login"
                        }
                    },
                    {
                        "frame": {
                            "id": "child-2",
                            "url": "about:blank",
                            "securityOrigin": "null"
                        }
                    }
                ]
            }
        });
        assert_eq!(
            frame_security_origins_from_result(&frame_tree)
                .into_iter()
                .collect::<Vec<_>>(),
            vec![
                "https://accounts.example.test".to_owned(),
                "https://example.test".to_owned()
            ]
        );

        let dom_storage_items = dom_storage_entries_to_items(Some(&json!([
            ["zeta", "last"],
            ["alpha", "first"],
            ["ignored"],
        ])));
        assert_eq!(
            dom_storage_items,
            vec![
                json!({ "name": "alpha", "value": "first" }),
                json!({ "name": "zeta", "value": "last" }),
            ]
        );

        let mut origin_states = BTreeMap::new();
        upsert_origin_storage_state(
            &mut origin_states,
            json!({
                "origin": "https://example.test",
                "localStorage": [{ "name": "theme", "value": "dark" }],
                "sessionStorage": []
            }),
        );
        upsert_origin_storage_state(
            &mut origin_states,
            json!({
                "origin": "https://example.test",
                "localStorage": [{ "name": "theme", "value": "light" }],
                "sessionStorage": [{ "name": "tab", "value": "reports" }]
            }),
        );
        assert_eq!(
            origin_states["https://example.test"]["localStorage"],
            json!([{ "name": "theme", "value": "light" }])
        );
        assert_eq!(
            origin_states["https://example.test"]["sessionStorage"],
            json!([{ "name": "tab", "value": "reports" }])
        );
    }

    #[test]
    fn storage_state_origin_discovery_uses_frame_tree_boundary_like_upstream() {
        let frame_tree = json!({
            "frameTree": {
                "frame": {
                    "id": "root",
                    "url": "https://app.example.test/dashboard",
                    "securityOrigin": "https://app.example.test"
                },
                "childFrames": [
                    {
                        "frame": {
                            "id": "login",
                            "url": "https://login.example.test/embedded",
                            "securityOrigin": "https://login.example.test"
                        }
                    },
                    {
                        "frame": {
                            "id": "opaque",
                            "url": "about:blank",
                            "securityOrigin": "null"
                        }
                    },
                    {
                        "frame": {
                            "id": "browser-internal",
                            "url": "chrome://settings",
                            "securityOrigin": "chrome://settings"
                        }
                    }
                ]
            }
        });

        assert_eq!(
            frame_security_origins_from_result(&frame_tree)
                .into_iter()
                .collect::<Vec<_>>(),
            vec![
                "https://app.example.test".to_owned(),
                "https://login.example.test".to_owned()
            ]
        );
        assert_eq!(
            frame_security_origins_from_result(&json!({}))
                .into_iter()
                .collect::<Vec<_>>(),
            Vec::<String>::new()
        );
    }

    #[test]
    fn browser_lifecycle_events_are_bounded() {
        let mut events = VecDeque::new();
        for index in 0..(MAX_LIFECYCLE_EVENTS + 2) {
            push_lifecycle_event(
                &mut events,
                BrowserLifecycleEvent::navigation_completed(
                    format!("target-{index}"),
                    format!("https://example.test/{index}"),
                ),
            );
        }

        assert_eq!(events.len(), MAX_LIFECYCLE_EVENTS);
        assert_eq!(events[0].target_id.as_deref(), Some("target-2"));
        assert_eq!(
            events.back().and_then(|event| event.target_id.as_deref()),
            Some("target-33")
        );
    }

    #[test]
    fn lifecycle_event_bus_publishes_while_history_stays_bounded() {
        let (event_tx, mut event_rx) = broadcast::channel(64);
        let mut events = VecDeque::new();

        for index in 0..(MAX_LIFECYCLE_EVENTS + 2) {
            push_lifecycle_event_and_publish(
                &mut events,
                &event_tx,
                BrowserLifecycleEvent::navigation_completed(
                    format!("target-{index}"),
                    format!("https://example.test/{index}"),
                ),
            );
        }

        let mut received_targets = Vec::new();
        for _ in 0..(MAX_LIFECYCLE_EVENTS + 2) {
            let event = event_rx.try_recv().expect("published lifecycle event");
            received_targets.push(event.target_id.expect("target id"));
        }

        assert_eq!(events.len(), MAX_LIFECYCLE_EVENTS);
        assert_eq!(events[0].target_id.as_deref(), Some("target-2"));
        assert_eq!(
            events.back().and_then(|event| event.target_id.as_deref()),
            Some("target-33")
        );
        assert_eq!(
            received_targets.first().map(String::as_str),
            Some("target-0")
        );
        assert_eq!(
            received_targets.last().map(String::as_str),
            Some("target-33")
        );
        assert!(matches!(
            event_rx.try_recv(),
            Err(broadcast::error::TryRecvError::Empty)
        ));
    }

    #[tokio::test]
    async fn lifecycle_event_subscription_hides_broadcast_empty_and_closed_states() {
        let (event_tx, event_rx) = broadcast::channel(4);
        let mut subscription = BrowserLifecycleEventSubscription::new(event_rx);

        assert_eq!(subscription.try_recv().expect("empty stream"), None);

        let event = BrowserLifecycleEvent::target_switched("target-1");
        event_tx.send(event.clone()).expect("event sent");
        assert_eq!(
            subscription.try_recv().expect("published event"),
            Some(event)
        );

        drop(event_tx);
        assert!(matches!(
            subscription.recv().await,
            Err(BrowserLifecycleEventStreamError::Closed)
        ));
    }

    #[tokio::test]
    async fn lifecycle_event_subscription_reports_lagged_consumers() {
        let (event_tx, event_rx) = broadcast::channel(1);
        let mut subscription = BrowserLifecycleEventSubscription::new(event_rx);

        event_tx
            .send(BrowserLifecycleEvent::target_switched("target-1"))
            .expect("first event sent");
        event_tx
            .send(BrowserLifecycleEvent::target_switched("target-2"))
            .expect("second event sent");

        assert!(matches!(
            subscription.try_recv(),
            Err(BrowserLifecycleEventStreamError::Lagged(_))
        ));
        assert_eq!(
            subscription.try_recv().expect("latest event"),
            Some(BrowserLifecycleEvent::target_switched("target-2"))
        );
    }

    #[tokio::test]
    async fn lifecycle_event_subscription_resubscribes_at_current_tail() {
        let (event_tx, event_rx) = broadcast::channel(4);
        let mut subscription = BrowserLifecycleEventSubscription::new(event_rx);

        let first_event = BrowserLifecycleEvent::target_switched("target-1");
        event_tx
            .send(first_event.clone())
            .expect("first event sent");
        assert_eq!(subscription.recv().await.expect("first event"), first_event);

        let mut resubscribed = subscription.resubscribe();
        let second_event = BrowserLifecycleEvent::target_switched("target-2");
        event_tx
            .send(second_event.clone())
            .expect("second event sent");

        assert_eq!(
            resubscribed.recv().await.expect("resubscribed event"),
            second_event
        );
    }

    #[tokio::test]
    async fn closed_lifecycle_event_subscription_is_immediately_closed() {
        let mut subscription = BrowserLifecycleEventSubscription::closed();

        assert!(matches!(
            subscription.recv().await,
            Err(BrowserLifecycleEventStreamError::Closed)
        ));
    }

    #[test]
    fn url_policy_matches_allowed_domain_variants_and_wildcards() {
        let policy = UrlAccessPolicy::from_profile(&BrowserProfile {
            allowed_domains: vec![
                "*.google.com".to_owned(),
                "https://wiki.org".to_owned(),
                "https://*.test.com".to_owned(),
                "chrome://version".to_owned(),
                "brave://*".to_owned(),
            ],
            ..BrowserProfile::default()
        });

        assert!(policy.is_allowed("https://google.com"));
        assert!(policy.is_allowed("https://www.google.com"));
        assert!(policy.is_allowed("https://mail.google.com"));
        assert!(!policy.is_allowed("https://evilgoogle.com"));
        assert!(!policy.is_allowed("chrome://abc.google.com"));
        assert!(!policy.is_allowed("http://wiki.org"));
        assert!(policy.is_allowed("https://wiki.org/page"));
        assert!(policy.is_allowed("https://www.test.com"));
        assert!(!policy.is_allowed("https://www.testx.com"));
        assert!(policy.is_allowed("chrome://version"));
        assert!(!policy.is_allowed("chrome://settings"));
        assert!(policy.is_allowed("brave://anything/"));
    }

    #[test]
    fn url_policy_prevents_allowed_domain_auth_bypass() {
        let policy = UrlAccessPolicy::from_profile(&BrowserProfile {
            allowed_domains: vec!["example.com".to_owned(), "*.google.com".to_owned()],
            ..BrowserProfile::default()
        });

        assert!(!policy.is_allowed("https://example.com:password@malicious.com"));
        assert!(!policy.is_allowed("https://example.com@malicious.com"));
        assert!(!policy.is_allowed("https://example.com%20@malicious.com"));
        assert!(!policy.is_allowed("https://sub.google.com@evil.org"));
        assert!(policy.is_allowed("https://user:password@example.com"));
    }

    #[test]
    fn url_policy_root_domain_www_rules_match_upstream() {
        let simple = UrlAccessPolicy::from_profile(&BrowserProfile {
            allowed_domains: vec!["example.com".to_owned(), "test.org".to_owned()],
            ..BrowserProfile::default()
        });
        assert!(simple.is_allowed("https://example.com"));
        assert!(simple.is_allowed("https://www.example.com"));
        assert!(!simple.is_allowed("https://mail.example.com"));
        assert!(!simple.is_allowed("https://notexample.com"));

        let country_tld = UrlAccessPolicy::from_profile(&BrowserProfile {
            allowed_domains: vec!["example.co.uk".to_owned()],
            ..BrowserProfile::default()
        });
        assert!(country_tld.is_allowed("https://example.co.uk"));
        assert!(!country_tld.is_allowed("https://www.example.co.uk"));

        let full_url = UrlAccessPolicy::from_profile(&BrowserProfile {
            allowed_domains: vec!["https://example.com".to_owned()],
            ..BrowserProfile::default()
        });
        assert!(full_url.is_allowed("https://example.com/path"));
        assert!(!full_url.is_allowed("https://www.example.com"));
    }

    #[test]
    fn url_policy_blocks_prohibited_domains_and_preserves_allowlist_precedence() {
        let prohibited_policy = UrlAccessPolicy::from_profile(&BrowserProfile {
            prohibited_domains: vec![
                "example.com".to_owned(),
                "*.ads.example".to_owned(),
                "https://tracker.test".to_owned(),
                "brave://*".to_owned(),
            ],
            ..BrowserProfile::default()
        });

        assert!(!prohibited_policy.is_allowed("https://example.com"));
        assert!(!prohibited_policy.is_allowed("https://www.example.com"));
        assert!(prohibited_policy.is_allowed("https://mail.example.com"));
        assert!(!prohibited_policy.is_allowed("https://cdn.ads.example/pixel"));
        assert!(!prohibited_policy.is_allowed("https://tracker.test/collect?id=1"));
        assert!(prohibited_policy.is_allowed("http://tracker.test/collect?id=1"));
        assert!(!prohibited_policy.is_allowed("brave://anything/"));
        assert!(prohibited_policy.is_allowed("chrome://new-tab-page/"));

        let allowlist_wins = UrlAccessPolicy::from_profile(&BrowserProfile {
            allowed_domains: vec!["*.example.com".to_owned()],
            prohibited_domains: vec!["https://example.com".to_owned()],
            ..BrowserProfile::default()
        });
        assert!(allowlist_wins.is_allowed("https://example.com"));
        assert!(allowlist_wins.is_allowed("https://api.example.com"));
        assert!(!allowlist_wins.is_allowed("https://notexample.com"));
    }

    #[test]
    fn url_policy_blocks_ip_addresses_when_configured() {
        let policy = UrlAccessPolicy::from_profile(&BrowserProfile {
            block_ip_addresses: true,
            ..BrowserProfile::default()
        });

        assert!(!policy.is_allowed("http://127.0.0.1:9222/json"));
        assert!(!policy.is_allowed("http://[::1]/"));
        assert!(policy.is_allowed("https://example.com"));
    }

    #[test]
    fn url_policy_blocks_non_standard_ipv4_forms_when_configured() {
        let policy = UrlAccessPolicy::from_profile(&BrowserProfile {
            block_ip_addresses: true,
            ..BrowserProfile::default()
        });

        for url in [
            "http://2130706433/",
            "http://0x7f000001/",
            "http://0x7F.0x0.0x0.0x1/",
            "http://0177.0.0.1/",
            "http://127.1/",
            "http://127.0.1/",
            "http://10.1/",
        ] {
            assert!(
                !policy.is_allowed(url),
                "non-standard IPv4 should be blocked: {url}"
            );
        }

        assert!(policy.is_allowed("http://127.0.0.1.evil.test/"));
        assert!(policy.is_allowed("http://2130706433.evil.test/"));
        assert!(!is_ip_address("999.999.999.999"));
        assert!(!is_ip_address("1.2.3.4.5"));
    }

    #[test]
    fn ip_classifier_canonicalizes_encoded_and_unicode_hosts() {
        for host in [
            "%30x7f000001",
            "%31%32%37.0.0.1",
            "%32%31%33%30%37%30%36%34%33%33",
            "１２７.０.０.１",
            "０x7f000001",
            "①②⑦.⓪.⓪.①",
            "127。0。0。1",
            "127｡0｡0｡1",
            "127．0．0．1",
            "①②⑦。⓪。⓪。①",
        ] {
            assert!(
                is_ip_address(host),
                "host should classify as an IP address: {host}"
            );
        }

        for host in [
            "%",
            "%zz",
            "%2",
            "café.example",
            "xn--caf-dma.example",
            "日本.example",
            "xn--wgv71a.example",
            "2130706433.evil.test",
        ] {
            assert!(
                !is_ip_address(host),
                "host should remain classified as a domain: {host}"
            );
        }
    }

    #[test]
    fn url_policy_treats_ip_blocking_as_restricted() {
        assert!(UrlAccessPolicy::default().is_unrestricted());
        assert!(
            !UrlAccessPolicy::from_profile(&BrowserProfile {
                block_ip_addresses: true,
                ..BrowserProfile::default()
            })
            .is_unrestricted()
        );
        assert!(
            !UrlAccessPolicy::from_profile(&BrowserProfile {
                prohibited_domains: vec!["blocked.test".to_owned()],
                ..BrowserProfile::default()
            })
            .is_unrestricted()
        );
    }

    #[test]
    fn url_policy_validate_reports_block_reason() {
        let policy = UrlAccessPolicy::from_profile(&BrowserProfile {
            allowed_domains: vec!["example.com".to_owned()],
            ..BrowserProfile::default()
        });

        let error = policy
            .validate("https://blocked.test")
            .expect_err("navigation should be blocked");
        assert_eq!(
            error.to_string(),
            "navigation blocked by browser profile policy: https://blocked.test (not_in_allowed_domains)"
        );

        let ip_policy = UrlAccessPolicy::from_profile(&BrowserProfile {
            block_ip_addresses: true,
            ..BrowserProfile::default()
        });
        let error = ip_policy
            .validate("http://127.0.0.1/")
            .expect_err("ip navigation should be blocked");
        assert_eq!(
            error.to_string(),
            "navigation blocked by browser profile policy: http://127.0.0.1/ (ip_address_blocked)"
        );
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
    fn cloud_browser_response_converts_to_devtools_endpoint() {
        let response = CloudBrowserResponse {
            id: "browser-123".to_owned(),
            status: "running".to_owned(),
            live_url: "https://cloud.browser-use.com/live/browser-123".to_owned(),
            cdp_url: "wss://cdp.browser-use.com/devtools/browser/abc123".to_owned(),
            timeout_at: "2026-05-18T20:00:00Z".to_owned(),
            started_at: "2026-05-18T19:00:00Z".to_owned(),
            finished_at: None,
        };

        let endpoint = response.devtools_endpoint().expect("devtools endpoint");

        assert_eq!(endpoint.http_url, "https://cdp.browser-use.com");
        assert_eq!(
            endpoint.websocket_url,
            "wss://cdp.browser-use.com/devtools/browser/abc123"
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
    fn detects_pagination_buttons_from_dom_state() {
        let dom_state = SerializedDomState::from_elements(vec![
            test_dom_element(1, "button", Some("Next"), &[("id", "next")]),
            test_dom_element(2, "a", Some("2"), &[("href", "/page/2"), ("role", "link")]),
            test_dom_element(3, "button", Some("Export"), &[("id", "export")]),
            test_dom_element(4, "button", Some("Previous"), &[("class", "disabled")]),
        ]);

        let buttons = detect_pagination_buttons(&dom_state);

        assert_eq!(buttons.len(), 3);
        assert_eq!(buttons[0].button_type, PaginationButtonType::Next);
        assert_eq!(buttons[0].selector, "#next");
        assert_eq!(buttons[1].button_type, PaginationButtonType::PageNumber);
        assert_eq!(buttons[2].button_type, PaginationButtonType::Prev);
        assert!(buttons[2].is_disabled);
    }

    fn test_dom_element(
        index: u32,
        tag_name: &str,
        name: Option<&str>,
        attributes: &[(&str, &str)],
    ) -> DomElementRef {
        DomElementRef {
            index,
            target_id: "target".to_owned(),
            backend_node_id: u64::from(index),
            node_id: None,
            tag_name: tag_name.to_owned(),
            role: None,
            name: name.map(str::to_owned),
            text: None,
            attributes: attributes
                .iter()
                .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
                .collect(),
            bounds: None,
            is_visible: true,
            is_interactive: true,
            is_scrollable: false,
        }
    }

    #[test]
    fn finds_previous_navigation_history_entry() {
        let entry_id = previous_navigation_entry_id(&json!({
            "currentIndex": 2,
            "entries": [
                { "id": 10, "url": "https://example.com/one" },
                { "id": 11, "url": "https://example.com/two" },
                { "id": 12, "url": "https://example.com/three" }
            ]
        }))
        .expect("previous entry");

        assert_eq!(entry_id, 11);
    }

    #[test]
    fn reports_missing_previous_navigation_entry() {
        let error = previous_navigation_entry_id(&json!({
            "currentIndex": 0,
            "entries": [
                { "id": 10, "url": "https://example.com/one" }
            ]
        }))
        .expect_err("missing previous entry");

        assert!(matches!(error, BrowserError::ActionFailed(_)));
    }

    #[test]
    fn resolves_full_and_short_page_target_ids() {
        let tabs = vec![
            TabInfo {
                url: "https://example.com/one".to_owned(),
                title: "One".to_owned(),
                tab_id: TabInfo::tab_id_for_target("target-aaa111"),
                target_id: "target-aaa111".to_owned(),
                parent_target_id: None,
            },
            TabInfo {
                url: "https://example.com/two".to_owned(),
                title: "Two".to_owned(),
                tab_id: TabInfo::tab_id_for_target("target-bbb222"),
                target_id: "target-bbb222".to_owned(),
                parent_target_id: None,
            },
        ];

        assert_eq!(
            resolve_page_target_id_from_tabs(&tabs, "target-aaa111").expect("full target id"),
            "target-aaa111"
        );
        assert_eq!(
            resolve_page_target_id_from_tabs(&tabs, "b222").expect("short target id"),
            "target-bbb222"
        );
        assert!(matches!(
            resolve_page_target_id_from_tabs(&tabs, "nope"),
            Err(BrowserError::ActionFailed(_))
        ));
    }

    #[test]
    fn scroll_to_text_script_json_escapes_text() {
        let script = scroll_to_text_js(r#"Needle "quoted""#).expect("scroll script");

        assert!(script.contains(r#"Needle \"quoted\""#));
        assert!(script.contains("scrollIntoView"));
    }

    #[test]
    fn send_keys_normalizes_aliases_and_shortcuts() {
        assert_eq!(normalize_send_keys("ctrl+a"), "Control+a");
        assert_eq!(normalize_send_keys("Command+Shift+P"), "Meta+Shift+P");
        assert_eq!(normalize_send_keys("pagedown"), "PageDown");
        assert_eq!(normalize_send_keys("esc"), "Escape");
        assert_eq!(normalize_send_keys(" keep spaces "), " keep spaces ");
    }

    #[test]
    fn send_keys_key_events_include_codes_and_modifiers() {
        assert_eq!(
            modifier_mask(&["Control".to_owned(), "Shift".to_owned()]),
            10
        );

        assert_eq!(
            key_event_params("keyDown", "a", 2),
            json!({
                "type": "keyDown",
                "key": "a",
                "code": "KeyA",
                "modifiers": 2,
                "windowsVirtualKeyCode": 65,
            })
        );
        assert_eq!(
            key_event_params("keyUp", "PageDown", 0),
            json!({
                "type": "keyUp",
                "key": "PageDown",
                "code": "PageDown",
                "windowsVirtualKeyCode": 34,
            })
        );
    }

    #[test]
    fn dropdown_scripts_support_aria_options() {
        let options_script = dropdown_options_js(2);
        let select_script =
            select_dropdown_option_js(2, r#"Two "quoted""#).expect("select dropdown script");

        assert!(options_script.contains("aria-controls"));
        assert!(options_script.contains(r#"[role="option"]"#));
        assert!(options_script.contains("ARIA listbox"));
        assert!(select_script.contains(r#"const requested = "Two \"quoted\"";"#));
        assert!(select_script.contains("aria-selected"));
        assert!(select_script.contains("MouseEvent('click'"));
    }

    #[test]
    fn click_script_rejects_select_and_file_inputs() {
        let script = click_element_js(1);

        assert!(script.contains("Cannot click on <select> elements."));
        assert!(script.contains("select_dropdown_option"));
        assert!(script.contains("Cannot click on file input elements."));
        assert!(script.contains("Use upload_file instead."));
        assert!(script.contains("dispatchEvent(new MouseEvent('click'"));
    }

    #[test]
    fn cached_click_function_uses_same_guard_body() {
        let function = element_action_function_js(CLICK_ELEMENT_ACTION_JS);

        assert!(function.contains("const el = this;"));
        assert!(function.contains("Cannot click on <select> elements."));
        assert!(function.contains("Cannot click on file input elements."));
        assert!(function.contains("el.click();"));
        assert!(function.contains("dispatchEvent(new MouseEvent('click'"));
    }

    #[test]
    fn interaction_highlight_config_respects_enabled_and_bounds() {
        let config = InteractionHighlightConfig::from_profile(&BrowserProfile {
            interaction_highlight_color: "lime".to_owned(),
            interaction_highlight_duration: 0.25,
            ..BrowserProfile::default()
        });
        let bounds = ElementBounds {
            x: 10,
            y: 20,
            width: 30,
            height: 40,
        };
        let script = config
            .element_script(Some(bounds))
            .expect("highlight script");

        assert!(script.contains("data-browser-use-interaction-highlight"));
        assert!(script.contains("const color = \"lime\";"));
        assert!(script.contains("const duration = 250;"));
        assert!(config.element_script(None).is_none());
        assert!(
            config
                .element_script(Some(ElementBounds { width: 0, ..bounds }))
                .is_none()
        );

        let disabled = InteractionHighlightConfig::from_profile(&BrowserProfile {
            highlight_elements: false,
            ..BrowserProfile::default()
        });
        assert!(disabled.element_script(Some(bounds)).is_none());
        assert!(disabled.coordinate_script(1, 2).is_none());
    }

    #[test]
    fn interaction_highlight_scripts_escape_color_and_mark_coordinates() {
        let script = interaction_element_highlight_script(
            ElementBounds {
                x: 1,
                y: 2,
                width: 30,
                height: 20,
            },
            "rgb(1, 2, 3)\";window.__bad=true;//",
            1.5,
        );
        assert!(script.contains("const rect = {\"height\":20,\"width\":30,\"x\":1,\"y\":2};"));
        assert!(script.contains("const duration = 1500;"));
        assert!(script.contains("rgb(1, 2, 3)\\\";window.__bad=true;//"));
        assert!(script.contains("document.body.appendChild(container);"));

        let coordinate_script = interaction_coordinate_highlight_script(12, 34, "cyan", -1.0);
        assert!(coordinate_script.contains("data-browser-use-coordinate-highlight"));
        assert!(coordinate_script.contains("const x = 12;"));
        assert!(coordinate_script.contains("const y = 34;"));
        assert!(coordinate_script.contains("const duration = 0;"));
    }

    #[test]
    fn dom_highlight_overlay_elements_filter_verbose_labels() {
        let mut selector_map = BTreeMap::new();
        selector_map.insert(
            1,
            test_dom_bound_element(
                1,
                "target",
                "Go",
                Some(ElementBounds {
                    x: 10,
                    y: 20,
                    width: 30,
                    height: 40,
                }),
            ),
        );
        selector_map.insert(
            2,
            test_dom_bound_element(
                2,
                "target",
                "Very long button label",
                Some(ElementBounds {
                    x: 50,
                    y: 60,
                    width: 70,
                    height: 80,
                }),
            ),
        );
        selector_map.insert(3, test_dom_bound_element(3, "target", "Hidden", None));

        let filtered = dom_highlight_overlay_elements(&selector_map, true);
        assert_eq!(filtered.len(), 2);
        assert_eq!(filtered[0].label.as_deref(), Some("1"));
        assert_eq!(filtered[1].label, None);

        let unfiltered = dom_highlight_overlay_elements(&selector_map, false);
        assert_eq!(unfiltered[0].label.as_deref(), Some("1"));
        assert_eq!(unfiltered[1].label.as_deref(), Some("2"));
    }

    #[test]
    fn dom_highlight_config_and_script_match_overlay_contract() {
        let disabled = DomHighlightConfig::from_profile(&BrowserProfile::default());
        assert!(disabled.overlay_script(&BTreeMap::new()).is_none());

        let enabled = DomHighlightConfig::from_profile(&BrowserProfile {
            dom_highlight_elements: true,
            filter_highlight_ids: false,
            ..BrowserProfile::default()
        });
        let mut selector_map = BTreeMap::new();
        selector_map.insert(
            7,
            test_dom_bound_element(
                7,
                "target",
                "Submit",
                Some(ElementBounds {
                    x: 1,
                    y: 2,
                    width: 3,
                    height: 4,
                }),
            ),
        );
        let script = enabled
            .overlay_script(&selector_map)
            .expect("overlay script");

        assert!(script.contains("browser-use-debug-highlights"));
        assert!(script.contains("[data-browser-use-highlight]"));
        assert!(script.contains("\"index\":7"));
        assert!(script.contains("\"label\":\"7\""));
        assert!(script.contains("data-browser-use-index"));
        assert!(script.contains("document.body.appendChild(container);"));
    }

    #[test]
    fn dropdown_scripts_can_run_as_cached_element_functions() {
        let options_function = element_function_js(DROPDOWN_OPTIONS_BODY_JS);
        let select_body =
            select_dropdown_option_body_js("Enterprise").expect("select dropdown body");
        let select_function = element_function_js(&select_body);

        assert!(options_function.contains("const el = this;"));
        assert!(options_function.contains("return JSON.stringify(options);"));
        assert!(select_function.contains("const requested = \"Enterprise\";"));
        assert!(select_function.contains("No dropdown option found"));
    }

    #[test]
    fn interactive_snapshot_uses_image_alt_text_sources() {
        assert!(INTERACTIVE_ELEMENTS_JS.contains("descendantAltText"));
        assert!(INTERACTIVE_ELEMENTS_JS.contains("'img[alt], svg[aria-label]'"));
        assert!(INTERACTIVE_ELEMENTS_JS.contains("'alt'"));
        assert!(INTERACTIVE_ELEMENTS_JS.contains("'aria-describedby'"));
    }

    #[test]
    fn interactive_snapshot_uses_selected_option_text() {
        assert!(INTERACTIVE_ELEMENTS_JS.contains("controlValueText"));
        assert!(INTERACTIVE_ELEMENTS_JS.contains("selectedOptions"));
        assert!(INTERACTIVE_ELEMENTS_JS.contains("controlText || el.innerText"));
    }

    #[test]
    fn interactive_snapshot_summarizes_select_compound_options() {
        assert!(INTERACTIVE_ELEMENTS_JS.contains("selectCompoundComponents"));
        assert!(INTERACTIVE_ELEMENTS_JS.contains("compound_components"));
        assert!(INTERACTIVE_ELEMENTS_JS.contains("Dropdown Toggle"));
        assert!(INTERACTIVE_ELEMENTS_JS.contains("count=${options.length}"));
        assert!(INTERACTIVE_ELEMENTS_JS.contains("format=${formatHint}"));
        assert!(INTERACTIVE_ELEMENTS_JS.contains("... ${options.length - 4} more options..."));
    }

    #[test]
    fn interactive_snapshot_summarizes_compound_controls() {
        assert!(INTERACTIVE_ELEMENTS_JS.contains("inputCompoundComponents"));
        assert!(INTERACTIVE_ELEMENTS_JS.contains("compoundComponentsFor"));
        assert!(INTERACTIVE_ELEMENTS_JS.contains("audio[controls]"));
        assert!(INTERACTIVE_ELEMENTS_JS.contains("video[controls]"));
        assert!(INTERACTIVE_ELEMENTS_JS.contains("Browse Files"));
        assert!(INTERACTIVE_ELEMENTS_JS.contains("Files Selected"));
        assert!(INTERACTIVE_ELEMENTS_JS.contains("Color Picker"));
        assert!(INTERACTIVE_ELEMENTS_JS.contains("Toggle Disclosure"));
        assert!(INTERACTIVE_ELEMENTS_JS.contains("Fullscreen"));

        let action_script = click_element_js(1);
        assert!(action_script.contains("audio[controls]"));
        assert!(action_script.contains("video[controls]"));
    }

    #[test]
    fn interactive_snapshot_preserves_automation_attributes() {
        for attribute in [
            "aria-controls",
            "aria-disabled",
            "aria-haspopup",
            "aria-keyshortcuts",
            "aria-level",
            "aria-live",
            "aria-multiselectable",
            "aria-owns",
            "aria-placeholder",
            "aria-readonly",
            "aria-required",
            "aria-valuemax",
            "aria-valuetext",
            "autocomplete",
            "data-cy",
            "data-datepicker",
            "data-inputmask",
            "data-mask",
            "data-selenium",
            "data-state",
            "data-testid",
            "data-test",
            "data-qa",
            "data-value",
            "for",
            "itemscope",
            "itemprop",
            "lang",
            "inputmode",
            "max",
            "maxlength",
            "min",
            "minlength",
            "pattern",
            "readonly",
            "step",
            "uib-datepicker-popup",
        ] {
            assert!(
                INTERACTIVE_ELEMENTS_JS.contains(attribute),
                "missing attribute {attribute}"
            );
        }
        assert!(INTERACTIVE_ELEMENTS_JS.contains("attrs.value = controlText"));
        assert!(INTERACTIVE_ELEMENTS_JS.contains("attrs.checked = String(el.checked)"));
        assert!(INTERACTIVE_ELEMENTS_JS.contains("attrs.selected = String(el.selected)"));
        assert!(INTERACTIVE_ELEMENTS_JS.contains("booleanAttributeNames"));
        assert!(INTERACTIVE_ELEMENTS_JS.contains("booleanAttributeNames.has(name)"));
        assert!(INTERACTIVE_ELEMENTS_JS.contains("attrs[name] = 'true'"));
    }

    #[test]
    fn interactive_snapshot_keeps_hidden_file_inputs() {
        assert!(INTERACTIVE_ELEMENTS_JS.contains("isFileInput"));
        assert!(INTERACTIVE_ELEMENTS_JS.contains("toLowerCase() === 'file'"));
        assert!(INTERACTIVE_ELEMENTS_JS.contains("if (isFileInput(el)) return true;"));

        let action_script = click_element_js(1);
        assert!(action_script.contains("isFileInput"));
        assert!(action_script.contains("if (isFileInput(el)) return true;"));
    }

    #[test]
    fn interactive_snapshot_skips_decorative_svg_children() {
        assert!(INTERACTIVE_ELEMENTS_JS.contains("isDecorativeSvgChild"));
        assert!(INTERACTIVE_ELEMENTS_JS.contains("'path'"));
        assert!(INTERACTIVE_ELEMENTS_JS.contains("'tspan'"));

        let action_script = click_element_js(1);
        assert!(action_script.contains("isDecorativeSvgChild"));
        assert!(action_script.contains("'circle'"));
    }

    #[test]
    fn interactive_snapshot_marks_elements_for_accessibility_join() {
        assert!(INTERACTIVE_ELEMENTS_JS.contains(AX_REF_ATTRIBUTE));
        assert!(INTERACTIVE_ELEMENTS_JS.contains("ax_ref: axRef"));
        assert!(CLEANUP_AX_REFS_JS.contains(AX_REF_ATTRIBUTE));
    }

    #[test]
    fn dom_snapshot_refs_map_to_backend_node_ids() {
        let snapshot = json!({
            "strings": [
                AX_REF_ATTRIBUTE,
                "browser-use-rs-1",
                "id",
                "native-button"
            ],
            "documents": [{
                "nodes": {
                    "backendNodeId": [41, 42],
                    "attributes": [
                        [],
                        [0, 1, 2, 3]
                    ]
                }
            }]
        });

        let refs = snapshot_backend_ids_by_ax_ref(&snapshot);

        assert_eq!(refs.get("browser-use-rs-1"), Some(&42));
    }

    #[test]
    fn accessibility_tree_nodes_map_by_backend_id() {
        let tree = json!({
            "nodes": [
                {
                    "backendDOMNodeId": 42,
                    "role": { "type": "role", "value": "button" },
                    "name": { "type": "computedString", "value": "Save settings" },
                    "value": { "type": "string", "value": "Ready" },
                    "description": { "type": "computedString", "value": "Primary action" },
                    "properties": [
                        { "name": "expanded", "value": { "type": "boolean", "value": true } },
                        { "name": "valuenow", "value": { "type": "number", "value": 7 } },
                        { "name": "valuetext", "value": { "type": "string", "value": "Seven" } }
                    ]
                },
                {
                    "backendDOMNodeId": 43,
                    "ignored": true,
                    "role": { "type": "role", "value": "button" },
                    "name": { "type": "computedString", "value": "Ignored" }
                }
            ]
        });

        let nodes = accessibility_nodes_by_backend_id(&tree);
        let button = nodes.get(&42).expect("button ax node");

        assert_eq!(button.role.as_deref(), Some("button"));
        assert_eq!(button.name.as_deref(), Some("Save settings"));
        assert_eq!(
            button.properties.get("expanded").map(String::as_str),
            Some("true")
        );
        assert_eq!(
            button.properties.get("valuenow").map(String::as_str),
            Some("7")
        );
        assert_eq!(
            button.properties.get("valuetext").map(String::as_str),
            Some("Seven")
        );
        assert_eq!(
            button.properties.get("value").map(String::as_str),
            Some("Ready")
        );
        assert_eq!(
            button.properties.get("description").map(String::as_str),
            Some("Primary action")
        );
        assert!(!nodes.contains_key(&43));
    }

    #[test]
    fn dom_element_uses_accessibility_enrichment() {
        let accessibility = BTreeMap::from([(
            "browser-use-rs-1".to_owned(),
            AccessibilityNodeInfo {
                backend_node_id: 42,
                node_id: Some(84),
                role: Some("button".to_owned()),
                name: Some("Save settings".to_owned()),
                properties: BTreeMap::from([
                    ("description".to_owned(), "Primary action".to_owned()),
                    ("expanded".to_owned(), "true".to_owned()),
                ]),
            },
        )]);
        let element = dom_element_from_value(
            "target-1",
            &json!({
                "index": 1,
                "tag_name": "button",
                "name": "DOM fallback",
                "text": "DOM fallback",
                "attributes": { "id": "native-button" },
                "ax_ref": "browser-use-rs-1",
                "is_visible": true,
                "is_interactive": true
            }),
            &accessibility,
        )
        .expect("dom element");

        assert_eq!(element.backend_node_id, 42);
        assert_eq!(element.node_id, Some(84));
        assert_eq!(element.role.as_deref(), Some("button"));
        assert_eq!(element.name.as_deref(), Some("Save settings"));
        assert_eq!(
            element.attributes.get("expanded").map(String::as_str),
            Some("true")
        );
        assert_eq!(
            element.attributes.get("ax_name").map(String::as_str),
            Some("Save settings")
        );
        assert_eq!(
            element.attributes.get("ax_description").map(String::as_str),
            Some("Primary action")
        );
    }

    #[test]
    fn dom_state_parser_applies_ax_hidden_disabled_veto_and_preserves_metadata() {
        let accessibility = BTreeMap::from([
            (
                "browser-use-rs-hidden".to_owned(),
                AccessibilityNodeInfo {
                    backend_node_id: 41,
                    node_id: Some(81),
                    role: Some("button".to_owned()),
                    name: Some("Hidden action".to_owned()),
                    properties: BTreeMap::from([
                        ("focusable".to_owned(), "true".to_owned()),
                        ("hidden".to_owned(), "true".to_owned()),
                    ]),
                },
            ),
            (
                "browser-use-rs-disabled".to_owned(),
                AccessibilityNodeInfo {
                    backend_node_id: 42,
                    node_id: Some(82),
                    role: Some("button".to_owned()),
                    name: Some("Disabled action".to_owned()),
                    properties: BTreeMap::from([
                        ("disabled".to_owned(), "true".to_owned()),
                        ("focusable".to_owned(), "true".to_owned()),
                    ]),
                },
            ),
            (
                "browser-use-rs-editable".to_owned(),
                AccessibilityNodeInfo {
                    backend_node_id: 43,
                    node_id: Some(83),
                    role: Some("textbox".to_owned()),
                    name: Some("Search".to_owned()),
                    properties: BTreeMap::from([
                        ("autocomplete".to_owned(), "list".to_owned()),
                        ("editable".to_owned(), "true".to_owned()),
                        ("focusable".to_owned(), "true".to_owned()),
                        ("settable".to_owned(), "true".to_owned()),
                    ]),
                },
            ),
        ]);
        let state = dom_state_from_interactive_value(
            "target-1",
            &json!({
                "elements": [
                    {
                        "index": 1,
                        "tag_name": "button",
                        "attributes": { "id": "hidden-action" },
                        "ax_ref": "browser-use-rs-hidden",
                        "is_visible": true,
                        "is_interactive": true
                    },
                    {
                        "index": 2,
                        "tag_name": "button",
                        "attributes": { "id": "disabled-action" },
                        "ax_ref": "browser-use-rs-disabled",
                        "is_visible": true,
                        "is_interactive": true
                    },
                    {
                        "index": 3,
                        "tag_name": "input",
                        "attributes": { "id": "search", "type": "text" },
                        "ax_ref": "browser-use-rs-editable",
                        "is_visible": true,
                        "is_interactive": true
                    }
                ]
            }),
            &accessibility,
        )
        .expect("dom state");

        assert_eq!(state.selector_map.len(), 1);
        assert!(!state.selector_map.contains_key(&1));
        assert!(!state.selector_map.contains_key(&2));

        let editable = state.selector_map.get(&3).expect("editable element");
        assert_eq!(editable.backend_node_id, 43);
        assert_eq!(editable.role.as_deref(), Some("textbox"));
        assert_eq!(editable.name.as_deref(), Some("Search"));
        assert_eq!(
            editable.attributes.get("focusable").map(String::as_str),
            Some("true")
        );
        assert_eq!(
            editable.attributes.get("editable").map(String::as_str),
            Some("true")
        );
        assert_eq!(
            editable.attributes.get("settable").map(String::as_str),
            Some("true")
        );
        assert_eq!(
            editable.attributes.get("autocomplete").map(String::as_str),
            Some("list")
        );
        assert_eq!(
            editable.attributes.get("ax_name").map(String::as_str),
            Some("Search")
        );
        assert_eq!(
            state.llm_representation(),
            "[3] <input type=text id=search autocomplete=list> Search"
        );
        assert_eq!(
            state.llm_representation_with_attributes(&[
                "focusable".to_owned(),
                "editable".to_owned(),
                "settable".to_owned()
            ]),
            "[3] <input focusable=true editable=true settable=true> Search"
        );
    }

    #[test]
    fn dom_state_parser_preserves_native_boolean_attributes() {
        let state = dom_state_from_interactive_value(
            "target-1",
            &json!({
                "elements": [{
                    "index": 1,
                    "tag_name": "input",
                    "name": "Invoice id",
                    "text": "INV-123",
                    "attributes": {
                        "id": "invoice",
                        "readonly": "true",
                        "required": "true",
                        "multiple": "true"
                    },
                    "is_visible": true,
                    "is_interactive": true
                }]
            }),
            &BTreeMap::new(),
        )
        .expect("dom state");

        let llm = state.llm_representation();
        assert!(
            llm.contains("[1] <input id=invoice multiple=true required=true> Invoice id INV-123"),
            "DOM state did not render default native boolean attributes: {llm}"
        );
        assert_eq!(
            state.selector_map[&1]
                .attributes
                .get("readonly")
                .map(String::as_str),
            Some("true")
        );
    }

    #[test]
    fn dom_state_parser_carries_page_stats() {
        let state = dom_state_from_interactive_value(
            "target-1",
            &json!({
                "stats": {
                    "links": 1,
                    "iframes": 2,
                    "shadow_open": 1,
                    "shadow_closed": 0,
                    "scroll_containers": 3,
                    "images": 4,
                    "interactive_elements": 5,
                    "total_elements": 30,
                    "text_chars": 40
                },
                "elements": [{
                    "index": 1,
                    "tag_name": "a",
                    "name": "Docs",
                    "text": "Docs",
                    "attributes": { "href": "/docs" },
                    "is_visible": true,
                    "is_interactive": true
                }]
            }),
            &BTreeMap::new(),
        )
        .expect("dom state");

        assert_eq!(state.selector_map.len(), 1);
        assert_eq!(state.page_stats.links, 1);
        assert_eq!(state.page_stats.iframes, 2);
        assert_eq!(state.page_stats.shadow_open, 1);
        assert_eq!(state.page_stats.scroll_containers, 3);
        assert_eq!(state.page_stats.images, 4);
        assert_eq!(state.page_stats.interactive_elements, 5);
        assert_eq!(state.page_stats.total_elements, 30);
        assert_eq!(state.page_stats.text_chars, 40);
    }

    #[test]
    fn dom_state_parser_carries_eval_tree() {
        let accessibility = BTreeMap::from([(
            "browser-use-rs-1".to_owned(),
            AccessibilityNodeInfo {
                backend_node_id: 55,
                node_id: Some(77),
                role: Some("button".to_owned()),
                name: Some("Save settings".to_owned()),
                properties: BTreeMap::from([(
                    "description".to_owned(),
                    "Persists account settings".to_owned(),
                )]),
            },
        )]);
        let state = dom_state_from_interactive_value(
            "target-1",
            &json!({
                "elements": [{
                    "index": 1,
                    "tag_name": "button",
                    "name": "Save",
                    "text": "Save",
                    "attributes": { "data-testid": "save-settings" },
                    "is_visible": true,
                    "is_interactive": true,
                    "ax_ref": "browser-use-rs-1"
                }],
                "eval_tree": {
                    "node_type": "element",
                    "tag_name": "body",
                    "is_visible": true,
                    "children": [{
                        "node_type": "element",
                        "tag_name": "button",
                        "attributes": { "data-testid": "save-settings" },
                        "is_visible": true,
                        "is_interactive": true,
                        "ax_ref": "browser-use-rs-1",
                        "children": [{
                            "node_type": "text",
                            "node_value": "Save"
                        }]
                    }]
                }
            }),
            &accessibility,
        )
        .expect("dom state");

        assert_eq!(
            state.eval_representation(),
            "<body />\n\t[i_55] <button data-testid=\"save-settings\" ax_name=\"Save settings\" ax_description=\"Persists account settings\">Save"
        );
    }

    #[test]
    fn interactive_snapshot_detects_search_affordances() {
        assert!(INTERACTIVE_ELEMENTS_JS.contains("hasSearchIndicator"));
        assert!(INTERACTIVE_ELEMENTS_JS.contains("search-icon"));
        assert!(INTERACTIVE_ELEMENTS_JS.contains("attr.name.startsWith('data-')"));

        let action_script = click_element_js(1);
        assert!(action_script.contains("hasSearchIndicator"));
        assert!(action_script.contains("search-button"));
    }

    #[test]
    fn interactive_snapshot_detects_small_icon_controls() {
        assert!(INTERACTIVE_ELEMENTS_JS.contains("hasIconSignal"));
        assert!(INTERACTIVE_ELEMENTS_JS.contains("rect.width < 10"));
        assert!(INTERACTIVE_ELEMENTS_JS.contains("'data-action'"));
        assert!(INTERACTIVE_ELEMENTS_JS.contains("'aria-label'"));

        let action_script = click_element_js(1);
        assert!(action_script.contains("hasIconSignal"));
        assert!(action_script.contains("rect.height > 50"));
    }

    #[test]
    fn interactive_snapshot_detects_pointer_cursor_controls() {
        assert!(INTERACTIVE_ELEMENTS_JS.contains("hasPointerCursor"));
        assert!(INTERACTIVE_ELEMENTS_JS.contains("cursor === 'pointer'"));

        let action_script = click_element_js(1);
        assert!(action_script.contains("hasPointerCursor"));
        assert!(action_script.contains("cursor === 'pointer'"));
    }

    #[test]
    fn interactive_snapshot_detects_static_handlers_and_listboxes() {
        assert!(INTERACTIVE_ELEMENTS_JS.contains("[role=\"listbox\"]"));
        assert!(INTERACTIVE_ELEMENTS_JS.contains("[onmousedown]"));
        assert!(INTERACTIVE_ELEMENTS_JS.contains("[onkeydown]"));

        let action_script = click_element_js(1);
        assert!(action_script.contains("[role=\"listbox\"]"));
        assert!(action_script.contains("[onmouseup]"));
        assert!(action_script.contains("[onkeyup]"));
    }

    #[test]
    fn interactive_snapshot_indexes_all_tabindex_values_like_upstream() {
        assert!(INTERACTIVE_ELEMENTS_JS.contains("[tabindex]"));
        assert!(!INTERACTIVE_ELEMENTS_JS.contains("[tabindex]:not"));

        let action_script = click_element_js(1);
        assert!(action_script.contains("[tabindex]"));
        assert!(!action_script.contains("[tabindex]:not"));
    }

    #[test]
    fn interactive_snapshot_detects_aria_interactivity_properties() {
        assert!(INTERACTIVE_ELEMENTS_JS.contains("hasAriaInteractivityProperty"));
        assert!(INTERACTIVE_ELEMENTS_JS.contains("aria-required"));
        assert!(INTERACTIVE_ELEMENTS_JS.contains("aria-autocomplete"));
        assert!(INTERACTIVE_ELEMENTS_JS.contains("aria-keyshortcuts"));

        let action_script = click_element_js(1);
        assert!(action_script.contains("hasAriaInteractivityProperty"));
        assert!(action_script.contains("autocomplete !== 'none'"));
        assert!(action_script.contains("aria-keyshortcuts"));
    }

    #[test]
    fn interactive_snapshot_detects_contenteditable_variants() {
        let contenteditable_selector = r#"[contenteditable]:not([contenteditable="false"])"#;
        assert!(INTERACTIVE_ELEMENTS_JS.contains(contenteditable_selector));

        let action_script = click_element_js(1);
        assert!(action_script.contains(contenteditable_selector));
    }

    #[test]
    fn interactive_snapshot_indexes_anchor_tags_without_href() {
        assert!(INTERACTIVE_ELEMENTS_JS.contains("'a',"));
        assert!(!INTERACTIVE_ELEMENTS_JS.contains("'a[href]'"));

        let action_script = click_element_js(1);
        assert!(action_script.contains("'a',"));
        assert!(!action_script.contains("'a[href]'"));
    }

    #[test]
    fn interactive_snapshot_filters_occluded_elements() {
        assert!(INTERACTIVE_ELEMENTS_JS.contains("isTopmostAtCenter"));
        assert!(INTERACTIVE_ELEMENTS_JS.contains("elementFromPoint"));
        assert!(INTERACTIVE_ELEMENTS_JS.contains("root.host"));
        assert!(INTERACTIVE_ELEMENTS_JS.contains("const paintOrderFiltering = true;"));
        assert!(
            INTERACTIVE_ELEMENTS_JS.contains(
                "(!paintOrderFiltering || isNativeMediaControl || isTopmostAtCenter(el))"
            )
        );

        let action_script = click_element_js(1);
        assert!(action_script.contains("isTopmostAtCenter"));
        assert!(action_script.contains("elementFromPoint"));
    }

    #[test]
    fn interactive_snapshot_script_carries_paint_order_filtering_control() {
        let config = IframeTraversalConfig::from_profile(&BrowserProfile::default());
        let enabled = interactive_elements_js(config, true);
        assert!(enabled.contains("const paintOrderFiltering = true;"));
        assert!(
            enabled.contains(
                "(!paintOrderFiltering || isNativeMediaControl || isTopmostAtCenter(el))"
            )
        );

        let disabled = interactive_elements_js(config, false);
        assert!(disabled.contains("const paintOrderFiltering = false;"));
        assert!(
            disabled.contains(
                "(!paintOrderFiltering || isNativeMediaControl || isTopmostAtCenter(el))"
            )
        );
    }

    #[test]
    fn interactive_snapshot_skips_browser_use_excluded_subtrees() {
        assert!(INTERACTIVE_ELEMENTS_JS.contains("isBrowserUseExcluded"));
        assert!(INTERACTIVE_ELEMENTS_JS.contains("data-browser-use-exclude"));
        assert!(INTERACTIVE_ELEMENTS_JS.contains("data-browser-use-exclude-"));

        let action_script = click_element_js(1);
        assert!(action_script.contains("isBrowserUseExcluded"));
        assert!(action_script.contains("data-browser-use-exclude"));
        assert!(action_script.contains("data-browser-use-exclude-"));
    }

    #[test]
    fn interactive_snapshot_skips_non_content_dom_tags() {
        assert!(INTERACTIVE_ELEMENTS_JS.contains("isNonContentTag"));
        for tag in ["style", "script", "head", "meta", "link", "title"] {
            assert!(
                INTERACTIVE_ELEMENTS_JS.contains(tag),
                "state walker missing {tag}"
            );
        }

        let action_script = click_element_js(1);
        assert!(action_script.contains("isNonContentTag"));
        for tag in ["style", "script", "head", "meta", "link", "title"] {
            assert!(
                action_script.contains(tag),
                "action fallback walker missing {tag}"
            );
        }
    }

    #[test]
    fn interactive_snapshot_prunes_contained_action_descendants() {
        assert!(INTERACTIVE_ELEMENTS_JS.contains("isPropagatingActionContainer"));
        assert!(INTERACTIVE_ELEMENTS_JS.contains("isContainedByPropagatingActionContainer"));
        assert!(INTERACTIVE_ELEMENTS_JS.contains("shouldKeepContainedDescendant"));
        assert!(INTERACTIVE_ELEMENTS_JS.contains("containedByRect"));
        assert!(INTERACTIVE_ELEMENTS_JS.contains(">= 0.99"));

        let action_script = click_element_js(1);
        assert!(action_script.contains("isPropagatingActionContainer"));
        assert!(action_script.contains("isContainedByPropagatingActionContainer"));
        assert!(action_script.contains("shouldKeepContainedDescendant"));
        assert!(action_script.contains("containedByRect"));
        assert!(action_script.contains(">= 0.99"));
    }

    #[test]
    fn interactive_snapshot_detects_javascript_click_listeners() {
        assert!(INTERACTIVE_ELEMENTS_JS.contains("getEventListeners"));
        assert!(INTERACTIVE_ELEMENTS_JS.contains("hasJsClickListener"));
        assert!(INTERACTIVE_ELEMENTS_JS.contains("'pointerdown'"));
        assert!(INTERACTIVE_ELEMENTS_JS.contains("document.querySelectorAll('*').length <= 10000"));

        let params = runtime_evaluate_params(INTERACTIVE_ELEMENTS_JS, true);
        assert_eq!(params["includeCommandLineAPI"], true);

        let params = runtime_evaluate_params("document.title", false);
        assert!(params.get("includeCommandLineAPI").is_none());
    }

    #[test]
    fn interactive_snapshot_collects_page_statistics() {
        assert!(INTERACTIVE_ELEMENTS_JS.contains("const stats = {"));
        assert!(INTERACTIVE_ELEMENTS_JS.contains("shadow_open"));
        assert!(INTERACTIVE_ELEMENTS_JS.contains("interactive_elements"));
        assert!(INTERACTIVE_ELEMENTS_JS.contains("total_elements"));
        assert!(INTERACTIVE_ELEMENTS_JS.contains("text_chars"));
        assert!(INTERACTIVE_ELEMENTS_JS.contains("return {"));
        assert!(INTERACTIVE_ELEMENTS_JS.contains("elements: indexedElements"));
        assert!(INTERACTIVE_ELEMENTS_JS.contains("eval_tree: evalTreeForElement"));
        assert!(INTERACTIVE_ELEMENTS_JS.contains("node_type: 'document_fragment'"));
    }

    #[test]
    fn interactive_snapshot_indexes_scrollable_containers_without_descendant_controls() {
        assert!(INTERACTIVE_ELEMENTS_JS.contains("shouldIndexScrollable"));
        assert!(INTERACTIVE_ELEMENTS_JS.contains("hasInteractiveDescendant"));
        assert!(INTERACTIVE_ELEMENTS_JS.contains("isDropdownContainer"));
        assert!(INTERACTIVE_ELEMENTS_JS.contains("scrollInfoText"));
        assert!(INTERACTIVE_ELEMENTS_JS.contains("attrs.scroll = scroll"));

        let action_script = element_action_js(1, "el.scrollBy(0, el.clientHeight);");
        assert!(action_script.contains("shouldIndexScrollable"));
        assert!(action_script.contains("hasInteractiveDescendant"));
        assert!(action_script.contains("isDropdownContainer"));
    }

    #[test]
    fn renders_runtime_evaluate_values() {
        assert_eq!(
            render_runtime_evaluate_result(&json!({
                "result": { "type": "string", "value": "EvalOps" }
            }))
            .expect("string result"),
            "EvalOps"
        );
        assert_eq!(
            render_runtime_evaluate_result(&json!({
                "result": { "type": "number", "value": 42 }
            }))
            .expect("number result"),
            "42"
        );
        assert_eq!(
            render_runtime_evaluate_result(&json!({
                "result": { "type": "undefined" }
            }))
            .expect("undefined result"),
            "undefined"
        );
    }

    #[test]
    fn renders_runtime_evaluate_exception_as_error() {
        let error = render_runtime_evaluate_result(&json!({
            "exceptionDetails": { "text": "Uncaught Error: boom" }
        }))
        .expect_err("exception");

        assert!(matches!(error, BrowserError::CommandFailed { .. }));
    }

    #[test]
    fn executable_resolution_prefers_explicit_path() {
        let current_exe = std::env::current_exe().expect("current exe");
        let resolved = resolve_chrome_executable(Some(&current_exe), None, Vec::<PathBuf>::new())
            .expect("resolve executable");

        assert_eq!(resolved, current_exe);
    }

    #[test]
    fn executable_path_alias_resolves_before_env_and_candidates() {
        let alias_exe = std::env::current_exe().expect("current exe");
        let env_exe = PathBuf::from("/definitely/not/env-chrome");
        let candidate = PathBuf::from("/definitely/not/channel-browser");
        let profile: BrowserProfile = serde_json::from_value(json!({
            "browser_binary_path": alias_exe.display().to_string()
        }))
        .expect("browser binary alias profile");

        let resolved = resolve_chrome_executable(
            profile.executable_path.as_deref(),
            Some(env_exe),
            vec![candidate],
        )
        .expect("resolve alias executable");

        assert_eq!(resolved, alias_exe);
    }

    #[test]
    fn executable_resolution_prefers_env_before_candidates() {
        let env_exe = std::env::current_exe().expect("current exe");
        let candidate = PathBuf::from("/definitely/not/a/channel-browser");
        let resolved = resolve_chrome_executable(None, Some(env_exe.clone()), vec![candidate])
            .expect("resolve executable from env");

        assert_eq!(resolved, env_exe);
    }

    #[test]
    fn browser_channel_candidates_are_channel_specific() {
        let beta_candidates = browser_channel_candidates(BrowserChannel::ChromeBeta);
        assert!(!beta_candidates.is_empty());
        assert_eq!(
            browser_executable_candidates(Some(BrowserChannel::ChromeBeta)),
            beta_candidates
        );
        assert_eq!(
            browser_executable_candidates(None),
            default_chrome_candidates()
        );

        let beta_candidate_text = beta_candidates
            .iter()
            .map(|path| path.display().to_string().to_ascii_lowercase())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            beta_candidate_text.contains("beta"),
            "chrome-beta candidates should be beta-specific: {beta_candidates:?}"
        );
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
    async fn cdp_session_can_index_open_shadow_dom_elements() {
        let profile = BrowserProfile::default();
        let session = CdpBrowserSession::launch(&profile)
            .await
            .expect("launch CDP session");

        session
            .navigate(
                "data:text/html,<html><head><title>shadow smoke</title></head><body><div id='host'></div><script>const root=document.getElementById('host').attachShadow({mode:'open'});const button=document.createElement('button');button.textContent='Shadow click';button.onclick=()=>{document.title='shadow clicked'};const input=document.createElement('input');input.placeholder='Shadow name';root.append(button,input);</script></body></html>",
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
                .contains("Shadow click")
        );
        let eval = initial_state.dom_state.eval_representation();
        assert!(
            eval.contains("#shadow"),
            "eval tree missed shadow root: {eval}"
        );
        assert!(
            eval.contains("[i_") && eval.contains("Shadow click"),
            "eval tree missed backend-indexed shadow control: {eval}"
        );

        session.click(1).await.expect("shadow click");
        session
            .input_text(2, "EvalOps", true)
            .await
            .expect("shadow input");
        sleep(Duration::from_millis(100)).await;

        let state = session.state(false).await.expect("shadow state");
        assert_eq!(state.title, "shadow clicked");
        assert!(
            state.dom_state.llm_representation().contains("EvalOps"),
            "DOM state did not include shadow input value: {}",
            state.dom_state.llm_representation()
        );
    }

    #[tokio::test]
    #[ignore = "requires Chrome/Chromium installed on the local machine"]
    async fn cdp_session_indexes_javascript_listener_elements() {
        let profile = BrowserProfile::default();
        let session = CdpBrowserSession::launch(&profile)
            .await
            .expect("launch CDP session");

        session
            .navigate(
                "data:text/html,<html><head><title>listener smoke</title></head><body><div id='plain-listener' style='display:inline-block;width:80px;height:30px'>Plain listener</div><script>document.getElementById('plain-listener').addEventListener('click',()=>{document.title='listener clicked'});</script></body></html>",
                false,
            )
            .await
            .expect("navigate");
        sleep(Duration::from_millis(100)).await;

        let initial_state = session.state(false).await.expect("initial state");
        let listener = initial_state
            .dom_state
            .selector_map
            .values()
            .find(|element| {
                element.attributes.get("id").map(String::as_str) == Some("plain-listener")
            })
            .unwrap_or_else(|| {
                panic!(
                    "missing JS listener element: {}",
                    initial_state.dom_state.llm_representation()
                )
            });

        session
            .click(listener.index)
            .await
            .expect("listener-backed click");
        sleep(Duration::from_millis(100)).await;
        let state = session.state(false).await.expect("post-click state");

        assert_eq!(state.title, "listener clicked");
    }

    #[tokio::test]
    #[ignore = "requires Chrome/Chromium installed on the local machine"]
    async fn cdp_session_can_index_same_origin_iframe_elements() {
        let profile = BrowserProfile::default();
        let session = CdpBrowserSession::launch(&profile)
            .await
            .expect("launch CDP session");

        session
            .navigate(
                "data:text/html,<html><head><title>iframe smoke</title></head><body><script>const iframe=document.createElement('iframe');iframe.srcdoc='<button onclick=\"parent.document.title=&quot;iframe clicked&quot;\">Frame click</button><input placeholder=\"Frame name\">';document.body.appendChild(iframe);</script></body></html>",
                false,
            )
            .await
            .expect("navigate");
        sleep(Duration::from_millis(200)).await;

        let initial_state = session.state(false).await.expect("initial state");
        assert_eq!(initial_state.dom_state.element_count(), 3);
        assert!(
            initial_state
                .dom_state
                .llm_representation()
                .contains("Frame click")
        );
        let iframe = initial_state
            .dom_state
            .selector_map
            .values()
            .find(|element| element.tag_name == "iframe")
            .expect("iframe element");
        assert_eq!(iframe.index, 1);
        let frame_button_index = initial_state
            .dom_state
            .selector_map
            .values()
            .find(|element| element.name.as_deref() == Some("Frame click"))
            .expect("iframe button")
            .index;
        let frame_input_index = initial_state
            .dom_state
            .selector_map
            .values()
            .find(|element| element.name.as_deref() == Some("Frame name"))
            .expect("iframe input")
            .index;

        session
            .click(frame_button_index)
            .await
            .expect("iframe click");
        session
            .input_text(frame_input_index, "EvalOps", true)
            .await
            .expect("iframe input");
        sleep(Duration::from_millis(100)).await;

        let state = session.state(false).await.expect("iframe state");
        assert_eq!(state.title, "iframe clicked");
        let iframe_input_value = session
            .evaluate_json(
                "document.querySelector('iframe').contentDocument.querySelector('input').value",
            )
            .await
            .expect("iframe input value");
        assert_eq!(iframe_input_value.as_str(), Some("EvalOps"));
    }

    #[tokio::test]
    #[ignore = "requires Chrome/Chromium installed on the local machine"]
    async fn cdp_session_can_index_and_act_in_cross_origin_iframe_targets() {
        let child_html = "<html><body><button id='child-button' onclick=\"document.body.dataset.clicked='yes'\">Cross child</button><input id='child-input' placeholder='Cross input'></body></html>";
        let (child_addr, child_server) = spawn_static_html_server(child_html.to_owned()).await;
        let child_url = format!("http://127.0.0.1:{}/child", child_addr.port());
        let parent_html = format!(
            "<html><head><title>cross iframe smoke</title></head><body><iframe src='{child_url}' style='width:420px;height:180px;border:0'></iframe></body></html>"
        );
        let (parent_addr, parent_server) = spawn_static_html_server(parent_html).await;
        let parent_url = format!("http://localhost:{}/parent", parent_addr.port());
        let profile = BrowserProfile {
            args: vec!["--site-per-process".to_owned()],
            browser_start_timeout_ms: 30_000,
            ..BrowserProfile::default()
        };
        let session = CdpBrowserSession::launch(&profile)
            .await
            .expect("launch CDP session");

        session
            .navigate(&parent_url, false)
            .await
            .expect("navigate cross-origin parent");
        let mut initial_state = None;
        for _ in 0..20 {
            let state = session.state(false).await.expect("cross-origin state");
            if state.dom_state.llm_representation().contains("Cross child") {
                initial_state = Some(state);
                break;
            }
            sleep(Duration::from_millis(100)).await;
        }
        let initial_state = initial_state.expect("cross-origin iframe element state");
        let iframe = initial_state
            .dom_state
            .selector_map
            .values()
            .find(|element| element.tag_name == "iframe")
            .expect("iframe element");
        let child_button = initial_state
            .dom_state
            .selector_map
            .values()
            .find(|element| element.name.as_deref() == Some("Cross child"))
            .expect("child button");
        let child_input = initial_state
            .dom_state
            .selector_map
            .values()
            .find(|element| element.name.as_deref() == Some("Cross input"))
            .expect("child input");
        assert_ne!(child_button.target_id, iframe.target_id);
        assert_ne!(child_input.target_id, iframe.target_id);
        let eval = initial_state.dom_state.eval_representation();
        assert!(
            eval.contains("#iframe-content"),
            "cross-origin eval tree missed iframe content marker: {eval}"
        );
        assert!(
            eval.contains("Cross child"),
            "cross-origin eval tree missed child target content: {eval}"
        );

        session
            .click(child_button.index)
            .await
            .expect("cross-origin child click");
        session
            .input_text(child_input.index, "EvalOps", true)
            .await
            .expect("cross-origin child input");

        let page = session.current_page().await;
        let frame_infos = session
            .frame_element_infos(&page)
            .await
            .expect("frame element infos");
        let child_page = session
            .iframe_target_pages(&page, &frame_infos)
            .await
            .expect("iframe target pages")
            .into_iter()
            .find(|frame| frame.page.target_id == child_button.target_id)
            .expect("child target page");
        let clicked = session
            .evaluate_json_for_page(&child_page.page, "document.body.dataset.clicked", false)
            .await
            .expect("child clicked flag");
        let input_value = session
            .evaluate_json_for_page(
                &child_page.page,
                "document.getElementById('child-input').value",
                false,
            )
            .await
            .expect("child input value");

        assert_eq!(clicked.as_str(), Some("yes"));
        assert_eq!(input_value.as_str(), Some("EvalOps"));
        child_server.abort();
        parent_server.abort();
    }

    #[tokio::test]
    #[ignore = "requires Chrome/Chromium installed on the local machine"]
    async fn cdp_session_uses_cross_origin_iframe_target_for_stale_node_fallback() {
        let child_html = "<html><body><button id='child-button' onclick=\"document.body.dataset.clicked='initial'\">Cross child</button><input id='child-input' placeholder='Cross input'></body></html>";
        let (child_addr, child_server) = spawn_static_html_server(child_html.to_owned()).await;
        let child_url = format!("http://127.0.0.1:{}/child", child_addr.port());
        let parent_html = format!(
            "<html><head><title>cross stale fallback</title></head><body><iframe src='{child_url}' style='width:420px;height:180px;border:0'></iframe></body></html>"
        );
        let (parent_addr, parent_server) = spawn_static_html_server(parent_html).await;
        let parent_url = format!("http://localhost:{}/parent", parent_addr.port());
        let profile = BrowserProfile {
            args: vec!["--site-per-process".to_owned()],
            browser_start_timeout_ms: 30_000,
            ..BrowserProfile::default()
        };
        let session = CdpBrowserSession::launch(&profile)
            .await
            .expect("launch CDP session");

        session
            .navigate(&parent_url, false)
            .await
            .expect("navigate cross-origin parent");
        let mut initial_state = None;
        for _ in 0..20 {
            let state = session.state(false).await.expect("cross-origin state");
            if state.dom_state.llm_representation().contains("Cross child") {
                initial_state = Some(state);
                break;
            }
            sleep(Duration::from_millis(100)).await;
        }
        let initial_state = initial_state.expect("cross-origin iframe element state");
        let iframe = initial_state
            .dom_state
            .selector_map
            .values()
            .find(|element| element.tag_name == "iframe")
            .expect("iframe element");
        let child_button = initial_state
            .dom_state
            .selector_map
            .values()
            .find(|element| {
                element
                    .attributes
                    .get("id")
                    .is_some_and(|value| value == "child-button")
            })
            .expect("child button")
            .clone();
        let child_input = initial_state
            .dom_state
            .selector_map
            .values()
            .find(|element| {
                element
                    .attributes
                    .get("id")
                    .is_some_and(|value| value == "child-input")
            })
            .expect("child input")
            .clone();
        assert_ne!(child_button.target_id, iframe.target_id);
        assert_eq!(child_button.target_id, child_input.target_id);
        assert!(child_button.index > 1);
        assert!(child_input.index > 1);

        let page = session.current_page().await;
        let frame_infos = session
            .frame_element_infos(&page)
            .await
            .expect("frame element infos");
        let child_page = session
            .iframe_target_pages(&page, &frame_infos)
            .await
            .expect("iframe target pages")
            .into_iter()
            .find(|frame| frame.page.target_id == child_button.target_id)
            .expect("child target page");
        session
            .evaluate_json_for_page(
                &child_page.page,
                r#"
(() => {
  document.open();
  document.write(`<html><body><button id="child-button" onclick="document.body.dataset.clicked='replacement'">Replacement child</button><input id="child-input" placeholder="Replacement input"></body></html>`);
  document.close();
  return true;
})()
"#,
                false,
            )
            .await
            .expect("replace child document");
        sleep(Duration::from_millis(100)).await;

        session
            .click(child_button.index)
            .await
            .expect("click replacement child through fallback");
        session
            .input_text(child_input.index, "EvalOps", true)
            .await
            .expect("input replacement child through fallback");

        let values = session
            .evaluate_json_for_page(
                &child_page.page,
                "JSON.stringify({ clicked: document.body.dataset.clicked || '', input: document.getElementById('child-input').value || '' })",
                false,
            )
            .await
            .expect("child values");
        let values: Value = serde_json::from_str(values.as_str().expect("encoded child values"))
            .expect("child values json");
        assert_eq!(values["clicked"].as_str(), Some("replacement"));
        assert_eq!(values["input"].as_str(), Some("EvalOps"));

        child_server.abort();
        parent_server.abort();
    }

    #[tokio::test]
    #[ignore = "requires Chrome/Chromium installed on the local machine"]
    async fn cdp_session_detached_cached_node_falls_back_inside_cross_origin_iframe_target() {
        let child_html = r#"<html><body><button id='child-button' onclick="document.body.dataset.clicked='old'">Cross stale</button><script>
function replaceChildButton() {
  const next = document.createElement('button');
  next.id = 'child-button';
  next.textContent = 'Cross stale';
  next.onclick = () => { document.body.dataset.clicked = 'replacement'; };
  document.getElementById('child-button').replaceWith(next);
}
</script></body></html>"#;
        let (child_addr, child_server) = spawn_static_html_server(child_html.to_owned()).await;
        let child_url = format!("http://127.0.0.1:{}/child", child_addr.port());
        let parent_html = format!(
            "<html><head><title>cross iframe detached fallback</title></head><body><iframe src='{child_url}' style='width:420px;height:180px;border:0'></iframe></body></html>"
        );
        let (parent_addr, parent_server) = spawn_static_html_server(parent_html).await;
        let parent_url = format!("http://localhost:{}/parent", parent_addr.port());
        let profile = BrowserProfile {
            args: vec!["--site-per-process".to_owned()],
            browser_start_timeout_ms: 30_000,
            ..BrowserProfile::default()
        };
        let session = CdpBrowserSession::launch(&profile)
            .await
            .expect("launch CDP session");

        session
            .navigate(&parent_url, false)
            .await
            .expect("navigate cross-origin parent");
        let mut initial_state = None;
        for _ in 0..20 {
            let state = session.state(false).await.expect("cross-origin state");
            if state.dom_state.llm_representation().contains("Cross stale") {
                initial_state = Some(state);
                break;
            }
            sleep(Duration::from_millis(100)).await;
        }
        let initial_state = initial_state.expect("cross-origin iframe element state");
        let child_button = initial_state
            .dom_state
            .selector_map
            .values()
            .find(|element| element.name.as_deref() == Some("Cross stale"))
            .expect("child button");
        let page = session.current_page().await;
        let frame_infos = session
            .frame_element_infos(&page)
            .await
            .expect("frame element infos");
        let child_page = session
            .iframe_target_pages(&page, &frame_infos)
            .await
            .expect("iframe target pages")
            .into_iter()
            .find(|frame| frame.page.target_id == child_button.target_id)
            .expect("child target page");

        session
            .evaluate_json_for_page(&child_page.page, "replaceChildButton(); true", false)
            .await
            .expect("replace cached child button");
        session
            .click(child_button.index)
            .await
            .expect("fallback child click");
        let clicked = session
            .evaluate_json_for_page(&child_page.page, "document.body.dataset.clicked", false)
            .await
            .expect("child clicked flag");

        assert_eq!(clicked.as_str(), Some("replacement"));
        child_server.abort();
        parent_server.abort();
    }

    #[tokio::test]
    #[ignore = "requires Chrome/Chromium installed on the local machine"]
    async fn cdp_session_extracts_text_and_elements_from_cross_origin_iframe_targets() {
        let child_html = "<html><body><p>Frame only text</p><a id='child-link' href='https://example.com/frame'>Frame link</a></body></html>";
        let (child_addr, child_server) = spawn_static_html_server(child_html.to_owned()).await;
        let child_url = format!("http://127.0.0.1:{}/child", child_addr.port());
        let parent_html = format!(
            "<html><head><title>cross extract smoke</title></head><body><p>Parent only text</p><a id='parent-link' href='https://example.com/parent'>Parent link</a><iframe src='{child_url}' style='width:420px;height:180px;border:0'></iframe></body></html>"
        );
        let (parent_addr, parent_server) = spawn_static_html_server(parent_html).await;
        let parent_url = format!("http://localhost:{}/parent", parent_addr.port());
        let profile = BrowserProfile {
            args: vec!["--site-per-process".to_owned()],
            browser_start_timeout_ms: 30_000,
            ..BrowserProfile::default()
        };
        let session = CdpBrowserSession::launch(&profile)
            .await
            .expect("launch CDP session");

        session
            .navigate(&parent_url, false)
            .await
            .expect("navigate cross-origin parent");
        let mut page_text = String::new();
        for _ in 0..20 {
            page_text = session.page_text().await.expect("page text");
            if page_text.contains("Frame only text") {
                break;
            }
            sleep(Duration::from_millis(100)).await;
        }

        assert!(
            page_text.contains("Parent only text"),
            "missing parent text: {page_text}"
        );
        assert!(
            page_text.contains("Frame only text"),
            "missing child frame text: {page_text}"
        );

        let links = session
            .find_elements("a", &["href".to_owned()], 10, true)
            .await
            .expect("find links");
        assert!(
            links.iter().any(|link| {
                link.text.as_deref() == Some("Parent link")
                    && link.attributes.get("href").map(String::as_str)
                        == Some("https://example.com/parent")
            }),
            "missing parent link: {links:?}"
        );
        assert!(
            links.iter().any(|link| {
                link.text.as_deref() == Some("Frame link")
                    && link.attributes.get("href").map(String::as_str)
                        == Some("https://example.com/frame")
            }),
            "missing child frame link: {links:?}"
        );

        child_server.abort();
        parent_server.abort();
    }

    #[tokio::test]
    #[ignore = "requires Chrome/Chromium installed on the local machine"]
    async fn cdp_session_uses_labels_for_form_control_names() {
        let profile = BrowserProfile::default();
        let session = CdpBrowserSession::launch(&profile)
            .await
            .expect("launch CDP session");

        session
            .navigate(
                "data:text/html,<html><head><title>label smoke</title></head><body><label for='email'>Email address</label><input id='email' placeholder='Placeholder only'><span id='submit-name'>Submit request</span><button aria-labelledby='submit-name'>Ignored text</button></body></html>",
                false,
            )
            .await
            .expect("navigate");
        sleep(Duration::from_millis(100)).await;

        let state = session.state(false).await.expect("state");
        let input = state.dom_state.selector_map.get(&1).expect("labeled input");
        assert_eq!(input.name.as_deref(), Some("Email address"));
        let button = state
            .dom_state
            .selector_map
            .get(&2)
            .expect("labelled button");
        assert_eq!(button.name.as_deref(), Some("Submit request"));
    }

    #[tokio::test]
    #[ignore = "requires Chrome/Chromium installed on the local machine"]
    async fn cdp_session_enriches_dom_from_accessibility_tree() {
        let profile = BrowserProfile::default();
        let session = CdpBrowserSession::launch(&profile)
            .await
            .expect("launch CDP session");

        session
            .navigate(
                "data:text/html,<html><head><title>ax smoke</title></head><body><button id='native-button'>Save settings</button></body></html>",
                false,
            )
            .await
            .expect("navigate");
        sleep(Duration::from_millis(100)).await;

        let state = session.state(false).await.expect("state");
        let button = state
            .dom_state
            .selector_map
            .values()
            .find(|element| {
                element
                    .attributes
                    .get("id")
                    .is_some_and(|value| value == "native-button")
            })
            .expect("native button");

        assert!(button.backend_node_id > 0);
        assert!(button.node_id.is_some_and(|node_id| node_id > 0));
        assert_eq!(button.role.as_deref(), Some("button"));
        assert_eq!(button.name.as_deref(), Some("Save settings"));

        let leaked_probe = session
            .evaluate_json(&format!(
                "document.querySelector('[{}]') !== null",
                AX_REF_ATTRIBUTE
            ))
            .await
            .expect("probe leak check");
        assert_eq!(leaked_probe.as_bool(), Some(false));
    }

    #[tokio::test]
    #[ignore = "requires Chrome/Chromium installed on the local machine"]
    async fn cdp_session_click_uses_cached_observed_node_after_dom_reorder() {
        let profile = BrowserProfile::default();
        let session = CdpBrowserSession::launch(&profile)
            .await
            .expect("launch CDP session");

        session
            .navigate(
                "data:text/html,<html><head><title>stable click smoke</title></head><body><button id='target' onclick=\"document.title='target clicked'\">Target</button><script>function insertBeforeTarget(){const button=document.createElement('button');button.id='inserted';button.textContent='Inserted';button.onclick=()=>{document.title='inserted clicked'};document.body.insertBefore(button, document.getElementById('target'));}</script></body></html>",
                false,
            )
            .await
            .expect("navigate");
        sleep(Duration::from_millis(100)).await;

        let state = session.state(false).await.expect("state");
        let target_index = state
            .dom_state
            .selector_map
            .values()
            .find(|element| {
                element
                    .attributes
                    .get("id")
                    .is_some_and(|value| value == "target")
            })
            .expect("target button")
            .index;

        session
            .evaluate_json("insertBeforeTarget(); true")
            .await
            .expect("insert button before observed target");
        session
            .click(target_index)
            .await
            .expect("click cached target");

        let title = session
            .evaluate_json("document.title")
            .await
            .expect("title");
        assert_eq!(title.as_str(), Some("target clicked"));
    }

    #[tokio::test]
    #[ignore = "requires Chrome/Chromium installed on the local machine"]
    async fn cdp_session_input_uses_cached_observed_node_after_dom_reorder() {
        let profile = BrowserProfile::default();
        let session = CdpBrowserSession::launch(&profile)
            .await
            .expect("launch CDP session");

        session
            .navigate(
                "data:text/html,<html><head><title>stable input smoke</title></head><body><input id='target' placeholder='Target'><script>function insertBeforeTarget(){const input=document.createElement('input');input.id='inserted';input.placeholder='Inserted';document.body.insertBefore(input, document.getElementById('target'));}</script></body></html>",
                false,
            )
            .await
            .expect("navigate");
        sleep(Duration::from_millis(100)).await;

        let state = session.state(false).await.expect("state");
        let target_index = state
            .dom_state
            .selector_map
            .values()
            .find(|element| {
                element
                    .attributes
                    .get("id")
                    .is_some_and(|value| value == "target")
            })
            .expect("target input")
            .index;

        session
            .evaluate_json("insertBeforeTarget(); true")
            .await
            .expect("insert input before observed target");
        session
            .input_text(target_index, "EvalOps", true)
            .await
            .expect("input cached target");

        let values = session
            .evaluate_json(
                "JSON.stringify({ target: document.getElementById('target').value, inserted: document.getElementById('inserted').value })",
            )
            .await
            .expect("values");
        let values: Value =
            serde_json::from_str(values.as_str().expect("encoded values")).expect("values json");
        assert_eq!(values["target"].as_str(), Some("EvalOps"));
        assert_eq!(values["inserted"].as_str(), Some(""));
    }

    #[tokio::test]
    #[ignore = "requires Chrome/Chromium installed on the local machine"]
    async fn cdp_session_scroll_uses_cached_observed_node_after_dom_reorder() {
        let profile = BrowserProfile::default();
        let session = CdpBrowserSession::launch(&profile)
            .await
            .expect("launch CDP session");

        session
            .navigate(
                "data:text/html,<html><head><title>stable scroll smoke</title></head><body><div id='target' tabindex='0' style='height:60px;width:200px;overflow:auto;border:1px solid black'><div style='height:400px'>Target pane</div></div><script>function insertBeforeTarget(){const pane=document.createElement('div');pane.id='inserted';pane.tabIndex=0;pane.style.cssText='height:60px;width:200px;overflow:auto;border:1px solid black';const inner=document.createElement('div');inner.style.height='400px';inner.textContent='Inserted pane';pane.appendChild(inner);document.body.insertBefore(pane, document.getElementById('target'));}</script></body></html>",
                false,
            )
            .await
            .expect("navigate");
        sleep(Duration::from_millis(100)).await;

        let state = session.state(false).await.expect("state");
        let target_index = state
            .dom_state
            .selector_map
            .values()
            .find(|element| {
                element
                    .attributes
                    .get("id")
                    .is_some_and(|value| value == "target")
            })
            .expect("target pane")
            .index;

        session
            .evaluate_json("insertBeforeTarget(); true")
            .await
            .expect("insert pane before observed target");
        session
            .scroll(Some(target_index), true, 1.0)
            .await
            .expect("scroll cached target");

        let values = session
            .evaluate_json(
                "JSON.stringify({ target: document.getElementById('target').scrollTop, inserted: document.getElementById('inserted').scrollTop })",
            )
            .await
            .expect("scroll values");
        let values: Value = serde_json::from_str(values.as_str().expect("encoded scroll values"))
            .expect("scroll values json");
        assert!(values["target"].as_f64().unwrap_or_default() > 0.0);
        assert_eq!(values["inserted"].as_f64(), Some(0.0));
    }

    #[tokio::test]
    #[ignore = "requires Chrome/Chromium installed on the local machine"]
    async fn cdp_session_dropdown_uses_cached_observed_node_after_dom_reorder() {
        let profile = BrowserProfile::default();
        let session = CdpBrowserSession::launch(&profile)
            .await
            .expect("launch CDP session");

        session
            .navigate(
                "data:text/html,<html><head><title>stable dropdown smoke</title></head><body><select id='target'><option>Starter</option><option>Enterprise</option></select><script>function insertBeforeTarget(){const select=document.createElement('select');select.id='inserted';select.innerHTML='<option>Inserted A</option><option>Inserted B</option>';document.body.insertBefore(select, document.getElementById('target'));}</script></body></html>",
                false,
            )
            .await
            .expect("navigate");
        sleep(Duration::from_millis(100)).await;

        let state = session.state(false).await.expect("state");
        let target_index = state
            .dom_state
            .selector_map
            .values()
            .find(|element| {
                element
                    .attributes
                    .get("id")
                    .is_some_and(|value| value == "target")
            })
            .expect("target select")
            .index;

        session
            .evaluate_json("insertBeforeTarget(); true")
            .await
            .expect("insert select before observed target");
        let options = session
            .dropdown_options(target_index)
            .await
            .expect("cached target options");
        assert_eq!(options, ["Starter", "Enterprise"]);

        session
            .select_dropdown_option(target_index, "Enterprise")
            .await
            .expect("select cached target option");
        let values = session
            .evaluate_json(
                "JSON.stringify({ target: document.getElementById('target').value, inserted: document.getElementById('inserted').value })",
            )
            .await
            .expect("select values");
        let values: Value = serde_json::from_str(values.as_str().expect("encoded select values"))
            .expect("select values json");
        assert_eq!(values["target"].as_str(), Some("Enterprise"));
        assert_eq!(values["inserted"].as_str(), Some("Inserted A"));
    }

    #[tokio::test]
    #[ignore = "requires Chrome/Chromium installed on the local machine"]
    async fn cdp_session_upload_uses_cached_observed_node_after_dom_reorder() {
        let profile = BrowserProfile::default();
        let session = CdpBrowserSession::launch(&profile)
            .await
            .expect("launch CDP session");
        let upload_dir = tempfile::tempdir().expect("upload temp dir");
        let upload_path = upload_dir.path().join("cached-upload.txt");
        std::fs::write(&upload_path, "EvalOps cached upload").expect("write upload file");

        session
            .navigate(
                "data:text/html,<html><head><title>stable upload smoke</title></head><body><input id='target' type='file'><script>function insertBeforeTarget(){const input=document.createElement('input');input.id='inserted';input.type='file';document.body.insertBefore(input, document.getElementById('target'));}</script></body></html>",
                false,
            )
            .await
            .expect("navigate");
        sleep(Duration::from_millis(100)).await;

        let state = session.state(false).await.expect("state");
        let target_index = state
            .dom_state
            .selector_map
            .values()
            .find(|element| {
                element
                    .attributes
                    .get("id")
                    .is_some_and(|value| value == "target")
            })
            .expect("target file input")
            .index;

        session
            .evaluate_json("insertBeforeTarget(); true")
            .await
            .expect("insert file input before observed target");
        session
            .upload_file(target_index, &upload_path)
            .await
            .expect("upload cached target");

        let values = session
            .evaluate_json(
                "JSON.stringify({ target: document.getElementById('target').files[0]?.name || '', inserted: document.getElementById('inserted').files[0]?.name || '' })",
            )
            .await
            .expect("upload values");
        let values: Value = serde_json::from_str(values.as_str().expect("encoded upload values"))
            .expect("upload values json");
        assert_eq!(values["target"].as_str(), Some("cached-upload.txt"));
        assert_eq!(values["inserted"].as_str(), Some(""));
    }

    #[tokio::test]
    #[ignore = "requires Chrome/Chromium installed on the local machine"]
    async fn cdp_session_indexes_hidden_file_inputs_for_upload() {
        let profile = BrowserProfile::default();
        let session = CdpBrowserSession::launch(&profile)
            .await
            .expect("launch CDP session");
        let upload_dir = tempfile::tempdir().expect("upload temp dir");
        let upload_path = upload_dir.path().join("hidden-upload.txt");
        std::fs::write(&upload_path, "EvalOps hidden upload").expect("write upload file");

        session
            .navigate(
                "data:text/html,<html><head><title>hidden upload smoke</title></head><body><label for='hidden-file'>Upload</label><input id='hidden-file' type='file' style='display:none' onchange=\"document.body.dataset.uploaded=this.files[0]?.name || ''\"></body></html>",
                false,
            )
            .await
            .expect("navigate");
        sleep(Duration::from_millis(100)).await;

        let state = session.state(false).await.expect("state");
        let hidden_file = state
            .dom_state
            .selector_map
            .values()
            .find(|element| {
                element
                    .attributes
                    .get("id")
                    .is_some_and(|value| value == "hidden-file")
            })
            .expect("hidden file input indexed");
        assert_eq!(hidden_file.tag_name, "input");
        assert_eq!(
            hidden_file.attributes.get("type").map(String::as_str),
            Some("file")
        );

        session
            .upload_file(hidden_file.index, &upload_path)
            .await
            .expect("upload hidden file input");
        let uploaded_name = session
            .evaluate_json("document.body.dataset.uploaded || ''")
            .await
            .expect("uploaded file name");
        assert_eq!(uploaded_name.as_str(), Some("hidden-upload.txt"));
    }

    #[tokio::test]
    #[ignore = "requires Chrome/Chromium installed on the local machine"]
    async fn cdp_session_uses_image_alt_for_control_names() {
        let profile = BrowserProfile::default();
        let session = CdpBrowserSession::launch(&profile)
            .await
            .expect("launch CDP session");

        session
            .navigate(
                "data:text/html,<html><head><title>alt smoke</title></head><body><a id='report' href='https://example.com/report'><img alt='Download report' src='data:image/gif;base64,R0lGODlhAQABAIAAAAAAAP///ywAAAAAAQABAAACAUwAOw==' style='width:24px;height:24px'></a><button id='settings'><img alt='Open settings' src='data:image/gif;base64,R0lGODlhAQABAIAAAAAAAP///ywAAAAAAQABAAACAUwAOw==' style='width:24px;height:24px'></button><input id='image-submit' type='image' alt='Search icon' style='width:24px;height:24px'></body></html>",
                false,
            )
            .await
            .expect("navigate");
        sleep(Duration::from_millis(100)).await;

        let state = session.state(false).await.expect("state");
        let element_by_id = |id: &str| {
            state
                .dom_state
                .selector_map
                .values()
                .find(|element| {
                    element
                        .attributes
                        .get("id")
                        .is_some_and(|value| value == id)
                })
                .unwrap_or_else(|| panic!("missing interactive element with id {id}"))
        };

        assert_eq!(
            element_by_id("report").name.as_deref(),
            Some("Download report")
        );
        assert_eq!(
            element_by_id("settings").name.as_deref(),
            Some("Open settings")
        );
        assert_eq!(
            element_by_id("image-submit").name.as_deref(),
            Some("Search icon")
        );
    }

    #[tokio::test]
    #[ignore = "requires Chrome/Chromium installed on the local machine"]
    async fn cdp_session_skips_decorative_svg_children() {
        let profile = BrowserProfile::default();
        let session = CdpBrowserSession::launch(&profile)
            .await
            .expect("launch CDP session");

        session
            .navigate(
                "data:text/html,<html><head><title>svg smoke</title></head><body><svg id='svg-button' role='button' aria-label='Open vector' onclick=\"document.title='svg clicked'\" width='32' height='32'><path id='decorative-path' onclick=\"document.title='path clicked'\" d='M0 0h32v32H0z'></path></svg></body></html>",
                false,
            )
            .await
            .expect("navigate");
        sleep(Duration::from_millis(100)).await;

        let state = session.state(false).await.expect("state");
        let svg = state
            .dom_state
            .selector_map
            .values()
            .find(|element| {
                element
                    .attributes
                    .get("id")
                    .is_some_and(|value| value == "svg-button")
            })
            .expect("svg root indexed");
        assert_eq!(svg.tag_name, "svg");
        assert_eq!(svg.role.as_deref(), Some("button"));
        assert!(state.dom_state.selector_map.values().all(|element| {
            element
                .attributes
                .get("id")
                .is_none_or(|id| id != "decorative-path")
        }));

        session.click(svg.index).await.expect("click svg by index");
        let title = session
            .evaluate_json("document.title")
            .await
            .expect("document title");
        assert_eq!(title.as_str(), Some("svg clicked"));
    }

    #[tokio::test]
    #[ignore = "requires Chrome/Chromium installed on the local machine"]
    async fn cdp_session_filters_occluded_elements() {
        let profile = BrowserProfile::default();
        let session = CdpBrowserSession::launch(&profile)
            .await
            .expect("launch CDP session");

        session
            .navigate(
                "data:text/html,<html><head><title>occlusion smoke</title></head><body><button id='covered' onclick=\"document.title='covered clicked'\" style='position:absolute;left:20px;top:20px;width:120px;height:40px'>Covered</button><div id='cover' style='position:absolute;left:0;top:0;width:220px;height:100px;background:white;z-index:2'></div><button id='visible' onclick=\"document.title='visible clicked'\" style='position:absolute;left:20px;top:140px;width:120px;height:40px'>Visible</button></body></html>",
                false,
            )
            .await
            .expect("navigate");
        sleep(Duration::from_millis(100)).await;

        let state = session.state(false).await.expect("state");
        let ids = state
            .dom_state
            .selector_map
            .values()
            .filter_map(|element| element.attributes.get("id").map(String::as_str))
            .collect::<Vec<_>>();

        assert!(ids.contains(&"visible"), "missing visible button: {ids:?}");
        assert!(
            !ids.contains(&"covered"),
            "covered button should not be indexed: {ids:?}"
        );

        let visible = state
            .dom_state
            .selector_map
            .values()
            .find(|element| {
                element
                    .attributes
                    .get("id")
                    .is_some_and(|value| value == "visible")
            })
            .expect("visible button indexed");

        session.click(visible.index).await.expect("click visible");
        let title = session
            .evaluate_json("document.title")
            .await
            .expect("document title");
        assert_eq!(title.as_str(), Some("visible clicked"));
    }

    #[tokio::test]
    #[ignore = "requires Chrome/Chromium installed on the local machine"]
    async fn cdp_session_uses_selected_option_as_select_text() {
        let profile = BrowserProfile::default();
        let session = CdpBrowserSession::launch(&profile)
            .await
            .expect("launch CDP session");

        session
            .navigate(
                "data:text/html,<html><head><title>select smoke</title></head><body><label for='plan'>Plan</label><select id='plan'><option>Starter</option><option selected>Enterprise</option><option>Internal</option></select></body></html>",
                false,
            )
            .await
            .expect("navigate");
        sleep(Duration::from_millis(100)).await;

        let state = session.state(false).await.expect("state");
        let select = state.dom_state.selector_map.get(&1).expect("select");
        assert_eq!(select.name.as_deref(), Some("Plan"));
        assert_eq!(select.text.as_deref(), Some("Enterprise"));
        let compound_components = select
            .attributes
            .get("compound_components")
            .expect("select compound components");
        assert!(compound_components.contains("Dropdown Toggle"));
        assert!(compound_components.contains("count=3"));
        assert!(compound_components.contains("options=Starter|Enterprise|Internal"));
        let llm_representation = state.dom_state.llm_representation();
        assert!(
            llm_representation.contains("Plan Enterprise"),
            "DOM state did not include selected option value: {llm_representation}",
        );
        assert!(
            !llm_representation.contains("> Plan Starter"),
            "DOM state included unselected option as visible text: {llm_representation}",
        );
    }

    #[tokio::test]
    #[ignore = "requires Chrome/Chromium installed on the local machine"]
    async fn cdp_session_uses_accessibility_state_properties() {
        let profile = BrowserProfile::default();
        let session = CdpBrowserSession::launch(&profile)
            .await
            .expect("launch CDP session");

        session
            .navigate(
                "data:text/html,<html><head><title>ax props smoke</title></head><body><button id='toggle' aria-expanded='true'>Details</button><div id='slider' role='slider' aria-valuemin='0' aria-valuemax='10' aria-valuenow='7' aria-valuetext='Seven'>Volume</div><div id='results' role='listbox' aria-busy='true' aria-live='polite' aria-level='2' aria-multiselectable='true'>Results</div></body></html>",
                false,
            )
            .await
            .expect("navigate");
        sleep(Duration::from_millis(150)).await;

        let state = session.state(false).await.expect("state");
        let llm = state.dom_state.llm_representation();
        assert!(
            llm.contains("expanded=true"),
            "DOM state did not include AX expanded property: {llm}"
        );
        assert!(
            !llm.contains("aria-expanded=true"),
            "DOM state did not prefer AX expanded over aria-expanded: {llm}"
        );
        assert!(
            llm.contains("valuetext=Seven"),
            "DOM state did not include human-readable value text: {llm}"
        );
        assert!(
            llm.contains("valuemin=0") && llm.contains("valuemax=10") && llm.contains("valuenow=7"),
            "DOM state did not include AX-shaped numeric value metadata: {llm}"
        );
        assert!(
            !llm.contains("aria-valuenow=7"),
            "DOM state did not prefer AX-shaped value aliases over aria value attributes: {llm}"
        );
        assert!(
            llm.contains("busy=true") || llm.contains("busy=1"),
            "DOM state did not include busy live-region state: {llm}"
        );
        assert!(
            llm.contains("live=polite"),
            "DOM state did not include live-region politeness: {llm}"
        );
        assert!(
            llm.contains("level=2"),
            "DOM state did not include hierarchy level: {llm}"
        );
        assert!(
            llm.contains("multiselectable=true"),
            "DOM state did not include multiselectable state: {llm}"
        );
    }

    #[tokio::test]
    #[ignore = "requires Chrome/Chromium installed on the local machine"]
    async fn cdp_session_detects_pagination_buttons() {
        let profile = BrowserProfile::default();
        let session = CdpBrowserSession::launch(&profile)
            .await
            .expect("launch CDP session");

        session
            .navigate(
                "data:text/html,<html><head><title>pagination smoke</title></head><body><nav><button id='previous' class='disabled'>Previous</button><a id='page-two' href='https://example.com/page/2'>2</a><button id='next'>Next</button><button id='export'>Export</button></nav></body></html>",
                false,
            )
            .await
            .expect("navigate");
        sleep(Duration::from_millis(100)).await;

        let state = session.state(false).await.expect("state");
        assert_eq!(state.pagination_buttons.len(), 3);
        assert!(state.pagination_buttons.iter().any(|button| {
            button.button_type == PaginationButtonType::Prev
                && button.text.contains("Previous")
                && button.is_disabled
        }));
        assert!(state.pagination_buttons.iter().any(|button| {
            button.button_type == PaginationButtonType::Next && button.selector == "#next"
        }));
        assert!(state.pagination_buttons.iter().any(|button| {
            button.button_type == PaginationButtonType::PageNumber && button.text == "2"
        }));
    }

    #[tokio::test]
    #[ignore = "requires Chrome/Chromium installed on the local machine"]
    async fn cdp_session_indexes_accessibility_widget_roles() {
        let profile = BrowserProfile::default();
        let session = CdpBrowserSession::launch(&profile)
            .await
            .expect("launch CDP session");

        session
            .navigate(
                "data:text/html,<html><head><title>roles smoke</title></head><body><details id='details'><summary id='summary'>More details</summary><p>Body</p></details><div id='menuitem' role='menuitem' aria-label='Open menu'>Menu</div><div id='checkbox' role='checkbox' aria-checked='false'>Subscribe</div><div id='hidden-role' role='button' aria-hidden='true'>Hidden role</div><button id='disabled-button' disabled>Disabled</button></body></html>",
                false,
            )
            .await
            .expect("navigate");
        sleep(Duration::from_millis(100)).await;

        let state = session.state(false).await.expect("state");
        let element_by_id = |id: &str| {
            state
                .dom_state
                .selector_map
                .values()
                .find(|element| {
                    element
                        .attributes
                        .get("id")
                        .is_some_and(|value| value == id)
                })
                .unwrap_or_else(|| panic!("missing interactive element with id {id}"))
        };

        let summary = element_by_id("summary");
        assert_eq!(summary.tag_name, "summary");
        assert_eq!(summary.name.as_deref(), Some("More details"));

        let menuitem = element_by_id("menuitem");
        assert_eq!(menuitem.role.as_deref(), Some("menuitem"));
        assert_eq!(menuitem.name.as_deref(), Some("Open menu"));

        let checkbox = element_by_id("checkbox");
        assert_eq!(checkbox.role.as_deref(), Some("checkbox"));
        assert_eq!(checkbox.name.as_deref(), Some("Subscribe"));

        assert!(state.dom_state.selector_map.values().all(|element| {
            element
                .attributes
                .get("id")
                .is_none_or(|id| id != "hidden-role" && id != "disabled-button")
        }));

        session
            .click(summary.index)
            .await
            .expect("click summary element");
        let details_open = session
            .evaluate_json("document.getElementById('details').open")
            .await
            .expect("details open");
        assert_eq!(details_open.as_bool(), Some(true));
    }

    #[tokio::test]
    #[ignore = "requires Chrome/Chromium installed on the local machine"]
    async fn cdp_session_indexes_anchor_without_href() {
        let profile = BrowserProfile::default();
        let session = CdpBrowserSession::launch(&profile)
            .await
            .expect("launch CDP session");

        session
            .navigate(
                "data:text/html,<html><head><title>plain anchor smoke</title></head><body><a id='plain-anchor'>Plain Anchor</a><a id='href-anchor' href='/target'>Href Anchor</a><button id='button'>Button</button></body></html>",
                false,
            )
            .await
            .expect("navigate");
        sleep(Duration::from_millis(100)).await;

        let state = session.state(false).await.expect("state");
        let element_by_id = |id: &str| {
            state
                .dom_state
                .selector_map
                .values()
                .find(|element| {
                    element
                        .attributes
                        .get("id")
                        .is_some_and(|value| value == id)
                })
                .unwrap_or_else(|| panic!("missing interactive element with id {id}"))
        };

        let plain_anchor = element_by_id("plain-anchor");
        assert_eq!(plain_anchor.tag_name, "a");
        assert_eq!(plain_anchor.name.as_deref(), Some("Plain Anchor"));
        assert!(!plain_anchor.attributes.contains_key("href"));

        let href_anchor = element_by_id("href-anchor");
        assert_eq!(href_anchor.tag_name, "a");
        assert_eq!(
            href_anchor.attributes.get("href").map(String::as_str),
            Some("/target")
        );

        session
            .click(plain_anchor.index)
            .await
            .expect("click plain anchor by index");
    }

    #[tokio::test]
    #[ignore = "requires Chrome/Chromium installed on the local machine"]
    async fn cdp_session_indexes_aria_interactivity_properties() {
        let profile = BrowserProfile::default();
        let session = CdpBrowserSession::launch(&profile)
            .await
            .expect("launch CDP session");

        session
            .navigate(
                "data:text/html,<html><head><title>aria property smoke</title></head><body><div id='required-proxy' aria-required='true' aria-label='Required proxy'>Required</div><div id='autocomplete-proxy' aria-autocomplete='list' aria-label='Autocomplete proxy'>Autocomplete</div><div id='shortcut-proxy' aria-keyshortcuts='Alt+S' aria-label='Shortcut proxy'>Shortcut</div><div id='autocomplete-none' aria-autocomplete='none'>Ignored autocomplete</div></body></html>",
                false,
            )
            .await
            .expect("navigate");
        sleep(Duration::from_millis(100)).await;

        let state = session.state(false).await.expect("state");
        let element_by_id = |id: &str| {
            state
                .dom_state
                .selector_map
                .values()
                .find(|element| {
                    element
                        .attributes
                        .get("id")
                        .is_some_and(|value| value == id)
                })
                .unwrap_or_else(|| panic!("missing interactive element with id {id}"))
        };

        let required = element_by_id("required-proxy");
        assert_eq!(required.name.as_deref(), Some("Required proxy"));
        assert_eq!(
            required.attributes.get("aria-required").map(String::as_str),
            Some("true")
        );

        let autocomplete = element_by_id("autocomplete-proxy");
        assert_eq!(
            autocomplete
                .attributes
                .get("aria-autocomplete")
                .map(String::as_str),
            Some("list")
        );

        let shortcut = element_by_id("shortcut-proxy");
        assert_eq!(
            shortcut
                .attributes
                .get("aria-keyshortcuts")
                .map(String::as_str),
            Some("Alt+S")
        );

        assert!(state.dom_state.selector_map.values().all(|element| {
            element
                .attributes
                .get("id")
                .is_none_or(|id| id != "autocomplete-none")
        }));

        session
            .click(shortcut.index)
            .await
            .expect("click shortcut proxy by index");
    }

    #[tokio::test]
    #[ignore = "requires Chrome/Chromium installed on the local machine"]
    async fn cdp_session_indexes_contenteditable_variants() {
        let profile = BrowserProfile::default();
        let session = CdpBrowserSession::launch(&profile)
            .await
            .expect("launch CDP session");

        session
            .navigate(
                "data:text/html,<html><head><title>contenteditable smoke</title></head><body><div id='plain-editor' contenteditable='plaintext-only' aria-label='Plain editor'>Draft</div><div id='true-editor' contenteditable='true' aria-label='True editor'>Rich</div><div id='false-editor' contenteditable='false' aria-label='False editor'>Ignored</div></body></html>",
                false,
            )
            .await
            .expect("navigate");
        sleep(Duration::from_millis(100)).await;

        let state = session.state(false).await.expect("state");
        let element_by_id = |id: &str| {
            state
                .dom_state
                .selector_map
                .values()
                .find(|element| {
                    element
                        .attributes
                        .get("id")
                        .is_some_and(|value| value == id)
                })
                .unwrap_or_else(|| panic!("missing contenteditable element with id {id}"))
        };

        let plain = element_by_id("plain-editor");
        assert_eq!(plain.name.as_deref(), Some("Plain editor"));
        assert_eq!(
            plain.attributes.get("contenteditable").map(String::as_str),
            Some("plaintext-only")
        );

        let rich = element_by_id("true-editor");
        assert_eq!(rich.name.as_deref(), Some("True editor"));
        assert_eq!(
            rich.attributes.get("contenteditable").map(String::as_str),
            Some("true")
        );

        assert!(state.dom_state.selector_map.values().all(|element| {
            element
                .attributes
                .get("id")
                .is_none_or(|id| id != "false-editor")
        }));
    }

    #[tokio::test]
    #[ignore = "requires Chrome/Chromium installed on the local machine"]
    async fn cdp_session_indexes_media_controls() {
        let profile = BrowserProfile::default();
        let session = CdpBrowserSession::launch(&profile)
            .await
            .expect("launch CDP session");

        session
            .navigate(
                "data:text/html,<html><head><title>media smoke</title></head><body><audio id='audio-player' controls aria-label='Audio sample' style='display:block;width:320px;height:54px' src='data:audio/wav;base64,UklGRiQAAABXQVZFZm10IBAAAAABAAEAESsAACJWAAACABAAZGF0YQAAAAA='></audio><video id='video-player' controls aria-label='Video sample' width='320' height='180' src='data:video/webm;base64,GkXfo59ChoEBQveBAULygQRC84EIQoKEd2VibUKHgQJChYECGFOAZwEAAAAAAAIyEU2bdLpNu4tTq4QVSalmU6yBoU27i1OrhBZUrmtTrIHYTbuMU6uEElTDZ1OsggElTbuMU6uEHFO7a1OsggIc7AEAAAAAAABZAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAVSalmsirXsYMPQkBNgI1MYXZmNjIuMTIuMTAwV0GNTGF2ZjYyLjEyLjEwMESJiEBeAAAAAAAAFlSua8iuAQAAAAAAAD/XgQFzxYitKhHKPYuxgJyBACK1nIN1bmSIgQCGhVZfVlA5g4EBI+ODhAJiWgDgkLCBELqBEJqBAlWwhFW5gQESVMNnQIBzc6BjwIBnyJpFo4dFTkNPREVSRIeNTGF2ZjYyLjEyLjEwMHNz2mPAi2PFiK0qEco9i7GAZ8ilRaOHRU5DT0RFUkSHmExhdmM2Mi4yOC4xMDAgbGlidnB4LXZwOWfIoUWjiERVUkFUSU9ORIeTMDA6MDA6MDAuMTIwMDAwMDAwAB9DtnXs54EAo72BAACAgkmDQgAA8AD2BjgkHBhKAAAgQAAim///lXb23/SskhXr7zdPyoCRyEjNuPymkNJQgETBR424BAAAo5OBACgAhgBAkpxIUAAAA3AAAEJAo5OBAFAAhgBAkpxATuAAA3AAAEJAHFO7a5G7j7OBALeK94EB8YIBq/CBAw=='></video><audio id='silent-audio' aria-label='Silent sample'></audio></body></html>",
                false,
            )
            .await
            .expect("navigate");
        sleep(Duration::from_millis(100)).await;

        let state = session.state(false).await.expect("state");
        let element_by_id = |id: &str| {
            state
                .dom_state
                .selector_map
                .values()
                .find(|element| {
                    element
                        .attributes
                        .get("id")
                        .is_some_and(|value| value == id)
                })
                .unwrap_or_else(|| panic!("missing media element with id {id}"))
        };

        let audio = element_by_id("audio-player");
        assert_eq!(audio.tag_name, "audio");
        assert!(
            audio
                .attributes
                .get("compound_components")
                .is_some_and(|value| value.contains("Play/Pause") && value.contains("Volume"))
        );

        let video = element_by_id("video-player");
        assert_eq!(video.tag_name, "video");
        assert!(
            video
                .attributes
                .get("compound_components")
                .is_some_and(|value| value.contains("Fullscreen"))
        );

        assert!(state.dom_state.selector_map.values().all(|element| {
            element
                .attributes
                .get("id")
                .is_none_or(|id| id != "silent-audio")
        }));
    }

    #[tokio::test]
    #[ignore = "requires Chrome/Chromium installed on the local machine"]
    async fn cdp_session_skips_browser_use_excluded_elements() {
        let profile = BrowserProfile::default();
        let session = CdpBrowserSession::launch(&profile)
            .await
            .expect("launch CDP session");

        session
            .navigate(
                "data:text/html,<html><head><title>exclude smoke</title></head><body><button id='visible' onclick=\"document.title='visible clicked'\">Visible</button><button id='legacy' data-browser-use-exclude='true'>Legacy</button><div id='scoped' data-browser-use-exclude-demo='TRUE'><button id='nested'>Nested</button></div></body></html>",
                false,
            )
            .await
            .expect("navigate");
        sleep(Duration::from_millis(100)).await;

        let state = session.state(false).await.expect("state");
        let visible = state
            .dom_state
            .selector_map
            .values()
            .find(|element| {
                element
                    .attributes
                    .get("id")
                    .is_some_and(|value| value == "visible")
            })
            .expect("visible button indexed");

        assert!(state.dom_state.selector_map.values().all(|element| {
            element
                .attributes
                .get("id")
                .is_none_or(|id| id != "legacy" && id != "scoped" && id != "nested")
        }));

        session
            .click(visible.index)
            .await
            .expect("click visible element by index");
        let title = session
            .evaluate_json("document.title")
            .await
            .expect("document title");
        assert_eq!(title.as_str(), Some("visible clicked"));
    }

    #[tokio::test]
    #[ignore = "requires Chrome/Chromium installed on the local machine"]
    async fn cdp_session_indexes_search_affordance_signals() {
        let profile = BrowserProfile::default();
        let session = CdpBrowserSession::launch(&profile)
            .await
            .expect("launch CDP session");

        session
            .navigate(
                "data:text/html,<html><head><title>search smoke</title></head><body><div id='site-search' class='search-icon' style='width:24px;height:24px'>Find</div><div data-action='open-search' style='width:24px;height:24px'>Lookup</div></body></html>",
                false,
            )
            .await
            .expect("navigate");
        sleep(Duration::from_millis(100)).await;

        let state = session.state(false).await.expect("state");
        let search = state
            .dom_state
            .selector_map
            .values()
            .find(|element| {
                element
                    .attributes
                    .get("id")
                    .is_some_and(|value| value == "site-search")
            })
            .expect("search affordance indexed");
        assert_eq!(search.tag_name, "div");
        assert_eq!(search.name.as_deref(), Some("Find"));

        session
            .click(search.index)
            .await
            .expect("click search affordance by index");
    }

    #[tokio::test]
    #[ignore = "requires Chrome/Chromium installed on the local machine"]
    async fn cdp_session_indexes_small_icon_controls() {
        let profile = BrowserProfile::default();
        let session = CdpBrowserSession::launch(&profile)
            .await
            .expect("launch CDP session");

        session
            .navigate(
                "data:text/html,<html><head><title>icon smoke</title></head><body><span id='favorite-icon' data-action='favorite' aria-label='Favorite' style='display:inline-block;width:24px;height:24px'></span><span id='plain-small' style='display:inline-block;width:24px;height:24px'></span></body></html>",
                false,
            )
            .await
            .expect("navigate");
        sleep(Duration::from_millis(100)).await;

        let state = session.state(false).await.expect("state");
        let favorite = state
            .dom_state
            .selector_map
            .values()
            .find(|element| {
                element
                    .attributes
                    .get("id")
                    .is_some_and(|value| value == "favorite-icon")
            })
            .expect("icon control indexed");
        assert_eq!(favorite.tag_name, "span");
        assert_eq!(favorite.name.as_deref(), Some("Favorite"));
        assert!(state.dom_state.selector_map.values().all(|element| {
            element
                .attributes
                .get("id")
                .is_none_or(|id| id != "plain-small")
        }));

        session
            .click(favorite.index)
            .await
            .expect("click icon control by index");
    }

    #[tokio::test]
    #[ignore = "requires Chrome/Chromium installed on the local machine"]
    async fn cdp_session_indexes_pointer_cursor_elements() {
        let profile = BrowserProfile::default();
        let session = CdpBrowserSession::launch(&profile)
            .await
            .expect("launch CDP session");

        session
            .navigate(
                "data:text/html,<html><head><title>pointer cursor</title></head><body><div id='pointer' style='cursor:pointer;width:120px;height:32px'>Pointer target</div><div id='plain' style='width:120px;height:32px'>Plain target</div></body></html>",
                false,
            )
            .await
            .expect("navigate");
        sleep(Duration::from_millis(100)).await;

        let state = session.state(false).await.expect("state");
        let ids = state
            .dom_state
            .selector_map
            .values()
            .filter_map(|element| element.attributes.get("id").map(String::as_str))
            .collect::<Vec<_>>();
        assert!(
            ids.contains(&"pointer"),
            "DOM state did not index pointer cursor control: {}",
            state.dom_state.llm_representation()
        );
        assert!(
            !ids.contains(&"plain"),
            "plain non-pointer div should not be indexed: {}",
            state.dom_state.llm_representation()
        );
    }

    #[tokio::test]
    #[ignore = "requires Chrome/Chromium installed on the local machine"]
    async fn cdp_session_indexes_static_handlers_and_listboxes() {
        let profile = BrowserProfile::default();
        let session = CdpBrowserSession::launch(&profile)
            .await
            .expect("launch CDP session");

        session
            .navigate(
                "data:text/html,<html><head><title>static handler</title></head><body><div id='choices' role='listbox' style='width:160px;height:32px'>Choices</div><div id='mouse-down' onmousedown='document.body.dataset.mouse=\"down\"' style='width:120px;height:32px'>Mouse down</div><div id='key-down' onkeydown='document.body.dataset.key=\"down\"' style='width:120px;height:32px'>Key down</div><div id='plain-static' style='width:120px;height:32px'>Plain static</div></body></html>",
                false,
            )
            .await
            .expect("navigate");
        sleep(Duration::from_millis(100)).await;

        let state = session.state(false).await.expect("state");
        let ids = state
            .dom_state
            .selector_map
            .values()
            .filter_map(|element| element.attributes.get("id").map(String::as_str))
            .collect::<Vec<_>>();
        for expected in ["choices", "mouse-down", "key-down"] {
            assert!(
                ids.contains(&expected),
                "DOM state did not index {expected}: {}",
                state.dom_state.llm_representation()
            );
        }
        assert!(
            !ids.contains(&"plain-static"),
            "plain static div should not be indexed: {}",
            state.dom_state.llm_representation()
        );
    }

    #[tokio::test]
    #[ignore = "requires Chrome/Chromium installed on the local machine"]
    async fn cdp_session_indexes_negative_tabindex_like_upstream() {
        let profile = BrowserProfile {
            browser_start_timeout_ms: 30_000,
            ..BrowserProfile::default()
        };
        let session = CdpBrowserSession::launch(&profile)
            .await
            .expect("launch CDP session");

        session
            .navigate(
                "data:text/html,<html><head><title>tabindex smoke</title></head><body><div id='negative-tabindex' tabindex='-1' style='width:140px;height:32px'>Programmatic focus target</div><div id='plain-tabindex' tabindex='0' style='width:140px;height:32px'>Keyboard focus target</div><div id='plain-div' style='width:140px;height:32px'>Plain div</div></body></html>",
                false,
            )
            .await
            .expect("navigate");
        sleep(Duration::from_millis(100)).await;

        let state = session.state(false).await.expect("state");
        let ids = state
            .dom_state
            .selector_map
            .values()
            .filter_map(|element| element.attributes.get("id").map(String::as_str))
            .collect::<Vec<_>>();
        for expected in ["negative-tabindex", "plain-tabindex"] {
            assert!(
                ids.contains(&expected),
                "DOM state did not index {expected}: {}",
                state.dom_state.llm_representation()
            );
        }
        assert!(
            !ids.contains(&"plain-div"),
            "plain div should not be indexed: {}",
            state.dom_state.llm_representation()
        );
    }

    #[tokio::test]
    #[ignore = "requires Chrome/Chromium installed on the local machine"]
    async fn cdp_session_blocks_disallowed_profile_navigation() {
        let profile = BrowserProfile {
            allowed_domains: vec!["example.com".to_owned()],
            browser_start_timeout_ms: 30_000,
            ..BrowserProfile::default()
        };
        let session = CdpBrowserSession::launch(&profile)
            .await
            .expect("launch CDP session");

        let error = session
            .navigate("https://blocked.test", false)
            .await
            .expect_err("disallowed navigation should be blocked before CDP navigation");

        assert!(matches!(
            error,
            BrowserError::NavigationBlocked { ref reason, .. } if reason == "not_in_allowed_domains"
        ));

        let state = session
            .state(false)
            .await
            .expect("state after blocked preflight navigation");
        assert!(
            state
                .recent_events
                .as_deref()
                .is_some_and(|events| events.contains("no browser navigation was started")),
            "blocked preflight diagnostics missing from state: {state:?}"
        );
    }

    #[tokio::test]
    #[ignore = "requires Chrome/Chromium installed on the local machine"]
    async fn cdp_session_resets_disallowed_redirect_after_navigation() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let profile = BrowserProfile {
            block_ip_addresses: true,
            browser_start_timeout_ms: 30_000,
            ..BrowserProfile::default()
        };
        let session = CdpBrowserSession::launch(&profile)
            .await
            .expect("launch CDP session");
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind redirect server");
        let server_addr = listener.local_addr().expect("redirect server address");
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.expect("accept redirect request");
            let mut buffer = [0_u8; 1024];
            let _ = stream.read(&mut buffer).await;
            stream
                .write_all(
                    b"HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:1/blocked\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                )
                .await
                .expect("write redirect response");
        });
        let start_url = format!("http://localhost:{}/start", server_addr.port());

        let error = session
            .navigate(&start_url, false)
            .await
            .expect_err("redirected navigation should be reset by URL policy");
        server.await.expect("redirect server task");

        assert!(
            matches!(error, BrowserError::NavigationBlocked { .. }),
            "unexpected redirect policy error: {error:?}"
        );

        sleep(Duration::from_millis(250)).await;
        let state = session.state(false).await.expect("state after reset");
        assert_eq!(state.url, "about:blank");
    }

    #[tokio::test]
    #[ignore = "requires Chrome/Chromium installed on the local machine"]
    async fn cdp_session_watchdog_closes_disallowed_unsolicited_new_tab_before_state() {
        let profile = BrowserProfile {
            block_ip_addresses: true,
            browser_start_timeout_ms: 30_000,
            ..BrowserProfile::default()
        };
        let session = CdpBrowserSession::launch(&profile)
            .await
            .expect("launch CDP session");
        let blocked_target_id = create_target(&session.connection, "http://127.0.0.1:1/popup")
            .await
            .expect("create blocked tab");

        sleep(Duration::from_millis(500)).await;
        let tabs = page_tabs(&session.connection)
            .await
            .expect("tabs after watchdog enforcement");
        assert!(
            tabs.iter().all(|tab| tab.target_id != blocked_target_id),
            "blocked tab still open before state/action boundary: {tabs:?}"
        );

        let error = session
            .state(false)
            .await
            .expect_err("state observation should report watchdog-blocked tab");

        assert!(
            matches!(
                error,
                BrowserError::NavigationBlocked { ref url, ref reason }
                    if url.starts_with("http://127.0.0.1:1/popup")
                        && reason == "ip_address_blocked"
            ),
            "unexpected blocked popup policy error: {error:?}"
        );

        let state = session
            .state(false)
            .await
            .expect("state after watchdog policy error was reported");
        assert!(
            state
                .closed_popup_messages
                .iter()
                .any(|message| message.contains("http://127.0.0.1:1/popup")),
            "closed popup diagnostics missing from state: {state:?}"
        );
        assert!(
            state
                .recent_events
                .as_deref()
                .is_some_and(|events| events.contains("Closed popup")),
            "recent security events missing from state: {state:?}"
        );
    }

    #[tokio::test]
    #[ignore = "requires Chrome/Chromium installed on the local machine"]
    async fn cdp_session_rejects_disallowed_new_tab_from_coordinate_click_action() {
        let profile = BrowserProfile {
            block_ip_addresses: true,
            browser_start_timeout_ms: 30_000,
            ..BrowserProfile::default()
        };
        let session = CdpBrowserSession::launch(&profile)
            .await
            .expect("launch CDP session");

        session
            .navigate(
                "data:text/html,<html><head><title>blocked click</title></head><body style='margin:0'><button id='blocked' onclick=\"window.open('http://127.0.0.1:1/popup')\" style='position:absolute;left:20px;top:20px;width:180px;height:44px'>Blocked popup</button></body></html>",
                false,
            )
            .await
            .expect("navigate allowed data page");
        sleep(Duration::from_millis(100)).await;

        let error = session
            .click_coordinates(40, 40)
            .await
            .expect_err("coordinate click should enforce blocked popup policy");

        assert!(
            matches!(
                error,
                BrowserError::NavigationBlocked { ref url, ref reason }
                    if url.starts_with("http://127.0.0.1:1/popup")
                        && reason == "ip_address_blocked"
            ),
            "unexpected blocked popup policy error: {error:?}"
        );
    }

    #[tokio::test]
    #[ignore = "requires Chrome/Chromium installed on the local machine"]
    async fn cdp_session_scrolls_indexed_scrollable_element() {
        let profile = BrowserProfile::default();
        let session = CdpBrowserSession::launch(&profile)
            .await
            .expect("launch CDP session");

        session
            .navigate(
                "data:text/html,<html><head><title>scrollable smoke</title></head><body><button style='display:none'>Hidden</button><div id='pane' tabindex='0' style='height:60px;width:200px;overflow:auto;border:1px solid black'><div style='height:400px'>Top<br><button>Deep button</button></div></div></body></html>",
                false,
            )
            .await
            .expect("navigate");
        sleep(Duration::from_millis(150)).await;

        let state = session.state(false).await.expect("state");
        assert!(!state.dom_state.llm_representation().contains("Hidden"));
        let pane = state
            .dom_state
            .selector_map
            .get(&1)
            .expect("scrollable pane");
        assert!(
            pane.is_scrollable,
            "pane was not marked scrollable: {pane:?}"
        );

        session
            .scroll(Some(1), true, 1.0)
            .await
            .expect("scroll pane");
        let scroll_top = session
            .evaluate_json("document.getElementById('pane').scrollTop")
            .await
            .expect("scrollTop");
        assert!(scroll_top.as_f64().unwrap_or_default() > 0.0);
    }

    #[tokio::test]
    #[ignore = "requires Chrome/Chromium installed on the local machine"]
    async fn cdp_session_indexes_plain_scroll_container_without_tabindex() {
        let profile = BrowserProfile::default();
        let session = CdpBrowserSession::launch(&profile)
            .await
            .expect("launch CDP session");

        session
            .navigate(
                "data:text/html,<html><head><title>plain scroll container</title></head><body><div id='plain-pane' style='height:60px;width:200px;overflow:auto;border:1px solid black'><div style='height:400px'>Plain scroll content</div></div><div id='button-pane' style='height:60px;width:200px;overflow:auto;border:1px solid black'><button id='inner-button'>Inner button</button><div style='height:400px'></div></div></body></html>",
                false,
            )
            .await
            .expect("navigate");
        sleep(Duration::from_millis(150)).await;

        let state = session.state(false).await.expect("state");
        let plain_pane = state
            .dom_state
            .selector_map
            .values()
            .find(|element| {
                element
                    .attributes
                    .get("id")
                    .is_some_and(|value| value == "plain-pane")
            })
            .expect("plain scroll pane indexed");
        assert!(
            plain_pane.is_scrollable,
            "plain pane was not marked scrollable: {plain_pane:?}"
        );
        assert!(
            plain_pane
                .attributes
                .get("scroll")
                .is_some_and(|value| value.contains("pages below")),
            "plain pane was missing scroll context: {plain_pane:?}"
        );
        assert!(
            state.dom_state.llm_representation().contains("pages below"),
            "DOM state did not render scroll context: {}",
            state.dom_state.llm_representation()
        );
        assert!(state.dom_state.selector_map.values().any(|element| {
            element
                .attributes
                .get("id")
                .is_some_and(|id| id == "inner-button")
        }));
        assert!(state.dom_state.selector_map.values().all(|element| {
            element
                .attributes
                .get("id")
                .is_none_or(|id| id != "button-pane")
        }));

        session
            .scroll(Some(plain_pane.index), true, 1.0)
            .await
            .expect("scroll plain pane");
        let scroll_top = session
            .evaluate_json("document.getElementById('plain-pane').scrollTop")
            .await
            .expect("scrollTop");
        assert!(scroll_top.as_f64().unwrap_or_default() > 0.0);
    }

    #[tokio::test]
    #[ignore = "requires Chrome/Chromium installed on the local machine"]
    async fn cdp_session_skips_non_content_dom_tags() {
        let profile = BrowserProfile::default();
        let session = CdpBrowserSession::launch(&profile)
            .await
            .expect("launch CDP session");

        session
            .navigate(
                "data:text/html,<html><head><title>Hidden title copy</title><meta name='hidden' content='Hidden meta copy'><link rel='stylesheet' href='data:text/css,button{}'><style>Hidden style copy</style><script>window.__hiddenScriptCopy='Hidden script copy';</script></head><body><button id='visible' onclick=\"document.body.dataset.clicked='true'\">Visible</button></body></html>",
                false,
            )
            .await
            .expect("navigate");
        sleep(Duration::from_millis(100)).await;

        let state = session.state(false).await.expect("state");
        assert_eq!(state.dom_state.element_count(), 1);
        assert_eq!(state.dom_state.page_stats.total_elements, 3);
        assert_eq!(state.dom_state.page_stats.text_chars, 7);
        assert_eq!(
            state
                .dom_state
                .selector_map
                .values()
                .next()
                .and_then(|element| element.attributes.get("id"))
                .map(String::as_str),
            Some("visible")
        );
        for hidden_text in [
            "Hidden title copy",
            "Hidden style copy",
            "Hidden script copy",
        ] {
            assert!(
                !state.dom_state.llm_representation().contains(hidden_text),
                "non-content text leaked into DOM state: {}",
                state.dom_state.llm_representation()
            );
        }

        session.click(1).await.expect("click visible button");
        let clicked = session
            .evaluate_json("document.body.dataset.clicked")
            .await
            .expect("clicked flag");
        assert_eq!(clicked.as_str(), Some("true"));
    }

    #[tokio::test]
    #[ignore = "requires Chrome/Chromium installed on the local machine"]
    async fn cdp_session_prunes_contained_action_descendants() {
        let profile = BrowserProfile::default();
        let session = CdpBrowserSession::launch(&profile)
            .await
            .expect("launch CDP session");

        session
            .navigate(
                "data:text/html,<html><head><title>contained descendants</title></head><body><button id='outer-button' onclick=\"document.body.dataset.clicked='outer'\" style='width:160px;height:44px'><span id='button-icon' class='icon' style='display:inline-block;width:20px;height:20px'>x</span>Open</button><a id='outer-link' href='https://example.com/docs' style='display:inline-block;width:160px;height:44px'><span id='link-icon' class='icon' style='display:inline-block;width:20px;height:20px'>x</span>Docs</a><button id='labelled-outer' style='width:160px;height:44px'><span id='labelled-child' aria-label='Inner dismiss' style='display:inline-block;width:20px;height:20px'>x</span></button></body></html>",
                false,
            )
            .await
            .expect("navigate");
        sleep(Duration::from_millis(100)).await;

        let state = session.state(false).await.expect("state");
        let ids = state
            .dom_state
            .selector_map
            .values()
            .filter_map(|element| element.attributes.get("id").map(String::as_str))
            .collect::<Vec<_>>();

        for expected in [
            "outer-button",
            "outer-link",
            "labelled-outer",
            "labelled-child",
        ] {
            assert!(
                ids.contains(&expected),
                "DOM state missing {expected}: {}",
                state.dom_state.llm_representation()
            );
        }
        for pruned in ["button-icon", "link-icon"] {
            assert!(
                !ids.contains(&pruned),
                "contained generic descendant should be pruned: {pruned}; ids={ids:?}"
            );
        }

        let outer_index = state
            .dom_state
            .selector_map
            .values()
            .find(|element| {
                element
                    .attributes
                    .get("id")
                    .is_some_and(|id| id == "outer-button")
            })
            .map(|element| element.index)
            .expect("outer button index");
        session
            .click(outer_index)
            .await
            .expect("click outer button by index");
        let clicked = session
            .evaluate_json("document.body.dataset.clicked")
            .await
            .expect("clicked flag");
        assert_eq!(clicked.as_str(), Some("outer"));
    }

    #[tokio::test]
    #[ignore = "requires Chrome/Chromium installed on the local machine"]
    async fn cdp_session_can_navigate_read_state_and_capture_screenshot() {
        let profile = BrowserProfile::default();
        let session = CdpBrowserSession::launch(&profile)
            .await
            .expect("launch CDP session");
        let upload_dir = tempfile::tempdir().expect("upload temp dir");
        let upload_path = upload_dir.path().join("sample-upload.txt");
        std::fs::write(&upload_path, "EvalOps upload smoke").expect("write upload file");

        session
            .navigate(
                "data:text/html,<html><head><title>browser-use-rs smoke</title></head><body><button onclick=\"document.title='clicked'\">Click me</button><input placeholder='Name'><input type='file' onchange=\"document.body.dataset.uploaded=this.files[0]?.name || ''\"><div style='height:2000px'>Scroll target</div></body></html>",
                false,
            )
            .await
            .expect("navigate");
        sleep(Duration::from_millis(100)).await;

        let initial_state = session.state(false).await.expect("initial state");
        assert_eq!(initial_state.dom_state.element_count(), 3);
        assert!(initial_state.dom_state.page_stats.total_elements >= 5);
        assert_eq!(initial_state.dom_state.page_stats.interactive_elements, 3);
        assert!(initial_state.dom_state.page_stats.text_chars > 0);
        assert!(
            initial_state
                .dom_state
                .llm_representation()
                .contains("Click me")
        );
        let eval = initial_state.dom_state.eval_representation();
        assert!(
            eval.contains("<html"),
            "eval tree missed document root: {eval}"
        );
        assert!(
            eval.contains("[i_") && eval.contains("Click me"),
            "eval tree missed backend-indexed button: {eval}"
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
        session
            .upload_file(3, &upload_path)
            .await
            .expect("upload file");
        let uploaded_name = session
            .evaluate_json("document.body.dataset.uploaded || ''")
            .await
            .expect("uploaded file name");
        assert_eq!(uploaded_name.as_str(), Some("sample-upload.txt"));
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

        let original_target_id = state.tabs.first().expect("original tab").target_id.clone();
        session
            .navigate(
                "data:text/html,<html><head><title>browser-use-rs tab smoke</title></head><body>Second tab</body></html>",
                true,
            )
            .await
            .expect("navigate new tab");
        sleep(Duration::from_millis(100)).await;

        let tab_state = session.state(false).await.expect("new tab state");
        assert_eq!(tab_state.title, "browser-use-rs tab smoke");
        assert!(tab_state.tabs.len() >= 2);
        let new_target_id = tab_state
            .tabs
            .iter()
            .find(|tab| tab.title == "browser-use-rs tab smoke")
            .expect("new tab target")
            .target_id
            .clone();

        session
            .switch_tab(&original_target_id)
            .await
            .expect("switch original tab");
        sleep(Duration::from_millis(100)).await;
        let switched_state = session.state(false).await.expect("switched state");
        assert_eq!(switched_state.title, "clicked");

        session
            .switch_tab(&new_target_id)
            .await
            .expect("switch new tab");
        session
            .close_tab(&new_target_id)
            .await
            .expect("close new tab");
        sleep(Duration::from_millis(100)).await;

        let after_close = session.state(false).await.expect("state after close");
        assert_eq!(after_close.title, "clicked");
        assert!(
            after_close
                .tabs
                .iter()
                .all(|tab| tab.target_id != new_target_id)
        );
    }
}
