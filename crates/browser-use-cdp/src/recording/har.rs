//! HAR recording helpers.

use super::format_har_timestamp;
use crate::{
    BrowserError, BrowserProfile, CdpConnection, CdpEvent, RecordHarContent, RecordHarMode,
    cdp_request_key,
};
use base64::Engine;
use serde_json::{Value, json};
use sha1::{Digest, Sha1};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::Mutex;

#[derive(Debug)]
struct CdpHarConfig {
    path: PathBuf,
    content: RecordHarContent,
    mode: RecordHarMode,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct CdpHarState {
    pub(crate) entries: BTreeMap<String, CdpHarEntryBuilder>,
    pub(crate) pages: BTreeMap<String, CdpHarPageBuilder>,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct CdpHarEntryBuilder {
    pub(crate) frame_id: Option<String>,
    pub(crate) document_url: Option<String>,
    pub(crate) url: Option<String>,
    pub(crate) method: Option<String>,
    pub(crate) request_headers: BTreeMap<String, String>,
    pub(crate) post_data: Option<String>,
    pub(crate) status: Option<u64>,
    pub(crate) status_text: Option<String>,
    pub(crate) response_headers: BTreeMap<String, String>,
    pub(crate) mime_type: Option<String>,
    pub(crate) encoded_data: Vec<u8>,
    pub(crate) failed: bool,
    pub(crate) ts_request: Option<f64>,
    pub(crate) wall_time_request: Option<f64>,
    pub(crate) ts_response: Option<f64>,
    pub(crate) ts_finished: Option<f64>,
    pub(crate) encoded_data_length: Option<i64>,
    pub(crate) response_body: Option<Vec<u8>>,
    pub(crate) content_length: Option<i64>,
    pub(crate) protocol: Option<String>,
    pub(crate) server_ip_address: Option<String>,
    pub(crate) server_port: Option<i64>,
    pub(crate) security_details: Option<Value>,
    pub(crate) transfer_size: Option<i64>,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct CdpHarPageBuilder {
    url: String,
    title: String,
    started_date_time: Option<f64>,
    monotonic_start: Option<f64>,
    on_content_load: Option<i64>,
    on_load: Option<i64>,
}

#[derive(Debug)]
pub(crate) struct CdpHarRecorder {
    config: CdpHarConfig,
    pub(crate) state: Mutex<CdpHarState>,
}

impl CdpHarRecorder {
    pub(crate) fn from_profile(profile: &BrowserProfile) -> Option<Arc<Self>> {
        profile.record_har_path.as_ref().map(|path| {
            Arc::new(Self {
                config: CdpHarConfig {
                    path: path.clone(),
                    content: profile.record_har_content,
                    mode: profile.record_har_mode,
                },
                state: Mutex::new(CdpHarState::default()),
            })
        })
    }

    pub(crate) async fn observe_cdp_event(&self, connection: &CdpConnection, event: &CdpEvent) {
        match event.method.as_str() {
            "Network.requestWillBeSent" => self.observe_request_will_be_sent(event).await,
            "Network.responseReceived" => self.observe_response_received(event).await,
            "Network.dataReceived" => self.observe_data_received(event).await,
            "Network.loadingFinished" => {
                self.observe_loading_finished(connection, event).await;
            }
            "Network.loadingFailed" => self.observe_loading_failed(event).await,
            "Page.lifecycleEvent" => self.observe_page_lifecycle(event).await,
            "Page.frameNavigated" => self.observe_frame_navigated(event).await,
            _ => {}
        }
    }

    pub(crate) async fn observe_request_will_be_sent(&self, event: &CdpEvent) {
        if event
            .params
            .get("requestId")
            .and_then(Value::as_str)
            .is_none()
        {
            return;
        }
        let Some(request) = event.params.get("request") else {
            return;
        };
        let Some(url) = request.get("url").and_then(Value::as_str) else {
            return;
        };
        if !is_har_https(url) {
            return;
        }
        let Some(request_key) = cdp_request_key(event) else {
            return;
        };

        let method = request
            .get("method")
            .and_then(Value::as_str)
            .unwrap_or("GET")
            .to_owned();
        let post_data = request
            .get("postData")
            .and_then(Value::as_str)
            .map(str::to_owned);
        let frame_id = event
            .params
            .get("frameId")
            .and_then(Value::as_str)
            .map(str::to_owned);
        let document_url = event
            .params
            .get("documentURL")
            .and_then(Value::as_str)
            .map(str::to_owned);
        let ts_request = event.params.get("timestamp").and_then(Value::as_f64);
        let wall_time_request = event.params.get("wallTime").and_then(Value::as_f64);
        let resource_type = event.params.get("type").and_then(Value::as_str);
        let is_same_document = event
            .params
            .get("isSameDocument")
            .and_then(Value::as_bool)
            .unwrap_or(false);

        let mut state = self.state.lock().await;
        let entry = state
            .entries
            .entry(request_key)
            .or_insert_with(CdpHarEntryBuilder::default);
        entry.url = Some(url.to_owned());
        entry.method = Some(method);
        entry.post_data = post_data;
        entry.request_headers = har_headers_map(request.get("headers"));
        entry.frame_id = frame_id.clone();
        entry.document_url = document_url;
        entry.ts_request = ts_request;
        entry.wall_time_request = wall_time_request;

        if resource_type == Some("Document") && !is_same_document {
            if let Some(frame_id) = frame_id {
                let page = state
                    .pages
                    .entry(frame_id)
                    .or_insert_with(|| CdpHarPageBuilder {
                        url: url.to_owned(),
                        title: url.to_owned(),
                        started_date_time: wall_time_request,
                        monotonic_start: ts_request,
                        on_content_load: None,
                        on_load: None,
                    });
                if wall_time_request.is_some()
                    && (page.started_date_time.is_none()
                        || wall_time_request < page.started_date_time)
                {
                    page.url = url.to_owned();
                    page.title = url.to_owned();
                    page.started_date_time = wall_time_request;
                    page.monotonic_start = ts_request;
                }
            }
        }
    }

    pub(crate) async fn observe_response_received(&self, event: &CdpEvent) {
        let Some(request_key) = cdp_request_key(event) else {
            return;
        };
        let Some(response) = event.params.get("response") else {
            return;
        };

        let headers = har_headers_map(response.get("headers"));
        let mut state = self.state.lock().await;
        let Some(entry) = state.entries.get_mut(&request_key) else {
            return;
        };
        entry.status = response.get("status").and_then(Value::as_u64);
        entry.status_text = response
            .get("statusText")
            .and_then(Value::as_str)
            .map(str::to_owned);
        entry.content_length = headers
            .get("content-length")
            .and_then(|value| value.parse::<i64>().ok());
        entry.response_headers = headers;
        entry.mime_type = response
            .get("mimeType")
            .and_then(Value::as_str)
            .map(str::to_owned);
        entry.ts_response = event.params.get("timestamp").and_then(Value::as_f64);
        entry.protocol = response
            .get("protocol")
            .and_then(Value::as_str)
            .map(har_http_version);
        entry.server_ip_address = response
            .get("remoteIPAddress")
            .and_then(Value::as_str)
            .map(str::to_owned);
        entry.server_port = response.get("remotePort").and_then(Value::as_i64);
        entry.security_details = response.get("securityDetails").cloned();
    }

    pub(crate) async fn observe_data_received(&self, event: &CdpEvent) {
        let Some(request_key) = cdp_request_key(event) else {
            return;
        };
        let Some(data) = event.params.get("data").and_then(Value::as_str) else {
            return;
        };
        let mut state = self.state.lock().await;
        if let Some(entry) = state.entries.get_mut(&request_key) {
            entry.encoded_data.extend_from_slice(data.as_bytes());
        }
    }

    pub(crate) async fn observe_loading_finished(
        &self,
        connection: &CdpConnection,
        event: &CdpEvent,
    ) {
        let Some(request_key) = cdp_request_key(event) else {
            return;
        };
        let Some(request_id) = event.params.get("requestId").and_then(Value::as_str) else {
            return;
        };
        let should_fetch_body = {
            let mut state = self.state.lock().await;
            let Some(entry) = state.entries.get_mut(&request_key) else {
                return;
            };
            entry.ts_finished = event.params.get("timestamp").and_then(Value::as_f64);
            if let Some(encoded_data_length) = event
                .params
                .get("encodedDataLength")
                .and_then(Value::as_i64)
            {
                entry.encoded_data_length = Some(encoded_data_length);
                entry.transfer_size = Some(encoded_data_length);
            }
            self.config.content != RecordHarContent::Omit
        };

        if should_fetch_body {
            let body = connection
                .command(
                    "Network.getResponseBody",
                    json!({ "requestId": request_id }),
                    event.session_id.as_deref(),
                )
                .await
                .ok()
                .and_then(cdp_response_body_bytes_from_value);
            if let Some(body) = body {
                let mut state = self.state.lock().await;
                if let Some(entry) = state.entries.get_mut(&request_key) {
                    entry.response_body = Some(body);
                }
            }
        }
    }

    pub(crate) async fn observe_loading_failed(&self, event: &CdpEvent) {
        let Some(request_key) = cdp_request_key(event) else {
            return;
        };
        let mut state = self.state.lock().await;
        if let Some(entry) = state.entries.get_mut(&request_key) {
            entry.failed = true;
        }
    }

    pub(crate) async fn observe_page_lifecycle(&self, event: &CdpEvent) {
        let Some(frame_id) = event.params.get("frameId").and_then(Value::as_str) else {
            return;
        };
        let Some(name) = event.params.get("name").and_then(Value::as_str) else {
            return;
        };
        let Some(timestamp) = event.params.get("timestamp").and_then(Value::as_f64) else {
            return;
        };
        let mut state = self.state.lock().await;
        let Some(page) = state.pages.get_mut(frame_id) else {
            return;
        };
        let Some(start) = page.monotonic_start else {
            return;
        };
        let elapsed_ms = ((timestamp - start) * 1_000.0).round().max(0.0) as i64;
        match name {
            "DOMContentLoaded" => page.on_content_load = Some(elapsed_ms),
            "load" => page.on_load = Some(elapsed_ms),
            _ => {}
        }
    }

    pub(crate) async fn observe_frame_navigated(&self, event: &CdpEvent) {
        let Some(frame) = event.params.get("frame") else {
            return;
        };
        let Some(frame_id) = frame.get("id").and_then(Value::as_str) else {
            return;
        };
        let title = frame
            .get("name")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .or_else(|| frame.get("url").and_then(Value::as_str))
            .map(str::to_owned);
        let Some(title) = title else {
            return;
        };
        let mut state = self.state.lock().await;
        if let Some(page) = state.pages.get_mut(frame_id) {
            page.title = title;
        }
    }

    pub(crate) async fn write_har(&self) -> Result<(), BrowserError> {
        let state = self.state.lock().await.clone();
        let har_dir = self
            .config
            .path
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."));
        tokio::fs::create_dir_all(&har_dir)
            .await
            .map_err(|error| BrowserError::StateUnavailable(error.to_string()))?;

        let sidecar_dir = if self.config.content == RecordHarContent::Attach {
            let dir = har_dir.join(format!(
                "{}_har_parts",
                self.config
                    .path
                    .file_stem()
                    .and_then(|stem| stem.to_str())
                    .unwrap_or("recording")
            ));
            tokio::fs::create_dir_all(&dir)
                .await
                .map_err(|error| BrowserError::StateUnavailable(error.to_string()))?;
            Some(dir)
        } else {
            None
        };

        let mut entries = Vec::new();
        for entry in state
            .entries
            .values()
            .filter(|entry| self.include_entry(entry, &state.pages))
        {
            entries.push(self.har_entry_json(entry, sidecar_dir.as_deref()).await?);
        }

        let pages = state
            .pages
            .iter()
            .map(|(frame_id, page)| {
                let mut page_timings = serde_json::Map::new();
                if let Some(on_content_load) = page.on_content_load {
                    page_timings.insert("onContentLoad".to_owned(), json!(on_content_load));
                }
                if let Some(on_load) = page.on_load {
                    page_timings.insert("onLoad".to_owned(), json!(on_load));
                }
                json!({
                    "id": format!("page@{frame_id}"),
                    "title": page.title,
                    "startedDateTime": format_har_timestamp(page.started_date_time),
                    "pageTimings": page_timings,
                })
            })
            .collect::<Vec<_>>();

        let har = json!({
            "log": {
                "version": "1.2",
                "creator": {
                    "name": "browser-use-rs",
                    "version": env!("CARGO_PKG_VERSION"),
                },
                "browser": {
                    "name": "Chromium",
                    "version": "",
                },
                "pages": pages,
                "entries": entries,
            }
        });

        let bytes = serde_json::to_vec_pretty(&har)
            .map_err(|error| BrowserError::StateUnavailable(error.to_string()))?;
        let tmp_path = self.config.path.with_extension(format!(
            "{}tmp",
            self.config
                .path
                .extension()
                .and_then(|extension| extension.to_str())
                .map(|extension| format!("{extension}."))
                .unwrap_or_default()
        ));
        tokio::fs::write(&tmp_path, bytes)
            .await
            .map_err(|error| BrowserError::StateUnavailable(error.to_string()))?;
        tokio::fs::rename(&tmp_path, &self.config.path)
            .await
            .map_err(|error| BrowserError::StateUnavailable(error.to_string()))?;
        Ok(())
    }

    async fn har_entry_json(
        &self,
        entry: &CdpHarEntryBuilder,
        sidecar_dir: Option<&Path>,
    ) -> Result<Value, BrowserError> {
        let body_bytes = entry
            .response_body
            .as_deref()
            .unwrap_or(entry.encoded_data.as_slice());
        let content_size = i64::try_from(body_bytes.len()).unwrap_or(i64::MAX);
        let compression = match (entry.content_length, entry.encoded_data_length) {
            (Some(content_length), Some(encoded_data_length)) => {
                Some((content_length - encoded_data_length).max(0))
            }
            _ => None,
        };
        let content = self
            .har_content_json(
                body_bytes,
                entry.mime_type.as_deref(),
                compression,
                sidecar_dir,
            )
            .await?;
        let request_headers = har_header_list(&entry.request_headers);
        let response_headers = har_header_list(&entry.response_headers);
        let request_post_data = self
            .har_request_post_data(entry, sidecar_dir)
            .await?
            .unwrap_or(Value::Null);
        let (started_date_time, total_time_ms, timings) = har_timings(entry);
        let http_version = entry.protocol.as_deref().unwrap_or("HTTP/1.1");
        let response_body_size = entry
            .transfer_size
            .or(entry.encoded_data_length)
            .unwrap_or(if content_size > 0 { content_size } else { -1 });

        let mut entry_json = json!({
            "startedDateTime": started_date_time,
            "time": total_time_ms,
            "request": {
                "method": entry.method.as_deref().unwrap_or("GET"),
                "url": entry.url.as_deref().unwrap_or_default(),
                "httpVersion": http_version,
                "headers": request_headers,
                "queryString": [],
                "cookies": [],
                "headersSize": har_headers_size(
                    entry.method.as_deref(),
                    entry.url.as_deref(),
                    &entry.request_headers
                ),
                "bodySize": har_request_body_size(entry),
                "postData": request_post_data,
            },
            "response": {
                "status": entry.status.unwrap_or(0),
                "statusText": entry.status_text.as_deref().unwrap_or_default(),
                "httpVersion": http_version,
                "headers": response_headers,
                "cookies": [],
                "content": content,
                "redirectURL": "",
                "headersSize": har_headers_size(None, None, &entry.response_headers),
                "bodySize": response_body_size,
            },
            "cache": {},
            "timings": timings,
            "pageref": entry.frame_id.as_ref().and_then(|frame_id| {
                entry_has_page_ref(frame_id, entry, None)
            }),
        });

        if let Some(frame_id) = &entry.frame_id {
            if let Some(pageref) = entry_has_page_ref(frame_id, entry, Some(frame_id)) {
                entry_json["pageref"] = json!(pageref);
            }
        }
        if let Some(server_ip_address) = &entry.server_ip_address {
            entry_json["serverIPAddress"] = json!(server_ip_address);
        }
        if let Some(server_port) = entry.server_port {
            entry_json["_serverPort"] = json!(server_port);
        }
        if let Some(security_details) = har_security_details(entry.security_details.as_ref()) {
            entry_json["_securityDetails"] = security_details;
        }
        if let Some(transfer_size) = entry.transfer_size {
            entry_json["response"]["_transferSize"] = json!(transfer_size);
        }

        Ok(entry_json)
    }

    async fn har_content_json(
        &self,
        body_bytes: &[u8],
        mime_type: Option<&str>,
        compression: Option<i64>,
        sidecar_dir: Option<&Path>,
    ) -> Result<Value, BrowserError> {
        let mut content = serde_json::Map::from_iter([(
            "mimeType".to_owned(),
            json!(mime_type.unwrap_or_default()),
        )]);
        let content_size = i64::try_from(body_bytes.len()).unwrap_or(i64::MAX);
        content.insert("size".to_owned(), json!(content_size));

        match self.config.content {
            RecordHarContent::Embed if !body_bytes.is_empty() => {
                match std::str::from_utf8(body_bytes) {
                    Ok(text) => {
                        content.insert("text".to_owned(), json!(text));
                    }
                    Err(_) => {
                        content.insert(
                            "text".to_owned(),
                            json!(base64::engine::general_purpose::STANDARD.encode(body_bytes)),
                        );
                        content.insert("encoding".to_owned(), json!("base64"));
                    }
                }
            }
            RecordHarContent::Attach if !body_bytes.is_empty() => {
                if let Some(sidecar_dir) = sidecar_dir {
                    let filename = har_attachment_filename(body_bytes, mime_type);
                    tokio::fs::write(sidecar_dir.join(&filename), body_bytes)
                        .await
                        .map_err(|error| BrowserError::StateUnavailable(error.to_string()))?;
                    content.insert("_file".to_owned(), json!(filename));
                }
            }
            RecordHarContent::Omit | RecordHarContent::Embed | RecordHarContent::Attach => {}
        }

        if content_size > 0 {
            if let Some(compression) = compression {
                content.insert("compression".to_owned(), json!(compression));
            }
        }
        Ok(Value::Object(content))
    }

    async fn har_request_post_data(
        &self,
        entry: &CdpHarEntryBuilder,
        sidecar_dir: Option<&Path>,
    ) -> Result<Option<Value>, BrowserError> {
        let Some(post_data) = &entry.post_data else {
            return Ok(None);
        };
        if self.config.content == RecordHarContent::Omit {
            return Ok(None);
        }
        let mime_type = entry
            .request_headers
            .get("content-type")
            .map(String::as_str)
            .unwrap_or("text/plain");
        if self.config.content == RecordHarContent::Attach {
            let post_data_bytes = post_data.as_bytes();
            let filename = har_attachment_filename(post_data_bytes, Some(mime_type));
            if let Some(sidecar_dir) = sidecar_dir {
                tokio::fs::write(sidecar_dir.join(&filename), post_data_bytes)
                    .await
                    .map_err(|error| BrowserError::StateUnavailable(error.to_string()))?;
            }
            return Ok(Some(json!({
                "mimeType": mime_type,
                "_file": filename,
            })));
        }
        Ok(Some(json!({
            "mimeType": mime_type,
            "text": post_data,
        })))
    }

    fn include_entry(
        &self,
        entry: &CdpHarEntryBuilder,
        pages: &BTreeMap<String, CdpHarPageBuilder>,
    ) -> bool {
        let Some(url) = entry.url.as_deref() else {
            return false;
        };
        if !is_har_https(url) || url.to_ascii_lowercase().contains("/favicon.ico") {
            return false;
        }
        if self.config.mode == RecordHarMode::Full {
            return true;
        }
        let Some(frame_id) = &entry.frame_id else {
            return false;
        };
        let Some(page) = pages.get(frame_id) else {
            return false;
        };
        har_origin(url) == har_origin(&page.url)
    }
}

fn is_har_https(url: &str) -> bool {
    url.to_ascii_lowercase().starts_with("https://")
}

fn cdp_response_body_bytes_from_value(response: Value) -> Option<Vec<u8>> {
    let body = response.get("body").and_then(Value::as_str)?;
    if response
        .get("base64Encoded")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        base64::engine::general_purpose::STANDARD.decode(body).ok()
    } else {
        Some(body.as_bytes().to_vec())
    }
}

fn har_headers_map(headers: Option<&Value>) -> BTreeMap<String, String> {
    let Some(headers) = headers else {
        return BTreeMap::new();
    };
    match headers {
        Value::Object(headers) => headers
            .iter()
            .map(|(name, value)| {
                (
                    name.to_ascii_lowercase(),
                    value
                        .as_str()
                        .map(str::to_owned)
                        .unwrap_or_else(|| value.to_string()),
                )
            })
            .collect(),
        Value::Array(headers) => headers
            .iter()
            .filter_map(|header| {
                let name = header.get("name")?.as_str()?;
                let value = header
                    .get("value")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                Some((name.to_ascii_lowercase(), value.to_owned()))
            })
            .collect(),
        _ => BTreeMap::new(),
    }
}

fn har_header_list(headers: &BTreeMap<String, String>) -> Vec<Value> {
    headers
        .iter()
        .map(|(name, value)| json!({ "name": name, "value": value }))
        .collect()
}

fn har_headers_size(
    method: Option<&str>,
    url: Option<&str>,
    headers: &BTreeMap<String, String>,
) -> i64 {
    let mut size = 0i64;
    if let (Some(method), Some(url)) = (method, url) {
        size += format!("{method} {url} HTTP/1.1\r\n").len() as i64;
    }
    for (name, value) in headers {
        size += format!("{name}: {value}\r\n").len() as i64;
    }
    size + 2
}

fn har_request_body_size(entry: &CdpHarEntryBuilder) -> i64 {
    if let Some(content_length) = entry
        .request_headers
        .get("content-length")
        .and_then(|value| value.parse::<i64>().ok())
    {
        return content_length;
    }
    if let Some(post_data) = &entry.post_data {
        return i64::try_from(post_data.len()).unwrap_or(i64::MAX);
    }
    if matches!(entry.method.as_deref(), Some("GET" | "HEAD")) {
        return 0;
    }
    -1
}

fn har_http_version(protocol: &str) -> String {
    let protocol = protocol.to_ascii_lowercase();
    if protocol == "h2" || protocol.starts_with("http/2") {
        "HTTP/2.0".to_owned()
    } else if protocol.starts_with("http/1.1") {
        "HTTP/1.1".to_owned()
    } else if protocol.starts_with("http/1.0") {
        "HTTP/1.0".to_owned()
    } else {
        protocol.to_ascii_uppercase()
    }
}

fn har_timings(entry: &CdpHarEntryBuilder) -> (String, i64, Value) {
    let wait_ms = entry
        .ts_request
        .zip(entry.ts_response)
        .map(|(start, response)| ((response - start) * 1_000.0).round().max(0.0) as i64)
        .unwrap_or(0);
    let receive_ms = entry
        .ts_response
        .zip(entry.ts_finished)
        .map(|(response, finished)| ((finished - response) * 1_000.0).round().max(0.0) as i64)
        .unwrap_or(0);
    let total = wait_ms + receive_ms;
    (
        format_har_timestamp(entry.wall_time_request),
        total,
        json!({
            "dns": 0,
            "connect": 0,
            "ssl": 0,
            "send": 0,
            "wait": wait_ms,
            "receive": receive_ms,
        }),
    )
}

fn har_security_details(security_details: Option<&Value>) -> Option<Value> {
    let security_details = security_details?.as_object()?;
    let mut filtered = serde_json::Map::new();
    for key in ["protocol", "subjectName", "issuer", "validFrom", "validTo"] {
        if let Some(value) = security_details.get(key) {
            filtered.insert(key.to_owned(), value.clone());
        }
    }
    (!filtered.is_empty()).then_some(Value::Object(filtered))
}

fn entry_has_page_ref(
    frame_id: &str,
    entry: &CdpHarEntryBuilder,
    known_frame_id: Option<&String>,
) -> Option<String> {
    if known_frame_id.is_some() || entry.frame_id.as_deref() == Some(frame_id) {
        Some(format!("page@{frame_id}"))
    } else {
        None
    }
}

fn har_origin(raw_url: &str) -> Option<String> {
    let parsed = url::Url::parse(raw_url).ok()?;
    let host = parsed.host_str()?;
    let port = parsed
        .port()
        .map(|port| format!(":{port}"))
        .unwrap_or_default();
    Some(format!("{}://{host}{port}", parsed.scheme()))
}

fn har_attachment_filename(content: &[u8], mime_type: Option<&str>) -> String {
    let mut hasher = Sha1::new();
    hasher.update(content);
    let hash = format!("{:x}", hasher.finalize());
    format!("{hash}.{}", har_mime_extension(mime_type))
}

fn har_mime_extension(mime_type: Option<&str>) -> &'static str {
    let Some(mime_type) = mime_type else {
        return "bin";
    };
    match mime_type
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase()
        .as_str()
    {
        "text/html" => "html",
        "text/css" => "css",
        "text/javascript" | "application/javascript" | "application/x-javascript" => "js",
        "application/json" => "json",
        "application/xml" | "text/xml" => "xml",
        "text/plain" => "txt",
        "image/png" => "png",
        "image/jpeg" | "image/jpg" => "jpg",
        "image/gif" => "gif",
        "image/webp" => "webp",
        "image/svg+xml" => "svg",
        "image/x-icon" => "ico",
        "font/woff" | "application/font-woff" | "application/x-font-woff" => "woff",
        "font/woff2" | "application/font-woff2" | "application/x-font-woff2" => "woff2",
        "font/ttf" | "application/x-font-ttf" => "ttf",
        "font/otf" | "application/x-font-opentype" => "otf",
        "application/pdf" => "pdf",
        "application/zip" | "application/x-zip-compressed" => "zip",
        "video/mp4" => "mp4",
        "video/webm" => "webm",
        "audio/mpeg" | "audio/mp3" => "mp3",
        "audio/wav" => "wav",
        "audio/ogg" => "ogg",
        _ => "bin",
    }
}
