use crate::{
    BrowserError, CLOUD_HTTP_TIMEOUT, CloudProxyCountryCode, DevToolsEndpoint,
    deserialize_cloud_proxy_country_code, is_false, serialize_cloud_proxy_country_code,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::Mutex;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CloudBrowserCreateRequest {
    #[serde(
        default,
        alias = "cloud_profile_id",
        skip_serializing_if = "Option::is_none"
    )]
    pub profile_id: Option<String>,
    #[serde(
        default,
        alias = "cloud_proxy_country_code",
        skip_serializing_if = "CloudProxyCountryCode::is_unset",
        serialize_with = "serialize_cloud_proxy_country_code",
        deserialize_with = "deserialize_cloud_proxy_country_code"
    )]
    pub proxy_country_code: CloudProxyCountryCode,
    #[serde(
        default,
        alias = "cloud_timeout",
        skip_serializing_if = "Option::is_none"
    )]
    pub timeout: Option<u32>,
    #[serde(default, alias = "enableRecording", skip_serializing_if = "is_false")]
    pub enable_recording: bool,
}

pub type CreateCloudBrowserRequest = CloudBrowserCreateRequest;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CloudBrowserResponse {
    pub id: String,
    pub status: String,
    #[serde(rename = "liveUrl", alias = "live_url")]
    pub live_url: String,
    #[serde(rename = "cdpUrl", alias = "cdp_url")]
    pub cdp_url: String,
    #[serde(rename = "timeoutAt", alias = "timeout_at")]
    pub timeout_at: String,
    #[serde(rename = "startedAt", alias = "started_at")]
    pub started_at: String,
    #[serde(
        default,
        rename = "finishedAt",
        alias = "finished_at",
        skip_serializing_if = "Option::is_none"
    )]
    pub finished_at: Option<String>,
}

impl CloudBrowserResponse {
    pub fn devtools_endpoint(&self) -> Result<DevToolsEndpoint, BrowserError> {
        DevToolsEndpoint::from_cdp_url(&self.cdp_url)
    }
}

pub struct CloudBrowserClient {
    api_base_url: String,
    api_key: Option<String>,
    auth_config_path: Option<PathBuf>,
    client: reqwest::Client,
    current_session_id: Arc<Mutex<Option<String>>>,
}

impl Default for CloudBrowserClient {
    fn default() -> Self {
        Self::new()
    }
}

impl CloudBrowserClient {
    #[must_use]
    pub fn new() -> Self {
        Self {
            api_base_url: "https://api.browser-use.com".to_owned(),
            api_key: None,
            auth_config_path: None,
            client: cloud_http_client(),
            current_session_id: Arc::new(Mutex::new(None)),
        }
    }

    #[must_use]
    pub fn with_api_key(api_key: impl Into<String>) -> Self {
        Self {
            api_key: Some(api_key.into()),
            ..Self::new()
        }
    }

    #[must_use]
    pub fn with_api_base_url(mut self, api_base_url: impl Into<String>) -> Self {
        self.api_base_url = api_base_url.into().trim_end_matches('/').to_owned();
        self
    }

    #[must_use]
    pub fn with_base_url(self, api_base_url: impl Into<String>) -> Self {
        self.with_api_base_url(api_base_url)
    }

    #[must_use]
    pub fn with_auth_config_path(mut self, auth_config_path: impl Into<PathBuf>) -> Self {
        self.auth_config_path = Some(auth_config_path.into());
        self
    }

    pub async fn current_session_id(&self) -> Option<String> {
        self.current_session_id.lock().await.clone()
    }

    pub async fn create_browser(
        &self,
        request: &CloudBrowserCreateRequest,
    ) -> Result<CloudBrowserResponse, BrowserError> {
        self.create_browser_with_headers(request, std::iter::empty::<(&str, &str)>())
            .await
    }

    pub async fn create_browser_with_headers<K, V, I>(
        &self,
        request: &CloudBrowserCreateRequest,
        extra_headers: I,
    ) -> Result<CloudBrowserResponse, BrowserError>
    where
        K: AsRef<str>,
        V: AsRef<str>,
        I: IntoIterator<Item = (K, V)>,
    {
        let api_key = self.api_key()?;
        let url = format!("{}/api/v2/browsers", self.api_base_url);
        let headers = cloud_request_headers(api_key, extra_headers)?;
        let body =
            serde_json::to_vec(request).map_err(|error| BrowserError::Cloud(error.to_string()))?;
        let response = self
            .client
            .post(url)
            .headers(headers)
            .body(body)
            .send()
            .await
            .map_err(|error| cloud_request_error("creating", error))?;
        let status = response.status();
        if status == reqwest::StatusCode::UNAUTHORIZED {
            return Err(BrowserError::CloudAuth(
                "BROWSER_USE_API_KEY is invalid. Get a new key at https://cloud.browser-use.com/new-api-key?utm_source=oss&utm_medium=use_cloud"
                    .to_owned(),
            ));
        }
        if status == reqwest::StatusCode::FORBIDDEN {
            return Err(BrowserError::CloudAuth(
                "Access forbidden. Please check your Browser Use Cloud subscription status."
                    .to_owned(),
            ));
        }
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(BrowserError::Cloud(format!(
                "Failed to create cloud browser: HTTP {status}{}",
                render_cloud_error_body(&body)
            )));
        }
        let response = response
            .json::<CloudBrowserResponse>()
            .await
            .map_err(|error| {
                BrowserError::Cloud(format!("Unexpected error creating cloud browser: {error}"))
            })?;
        *self.current_session_id.lock().await = Some(response.id.clone());
        Ok(response)
    }

    pub async fn stop_browser(
        &self,
        session_id: Option<&str>,
    ) -> Result<CloudBrowserResponse, BrowserError> {
        self.stop_browser_with_headers(session_id, std::iter::empty::<(&str, &str)>())
            .await
    }

    pub async fn stop_browser_with_headers<K, V, I>(
        &self,
        session_id: Option<&str>,
        extra_headers: I,
    ) -> Result<CloudBrowserResponse, BrowserError>
    where
        K: AsRef<str>,
        V: AsRef<str>,
        I: IntoIterator<Item = (K, V)>,
    {
        let session_id = match session_id {
            Some(session_id) if !session_id.trim().is_empty() => session_id.to_owned(),
            _ => self.current_session_id().await.ok_or_else(|| {
                BrowserError::Cloud(
                    "No session ID provided and no current session available".to_owned(),
                )
            })?,
        };
        let api_key = self.api_key()?;
        let url = format!("{}/api/v2/browsers/{session_id}", self.api_base_url);
        let headers = cloud_request_headers(api_key, extra_headers)?;
        let body = serde_json::to_vec(&serde_json::json!({ "action": "stop" }))
            .map_err(|error| BrowserError::Cloud(error.to_string()))?;
        let response = self
            .client
            .patch(url)
            .headers(headers)
            .body(body)
            .send()
            .await
            .map_err(|error| cloud_request_error("stopping", error))?;
        let status = response.status();
        if status == reqwest::StatusCode::UNAUTHORIZED {
            return Err(BrowserError::CloudAuth(
                "Authentication failed. Please make sure BROWSER_USE_API_KEY is set for Browser Use Cloud."
                    .to_owned(),
            ));
        }
        if status == reqwest::StatusCode::NOT_FOUND {
            self.clear_current_session_if(&session_id).await;
            return Err(BrowserError::Cloud(format!(
                "Cloud browser session {session_id} not found"
            )));
        }
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(BrowserError::Cloud(format!(
                "Failed to stop cloud browser: HTTP {status}{}",
                render_cloud_error_body(&body)
            )));
        }
        let response = response
            .json::<CloudBrowserResponse>()
            .await
            .map_err(|error| {
                BrowserError::Cloud(format!("Unexpected error stopping cloud browser: {error}"))
            })?;
        self.clear_current_session_if(&session_id).await;
        Ok(response)
    }

    pub async fn close(&self) {
        let _ = self.stop_browser(None).await;
    }

    fn api_key(&self) -> Result<String, BrowserError> {
        resolve_cloud_api_key(
            self.api_key.as_deref(),
            std::env::var("BROWSER_USE_API_KEY").ok(),
            self.auth_config_path.as_deref(),
        )
        .ok_or_else(|| {
                BrowserError::CloudAuth(
                    "BROWSER_USE_API_KEY is not set. To use cloud browsers, get a key at https://cloud.browser-use.com/new-api-key?utm_source=oss&utm_medium=use_cloud"
                        .to_owned(),
                )
            })
    }

    async fn clear_current_session_if(&self, session_id: &str) {
        let mut current_session_id = self.current_session_id.lock().await;
        if current_session_id.as_deref() == Some(session_id) {
            *current_session_id = None;
        }
    }
}

fn cloud_http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(CLOUD_HTTP_TIMEOUT)
        .build()
        .expect("valid Browser Use Cloud HTTP client")
}

pub(crate) fn download_http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(CLOUD_HTTP_TIMEOUT)
        .build()
        .expect("valid Browser Use download HTTP client")
}

fn cloud_request_headers<K, V, I>(
    api_key: String,
    extra_headers: I,
) -> Result<reqwest::header::HeaderMap, BrowserError>
where
    K: AsRef<str>,
    V: AsRef<str>,
    I: IntoIterator<Item = (K, V)>,
{
    let mut headers = reqwest::header::HeaderMap::new();
    let api_key = reqwest::header::HeaderValue::from_str(&api_key)
        .map_err(|error| BrowserError::Cloud(format!("Invalid cloud API key header: {error}")))?;
    headers.insert("X-Browser-Use-API-Key", api_key);
    headers.insert(
        reqwest::header::CONTENT_TYPE,
        reqwest::header::HeaderValue::from_static("application/json"),
    );
    for (name, value) in extra_headers {
        let name = name.as_ref();
        let header_name =
            reqwest::header::HeaderName::from_bytes(name.as_bytes()).map_err(|error| {
                BrowserError::Cloud(format!("Invalid cloud extra header name {name:?}: {error}"))
            })?;
        let value = value.as_ref();
        let header_value = reqwest::header::HeaderValue::from_str(value).map_err(|error| {
            BrowserError::Cloud(format!(
                "Invalid cloud extra header value for {header_name}: {error}"
            ))
        })?;
        headers.insert(header_name, header_value);
    }
    Ok(headers)
}

fn cloud_request_error(action: &str, error: reqwest::Error) -> BrowserError {
    if error.is_timeout() {
        return BrowserError::Cloud(format!(
            "Timeout while {action} cloud browser. Please try again."
        ));
    }
    if error.is_connect() {
        return BrowserError::Cloud(
            "Failed to connect to cloud browser service. Please check your internet connection."
                .to_owned(),
        );
    }
    BrowserError::Cloud(format!("Unexpected error {action} cloud browser: {error}"))
}

pub(crate) fn resolve_cloud_api_key(
    explicit_api_key: Option<&str>,
    env_api_key: Option<String>,
    auth_config_path: Option<&Path>,
) -> Option<String> {
    explicit_api_key
        .and_then(non_empty_string)
        .or_else(|| env_api_key.as_deref().and_then(non_empty_string))
        .or_else(|| load_cloud_auth_api_token(auth_config_path))
}

fn non_empty_string(value: &str) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_owned())
}

pub(crate) fn load_cloud_auth_api_token(auth_config_path: Option<&Path>) -> Option<String> {
    let path = auth_config_path
        .map(Path::to_path_buf)
        .unwrap_or_else(default_cloud_auth_config_path);
    let contents = std::fs::read_to_string(path).ok()?;
    let value = serde_json::from_str::<Value>(&contents).ok()?;
    value
        .get("api_token")
        .and_then(Value::as_str)
        .and_then(|token| {
            let token = token.trim();
            (!token.is_empty()).then(|| token.to_owned())
        })
}

fn default_cloud_auth_config_path() -> PathBuf {
    cloud_auth_config_path(
        std::env::var_os("BROWSER_USE_CONFIG_DIR").map(PathBuf::from),
        std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from),
        std::env::var_os("HOME").map(PathBuf::from),
    )
}

pub(crate) fn cloud_auth_config_path(
    browser_use_config_dir: Option<PathBuf>,
    xdg_config_home: Option<PathBuf>,
    home: Option<PathBuf>,
) -> PathBuf {
    let config_dir = browser_use_config_dir
        .filter(|path| !path.as_os_str().is_empty())
        .map(|path| expand_home(path, home.as_deref()))
        .unwrap_or_else(|| {
            xdg_config_home
                .filter(|path| !path.as_os_str().is_empty())
                .map(|path| expand_home(path, home.as_deref()))
                .unwrap_or_else(|| expand_home(PathBuf::from("~/.config"), home.as_deref()))
                .join("browseruse")
        });
    config_dir.join("cloud_auth.json")
}

fn expand_home(path: PathBuf, home: Option<&Path>) -> PathBuf {
    let Some(path_text) = path.to_str() else {
        return path;
    };
    if path_text == "~" {
        return home.map(Path::to_path_buf).unwrap_or(path);
    }
    if let Some(rest) = path_text.strip_prefix("~/") {
        return home
            .map(|home| home.join(rest))
            .unwrap_or_else(|| PathBuf::from(path_text));
    }
    path
}

fn render_cloud_error_body(body: &str) -> String {
    if body.trim().is_empty() {
        return String::new();
    }
    serde_json::from_str::<Value>(body)
        .ok()
        .and_then(|value| {
            value
                .get("detail")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned)
        })
        .or_else(|| Some(body.to_owned()))
        .map(|detail| format!(" - {detail}"))
        .unwrap_or_default()
}
