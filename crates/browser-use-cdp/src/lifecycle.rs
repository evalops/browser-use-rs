//! Browser lifecycle event vocabulary and subscriptions.
//!
//! CDP sessions publish structured events for browser connection, navigation,
//! downloads, storage, and diagnostics. The adapter event layer preserves a
//! simpler browser-use-compatible taxonomy for downstream consumers that do
//! not need every low-level event kind.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use thiserror::Error;
use tokio::sync::broadcast;

/// Fine-grained lifecycle event category emitted by the CDP session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum BrowserLifecycleEventKind {
    /// CDP connection was established.
    BrowserConnected,
    /// Browser close was requested.
    BrowserCloseRequested,
    /// Browser stopped or disconnected intentionally.
    BrowserStopped,
    /// Transport is attempting to reconnect.
    BrowserReconnecting,
    /// Transport reconnected successfully.
    BrowserReconnected,
    /// Non-fatal diagnostic information.
    BrowserDiagnostic,
    /// Chrome target was created.
    TargetCreated,
    /// Chrome target was closed.
    TargetClosed,
    /// Agent focus switched to another target.
    TargetSwitched,
    /// Target crashed.
    TargetCrashed,
    /// Navigation started.
    NavigationStarted,
    /// Navigation completed.
    NavigationCompleted,
    /// Navigation failed.
    NavigationFailed,
    /// Network activity exceeded the configured timeout.
    NetworkTimeout,
    /// Navigation was blocked by URL policy.
    NavigationBlocked,
    /// Current target was reset successfully.
    CurrentTargetReset,
    /// Current target reset failed.
    CurrentTargetResetFailed,
    /// Popup was closed automatically.
    PopupClosed,
    /// Popup close attempt failed.
    PopupCloseFailed,
    /// JavaScript dialog was accepted or dismissed.
    JavaScriptDialogHandled,
    /// Download started.
    DownloadStarted,
    /// Download progress update.
    DownloadProgress,
    /// Download completed and was written to disk.
    FileDownloaded,
    /// Browser storage state was saved.
    StorageStateSaved,
    /// Browser storage state was loaded.
    StorageStateLoaded,
}

/// Structured lifecycle event emitted by browser sessions.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct BrowserLifecycleEvent {
    /// Event category.
    pub kind: BrowserLifecycleEventKind,
    /// Target id associated with the event, when any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_id: Option<String>,
    /// URL associated with the event, when any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// Machine-readable reason string.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// Error text for failed events.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Additional structured string details.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub details: BTreeMap<String, String>,
    /// Human-readable event message.
    pub message: String,
}

impl BrowserLifecycleEvent {
    /// Creates a lifecycle event with all fields supplied explicitly.
    pub fn new(
        kind: BrowserLifecycleEventKind,
        target_id: Option<String>,
        url: Option<String>,
        reason: Option<String>,
        error: Option<String>,
        details: BTreeMap<String, String>,
        message: String,
    ) -> Self {
        Self {
            kind,
            target_id,
            url,
            reason,
            error,
            details,
            message,
        }
    }

    /// Event emitted after a browser connection is established.
    pub fn browser_connected(url: impl Into<String>) -> Self {
        let url = url.into();
        Self::new(
            BrowserLifecycleEventKind::BrowserConnected,
            None,
            Some(url.clone()),
            None,
            None,
            BTreeMap::new(),
            format!("Browser connected at {url}"),
        )
    }

    /// Event emitted before attempting to close the browser.
    pub fn browser_close_requested() -> Self {
        Self::new(
            BrowserLifecycleEventKind::BrowserCloseRequested,
            None,
            None,
            None,
            None,
            BTreeMap::new(),
            "Browser close requested".to_owned(),
        )
    }

    /// Event emitted when the browser stops.
    pub fn browser_stopped(reason: impl Into<String>) -> Self {
        let reason = reason.into();
        Self::new(
            BrowserLifecycleEventKind::BrowserStopped,
            None,
            None,
            Some(reason.clone()),
            None,
            BTreeMap::new(),
            format!("Browser stopped ({reason})"),
        )
    }

    /// Event emitted before a reconnect attempt.
    pub fn browser_reconnecting(url: impl Into<String>, attempt: u32, max_attempts: u32) -> Self {
        let url = url.into();
        Self::new(
            BrowserLifecycleEventKind::BrowserReconnecting,
            None,
            Some(url.clone()),
            None,
            None,
            BTreeMap::from([
                ("attempt".to_owned(), attempt.to_string()),
                ("max_attempts".to_owned(), max_attempts.to_string()),
            ]),
            format!("Browser reconnecting to {url} (attempt {attempt}/{max_attempts})"),
        )
    }

    /// Event emitted after a successful reconnect.
    pub fn browser_reconnected(
        url: impl Into<String>,
        attempt: u32,
        downtime_seconds: impl Into<String>,
    ) -> Self {
        let url = url.into();
        let downtime_seconds = downtime_seconds.into();
        Self::new(
            BrowserLifecycleEventKind::BrowserReconnected,
            None,
            Some(url.clone()),
            None,
            None,
            BTreeMap::from([
                ("attempt".to_owned(), attempt.to_string()),
                ("downtime_seconds".to_owned(), downtime_seconds.clone()),
            ]),
            format!("Browser reconnected to {url} on attempt {attempt} after {downtime_seconds}s"),
        )
    }

    /// Event emitted for non-fatal browser diagnostics.
    pub fn browser_diagnostic(
        reason: impl Into<String>,
        details: BTreeMap<String, String>,
        error: Option<String>,
        message: impl Into<String>,
    ) -> Self {
        let reason = reason.into();
        Self::new(
            BrowserLifecycleEventKind::BrowserDiagnostic,
            None,
            None,
            Some(reason),
            error,
            details,
            message.into(),
        )
    }

    /// Event emitted when permission grant via CDP fails.
    pub fn permissions_grant_failed(permissions: &[String], error: impl Into<String>) -> Self {
        let error = error.into();
        Self::browser_diagnostic(
            "permissions_grant_failed",
            BTreeMap::from([
                ("permissions".to_owned(), permissions.join(",")),
                (
                    "permissions_count".to_owned(),
                    permissions.len().to_string(),
                ),
            ]),
            Some(error.clone()),
            format!("Browser permission grant failed: {error}"),
        )
    }

    /// Event emitted when Chrome creates a target.
    pub fn target_created(target_id: impl Into<String>, url: impl Into<String>) -> Self {
        let target_id = target_id.into();
        let url = url.into();
        Self::new(
            BrowserLifecycleEventKind::TargetCreated,
            Some(target_id.clone()),
            Some(url.clone()),
            None,
            None,
            BTreeMap::new(),
            format!("Target {target_id} created for {url}"),
        )
    }

    /// Event emitted when Chrome closes a target.
    pub fn target_closed(target_id: impl Into<String>) -> Self {
        let target_id = target_id.into();
        Self::new(
            BrowserLifecycleEventKind::TargetClosed,
            Some(target_id.clone()),
            None,
            None,
            None,
            BTreeMap::new(),
            format!("Target {target_id} closed"),
        )
    }

    /// Event emitted when the agent switches focus to a target.
    pub fn target_switched(target_id: impl Into<String>) -> Self {
        let target_id = target_id.into();
        Self::new(
            BrowserLifecycleEventKind::TargetSwitched,
            Some(target_id.clone()),
            None,
            None,
            None,
            BTreeMap::new(),
            format!("Agent focus switched to target {target_id}"),
        )
    }

    /// Event emitted when a target crash is observed.
    pub fn target_crashed(target_id: impl Into<String>, error: impl Into<String>) -> Self {
        let target_id = target_id.into();
        let error = error.into();
        Self::new(
            BrowserLifecycleEventKind::TargetCrashed,
            Some(target_id.clone()),
            None,
            None,
            Some(error.clone()),
            BTreeMap::new(),
            format!("Target {target_id} crashed: {error}"),
        )
    }

    /// Event emitted when navigation begins.
    pub fn navigation_started(target_id: impl Into<String>, url: impl Into<String>) -> Self {
        let target_id = target_id.into();
        let url = url.into();
        Self::new(
            BrowserLifecycleEventKind::NavigationStarted,
            Some(target_id.clone()),
            Some(url.clone()),
            None,
            None,
            BTreeMap::new(),
            format!("Navigation started on target {target_id} to {url}"),
        )
    }

    /// Event emitted when navigation completes.
    pub fn navigation_completed(target_id: impl Into<String>, url: impl Into<String>) -> Self {
        let target_id = target_id.into();
        let url = url.into();
        Self::new(
            BrowserLifecycleEventKind::NavigationCompleted,
            Some(target_id.clone()),
            Some(url.clone()),
            None,
            None,
            BTreeMap::new(),
            format!("Navigation completed on target {target_id} to {url}"),
        )
    }

    /// Event emitted when navigation fails.
    pub fn navigation_failed(
        target_id: impl Into<String>,
        url: impl Into<String>,
        error: impl Into<String>,
    ) -> Self {
        let target_id = target_id.into();
        let url = url.into();
        let error = error.into();
        Self::new(
            BrowserLifecycleEventKind::NavigationFailed,
            Some(target_id.clone()),
            Some(url.clone()),
            None,
            Some(error.clone()),
            BTreeMap::new(),
            format!("Navigation failed on target {target_id} to {url}: {error}"),
        )
    }

    /// Event emitted when network activity does not become idle in time.
    pub fn network_timeout(
        target_id: impl Into<String>,
        url: impl Into<String>,
        timeout_seconds: impl Into<String>,
    ) -> Self {
        let target_id = target_id.into();
        let url = url.into();
        let timeout_seconds = timeout_seconds.into();
        Self::new(
            BrowserLifecycleEventKind::NetworkTimeout,
            Some(target_id.clone()),
            Some(url.clone()),
            Some("network_timeout".to_owned()),
            Some(format!("timed out after {timeout_seconds}s")),
            BTreeMap::from([("timeout_seconds".to_owned(), timeout_seconds.clone())]),
            format!("Network timeout on target {target_id} for {url} after {timeout_seconds}s"),
        )
    }

    /// Event emitted when an alert/confirm/prompt dialog is handled.
    pub fn javascript_dialog_handled(
        url: impl Into<String>,
        dialog_type: impl Into<String>,
        message: impl Into<String>,
        accepted: bool,
    ) -> Self {
        let url = url.into();
        let dialog_type = dialog_type.into();
        let message = message.into();
        let action = if accepted { "accepted" } else { "dismissed" };
        Self::new(
            BrowserLifecycleEventKind::JavaScriptDialogHandled,
            None,
            Some(url.clone()),
            Some(dialog_type.clone()),
            None,
            BTreeMap::from([
                ("dialog_type".to_owned(), dialog_type.clone()),
                ("dialog_message".to_owned(), message.clone()),
                ("action".to_owned(), action.to_owned()),
            ]),
            format!("JavaScript {dialog_type} dialog on {url} was {action}: {message}"),
        )
    }

    /// Event emitted when Chrome reports a download start.
    pub fn download_started(
        guid: impl Into<String>,
        url: impl Into<String>,
        suggested_filename: impl Into<String>,
    ) -> Self {
        let guid = guid.into();
        let url = url.into();
        let suggested_filename = suggested_filename.into();
        Self::new(
            BrowserLifecycleEventKind::DownloadStarted,
            None,
            Some(url.clone()),
            None,
            None,
            BTreeMap::from([
                ("guid".to_owned(), guid.clone()),
                ("suggested_filename".to_owned(), suggested_filename.clone()),
            ]),
            format!("Download {guid} started from {url} as {suggested_filename}"),
        )
    }

    /// Event emitted for Chrome download progress updates.
    pub fn download_progress(
        guid: impl Into<String>,
        received_bytes: u64,
        total_bytes: Option<u64>,
        state: impl Into<String>,
    ) -> Self {
        let guid = guid.into();
        let state = state.into();
        let mut details = BTreeMap::from([
            ("guid".to_owned(), guid.clone()),
            ("received_bytes".to_owned(), received_bytes.to_string()),
            ("state".to_owned(), state.clone()),
        ]);
        if let Some(total_bytes) = total_bytes {
            details.insert("total_bytes".to_owned(), total_bytes.to_string());
        }
        Self::new(
            BrowserLifecycleEventKind::DownloadProgress,
            None,
            None,
            Some(state.clone()),
            None,
            details,
            format!("Download {guid} progress: {state} ({received_bytes} bytes received)"),
        )
    }

    /// Event emitted when a download is written to disk.
    pub fn file_downloaded(
        guid: impl Into<String>,
        path: impl Into<String>,
        file_name: impl Into<String>,
        file_size: u64,
    ) -> Self {
        let guid = guid.into();
        let path = path.into();
        let file_name = file_name.into();
        Self::new(
            BrowserLifecycleEventKind::FileDownloaded,
            None,
            None,
            None,
            None,
            BTreeMap::from([
                ("guid".to_owned(), guid.clone()),
                ("path".to_owned(), path.clone()),
                ("file_name".to_owned(), file_name.clone()),
                ("file_size".to_owned(), file_size.to_string()),
            ]),
            format!("Download {guid} completed at {path} ({file_name}, {file_size} bytes)"),
        )
    }

    /// Event emitted when the PDF auto-download path writes a PDF file.
    pub fn pdf_auto_downloaded(
        url: impl Into<String>,
        path: impl Into<String>,
        file_name: impl Into<String>,
        file_size: u64,
    ) -> Self {
        let url = url.into();
        let path = path.into();
        let file_name = file_name.into();
        let guid = format!("auto-pdf:{url}");
        Self::new(
            BrowserLifecycleEventKind::FileDownloaded,
            None,
            Some(url.clone()),
            Some("pdf_auto_download".to_owned()),
            None,
            BTreeMap::from([
                ("guid".to_owned(), guid.clone()),
                ("path".to_owned(), path.clone()),
                ("file_name".to_owned(), file_name.clone()),
                ("file_size".to_owned(), file_size.to_string()),
                ("auto_download".to_owned(), "true".to_owned()),
            ]),
            format!("Auto-downloaded PDF {url} to {path} ({file_name}, {file_size} bytes)"),
        )
    }

    /// Event emitted when PDF auto-download fails.
    pub fn pdf_auto_download_failed(url: impl Into<String>, error: impl Into<String>) -> Self {
        let url = url.into();
        let error = error.into();
        Self::new(
            BrowserLifecycleEventKind::BrowserDiagnostic,
            None,
            Some(url.clone()),
            Some("pdf_auto_download_failed".to_owned()),
            Some(error.clone()),
            BTreeMap::from([("auto_download".to_owned(), "true".to_owned())]),
            format!("Failed to auto-download PDF {url}: {error}"),
        )
    }

    /// Event emitted after storage state is saved.
    pub fn storage_state_saved(
        path: impl Into<String>,
        cookies_count: usize,
        origins_count: usize,
    ) -> Self {
        let path = path.into();
        Self::new(
            BrowserLifecycleEventKind::StorageStateSaved,
            None,
            None,
            Some("storage_state".to_owned()),
            None,
            BTreeMap::from([
                ("path".to_owned(), path.clone()),
                ("cookies_count".to_owned(), cookies_count.to_string()),
                ("origins_count".to_owned(), origins_count.to_string()),
            ]),
            format!(
                "Storage state saved to {path} ({cookies_count} cookies, {origins_count} origins)"
            ),
        )
    }

    /// Event emitted after storage state is loaded.
    pub fn storage_state_loaded(
        path: impl Into<String>,
        cookies_count: usize,
        origins_count: usize,
    ) -> Self {
        let path = path.into();
        Self::new(
            BrowserLifecycleEventKind::StorageStateLoaded,
            None,
            None,
            Some("storage_state".to_owned()),
            None,
            BTreeMap::from([
                ("path".to_owned(), path.clone()),
                ("cookies_count".to_owned(), cookies_count.to_string()),
                ("origins_count".to_owned(), origins_count.to_string()),
            ]),
            format!(
                "Storage state loaded from {path} ({cookies_count} cookies, {origins_count} origins)"
            ),
        )
    }
}

/// Coarser lifecycle category compatible with browser-use adapter events.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum BrowserLifecycleAdapterEventKind {
    /// Browser stop was requested.
    BrowserStop,
    /// Browser connected.
    BrowserConnected,
    /// Browser stopped.
    BrowserStopped,
    /// Browser reconnect attempt started.
    BrowserReconnecting,
    /// Browser reconnected.
    BrowserReconnected,
    /// Tab was created.
    TabCreated,
    /// Tab was closed.
    TabClosed,
    /// Agent focus changed tabs/targets.
    AgentFocusChanged,
    /// Target crashed.
    TargetCrashed,
    /// Navigation started.
    NavigationStarted,
    /// Navigation completed.
    NavigationComplete,
    /// Browser-level error.
    BrowserError,
    /// JavaScript dialog was handled.
    JavaScriptDialogHandled,
    /// Download started.
    DownloadStarted,
    /// Download progress changed.
    DownloadProgress,
    /// File downloaded.
    FileDownloaded,
    /// Storage state changed.
    StorageState,
    /// Browser diagnostic event.
    BrowserDiagnostic,
}

/// Adapter event derived from a [`BrowserLifecycleEvent`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct BrowserLifecycleAdapterEvent {
    /// Adapter event category.
    pub kind: BrowserLifecycleAdapterEventKind,
    /// Original fine-grained lifecycle kind.
    pub source_kind: BrowserLifecycleEventKind,
    /// Target id associated with the source event, when any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_id: Option<String>,
    /// URL associated with the source event, when any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// Machine-readable reason string.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// Error text for failed events.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Additional structured string details.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub details: BTreeMap<String, String>,
    /// Human-readable event message.
    pub message: String,
}

impl BrowserLifecycleAdapterEvent {
    /// Converts a fine-grained lifecycle event to the adapter taxonomy.
    pub fn from_lifecycle_event(event: &BrowserLifecycleEvent) -> Self {
        let kind = match &event.kind {
            BrowserLifecycleEventKind::BrowserConnected => {
                BrowserLifecycleAdapterEventKind::BrowserConnected
            }
            BrowserLifecycleEventKind::BrowserCloseRequested => {
                BrowserLifecycleAdapterEventKind::BrowserStop
            }
            BrowserLifecycleEventKind::BrowserStopped => {
                BrowserLifecycleAdapterEventKind::BrowserStopped
            }
            BrowserLifecycleEventKind::BrowserReconnecting => {
                BrowserLifecycleAdapterEventKind::BrowserReconnecting
            }
            BrowserLifecycleEventKind::BrowserReconnected => {
                BrowserLifecycleAdapterEventKind::BrowserReconnected
            }
            BrowserLifecycleEventKind::BrowserDiagnostic => {
                BrowserLifecycleAdapterEventKind::BrowserDiagnostic
            }
            BrowserLifecycleEventKind::TargetCreated => {
                BrowserLifecycleAdapterEventKind::TabCreated
            }
            BrowserLifecycleEventKind::TargetClosed => BrowserLifecycleAdapterEventKind::TabClosed,
            BrowserLifecycleEventKind::TargetSwitched => {
                BrowserLifecycleAdapterEventKind::AgentFocusChanged
            }
            BrowserLifecycleEventKind::TargetCrashed => {
                BrowserLifecycleAdapterEventKind::TargetCrashed
            }
            BrowserLifecycleEventKind::NavigationStarted => {
                BrowserLifecycleAdapterEventKind::NavigationStarted
            }
            BrowserLifecycleEventKind::NavigationCompleted => {
                BrowserLifecycleAdapterEventKind::NavigationComplete
            }
            BrowserLifecycleEventKind::NavigationFailed
            | BrowserLifecycleEventKind::NetworkTimeout
            | BrowserLifecycleEventKind::NavigationBlocked
            | BrowserLifecycleEventKind::CurrentTargetResetFailed
            | BrowserLifecycleEventKind::PopupCloseFailed => {
                BrowserLifecycleAdapterEventKind::BrowserError
            }
            BrowserLifecycleEventKind::CurrentTargetReset
            | BrowserLifecycleEventKind::PopupClosed => {
                BrowserLifecycleAdapterEventKind::BrowserDiagnostic
            }
            BrowserLifecycleEventKind::JavaScriptDialogHandled => {
                BrowserLifecycleAdapterEventKind::JavaScriptDialogHandled
            }
            BrowserLifecycleEventKind::DownloadStarted => {
                BrowserLifecycleAdapterEventKind::DownloadStarted
            }
            BrowserLifecycleEventKind::DownloadProgress => {
                BrowserLifecycleAdapterEventKind::DownloadProgress
            }
            BrowserLifecycleEventKind::FileDownloaded => {
                BrowserLifecycleAdapterEventKind::FileDownloaded
            }
            BrowserLifecycleEventKind::StorageStateSaved
            | BrowserLifecycleEventKind::StorageStateLoaded => {
                BrowserLifecycleAdapterEventKind::StorageState
            }
        };

        Self {
            kind,
            source_kind: event.kind.clone(),
            target_id: event.target_id.clone(),
            url: event.url.clone(),
            reason: event.reason.clone(),
            error: event.error.clone(),
            details: event.details.clone(),
            message: event.message.clone(),
        }
    }
}

/// Converts a slice of lifecycle events to adapter events.
pub fn browser_lifecycle_adapter_events(
    events: &[BrowserLifecycleEvent],
) -> Vec<BrowserLifecycleAdapterEvent> {
    events
        .iter()
        .map(BrowserLifecycleAdapterEvent::from_lifecycle_event)
        .collect()
}

/// Error returned by lifecycle event subscriptions.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum BrowserLifecycleEventStreamError {
    /// Sender side closed.
    #[error("lifecycle event stream closed")]
    Closed,
    /// Receiver lagged behind and missed events.
    #[error("lifecycle event stream lagged by {0} events")]
    Lagged(u64),
}

/// Subscription for fine-grained lifecycle events.
#[derive(Debug)]
pub struct BrowserLifecycleEventSubscription {
    receiver: broadcast::Receiver<BrowserLifecycleEvent>,
}

impl BrowserLifecycleEventSubscription {
    pub(crate) fn new(receiver: broadcast::Receiver<BrowserLifecycleEvent>) -> Self {
        Self { receiver }
    }

    /// Creates a subscription that is already closed.
    pub fn closed() -> Self {
        let (sender, receiver) = broadcast::channel(1);
        drop(sender);
        Self::new(receiver)
    }

    /// Waits for the next lifecycle event.
    pub async fn recv(
        &mut self,
    ) -> Result<BrowserLifecycleEvent, BrowserLifecycleEventStreamError> {
        match self.receiver.recv().await {
            Ok(event) => Ok(event),
            Err(broadcast::error::RecvError::Closed) => {
                Err(BrowserLifecycleEventStreamError::Closed)
            }
            Err(broadcast::error::RecvError::Lagged(count)) => {
                Err(BrowserLifecycleEventStreamError::Lagged(count))
            }
        }
    }

    /// Attempts to receive one lifecycle event without waiting.
    pub fn try_recv(
        &mut self,
    ) -> Result<Option<BrowserLifecycleEvent>, BrowserLifecycleEventStreamError> {
        match self.receiver.try_recv() {
            Ok(event) => Ok(Some(event)),
            Err(broadcast::error::TryRecvError::Empty) => Ok(None),
            Err(broadcast::error::TryRecvError::Closed) => {
                Err(BrowserLifecycleEventStreamError::Closed)
            }
            Err(broadcast::error::TryRecvError::Lagged(count)) => {
                Err(BrowserLifecycleEventStreamError::Lagged(count))
            }
        }
    }

    /// Creates another receiver on the same broadcast stream.
    pub fn resubscribe(&self) -> Self {
        Self::new(self.receiver.resubscribe())
    }
}

/// Subscription that maps lifecycle events to adapter events as they arrive.
#[derive(Debug)]
pub struct BrowserLifecycleAdapterEventSubscription {
    subscription: BrowserLifecycleEventSubscription,
}

impl BrowserLifecycleAdapterEventSubscription {
    /// Wraps a fine-grained lifecycle subscription.
    pub fn new(subscription: BrowserLifecycleEventSubscription) -> Self {
        Self { subscription }
    }

    /// Creates an adapter subscription that is already closed.
    pub fn closed() -> Self {
        Self::new(BrowserLifecycleEventSubscription::closed())
    }

    /// Waits for the next adapter event.
    pub async fn recv(
        &mut self,
    ) -> Result<BrowserLifecycleAdapterEvent, BrowserLifecycleEventStreamError> {
        self.subscription
            .recv()
            .await
            .map(|event| BrowserLifecycleAdapterEvent::from_lifecycle_event(&event))
    }

    /// Attempts to receive one adapter event without waiting.
    pub fn try_recv(
        &mut self,
    ) -> Result<Option<BrowserLifecycleAdapterEvent>, BrowserLifecycleEventStreamError> {
        self.subscription.try_recv().map(|event| {
            event.map(|event| BrowserLifecycleAdapterEvent::from_lifecycle_event(&event))
        })
    }

    /// Creates another adapter receiver on the same broadcast stream.
    pub fn resubscribe(&self) -> Self {
        Self::new(self.subscription.resubscribe())
    }
}
