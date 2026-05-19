use crate::dom::is_missing_target_error;
use crate::storage::page_tabs;
use crate::{BrowserError, BrowserLifecycleEvent, BrowserProfile, BrowserViewport, CdpConnection};
use browser_use_dom::TabInfo;
use serde_json::{Value, json};
use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttachedPage {
    pub target_id: String,
    pub session_id: String,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct ViewportEmulationConfig {
    pub(crate) viewport: Option<BrowserViewport>,
    pub(crate) device_scale_factor: f64,
}

impl ViewportEmulationConfig {
    pub(crate) fn from_profile(profile: &BrowserProfile) -> Self {
        let viewport = (!profile.no_viewport).then_some(profile.viewport);
        Self {
            viewport,
            device_scale_factor: profile.device_scale_factor.unwrap_or(1.0),
        }
    }
}

pub(crate) async fn attach_or_create_page(
    connection: &CdpConnection,
) -> Result<AttachedPage, BrowserError> {
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
            Err(error) if is_missing_target_error(&error) => continue,
            Err(error) => return Err(error),
        }
    }

    let target_id = create_target(connection, "about:blank").await?;
    attach_to_target(connection, target_id).await
}

pub(crate) async fn create_target(
    connection: &CdpConnection,
    url: &str,
) -> Result<String, BrowserError> {
    connection
        .command("Target.createTarget", json!({ "url": url }), None)
        .await?
        .get("targetId")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| BrowserError::MissingResponseData("Target.createTarget targetId".to_owned()))
}

pub(crate) async fn attach_to_target(
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

pub(crate) fn viewport_emulation_params(config: ViewportEmulationConfig) -> Option<Value> {
    config.viewport.map(|viewport| {
        json!({
            "width": viewport.width,
            "height": viewport.height,
            "deviceScaleFactor": config.device_scale_factor,
            "mobile": false,
        })
    })
}

pub(crate) async fn apply_viewport_emulation_for_page(
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

pub(crate) fn browser_permission_grant_params(permissions: &[String]) -> Option<Value> {
    (!permissions.is_empty()).then(|| json!({ "permissions": permissions }))
}

pub(crate) async fn grant_browser_permissions(
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

pub(crate) async fn enable_browser_download_events(
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

pub(crate) fn resolve_page_target_id_from_tabs(
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

pub(crate) async fn resolve_page_target_id(
    connection: &CdpConnection,
    tab_id_or_target_id: &str,
) -> Result<String, BrowserError> {
    let tabs = page_tabs(connection).await?;
    resolve_page_target_id_from_tabs(&tabs, tab_id_or_target_id)
}
