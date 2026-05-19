//! Background CDP watchdog tasks.
//!
//! Watchdogs subscribe to raw CDP events and turn them into durable lifecycle,
//! security, network, popup, download, and reconnect state. Keeping them in one
//! module makes the session facade read like orchestration instead of event
//! plumbing.

use crate::{
    AttachedPage, BrowserError, BrowserLifecycleEvent, BrowserLifecycleEventKind, CdpConnection,
    CdpEvent, CdpHarRecorder, CdpVideoRecorder, NetworkActivityState, UrlAccessPolicy,
};
use base64::Engine;
use browser_use_dom::SerializedDomState;
use percent_encoding::percent_decode_str;
use serde_json::{Value, json};
use std::collections::{BTreeMap, HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{Mutex, broadcast};

pub(crate) const MAX_SECURITY_EVENTS: usize = 8;
pub(crate) const MAX_LIFECYCLE_EVENTS: usize = 32;

pub(crate) struct BrowserLifecycleWatchdog {
    pub(crate) handle: tokio::task::JoinHandle<()>,
}

pub(crate) struct BrowserLifecycleWatchdogRecorders {
    pub(crate) cdp_auto_pdf_download: Option<Arc<CdpAutoPdfDownloadState>>,
    pub(crate) har_recorder: Option<Arc<CdpHarRecorder>>,
    pub(crate) video_recorder: Option<Arc<CdpVideoRecorder>>,
}

impl BrowserLifecycleWatchdog {
    pub(crate) fn start(
        connection: Arc<CdpConnection>,
        lifecycle_events: Arc<Mutex<VecDeque<BrowserLifecycleEvent>>>,
        lifecycle_event_tx: broadcast::Sender<BrowserLifecycleEvent>,
        network_request_timeout_ms: u64,
        network_activity: Arc<Mutex<NetworkActivityState>>,
        recorders: BrowserLifecycleWatchdogRecorders,
    ) -> Self {
        let mut events = connection.subscribe_events();
        let lifecycle_event_sink = LifecycleEventSink {
            events: lifecycle_events,
            event_tx: lifecycle_event_tx,
        };
        let handle = tokio::spawn(async move {
            let mut active_network_requests = HashMap::new();
            let mut interval = tokio::time::interval(Duration::from_millis(1_000));
            let network_request_timeout = (network_request_timeout_ms > 0)
                .then(|| Duration::from_millis(network_request_timeout_ms));

            loop {
                tokio::select! {
                    event = events.recv() => {
                        match event {
                            Ok(event) => {
                                handle_lifecycle_cdp_event(
                                    &connection,
                                    &lifecycle_event_sink,
                                    &mut active_network_requests,
                                    &network_activity,
                                    &recorders,
                                    event,
                                )
                                .await;
                            }
                            Err(broadcast::error::RecvError::Lagged(_)) => continue,
                            Err(broadcast::error::RecvError::Closed) => break,
                        }
                    }
                    _ = interval.tick(), if network_request_timeout.is_some() => {
                        let timeout = network_request_timeout.expect("guarded by is_some");
                        let events = lifecycle_events_for_timed_out_network_requests(
                            &mut active_network_requests,
                            Instant::now(),
                            timeout,
                        );
                        record_lifecycle_events(
                            &lifecycle_event_sink.events,
                            &lifecycle_event_sink.event_tx,
                            events,
                        )
                        .await;
                    }
                }
            }
        });

        Self { handle }
    }
}

impl Drop for BrowserLifecycleWatchdog {
    fn drop(&mut self) {
        self.handle.abort();
    }
}

pub(crate) struct ActiveNetworkRequest {
    pub(crate) request_id: String,
    pub(crate) url: String,
    pub(crate) method: String,
    pub(crate) resource_type: Option<String>,
    pub(crate) session_id: Option<String>,
    pub(crate) started_at: Instant,
}

pub(crate) async fn handle_lifecycle_cdp_event(
    connection: &CdpConnection,
    lifecycle_event_sink: &LifecycleEventSink,
    active_network_requests: &mut HashMap<String, ActiveNetworkRequest>,
    network_activity: &Arc<Mutex<NetworkActivityState>>,
    recorders: &BrowserLifecycleWatchdogRecorders,
    event: CdpEvent,
) {
    if let Some(har_recorder) = &recorders.har_recorder {
        har_recorder.observe_cdp_event(connection, &event).await;
    }
    if let Some(video_recorder) = &recorders.video_recorder {
        video_recorder.observe_cdp_event(connection, &event).await;
    }

    match event.method.as_str() {
        "Network.requestWillBeSent" => {
            track_network_request(active_network_requests, &event);
            track_network_activity_started(network_activity, &event).await;
        }
        "Network.loadingFinished" | "Network.loadingFailed" => {
            forget_network_request(active_network_requests, &event);
            track_network_activity_finished(network_activity, &event).await;
            if event.method == "Network.loadingFinished" {
                if let Some(event) = cdp_auto_pdf_lifecycle_event(
                    connection,
                    &recorders.cdp_auto_pdf_download,
                    &event,
                )
                .await
                {
                    lifecycle_event_sink.push(event).await;
                }
            } else if let Some(cdp_auto_pdf_download) = &recorders.cdp_auto_pdf_download {
                cdp_auto_pdf_download.forget_candidate(&event).await;
            }
        }
        "Network.responseReceived" => {
            if let Some(cdp_auto_pdf_download) = &recorders.cdp_auto_pdf_download {
                cdp_auto_pdf_download.observe_response(&event).await;
            }
        }
        "browser-use-rs.websocket-closed" => {
            lifecycle_event_sink
                .push(lifecycle_event_for_websocket_closed(&event))
                .await;
        }
        "browser-use-rs.websocket-reconnecting" => {
            if let Some(event) = lifecycle_event_for_websocket_reconnecting(&event) {
                lifecycle_event_sink.push(event).await;
            }
        }
        "browser-use-rs.websocket-reconnected" => {
            if let Some(event) = lifecycle_event_for_websocket_reconnected(&event) {
                lifecycle_event_sink.push(event).await;
            }
        }
        "browser-use-rs.websocket-reconnect-failed" => {
            lifecycle_event_sink
                .push(lifecycle_event_for_websocket_reconnect_failed(&event))
                .await;
        }
        "Target.targetCrashed" | "Inspector.targetCrashed" => {
            record_lifecycle_events(
                &lifecycle_event_sink.events,
                &lifecycle_event_sink.event_tx,
                lifecycle_events_for_target_crash(&event),
            )
            .await;
        }
        "Page.javascriptDialogOpening" => {
            let event = lifecycle_event_for_javascript_dialog(connection, &event).await;
            lifecycle_event_sink.push(event).await;
        }
        "Browser.downloadWillBegin" => {
            if let Some(event) = lifecycle_event_for_download_start(&event) {
                lifecycle_event_sink.push(event).await;
            }
        }
        "Browser.downloadProgress" => {
            record_lifecycle_events(
                &lifecycle_event_sink.events,
                &lifecycle_event_sink.event_tx,
                lifecycle_events_for_download_progress(&event),
            )
            .await;
        }
        _ => {}
    }
}

pub(crate) fn track_network_request(
    active_network_requests: &mut HashMap<String, ActiveNetworkRequest>,
    event: &CdpEvent,
) {
    let Some(request_id) = event.params.get("requestId").and_then(Value::as_str) else {
        return;
    };
    let Some(request) = event.params.get("request") else {
        return;
    };
    let url = request
        .get("url")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if !url.starts_with("http://") && !url.starts_with("https://") {
        return;
    }
    active_network_requests.insert(
        request_id.to_owned(),
        ActiveNetworkRequest {
            request_id: request_id.to_owned(),
            url: url.to_owned(),
            method: request
                .get("method")
                .and_then(Value::as_str)
                .unwrap_or("GET")
                .to_owned(),
            resource_type: event
                .params
                .get("type")
                .and_then(Value::as_str)
                .map(str::to_owned),
            session_id: event.session_id.clone(),
            started_at: Instant::now(),
        },
    );
}

#[derive(Debug, Clone)]
pub(crate) struct CdpAutoPdfCandidate {
    pub(crate) request_id: String,
    pub(crate) request_key: String,
    pub(crate) session_id: Option<String>,
    pub(crate) url: String,
    pub(crate) file_name: String,
}

#[derive(Debug)]
pub(crate) struct CdpAutoPdfDownloadState {
    pub(crate) downloads_path: PathBuf,
    pub(crate) downloaded_urls: Arc<Mutex<BTreeMap<String, PathBuf>>>,
    pub(crate) candidates: Mutex<BTreeMap<String, CdpAutoPdfCandidate>>,
}

impl CdpAutoPdfDownloadState {
    pub(crate) fn from_downloads(
        auto_download_pdfs: bool,
        downloads_path: Option<&Path>,
        downloaded_urls: Arc<Mutex<BTreeMap<String, PathBuf>>>,
    ) -> Option<Arc<Self>> {
        if !auto_download_pdfs {
            return None;
        }
        downloads_path.map(|downloads_path| {
            Arc::new(Self {
                downloads_path: downloads_path.to_path_buf(),
                downloaded_urls,
                candidates: Mutex::new(BTreeMap::new()),
            })
        })
    }

    pub(crate) async fn observe_response(&self, event: &CdpEvent) {
        let Some(candidate) = cdp_auto_pdf_candidate_from_response(event) else {
            return;
        };
        self.candidates
            .lock()
            .await
            .insert(candidate.request_key.clone(), candidate);
    }

    pub(crate) async fn forget_candidate(&self, event: &CdpEvent) {
        let Some(request_key) = cdp_request_key(event) else {
            return;
        };
        self.candidates.lock().await.remove(&request_key);
    }

    pub(crate) async fn take_finished_candidate(
        &self,
        event: &CdpEvent,
    ) -> Option<CdpAutoPdfCandidate> {
        let request_key = cdp_request_key(event)?;
        let candidate = self.candidates.lock().await.remove(&request_key)?;
        let cached_path = self
            .downloaded_urls
            .lock()
            .await
            .get(&candidate.url)
            .cloned();
        if let Some(path) = cached_path {
            if tokio::fs::metadata(&path).await.is_ok() {
                return None;
            }
            let mut downloaded_urls = self.downloaded_urls.lock().await;
            if downloaded_urls.get(&candidate.url) == Some(&path) {
                downloaded_urls.remove(&candidate.url);
            }
        }
        Some(candidate)
    }

    pub(crate) async fn write_candidate(
        &self,
        candidate: &CdpAutoPdfCandidate,
        bytes: &[u8],
    ) -> Result<BrowserLifecycleEvent, BrowserError> {
        tokio::fs::create_dir_all(&self.downloads_path)
            .await
            .map_err(|error| BrowserError::StateUnavailable(error.to_string()))?;
        let path = unique_download_path(&self.downloads_path, &candidate.file_name).await?;
        tokio::fs::write(&path, bytes)
            .await
            .map_err(|error| BrowserError::StateUnavailable(error.to_string()))?;
        self.downloaded_urls
            .lock()
            .await
            .insert(candidate.url.clone(), path.clone());
        let file_name = path
            .file_name()
            .and_then(|name| name.to_str())
            .map(str::to_owned)
            .unwrap_or_else(|| candidate.file_name.clone());
        Ok(BrowserLifecycleEvent::pdf_auto_downloaded(
            &candidate.url,
            path.display().to_string(),
            file_name,
            u64::try_from(bytes.len()).unwrap_or(u64::MAX),
        ))
    }
}

pub(crate) async fn cdp_auto_pdf_lifecycle_event(
    connection: &CdpConnection,
    cdp_auto_pdf_download: &Option<Arc<CdpAutoPdfDownloadState>>,
    event: &CdpEvent,
) -> Option<BrowserLifecycleEvent> {
    let cdp_auto_pdf_download = cdp_auto_pdf_download.as_ref()?;
    let candidate = cdp_auto_pdf_download.take_finished_candidate(event).await?;
    match cdp_response_body_bytes(connection, &candidate).await {
        Ok(bytes) => match cdp_auto_pdf_download
            .write_candidate(&candidate, &bytes)
            .await
        {
            Ok(event) => Some(event),
            Err(error) => Some(BrowserLifecycleEvent::pdf_auto_download_failed(
                candidate.url,
                error.to_string(),
            )),
        },
        Err(error) => Some(BrowserLifecycleEvent::pdf_auto_download_failed(
            candidate.url,
            error.to_string(),
        )),
    }
}

pub(crate) async fn cdp_response_body_bytes(
    connection: &CdpConnection,
    candidate: &CdpAutoPdfCandidate,
) -> Result<Vec<u8>, BrowserError> {
    let response = connection
        .command(
            "Network.getResponseBody",
            json!({ "requestId": candidate.request_id }),
            candidate.session_id.as_deref(),
        )
        .await?;
    let body = response
        .get("body")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            BrowserError::MissingResponseData("Network.getResponseBody.body".to_owned())
        })?;
    let base64_encoded = response
        .get("base64Encoded")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    if base64_encoded {
        base64::engine::general_purpose::STANDARD
            .decode(body)
            .map_err(|error| BrowserError::StateUnavailable(error.to_string()))
    } else {
        Ok(body.as_bytes().to_vec())
    }
}

pub(crate) fn cdp_auto_pdf_candidate_from_response(
    event: &CdpEvent,
) -> Option<CdpAutoPdfCandidate> {
    let request_id = event.params.get("requestId").and_then(Value::as_str)?;
    let response = event.params.get("response")?;
    let url = response.get("url").and_then(Value::as_str)?.to_owned();
    let mime_type = response
        .get("mimeType")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let headers = response.get("headers");
    let content_type = headers.and_then(|headers| cdp_header_value(headers, "content-type"));
    if !is_application_pdf(mime_type) && !content_type.as_deref().is_some_and(is_application_pdf) {
        return None;
    }
    let content_disposition =
        headers.and_then(|headers| cdp_header_value(headers, "content-disposition"));
    let file_name = content_disposition
        .as_deref()
        .and_then(content_disposition_filename)
        .unwrap_or_else(|| pdf_download_filename_from_url(&url));
    Some(CdpAutoPdfCandidate {
        request_id: request_id.to_owned(),
        request_key: cdp_request_key(event)?,
        session_id: event.session_id.clone(),
        url,
        file_name,
    })
}

pub(crate) fn cdp_request_key(event: &CdpEvent) -> Option<String> {
    let request_id = event.params.get("requestId").and_then(Value::as_str)?;
    Some(match event.session_id.as_deref() {
        Some(session_id) => format!("{session_id}:{request_id}"),
        None => format!("root:{request_id}"),
    })
}

pub(crate) fn cdp_header_value(headers: &Value, name: &str) -> Option<String> {
    let object = headers.as_object()?;
    object.iter().find_map(|(header_name, value)| {
        header_name.eq_ignore_ascii_case(name).then(|| match value {
            Value::String(value) => value.clone(),
            other => other.to_string(),
        })
    })
}

pub(crate) fn is_application_pdf(value: &str) -> bool {
    value
        .split(';')
        .any(|part| part.trim().eq_ignore_ascii_case("application/pdf"))
}

pub(crate) fn content_disposition_filename(value: &str) -> Option<String> {
    for part in value.split(';') {
        let Some((name, value)) = part.trim().split_once('=') else {
            continue;
        };
        let name = name.trim().to_ascii_lowercase();
        if name == "filename*" {
            let value = value.trim().trim_matches('"');
            let encoded = value
                .rsplit_once("''")
                .map_or(value, |(_, encoded)| encoded);
            let decoded = percent_decode_str(encoded).decode_utf8_lossy();
            return Some(ensure_pdf_extension(sanitize_download_filename(&decoded)));
        }
        if name == "filename" {
            return Some(ensure_pdf_extension(sanitize_download_filename(
                value.trim().trim_matches('"'),
            )));
        }
    }
    None
}

pub(crate) async fn track_network_activity_started(
    network_activity: &Arc<Mutex<NetworkActivityState>>,
    event: &CdpEvent,
) {
    let Some(request_id) = event.params.get("requestId").and_then(Value::as_str) else {
        return;
    };
    network_activity
        .lock()
        .await
        .observe_request_started(request_id, Instant::now());
}

pub(crate) fn forget_network_request(
    active_network_requests: &mut HashMap<String, ActiveNetworkRequest>,
    event: &CdpEvent,
) {
    if let Some(request_id) = event.params.get("requestId").and_then(Value::as_str) {
        active_network_requests.remove(request_id);
    }
}

pub(crate) async fn track_network_activity_finished(
    network_activity: &Arc<Mutex<NetworkActivityState>>,
    event: &CdpEvent,
) {
    let Some(request_id) = event.params.get("requestId").and_then(Value::as_str) else {
        return;
    };
    network_activity
        .lock()
        .await
        .observe_request_finished(request_id, Instant::now());
}

pub(crate) fn lifecycle_events_for_timed_out_network_requests(
    active_network_requests: &mut HashMap<String, ActiveNetworkRequest>,
    now: Instant,
    timeout: Duration,
) -> Vec<BrowserLifecycleEvent> {
    let request_ids = active_network_requests
        .iter()
        .filter(|(_, request)| now.duration_since(request.started_at) >= timeout)
        .map(|(request_id, _)| request_id.clone())
        .collect::<Vec<_>>();

    request_ids
        .into_iter()
        .filter_map(|request_id| active_network_requests.remove(&request_id))
        .map(|request| lifecycle_event_for_network_request_timeout(request, timeout))
        .collect()
}

pub(crate) fn lifecycle_event_for_network_request_timeout(
    request: ActiveNetworkRequest,
    timeout: Duration,
) -> BrowserLifecycleEvent {
    let timeout_seconds = format!("{:.3}", timeout.as_secs_f64());
    let mut details = BTreeMap::from([
        ("request_id".to_owned(), request.request_id.clone()),
        ("method".to_owned(), request.method.clone()),
        ("timeout_seconds".to_owned(), timeout_seconds.clone()),
    ]);
    if let Some(resource_type) = &request.resource_type {
        details.insert("resource_type".to_owned(), resource_type.clone());
    }
    if let Some(session_id) = &request.session_id {
        details.insert("session_id".to_owned(), session_id.clone());
    }
    BrowserLifecycleEvent::new(
        BrowserLifecycleEventKind::NetworkTimeout,
        None,
        Some(request.url.clone()),
        Some("network_request_timeout".to_owned()),
        Some(format!("request timed out after {timeout_seconds}s")),
        details,
        format!(
            "Network request {} {} timed out after {timeout_seconds}s",
            request.method, request.url
        ),
    )
}

pub(crate) fn lifecycle_event_for_websocket_closed(event: &CdpEvent) -> BrowserLifecycleEvent {
    let reason = event
        .params
        .get("reason")
        .and_then(Value::as_str)
        .unwrap_or("websocket_closed");
    let error = event
        .params
        .get("error")
        .and_then(Value::as_str)
        .map(str::to_owned);
    let mut details = BTreeMap::from([("reason".to_owned(), reason.to_owned())]);
    if let Some(error) = &error {
        details.insert("error".to_owned(), error.clone());
    }
    let message = match &error {
        Some(error) => format!("CDP websocket closed ({reason}): {error}"),
        None => format!("CDP websocket closed ({reason})"),
    };
    BrowserLifecycleEvent::new(
        BrowserLifecycleEventKind::BrowserStopped,
        None,
        None,
        Some(reason.to_owned()),
        error,
        details,
        message,
    )
}

pub(crate) fn lifecycle_event_for_websocket_reconnecting(
    event: &CdpEvent,
) -> Option<BrowserLifecycleEvent> {
    let cdp_url = event.params.get("cdp_url")?.as_str()?;
    let attempt = event.params.get("attempt")?.as_u64()? as u32;
    let max_attempts = event.params.get("max_attempts")?.as_u64()? as u32;
    Some(BrowserLifecycleEvent::browser_reconnecting(
        cdp_url,
        attempt,
        max_attempts,
    ))
}

pub(crate) fn lifecycle_event_for_websocket_reconnected(
    event: &CdpEvent,
) -> Option<BrowserLifecycleEvent> {
    let cdp_url = event.params.get("cdp_url")?.as_str()?;
    let attempt = event.params.get("attempt")?.as_u64()? as u32;
    let downtime_seconds = event.params.get("downtime_seconds")?.as_str()?;
    let mut lifecycle_event =
        BrowserLifecycleEvent::browser_reconnected(cdp_url, attempt, downtime_seconds);
    if let Some(generation) = event
        .params
        .get("connection_generation")
        .and_then(Value::as_u64)
    {
        lifecycle_event
            .details
            .insert("connection_generation".to_owned(), generation.to_string());
    }
    Some(lifecycle_event)
}

pub(crate) fn lifecycle_event_for_websocket_reconnect_failed(
    event: &CdpEvent,
) -> BrowserLifecycleEvent {
    let cdp_url = event
        .params
        .get("cdp_url")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let max_attempts = event
        .params
        .get("max_attempts")
        .and_then(Value::as_u64)
        .unwrap_or_default();
    let downtime_seconds = event
        .params
        .get("downtime_seconds")
        .and_then(Value::as_str)
        .unwrap_or("0.000");
    let error = event
        .params
        .get("error")
        .and_then(Value::as_str)
        .map(str::to_owned);
    let mut details = BTreeMap::from([
        ("cdp_url".to_owned(), cdp_url.to_owned()),
        ("max_attempts".to_owned(), max_attempts.to_string()),
        ("downtime_seconds".to_owned(), downtime_seconds.to_owned()),
    ]);
    if let Some(error) = &error {
        details.insert("error".to_owned(), error.clone());
    }
    BrowserLifecycleEvent::new(
        BrowserLifecycleEventKind::BrowserStopped,
        None,
        Some(cdp_url.to_owned()),
        Some("reconnect_failed".to_owned()),
        error,
        details,
        format!("CDP websocket failed to reconnect after {max_attempts} attempts"),
    )
}

pub(crate) async fn record_lifecycle_events(
    lifecycle_events: &Arc<Mutex<VecDeque<BrowserLifecycleEvent>>>,
    lifecycle_event_tx: &broadcast::Sender<BrowserLifecycleEvent>,
    events: Vec<BrowserLifecycleEvent>,
) {
    if events.is_empty() {
        return;
    }

    let mut queue = lifecycle_events.lock().await;
    for event in events {
        push_lifecycle_event_and_publish(&mut queue, lifecycle_event_tx, event);
    }
}

pub(crate) fn lifecycle_events_for_target_crash(event: &CdpEvent) -> Vec<BrowserLifecycleEvent> {
    let target_id = event
        .params
        .get("targetId")
        .and_then(Value::as_str)
        .map(str::to_owned);
    let mut details = BTreeMap::new();
    if let Some(session_id) = &event.session_id {
        details.insert("session_id".to_owned(), session_id.clone());
    }
    if let Some(status) = event.params.get("status").and_then(cdp_value_to_string) {
        details.insert("status".to_owned(), status);
    }
    if let Some(error_code) = event.params.get("errorCode").and_then(cdp_value_to_string) {
        details.insert("error_code".to_owned(), error_code);
    }

    let error = target_crash_error_message(&details);
    let lifecycle_event = match target_id {
        Some(target_id) => {
            let mut event = BrowserLifecycleEvent::target_crashed(target_id, error);
            event.details = details;
            event
        }
        None => BrowserLifecycleEvent::new(
            BrowserLifecycleEventKind::TargetCrashed,
            None,
            None,
            None,
            Some(error.clone()),
            details,
            format!("Target crashed: {error}"),
        ),
    };

    vec![lifecycle_event]
}

pub(crate) fn target_crash_error_message(details: &BTreeMap<String, String>) -> String {
    match (details.get("status"), details.get("error_code")) {
        (Some(status), Some(error_code)) => format!("{status} ({error_code})"),
        (Some(status), None) => status.clone(),
        (None, Some(error_code)) => error_code.clone(),
        (None, None) => "Inspector target crashed".to_owned(),
    }
}

pub(crate) async fn lifecycle_event_for_javascript_dialog(
    connection: &CdpConnection,
    event: &CdpEvent,
) -> BrowserLifecycleEvent {
    let dialog_type = event
        .params
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or("alert");
    let dialog_message = event
        .params
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let url = event
        .params
        .get("url")
        .and_then(Value::as_str)
        .unwrap_or("about:blank");
    let accepted = matches!(dialog_type, "alert" | "confirm" | "beforeunload");
    let action = if accepted { "accepted" } else { "dismissed" };
    let mut details = BTreeMap::from([
        ("dialog_type".to_owned(), dialog_type.to_owned()),
        ("dialog_message".to_owned(), dialog_message.to_owned()),
        ("action".to_owned(), action.to_owned()),
    ]);
    if let Some(frame_id) = event.params.get("frameId").and_then(Value::as_str) {
        details.insert("frame_id".to_owned(), frame_id.to_owned());
    }
    if let Some(session_id) = &event.session_id {
        details.insert("session_id".to_owned(), session_id.clone());
    }

    let error = match event.session_id.as_deref() {
        Some(session_id) => connection
            .command(
                "Page.handleJavaScriptDialog",
                json!({ "accept": accepted }),
                Some(session_id),
            )
            .await
            .err()
            .map(|error| error.to_string()),
        None => Some("missing CDP session id".to_owned()),
    };

    let message = match &error {
        Some(error) => {
            format!(
                "JavaScript {dialog_type} dialog on {url} failed to be {action}: {dialog_message}: {error}"
            )
        }
        None => format!("JavaScript {dialog_type} dialog on {url} was {action}: {dialog_message}"),
    };

    BrowserLifecycleEvent::new(
        BrowserLifecycleEventKind::JavaScriptDialogHandled,
        None,
        Some(url.to_owned()),
        Some(dialog_type.to_owned()),
        error,
        details,
        message,
    )
}

pub(crate) fn lifecycle_event_for_download_start(
    event: &CdpEvent,
) -> Option<BrowserLifecycleEvent> {
    let guid = event.params.get("guid")?.as_str()?;
    let url = event
        .params
        .get("url")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let suggested_filename = event
        .params
        .get("suggestedFilename")
        .and_then(Value::as_str)
        .map(sanitize_download_filename)
        .unwrap_or_else(|| "download".to_owned());
    Some(BrowserLifecycleEvent::download_started(
        guid,
        url,
        suggested_filename,
    ))
}

pub(crate) fn lifecycle_events_for_download_progress(
    event: &CdpEvent,
) -> Vec<BrowserLifecycleEvent> {
    let Some(guid) = event.params.get("guid").and_then(Value::as_str) else {
        return Vec::new();
    };
    let received_bytes = event
        .params
        .get("receivedBytes")
        .and_then(cdp_value_to_u64)
        .unwrap_or_default();
    let total_bytes = event.params.get("totalBytes").and_then(cdp_value_to_u64);
    let state = event
        .params
        .get("state")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    let mut events = vec![BrowserLifecycleEvent::download_progress(
        guid,
        received_bytes,
        total_bytes,
        state,
    )];

    if state == "completed" {
        if let Some(file_path) = event.params.get("filePath").and_then(Value::as_str) {
            let file_name = Path::new(file_path)
                .file_name()
                .and_then(|name| name.to_str())
                .map(sanitize_download_filename)
                .unwrap_or_else(|| "download".to_owned());
            events.push(BrowserLifecycleEvent::file_downloaded(
                guid,
                file_path,
                file_name,
                total_bytes.unwrap_or(received_bytes),
            ));
        }
    }

    events
}

pub(crate) fn sanitize_download_filename(name: &str) -> String {
    let cleaned = name.replace('\0', "").replace('\\', "/");
    let basename = cleaned
        .rsplit('/')
        .find(|part| !part.is_empty())
        .unwrap_or("");
    if matches!(basename, "" | "." | "..") {
        "download".to_owned()
    } else {
        basename.to_owned()
    }
}

pub(crate) fn is_pdf_viewer_url(url: &str) -> bool {
    let path = url::Url::parse(url)
        .map(|parsed| parsed.path().to_owned())
        .unwrap_or_else(|_| url.split(['?', '#']).next().unwrap_or_default().to_owned());
    let path = path.to_ascii_lowercase();
    path.ends_with(".pdf") || path.contains("/pdf/")
}

pub(crate) fn pdf_download_filename_from_url(url: &str) -> String {
    let decoded_name = url::Url::parse(url)
        .ok()
        .and_then(|parsed| {
            parsed
                .path_segments()
                .and_then(|mut segments| segments.rfind(|segment| !segment.is_empty()))
                .map(|segment| percent_decode_str(segment).decode_utf8_lossy().to_string())
        })
        .unwrap_or_else(|| "download.pdf".to_owned());
    let file_name = sanitize_download_filename(&decoded_name);
    ensure_pdf_extension(file_name)
}

pub(crate) fn ensure_pdf_extension(file_name: String) -> String {
    if file_name.to_ascii_lowercase().ends_with(".pdf") {
        file_name
    } else {
        format!("{file_name}.pdf")
    }
}

pub(crate) async fn unique_download_path(
    downloads_path: &Path,
    file_name: &str,
) -> Result<PathBuf, BrowserError> {
    let file_name = sanitize_download_filename(file_name);
    let path = downloads_path.join(&file_name);
    if tokio::fs::metadata(&path).await.is_err() {
        return Ok(path);
    }

    let extension = Path::new(&file_name)
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_owned);
    let stem = Path::new(&file_name)
        .file_stem()
        .and_then(|stem| stem.to_str())
        .filter(|stem| !stem.is_empty())
        .unwrap_or("download");

    for suffix in 1_u32.. {
        let candidate_name = match &extension {
            Some(extension) if !extension.is_empty() => format!("{stem}-{suffix}.{extension}"),
            _ => format!("{stem}-{suffix}"),
        };
        let candidate = downloads_path.join(candidate_name);
        if tokio::fs::metadata(&candidate).await.is_err() {
            return Ok(candidate);
        }
    }

    unreachable!("unbounded suffix search should always return")
}

#[cfg(test)]
pub(crate) fn is_path_contained(path: &Path, directory: &Path) -> bool {
    let Ok(directory) = normalize_existing_or_lexical_path(directory) else {
        return false;
    };
    let Ok(path) = normalize_existing_or_lexical_path(path) else {
        return false;
    };
    path == directory || path.starts_with(&directory)
}

#[cfg(test)]
pub(crate) fn normalize_existing_or_lexical_path(path: &Path) -> Result<PathBuf, std::io::Error> {
    match std::fs::canonicalize(path) {
        Ok(path) => Ok(path),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            Ok(normalize_lexical_path(path))
        }
        Err(error) => Err(error),
    }
}

#[cfg(test)]
pub(crate) fn normalize_lexical_path(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                normalized.pop();
            }
            other => normalized.push(other.as_os_str()),
        }
    }
    normalized
}

pub(crate) fn cdp_value_to_string(value: &Value) -> Option<String> {
    match value {
        Value::String(value) => Some(value.clone()),
        Value::Number(value) => Some(value.to_string()),
        Value::Bool(value) => Some(value.to_string()),
        _ => None,
    }
}

pub(crate) fn cdp_value_to_u64(value: &Value) -> Option<u64> {
    value.as_u64().or_else(|| {
        value
            .as_f64()
            .filter(|value| *value >= 0.0)
            .map(|value| value as u64)
    })
}

pub(crate) struct BrowserSecurityWatchdog {
    pub(crate) handle: tokio::task::JoinHandle<()>,
}

#[derive(Clone)]
pub(crate) struct LifecycleEventSink {
    pub(crate) events: Arc<Mutex<VecDeque<BrowserLifecycleEvent>>>,
    pub(crate) event_tx: broadcast::Sender<BrowserLifecycleEvent>,
}

impl LifecycleEventSink {
    pub(crate) async fn push(&self, event: BrowserLifecycleEvent) {
        let mut events = self.events.lock().await;
        push_lifecycle_event_and_publish(&mut events, &self.event_tx, event);
    }
}

impl BrowserSecurityWatchdog {
    pub(crate) async fn start(
        connection: Arc<CdpConnection>,
        page: Arc<Mutex<AttachedPage>>,
        last_dom_state: Arc<Mutex<Option<SerializedDomState>>>,
        pending_url_policy_error: Arc<Mutex<Option<BrowserError>>>,
        security_events: Arc<Mutex<VecDeque<BrowserSecurityEvent>>>,
        lifecycle_event_sink: LifecycleEventSink,
        url_policy: UrlAccessPolicy,
    ) -> Result<Option<Self>, BrowserError> {
        if url_policy.is_unrestricted() {
            return Ok(None);
        }

        let mut events = connection.subscribe_events();
        connection
            .command(
                "Target.setDiscoverTargets",
                json!({ "discover": true }),
                None,
            )
            .await?;

        let handle = tokio::spawn(async move {
            while let Ok(event) = events.recv().await {
                let current_page = page.lock().await.clone();
                let Some(action) =
                    url_policy_watchdog_action_for_event(&url_policy, &current_page, &event)
                else {
                    continue;
                };
                apply_url_policy_watchdog_action(
                    &connection,
                    &last_dom_state,
                    &pending_url_policy_error,
                    &security_events,
                    &lifecycle_event_sink,
                    action,
                )
                .await;
            }
        });

        Ok(Some(Self { handle }))
    }
}

impl Drop for BrowserSecurityWatchdog {
    fn drop(&mut self) {
        self.handle.abort();
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum UrlPolicyWatchdogAction {
    ResetCurrent {
        session_id: String,
        url: String,
        reason: String,
    },
    CloseTarget {
        target_id: String,
        url: String,
        reason: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BrowserSecurityEvent {
    pub(crate) message: String,
    pub(crate) browser_error_message: Option<String>,
    pub(crate) closed_popup_message: Option<String>,
    pub(crate) lifecycle_event: BrowserLifecycleEvent,
}

impl BrowserSecurityEvent {
    pub(crate) fn prevented_navigation(url: String, reason: String) -> Self {
        let message =
            format!("Blocked navigation to {url} ({reason}); no browser navigation was started");
        Self {
            lifecycle_event: BrowserLifecycleEvent::new(
                BrowserLifecycleEventKind::NavigationBlocked,
                None,
                Some(url),
                Some(reason),
                None,
                BTreeMap::new(),
                message.clone(),
            ),
            message,
            browser_error_message: None,
            closed_popup_message: None,
        }
    }

    pub(crate) fn reset_current(url: String, reason: String) -> Self {
        let message =
            format!("Blocked navigation to {url} ({reason}); reset current tab to about:blank");
        Self {
            lifecycle_event: BrowserLifecycleEvent::new(
                BrowserLifecycleEventKind::CurrentTargetReset,
                None,
                Some(url),
                Some(reason),
                None,
                BTreeMap::new(),
                message.clone(),
            ),
            message,
            browser_error_message: None,
            closed_popup_message: None,
        }
    }

    pub(crate) fn reset_current_failed(url: String, reason: String, error: String) -> Self {
        let message = format!(
            "Failed to reset blocked navigation to {url} ({reason}) to about:blank: {error}"
        );
        Self {
            lifecycle_event: BrowserLifecycleEvent::new(
                BrowserLifecycleEventKind::CurrentTargetResetFailed,
                None,
                Some(url),
                Some(reason),
                Some(error),
                BTreeMap::new(),
                message.clone(),
            ),
            browser_error_message: Some(message.clone()),
            message,
            closed_popup_message: None,
        }
    }

    pub(crate) fn closed_popup(url: String, reason: String) -> Self {
        let message = format!("Closed popup {url} ({reason})");
        Self {
            lifecycle_event: BrowserLifecycleEvent::new(
                BrowserLifecycleEventKind::PopupClosed,
                None,
                Some(url),
                Some(reason),
                None,
                BTreeMap::new(),
                message.clone(),
            ),
            browser_error_message: None,
            closed_popup_message: Some(message.clone()),
            message,
        }
    }

    pub(crate) fn close_popup_failed(url: String, reason: String, error: String) -> Self {
        let message = format!("Failed to close popup {url} ({reason}): {error}");
        Self {
            lifecycle_event: BrowserLifecycleEvent::new(
                BrowserLifecycleEventKind::PopupCloseFailed,
                None,
                Some(url),
                Some(reason),
                Some(error),
                BTreeMap::new(),
                message.clone(),
            ),
            browser_error_message: Some(message.clone()),
            closed_popup_message: None,
            message,
        }
    }

    pub(crate) fn from_watchdog_action(action: &UrlPolicyWatchdogAction) -> Self {
        match action {
            UrlPolicyWatchdogAction::ResetCurrent { url, reason, .. } => {
                Self::reset_current(url.clone(), reason.clone())
            }
            UrlPolicyWatchdogAction::CloseTarget { url, reason, .. } => {
                Self::closed_popup(url.clone(), reason.clone())
            }
        }
    }
}

pub(crate) fn url_policy_watchdog_action_for_event(
    policy: &UrlAccessPolicy,
    current_page: &AttachedPage,
    event: &CdpEvent,
) -> Option<UrlPolicyWatchdogAction> {
    match event.method.as_str() {
        "Target.targetCreated" | "Target.targetInfoChanged" => {
            let target_info = event.params.get("targetInfo")?;
            url_policy_watchdog_action_for_target_info(policy, current_page, target_info)
        }
        "Page.frameNavigated" => {
            let session_id = event.session_id.as_deref()?;
            if session_id != current_page.session_id {
                return None;
            }
            let url = event.params.get("frame")?.get("url")?.as_str()?;
            if url.is_empty() {
                return None;
            }
            if policy.is_allowed(url) {
                return None;
            }
            Some(UrlPolicyWatchdogAction::ResetCurrent {
                session_id: current_page.session_id.clone(),
                url: url.to_owned(),
                reason: policy.block_reason(url).to_owned(),
            })
        }
        _ => None,
    }
}

pub(crate) fn url_policy_watchdog_action_for_target_info(
    policy: &UrlAccessPolicy,
    current_page: &AttachedPage,
    target_info: &Value,
) -> Option<UrlPolicyWatchdogAction> {
    if target_info.get("type").and_then(Value::as_str) != Some("page") {
        return None;
    }

    let url = target_info.get("url")?.as_str()?;
    if url.is_empty() {
        return None;
    }
    if policy.is_allowed(url) {
        return None;
    }

    let target_id = target_info.get("targetId")?.as_str()?;
    let reason = policy.block_reason(url).to_owned();
    if target_id == current_page.target_id {
        Some(UrlPolicyWatchdogAction::ResetCurrent {
            session_id: current_page.session_id.clone(),
            url: url.to_owned(),
            reason,
        })
    } else {
        Some(UrlPolicyWatchdogAction::CloseTarget {
            target_id: target_id.to_owned(),
            url: url.to_owned(),
            reason,
        })
    }
}

pub(crate) async fn apply_url_policy_watchdog_action(
    connection: &CdpConnection,
    last_dom_state: &Arc<Mutex<Option<SerializedDomState>>>,
    pending_url_policy_error: &Arc<Mutex<Option<BrowserError>>>,
    security_events: &Arc<Mutex<VecDeque<BrowserSecurityEvent>>>,
    lifecycle_event_sink: &LifecycleEventSink,
    action: UrlPolicyWatchdogAction,
) {
    let event = BrowserSecurityEvent::from_watchdog_action(&action);
    let (url, reason, outcome) = match &action {
        UrlPolicyWatchdogAction::ResetCurrent {
            session_id,
            url,
            reason,
        } => (
            url.clone(),
            reason.clone(),
            connection
                .command(
                    "Page.navigate",
                    json!({ "url": "about:blank" }),
                    Some(session_id),
                )
                .await,
        ),
        UrlPolicyWatchdogAction::CloseTarget {
            target_id,
            url,
            reason,
        } => (
            url.clone(),
            reason.clone(),
            connection
                .command("Target.closeTarget", json!({ "targetId": target_id }), None)
                .await,
        ),
    };

    if let Err(error) = outcome {
        let failure_event = match &action {
            UrlPolicyWatchdogAction::ResetCurrent { .. } => {
                BrowserSecurityEvent::reset_current_failed(url, reason, error.to_string())
            }
            UrlPolicyWatchdogAction::CloseTarget { .. } => {
                BrowserSecurityEvent::close_popup_failed(url, reason, error.to_string())
            }
        };
        let lifecycle_event = failure_event.lifecycle_event.clone();
        let mut events = security_events.lock().await;
        push_security_event(&mut events, failure_event);
        drop(events);
        lifecycle_event_sink.push(lifecycle_event).await;
        return;
    }

    *last_dom_state.lock().await = None;
    {
        let lifecycle_event = event.lifecycle_event.clone();
        let mut events = security_events.lock().await;
        push_security_event(&mut events, event);
        drop(events);
        lifecycle_event_sink.push(lifecycle_event).await;
    }
    let mut pending = pending_url_policy_error.lock().await;
    if pending.is_none() {
        *pending = Some(BrowserError::NavigationBlocked { url, reason });
    }
}

pub(crate) fn push_security_event(
    events: &mut VecDeque<BrowserSecurityEvent>,
    event: BrowserSecurityEvent,
) {
    while events.len() >= MAX_SECURITY_EVENTS {
        events.pop_front();
    }
    events.push_back(event);
}

pub(crate) fn push_lifecycle_event(
    events: &mut VecDeque<BrowserLifecycleEvent>,
    event: BrowserLifecycleEvent,
) {
    while events.len() >= MAX_LIFECYCLE_EVENTS {
        events.pop_front();
    }
    events.push_back(event);
}

pub(crate) fn push_lifecycle_event_and_publish(
    events: &mut VecDeque<BrowserLifecycleEvent>,
    lifecycle_event_tx: &broadcast::Sender<BrowserLifecycleEvent>,
    event: BrowserLifecycleEvent,
) {
    push_lifecycle_event(events, event.clone());
    let _ = lifecycle_event_tx.send(event);
}

pub(crate) fn security_event_state_fields(
    events: &VecDeque<BrowserSecurityEvent>,
) -> (Option<String>, Vec<String>, Vec<String>) {
    let recent_events = (!events.is_empty()).then(|| {
        events
            .iter()
            .map(|event| event.message.as_str())
            .collect::<Vec<_>>()
            .join("\n")
    });
    let closed_popup_messages = events
        .iter()
        .filter_map(|event| event.closed_popup_message.clone())
        .collect();
    let browser_errors = events
        .iter()
        .filter_map(|event| event.browser_error_message.clone())
        .collect();
    (recent_events, closed_popup_messages, browser_errors)
}
