use crate::{
    AttachedPage, BrowserError, CdpConnection, runtime_evaluate_params, runtime_evaluate_value,
};
use browser_use_dom::TabInfo;
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

pub(crate) const ORIGIN_STORAGE_STATE_JS: &str = r#"
(() => {
  const origin = window.location && window.location.origin;
  if (!origin || origin === 'null') return null;
  const entries = (storage) => {
    const out = [];
    for (let index = 0; index < storage.length; index += 1) {
      const name = storage.key(index);
      if (name === null) continue;
      out.push({ name, value: storage.getItem(name) || '' });
    }
    return out;
  };
  return {
    origin,
    localStorage: entries(window.localStorage),
    sessionStorage: entries(window.sessionStorage),
  };
})()
"#;

pub(crate) async fn browser_storage_state(
    connection: &CdpConnection,
    page: Option<&AttachedPage>,
) -> Result<Value, BrowserError> {
    let cookies = connection
        .command("Network.getAllCookies", json!({}), None)
        .await?
        .get("cookies")
        .cloned()
        .unwrap_or_else(|| json!([]));
    let mut state = json!({
        "cookies": cookies,
        "origins": [],
    });
    if let Some(page) = page {
        state["origins"] = Value::Array(origin_storage_states(connection, page).await?);
    }
    Ok(state)
}

pub(crate) async fn origin_storage_states(
    connection: &CdpConnection,
    page: &AttachedPage,
) -> Result<Vec<Value>, BrowserError> {
    let mut states = BTreeMap::new();
    if let Some(origin_state) = current_origin_storage_state(connection, page).await? {
        upsert_origin_storage_state(&mut states, origin_state);
    }

    if let Ok(origins) = frame_security_origins(connection, page).await {
        let _ = connection
            .command("DOMStorage.enable", json!({}), Some(&page.session_id))
            .await;
        for origin in origins {
            if let Some(origin_state) =
                dom_storage_origin_state(connection, page, origin.as_str()).await
            {
                upsert_origin_storage_state(&mut states, origin_state);
            }
        }
    }

    Ok(states.into_values().collect())
}

pub(crate) async fn current_origin_storage_state(
    connection: &CdpConnection,
    page: &AttachedPage,
) -> Result<Option<Value>, BrowserError> {
    let result = connection
        .command(
            "Runtime.evaluate",
            runtime_evaluate_params(ORIGIN_STORAGE_STATE_JS, false),
            Some(&page.session_id),
        )
        .await?;
    let value = runtime_evaluate_value(result)?;
    if value.is_null() || !origin_storage_has_items(&value) {
        return Ok(None);
    }
    Ok(Some(value))
}

pub(crate) fn origin_storage_has_items(origin_state: &Value) -> bool {
    origin_state
        .get("localStorage")
        .and_then(Value::as_array)
        .is_some_and(|items| !items.is_empty())
        || origin_state
            .get("sessionStorage")
            .and_then(Value::as_array)
            .is_some_and(|items| !items.is_empty())
}

pub(crate) fn upsert_origin_storage_state(
    states: &mut BTreeMap<String, Value>,
    origin_state: Value,
) {
    let Some(origin) = origin_state.get("origin").and_then(Value::as_str) else {
        return;
    };
    if !origin_storage_has_items(&origin_state) {
        return;
    }

    states
        .entry(origin.to_owned())
        .and_modify(|existing| {
            *existing = merge_origin_storage_states(existing, &origin_state);
        })
        .or_insert(origin_state);
}

pub(crate) fn merge_origin_storage_states(existing: &Value, incoming: &Value) -> Value {
    let origin = incoming
        .get("origin")
        .and_then(Value::as_str)
        .or_else(|| existing.get("origin").and_then(Value::as_str))
        .unwrap_or_default();
    json!({
        "origin": origin,
        "localStorage": merge_storage_item_arrays(
            existing.get("localStorage"),
            incoming.get("localStorage"),
        ),
        "sessionStorage": merge_storage_item_arrays(
            existing.get("sessionStorage"),
            incoming.get("sessionStorage"),
        ),
    })
}

pub(crate) fn merge_storage_item_arrays(first: Option<&Value>, second: Option<&Value>) -> Value {
    let mut items = BTreeMap::new();
    for item in first
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .chain(second.and_then(Value::as_array).into_iter().flatten())
    {
        let Some(name) = item.get("name").and_then(Value::as_str) else {
            continue;
        };
        let value = item
            .get("value")
            .and_then(Value::as_str)
            .unwrap_or_default();
        items.insert(name.to_owned(), value.to_owned());
    }

    Value::Array(
        items
            .into_iter()
            .map(|(name, value)| json!({ "name": name, "value": value }))
            .collect(),
    )
}

pub(crate) async fn frame_security_origins(
    connection: &CdpConnection,
    page: &AttachedPage,
) -> Result<BTreeSet<String>, BrowserError> {
    let result = connection
        .command("Page.getFrameTree", json!({}), Some(&page.session_id))
        .await?;
    Ok(frame_security_origins_from_result(&result))
}

pub(crate) fn frame_security_origins_from_result(result: &Value) -> BTreeSet<String> {
    let mut origins = BTreeSet::new();
    if let Some(frame_tree) = result.get("frameTree") {
        collect_frame_security_origins(frame_tree, &mut origins);
    }
    origins
}

pub(crate) fn collect_frame_security_origins(frame_tree: &Value, origins: &mut BTreeSet<String>) {
    if let Some(frame) = frame_tree.get("frame") {
        if let Some(origin) = security_origin_for_frame(frame) {
            origins.insert(origin);
        }
    }
    for child in frame_tree
        .get("childFrames")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        collect_frame_security_origins(child, origins);
    }
}

pub(crate) fn security_origin_for_frame(frame: &Value) -> Option<String> {
    frame
        .get("securityOrigin")
        .and_then(Value::as_str)
        .and_then(normalize_http_origin)
        .or_else(|| {
            frame
                .get("url")
                .and_then(Value::as_str)
                .and_then(http_origin_for_url)
        })
}

pub(crate) fn normalize_http_origin(origin: &str) -> Option<String> {
    let origin = origin.trim();
    if origin.starts_with("http://") || origin.starts_with("https://") {
        Some(origin.trim_end_matches('/').to_owned())
    } else {
        None
    }
}

pub(crate) fn http_origin_for_url(url: &str) -> Option<String> {
    let parsed = url::Url::parse(url).ok()?;
    if parsed.scheme() != "http" && parsed.scheme() != "https" {
        return None;
    }
    Some(parsed.origin().ascii_serialization())
}

pub(crate) async fn dom_storage_origin_state(
    connection: &CdpConnection,
    page: &AttachedPage,
    origin: &str,
) -> Option<Value> {
    let local_storage = dom_storage_items(connection, page, origin, true)
        .await
        .unwrap_or_default();
    let session_storage = dom_storage_items(connection, page, origin, false)
        .await
        .unwrap_or_default();
    let origin_state = json!({
        "origin": origin,
        "localStorage": local_storage,
        "sessionStorage": session_storage,
    });
    origin_storage_has_items(&origin_state).then_some(origin_state)
}

pub(crate) async fn dom_storage_items(
    connection: &CdpConnection,
    page: &AttachedPage,
    origin: &str,
    is_local_storage: bool,
) -> Result<Vec<Value>, BrowserError> {
    let result = connection
        .command(
            "DOMStorage.getDOMStorageItems",
            json!({
                "storageId": {
                    "securityOrigin": origin,
                    "isLocalStorage": is_local_storage,
                }
            }),
            Some(&page.session_id),
        )
        .await?;
    Ok(dom_storage_entries_to_items(result.get("entries")))
}

pub(crate) fn dom_storage_entries_to_items(entries: Option<&Value>) -> Vec<Value> {
    let mut items = BTreeMap::new();
    for entry in entries.and_then(Value::as_array).into_iter().flatten() {
        let Some(pair) = entry.as_array() else {
            continue;
        };
        let Some(name) = pair.first().and_then(Value::as_str) else {
            continue;
        };
        let Some(value) = pair.get(1).and_then(Value::as_str) else {
            continue;
        };
        items.insert(name.to_owned(), value.to_owned());
    }
    items
        .into_iter()
        .map(|(name, value)| json!({ "name": name, "value": value }))
        .collect()
}

pub(crate) async fn load_browser_storage_state(
    connection: &CdpConnection,
    path: &Path,
) -> Result<Value, BrowserError> {
    if !path.exists() {
        return Ok(json!({
            "cookies": [],
            "origins": [],
        }));
    }
    let text = tokio::fs::read_to_string(path)
        .await
        .map_err(|error| BrowserError::StateUnavailable(error.to_string()))?;
    let storage_state: Value = serde_json::from_str(&text)
        .map_err(|error| BrowserError::StateUnavailable(error.to_string()))?;
    if let Some(cookies) = storage_state.get("cookies").and_then(Value::as_array) {
        if !cookies.is_empty() {
            connection
                .command("Network.setCookies", json!({ "cookies": cookies }), None)
                .await?;
        }
    }
    Ok(storage_state)
}

pub(crate) async fn apply_origin_storage_state(
    connection: &CdpConnection,
    page: &AttachedPage,
    storage_state: &Value,
) -> Result<(), BrowserError> {
    let origins = storage_state
        .get("origins")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    for origin_state in origins {
        let Some(source) = origin_storage_apply_script(&origin_state) else {
            continue;
        };
        connection
            .command(
                "Page.addScriptToEvaluateOnNewDocument",
                json!({ "source": source }),
                Some(&page.session_id),
            )
            .await?;
        connection
            .command(
                "Runtime.evaluate",
                runtime_evaluate_params(&source, false),
                Some(&page.session_id),
            )
            .await?;
    }
    Ok(())
}

pub(crate) fn origin_storage_apply_script(origin_state: &Value) -> Option<String> {
    let origin = origin_state.get("origin")?.as_str()?;
    let local_storage = storage_items_object(origin_state.get("localStorage"));
    let session_storage = storage_items_object(origin_state.get("sessionStorage"));
    if storage_items_are_empty(&local_storage) && storage_items_are_empty(&session_storage) {
        return None;
    }
    Some(format!(
        r#"(() => {{
  const expectedOrigin = {origin_json};
  if (!window.location || window.location.origin !== expectedOrigin) return;
  const localItems = {local_json};
  for (const [name, value] of Object.entries(localItems)) window.localStorage.setItem(name, value);
  const sessionItems = {session_json};
  for (const [name, value] of Object.entries(sessionItems)) window.sessionStorage.setItem(name, value);
}})()"#,
        origin_json = serde_json::to_string(origin).ok()?,
        local_json = local_storage,
        session_json = session_storage,
    ))
}

pub(crate) fn storage_items_are_empty(value: &Value) -> bool {
    value
        .as_object()
        .map(serde_json::Map::is_empty)
        .unwrap_or(true)
}

pub(crate) fn storage_items_object(items: Option<&Value>) -> Value {
    let mut object = serde_json::Map::new();
    for item in items.and_then(Value::as_array).into_iter().flatten() {
        let Some(name) = item.get("name").and_then(Value::as_str) else {
            continue;
        };
        let value = item
            .get("value")
            .and_then(Value::as_str)
            .unwrap_or_default();
        object.insert(name.to_owned(), Value::String(value.to_owned()));
    }
    Value::Object(object)
}

pub(crate) async fn write_storage_state(
    path: &Path,
    storage_state: &Value,
) -> Result<(), BrowserError> {
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|error| BrowserError::StateUnavailable(error.to_string()))?;
    }
    let text = serde_json::to_string_pretty(storage_state)
        .map_err(|error| BrowserError::StateUnavailable(error.to_string()))?;
    tokio::fs::write(path, text)
        .await
        .map_err(|error| BrowserError::StateUnavailable(error.to_string()))
}

pub(crate) fn storage_state_counts(storage_state: &Value) -> (usize, usize) {
    let cookies_count = storage_state
        .get("cookies")
        .and_then(Value::as_array)
        .map_or(0, Vec::len);
    let origins_count = storage_state
        .get("origins")
        .and_then(Value::as_array)
        .map_or(0, Vec::len);
    (cookies_count, origins_count)
}

pub(crate) async fn page_tabs(connection: &CdpConnection) -> Result<Vec<TabInfo>, BrowserError> {
    let targets = connection
        .command("Target.getTargets", json!({}), None)
        .await?;
    let tabs = targets
        .get("targetInfos")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|target| target.get("type").and_then(Value::as_str) == Some("page"))
        .filter_map(|target| {
            let target_id = target.get("targetId")?.as_str()?.to_owned();
            let tab_id = TabInfo::tab_id_for_target(&target_id);
            Some(TabInfo {
                url: target_info_url(target),
                title: target
                    .get("title")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_owned(),
                tab_id,
                target_id,
                parent_target_id: None,
            })
        })
        .collect();
    Ok(tabs)
}

pub(crate) fn target_info_url(target: &Value) -> String {
    match target.get("url").and_then(Value::as_str) {
        Some("") | None => "about:blank".to_owned(),
        Some(url) => url.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn target_info_url_normalizes_empty_chrome_targets() {
        assert_eq!(
            target_info_url(&json!({ "url": "https://example.test/page" })),
            "https://example.test/page"
        );
        assert_eq!(target_info_url(&json!({ "url": "" })), "about:blank");
        assert_eq!(target_info_url(&json!({})), "about:blank");
    }
}
