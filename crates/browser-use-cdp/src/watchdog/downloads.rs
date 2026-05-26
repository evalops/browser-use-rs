use crate::{BrowserError, BrowserLifecycleEvent, CdpConnection, CdpEvent};
use base64::Engine;
use percent_encoding::percent_decode_str;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::Mutex;

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

fn cdp_value_to_u64(value: &Value) -> Option<u64> {
    value.as_u64().or_else(|| {
        value
            .as_f64()
            .filter(|value| *value >= 0.0)
            .map(|value| value as u64)
    })
}
