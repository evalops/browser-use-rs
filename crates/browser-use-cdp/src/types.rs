//! Shared public types for the CDP browser-session layer.
//!
//! These are deliberately small data carriers. Higher-level modules compose
//! them into launch profiles, cloud requests, browser-state snapshots, and
//! executor results.

use schemars::JsonSchema;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::PathBuf;
use thiserror::Error;

/// Error type used by browser session, launch, cloud, and CDP transport code.
#[derive(Debug, Clone, Error)]
pub enum BrowserError {
    /// A browser command was attempted before a connection was available.
    #[error("browser is not connected")]
    NotConnected,
    /// No Chrome or Chromium executable was found in any checked location.
    #[error("Chrome/Chromium executable not found; checked: {0:?}")]
    ExecutableNotFound(Vec<PathBuf>),
    /// Chrome started unsuccessfully or a launch profile was invalid.
    #[error("browser launch failed: {0}")]
    LaunchFailed(String),
    /// Chrome did not write the `DevToolsActivePort` file before the timeout.
    #[error("timed out waiting for DevToolsActivePort at {0}")]
    DevToolsEndpointTimedOut(PathBuf),
    /// WebSocket transport failed before a structured CDP response existed.
    #[error("CDP transport error: {0}")]
    Transport(String),
    /// Chrome returned an error response for a CDP command.
    #[error("CDP command {method} failed: {message}")]
    CommandFailed {
        /// CDP method name that failed.
        method: String,
        /// CDP error message.
        message: String,
    },
    /// A successful CDP response lacked a required field.
    #[error("CDP response for {0} was missing expected data")]
    MissingResponseData(String),
    /// A navigation operation failed.
    #[error("navigation failed: {0}")]
    NavigationFailed(String),
    /// URL policy rejected a navigation.
    #[error("navigation blocked by browser profile policy: {url} ({reason})")]
    NavigationBlocked {
        /// Rejected URL.
        url: String,
        /// Policy reason explaining the rejection.
        reason: String,
    },
    /// Browser action failed in a user-visible way.
    #[error("action failed: {0}")]
    ActionFailed(String),
    /// Browser state could not be captured or interpreted.
    #[error("browser state unavailable: {0}")]
    StateUnavailable(String),
    /// Browser Use Cloud authentication failed.
    #[error("Browser Use Cloud authentication failed: {0}")]
    CloudAuth(String),
    /// Browser Use Cloud request failed after authentication.
    #[error("Browser Use Cloud request failed: {0}")]
    Cloud(String),
}

/// PNG screenshot returned by CDP.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Screenshot {
    /// Base64-encoded PNG bytes without a data-URL prefix.
    pub base64_png: String,
}

/// PDF bytes returned by Chrome's print-to-PDF command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pdf {
    /// Base64-encoded PDF bytes without a data-URL prefix.
    pub base64_pdf: String,
}

/// Element returned by the `find_elements` browser action.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct FoundElement {
    /// Element tag name.
    pub tag_name: String,
    /// Optional visible text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    /// Requested attributes captured for the element.
    #[serde(default)]
    pub attributes: BTreeMap<String, String>,
}

/// Width and height pair used for browser viewport and window dimensions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct BrowserViewport {
    /// Width in CSS pixels.
    pub width: u32,
    /// Height in CSS pixels.
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

/// Proxy configuration translated into Chrome launch arguments.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ProxySettings {
    /// Proxy server URI or host:port accepted by Chrome's `--proxy-server`.
    pub server: String,
    /// Optional Chrome proxy bypass list.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bypass: Option<String>,
    /// Optional proxy username for callers that need to retain credentials.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    /// Optional proxy password for callers that need to retain credentials.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub password: Option<String>,
}

/// Browser Use Cloud proxy-country setting.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum CloudProxyCountryCode {
    /// Caller did not specify the field.
    #[default]
    Unset,
    /// Caller explicitly disabled country selection.
    Disabled,
    /// Caller requested a country code such as `US`.
    Country(String),
}

impl CloudProxyCountryCode {
    /// Returns the explicit disabled state.
    #[must_use]
    pub fn disabled() -> Self {
        Self::Disabled
    }

    /// Returns a country-code request.
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
