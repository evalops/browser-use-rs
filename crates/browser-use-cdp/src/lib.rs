//! Chrome DevTools Protocol browser-session layer.
//!
//! This crate turns Chrome DevTools Protocol commands into the
//! provider-neutral [`BrowserSession`] trait used by the core agent. The public
//! entry point is [`CdpBrowserSession`], while the internal modules split CDP
//! transport, DOM capture, input synthesis, launch profile handling, lifecycle
//! events, recordings, storage state, URL policy, and watchdog behavior.
//!
//! ```mermaid
//! flowchart TD
//!     Profile["BrowserProfile"] --> Launch["launch or connect"]
//!     Launch --> Conn["CdpConnection"]
//!     Conn --> Page["AttachedPage"]
//!     Page --> State["BrowserSession::state"]
//!     State --> Dom["interactive DOM + accessibility join"]
//!     State --> Metrics["page info, tabs, events, screenshot"]
//!     Dom --> Summary["BrowserStateSummary"]
//!     Metrics --> Summary
//!     Summary --> Core["browser-use-core Agent"]
//!     Core --> Action["BrowserAction"]
//!     Action --> Session["BrowserSession action methods"]
//!     Session --> Conn
//! ```

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
use serde_json::{Value, json};
#[cfg(test)]
use std::sync::atomic::{AtomicBool, AtomicU64};
use tempfile::TempDir;
#[cfg(test)]
use tokio::sync::mpsc;
use tokio::sync::{Mutex, broadcast};
use tokio::time::sleep;
#[cfg(test)]
use tokio_tungstenite::tungstenite::Message;

mod browser_session_impl;
mod cloud;
mod dom;
mod input;
mod lifecycle;
mod policy;
mod profile;
mod recording;
mod runtime;
mod storage;
mod target;
mod transport;
mod types;
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
pub use target::AttachedPage;
pub(crate) use target::{
    ViewportEmulationConfig, apply_viewport_emulation_for_page, attach_or_create_page,
    attach_to_target, create_target, enable_browser_download_events, grant_browser_permissions,
    resolve_page_target_id,
};
#[cfg(test)]
pub(crate) use target::{
    browser_permission_grant_params, resolve_page_target_id_from_tabs, viewport_emulation_params,
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
pub use types::{
    BrowserError, BrowserViewport, CloudProxyCountryCode, FoundElement, Pdf, ProxySettings,
    Screenshot,
};
pub(crate) use types::{
    deserialize_cloud_proxy_country_code, deserialize_env_map, deserialize_non_negative_f64,
    deserialize_non_negative_f64_option, serialize_cloud_proxy_country_code,
};

const URL_POLICY_SETTLE_MS: u64 = 200;

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

/// Browser session backed by a Chrome DevTools Protocol connection.
///
/// The session owns shared, async-safe state: the active page target, latest DOM
/// snapshot, navigation policy, lifecycle event streams, recording hooks,
/// download state, and watchdogs. Methods on [`BrowserSession`] translate
/// high-level browser-use actions into CDP commands against this state.
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
    /// Connects to an existing DevTools endpoint with the default profile.
    pub async fn connect(endpoint: DevToolsEndpoint) -> Result<Self, BrowserError> {
        Self::connect_with_profile(endpoint, &BrowserProfile::default()).await
    }

    /// Connects to an existing DevTools endpoint using profile-specific options.
    ///
    /// This path does not launch or own the browser process. It still applies
    /// permissions, download behavior, viewport emulation, recording hooks, and
    /// lifecycle watchdogs where those settings make sense for an attached
    /// browser.
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

    /// Launches or creates a browser described by `profile` and connects to it.
    ///
    /// Local profiles spawn Chrome and keep a process handle unless
    /// `keep_alive` asks to detach. Cloud profiles create a Browser Use Cloud
    /// session and connect to its returned CDP websocket.
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

    /// Closes the browser after flushing configured storage, HAR, video, and trace artifacts.
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

    /// Saves cookies and origin storage to a Playwright/browser-use compatible JSON file.
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

    /// Loads cookies and origin storage from a storage-state JSON file.
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

    /// Returns a snapshot of fine-grained lifecycle events recorded so far.
    pub async fn lifecycle_events(&self) -> Vec<BrowserLifecycleEvent> {
        self.lifecycle_events.lock().await.iter().cloned().collect()
    }

    /// Returns recorded lifecycle events converted to the adapter taxonomy.
    pub async fn lifecycle_adapter_events(&self) -> Vec<BrowserLifecycleAdapterEvent> {
        browser_lifecycle_adapter_events(&self.lifecycle_events().await)
    }

    /// Subscribes to future fine-grained lifecycle events.
    pub fn subscribe_lifecycle_events(&self) -> BrowserLifecycleEventSubscription {
        BrowserLifecycleEventSubscription::new(self.lifecycle_event_tx.subscribe())
    }

    /// Subscribes to future lifecycle events converted to adapter events.
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
                    Err(error) if is_missing_target_error(&error) => {
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
/// Provider-neutral browser-control interface used by the core executor.
///
/// The trait lets tests and future browser backends satisfy the same contract
/// as [`CdpBrowserSession`]. Each method is intentionally action-shaped: the
/// core executor should not need to know whether a backend uses CDP,
/// Playwright, WebDriver, or a mock session.
pub trait BrowserSession: Send + Sync {
    /// Subscribes to fine-grained lifecycle events, or returns a closed stream by default.
    fn subscribe_lifecycle_events(&self) -> BrowserLifecycleEventSubscription {
        BrowserLifecycleEventSubscription::closed()
    }

    /// Subscribes to adapter lifecycle events derived from the fine-grained stream.
    fn subscribe_lifecycle_adapter_events(&self) -> BrowserLifecycleAdapterEventSubscription {
        BrowserLifecycleAdapterEventSubscription::new(self.subscribe_lifecycle_events())
    }

    /// Captures current browser state for the agent.
    async fn state(&self, include_screenshot: bool) -> Result<BrowserStateSummary, BrowserError>;

    /// Navigates to a URL in the current tab or a new tab.
    async fn navigate(&self, url: &str, new_tab: bool) -> Result<(), BrowserError>;

    /// Goes back in the active tab history.
    async fn go_back(&self) -> Result<(), BrowserError>;

    /// Switches focus to a tab by short tab id or full target id.
    async fn switch_tab(&self, target_id: &str) -> Result<(), BrowserError>;

    /// Closes a tab by short tab id or full target id.
    async fn close_tab(&self, target_id: &str) -> Result<(), BrowserError>;

    /// Clicks an indexed DOM element.
    async fn click(&self, index: u32) -> Result<(), BrowserError>;

    /// Clicks explicit viewport coordinates.
    async fn click_coordinates(&self, x: i32, y: i32) -> Result<(), BrowserError>;

    /// Inputs text into an indexed element.
    async fn input_text(&self, index: u32, text: &str, clear: bool) -> Result<(), BrowserError>;

    /// Scrolls the page or an indexed scrollable element.
    async fn scroll(&self, index: Option<u32>, down: bool, pages: f64) -> Result<(), BrowserError>;

    /// Finds text on the active page.
    async fn find_text(&self, text: &str) -> Result<bool, BrowserError>;

    /// Evaluates JavaScript on the active page and returns serialized output.
    async fn evaluate(&self, code: &str) -> Result<String, BrowserError>;

    /// Returns options for an indexed dropdown.
    async fn dropdown_options(&self, index: u32) -> Result<Vec<String>, BrowserError>;

    /// Selects an option in an indexed dropdown by visible text.
    async fn select_dropdown_option(&self, index: u32, text: &str) -> Result<(), BrowserError>;

    /// Returns visible page text used by search/extract actions.
    async fn page_text(&self) -> Result<String, BrowserError>;

    /// Finds elements by CSS selector with optional attributes and text.
    async fn find_elements(
        &self,
        selector: &str,
        attributes: &[String],
        max_results: usize,
        include_text: bool,
    ) -> Result<Vec<FoundElement>, BrowserError>;

    /// Sends keyboard input to the active page.
    async fn send_keys(&self, keys: &str) -> Result<(), BrowserError>;

    /// Uploads a file through an indexed file input element.
    async fn upload_file(&self, index: u32, path: &Path) -> Result<(), BrowserError>;

    /// Captures a PNG screenshot.
    async fn screenshot(&self) -> Result<Screenshot, BrowserError>;

    /// Prints the active page to PDF.
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
mod tests;
