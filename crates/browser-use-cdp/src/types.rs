use schemars::JsonSchema;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::PathBuf;
use thiserror::Error;

#[derive(Debug, Clone, Error)]
pub enum BrowserError {
    #[error("browser is not connected")]
    NotConnected,
    #[error("Chrome/Chromium executable not found; checked: {0:?}")]
    ExecutableNotFound(Vec<PathBuf>),
    #[error("browser launch failed: {0}")]
    LaunchFailed(String),
    #[error("timed out waiting for DevToolsActivePort at {0}")]
    DevToolsEndpointTimedOut(PathBuf),
    #[error("CDP transport error: {0}")]
    Transport(String),
    #[error("CDP command {method} failed: {message}")]
    CommandFailed { method: String, message: String },
    #[error("CDP response for {0} was missing expected data")]
    MissingResponseData(String),
    #[error("navigation failed: {0}")]
    NavigationFailed(String),
    #[error("navigation blocked by browser profile policy: {url} ({reason})")]
    NavigationBlocked { url: String, reason: String },
    #[error("action failed: {0}")]
    ActionFailed(String),
    #[error("browser state unavailable: {0}")]
    StateUnavailable(String),
    #[error("Browser Use Cloud authentication failed: {0}")]
    CloudAuth(String),
    #[error("Browser Use Cloud request failed: {0}")]
    Cloud(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Screenshot {
    pub base64_png: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pdf {
    pub base64_pdf: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct FoundElement {
    pub tag_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(default)]
    pub attributes: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct BrowserViewport {
    pub width: u32,
    pub height: u32,
}

impl Default for BrowserViewport {
    fn default() -> Self {
        Self {
            width: 1280,
            height: 720,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ProxySettings {
    pub server: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bypass: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub password: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum CloudProxyCountryCode {
    #[default]
    Unset,
    Disabled,
    Country(String),
}

impl CloudProxyCountryCode {
    #[must_use]
    pub fn disabled() -> Self {
        Self::Disabled
    }

    #[must_use]
    pub fn country(country_code: impl Into<String>) -> Self {
        Self::Country(country_code.into())
    }

    pub(crate) fn is_unset(&self) -> bool {
        matches!(self, Self::Unset)
    }
}

impl JsonSchema for CloudProxyCountryCode {
    fn schema_name() -> String {
        "CloudProxyCountryCode".to_owned()
    }

    fn json_schema(_gen: &mut schemars::r#gen::SchemaGenerator) -> schemars::schema::Schema {
        serde_json::from_value(serde_json::json!({
            "oneOf": [
                { "type": "string" },
                { "type": "null" }
            ]
        }))
        .expect("valid CloudProxyCountryCode JSON schema")
    }
}

pub(crate) fn serialize_cloud_proxy_country_code<S>(
    value: &CloudProxyCountryCode,
    serializer: S,
) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    match value {
        CloudProxyCountryCode::Unset => serializer.serialize_none(),
        CloudProxyCountryCode::Disabled => serializer.serialize_none(),
        CloudProxyCountryCode::Country(country_code) => serializer.serialize_str(country_code),
    }
}

pub(crate) fn deserialize_cloud_proxy_country_code<'de, D>(
    deserializer: D,
) -> Result<CloudProxyCountryCode, D::Error>
where
    D: Deserializer<'de>,
{
    Ok(match Option::<String>::deserialize(deserializer)? {
        Some(country_code) => CloudProxyCountryCode::Country(country_code),
        None => CloudProxyCountryCode::Disabled,
    })
}

pub(crate) fn deserialize_env_map<'de, D>(
    deserializer: D,
) -> Result<BTreeMap<String, String>, D::Error>
where
    D: Deserializer<'de>,
{
    let Some(values) = Option::<BTreeMap<String, Value>>::deserialize(deserializer)? else {
        return Ok(BTreeMap::new());
    };
    values
        .into_iter()
        .map(|(key, value)| env_value_to_string(value).map(|value| (key, value)))
        .collect()
}

fn env_value_to_string<E>(value: Value) -> Result<String, E>
where
    E: serde::de::Error,
{
    match value {
        Value::String(value) => Ok(value),
        Value::Bool(value) => Ok(value.to_string()),
        Value::Number(value) => Ok(value.to_string()),
        other => Err(E::custom(format!(
            "browser env values must be strings, numbers, or booleans; got {other}"
        ))),
    }
}

pub(crate) fn deserialize_non_negative_f64_option<'de, D>(
    deserializer: D,
) -> Result<Option<f64>, D::Error>
where
    D: Deserializer<'de>,
{
    let value = Option::<f64>::deserialize(deserializer)?;
    match value {
        Some(value) if value.is_finite() && value >= 0.0 => Ok(Some(value)),
        Some(value) => Err(serde::de::Error::custom(format!(
            "device_scale_factor must be a finite non-negative number; got {value}"
        ))),
        None => Ok(None),
    }
}

pub(crate) fn deserialize_non_negative_f64<'de, D>(deserializer: D) -> Result<f64, D::Error>
where
    D: Deserializer<'de>,
{
    let value = f64::deserialize(deserializer)?;
    if value.is_finite() && value >= 0.0 {
        Ok(value)
    } else {
        Err(serde::de::Error::custom(format!(
            "page-load wait seconds must be a finite non-negative number; got {value}"
        )))
    }
}
