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

#[cfg(test)]
mod tests;
