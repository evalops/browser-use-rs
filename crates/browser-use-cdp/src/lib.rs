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
use std::collections::{BTreeMap, VecDeque};
use std::path::{Path, PathBuf};
#[cfg(test)]
use std::process::Stdio;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

#[cfg(test)]
use base64::Engine;
use browser_use_dom::{DomElementRef, PageInfo, SerializedDomState};
#[cfg(test)]
use browser_use_dom::{DomEvalNode, DomPageStats, ElementBounds, PaginationButtonType, TabInfo};
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

mod browser_session;
mod browser_session_impl;
mod cloud;
mod dom;
mod dom_session;
mod input;
mod lifecycle;
mod policy;
mod profile;
mod recording;
mod runtime;
mod session;
mod session_helpers;
mod session_types;
mod storage;
mod target;
mod transport;
mod types;
mod watchdog;

pub use browser_session::BrowserSession;
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
pub(crate) use session_types::{
    AttachedFramePage, CachedDomElementRef, DomHighlightConfig, FrameElementInfo, FrameOffset,
    IframeTargetInfo, IframeTraversalConfig, InteractionHighlightConfig, NetworkActivityState,
    PageLoadWaitConfig, SessionDownloads,
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

impl CdpBrowserSession {
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

#[cfg(test)]
mod tests;
