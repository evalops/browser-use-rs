//! Core agent contracts for browser-use-rs.

use regex::Regex;
use schemars::JsonSchema;
use serde::{Deserialize, Deserializer, Serialize, Serializer, de};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::future::Future;
use std::io::{Read, Write};
use std::pin::Pin;
use std::sync::OnceLock;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use thiserror::Error;
use tokio::time::{sleep, timeout};
use uuid::Uuid;

use async_trait::async_trait;
use base64::Engine;
use browser_use_cdp::{BrowserError, BrowserSession, FoundElement};
use encoding_rs::Encoding;
use url::form_urlencoded;

pub use browser_use_dom::{
    BrowserStateSummary, DomInteractedElement, DomInteractedElementMatch,
    DomInteractedElementMatchFailure, DomInteractedElementMatchFailureReason,
    DomInteractedElementMatchLevel, SerializedDomState,
};
pub use browser_use_llm::{
    AnthropicChatModel, ChatCompletion, ChatMessage, ChatModel, ChatRequest, ContentPart,
    GeminiChatModel, ImageDetailLevel, LlmError, MessageRole, OllamaChatModel,
    OpenAiCompatibleChatModel,
};
pub use browser_use_tools::{BrowserAction, SearchEngine};

/// Version of the upstream browser-use source that this crate initially targets.
pub const INITIAL_UPSTREAM_COMMIT: &str = "f09a86671591312bbc272403a7409d56f4cec668";

const ACTION_TIMEOUT_ENV_VAR: &str = "BROWSER_USE_ACTION_TIMEOUT_S";
const ACTION_TIMEOUT_FALLBACK_SECONDS: f64 = 180.0;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct AgentSettings {
    #[serde(default = "default_use_vision")]
    pub use_vision: VisionMode,
    #[serde(default = "default_vision_detail_level")]
    pub vision_detail_level: ImageDetailLevel,
    #[serde(default = "default_max_failures")]
    pub max_failures: u32,
    #[serde(default)]
    pub generate_gif: GenerateGif,
    #[serde(default = "default_max_actions_per_step")]
    pub max_actions_per_step: usize,
    #[serde(default = "default_llm_timeout_seconds")]
    pub llm_timeout_seconds: u64,
    #[serde(default = "default_step_timeout_seconds")]
    pub step_timeout_seconds: u64,
    #[serde(default = "default_action_timeout_seconds")]
    pub action_timeout_seconds: f64,
    #[serde(default = "default_true")]
    pub directly_open_url: bool,
    #[serde(default = "default_final_response_after_failure")]
    pub final_response_after_failure: bool,
    #[serde(default = "default_display_files_in_done_text")]
    pub display_files_in_done_text: bool,
    #[serde(default = "default_loop_detection_window")]
    pub loop_detection_window: usize,
    #[serde(default = "default_loop_detection_enabled")]
    pub loop_detection_enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_history_items: Option<usize>,
    #[serde(default = "default_max_clickable_elements_length")]
    pub max_clickable_elements_length: usize,
    #[serde(default)]
    pub include_recent_events: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sample_images: Vec<ContentPart>,
    #[serde(default = "default_enable_planning")]
    pub enable_planning: bool,
    #[serde(default = "default_planning_replan_on_stall")]
    pub planning_replan_on_stall: usize,
    #[serde(default = "default_planning_exploration_limit")]
    pub planning_exploration_limit: usize,
    #[serde(default = "default_use_thinking")]
    pub use_thinking: bool,
    #[serde(default)]
    pub flash_mode: bool,
    #[serde(default = "default_use_judge")]
    pub use_judge: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ground_truth: Option<String>,
    #[serde(default, skip_serializing_if = "is_default_message_compaction")]
    pub message_compaction: MessageCompaction,
    #[serde(default)]
    pub calculate_cost: bool,
    #[serde(default)]
    pub include_tool_call_examples: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub save_conversation_path: Option<String>,
    #[serde(
        default = "default_save_conversation_path_encoding",
        skip_serializing_if = "is_default_save_conversation_path_encoding"
    )]
    pub save_conversation_path_encoding: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub include_attributes: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub available_file_paths: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub initial_actions: Vec<BrowserAction>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub excluded_actions: Vec<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub sensitive_data: BTreeMap<String, SensitiveDataValue>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub override_system_message: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extend_system_message: Option<String>,
}

/// Upstream-compatible vision behavior.
///
/// Python browser-use accepts `True`, `False`, or `"auto"` for `use_vision`.
/// The JSON contract preserves that shape so existing MCP/CLI callers can send
/// booleans while Rust code gets an explicit mode.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum VisionMode {
    #[default]
    Always,
    Never,
    Auto,
}

impl VisionMode {
    #[must_use]
    pub fn includes_screenshot_by_default(self) -> bool {
        matches!(self, Self::Always)
    }

    #[must_use]
    pub fn allows_screenshot_action(self) -> bool {
        matches!(self, Self::Auto)
    }

    #[must_use]
    pub fn should_include_screenshot(self, action_requested_screenshot: bool) -> bool {
        match self {
            Self::Always => true,
            Self::Never => false,
            Self::Auto => action_requested_screenshot,
        }
    }

    #[must_use]
    pub fn accepts_prompt_image(self) -> bool {
        !matches!(self, Self::Never)
    }
}

impl Serialize for VisionMode {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self {
            Self::Always => serializer.serialize_bool(true),
            Self::Never => serializer.serialize_bool(false),
            Self::Auto => serializer.serialize_str("auto"),
        }
    }
}

impl<'de> Deserialize<'de> for VisionMode {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_any(VisionModeVisitor)
    }
}

struct VisionModeVisitor;

impl<'de> de::Visitor<'de> for VisionModeVisitor {
    type Value = VisionMode;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("true, false, or \"auto\"")
    }

    fn visit_bool<E>(self, value: bool) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(if value {
            VisionMode::Always
        } else {
            VisionMode::Never
        })
    }

    fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        match value.trim().to_ascii_lowercase().as_str() {
            "auto" => Ok(VisionMode::Auto),
            "true" | "always" => Ok(VisionMode::Always),
            "false" | "never" => Ok(VisionMode::Never),
            _ => Err(E::custom("expected true, false, or \"auto\"")),
        }
    }

    fn visit_string<E>(self, value: String) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        self.visit_str(&value)
    }
}

impl JsonSchema for VisionMode {
    fn schema_name() -> String {
        "VisionMode".to_owned()
    }

    fn json_schema(_gen: &mut schemars::r#gen::SchemaGenerator) -> schemars::schema::Schema {
        serde_json::from_value(serde_json::json!({
            "oneOf": [
                { "type": "boolean" },
                {
                    "type": "string",
                    "enum": ["auto"]
                }
            ]
        }))
        .expect("valid VisionMode JSON schema")
    }
}

impl Default for AgentSettings {
    fn default() -> Self {
        Self {
            use_vision: default_use_vision(),
            vision_detail_level: default_vision_detail_level(),
            max_failures: default_max_failures(),
            generate_gif: GenerateGif::default(),
            max_actions_per_step: default_max_actions_per_step(),
            llm_timeout_seconds: default_llm_timeout_seconds(),
            step_timeout_seconds: default_step_timeout_seconds(),
            action_timeout_seconds: default_action_timeout_seconds(),
            directly_open_url: true,
            final_response_after_failure: default_final_response_after_failure(),
            display_files_in_done_text: default_display_files_in_done_text(),
            loop_detection_window: default_loop_detection_window(),
            loop_detection_enabled: default_loop_detection_enabled(),
            max_history_items: None,
            max_clickable_elements_length: default_max_clickable_elements_length(),
            include_recent_events: false,
            sample_images: Vec::new(),
            enable_planning: default_enable_planning(),
            planning_replan_on_stall: default_planning_replan_on_stall(),
            planning_exploration_limit: default_planning_exploration_limit(),
            use_thinking: default_use_thinking(),
            flash_mode: false,
            use_judge: default_use_judge(),
            ground_truth: None,
            message_compaction: MessageCompaction::default(),
            calculate_cost: false,
            include_tool_call_examples: false,
            save_conversation_path: None,
            save_conversation_path_encoding: default_save_conversation_path_encoding(),
            include_attributes: Vec::new(),
            available_file_paths: Vec::new(),
            initial_actions: Vec::new(),
            excluded_actions: Vec::new(),
            sensitive_data: BTreeMap::new(),
            override_system_message: None,
            extend_system_message: None,
        }
    }
}

impl AgentSettings {
    #[must_use]
    pub fn effective_action_timeout_seconds(&self) -> f64 {
        coerce_valid_action_timeout_seconds(self.action_timeout_seconds)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct MessageCompactionSettings {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_compact_every_n_steps")]
    pub compact_every_n_steps: usize,
    #[serde(
        default = "default_trigger_char_count",
        skip_serializing_if = "Option::is_none"
    )]
    pub trigger_char_count: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trigger_token_count: Option<usize>,
    #[serde(default = "default_chars_per_token")]
    pub chars_per_token: f64,
    #[serde(default = "default_keep_last_items")]
    pub keep_last_items: usize,
    #[serde(default = "default_summary_max_chars")]
    pub summary_max_chars: usize,
    #[serde(default)]
    pub include_read_state: bool,
}

impl Default for MessageCompactionSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            compact_every_n_steps: default_compact_every_n_steps(),
            trigger_char_count: default_trigger_char_count(),
            trigger_token_count: None,
            chars_per_token: default_chars_per_token(),
            keep_last_items: default_keep_last_items(),
            summary_max_chars: default_summary_max_chars(),
            include_read_state: false,
        }
    }
}

impl MessageCompactionSettings {
    #[must_use]
    pub fn effective_trigger_char_count(&self) -> usize {
        self.trigger_char_count.unwrap_or(40_000)
    }
}

impl<'de> Deserialize<'de> for MessageCompactionSettings {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct Wire {
            #[serde(default = "default_true")]
            enabled: bool,
            #[serde(default = "default_compact_every_n_steps")]
            compact_every_n_steps: usize,
            #[serde(default)]
            trigger_char_count: Option<usize>,
            #[serde(default)]
            trigger_token_count: Option<usize>,
            #[serde(default = "default_chars_per_token")]
            chars_per_token: f64,
            #[serde(default = "default_keep_last_items")]
            keep_last_items: usize,
            #[serde(default = "default_summary_max_chars")]
            summary_max_chars: usize,
            #[serde(default)]
            include_read_state: bool,
        }

        let wire = Wire::deserialize(deserializer)?;
        if wire.trigger_char_count.is_some() && wire.trigger_token_count.is_some() {
            return Err(de::Error::custom(
                "set trigger_char_count or trigger_token_count, not both",
            ));
        }
        let trigger_char_count = wire
            .trigger_char_count
            .or_else(|| {
                wire.trigger_token_count
                    .map(|tokens| (tokens as f64 * wire.chars_per_token).floor() as usize)
            })
            .or_else(default_trigger_char_count);

        Ok(Self {
            enabled: wire.enabled,
            compact_every_n_steps: wire.compact_every_n_steps,
            trigger_char_count,
            trigger_token_count: wire.trigger_token_count,
            chars_per_token: wire.chars_per_token,
            keep_last_items: wire.keep_last_items,
            summary_max_chars: wire.summary_max_chars,
            include_read_state: wire.include_read_state,
        })
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub enum MessageCompaction {
    Disabled,
    #[default]
    Enabled,
    Settings(MessageCompactionSettings),
}

impl MessageCompaction {
    #[must_use]
    pub fn is_enabled(&self) -> bool {
        match self {
            Self::Disabled => false,
            Self::Enabled => true,
            Self::Settings(settings) => settings.enabled,
        }
    }

    #[must_use]
    pub fn resolved_settings(&self) -> Option<MessageCompactionSettings> {
        match self {
            Self::Disabled => None,
            Self::Enabled => Some(MessageCompactionSettings::default()),
            Self::Settings(settings) if settings.enabled => Some(settings.clone()),
            Self::Settings(_) => None,
        }
    }
}

impl Serialize for MessageCompaction {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self {
            Self::Disabled => serializer.serialize_bool(false),
            Self::Enabled => serializer.serialize_bool(true),
            Self::Settings(settings) => settings.serialize(serializer),
        }
    }
}

impl<'de> Deserialize<'de> for MessageCompaction {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_any(MessageCompactionVisitor)
    }
}

struct MessageCompactionVisitor;

impl<'de> de::Visitor<'de> for MessageCompactionVisitor {
    type Value = MessageCompaction;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("true, false, null, or a MessageCompactionSettings object")
    }

    fn visit_bool<E>(self, value: bool) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(if value {
            MessageCompaction::Enabled
        } else {
            MessageCompaction::Disabled
        })
    }

    fn visit_none<E>(self) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(MessageCompaction::Disabled)
    }

    fn visit_unit<E>(self) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(MessageCompaction::Disabled)
    }

    fn visit_map<M>(self, map: M) -> Result<Self::Value, M::Error>
    where
        M: de::MapAccess<'de>,
    {
        let settings =
            MessageCompactionSettings::deserialize(de::value::MapAccessDeserializer::new(map))?;
        Ok(MessageCompaction::Settings(settings))
    }
}

impl JsonSchema for MessageCompaction {
    fn schema_name() -> String {
        "MessageCompaction".to_owned()
    }

    fn json_schema(r#gen: &mut schemars::r#gen::SchemaGenerator) -> schemars::schema::Schema {
        let settings_schema = r#gen.subschema_for::<MessageCompactionSettings>();
        serde_json::from_value(serde_json::json!({
            "oneOf": [
                { "type": "boolean" },
                { "type": "null" },
                settings_schema
            ]
        }))
        .expect("valid MessageCompaction JSON schema")
    }
}

fn is_default_message_compaction(value: &MessageCompaction) -> bool {
    matches!(value, MessageCompaction::Enabled)
}

fn default_true() -> bool {
    true
}

fn default_compact_every_n_steps() -> usize {
    25
}

fn default_trigger_char_count() -> Option<usize> {
    Some(40_000)
}

fn default_chars_per_token() -> f64 {
    4.0
}

fn default_keep_last_items() -> usize {
    6
}

fn default_summary_max_chars() -> usize {
    6_000
}

fn is_zero(value: &usize) -> bool {
    *value == 0
}

/// Upstream-compatible GIF generation setting.
///
/// Python browser-use accepts `False`, `True`, or a string output path. The
/// Rust runtime preserves that public shape even before GIF rendering side
/// effects are implemented.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum GenerateGif {
    #[default]
    Disabled,
    Enabled,
    Path(String),
}

impl Serialize for GenerateGif {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self {
            Self::Disabled => serializer.serialize_bool(false),
            Self::Enabled => serializer.serialize_bool(true),
            Self::Path(path) => serializer.serialize_str(path),
        }
    }
}

impl<'de> Deserialize<'de> for GenerateGif {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_any(GenerateGifVisitor)
    }
}

struct GenerateGifVisitor;

impl<'de> de::Visitor<'de> for GenerateGifVisitor {
    type Value = GenerateGif;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("true, false, or a GIF output path string")
    }

    fn visit_bool<E>(self, value: bool) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(if value {
            GenerateGif::Enabled
        } else {
            GenerateGif::Disabled
        })
    }

    fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(match value.trim().to_ascii_lowercase().as_str() {
            "true" => GenerateGif::Enabled,
            "false" => GenerateGif::Disabled,
            _ => GenerateGif::Path(value.to_owned()),
        })
    }

    fn visit_string<E>(self, value: String) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        self.visit_str(&value)
    }
}

impl JsonSchema for GenerateGif {
    fn schema_name() -> String {
        "GenerateGif".to_owned()
    }

    fn json_schema(_gen: &mut schemars::r#gen::SchemaGenerator) -> schemars::schema::Schema {
        serde_json::from_value(serde_json::json!({
            "oneOf": [
                { "type": "boolean" },
                { "type": "string" }
            ]
        }))
        .expect("valid GenerateGif JSON schema")
    }
}

fn default_use_vision() -> VisionMode {
    VisionMode::Always
}

fn default_vision_detail_level() -> ImageDetailLevel {
    ImageDetailLevel::Auto
}

fn default_max_failures() -> u32 {
    5
}

fn default_max_actions_per_step() -> usize {
    5
}

fn default_llm_timeout_seconds() -> u64 {
    60
}

fn default_step_timeout_seconds() -> u64 {
    180
}

fn default_action_timeout_seconds() -> f64 {
    parse_action_timeout_seconds(std::env::var(ACTION_TIMEOUT_ENV_VAR).ok().as_deref())
}

fn parse_action_timeout_seconds(raw: Option<&str>) -> f64 {
    let Some(raw) = raw else {
        return ACTION_TIMEOUT_FALLBACK_SECONDS;
    };
    if raw.is_empty() {
        return ACTION_TIMEOUT_FALLBACK_SECONDS;
    }
    let Ok(parsed) = raw.parse::<f64>() else {
        return ACTION_TIMEOUT_FALLBACK_SECONDS;
    };
    coerce_valid_action_timeout_seconds(parsed)
}

fn coerce_valid_action_timeout_seconds(value: f64) -> f64 {
    if value.is_finite() && value > 0.0 && Duration::try_from_secs_f64(value).is_ok() {
        value
    } else {
        ACTION_TIMEOUT_FALLBACK_SECONDS
    }
}

fn action_timeout_duration(seconds: f64) -> Duration {
    Duration::try_from_secs_f64(coerce_valid_action_timeout_seconds(seconds))
        .unwrap_or_else(|_| Duration::from_secs(ACTION_TIMEOUT_FALLBACK_SECONDS as u64))
}

fn timed_out_action_result(action: &BrowserAction, timeout_seconds: f64) -> ActionResult {
    ActionResult::error(format!(
        "Action {} timed out after {:.0}s. The browser may be unresponsive (dead CDP WebSocket). Try again or a different approach.",
        action.name(),
        coerce_valid_action_timeout_seconds(timeout_seconds)
    ))
}

fn default_final_response_after_failure() -> bool {
    true
}

fn default_display_files_in_done_text() -> bool {
    true
}

fn default_loop_detection_window() -> usize {
    20
}

fn default_loop_detection_enabled() -> bool {
    true
}

fn default_max_clickable_elements_length() -> usize {
    40_000
}

fn default_enable_planning() -> bool {
    true
}

fn default_planning_replan_on_stall() -> usize {
    3
}

fn default_planning_exploration_limit() -> usize {
    5
}

fn default_use_thinking() -> bool {
    true
}

fn default_use_judge() -> bool {
    true
}

fn default_save_conversation_path_encoding() -> Option<String> {
    Some("utf-8".to_owned())
}

fn is_default_save_conversation_path_encoding(value: &Option<String>) -> bool {
    value.as_deref() == Some("utf-8")
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum SensitiveDataValue {
    Value(String),
    Domain(BTreeMap<String, String>),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct AgentTask {
    pub id: Uuid,
    pub task: String,
    #[serde(default)]
    pub settings: AgentSettings,
}

impl AgentTask {
    #[must_use]
    pub fn new(task: impl Into<String>) -> Self {
        Self {
            id: Uuid::now_v7(),
            task: task.into(),
            settings: AgentSettings::default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct AgentOutput {
    #[serde(default, skip_serializing_if = "AgentCurrentState::is_empty")]
    pub current_state: AgentCurrentState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thinking: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evaluation_previous_goal: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memory: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_goal: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_plan_item: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan_update: Option<Vec<String>>,
    pub action: Vec<BrowserAction>,
}

impl AgentOutput {
    #[must_use]
    pub fn current_brain(&self) -> AgentCurrentState {
        AgentCurrentState {
            thinking: self
                .thinking
                .clone()
                .or_else(|| self.current_state.thinking.clone()),
            evaluation_previous_goal: self
                .evaluation_previous_goal
                .clone()
                .or_else(|| self.current_state.evaluation_previous_goal.clone()),
            memory: self
                .memory
                .clone()
                .or_else(|| self.current_state.memory.clone()),
            next_goal: self
                .next_goal
                .clone()
                .or_else(|| self.current_state.next_goal.clone()),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct AgentCurrentState {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thinking: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evaluation_previous_goal: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memory: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_goal: Option<String>,
}

impl AgentCurrentState {
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.thinking.is_none()
            && self.evaluation_previous_goal.is_none()
            && self.memory.is_none()
            && self.next_goal.is_none()
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct JudgementResult {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<String>,
    pub verdict: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure_reason: Option<String>,
    #[serde(default)]
    pub impossible_task: bool,
    #[serde(default)]
    pub reached_captcha: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct MessageCompactionOutput {
    pub summary: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct ActionResult {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extracted_content: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub judgement: Option<JudgementResult>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub long_term_memory: Option<String>,
    #[serde(default)]
    pub include_extracted_content_only_once: bool,
    #[serde(default)]
    pub include_in_memory: bool,
    #[serde(default)]
    pub is_done: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub success: Option<bool>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attachments: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub images: Vec<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<Value>,
}

#[derive(Deserialize)]
struct ActionResultWire {
    #[serde(default)]
    extracted_content: Option<String>,
    #[serde(default)]
    error: Option<String>,
    #[serde(default)]
    judgement: Option<JudgementResult>,
    #[serde(default)]
    long_term_memory: Option<String>,
    #[serde(default)]
    include_extracted_content_only_once: bool,
    #[serde(default)]
    include_in_memory: bool,
    #[serde(default)]
    is_done: bool,
    #[serde(default)]
    success: Option<bool>,
    #[serde(default)]
    attachments: Vec<String>,
    #[serde(default)]
    images: Vec<Value>,
    #[serde(default)]
    metadata: Option<Value>,
}

impl<'de> Deserialize<'de> for ActionResult {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let wire = ActionResultWire::deserialize(deserializer)?;
        if wire.success == Some(true) && !wire.is_done {
            return Err(serde::de::Error::custom(
                "success=true can only be set when is_done=true",
            ));
        }

        Ok(Self {
            extracted_content: wire.extracted_content,
            error: wire.error,
            judgement: wire.judgement,
            long_term_memory: wire.long_term_memory,
            include_extracted_content_only_once: wire.include_extracted_content_only_once,
            include_in_memory: wire.include_in_memory,
            is_done: wire.is_done,
            success: wire.success,
            attachments: wire.attachments,
            images: wire.images,
            metadata: wire.metadata,
        })
    }
}

impl ActionResult {
    #[must_use]
    pub fn extracted(text: impl Into<String>) -> Self {
        let text = text.into();
        Self {
            extracted_content: Some(text.clone()),
            error: None,
            judgement: None,
            long_term_memory: Some(text),
            include_extracted_content_only_once: false,
            include_in_memory: false,
            is_done: false,
            success: None,
            attachments: Vec::new(),
            images: Vec::new(),
            metadata: None,
        }
    }

    #[must_use]
    pub fn error(error: impl Into<String>) -> Self {
        Self {
            extracted_content: None,
            error: Some(error.into()),
            judgement: None,
            long_term_memory: None,
            include_extracted_content_only_once: false,
            include_in_memory: true,
            is_done: false,
            success: None,
            attachments: Vec::new(),
            images: Vec::new(),
            metadata: None,
        }
    }

    #[must_use]
    pub fn done(text: impl Into<String>, success: bool) -> Self {
        Self {
            extracted_content: Some(text.into()),
            error: None,
            judgement: None,
            long_term_memory: None,
            include_extracted_content_only_once: false,
            include_in_memory: true,
            is_done: true,
            success: Some(success),
            attachments: Vec::new(),
            images: Vec::new(),
            metadata: None,
        }
    }

    #[must_use]
    pub fn done_with_attachments(
        text: impl Into<String>,
        success: bool,
        attachments: Vec<String>,
    ) -> Self {
        Self {
            attachments,
            ..Self::done(text, success)
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct AgentHistoryItem {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_output: Option<AgentOutput>,
    pub result: Vec<ActionResult>,
    pub state: BrowserStateSummary,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<StepMetadata>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct StepMetadata {
    pub step_start_time: f64,
    pub step_end_time: f64,
    pub step_number: usize,
    #[serde(default)]
    pub step_interval: Option<f64>,
}

impl StepMetadata {
    #[must_use]
    pub fn duration_seconds(&self) -> f64 {
        self.step_end_time - self.step_start_time
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct AgentHistory {
    #[serde(default)]
    pub items: Vec<AgentHistoryItem>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compacted_memory: Option<String>,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub compaction_count: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_compaction_step: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ActionReplayRematch {
    pub action: BrowserAction,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub original_index: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rematched_index: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub match_result: Option<DomInteractedElementMatch>,
    #[serde(default)]
    pub changed: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct AgentHistoryReplayPlan {
    #[serde(default)]
    pub actions: Vec<AgentHistoryReplayPlanItem>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct AgentHistoryReplayPlanItem {
    pub step_index: usize,
    pub action_index: usize,
    pub original_action: BrowserAction,
    pub remapped_action: BrowserAction,
    pub rematch: ActionReplayRematch,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct AgentHistoryReplayPlanError {
    pub step_index: usize,
    pub action_index: usize,
    pub original_action: Box<BrowserAction>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub original_index: Option<u32>,
    pub failure: Box<DomInteractedElementMatchFailure>,
}

impl std::fmt::Display for AgentHistoryReplayPlanError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "failed to rematch replay action {} in step {}: {}",
            self.action_index, self.step_index, self.failure.message
        )
    }
}

impl std::error::Error for AgentHistoryReplayPlanError {}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct AgentHistoryReplayExecution {
    pub items: Vec<AgentHistoryReplayExecutionItem>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stop: Option<AgentHistoryReplayStop>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct AgentHistoryReplayExecutionItem {
    pub step_index: usize,
    pub action_index: usize,
    pub original_action: BrowserAction,
    pub executed_action: BrowserAction,
    pub rematch: ActionReplayRematch,
    pub result: ActionResult,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct AgentHistoryReplayRun {
    pub current_state: BrowserStateSummary,
    pub plan: AgentHistoryReplayPlan,
    pub execution: AgentHistoryReplayExecution,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AgentHistoryReplayRunError {
    CurrentState {
        message: String,
    },
    Plan {
        error: Box<AgentHistoryReplayPlanError>,
    },
}

impl std::fmt::Display for AgentHistoryReplayRunError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::CurrentState { message } => {
                write!(
                    formatter,
                    "failed to capture current browser state: {message}"
                )
            }
            Self::Plan { error } => write!(formatter, "{error}"),
        }
    }
}

impl std::error::Error for AgentHistoryReplayRunError {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct AgentHistoryReplayStop {
    pub step_index: usize,
    pub action_index: usize,
    pub reason: AgentHistoryReplayStopReason,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diagnostic: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AgentHistoryReplayStopReason {
    DoneAfterPriorAction,
    Done,
    Error,
    PageChanged,
    TerminatingAction,
}

#[derive(Debug, Clone)]
struct HistoricalReplayAction {
    step_index: usize,
    action_index: usize,
    action: BrowserAction,
    interacted_element: Option<DomInteractedElement>,
}

fn historical_replay_actions(history: &AgentHistory) -> Vec<HistoricalReplayAction> {
    let mut actions = Vec::new();
    for (step_index, item) in history.items.iter().enumerate() {
        let Some(output) = item.model_output.as_ref() else {
            continue;
        };
        for (action_index, action) in output.action.iter().enumerate() {
            let interacted_element = action
                .interacted_element_index()
                .and_then(|index| item.state.dom_state.selector_map.get(&index))
                .map(DomInteractedElement::from_element);
            actions.push(HistoricalReplayAction {
                step_index,
                action_index,
                action: action.clone(),
                interacted_element,
            });
        }
    }
    actions
}

pub fn build_history_replay_plan(
    history: &AgentHistory,
    current_dom: &SerializedDomState,
) -> Result<AgentHistoryReplayPlan, AgentHistoryReplayPlanError> {
    let mut actions = Vec::new();
    for historical in historical_replay_actions(history) {
        let rematch = rematch_action_for_replay(
            &historical.action,
            historical.interacted_element.as_ref(),
            current_dom,
        )
        .map_err(|failure| AgentHistoryReplayPlanError {
            step_index: historical.step_index,
            action_index: historical.action_index,
            original_action: Box::new(historical.action.clone()),
            original_index: historical.action.interacted_element_index(),
            failure: Box::new(failure),
        })?;
        actions.push(AgentHistoryReplayPlanItem {
            step_index: historical.step_index,
            action_index: historical.action_index,
            original_action: historical.action,
            remapped_action: rematch.action.clone(),
            rematch,
        });
    }

    Ok(AgentHistoryReplayPlan { actions })
}

pub fn rematch_action_for_replay(
    action: &BrowserAction,
    interacted_element: Option<&DomInteractedElement>,
    current_dom: &SerializedDomState,
) -> Result<ActionReplayRematch, DomInteractedElementMatchFailure> {
    let Some(original_index) = action.interacted_element_index() else {
        return Ok(ActionReplayRematch {
            action: action.clone(),
            original_index: None,
            rematched_index: None,
            match_result: None,
            changed: false,
        });
    };
    let Some(interacted_element) = interacted_element else {
        return Ok(ActionReplayRematch {
            action: action.clone(),
            original_index: Some(original_index),
            rematched_index: None,
            match_result: None,
            changed: false,
        });
    };

    let match_result = interacted_element.rematch(current_dom)?;
    let rematched_index = match_result.index;
    let changed = rematched_index != original_index;
    Ok(ActionReplayRematch {
        action: action_with_interacted_index(action, rematched_index),
        original_index: Some(original_index),
        rematched_index: Some(rematched_index),
        match_result: Some(match_result),
        changed,
    })
}

impl AgentHistory {
    #[must_use]
    pub fn final_result(&self) -> Option<&str> {
        self.last_result()
            .and_then(|result| result.extracted_content.as_deref())
    }

    #[must_use]
    pub fn is_done(&self) -> bool {
        self.last_result().is_some_and(|result| result.is_done)
    }

    #[must_use]
    pub fn is_successful(&self) -> Option<bool> {
        self.last_result()
            .filter(|result| result.is_done)
            .and_then(|result| result.success)
    }

    #[must_use]
    pub fn errors(&self) -> Vec<Option<&str>> {
        self.items
            .iter()
            .map(|item| {
                item.result
                    .iter()
                    .find_map(|result| result.error.as_deref())
            })
            .collect()
    }

    #[must_use]
    pub fn has_errors(&self) -> bool {
        self.errors().iter().any(Option::is_some)
    }

    #[must_use]
    pub fn judgement(&self) -> Option<&JudgementResult> {
        self.last_result()
            .and_then(|result| result.judgement.as_ref())
    }

    #[must_use]
    pub fn is_judged(&self) -> bool {
        self.judgement().is_some()
    }

    #[must_use]
    pub fn is_validated(&self) -> Option<bool> {
        self.judgement().map(|judgement| judgement.verdict)
    }

    #[must_use]
    pub fn total_duration_seconds(&self) -> f64 {
        self.items
            .iter()
            .filter_map(|item| item.metadata.as_ref())
            .map(StepMetadata::duration_seconds)
            .sum()
    }

    #[must_use]
    pub fn number_of_steps(&self) -> usize {
        self.items.len()
    }

    #[must_use]
    pub fn urls(&self) -> Vec<&str> {
        self.items
            .iter()
            .map(|item| item.state.url.as_str())
            .collect()
    }

    #[must_use]
    pub fn screenshots(
        &self,
        n_last: Option<usize>,
        return_none_if_not_screenshot: bool,
    ) -> Vec<Option<&str>> {
        if n_last == Some(0) {
            return Vec::new();
        }

        let items = if let Some(n_last) = n_last {
            let start = self.items.len().saturating_sub(n_last);
            &self.items[start..]
        } else {
            &self.items
        };

        items
            .iter()
            .filter_map(|item| match item.state.screenshot.as_deref() {
                Some(screenshot) if !screenshot.is_empty() => Some(Some(screenshot)),
                _ if return_none_if_not_screenshot => Some(None),
                _ => None,
            })
            .collect()
    }

    #[must_use]
    pub fn action_results(&self) -> Vec<&ActionResult> {
        self.items
            .iter()
            .flat_map(|item| item.result.iter())
            .collect()
    }

    #[must_use]
    pub fn extracted_content(&self) -> Vec<&str> {
        self.action_results()
            .into_iter()
            .filter_map(|result| result.extracted_content.as_deref())
            .collect()
    }

    #[must_use]
    pub fn last_action(&self) -> Option<Value> {
        self.items
            .last()
            .and_then(|item| item.model_output.as_ref())
            .and_then(|output| output.action.last())
            .and_then(|action| serde_json::to_value(action).ok())
    }

    #[must_use]
    pub fn model_outputs(&self) -> Vec<&AgentOutput> {
        self.items
            .iter()
            .filter_map(|item| item.model_output.as_ref())
            .collect()
    }

    #[must_use]
    pub fn model_thoughts(&self) -> Vec<AgentCurrentState> {
        self.model_outputs()
            .into_iter()
            .map(AgentOutput::current_brain)
            .collect()
    }

    #[must_use]
    pub fn model_actions(&self) -> Vec<Value> {
        self.items
            .iter()
            .flat_map(|item| {
                item.model_output
                    .as_ref()
                    .map(|output| {
                        output
                            .action
                            .iter()
                            .filter_map(|action| {
                                action_value_with_interacted_element(action, &item.state)
                            })
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default()
            })
            .collect()
    }

    #[must_use]
    pub fn action_history(&self) -> Vec<Vec<Value>> {
        self.items
            .iter()
            .map(|item| {
                let Some(output) = item.model_output.as_ref() else {
                    return Vec::new();
                };

                output
                    .action
                    .iter()
                    .zip(item.result.iter())
                    .filter_map(|(action, result)| {
                        let mut action_output =
                            action_value_with_interacted_element(action, &item.state)?;
                        if let Value::Object(attributes) = &mut action_output {
                            attributes.insert(
                                "result".to_owned(),
                                result
                                    .long_term_memory
                                    .as_ref()
                                    .map(|memory| Value::String(memory.clone()))
                                    .unwrap_or(Value::Null),
                            );
                        }
                        Some(action_output)
                    })
                    .collect()
            })
            .collect()
    }

    pub fn replay_plan(
        &self,
        current_dom: &SerializedDomState,
    ) -> Result<AgentHistoryReplayPlan, AgentHistoryReplayPlanError> {
        build_history_replay_plan(self, current_dom)
    }

    #[must_use]
    pub fn model_actions_filtered(&self, include: &[&str]) -> Vec<Value> {
        self.items
            .iter()
            .filter_map(|item| item.model_output.as_ref())
            .flat_map(|output| output.action.iter())
            .filter(|action| include.contains(&action.name()))
            .filter_map(|action| serde_json::to_value(action).ok())
            .collect()
    }

    #[must_use]
    pub fn action_names(&self) -> Vec<&'static str> {
        self.items
            .iter()
            .filter_map(|item| item.model_output.as_ref())
            .flat_map(|output| output.action.iter())
            .map(BrowserAction::name)
            .collect()
    }

    fn last_result(&self) -> Option<&ActionResult> {
        self.items.last().and_then(|item| item.result.last())
    }
}

fn action_value_with_interacted_element(
    action: &BrowserAction,
    state: &BrowserStateSummary,
) -> Option<Value> {
    let mut action_output = serde_json::to_value(action).ok()?;
    if let Value::Object(attributes) = &mut action_output {
        let interacted_element = action
            .interacted_element_index()
            .and_then(|index| state.dom_state.selector_map.get(&index))
            .map(DomInteractedElement::from_element)
            .and_then(|element| serde_json::to_value(element).ok())
            .unwrap_or(Value::Null);
        attributes.insert("interacted_element".to_owned(), interacted_element);
    }
    Some(action_output)
}

fn action_with_interacted_index(action: &BrowserAction, index: u32) -> BrowserAction {
    action
        .with_interacted_element_index(index)
        .unwrap_or_else(|| action.clone())
}

#[async_trait]
pub trait ActionExecutor {
    async fn execute(&mut self, action: &BrowserAction) -> ActionResult;
}

pub struct BrowserActionExecutor<S> {
    session: S,
    file_system: ManagedFileSystem,
    display_files_in_done_text: bool,
    action_timeout_seconds: f64,
    enforce_upload_file_availability: bool,
    available_file_paths: BTreeSet<String>,
}

impl<S> BrowserActionExecutor<S> {
    #[must_use]
    pub fn new(session: S) -> Self {
        Self::with_file_system(
            session,
            ManagedFileSystem::new_in_temp().expect("create managed file system"),
        )
    }

    #[must_use]
    pub fn with_file_system(session: S, file_system: ManagedFileSystem) -> Self {
        Self {
            session,
            file_system,
            display_files_in_done_text: true,
            action_timeout_seconds: default_action_timeout_seconds(),
            enforce_upload_file_availability: false,
            available_file_paths: BTreeSet::new(),
        }
    }

    #[must_use]
    pub fn session(&self) -> &S {
        &self.session
    }

    #[must_use]
    pub fn file_system(&self) -> &ManagedFileSystem {
        &self.file_system
    }

    pub fn file_system_mut(&mut self) -> &mut ManagedFileSystem {
        &mut self.file_system
    }

    pub fn set_display_files_in_done_text(&mut self, display_files_in_done_text: bool) {
        self.display_files_in_done_text = display_files_in_done_text;
    }

    pub fn set_action_timeout_seconds(&mut self, action_timeout_seconds: f64) {
        self.action_timeout_seconds = coerce_valid_action_timeout_seconds(action_timeout_seconds);
    }

    #[must_use]
    pub fn action_timeout_seconds(&self) -> f64 {
        coerce_valid_action_timeout_seconds(self.action_timeout_seconds)
    }

    pub fn set_upload_file_availability(
        &mut self,
        enforce_upload_file_availability: bool,
        available_file_paths: Vec<String>,
    ) {
        self.enforce_upload_file_availability = enforce_upload_file_availability;
        self.available_file_paths = available_file_paths.into_iter().collect();
    }
}

impl<S> BrowserActionExecutor<S>
where
    S: BrowserSession + Send + Sync,
{
    async fn execute_for_agent(&mut self, action: &BrowserAction) -> ActionResult {
        let timeout_seconds = self.action_timeout_seconds();
        match timeout(
            action_timeout_duration(timeout_seconds),
            execute_browser_action(
                &self.session,
                &mut self.file_system,
                action,
                self.display_files_in_done_text,
                self.enforce_upload_file_availability,
                &self.available_file_paths,
                false,
            ),
        )
        .await
        {
            Ok(Ok(result)) => result,
            Ok(Err(error)) => ActionResult::error(error.to_string()),
            Err(_) => timed_out_action_result(action, timeout_seconds),
        }
    }

    pub async fn execute_sequence(&mut self, actions: &[BrowserAction]) -> Vec<ActionResult> {
        let mut results = Vec::new();

        for (index, action) in actions.iter().enumerate() {
            if index > 0 && matches!(action, BrowserAction::Done(_)) {
                break;
            }

            let needs_page_change_guard = !action.terminates_sequence();
            let before = if needs_page_change_guard {
                match self.session.state(false).await {
                    Ok(state) => Some(state),
                    Err(error) => {
                        results.push(ActionResult::error(error.to_string()));
                        break;
                    }
                }
            } else {
                None
            };
            let result = self.execute(action).await;
            let should_stop =
                result.is_done || result.error.is_some() || action.terminates_sequence();
            let page_changed = if should_stop {
                false
            } else if let Some(before) = before {
                match self.session.state(false).await {
                    Ok(after) => after.url != before.url,
                    Err(error) => {
                        results.push(result);
                        results.push(ActionResult::error(error.to_string()));
                        break;
                    }
                }
            } else {
                false
            };

            results.push(result);

            if should_stop || page_changed {
                break;
            }
        }

        results
    }

    pub async fn execute_replay_plan(
        &mut self,
        plan: &AgentHistoryReplayPlan,
    ) -> AgentHistoryReplayExecution {
        let mut items = Vec::new();
        let mut stop = None;

        for (plan_index, item) in plan.actions.iter().enumerate() {
            let action = &item.remapped_action;
            if plan_index > 0 && matches!(action, BrowserAction::Done(_)) {
                stop = Some(AgentHistoryReplayStop {
                    step_index: item.step_index,
                    action_index: item.action_index,
                    reason: AgentHistoryReplayStopReason::DoneAfterPriorAction,
                    diagnostic: None,
                });
                break;
            }

            let needs_page_change_guard = !action.terminates_sequence();
            let before = if needs_page_change_guard {
                match self.session.state(false).await {
                    Ok(state) => Some(state),
                    Err(error) => {
                        let result = ActionResult::error(error.to_string());
                        stop = Some(AgentHistoryReplayStop {
                            step_index: item.step_index,
                            action_index: item.action_index,
                            reason: AgentHistoryReplayStopReason::Error,
                            diagnostic: result.error.clone(),
                        });
                        items.push(AgentHistoryReplayExecutionItem {
                            step_index: item.step_index,
                            action_index: item.action_index,
                            original_action: item.original_action.clone(),
                            executed_action: action.clone(),
                            rematch: item.rematch.clone(),
                            result,
                        });
                        break;
                    }
                }
            } else {
                None
            };

            let result = self.execute(action).await;
            let mut stop_reason = replay_stop_reason(action, &result);
            let mut stop_diagnostic = result.error.clone();
            let mut page_changed = false;
            if stop_reason.is_none() {
                if let Some(before) = before {
                    match self.session.state(false).await {
                        Ok(after) => page_changed = after.url != before.url,
                        Err(error) => {
                            stop_reason = Some(AgentHistoryReplayStopReason::Error);
                            stop_diagnostic = Some(error.to_string());
                        }
                    }
                }
            }

            items.push(AgentHistoryReplayExecutionItem {
                step_index: item.step_index,
                action_index: item.action_index,
                original_action: item.original_action.clone(),
                executed_action: action.clone(),
                rematch: item.rematch.clone(),
                result,
            });

            if let Some(reason) = stop_reason {
                stop = Some(AgentHistoryReplayStop {
                    step_index: item.step_index,
                    action_index: item.action_index,
                    reason,
                    diagnostic: stop_diagnostic,
                });
                break;
            }

            if page_changed {
                stop = Some(AgentHistoryReplayStop {
                    step_index: item.step_index,
                    action_index: item.action_index,
                    reason: AgentHistoryReplayStopReason::PageChanged,
                    diagnostic: None,
                });
                break;
            }
        }

        AgentHistoryReplayExecution { items, stop }
    }

    pub async fn replay_history(
        &mut self,
        history: &AgentHistory,
    ) -> Result<AgentHistoryReplayRun, AgentHistoryReplayRunError> {
        let current_state = self.session.state(false).await.map_err(|error| {
            AgentHistoryReplayRunError::CurrentState {
                message: error.to_string(),
            }
        })?;
        let (plan, execution) = self
            .execute_history_replay_with_recapture(history, current_state.clone())
            .await?;
        Ok(AgentHistoryReplayRun {
            current_state,
            plan,
            execution,
        })
    }

    async fn execute_history_replay_with_recapture(
        &mut self,
        history: &AgentHistory,
        mut latest_state: BrowserStateSummary,
    ) -> Result<(AgentHistoryReplayPlan, AgentHistoryReplayExecution), AgentHistoryReplayRunError>
    {
        let mut plan_items = Vec::new();
        let mut execution_items = Vec::new();
        let mut stop = None;

        for historical in historical_replay_actions(history) {
            if !plan_items.is_empty() && matches!(historical.action, BrowserAction::Done(_)) {
                stop = Some(AgentHistoryReplayStop {
                    step_index: historical.step_index,
                    action_index: historical.action_index,
                    reason: AgentHistoryReplayStopReason::DoneAfterPriorAction,
                    diagnostic: None,
                });
                break;
            }

            let rematch = rematch_action_for_replay(
                &historical.action,
                historical.interacted_element.as_ref(),
                &latest_state.dom_state,
            )
            .map_err(|failure| AgentHistoryReplayRunError::Plan {
                error: Box::new(AgentHistoryReplayPlanError {
                    step_index: historical.step_index,
                    action_index: historical.action_index,
                    original_action: Box::new(historical.action.clone()),
                    original_index: historical.action.interacted_element_index(),
                    failure: Box::new(failure),
                }),
            })?;
            let action = rematch.action.clone();
            let plan_item = AgentHistoryReplayPlanItem {
                step_index: historical.step_index,
                action_index: historical.action_index,
                original_action: historical.action.clone(),
                remapped_action: action.clone(),
                rematch,
            };
            plan_items.push(plan_item.clone());

            let needs_recapture = !action.terminates_sequence();
            let result = self.execute(&action).await;
            let mut stop_reason = replay_stop_reason(&action, &result);
            let mut stop_diagnostic = result.error.clone();
            let mut recaptured_state = None;
            if stop_reason.is_none() && needs_recapture {
                match self.session.state(false).await {
                    Ok(state) => recaptured_state = Some(state),
                    Err(error) => {
                        stop_reason = Some(AgentHistoryReplayStopReason::Error);
                        stop_diagnostic = Some(error.to_string());
                    }
                }
            }

            execution_items.push(AgentHistoryReplayExecutionItem {
                step_index: historical.step_index,
                action_index: historical.action_index,
                original_action: historical.action,
                executed_action: action,
                rematch: plan_item.rematch,
                result,
            });

            if let Some(reason) = stop_reason {
                stop = Some(AgentHistoryReplayStop {
                    step_index: historical.step_index,
                    action_index: historical.action_index,
                    reason,
                    diagnostic: stop_diagnostic,
                });
                break;
            }

            if let Some(state) = recaptured_state {
                latest_state = state;
            }
        }

        Ok((
            AgentHistoryReplayPlan {
                actions: plan_items,
            },
            AgentHistoryReplayExecution {
                items: execution_items,
                stop,
            },
        ))
    }
}

#[async_trait]
impl<S> ActionExecutor for BrowserActionExecutor<S>
where
    S: BrowserSession + Send + Sync,
{
    async fn execute(&mut self, action: &BrowserAction) -> ActionResult {
        let timeout_seconds = self.action_timeout_seconds();
        match timeout(
            action_timeout_duration(timeout_seconds),
            execute_browser_action(
                &self.session,
                &mut self.file_system,
                action,
                self.display_files_in_done_text,
                self.enforce_upload_file_availability,
                &self.available_file_paths,
                true,
            ),
        )
        .await
        {
            Ok(Ok(result)) => result,
            Ok(Err(error)) => ActionResult::error(error.to_string()),
            Err(_) => timed_out_action_result(action, timeout_seconds),
        }
    }
}

async fn execute_browser_action<S>(
    session: &S,
    file_system: &mut ManagedFileSystem,
    action: &BrowserAction,
    display_files_in_done_text: bool,
    enforce_upload_file_availability: bool,
    available_file_paths: &BTreeSet<String>,
    save_extract_envelope_to_file_system: bool,
) -> Result<ActionResult, BrowserError>
where
    S: BrowserSession + Send + Sync,
{
    match action {
        BrowserAction::Search(params) => {
            let url = search_url(&params.engine, &params.query);
            session.navigate(&url, false).await?;
            Ok(ActionResult::extracted(format!(
                "Searched {:?} for '{}'",
                params.engine, params.query
            )))
        }
        BrowserAction::Navigate(params) => {
            session.navigate(&params.url, params.new_tab).await?;
            Ok(ActionResult::extracted(format!(
                "Navigated to {}",
                params.url
            )))
        }
        BrowserAction::GoBack(_) => {
            session.go_back().await?;
            Ok(ActionResult::extracted("Navigated back"))
        }
        BrowserAction::SwitchTab(params) => {
            session.switch_tab(&params.tab_id).await?;
            Ok(ActionResult::extracted(format!(
                "Switched to tab {}",
                params.tab_id
            )))
        }
        BrowserAction::CloseTab(params) => {
            session.close_tab(&params.tab_id).await?;
            Ok(ActionResult::extracted(format!(
                "Closed tab {}",
                params.tab_id
            )))
        }
        BrowserAction::Click(params) => {
            match (params.index, params.coordinate_x, params.coordinate_y) {
                (Some(0), _, _) => Ok(ActionResult::error(
                    "Cannot click on element with index 0. Use a positive browser_state index.",
                )),
                (Some(index), _, _) => match session.click(index).await {
                    Ok(()) => Ok(ActionResult::extracted(format!("Clicked element {index}"))),
                    Err(error) if is_select_click_validation_error(&error) => {
                        match session.dropdown_options(index).await {
                            Ok(options) => Ok(ActionResult::extracted(format!(
                                "Dropdown options: {}",
                                options.join(", ")
                            ))),
                            Err(_) => Ok(ActionResult::error(error.to_string())),
                        }
                    }
                    Err(error) if is_file_input_click_validation_error(&error) => {
                        Ok(ActionResult::error(
                            "Cannot click on file input elements. Use upload_file with the same index instead.",
                        ))
                    }
                    Err(error) => Err(error),
                },
                (None, Some(x), Some(y)) => {
                    session.click_coordinates(x, y).await?;
                    Ok(ActionResult::extracted(format!(
                        "Clicked coordinates ({x}, {y})"
                    )))
                }
                _ => Ok(ActionResult::error(
                    "click requires either an element index or both coordinate_x and coordinate_y",
                )),
            }
        }
        BrowserAction::Input(params) => {
            session
                .input_text(params.index, &params.text, params.clear)
                .await?;
            Ok(ActionResult::extracted(format!(
                "Typed text into element {}",
                params.index
            )))
        }
        BrowserAction::Scroll(params) => {
            let index = params.index.filter(|index| *index != 0);
            session.scroll(index, params.down, params.pages).await?;
            Ok(ActionResult::extracted("Scrolled page"))
        }
        BrowserAction::FindText(params) => {
            if session.find_text(&params.text).await? {
                Ok(ActionResult::extracted(format!(
                    "Scrolled to text: {}",
                    params.text
                )))
            } else {
                Ok(ActionResult {
                    extracted_content: Some(format!(
                        "Text '{}' not found or not visible on page",
                        params.text
                    )),
                    error: None,
                    judgement: None,
                    long_term_memory: Some(format!(
                        "Tried scrolling to text '{}' but it was not found",
                        params.text
                    )),
                    include_extracted_content_only_once: false,
                    include_in_memory: false,
                    is_done: false,
                    success: None,
                    attachments: Vec::new(),
                    images: Vec::new(),
                    metadata: None,
                })
            }
        }
        BrowserAction::Evaluate(params) => match session.evaluate(&params.code).await {
            Ok(result_text) => {
                let result_text = truncate_evaluate_result(result_text);
                let long_term_memory = if result_text.chars().count() < 10_000 {
                    result_text.clone()
                } else {
                    format!(
                        "JavaScript executed successfully, result length: {} characters.",
                        result_text.chars().count()
                    )
                };
                Ok(ActionResult {
                    include_extracted_content_only_once: long_term_memory != result_text,
                    extracted_content: Some(result_text),
                    error: None,
                    judgement: None,
                    long_term_memory: Some(long_term_memory),
                    include_in_memory: true,
                    is_done: false,
                    success: None,
                    attachments: Vec::new(),
                    images: Vec::new(),
                    metadata: None,
                })
            }
            Err(error) => Ok(ActionResult::error(format!(
                "Failed to execute JavaScript: {error}"
            ))),
        },
        BrowserAction::Wait(params) => {
            let actual_seconds = params.seconds.saturating_sub(1).clamp(0, 30) as u64;
            sleep(Duration::from_secs(actual_seconds)).await;
            Ok(ActionResult::extracted(format!(
                "Waited for {} seconds",
                params.seconds
            )))
        }
        BrowserAction::Screenshot(params) => {
            if let Some(file_name) = params.file_name.as_deref() {
                let screenshot = session.screenshot().await?;
                let output_path = screenshot_output_path(file_name);
                if let Some(parent) = output_path.parent()
                    && !parent.as_os_str().is_empty()
                {
                    std::fs::create_dir_all(parent)
                        .map_err(|error| BrowserError::ActionFailed(error.to_string()))?;
                }
                let bytes = base64::engine::general_purpose::STANDARD
                    .decode(&screenshot.base64_png)
                    .map_err(|error| BrowserError::ActionFailed(error.to_string()))?;
                std::fs::write(&output_path, bytes)
                    .map_err(|error| BrowserError::ActionFailed(error.to_string()))?;
                let file_name = output_path.display().to_string();
                Ok(ActionResult {
                    extracted_content: Some(format!("Screenshot saved to {file_name}")),
                    error: None,
                    judgement: None,
                    long_term_memory: Some(format!("Screenshot saved to {file_name}")),
                    include_extracted_content_only_once: true,
                    include_in_memory: true,
                    is_done: false,
                    success: None,
                    attachments: vec![file_name],
                    images: Vec::new(),
                    metadata: None,
                })
            } else {
                Ok(ActionResult {
                    extracted_content: Some("Requested screenshot for next observation".to_owned()),
                    error: None,
                    judgement: None,
                    long_term_memory: None,
                    include_extracted_content_only_once: false,
                    include_in_memory: false,
                    is_done: false,
                    success: None,
                    attachments: Vec::new(),
                    images: Vec::new(),
                    metadata: Some(serde_json::json!({ "include_screenshot": true })),
                })
            }
        }
        BrowserAction::Done(params) => Ok(done_action_result(
            params,
            Some(file_system),
            display_files_in_done_text,
        )),
        BrowserAction::Extract(params) => {
            let text = session.page_text().await?;
            let source_url = session.state(false).await.ok().map(|state| state.url);
            let extract_images = should_extract_images(&params.query, params.extract_images);
            let links = if params.extract_links {
                Some(
                    session
                        .find_elements(
                            "a[href]",
                            &extract_link_attributes(),
                            MAX_EXTRACT_RELATED_ELEMENTS,
                            true,
                        )
                        .await?,
                )
            } else {
                None
            };
            let images = if extract_images {
                Some(
                    session
                        .find_elements(
                            "img[src], img[data-src], picture source[srcset]",
                            &extract_image_attributes(),
                            MAX_EXTRACT_RELATED_ELEMENTS,
                            false,
                        )
                        .await?,
                )
            } else {
                None
            };
            Ok(extract_action_result(
                params,
                &text,
                source_url.as_deref(),
                extract_images,
                links.as_deref(),
                images.as_deref(),
                save_extract_envelope_to_file_system.then_some(file_system),
            ))
        }
        BrowserAction::SearchPage(params) => {
            let text = if let Some(scope) = &params.css_scope {
                session
                    .find_elements(scope, &[], params.max_results.max(1), true)
                    .await?
                    .into_iter()
                    .filter_map(|element| element.text)
                    .collect::<Vec<_>>()
                    .join("\n")
            } else {
                session.page_text().await?
            };
            let matches = search_text_matches(
                &text,
                &params.pattern,
                params.regex,
                params.case_sensitive,
                params.context_chars,
                params.max_results,
            )
            .map_err(BrowserError::ActionFailed)?;

            Ok(ActionResult::extracted(format_search_page_results(
                &params.pattern,
                &matches,
            )))
        }
        BrowserAction::FindElements(params) => {
            let attributes = params
                .attributes
                .clone()
                .unwrap_or_else(default_find_element_attributes);
            let elements = session
                .find_elements(
                    &params.selector,
                    &attributes,
                    params.max_results,
                    params.include_text,
                )
                .await?;
            Ok(ActionResult::extracted(format_find_elements_results(
                &params.selector,
                &elements,
            )))
        }
        BrowserAction::GetDropdownOptions(params) => {
            let options = session.dropdown_options(params.index).await?;
            Ok(ActionResult::extracted(format!(
                "Dropdown options: {}",
                options.join(", ")
            )))
        }
        BrowserAction::SelectDropdownOption(params) => {
            session
                .select_dropdown_option(params.index, &params.text)
                .await?;
            Ok(ActionResult::extracted(format!(
                "Selected dropdown option '{}' on element {}",
                params.text, params.index
            )))
        }
        BrowserAction::SendKeys(params) => {
            session.send_keys(&params.keys).await?;
            Ok(ActionResult::extracted(format!(
                "Sent keys '{}'",
                params.keys
            )))
        }
        BrowserAction::SaveAsPdf(params) => {
            let pdf = session
                .save_pdf(
                    params.print_background,
                    params.landscape,
                    params.scale,
                    &params.paper_format,
                )
                .await?;

            let page_title = if params.file_name.is_none() {
                session.state(false).await.ok().map(|state| state.title)
            } else {
                None
            };
            let output_path = pdf_output_path(params.file_name.as_deref(), page_title.as_deref());
            if let Some(parent) = output_path.parent()
                && !parent.as_os_str().is_empty()
            {
                std::fs::create_dir_all(parent)
                    .map_err(|error| BrowserError::ActionFailed(error.to_string()))?;
            }
            let output_path = next_available_pdf_path(output_path);
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(&pdf.base64_pdf)
                .map_err(|error| BrowserError::ActionFailed(error.to_string()))?;
            std::fs::write(&output_path, bytes)
                .map_err(|error| BrowserError::ActionFailed(error.to_string()))?;
            let file_name = output_path.display().to_string();
            Ok(ActionResult {
                extracted_content: Some(format!("Saved PDF to {file_name}")),
                error: None,
                judgement: None,
                long_term_memory: Some(format!("Saved PDF to {file_name}")),
                include_extracted_content_only_once: true,
                include_in_memory: true,
                is_done: false,
                success: None,
                attachments: vec![file_name],
                images: Vec::new(),
                metadata: None,
            })
        }
        BrowserAction::UploadFile(params) => {
            let upload_path = match upload_file_action_path(
                params,
                file_system,
                enforce_upload_file_availability,
                available_file_paths,
            ) {
                Ok(upload_path) => upload_path,
                Err(error) => return Ok(ActionResult::error(error)),
            };
            session.upload_file(params.index, &upload_path).await?;
            Ok(ActionResult::extracted(format!(
                "Uploaded {} to element {}",
                upload_path.display(),
                params.index
            )))
        }
        BrowserAction::WriteFile(params) => file_system.write_file(params),
        BrowserAction::ReadFile(params) => file_system.read_file(&params.file_name),
        BrowserAction::ReplaceFile(params) => {
            file_system.replace_file(&params.file_name, &params.old_str, &params.new_str)
        }
    }
}

fn is_select_click_validation_error(error: &BrowserError) -> bool {
    error
        .to_string()
        .contains("Cannot click on <select> elements.")
}

fn is_file_input_click_validation_error(error: &BrowserError) -> bool {
    error
        .to_string()
        .contains("Cannot click on file input elements.")
}

fn done_action_result(
    params: &browser_use_tools::DoneAction,
    file_system: Option<&ManagedFileSystem>,
    display_files_in_done_text: bool,
) -> ActionResult {
    let mut user_message = params.text.clone();
    let mut file_sections = Vec::new();
    let mut attachments = Vec::new();

    for file_name in &params.files_to_display {
        let displayed_file = file_system
            .and_then(|file_system| file_system.display_done_file(file_name))
            .or_else(|| display_done_file(file_name));
        if let Some((section, attachment)) = displayed_file {
            if display_files_in_done_text {
                file_sections.push(section);
            }
            attachments.push(attachment);
        }
    }

    if !file_sections.is_empty() {
        user_message.push_str("\n\nAttachments:");
        for section in file_sections {
            user_message.push_str("\n\n");
            user_message.push_str(&section);
        }
    }

    ActionResult::done_with_attachments(user_message, params.success, attachments)
}

fn display_done_file(file_name: &str) -> Option<(String, String)> {
    if validate_text_file_name(file_name).is_some() {
        return None;
    }

    let content = std::fs::read_to_string(file_name).ok()?;
    let attachment = std::fs::canonicalize(file_name)
        .unwrap_or_else(|_| std::path::PathBuf::from(file_name))
        .display()
        .to_string();
    Some((format!("{file_name}:\n{content}"), attachment))
}

fn upload_file_action_path(
    params: &browser_use_tools::UploadFileAction,
    file_system: &ManagedFileSystem,
    enforce_upload_file_availability: bool,
    available_file_paths: &BTreeSet<String>,
) -> Result<std::path::PathBuf, String> {
    if !enforce_upload_file_availability {
        return Ok(std::path::PathBuf::from(&params.path));
    }

    let path = if available_file_paths.contains(&params.path) {
        std::path::PathBuf::from(&params.path)
    } else if let Some(path) = file_system.upload_file_path(&params.path) {
        path
    } else {
        return Err(format!(
            "File path {} is not available. Add it to AgentSettings.available_file_paths before using upload_file.",
            params.path
        ));
    };

    if !path.exists() {
        return Err(format!("File {} does not exist", path.display()));
    }
    if path.metadata().map(|metadata| metadata.len()).unwrap_or(0) == 0 {
        return Err(format!(
            "File {} is empty (0 bytes). The file may not have been saved correctly.",
            path.display()
        ));
    }

    Ok(path)
}

const MAX_EXTRACT_CHAR_LIMIT: usize = 100_000;
const MAX_EXTRACT_RELATED_ELEMENTS: usize = 200;
const MAX_EXTRACT_MEMORY_LENGTH: usize = 10_000;
const MAX_PROMPT_CONTENT_CHARS: usize = 60_000;
const MAX_PROMPT_ERROR_CHARS: usize = 200;
const PROMPT_ERROR_EDGE_CHARS: usize = 100;
const IMAGE_QUERY_KEYWORDS: &[&str] = &[
    "image",
    "photo",
    "picture",
    "thumbnail",
    "img url",
    "image url",
    "photo url",
    "product image",
];

fn should_extract_images(query: &str, requested: bool) -> bool {
    let query = query.to_ascii_lowercase();
    requested
        || IMAGE_QUERY_KEYWORDS
            .iter()
            .any(|keyword| query.contains(keyword))
}

fn extract_action_result(
    params: &browser_use_tools::ExtractAction,
    page_text: &str,
    source_url: Option<&str>,
    extract_images: bool,
    links: Option<&[FoundElement]>,
    images: Option<&[FoundElement]>,
    file_system: Option<&mut ManagedFileSystem>,
) -> ActionResult {
    let total_chars = page_text.chars().count();
    if params.start_from_char > total_chars {
        return ActionResult::error(format!(
            "start_from_char ({}) exceeds content length {total_chars} characters.",
            params.start_from_char
        ));
    }

    let available_chars = total_chars.saturating_sub(params.start_from_char);
    let truncated = available_chars > MAX_EXTRACT_CHAR_LIMIT;
    let content: String = page_text
        .chars()
        .skip(params.start_from_char)
        .take(MAX_EXTRACT_CHAR_LIMIT)
        .collect();
    let next_start_char = params.start_from_char + content.chars().count();
    let content_stats = extract_content_stats(
        total_chars,
        params.start_from_char,
        content.chars().count(),
        truncated,
        next_start_char,
        params.extract_links,
        extract_images,
    );
    let rendered =
        render_extract_envelope(params, source_url, &content, &content_stats, links, images);
    let memory = if rendered.chars().count() < 10_000 {
        rendered.clone()
    } else if let Some(file_name) =
        file_system.and_then(|file_system| file_system.save_extracted_content(&rendered).ok())
    {
        format!(
            "Query: {}\nContent in {file_name} and once in <read_state>.",
            params.query
        )
    } else {
        format!(
            "Query: {}\nContent prepared for extraction, length: {} characters.",
            params.query,
            content.chars().count()
        )
    };

    ActionResult {
        extracted_content: Some(rendered),
        error: None,
        judgement: None,
        long_term_memory: Some(memory),
        include_extracted_content_only_once: true,
        include_in_memory: true,
        is_done: false,
        success: None,
        attachments: Vec::new(),
        images: Vec::new(),
        metadata: extract_metadata(
            params,
            source_url,
            total_chars,
            params.start_from_char,
            content.chars().count(),
            truncated,
            next_start_char,
            extract_images,
            links.map_or(0, <[FoundElement]>::len),
            images.map_or(0, <[FoundElement]>::len),
        ),
    }
}

fn extract_content_stats(
    total_chars: usize,
    start_from_char: usize,
    content_chars: usize,
    truncated: bool,
    next_start_char: usize,
    extract_links: bool,
    extract_images: bool,
) -> String {
    let mut stats =
        format!("Content processed: {total_chars} text chars -> {total_chars} filtered text chars");
    if start_from_char > 0 {
        stats.push_str(&format!(" (started from char {start_from_char})"));
    }
    if truncated {
        stats.push_str(&format!(
            " -> {content_chars} final chars (use start_from_char={next_start_char} to continue)"
        ));
    }
    if extract_links || extract_images {
        stats.push_str(&format!(
            "\nExtraction options: extract_links={extract_links}, extract_images={extract_images}"
        ));
    }
    stats
}

#[allow(clippy::too_many_arguments)]
fn extract_metadata(
    params: &browser_use_tools::ExtractAction,
    source_url: Option<&str>,
    original_chars: usize,
    start_from_char: usize,
    content_chars: usize,
    truncated: bool,
    next_start_char: usize,
    extract_images: bool,
    links_count: usize,
    images_count: usize,
) -> Option<Value> {
    let schema = params.output_schema.as_ref()?;
    Some(serde_json::json!({
        "structured_extraction": true,
        "schema_used": schema,
        "is_partial": truncated,
        "source_url": source_url,
        "content_stats": {
            "method": "page_text",
            "original_text_chars": original_chars,
            "final_filtered_chars": original_chars,
            "started_from_char": start_from_char,
            "returned_chars": content_chars,
            "next_start_char": if truncated { Some(next_start_char) } else { None::<usize> },
        },
        "options": {
            "extract_links": params.extract_links,
            "extract_images": extract_images,
            "links_count": links_count,
            "images_count": images_count,
            "already_collected_count": params.already_collected.len(),
        }
    }))
}

fn render_extract_envelope(
    params: &browser_use_tools::ExtractAction,
    source_url: Option<&str>,
    content: &str,
    content_stats: &str,
    links: Option<&[FoundElement]>,
    images: Option<&[FoundElement]>,
) -> String {
    let mut rendered = source_url
        .map(|url| format!("<url>\n{url}\n</url>\n"))
        .unwrap_or_default();
    rendered.push_str(&format!("<query>\n{}\n</query>\n\n", params.query));

    if let Some(schema) = &params.output_schema {
        let schema = serde_json::to_string_pretty(schema).unwrap_or_else(|_| schema.to_string());
        rendered.push_str(&format!("<output_schema>\n{schema}\n</output_schema>\n\n"));
    }

    rendered.push_str(&format!(
        "<content_stats>\n{content_stats}\n</content_stats>\n\n<webpage_content>\n{content}\n</webpage_content>"
    ));

    if let Some(links) = links.and_then(render_link_appendix) {
        rendered.push_str(&format!("\n\n<links>\n{links}\n</links>"));
    }

    if let Some(images) = images.and_then(render_image_appendix) {
        rendered.push_str(&format!("\n\n<images>\n{images}\n</images>"));
    }

    if !params.already_collected.is_empty() {
        let items = params
            .already_collected
            .iter()
            .take(100)
            .map(|item| format!("- {item}"))
            .collect::<Vec<_>>()
            .join("\n");
        rendered.push_str(&format!(
            "\n\n<already_collected>\nSkip items whose name/title/URL matches any of these already-collected identifiers:\n{items}\n</already_collected>"
        ));
    }

    rendered
}

fn build_extract_llm_request(
    params: &browser_use_tools::ExtractAction,
    raw_envelope: &str,
) -> ChatRequest {
    let structured_schema = params.output_schema.clone();
    let system_prompt = if structured_schema.is_some() {
        "You are an expert at extracting structured data from webpage markdown. Extract only information present in the webpage. Return data that conforms exactly to the provided JSON Schema."
    } else {
        "You are an expert at extracting data from webpage markdown. Extract only information relevant to the query. Do not guess or use outside knowledge."
    };
    let user_prompt = format!(
        "Use the prepared extraction envelope below. It contains the query, content statistics, webpage markdown, and any link/image/already-collected context.\n\n{raw_envelope}"
    );

    ChatRequest {
        messages: vec![
            ChatMessage::text(MessageRole::System, system_prompt),
            ChatMessage::text(MessageRole::User, user_prompt),
        ],
        output_schema: Some(structured_schema.unwrap_or_else(schema_for_extract_text_result)),
    }
}

fn schema_for_extract_text_result() -> Value {
    serde_json::json!({
        "type": "object",
        "properties": {
            "result": { "type": "string" }
        },
        "required": ["result"],
        "additionalProperties": false
    })
}

fn complete_llm_extract_result(
    params: &browser_use_tools::ExtractAction,
    raw_envelope: &str,
    raw_metadata: Option<&Value>,
    completion: Value,
    file_system: &mut ManagedFileSystem,
) -> ActionResult {
    let source_url = tagged_section(raw_envelope, "url").unwrap_or("about:blank");
    let metadata = params
        .output_schema
        .as_ref()
        .map(|schema| structured_extract_metadata(schema, raw_metadata, source_url, &completion));
    let extracted_content = if params.output_schema.is_some() {
        format!(
            "<url>\n{source_url}\n</url>\n<query>\n{}\n</query>\n<structured_result>\n{}\n</structured_result>",
            params.query,
            serde_json::to_string(&completion).unwrap_or_else(|_| completion.to_string())
        )
    } else {
        format!(
            "<url>\n{source_url}\n</url>\n<query>\n{}\n</query>\n<result>\n{}\n</result>",
            params.query,
            extract_text_completion(completion)
        )
    };
    let (long_term_memory, include_extracted_content_only_once) =
        extract_memory_fields(&params.query, &extracted_content, file_system);

    ActionResult {
        extracted_content: Some(extracted_content),
        error: None,
        judgement: None,
        long_term_memory: Some(long_term_memory),
        include_extracted_content_only_once,
        include_in_memory: true,
        is_done: false,
        success: None,
        attachments: Vec::new(),
        images: Vec::new(),
        metadata,
    }
}

fn extract_text_completion(completion: Value) -> String {
    completion
        .get("result")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
        .or_else(|| completion.as_str().map(ToOwned::to_owned))
        .unwrap_or_else(|| completion.to_string())
}

fn extract_memory_fields(
    query: &str,
    extracted_content: &str,
    file_system: &mut ManagedFileSystem,
) -> (String, bool) {
    if extracted_content.chars().count() < MAX_EXTRACT_MEMORY_LENGTH {
        return (extracted_content.to_owned(), false);
    }

    if let Ok(file_name) = file_system.save_extracted_content(extracted_content) {
        (
            format!("Query: {query}\nContent in {file_name} and once in <read_state>."),
            true,
        )
    } else {
        (
            format!(
                "Query: {query}\nExtracted result length: {} characters.",
                extracted_content.chars().count()
            ),
            true,
        )
    }
}

fn structured_extract_metadata(
    schema: &Value,
    raw_metadata: Option<&Value>,
    source_url: &str,
    data: &Value,
) -> Value {
    serde_json::json!({
        "structured_extraction": true,
        "extraction_result": {
            "data": data,
            "schema_used": schema,
            "is_partial": raw_metadata
                .and_then(|metadata| metadata.get("is_partial"))
                .cloned()
                .unwrap_or(Value::Bool(false)),
            "source_url": source_url,
            "content_stats": raw_metadata
                .and_then(|metadata| metadata.get("content_stats"))
                .cloned()
                .unwrap_or(Value::Null),
        }
    })
}

fn tagged_section<'a>(text: &'a str, tag: &str) -> Option<&'a str> {
    let start_tag = format!("<{tag}>\n");
    let end_tag = format!("\n</{tag}>");
    let start = text.find(&start_tag)? + start_tag.len();
    let end = text[start..].find(&end_tag)? + start;
    Some(&text[start..end])
}

fn render_link_appendix(elements: &[FoundElement]) -> Option<String> {
    let lines = elements
        .iter()
        .filter_map(|element| {
            let href = element.attributes.get("href")?.trim();
            if href.is_empty() {
                return None;
            }
            let label = element_label(element)
                .filter(|label| !label.is_empty())
                .unwrap_or(href);
            Some(format!("- {label}: {href}"))
        })
        .collect::<Vec<_>>();

    (!lines.is_empty()).then(|| lines.join("\n"))
}

fn render_image_appendix(elements: &[FoundElement]) -> Option<String> {
    let lines = elements
        .iter()
        .filter_map(|element| {
            let src = element
                .attributes
                .get("src")
                .or_else(|| element.attributes.get("data-src"))
                .or_else(|| element.attributes.get("srcset"))?
                .trim();
            if src.is_empty() {
                return None;
            }
            let label = element_label(element)
                .filter(|label| !label.is_empty())
                .unwrap_or("image");
            Some(format!("- {label}: {src}"))
        })
        .collect::<Vec<_>>();

    (!lines.is_empty()).then(|| lines.join("\n"))
}

fn element_label(element: &FoundElement) -> Option<&str> {
    element
        .text
        .as_deref()
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .or_else(|| attr_label(element, "alt"))
        .or_else(|| attr_label(element, "title"))
        .or_else(|| attr_label(element, "aria-label"))
}

fn attr_label<'a>(element: &'a FoundElement, name: &str) -> Option<&'a str> {
    element
        .attributes
        .get(name)
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
}

fn extract_link_attributes() -> Vec<String> {
    ["href", "title", "aria-label", "rel"]
        .into_iter()
        .map(str::to_owned)
        .collect()
}

fn extract_image_attributes() -> Vec<String> {
    ["src", "data-src", "srcset", "alt", "title", "aria-label"]
        .into_iter()
        .map(str::to_owned)
        .collect()
}

fn pdf_output_path(file_name: Option<&str>, page_title: Option<&str>) -> std::path::PathBuf {
    let raw_name = file_name
        .filter(|name| !name.trim().is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| {
            page_title
                .map(sanitize_pdf_title)
                .filter(|title| !title.is_empty())
                .unwrap_or_else(|| "page".to_owned())
        });
    let path = std::path::PathBuf::from(raw_name);
    ensure_pdf_extension(path)
}

fn sanitize_pdf_title(title: &str) -> String {
    title
        .chars()
        .filter(|character| {
            character.is_alphanumeric()
                || *character == '_'
                || *character == ' '
                || *character == '-'
        })
        .collect::<String>()
        .trim()
        .chars()
        .take(50)
        .collect()
}

fn ensure_pdf_extension(mut path: std::path::PathBuf) -> std::path::PathBuf {
    let has_pdf_extension = path
        .file_name()
        .and_then(std::ffi::OsStr::to_str)
        .is_some_and(|file_name| file_name.to_ascii_lowercase().ends_with(".pdf"));
    if has_pdf_extension {
        return path;
    }

    let Some(file_name) = path
        .file_name()
        .and_then(std::ffi::OsStr::to_str)
        .map(str::to_owned)
    else {
        return std::path::PathBuf::from("page.pdf");
    };
    path.set_file_name(format!("{file_name}.pdf"));
    path
}

fn next_available_pdf_path(path: std::path::PathBuf) -> std::path::PathBuf {
    if !path.exists() {
        return path;
    }

    let parent = path.parent().map(std::path::Path::to_path_buf);
    let stem = path
        .file_stem()
        .and_then(std::ffi::OsStr::to_str)
        .unwrap_or("page");
    let extension = path
        .extension()
        .and_then(std::ffi::OsStr::to_str)
        .unwrap_or("pdf");

    for counter in 1.. {
        let candidate_name = format!("{stem} ({counter}).{extension}");
        let candidate = parent.as_ref().map_or_else(
            || std::path::PathBuf::from(&candidate_name),
            |parent| parent.join(&candidate_name),
        );
        if !candidate.exists() {
            return candidate;
        }
    }
    unreachable!("unbounded PDF filename counter should always return")
}

fn screenshot_output_path(file_name: &str) -> std::path::PathBuf {
    let path = std::path::PathBuf::from(if file_name.trim().is_empty() {
        "screenshot".to_owned()
    } else {
        file_name.to_owned()
    });
    ensure_png_extension(path)
}

fn ensure_png_extension(mut path: std::path::PathBuf) -> std::path::PathBuf {
    let has_png_extension = path
        .file_name()
        .and_then(std::ffi::OsStr::to_str)
        .is_some_and(|file_name| file_name.to_ascii_lowercase().ends_with(".png"));
    if has_png_extension {
        return path;
    }

    let Some(file_name) = path
        .file_name()
        .and_then(std::ffi::OsStr::to_str)
        .map(str::to_owned)
    else {
        return std::path::PathBuf::from("screenshot.png");
    };
    path.set_file_name(format!("{file_name}.png"));
    path
}

fn write_file_action(
    params: &browser_use_tools::WriteFileAction,
) -> Result<ActionResult, BrowserError> {
    let resolved_file = resolve_file_action_path(&params.file_name, supported_write_extensions());
    if let Some(result) = validate_write_file_name(&resolved_file.display_name) {
        return Ok(result);
    }
    let path = resolved_file.path.as_path();
    if params.append && !path.exists() {
        return Ok(ActionResult::error(format!(
            "File '{}' not found.",
            resolved_file.display_name
        )));
    }
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent)
            .map_err(|error| BrowserError::ActionFailed(error.to_string()))?;
    }

    let mut content = params.content.clone();
    if params.trailing_newline {
        content.push('\n');
    }
    if params.leading_newline {
        content.insert(0, '\n');
    }

    if is_csv_file(&resolved_file.display_name) {
        content = normalize_csv_content(&content);
    }

    if params.append {
        if is_pdf_file(&resolved_file.display_name) {
            let existing = pdf_extract::extract_text(path)
                .map_err(|error| BrowserError::ActionFailed(error.to_string()))?;
            let merged = merge_pdf_append_content(&existing, &content);
            write_pdf_text(path, &merged)
                .map_err(|error| BrowserError::ActionFailed(error.to_string()))?;
        } else if is_docx_file(&resolved_file.display_name) {
            let existing = read_docx_text(&resolved_file.path_string())
                .map_err(|error| BrowserError::ActionFailed(error.to_string()))?;
            let merged = merge_docx_append_content(&existing, &content);
            write_docx_text(path, &merged)
                .map_err(|error| BrowserError::ActionFailed(error.to_string()))?;
        } else if is_csv_file(&resolved_file.display_name) {
            let existing = std::fs::read_to_string(path)
                .map_err(|error| BrowserError::ActionFailed(error.to_string()))?;
            let merged = merge_csv_append_content(&existing, &content);
            std::fs::write(path, merged)
                .map_err(|error| BrowserError::ActionFailed(error.to_string()))?;
        } else {
            let mut file = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)
                .map_err(|error| BrowserError::ActionFailed(error.to_string()))?;
            file.write_all(content.as_bytes())
                .map_err(|error| BrowserError::ActionFailed(error.to_string()))?;
        }
        Ok(ActionResult::extracted(format!(
            "Appended to file {}{}",
            resolved_file.display_name,
            resolved_file.correction_suffix()
        )))
    } else {
        if is_pdf_file(&resolved_file.display_name) {
            write_pdf_text(path, &content)
                .map_err(|error| BrowserError::ActionFailed(error.to_string()))?;
        } else if is_docx_file(&resolved_file.display_name) {
            write_docx_text(path, &content)
                .map_err(|error| BrowserError::ActionFailed(error.to_string()))?;
        } else {
            std::fs::write(path, content)
                .map_err(|error| BrowserError::ActionFailed(error.to_string()))?;
        }
        Ok(ActionResult::extracted(format!(
            "Wrote file {}{}",
            resolved_file.display_name,
            resolved_file.correction_suffix()
        )))
    }
}

fn is_csv_file(file_name: &str) -> bool {
    file_extension(file_name).as_deref() == Some("csv")
}

fn is_pdf_file(file_name: &str) -> bool {
    file_extension(file_name).as_deref() == Some("pdf")
}

fn is_docx_file(file_name: &str) -> bool {
    file_extension(file_name).as_deref() == Some("docx")
}

fn merge_pdf_append_content(existing: &str, new_content: &str) -> String {
    let existing = existing.trim_end_matches(['\n', '\r', '\u{c}']);
    merge_document_append_content(existing, new_content)
}

fn merge_docx_append_content(existing: &str, new_content: &str) -> String {
    merge_document_append_content(existing, new_content)
}

fn merge_document_append_content(existing: &str, new_content: &str) -> String {
    if new_content
        .trim_matches(|char| char == '\n' || char == '\r')
        .is_empty()
    {
        return existing.to_owned();
    }

    let mut merged = existing.to_owned();
    if !merged.is_empty() && !new_content.starts_with('\n') {
        merged.push('\n');
    }
    merged.push_str(new_content);
    merged
}

fn merge_csv_append_content(existing: &str, new_content: &str) -> String {
    if new_content
        .trim_matches(|char| char == '\n' || char == '\r')
        .is_empty()
    {
        return existing.to_owned();
    }

    let mut merged = existing.to_owned();
    if !merged.is_empty() && !merged.ends_with('\n') {
        merged.push('\n');
    }
    merged.push_str(new_content);
    normalize_csv_content(&merged)
}

fn normalize_csv_content(raw: &str) -> String {
    let mut content = raw
        .trim_matches(|char| char == '\n' || char == '\r')
        .to_owned();
    if content.is_empty() {
        return raw.to_owned();
    }

    if !content.contains('\n') && content.contains("\\n") {
        content = content.replace("\\\"", "\"").replace("\\n", "\n");
    }

    let mut reader = csv::ReaderBuilder::new()
        .has_headers(false)
        .flexible(true)
        .from_reader(content.as_bytes());
    let mut rows = Vec::new();
    for record in reader.records() {
        let Ok(record) = record else {
            return raw.to_owned();
        };
        if !record.is_empty() {
            rows.push(record);
        }
    }

    if rows.is_empty() {
        return raw.to_owned();
    }

    let mut output = Vec::new();
    {
        let mut writer = csv::WriterBuilder::new()
            .has_headers(false)
            .terminator(csv::Terminator::Any(b'\n'))
            .from_writer(&mut output);
        for row in rows {
            if writer.write_record(&row).is_err() {
                return raw.to_owned();
            }
        }
        if writer.flush().is_err() {
            return raw.to_owned();
        }
    }

    let Ok(mut normalized) = String::from_utf8(output) else {
        return raw.to_owned();
    };
    while normalized.ends_with('\n') {
        normalized.pop();
    }
    normalized
}

fn read_file_action(file_name: &str) -> Result<ActionResult, BrowserError> {
    let read_extensions = supported_read_extensions();
    let resolved_file = resolve_file_action_path(file_name, &read_extensions);
    if let Some(result) = validate_read_file_name(&resolved_file.display_name) {
        return Ok(result);
    }
    let path_string = resolved_file.path_string();
    if is_supported_read_image_file(&resolved_file.display_name) {
        let mut result = read_image_file_action(&path_string)?;
        apply_file_name_correction_note(&mut result, &resolved_file);
        return Ok(result);
    }
    if is_supported_read_document_file(&resolved_file.display_name) {
        let mut result = read_document_file_action(&path_string)?;
        apply_file_name_correction_note(&mut result, &resolved_file);
        return Ok(result);
    }
    let content = std::fs::read_to_string(&resolved_file.path)
        .map_err(|error| BrowserError::ActionFailed(error.to_string()))?;
    let memory = read_file_memory(&content);
    Ok(ActionResult {
        extracted_content: Some(format!(
            "{}Read file {}:\n{content}",
            resolved_file.correction_prefix(),
            resolved_file.display_name
        )),
        error: None,
        judgement: None,
        long_term_memory: Some(memory),
        include_extracted_content_only_once: true,
        include_in_memory: true,
        is_done: false,
        success: None,
        attachments: Vec::new(),
        images: Vec::new(),
        metadata: None,
    })
}

fn read_document_file_action(file_name: &str) -> Result<ActionResult, BrowserError> {
    if is_pdf_file(file_name) {
        return read_pdf_file_action(file_name);
    }

    let content = match read_document_file_content(file_name) {
        Ok(content) => content,
        Err(error) => {
            return Ok(ActionResult::error(format!(
                "Error: Could not read file '{file_name}'. {error}"
            )));
        }
    };
    let content = truncate_read_document_content(&content);
    let memory = read_file_memory(&content);
    Ok(ActionResult {
        extracted_content: Some(format!(
            "Read from file {file_name}.\n<content>\n{content}\n</content>"
        )),
        error: None,
        judgement: None,
        long_term_memory: Some(memory),
        include_extracted_content_only_once: true,
        include_in_memory: true,
        is_done: false,
        success: None,
        attachments: Vec::new(),
        images: Vec::new(),
        metadata: None,
    })
}

fn read_document_file_content(file_name: &str) -> Result<String, String> {
    match file_extension(file_name).as_deref() {
        Some("docx") => read_docx_text(file_name),
        _ => Err("unsupported document extension".to_owned()),
    }
}

fn read_pdf_file_action(file_name: &str) -> Result<ActionResult, BrowserError> {
    let pages = match pdf_extract::extract_text_by_pages(file_name) {
        Ok(pages) => pdf_pages_or_empty_page(pages),
        Err(error) => {
            return Ok(ActionResult::error(format!(
                "Error: Could not read file '{file_name}'. {error}"
            )));
        }
    };
    let envelope = render_pdf_read_envelope(file_name, &pages);
    let memory = read_file_memory(&pages.join("\n"));

    Ok(ActionResult {
        extracted_content: Some(envelope),
        error: None,
        judgement: None,
        long_term_memory: Some(memory),
        include_extracted_content_only_once: true,
        include_in_memory: true,
        is_done: false,
        success: None,
        attachments: Vec::new(),
        images: Vec::new(),
        metadata: None,
    })
}

fn pdf_pages_or_empty_page(pages: Vec<String>) -> Vec<String> {
    if pages.is_empty() {
        vec![String::new()]
    } else {
        pages
    }
}

const PDF_READ_MAX_CHARS: usize = 60_000;

fn render_pdf_read_envelope(file_name: &str, pages: &[String]) -> String {
    let total_pages = pages.len();
    let total_chars: usize = pages.iter().map(|page| page.chars().count()).sum();
    if total_chars <= PDF_READ_MAX_CHARS {
        let content = render_pdf_page_markers(
            pages
                .iter()
                .enumerate()
                .filter(|(_, text)| !text.trim().is_empty())
                .map(|(index, text)| (index + 1, text.as_str())),
        );
        return format!(
            "Read from file {file_name} ({total_pages} pages, {} chars).\n<content>\n{content}\n</content>",
            format_usize_with_commas(total_chars)
        );
    }

    let mut content_parts = Vec::new();
    let mut pages_included = BTreeSet::new();
    let mut chars_used = 0usize;
    for page_number in pdf_priority_pages(pages) {
        let text = &pages[page_number - 1];
        if text.trim().is_empty() {
            continue;
        }

        let header = format!("--- Page {page_number} ---\n");
        let truncation_suffix = "\n[...truncated]";
        let remaining = PDF_READ_MAX_CHARS.saturating_sub(chars_used);
        let min_useful = header.chars().count() + truncation_suffix.chars().count() + 50;
        if remaining < min_useful {
            break;
        }

        let mut page_content = format!("{header}{text}");
        if page_content.chars().count() > remaining {
            let kept_chars = remaining.saturating_sub(truncation_suffix.chars().count());
            page_content = format!(
                "{}{truncation_suffix}",
                page_content.chars().take(kept_chars).collect::<String>()
            );
        }
        chars_used += page_content.chars().count();
        pages_included.insert(page_number);
        content_parts.push((page_number, page_content));
        if chars_used >= PDF_READ_MAX_CHARS {
            break;
        }
    }

    content_parts.sort_by_key(|(page_number, _)| *page_number);
    let mut content = content_parts
        .into_iter()
        .map(|(_, content)| content)
        .collect::<Vec<_>>()
        .join("\n\n");
    if pages_included.len() < total_pages {
        let skipped = (1..=total_pages)
            .filter(|page_number| !pages_included.contains(page_number))
            .collect::<Vec<_>>();
        let skipped_preview = skipped
            .iter()
            .take(10)
            .map(usize::to_string)
            .collect::<Vec<_>>()
            .join(", ");
        let ellipsis = if skipped.len() > 10 { "..." } else { "" };
        content.push_str(&format!(
            "\n\n[Showing {} of {total_pages} pages. Skipped pages: [{skipped_preview}]{ellipsis}. Use extract with start_from_char to read further into the file.]",
            pages_included.len()
        ));
    }

    format!(
        "Read from file {file_name} ({total_pages} pages, {} chars total).\n<content>\n{content}\n</content>",
        format_usize_with_commas(total_chars)
    )
}

fn render_pdf_page_markers<'a>(pages: impl Iterator<Item = (usize, &'a str)>) -> String {
    pages
        .map(|(page_number, text)| format!("--- Page {page_number} ---\n{text}"))
        .collect::<Vec<_>>()
        .join("\n\n")
}

fn pdf_priority_pages(pages: &[String]) -> Vec<usize> {
    let word_pattern = regex::Regex::new(r"\b[a-zA-Z]{4,}\b").expect("valid word regex");
    let total_pages = pages.len();
    let mut page_words = BTreeMap::<usize, BTreeSet<String>>::new();
    let mut word_to_pages = BTreeMap::<String, BTreeSet<usize>>::new();

    for (index, text) in pages.iter().enumerate() {
        let page_number = index + 1;
        let words = word_pattern
            .find_iter(text)
            .map(|word| word.as_str().to_ascii_lowercase())
            .collect::<BTreeSet<_>>();
        for word in &words {
            word_to_pages
                .entry(word.clone())
                .or_default()
                .insert(page_number);
        }
        page_words.insert(page_number, words);
    }

    let mut scored_pages = page_words
        .iter()
        .map(|(page_number, words)| {
            let score = words
                .iter()
                .filter_map(|word| word_to_pages.get(word))
                .map(|pages_with_word| (total_pages as f64 / pages_with_word.len() as f64).ln())
                .sum::<f64>();
            (*page_number, score)
        })
        .collect::<Vec<_>>();
    scored_pages.sort_by(|(left_page, left_score), (right_page, right_score)| {
        right_score
            .partial_cmp(left_score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| left_page.cmp(right_page))
    });

    let mut priority_pages = Vec::new();
    if total_pages > 0 {
        priority_pages.push(1);
    }
    for (page_number, _) in scored_pages {
        if !priority_pages.contains(&page_number) {
            priority_pages.push(page_number);
        }
    }
    for page_number in 1..=total_pages {
        if !priority_pages.contains(&page_number) {
            priority_pages.push(page_number);
        }
    }
    priority_pages
}

fn format_usize_with_commas(value: usize) -> String {
    let text = value.to_string();
    let mut formatted = String::with_capacity(text.len() + text.len() / 3);
    for (index, character) in text.chars().rev().enumerate() {
        if index > 0 && index % 3 == 0 {
            formatted.push(',');
        }
        formatted.push(character);
    }
    formatted.chars().rev().collect()
}

fn write_pdf_text(path: &std::path::Path, content: &str) -> Result<(), String> {
    std::fs::write(path, pdf_document_bytes(content)).map_err(|error| error.to_string())
}

fn pdf_document_bytes(content: &str) -> Vec<u8> {
    let streams = pdf_page_streams(content);
    let page_count = streams.len();
    let font_object_id = 3usize;
    let first_page_object_id = 4usize;
    let first_content_object_id = first_page_object_id + page_count;
    let kids = (0..page_count)
        .map(|index| format!("{} 0 R", first_page_object_id + index))
        .collect::<Vec<_>>()
        .join(" ");

    let mut objects = vec![
        "<< /Type /Catalog /Pages 2 0 R >>".to_owned(),
        format!("<< /Type /Pages /Kids [{kids}] /Count {page_count} >>"),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_owned(),
    ];

    for (index, _) in streams.iter().enumerate() {
        let page_object_id = first_page_object_id + index;
        let content_object_id = first_content_object_id + index;
        objects.push(format!(
            "<< /Type /Page /Parent 2 0 R /Resources << /Font << /F1 {font_object_id} 0 R >> >> /MediaBox [0 0 612 792] /Contents {content_object_id} 0 R >>"
        ));
        debug_assert_eq!(objects.len(), page_object_id);
    }

    for stream in streams {
        objects.push(format!(
            "<< /Length {} >>\nstream\n{}endstream",
            stream.len(),
            stream
        ));
    }

    pdf_objects_to_bytes(&objects)
}

fn pdf_page_streams(content: &str) -> Vec<String> {
    let mut streams = Vec::new();
    let mut stream = String::new();
    let mut y = 720i32;
    let mut has_text_on_page = false;

    for line in content.split('\n') {
        let line = pdf_line_style(line);
        if y - line.advance < 72 && has_text_on_page {
            streams.push(stream);
            stream = String::new();
            y = 720;
            has_text_on_page = false;
        }

        if let Some(text) = line.text {
            stream.push_str("BT\n");
            stream.push_str(&format!("/F1 {} Tf\n", line.font_size));
            stream.push_str(&format!("72 {y} Td\n"));
            stream.push_str(&format!("({}) Tj\n", pdf_escape_literal_text(&text)));
            stream.push_str("ET\n");
            has_text_on_page = true;
        }
        y -= line.advance;
    }

    streams.push(stream);
    streams
}

struct PdfLineStyle {
    text: Option<String>,
    font_size: u32,
    advance: i32,
}

fn pdf_line_style(line: &str) -> PdfLineStyle {
    if line.trim().is_empty() {
        return PdfLineStyle {
            text: None,
            font_size: 12,
            advance: 6,
        };
    }

    if let Some(text) = line.strip_prefix("# ") {
        return PdfLineStyle {
            text: Some(text.to_owned()),
            font_size: 24,
            advance: 34,
        };
    }
    if let Some(text) = line.strip_prefix("## ") {
        return PdfLineStyle {
            text: Some(text.to_owned()),
            font_size: 18,
            advance: 26,
        };
    }
    if let Some(text) = line.strip_prefix("### ") {
        return PdfLineStyle {
            text: Some(text.to_owned()),
            font_size: 14,
            advance: 20,
        };
    }

    PdfLineStyle {
        text: Some(line.to_owned()),
        font_size: 12,
        advance: 17,
    }
}

fn pdf_escape_literal_text(text: &str) -> String {
    let mut escaped = String::new();
    for character in text.chars() {
        match character {
            '\\' => escaped.push_str(r"\\"),
            '(' => escaped.push_str(r"\("),
            ')' => escaped.push_str(r"\)"),
            '\t' => escaped.push_str(r"\t"),
            '\r' => {}
            character if character.is_control() => {
                escaped.push_str(&format!(r"\{:03o}", character as u32));
            }
            character => escaped.push(character),
        }
    }
    escaped
}

fn pdf_objects_to_bytes(objects: &[String]) -> Vec<u8> {
    let mut pdf = b"%PDF-1.4\n".to_vec();
    let mut offsets = Vec::with_capacity(objects.len());
    for (index, object) in objects.iter().enumerate() {
        offsets.push(pdf.len());
        pdf.extend_from_slice(format!("{} 0 obj\n{}\nendobj\n", index + 1, object).as_bytes());
    }
    let xref_offset = pdf.len();
    pdf.extend_from_slice(format!("xref\n0 {}\n", objects.len() + 1).as_bytes());
    pdf.extend_from_slice(b"0000000000 65535 f \n");
    for offset in offsets {
        pdf.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    pdf.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{}\n%%EOF\n",
            objects.len() + 1,
            xref_offset
        )
        .as_bytes(),
    );
    pdf
}

fn read_docx_text(file_name: &str) -> Result<String, String> {
    let file = std::fs::File::open(file_name).map_err(|error| error.to_string())?;
    let mut archive = zip::ZipArchive::new(file).map_err(|error| error.to_string())?;
    let mut document = archive
        .by_name("word/document.xml")
        .map_err(|error| error.to_string())?;
    let mut xml = String::new();
    document
        .read_to_string(&mut xml)
        .map_err(|error| error.to_string())?;
    docx_document_xml_to_text(&xml)
}

fn write_docx_text(path: &std::path::Path, content: &str) -> Result<(), String> {
    let file = std::fs::File::create(path).map_err(|error| error.to_string())?;
    let mut archive = zip::ZipWriter::new(file);
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);

    archive
        .start_file("[Content_Types].xml", options)
        .map_err(|error| error.to_string())?;
    archive
        .write_all(docx_content_types_xml().as_bytes())
        .map_err(|error| error.to_string())?;
    archive
        .start_file("_rels/.rels", options)
        .map_err(|error| error.to_string())?;
    archive
        .write_all(docx_root_relationships_xml().as_bytes())
        .map_err(|error| error.to_string())?;
    archive
        .start_file("word/document.xml", options)
        .map_err(|error| error.to_string())?;
    archive
        .write_all(docx_document_xml(content).as_bytes())
        .map_err(|error| error.to_string())?;
    archive.finish().map_err(|error| error.to_string())?;
    Ok(())
}

fn docx_content_types_xml() -> &'static str {
    r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/></Types>"#
}

fn docx_root_relationships_xml() -> &'static str {
    r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#
}

fn docx_document_xml(content: &str) -> String {
    let mut xml = String::from(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body>"#,
    );
    for paragraph in content.split('\n') {
        xml.push_str("<w:p>");
        xml.push_str(&docx_paragraph_runs(paragraph));
        xml.push_str("</w:p>");
    }
    xml.push_str(r#"<w:sectPr><w:pgSz w:w="12240" w:h="15840"/><w:pgMar w:top="1440" w:right="1440" w:bottom="1440" w:left="1440" w:header="720" w:footer="720" w:gutter="0"/></w:sectPr></w:body></w:document>"#);
    xml
}

fn docx_paragraph_runs(paragraph: &str) -> String {
    let mut runs = String::new();
    let mut text = String::new();
    for character in paragraph.chars() {
        if character == '\t' {
            push_docx_text_run(&mut runs, &text);
            text.clear();
            runs.push_str("<w:r><w:tab/></w:r>");
        } else {
            text.push(character);
        }
    }
    push_docx_text_run(&mut runs, &text);
    runs
}

fn push_docx_text_run(runs: &mut String, text: &str) {
    if text.is_empty() {
        return;
    }
    runs.push_str(r#"<w:r><w:t xml:space="preserve">"#);
    push_xml_escaped(runs, text);
    runs.push_str("</w:t></w:r>");
}

fn push_xml_escaped(output: &mut String, text: &str) {
    for character in text.chars() {
        match character {
            '&' => output.push_str("&amp;"),
            '<' => output.push_str("&lt;"),
            '>' => output.push_str("&gt;"),
            _ => output.push(character),
        }
    }
}

fn docx_document_xml_to_text(xml: &str) -> Result<String, String> {
    let mut reader = quick_xml::Reader::from_str(xml);
    reader.config_mut().trim_text(false);
    let mut text = String::new();
    let mut in_text = false;

    loop {
        match reader.read_event() {
            Ok(quick_xml::events::Event::Eof) => break,
            Ok(quick_xml::events::Event::Start(event))
                if local_xml_name(event.name().as_ref()) == b"t" =>
            {
                in_text = true;
            }
            Ok(quick_xml::events::Event::End(event))
                if local_xml_name(event.name().as_ref()) == b"t" =>
            {
                in_text = false;
            }
            Ok(quick_xml::events::Event::Text(event)) => {
                if in_text {
                    let decoded = event.decode().map_err(|error| error.to_string())?;
                    text.push_str(&decoded);
                }
            }
            Ok(quick_xml::events::Event::CData(event)) => {
                if in_text {
                    let decoded = event.decode().map_err(|error| error.to_string())?;
                    text.push_str(&decoded);
                }
            }
            Ok(quick_xml::events::Event::GeneralRef(event)) => {
                if in_text {
                    let decoded = event.decode().map_err(|error| error.to_string())?;
                    text.push_str(&decode_xml_general_ref(&decoded)?);
                }
            }
            Ok(quick_xml::events::Event::Empty(event))
                if local_xml_name(event.name().as_ref()) == b"tab" =>
            {
                text.push('\t');
            }
            Ok(quick_xml::events::Event::Empty(event))
                if local_xml_name(event.name().as_ref()) == b"br" =>
            {
                push_docx_newline(&mut text);
            }
            Ok(quick_xml::events::Event::End(event))
                if local_xml_name(event.name().as_ref()) == b"p" =>
            {
                push_docx_newline(&mut text);
            }
            Ok(_) => {}
            Err(error) => return Err(error.to_string()),
        }
    }

    Ok(text.trim_end_matches('\n').to_owned())
}

fn decode_xml_general_ref(reference: &str) -> Result<String, String> {
    match reference {
        "amp" => return Ok("&".to_owned()),
        "lt" => return Ok("<".to_owned()),
        "gt" => return Ok(">".to_owned()),
        "quot" => return Ok("\"".to_owned()),
        "apos" => return Ok("'".to_owned()),
        _ => {}
    }

    let value = if let Some(hex) = reference.strip_prefix("#x") {
        u32::from_str_radix(hex, 16).map_err(|error| error.to_string())?
    } else if let Some(decimal) = reference.strip_prefix('#') {
        decimal.parse::<u32>().map_err(|error| error.to_string())?
    } else {
        return Err(format!("unsupported XML entity reference '&{reference};'"));
    };
    char::from_u32(value)
        .map(|character| character.to_string())
        .ok_or_else(|| format!("invalid XML character reference '&{reference};'"))
}

fn local_xml_name(name: &[u8]) -> &[u8] {
    name.rsplit(|byte| *byte == b':').next().unwrap_or(name)
}

fn push_docx_newline(text: &mut String) {
    if !text.ends_with('\n') {
        text.push('\n');
    }
}

fn truncate_read_document_content(content: &str) -> String {
    const MAX_CHARS: usize = 60_000;
    if content.chars().count() <= MAX_CHARS {
        return content.to_owned();
    }

    let mut truncated = content.chars().take(MAX_CHARS).collect::<String>();
    truncated.push_str("\n[...truncated]");
    truncated
}

fn read_image_file_action(file_name: &str) -> Result<ActionResult, BrowserError> {
    let bytes =
        std::fs::read(file_name).map_err(|error| BrowserError::ActionFailed(error.to_string()))?;
    let data = base64::engine::general_purpose::STANDARD.encode(bytes);
    let image_name = std::path::Path::new(file_name)
        .file_name()
        .and_then(std::ffi::OsStr::to_str)
        .unwrap_or(file_name)
        .to_owned();

    Ok(ActionResult {
        extracted_content: Some(format!("Read image file {file_name}.")),
        error: None,
        judgement: None,
        long_term_memory: Some(format!("Read image file {file_name}")),
        include_extracted_content_only_once: true,
        include_in_memory: true,
        is_done: false,
        success: None,
        attachments: Vec::new(),
        images: vec![serde_json::json!({
            "name": image_name,
            "data": data,
        })],
        metadata: None,
    })
}

fn replace_file_action(
    file_name: &str,
    old_str: &str,
    new_str: &str,
) -> Result<ActionResult, BrowserError> {
    let resolved_file = resolve_file_action_path(file_name, supported_text_extensions());
    if let Some(result) = validate_text_file_name(&resolved_file.display_name) {
        return Ok(result);
    }
    if old_str.is_empty() {
        return Ok(ActionResult::error(
            "Cannot replace empty string. Please provide a non-empty string to replace.",
        ));
    }
    let content = std::fs::read_to_string(&resolved_file.path)
        .map_err(|error| BrowserError::ActionFailed(error.to_string()))?;
    if !content.contains(old_str) {
        return Ok(ActionResult::error(format!(
            "Could not find text to replace in {}",
            resolved_file.display_name
        )));
    }
    let updated = content.replace(old_str, new_str);
    std::fs::write(&resolved_file.path, updated)
        .map_err(|error| BrowserError::ActionFailed(error.to_string()))?;
    Ok(ActionResult::extracted(format!(
        "Replaced text in file {}{}",
        resolved_file.display_name,
        resolved_file.correction_suffix()
    )))
}

pub const DEFAULT_FILE_SYSTEM_PATH: &str = "browseruse_agent_data";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct FileSystemState {
    pub files: BTreeMap<String, FileSystemStoredFile>,
    pub base_dir: String,
    pub extracted_content_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct FileSystemStoredFile {
    #[serde(rename = "type")]
    pub file_type: String,
    pub data: FileSystemFileData,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct FileSystemFileData {
    pub name: String,
    pub content: String,
}

#[derive(Debug, Clone)]
pub struct ManagedFileSystem {
    base_dir: std::path::PathBuf,
    data_dir: std::path::PathBuf,
    files: BTreeMap<String, FileSystemStoredFile>,
    extracted_content_count: usize,
}

impl ManagedFileSystem {
    pub fn new_in_temp() -> Result<Self, BrowserError> {
        let base_dir = std::env::temp_dir().join(format!("browser_use_agent_{}", Uuid::now_v7()));
        Self::new(base_dir)
    }

    pub fn new(base_dir: impl Into<std::path::PathBuf>) -> Result<Self, BrowserError> {
        let base_dir = base_dir.into();
        std::fs::create_dir_all(&base_dir)
            .map_err(|error| BrowserError::ActionFailed(error.to_string()))?;
        let data_dir = base_dir.join(DEFAULT_FILE_SYSTEM_PATH);
        if data_dir.exists() {
            std::fs::remove_dir_all(&data_dir)
                .map_err(|error| BrowserError::ActionFailed(error.to_string()))?;
        }
        std::fs::create_dir_all(&data_dir)
            .map_err(|error| BrowserError::ActionFailed(error.to_string()))?;

        let mut file_system = Self {
            base_dir,
            data_dir,
            files: BTreeMap::new(),
            extracted_content_count: 0,
        };
        file_system.write_stored_file("todo.md", "")?;
        Ok(file_system)
    }

    pub fn from_state(state: FileSystemState) -> Result<Self, BrowserError> {
        let base_dir = std::path::PathBuf::from(&state.base_dir);
        std::fs::create_dir_all(&base_dir)
            .map_err(|error| BrowserError::ActionFailed(error.to_string()))?;
        let data_dir = base_dir.join(DEFAULT_FILE_SYSTEM_PATH);
        if data_dir.exists() {
            std::fs::remove_dir_all(&data_dir)
                .map_err(|error| BrowserError::ActionFailed(error.to_string()))?;
        }
        std::fs::create_dir_all(&data_dir)
            .map_err(|error| BrowserError::ActionFailed(error.to_string()))?;

        let mut file_system = Self {
            base_dir,
            data_dir,
            files: BTreeMap::new(),
            extracted_content_count: state.extracted_content_count,
        };
        for (file_name, file) in state.files {
            if validate_write_file_name(&file_name).is_some() {
                continue;
            }
            file_system.sync_stored_file_to_disk(&file_name, &file.data.content)?;
            file_system.files.insert(file_name, file);
        }
        Ok(file_system)
    }

    pub fn base_dir(&self) -> &std::path::Path {
        &self.base_dir
    }

    pub fn data_dir(&self) -> &std::path::Path {
        &self.data_dir
    }

    pub fn list_files(&self) -> Vec<String> {
        self.files.keys().cloned().collect()
    }

    pub fn get_todo_contents(&self) -> String {
        self.files
            .get("todo.md")
            .map(|file| file.data.content.clone())
            .unwrap_or_default()
    }

    pub fn get_state(&self) -> FileSystemState {
        FileSystemState {
            files: self.files.clone(),
            base_dir: self.base_dir.display().to_string(),
            extracted_content_count: self.extracted_content_count,
        }
    }

    pub fn nuke(&mut self) -> Result<(), BrowserError> {
        if self.data_dir.exists() {
            std::fs::remove_dir_all(&self.data_dir)
                .map_err(|error| BrowserError::ActionFailed(error.to_string()))?;
        }
        self.files.clear();
        Ok(())
    }

    pub fn save_extracted_content(&mut self, content: &str) -> Result<String, BrowserError> {
        let stem = format!("extracted_content_{}", self.extracted_content_count);
        let file_name = format!("{stem}.md");
        self.write_stored_file(&file_name, content)?;
        self.extracted_content_count += 1;
        Ok(file_name)
    }

    pub fn describe(&self) -> String {
        const DISPLAY_CHARS: usize = 400;
        let mut description = String::new();
        for (file_name, file) in &self.files {
            if file_name == "todo.md" {
                continue;
            }

            let content = &file.data.content;
            if content.is_empty() {
                description.push_str(&format!("<file>\n{file_name} - [empty file]\n</file>\n"));
                continue;
            }

            let lines = content.lines().collect::<Vec<_>>();
            let line_count = lines.len();
            let whole_file_description = format!(
                "<file>\n{file_name} - {line_count} lines\n<content>\n{content}\n</content>\n</file>\n"
            );
            if content.chars().count() < DISPLAY_CHARS * 3 / 2 {
                description.push_str(&whole_file_description);
                continue;
            }

            let (start_preview, start_line_count) =
                preview_lines(lines.iter().copied(), DISPLAY_CHARS / 2);
            let (end_preview, end_line_count) =
                preview_lines(lines.iter().rev().copied(), DISPLAY_CHARS / 2);
            let middle_line_count = line_count.saturating_sub(start_line_count + end_line_count);
            if middle_line_count == 0 {
                description.push_str(&whole_file_description);
                continue;
            }

            let end_preview = end_preview
                .lines()
                .rev()
                .collect::<Vec<_>>()
                .join("\n")
                .trim()
                .to_owned();
            description.push_str(&format!(
                "<file>\n{file_name} - {line_count} lines\n<content>\n{}\n... {middle_line_count} more lines ...\n{end_preview}\n</content>\n</file>\n",
                start_preview.trim()
            ));
        }
        description.trim_end_matches('\n').to_owned()
    }

    pub fn display_file(&self, file_name: &str) -> Option<String> {
        let resolved_file = resolve_file_action_path_at(
            file_name,
            supported_text_extensions(),
            Some(&self.data_dir),
        );
        if validate_text_file_name(&resolved_file.display_name).is_some() {
            return None;
        }
        self.files
            .get(&resolved_file.display_name)
            .map(|file| file.data.content.clone())
    }

    pub fn display_done_file(&self, file_name: &str) -> Option<(String, String)> {
        if std::path::Path::new(file_name).is_absolute() {
            return display_done_file(file_name);
        }

        let resolved_file = resolve_file_action_path_at(
            file_name,
            supported_text_extensions(),
            Some(&self.data_dir),
        );
        if validate_text_file_name(&resolved_file.display_name).is_some() {
            return None;
        }

        let content = self
            .files
            .get(&resolved_file.display_name)
            .map(|file| file.data.content.clone())?;
        let attachment = std::fs::canonicalize(&resolved_file.path)
            .unwrap_or_else(|_| resolved_file.path.clone())
            .display()
            .to_string();
        Some((
            format!("{}:\n{content}", resolved_file.display_name),
            attachment,
        ))
    }

    pub fn upload_file_path(&self, file_name: &str) -> Option<std::path::PathBuf> {
        if std::path::Path::new(file_name).is_absolute() {
            return None;
        }

        let resolved_file = resolve_file_action_path_at(
            file_name,
            supported_write_extensions(),
            Some(&self.data_dir),
        );
        if validate_write_file_name(&resolved_file.display_name).is_some()
            || !self.files.contains_key(&resolved_file.display_name)
        {
            return None;
        }

        Some(resolved_file.path)
    }

    pub fn write_file(
        &mut self,
        params: &browser_use_tools::WriteFileAction,
    ) -> Result<ActionResult, BrowserError> {
        if std::path::Path::new(&params.file_name).is_absolute() {
            return write_file_action(params);
        }

        let resolved_file = resolve_file_action_path_at(
            &params.file_name,
            supported_write_extensions(),
            Some(&self.data_dir),
        );
        if let Some(result) = validate_write_file_name(&resolved_file.display_name) {
            return Ok(result);
        }
        if params.append && !self.files.contains_key(&resolved_file.display_name) {
            return Ok(ActionResult::error(format!(
                "File '{}' not found.",
                resolved_file.display_name
            )));
        }

        let mut content = params.content.clone();
        if params.trailing_newline {
            content.push('\n');
        }
        if params.leading_newline {
            content.insert(0, '\n');
        }

        let stored_content = if params.append {
            let existing = self
                .files
                .get(&resolved_file.display_name)
                .map(|file| file.data.content.as_str())
                .unwrap_or_default();
            merge_managed_append_content(&resolved_file.display_name, existing, &content)
        } else if is_csv_file(&resolved_file.display_name) {
            normalize_csv_content(&content)
        } else {
            content
        };

        self.write_stored_file(&resolved_file.display_name, &stored_content)?;
        Ok(ActionResult::extracted(format!(
            "{} file {}{}",
            if params.append {
                "Appended to"
            } else {
                "Wrote"
            },
            resolved_file.display_name,
            resolved_file.correction_suffix()
        )))
    }

    pub fn read_file(&self, file_name: &str) -> Result<ActionResult, BrowserError> {
        if std::path::Path::new(file_name).is_absolute() {
            return read_file_action(file_name);
        }

        let read_extensions = supported_read_extensions();
        let resolved_file =
            resolve_file_action_path_at(file_name, &read_extensions, Some(&self.data_dir));
        if let Some(result) = validate_read_file_name(&resolved_file.display_name) {
            return Ok(result);
        }
        let Some(file) = self.files.get(&resolved_file.display_name) else {
            return Ok(ActionResult::error(format!(
                "File '{}' not found.{}",
                resolved_file.display_name,
                if resolved_file.was_corrected {
                    format!(
                        " (Filename was auto-corrected from '{}')",
                        resolved_file.original_name
                    )
                } else {
                    String::new()
                }
            )));
        };
        let content = &file.data.content;
        let memory = read_file_memory(content);
        Ok(ActionResult {
            extracted_content: Some(format!(
                "{}Read from file {}.\n<content>\n{content}\n</content>",
                resolved_file.correction_prefix(),
                resolved_file.display_name
            )),
            error: None,
            judgement: None,
            long_term_memory: Some(memory),
            include_extracted_content_only_once: true,
            include_in_memory: true,
            is_done: false,
            success: None,
            attachments: Vec::new(),
            images: Vec::new(),
            metadata: None,
        })
    }

    pub fn replace_file(
        &mut self,
        file_name: &str,
        old_str: &str,
        new_str: &str,
    ) -> Result<ActionResult, BrowserError> {
        if std::path::Path::new(file_name).is_absolute() {
            return replace_file_action(file_name, old_str, new_str);
        }

        let resolved_file = resolve_file_action_path_at(
            file_name,
            supported_text_extensions(),
            Some(&self.data_dir),
        );
        if let Some(result) = validate_text_file_name(&resolved_file.display_name) {
            return Ok(result);
        }
        if old_str.is_empty() {
            return Ok(ActionResult::error(
                "Cannot replace empty string. Please provide a non-empty string to replace.",
            ));
        }
        let Some(existing) = self.files.get(&resolved_file.display_name) else {
            return Ok(ActionResult::error(format!(
                "File '{}' not found.",
                resolved_file.display_name
            )));
        };
        if !existing.data.content.contains(old_str) {
            return Ok(ActionResult::error(format!(
                "Could not find text to replace in {}",
                resolved_file.display_name
            )));
        }
        let updated = existing.data.content.replace(old_str, new_str);
        self.write_stored_file(&resolved_file.display_name, &updated)?;
        Ok(ActionResult::extracted(format!(
            "Replaced text in file {}{}",
            resolved_file.display_name,
            resolved_file.correction_suffix()
        )))
    }

    fn write_stored_file(&mut self, file_name: &str, content: &str) -> Result<(), BrowserError> {
        self.sync_stored_file_to_disk(file_name, content)?;
        self.files
            .insert(file_name.to_owned(), stored_file_state(file_name, content)?);
        Ok(())
    }

    fn sync_stored_file_to_disk(&self, file_name: &str, content: &str) -> Result<(), BrowserError> {
        let path = self.data_dir.join(file_name);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|error| BrowserError::ActionFailed(error.to_string()))?;
        }
        write_supported_artifact(&path, file_name, content)
    }
}

fn preview_lines<'a>(lines: impl Iterator<Item = &'a str>, max_chars: usize) -> (String, usize) {
    let mut preview = String::new();
    let mut line_count = 0;
    let mut chars_count = 0;
    for line in lines {
        let next = line.chars().count() + 1;
        if chars_count + next > max_chars {
            break;
        }
        preview.push_str(line);
        preview.push('\n');
        chars_count += next;
        line_count += 1;
    }
    (preview, line_count)
}

fn merge_managed_append_content(file_name: &str, existing: &str, new_content: &str) -> String {
    if is_pdf_file(file_name) {
        merge_pdf_append_content(existing, new_content)
    } else if is_docx_file(file_name) {
        merge_docx_append_content(existing, new_content)
    } else if is_csv_file(file_name) {
        merge_csv_append_content(existing, new_content)
    } else {
        let mut merged = existing.to_owned();
        merged.push_str(new_content);
        merged
    }
}

fn stored_file_state(file_name: &str, content: &str) -> Result<FileSystemStoredFile, BrowserError> {
    let Some((name, extension)) = file_name.rsplit_once('.') else {
        return Err(BrowserError::ActionFailed(format!(
            "Filename '{file_name}' has no extension"
        )));
    };
    let file_type = file_type_for_extension(extension).ok_or_else(|| {
        BrowserError::ActionFailed(format!("Unsupported managed file extension '.{extension}'"))
    })?;
    Ok(FileSystemStoredFile {
        file_type: file_type.to_owned(),
        data: FileSystemFileData {
            name: name.to_owned(),
            content: content.to_owned(),
        },
    })
}

fn file_type_for_extension(extension: &str) -> Option<&'static str> {
    match extension.to_ascii_lowercase().as_str() {
        "md" => Some("MarkdownFile"),
        "txt" => Some("TxtFile"),
        "json" => Some("JsonFile"),
        "jsonl" => Some("JsonlFile"),
        "csv" => Some("CsvFile"),
        "pdf" => Some("PdfFile"),
        "docx" => Some("DocxFile"),
        "html" => Some("HtmlFile"),
        "xml" => Some("XmlFile"),
        _ => None,
    }
}

fn write_supported_artifact(
    path: &std::path::Path,
    file_name: &str,
    content: &str,
) -> Result<(), BrowserError> {
    if is_pdf_file(file_name) {
        write_pdf_text(path, content).map_err(BrowserError::ActionFailed)
    } else if is_docx_file(file_name) {
        write_docx_text(path, content).map_err(BrowserError::ActionFailed)
    } else {
        std::fs::write(path, content).map_err(|error| BrowserError::ActionFailed(error.to_string()))
    }
}

#[derive(Debug, Clone)]
struct ResolvedFileActionPath {
    path: std::path::PathBuf,
    display_name: String,
    original_name: String,
    was_corrected: bool,
}

impl ResolvedFileActionPath {
    fn path_string(&self) -> String {
        self.path.to_string_lossy().into_owned()
    }

    fn correction_suffix(&self) -> String {
        if self.was_corrected {
            format!(" (auto-corrected from '{}')", self.original_name)
        } else {
            String::new()
        }
    }

    fn correction_prefix(&self) -> String {
        if self.was_corrected {
            format!(
                "Note: filename was auto-corrected from '{}' to '{}'. ",
                self.original_name, self.display_name
            )
        } else {
            String::new()
        }
    }
}

fn apply_file_name_correction_note(
    result: &mut ActionResult,
    resolved_file: &ResolvedFileActionPath,
) {
    if !resolved_file.was_corrected {
        return;
    }
    if let Some(content) = result.extracted_content.as_mut() {
        content.insert_str(0, &resolved_file.correction_prefix());
    }
    if let Some(memory) = result.long_term_memory.as_mut() {
        memory.insert_str(0, &resolved_file.correction_prefix());
    }
}

fn resolve_file_action_path(
    file_name: &str,
    supported_extensions: &[&str],
) -> ResolvedFileActionPath {
    resolve_file_action_path_at(file_name, supported_extensions, None)
}

fn resolve_file_action_path_at(
    file_name: &str,
    supported_extensions: &[&str],
    relative_root: Option<&std::path::Path>,
) -> ResolvedFileActionPath {
    let path = std::path::Path::new(file_name);
    if path.is_absolute() {
        return ResolvedFileActionPath {
            path: path.to_path_buf(),
            display_name: file_name.to_owned(),
            original_name: file_name.to_owned(),
            was_corrected: false,
        };
    }

    let base_name = path
        .file_name()
        .and_then(std::ffi::OsStr::to_str)
        .unwrap_or(file_name);
    let mut display_name = base_name.to_owned();
    let mut was_corrected = base_name != file_name;

    if !is_valid_action_file_name(&display_name, supported_extensions) {
        let sanitized = sanitize_action_file_name(&display_name);
        if sanitized != display_name && is_valid_action_file_name(&sanitized, supported_extensions)
        {
            display_name = sanitized;
            was_corrected = true;
        }
    }

    let path = relative_root
        .map(|root| root.join(&display_name))
        .unwrap_or_else(|| std::path::PathBuf::from(&display_name));

    ResolvedFileActionPath {
        path,
        display_name,
        original_name: file_name.to_owned(),
        was_corrected,
    }
}

fn is_valid_action_file_name(file_name: &str, supported_extensions: &[&str]) -> bool {
    let Some((name, extension)) = file_name.rsplit_once('.') else {
        return false;
    };
    if name.trim().is_empty() {
        return false;
    }
    let extension = extension.to_ascii_lowercase();
    if !supported_extensions.contains(&extension.as_str()) {
        return false;
    }
    name.chars().all(is_valid_action_file_name_char)
}

fn is_valid_action_file_name_char(character: char) -> bool {
    character.is_ascii_alphanumeric()
        || matches!(character, '_' | '-' | '.' | '(' | ')' | ' ')
        || ('\u{4e00}'..='\u{9fff}').contains(&character)
}

fn sanitize_action_file_name(file_name: &str) -> String {
    let Some((name, extension)) = file_name.rsplit_once('.') else {
        return file_name.to_owned();
    };
    let mut sanitized_name = name
        .replace(' ', "-")
        .chars()
        .filter(|character| {
            character.is_ascii_alphanumeric()
                || matches!(*character, '_' | '-' | '.' | '(' | ')')
                || ('\u{4e00}'..='\u{9fff}').contains(character)
        })
        .collect::<String>();
    while sanitized_name.contains("--") {
        sanitized_name = sanitized_name.replace("--", "-");
    }
    sanitized_name = sanitized_name
        .trim_matches(|character| character == '-' || character == '.')
        .to_owned();
    if sanitized_name.is_empty() {
        sanitized_name = "file".to_owned();
    }
    format!("{}.{}", sanitized_name, extension.to_ascii_lowercase())
}

fn validate_write_file_name(file_name: &str) -> Option<ActionResult> {
    let path = std::path::Path::new(file_name);
    let base_name = path
        .file_name()
        .and_then(std::ffi::OsStr::to_str)
        .unwrap_or(file_name);
    let Some(extension) = path.extension().and_then(std::ffi::OsStr::to_str) else {
        return Some(ActionResult::error(format!(
            "Filename '{base_name}' has no extension. Supported extensions: {}.",
            supported_write_extensions_message()
        )));
    };
    let extension = extension.to_ascii_lowercase();

    if unsupported_binary_extensions().contains(&extension.as_str()) {
        return Some(ActionResult::error(format!(
            "Cannot write binary/image file '{base_name}'. The write_file action supports text files and PDF/DOCX documents. Supported extensions: {}.",
            supported_write_extensions_message()
        )));
    }

    if !supported_write_extensions().contains(&extension.as_str()) {
        return Some(ActionResult::error(format!(
            "Unsupported file extension '.{extension}' in '{base_name}'. Supported extensions: {}.",
            supported_write_extensions_message()
        )));
    }

    None
}

fn validate_text_file_name(file_name: &str) -> Option<ActionResult> {
    let path = std::path::Path::new(file_name);
    let base_name = path
        .file_name()
        .and_then(std::ffi::OsStr::to_str)
        .unwrap_or(file_name);
    let Some(extension) = path.extension().and_then(std::ffi::OsStr::to_str) else {
        return Some(ActionResult::error(format!(
            "Filename '{base_name}' has no extension. Supported extensions: {}.",
            supported_text_extensions_message()
        )));
    };
    let extension = extension.to_ascii_lowercase();

    if unsupported_binary_extensions().contains(&extension.as_str()) {
        return Some(ActionResult::error(format!(
            "Cannot write binary/image file '{base_name}'. The file actions only support text-based files. Supported extensions: {}.",
            supported_text_extensions_message()
        )));
    }

    if !supported_text_extensions().contains(&extension.as_str()) {
        return Some(ActionResult::error(format!(
            "Unsupported file extension '.{extension}' in '{base_name}'. Supported extensions: {}.",
            supported_text_extensions_message()
        )));
    }

    None
}

fn validate_read_file_name(file_name: &str) -> Option<ActionResult> {
    let path = std::path::Path::new(file_name);
    let base_name = path
        .file_name()
        .and_then(std::ffi::OsStr::to_str)
        .unwrap_or(file_name);
    let Some(extension) = path.extension().and_then(std::ffi::OsStr::to_str) else {
        return Some(ActionResult::error(format!(
            "Filename '{base_name}' has no extension. Supported extensions: {}.",
            supported_read_extensions_message()
        )));
    };
    let extension = extension.to_ascii_lowercase();

    if supported_text_extensions().contains(&extension.as_str())
        || supported_read_image_extensions().contains(&extension.as_str())
        || supported_read_document_extensions().contains(&extension.as_str())
    {
        return None;
    }

    if unsupported_binary_extensions().contains(&extension.as_str()) {
        return Some(ActionResult::error(format!(
            "Cannot read binary/image file '{base_name}'. The read_file action supports text files, PDF/DOCX documents, and PNG/JPEG images. Supported extensions: {}.",
            supported_read_extensions_message()
        )));
    }

    Some(ActionResult::error(format!(
        "Unsupported file extension '.{extension}' in '{base_name}'. Supported extensions: {}.",
        supported_read_extensions_message()
    )))
}

fn supported_text_extensions() -> &'static [&'static str] {
    &["txt", "md", "json", "jsonl", "csv", "html", "xml"]
}

fn supported_write_extensions() -> &'static [&'static str] {
    &[
        "txt", "md", "json", "jsonl", "csv", "html", "xml", "pdf", "docx",
    ]
}

fn supported_read_image_extensions() -> &'static [&'static str] {
    &["png", "jpg", "jpeg"]
}

fn supported_read_document_extensions() -> &'static [&'static str] {
    &["pdf", "docx"]
}

fn supported_read_extensions() -> Vec<&'static str> {
    supported_text_extensions()
        .iter()
        .chain(supported_read_document_extensions().iter())
        .chain(supported_read_image_extensions().iter())
        .copied()
        .collect()
}

fn is_supported_read_image_file(file_name: &str) -> bool {
    file_extension(file_name)
        .is_some_and(|extension| supported_read_image_extensions().contains(&extension.as_str()))
}

fn is_supported_read_document_file(file_name: &str) -> bool {
    file_extension(file_name)
        .is_some_and(|extension| supported_read_document_extensions().contains(&extension.as_str()))
}

fn file_extension(file_name: &str) -> Option<String> {
    std::path::Path::new(file_name)
        .extension()
        .and_then(std::ffi::OsStr::to_str)
        .map(str::to_ascii_lowercase)
}

fn unsupported_binary_extensions() -> &'static [&'static str] {
    &[
        "png", "jpg", "jpeg", "gif", "bmp", "svg", "webp", "ico", "mp3", "mp4", "wav", "avi",
        "mov", "zip", "tar", "gz", "rar", "exe", "bin", "dll", "so",
    ]
}

fn supported_text_extensions_message() -> String {
    supported_text_extensions()
        .iter()
        .map(|extension| format!(".{extension}"))
        .collect::<Vec<_>>()
        .join(", ")
}

fn supported_write_extensions_message() -> String {
    supported_write_extensions()
        .iter()
        .map(|extension| format!(".{extension}"))
        .collect::<Vec<_>>()
        .join(", ")
}

fn supported_read_extensions_message() -> String {
    supported_text_extensions()
        .iter()
        .chain(supported_read_document_extensions().iter())
        .chain(supported_read_image_extensions().iter())
        .map(|extension| format!(".{extension}"))
        .collect::<Vec<_>>()
        .join(", ")
}

fn read_file_memory(content: &str) -> String {
    const MAX_MEMORY_SIZE: usize = 1_000;
    if content.len() <= MAX_MEMORY_SIZE {
        return content.to_owned();
    }

    let mut display = String::new();
    let mut lines_count = 0;
    let lines: Vec<&str> = content.lines().collect();
    for line in &lines {
        if display.len() + line.len() + 1 < MAX_MEMORY_SIZE {
            display.push_str(line);
            display.push('\n');
            lines_count += 1;
        } else {
            break;
        }
    }
    let remaining_lines = lines.len().saturating_sub(lines_count);
    if remaining_lines > 0 {
        format!("{display}{remaining_lines} more lines...")
    } else {
        display
    }
}

fn default_find_element_attributes() -> Vec<String> {
    [
        "id",
        "class",
        "name",
        "type",
        "placeholder",
        "href",
        "aria-label",
        "role",
        "title",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect()
}

fn truncate_evaluate_result(result: String) -> String {
    const MAX_CHARS: usize = 20_000;
    const PREFIX_CHARS: usize = 19_950;

    if result.chars().count() <= MAX_CHARS {
        return result;
    }

    let mut truncated: String = result.chars().take(PREFIX_CHARS).collect();
    truncated.push_str("\n... [Truncated after 20000 characters]");
    truncated
}

#[must_use]
pub fn search_url(engine: &SearchEngine, query: &str) -> String {
    let encoded: String = form_urlencoded::byte_serialize(query.as_bytes()).collect();
    match engine {
        SearchEngine::DuckDuckGo => format!("https://duckduckgo.com/?q={encoded}"),
        SearchEngine::Google => format!("https://www.google.com/search?q={encoded}&udm=14"),
        SearchEngine::Bing => format!("https://www.bing.com/search?q={encoded}"),
    }
}

struct SearchTextResult {
    matches: Vec<String>,
    total: usize,
    has_more: bool,
}

fn search_text_matches(
    text: &str,
    pattern: &str,
    regex: bool,
    case_sensitive: bool,
    context_chars: usize,
    max_results: usize,
) -> Result<SearchTextResult, String> {
    if pattern.is_empty() || max_results == 0 {
        return Ok(SearchTextResult {
            matches: Vec::new(),
            total: 0,
            has_more: false,
        });
    }

    let pattern = if regex {
        pattern.to_owned()
    } else {
        regex::escape(pattern)
    };
    let matcher = regex::RegexBuilder::new(&pattern)
        .case_insensitive(!case_sensitive)
        .build()
        .map_err(|error| error.to_string())?;

    let mut matches = Vec::new();
    let mut total = 0;
    for hit in matcher.find_iter(text) {
        total += 1;
        if matches.len() < max_results {
            matches.push(context_snippet(text, hit.start(), hit.end(), context_chars));
        }
    }

    Ok(SearchTextResult {
        has_more: total > matches.len(),
        matches,
        total,
    })
}

fn format_search_page_results(pattern: &str, result: &SearchTextResult) -> String {
    if result.total == 0 {
        return format!("No matches found for \"{pattern}\" on page.");
    }

    let mut lines = vec![format!(
        "Found {} {} for \"{pattern}\" on page:",
        result.total,
        if result.total == 1 {
            "match"
        } else {
            "matches"
        }
    )];
    lines.push(String::new());
    lines.extend(
        result
            .matches
            .iter()
            .enumerate()
            .map(|(index, context)| format!("[{}] {context}", index + 1)),
    );

    if result.has_more {
        lines.push(format!(
            "\n... showing {} of {} total matches. Increase max_results to see more.",
            result.matches.len(),
            result.total
        ));
    }

    lines.join("\n")
}

fn format_find_elements_results(selector: &str, elements: &[FoundElement]) -> String {
    if elements.is_empty() {
        return format!("No elements found matching \"{selector}\".");
    }

    let mut lines = vec![format!(
        "Found {} {} matching \"{selector}\":",
        elements.len(),
        if elements.len() == 1 {
            "element"
        } else {
            "elements"
        }
    )];
    lines.push(String::new());
    lines.extend(
        elements
            .iter()
            .enumerate()
            .map(|(index, element)| format_found_element(index, element)),
    );
    lines.join("\n")
}

fn format_found_element(index: usize, element: &FoundElement) -> String {
    let mut parts = vec![format!("[{index}] <{}>", element.tag_name)];
    if let Some(text) = element
        .text
        .as_deref()
        .map(collapse_whitespace)
        .filter(|text| !text.is_empty())
    {
        parts.push(format!("\"{}\"", truncate_chars(&text, 120)));
    }
    if !element.attributes.is_empty() {
        let attrs = element
            .attributes
            .iter()
            .map(|(key, value)| format!("{key}=\"{}\"", truncate_chars(value, 500)))
            .collect::<Vec<_>>()
            .join(", ");
        parts.push(format!("{{{attrs}}}"));
    }
    parts.join(" ")
}

fn collapse_whitespace(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn truncate_chars(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_owned();
    }
    let mut truncated = text.chars().take(max_chars).collect::<String>();
    truncated.push_str("...");
    truncated
}

fn context_snippet(text: &str, start: usize, end: usize, context_chars: usize) -> String {
    let has_prefix = text[..start].chars().count() > context_chars;
    let has_suffix = text[end..].chars().count() > context_chars;
    let prefix: String = text[..start]
        .chars()
        .rev()
        .take(context_chars)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    let suffix: String = text[end..].chars().take(context_chars).collect();
    format!(
        "{}{prefix}{}{suffix}{}",
        if has_prefix { "..." } else { "" },
        &text[start..end],
        if has_suffix { "..." } else { "" }
    )
}

#[derive(Debug, Error)]
pub enum AgentRunError {
    #[error(transparent)]
    Browser(#[from] BrowserError),
    #[error(transparent)]
    Llm(#[from] LlmError),
    #[error("invalid agent output: {0}")]
    InvalidOutput(String),
    #[error("LLM call timed out after {seconds} seconds")]
    LlmTimedOut { seconds: u64 },
    #[error("agent step timed out after {seconds} seconds")]
    StepTimedOut { seconds: u64 },
    #[error("agent reached max steps ({max_steps}) without completing")]
    StepLimitReached { max_steps: usize },
    #[error("agent stopped after {failures} consecutive failures")]
    MaxFailuresExceeded { failures: u32 },
    #[error("agent repeated the same action sequence for {window} steps")]
    LoopDetected { window: usize },
    #[error("agent stopped before the next step: {reason}")]
    Stopped { reason: String },
    #[error("agent callback {callback} failed: {message}")]
    Callback {
        callback: &'static str,
        message: String,
    },
    #[error("failed to save conversation to {path}: {source}")]
    ConversationSave {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("unsupported conversation transcript encoding {encoding:?}")]
    ConversationEncoding { encoding: String },
    #[error("conversation transcript encoding {encoding:?} cannot represent the transcript text")]
    ConversationEncodingLossy { encoding: String },
    #[error("failed to save agent GIF at {path}: {message}")]
    GifSave { path: String, message: String },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct AgentCheckpoint {
    pub task: String,
    pub settings: AgentSettings,
    pub history: AgentHistory,
    pub initial_actions_executed: bool,
    pub file_system_state: FileSystemState,
}

pub type AgentCallbackFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T, String>> + Send + 'a>>;
pub type AgentStepCallback = Box<
    dyn for<'a> FnMut(
            &'a BrowserStateSummary,
            &'a AgentOutput,
            usize,
        ) -> AgentCallbackFuture<'a, ()>
        + Send
        + 'static,
>;
pub type AgentDoneCallback =
    Box<dyn for<'a> FnMut(&'a AgentHistory) -> AgentCallbackFuture<'a, ()> + Send + 'static>;
pub type AgentShouldStopCallback =
    Box<dyn FnMut() -> AgentCallbackFuture<'static, bool> + Send + 'static>;

pub struct Agent<M, S> {
    id: Uuid,
    task: String,
    settings: AgentSettings,
    llm: M,
    executor: BrowserActionExecutor<S>,
    history: AgentHistory,
    initial_actions_executed: bool,
    stopped: bool,
    step_callbacks: Vec<AgentStepCallback>,
    done_callbacks: Vec<AgentDoneCallback>,
    should_stop_callback: Option<AgentShouldStopCallback>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StepRequestKind {
    Normal,
    BudgetWarning { steps_used: usize, max_steps: usize },
    FinalStep { max_steps: usize },
}

impl<M, S> Agent<M, S>
where
    M: ChatModel,
    S: BrowserSession + Send + Sync,
{
    #[must_use]
    pub fn new(task: impl Into<String>, llm: M, session: S) -> Self {
        Self::with_settings(task, AgentSettings::default(), llm, session)
    }

    #[must_use]
    pub fn with_settings(
        task: impl Into<String>,
        settings: AgentSettings,
        llm: M,
        session: S,
    ) -> Self {
        Self::with_settings_and_file_system(
            task,
            settings,
            llm,
            session,
            ManagedFileSystem::new_in_temp().expect("create managed file system"),
        )
    }

    #[must_use]
    pub fn with_settings_and_file_system(
        task: impl Into<String>,
        settings: AgentSettings,
        llm: M,
        session: S,
        file_system: ManagedFileSystem,
    ) -> Self {
        let task = task.into();
        let settings = settings_with_direct_start_url(&task, settings);
        let mut executor = BrowserActionExecutor::with_file_system(session, file_system);
        executor.set_display_files_in_done_text(settings.display_files_in_done_text);
        executor.set_action_timeout_seconds(settings.action_timeout_seconds);
        executor.set_upload_file_availability(true, settings.available_file_paths.clone());
        Self {
            id: Uuid::now_v7(),
            task,
            settings,
            llm,
            executor,
            history: AgentHistory::default(),
            initial_actions_executed: false,
            stopped: false,
            step_callbacks: Vec::new(),
            done_callbacks: Vec::new(),
            should_stop_callback: None,
        }
    }

    pub fn from_checkpoint(
        checkpoint: AgentCheckpoint,
        llm: M,
        session: S,
    ) -> Result<Self, BrowserError> {
        let file_system = ManagedFileSystem::from_state(checkpoint.file_system_state)?;
        let mut executor = BrowserActionExecutor::with_file_system(session, file_system);
        executor.set_display_files_in_done_text(checkpoint.settings.display_files_in_done_text);
        executor.set_action_timeout_seconds(checkpoint.settings.action_timeout_seconds);
        executor
            .set_upload_file_availability(true, checkpoint.settings.available_file_paths.clone());
        Ok(Self {
            id: Uuid::now_v7(),
            task: checkpoint.task,
            settings: checkpoint.settings,
            llm,
            executor,
            history: checkpoint.history,
            initial_actions_executed: checkpoint.initial_actions_executed,
            stopped: false,
            step_callbacks: Vec::new(),
            done_callbacks: Vec::new(),
            should_stop_callback: None,
        })
    }

    pub fn history(&self) -> &AgentHistory {
        &self.history
    }

    pub fn id(&self) -> Uuid {
        self.id
    }

    pub fn checkpoint(&self) -> AgentCheckpoint {
        AgentCheckpoint {
            task: self.task.clone(),
            settings: self.settings.clone(),
            history: self.history.clone(),
            initial_actions_executed: self.initial_actions_executed,
            file_system_state: self.file_system_state(),
        }
    }

    pub fn file_system(&self) -> &ManagedFileSystem {
        self.executor.file_system()
    }

    pub fn file_system_mut(&mut self) -> &mut ManagedFileSystem {
        self.executor.file_system_mut()
    }

    pub fn file_system_state(&self) -> FileSystemState {
        self.executor.file_system().get_state()
    }

    pub fn register_new_step_callback<F, E>(&mut self, mut callback: F)
    where
        F: FnMut(&BrowserStateSummary, &AgentOutput, usize) -> Result<(), E> + Send + 'static,
        E: ToString + 'static,
    {
        self.register_new_step_callback_async(move |state, output, step| {
            let result = callback(state, output, step).map_err(|error| error.to_string());
            Box::pin(async move { result })
        });
    }

    pub fn register_new_step_callback_async<F>(&mut self, callback: F)
    where
        F: for<'a> FnMut(
                &'a BrowserStateSummary,
                &'a AgentOutput,
                usize,
            ) -> AgentCallbackFuture<'a, ()>
            + Send
            + 'static,
    {
        self.step_callbacks.push(Box::new(callback));
    }

    pub fn clear_new_step_callbacks(&mut self) {
        self.step_callbacks.clear();
    }

    pub fn register_done_callback<F, E>(&mut self, mut callback: F)
    where
        F: FnMut(&AgentHistory) -> Result<(), E> + Send + 'static,
        E: ToString + 'static,
    {
        self.register_done_callback_async(move |history| {
            let result = callback(history).map_err(|error| error.to_string());
            Box::pin(async move { result })
        });
    }

    pub fn register_done_callback_async<F>(&mut self, callback: F)
    where
        F: for<'a> FnMut(&'a AgentHistory) -> AgentCallbackFuture<'a, ()> + Send + 'static,
    {
        self.done_callbacks.push(Box::new(callback));
    }

    pub fn clear_done_callbacks(&mut self) {
        self.done_callbacks.clear();
    }

    pub fn register_should_stop_callback<F, E>(&mut self, mut callback: F)
    where
        F: FnMut() -> Result<bool, E> + Send + 'static,
        E: ToString + 'static,
    {
        self.register_should_stop_callback_async(move || {
            let result = callback().map_err(|error| error.to_string());
            Box::pin(async move { result })
        });
    }

    pub fn register_should_stop_callback_async<F>(&mut self, callback: F)
    where
        F: FnMut() -> AgentCallbackFuture<'static, bool> + Send + 'static,
    {
        self.should_stop_callback = Some(Box::new(callback));
    }

    pub fn clear_should_stop_callback(&mut self) {
        self.should_stop_callback = None;
    }

    pub fn stop(&mut self) {
        self.stopped = true;
    }

    pub fn is_stopped(&self) -> bool {
        self.stopped
    }

    async fn check_stop_requested(&mut self) -> Result<(), AgentRunError> {
        if self.stopped {
            return Err(AgentRunError::Stopped {
                reason: "stop requested".to_owned(),
            });
        }

        if let Some(callback) = self.should_stop_callback.as_mut() {
            let should_stop = callback()
                .await
                .map_err(|message| AgentRunError::Callback {
                    callback: "should_stop",
                    message,
                })?;
            if should_stop {
                self.stopped = true;
                return Err(AgentRunError::Stopped {
                    reason: "should_stop callback requested stop".to_owned(),
                });
            }
        }

        Ok(())
    }

    async fn invoke_step_callbacks(
        &mut self,
        state: &BrowserStateSummary,
        model_output: &AgentOutput,
        step_number: usize,
    ) -> Result<(), AgentRunError> {
        for callback in &mut self.step_callbacks {
            callback(state, model_output, step_number)
                .await
                .map_err(|message| AgentRunError::Callback {
                    callback: "new_step",
                    message,
                })?;
        }
        Ok(())
    }

    async fn invoke_done_callbacks(&mut self) -> Result<(), AgentRunError> {
        for callback in &mut self.done_callbacks {
            callback(&self.history)
                .await
                .map_err(|message| AgentRunError::Callback {
                    callback: "done",
                    message,
                })?;
        }
        Ok(())
    }

    pub async fn run(&mut self, max_steps: usize) -> Result<&AgentHistory, AgentRunError> {
        let mut consecutive_failures = 0;

        self.check_stop_requested().await?;
        self.execute_initial_actions().await?;
        if self
            .history
            .items
            .last()
            .is_some_and(|item| item.result.iter().any(|result| result.is_done))
        {
            return self.finish_successful_run().await;
        }

        for step_index in 0..max_steps {
            self.check_stop_requested().await?;
            let (is_done, has_error) = {
                let seconds = self.settings.step_timeout_seconds;
                let steps_used = step_index + 1;
                let step_kind = if steps_used == max_steps {
                    StepRequestKind::FinalStep { max_steps }
                } else if should_inject_step_budget_warning(steps_used, max_steps) {
                    StepRequestKind::BudgetWarning {
                        steps_used,
                        max_steps,
                    }
                } else {
                    StepRequestKind::Normal
                };
                let item = timeout(
                    Duration::from_secs(seconds),
                    self.step_recovering_model_errors_with_kind(step_kind),
                )
                .await
                .map_err(|_| AgentRunError::StepTimedOut { seconds })??;
                (
                    item.result.iter().any(|result| result.is_done),
                    item.result.iter().any(|result| result.error.is_some()),
                )
            };

            if is_done {
                return self.finish_successful_run().await;
            }
            self.check_stop_requested().await?;

            if self.settings.loop_detection_enabled
                && repeated_action_loop(&self.history, self.settings.loop_detection_window)
            {
                return Err(AgentRunError::LoopDetected {
                    window: self.settings.loop_detection_window,
                });
            }

            if has_error {
                consecutive_failures += 1;
                if consecutive_failures >= self.settings.max_failures {
                    if self.settings.final_response_after_failure {
                        let final_item = self
                            .record_final_response_after_failure(consecutive_failures)
                            .await?;
                        let final_is_done = final_item.result.iter().any(|result| result.is_done);
                        if final_is_done {
                            return self.finish_successful_run().await;
                        }
                    }
                    return Err(AgentRunError::MaxFailuresExceeded {
                        failures: consecutive_failures,
                    });
                }
            } else {
                consecutive_failures = 0;
            }

            self.maybe_compact_history().await;
        }

        Err(AgentRunError::StepLimitReached { max_steps })
    }

    pub async fn step(&mut self) -> Result<&AgentHistoryItem, AgentRunError> {
        self.check_stop_requested().await?;
        let seconds = self.settings.step_timeout_seconds;
        timeout(Duration::from_secs(seconds), self.step_inner())
            .await
            .map_err(|_| AgentRunError::StepTimedOut { seconds })?
    }

    async fn step_inner(&mut self) -> Result<&AgentHistoryItem, AgentRunError> {
        let step_start_time = now_seconds();
        let include_screenshot = self.should_include_screenshot();
        let state = self.executor.session().state(include_screenshot).await?;
        let request = build_step_request_with_file_system(
            &self.task,
            &state,
            &self.history,
            &self.settings,
            Some(self.executor.file_system()),
        )?;
        let request_for_transcript = request.clone();
        let completion = timeout(
            Duration::from_secs(self.settings.llm_timeout_seconds),
            self.llm.invoke_json(request),
        )
        .await
        .map_err(|_| AgentRunError::LlmTimedOut {
            seconds: self.settings.llm_timeout_seconds,
        })??;
        let model_output: AgentOutput = serde_json::from_value(completion.content)
            .map_err(|error| AgentRunError::InvalidOutput(error.to_string()))?;
        self.save_conversation_snapshot(&request_for_transcript, &model_output)?;
        self.record_model_output(state, model_output, Some(step_start_time))
            .await
    }

    async fn step_recovering_model_errors_with_kind(
        &mut self,
        step_kind: StepRequestKind,
    ) -> Result<&AgentHistoryItem, AgentRunError> {
        self.check_stop_requested().await?;
        let step_start_time = now_seconds();
        let include_screenshot = self.should_include_screenshot();
        let state = self.executor.session().state(include_screenshot).await?;
        let request = match step_kind {
            StepRequestKind::Normal => build_step_request_with_file_system(
                &self.task,
                &state,
                &self.history,
                &self.settings,
                Some(self.executor.file_system()),
            )?,
            StepRequestKind::BudgetWarning {
                steps_used,
                max_steps,
            } => build_step_request_with_budget_warning(
                &self.task,
                &state,
                &self.history,
                &self.settings,
                Some(self.executor.file_system()),
                steps_used,
                max_steps,
            )?,
            StepRequestKind::FinalStep { max_steps } => {
                build_final_response_after_step_limit_request(
                    &self.task,
                    &state,
                    &self.history,
                    &self.settings,
                    Some(self.executor.file_system()),
                    max_steps,
                )?
            }
        };
        let request_for_transcript = request.clone();
        let completion = match timeout(
            Duration::from_secs(self.settings.llm_timeout_seconds),
            self.llm.invoke_json(request),
        )
        .await
        {
            Ok(Ok(completion)) => completion,
            Ok(Err(error)) => {
                return self.record_model_error(
                    state,
                    format!("LLM provider error: {error}"),
                    Some(step_start_time),
                );
            }
            Err(_) => {
                return self.record_model_error(
                    state,
                    format!(
                        "LLM call timed out after {} seconds",
                        self.settings.llm_timeout_seconds
                    ),
                    Some(step_start_time),
                );
            }
        };
        let model_output: AgentOutput = match serde_json::from_value(completion.content) {
            Ok(model_output) => model_output,
            Err(error) => {
                return self.record_model_error(
                    state,
                    format!("invalid agent output: {error}"),
                    Some(step_start_time),
                );
            }
        };
        self.save_conversation_snapshot(&request_for_transcript, &model_output)?;
        if matches!(step_kind, StepRequestKind::FinalStep { .. })
            && !is_single_done_output(&model_output)
        {
            return self.record_model_error(
                state,
                "final response at step limit must return exactly one done action".to_owned(),
                Some(step_start_time),
            );
        }
        self.record_model_output(state, model_output, Some(step_start_time))
            .await
    }

    async fn record_model_output(
        &mut self,
        state: BrowserStateSummary,
        mut model_output: AgentOutput,
        step_start_time: Option<f64>,
    ) -> Result<&AgentHistoryItem, AgentRunError> {
        if model_output.action.len() > self.settings.max_actions_per_step {
            model_output
                .action
                .truncate(self.settings.max_actions_per_step);
        }
        let step_number = latest_history_step_number(&self.history)
            .map(|step| step + 1)
            .unwrap_or(1);
        self.invoke_step_callbacks(&state, &model_output, step_number)
            .await?;
        self.check_stop_requested().await?;
        if let Some(error) = excluded_action_error(&model_output.action, &self.settings) {
            return self.record_model_error(state, error, step_start_time);
        }
        let actions = actions_for_execution(&model_output.action, &self.settings, &state.url);
        let result = self.execute_agent_sequence(&actions).await?;
        let metadata = step_start_time.map(|start| self.step_metadata(start, now_seconds()));

        self.history.items.push(AgentHistoryItem {
            model_output: Some(model_output),
            result,
            state,
            metadata,
        });

        self.history
            .items
            .last()
            .ok_or_else(|| AgentRunError::InvalidOutput("history item was not recorded".to_owned()))
    }

    async fn execute_agent_sequence(
        &mut self,
        actions: &[BrowserAction],
    ) -> Result<Vec<ActionResult>, AgentRunError> {
        let mut results = Vec::new();

        for (index, action) in actions.iter().enumerate() {
            self.check_stop_requested().await?;

            if index > 0 && matches!(action, BrowserAction::Done(_)) {
                break;
            }

            let needs_page_change_guard = !action.terminates_sequence();
            let before = if needs_page_change_guard {
                match self.executor.session().state(false).await {
                    Ok(state) => Some(state),
                    Err(error) => {
                        results.push(ActionResult::error(error.to_string()));
                        break;
                    }
                }
            } else {
                None
            };

            let timeout_seconds = self.settings.effective_action_timeout_seconds();
            let result = match timeout(action_timeout_duration(timeout_seconds), async {
                let raw_result = self.executor.execute_for_agent(action).await;
                self.resolve_agent_action_result(action, raw_result).await
            })
            .await
            {
                Ok(result) => result,
                Err(_) => timed_out_action_result(action, timeout_seconds),
            };
            let should_stop =
                result.is_done || result.error.is_some() || action.terminates_sequence();
            let page_changed = if should_stop {
                false
            } else if let Some(before) = before {
                match self.executor.session().state(false).await {
                    Ok(after) => after.url != before.url,
                    Err(error) => {
                        results.push(result);
                        results.push(ActionResult::error(error.to_string()));
                        break;
                    }
                }
            } else {
                false
            };

            results.push(result);

            if should_stop || page_changed {
                break;
            }
        }

        Ok(results)
    }

    async fn resolve_agent_action_result(
        &mut self,
        action: &BrowserAction,
        raw_result: ActionResult,
    ) -> ActionResult {
        let BrowserAction::Extract(params) = action else {
            return raw_result;
        };
        if raw_result.error.is_some() {
            return raw_result;
        }
        let Some(raw_content) = raw_result.extracted_content.as_deref() else {
            return raw_result;
        };

        let request = build_extract_llm_request(params, raw_content);
        let completion = match timeout(
            Duration::from_secs(self.settings.llm_timeout_seconds),
            self.llm.invoke_json(request),
        )
        .await
        {
            Ok(Ok(completion)) => completion,
            Ok(Err(error)) => {
                return ActionResult::error(format!("LLM-backed extract failed: {error}"));
            }
            Err(_) => {
                return ActionResult::error(format!(
                    "LLM-backed extract timed out after {} seconds",
                    self.settings.llm_timeout_seconds
                ));
            }
        };

        complete_llm_extract_result(
            params,
            raw_content,
            raw_result.metadata.as_ref(),
            completion.content,
            self.executor.file_system_mut(),
        )
    }

    async fn execute_initial_actions(&mut self) -> Result<(), AgentRunError> {
        if self.initial_actions_executed || self.settings.initial_actions.is_empty() {
            return Ok(());
        }
        self.initial_actions_executed = true;

        let step_start_time = now_seconds();
        let initial_actions = self.settings.initial_actions.clone();
        let state = initial_actions_state_history(&initial_actions);
        let current_url = state.url.clone();
        let execution_actions =
            actions_for_execution(&initial_actions, &self.settings, &current_url);
        let result = self.execute_agent_sequence(&execution_actions).await?;
        let step_end_time = now_seconds();

        self.history.items.push(AgentHistoryItem {
            model_output: Some(initial_actions_model_output(
                initial_actions,
                self.settings.flash_mode,
            )),
            result,
            state,
            metadata: Some(StepMetadata {
                step_start_time,
                step_end_time,
                step_number: 0,
                step_interval: None,
            }),
        });

        Ok(())
    }

    async fn record_final_response_after_failure(
        &mut self,
        failures: u32,
    ) -> Result<&AgentHistoryItem, AgentRunError> {
        self.check_stop_requested().await?;
        let step_start_time = now_seconds();
        let include_screenshot = self.should_include_screenshot();
        let state = self.executor.session().state(include_screenshot).await?;
        let request = build_final_response_after_failure_request(
            &self.task,
            &state,
            &self.history,
            &self.settings,
            Some(self.executor.file_system()),
            failures,
        )?;
        let request_for_transcript = request.clone();
        let completion = match timeout(
            Duration::from_secs(self.settings.llm_timeout_seconds),
            self.llm.invoke_json(request),
        )
        .await
        {
            Ok(Ok(completion)) => completion,
            Ok(Err(error)) => {
                return self.record_model_error(
                    state,
                    format!("LLM provider error during final response after failure: {error}"),
                    Some(step_start_time),
                );
            }
            Err(_) => {
                return self.record_model_error(
                    state,
                    format!(
                        "LLM call timed out after {} seconds during final response after failure",
                        self.settings.llm_timeout_seconds
                    ),
                    Some(step_start_time),
                );
            }
        };

        let model_output: AgentOutput = match serde_json::from_value(completion.content) {
            Ok(model_output) => model_output,
            Err(error) => {
                return self.record_model_error(
                    state,
                    format!("invalid final response after failure: {error}"),
                    Some(step_start_time),
                );
            }
        };
        self.save_conversation_snapshot(&request_for_transcript, &model_output)?;

        if !is_single_done_output(&model_output) {
            return self.record_model_error(
                state,
                "final response after failure must return exactly one done action".to_owned(),
                Some(step_start_time),
            );
        }

        self.record_model_output(state, model_output, Some(step_start_time))
            .await
    }

    fn record_model_error(
        &mut self,
        state: BrowserStateSummary,
        error: String,
        step_start_time: Option<f64>,
    ) -> Result<&AgentHistoryItem, AgentRunError> {
        let metadata = step_start_time.map(|start| self.step_metadata(start, now_seconds()));
        self.history.items.push(AgentHistoryItem {
            model_output: None,
            result: vec![ActionResult::error(error)],
            state,
            metadata,
        });

        self.history
            .items
            .last()
            .ok_or_else(|| AgentRunError::InvalidOutput("history item was not recorded".to_owned()))
    }

    fn save_conversation_snapshot(
        &self,
        request: &ChatRequest,
        model_output: &AgentOutput,
    ) -> Result<(), AgentRunError> {
        let Some(directory) = self.settings.save_conversation_path.as_deref() else {
            return Ok(());
        };
        let directory = expand_user_path(directory);
        std::fs::create_dir_all(&directory).map_err(|source| AgentRunError::ConversationSave {
            path: directory.display().to_string(),
            source,
        })?;
        let target = directory.join(format!(
            "conversation_{}_{}.txt",
            self.id,
            self.next_step_number()
        ));
        let snapshot = format_conversation_snapshot(request, model_output);
        let bytes = encode_conversation_snapshot(
            &snapshot,
            self.settings.save_conversation_path_encoding.as_deref(),
        )?;
        std::fs::write(&target, bytes).map_err(|source| AgentRunError::ConversationSave {
            path: target.display().to_string(),
            source,
        })
    }

    fn save_history_gif(&self) -> Result<(), AgentRunError> {
        let Some(path) = generate_gif_output_path(&self.settings.generate_gif) else {
            return Ok(());
        };
        write_history_gif(&self.history, &path).map_err(|message| AgentRunError::GifSave {
            path: path.display().to_string(),
            message,
        })
    }

    async fn maybe_judge_done_result(&mut self) {
        if !self.settings.use_judge || !self.history.is_done() {
            return;
        }

        let request = build_judge_request(&self.task, &self.history, &self.settings);
        let Ok(completion) = timeout(
            Duration::from_secs(self.settings.llm_timeout_seconds),
            self.llm.invoke_json(request),
        )
        .await
        else {
            return;
        };
        let Ok(completion) = completion else {
            return;
        };
        let Ok(judgement) = serde_json::from_value::<JudgementResult>(completion.content) else {
            return;
        };

        if let Some(result) = self
            .history
            .items
            .last_mut()
            .and_then(|item| item.result.last_mut())
            .filter(|result| result.is_done)
        {
            result.judgement = Some(judgement);
        }
    }

    async fn finish_successful_run(&mut self) -> Result<&AgentHistory, AgentRunError> {
        self.maybe_judge_done_result().await;
        self.save_history_gif()?;
        self.invoke_done_callbacks().await?;
        Ok(&self.history)
    }

    async fn maybe_compact_history(&mut self) -> bool {
        let Some(compaction_settings) = self.settings.message_compaction.resolved_settings() else {
            return false;
        };
        let Some(step_number) = latest_history_step_number(&self.history) else {
            return false;
        };
        let steps_since =
            step_number.saturating_sub(self.history.last_compaction_step.unwrap_or(0));
        if steps_since < compaction_settings.compact_every_n_steps {
            return false;
        }

        let full_history_text = render_history_items_for_compaction(&self.history);
        if full_history_text.chars().count() < compaction_settings.effective_trigger_char_count() {
            return false;
        }

        let request = build_message_compaction_request(
            &self.history,
            &compaction_settings,
            &self.settings.sensitive_data,
        );
        let Ok(completion) = timeout(
            Duration::from_secs(self.settings.llm_timeout_seconds),
            self.llm.invoke_json(request),
        )
        .await
        else {
            return false;
        };
        let Ok(completion) = completion else {
            return false;
        };
        let Ok(output) = serde_json::from_value::<MessageCompactionOutput>(completion.content)
        else {
            return false;
        };
        let mut summary = output.summary.trim().to_owned();
        if summary.is_empty() {
            return false;
        }
        if compaction_settings.summary_max_chars > 0
            && summary.chars().count() > compaction_settings.summary_max_chars
        {
            summary = truncate_chars(&summary, compaction_settings.summary_max_chars);
        }

        self.history.compacted_memory = Some(summary);
        self.history.compaction_count += 1;
        self.history.last_compaction_step = Some(step_number);
        retain_first_and_recent_history_items(
            &mut self.history,
            compaction_settings.keep_last_items,
        );
        true
    }

    fn step_metadata(&self, step_start_time: f64, step_end_time: f64) -> StepMetadata {
        let step_interval = self
            .history
            .items
            .last()
            .and_then(|item| item.metadata.as_ref())
            .map(|metadata| metadata.duration_seconds().max(0.0));

        StepMetadata {
            step_start_time,
            step_end_time,
            step_number: self.next_step_number(),
            step_interval,
        }
    }

    fn next_step_number(&self) -> usize {
        self.history
            .items
            .iter()
            .filter_map(|item| item.metadata.as_ref().map(|metadata| metadata.step_number))
            .max()
            .unwrap_or(0)
            + 1
    }

    fn should_include_screenshot(&self) -> bool {
        let action_requested_screenshot = self
            .history
            .items
            .last()
            .is_some_and(|item| item.result.iter().any(result_requests_screenshot));
        self.settings
            .use_vision
            .should_include_screenshot(action_requested_screenshot)
    }
}

fn encode_conversation_snapshot(
    snapshot: &str,
    encoding: Option<&str>,
) -> Result<Vec<u8>, AgentRunError> {
    let encoding = encoding.unwrap_or("utf-8");
    let Some(encoding_impl) = Encoding::for_label(encoding.as_bytes()) else {
        return Err(AgentRunError::ConversationEncoding {
            encoding: encoding.to_owned(),
        });
    };
    let (bytes, _, had_errors) = encoding_impl.encode(snapshot);
    if had_errors {
        return Err(AgentRunError::ConversationEncodingLossy {
            encoding: encoding.to_owned(),
        });
    }
    Ok(bytes.into_owned())
}

fn generate_gif_output_path(generate_gif: &GenerateGif) -> Option<std::path::PathBuf> {
    match generate_gif {
        GenerateGif::Disabled => None,
        GenerateGif::Enabled => Some(std::path::PathBuf::from("agent_history.gif")),
        GenerateGif::Path(path) => Some(expand_user_path(path)),
    }
}

fn write_history_gif(history: &AgentHistory, path: &std::path::Path) -> Result<(), String> {
    let mut frames = Vec::new();
    for screenshot in history.screenshots(None, false).into_iter().flatten() {
        if let Some(frame) = decode_gif_frame(screenshot)? {
            frames.push(frame);
        }
    }
    if frames.is_empty() {
        return Ok(());
    }

    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    let file = std::fs::File::create(path).map_err(|error| error.to_string())?;
    let mut encoder = image::codecs::gif::GifEncoder::new(file);
    encoder
        .set_repeat(image::codecs::gif::Repeat::Infinite)
        .map_err(|error| error.to_string())?;
    let (target_width, target_height) = frames[0].dimensions();
    for frame in frames {
        if frame.width() != target_width || frame.height() != target_height {
            continue;
        }
        let delay = image::Delay::from_numer_denom_ms(3000, 1);
        encoder
            .encode_frame(image::Frame::from_parts(frame, 0, 0, delay))
            .map_err(|error| error.to_string())?;
    }
    Ok(())
}

fn decode_gif_frame(screenshot: &str) -> Result<Option<image::RgbaImage>, String> {
    let screenshot = screenshot.trim();
    let screenshot = screenshot
        .strip_prefix("data:image/png;base64,")
        .unwrap_or(screenshot);
    let bytes = match base64::engine::general_purpose::STANDARD.decode(screenshot) {
        Ok(bytes) => bytes,
        Err(error) => return Err(error.to_string()),
    };
    let image = match image::load_from_memory(&bytes) {
        Ok(image) => image.to_rgba8(),
        Err(error) => return Err(error.to_string()),
    };
    if image.width() <= 4 && image.height() <= 4 {
        return Ok(None);
    }
    Ok(Some(image))
}

fn result_requests_screenshot(result: &ActionResult) -> bool {
    result
        .metadata
        .as_ref()
        .and_then(|metadata| metadata.get("include_screenshot"))
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

fn format_conversation_snapshot(request: &ChatRequest, model_output: &AgentOutput) -> String {
    let mut lines = Vec::new();
    for message in &request.messages {
        lines.push(format!(" {} ", message_role_name(&message.role)));
        lines.push(render_message_content(&message.content));
        lines.push(String::new());
    }
    lines.push(serde_json::to_string_pretty(model_output).unwrap_or_else(|_| "{}".to_owned()));
    lines.join("\n")
}

fn message_role_name(role: &MessageRole) -> &'static str {
    match role {
        MessageRole::System => "system",
        MessageRole::User => "user",
        MessageRole::Assistant => "assistant",
        MessageRole::Tool => "tool",
    }
}

fn render_message_content(content: &[ContentPart]) -> String {
    content
        .iter()
        .map(|part| match part {
            ContentPart::Text { text } => text.clone(),
            ContentPart::ImageUrl { image_url, detail } => {
                let detail = detail.map(ImageDetailLevel::as_str).unwrap_or("auto");
                format!("<image_url detail=\"{detail}\">\n{image_url}\n</image_url>")
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn expand_user_path(path: &str) -> std::path::PathBuf {
    if path == "~" {
        return std::env::var_os("HOME")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| std::path::PathBuf::from(path));
    }
    if let Some(rest) = path.strip_prefix("~/")
        && let Some(home) = std::env::var_os("HOME")
    {
        return std::path::PathBuf::from(home).join(rest);
    }
    std::path::PathBuf::from(path)
}

fn now_seconds() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs_f64())
        .unwrap_or(0.0)
}

fn is_single_done_output(output: &AgentOutput) -> bool {
    matches!(output.action.as_slice(), [BrowserAction::Done(_)])
}

const START_URL_EXCLUDED_EXTENSIONS: &[&str] = &[
    "pdf",
    "doc",
    "docx",
    "xls",
    "xlsx",
    "ppt",
    "pptx",
    "odt",
    "ods",
    "odp",
    "txt",
    "md",
    "csv",
    "json",
    "xml",
    "yaml",
    "yml",
    "zip",
    "rar",
    "7z",
    "tar",
    "gz",
    "bz2",
    "xz",
    "jpg",
    "jpeg",
    "png",
    "gif",
    "bmp",
    "svg",
    "webp",
    "ico",
    "mp3",
    "mp4",
    "avi",
    "mkv",
    "mov",
    "wav",
    "flac",
    "ogg",
    "py",
    "js",
    "css",
    "java",
    "cpp",
    "bib",
    "bibtex",
    "tex",
    "latex",
    "cls",
    "sty",
    "exe",
    "msi",
    "dmg",
    "pkg",
    "deb",
    "rpm",
    "iso",
    "polynomial",
];

const START_URL_EXCLUDED_WORDS: &[&str] = &["never", "dont", "not", "don't"];

fn settings_with_direct_start_url(task: &str, mut settings: AgentSettings) -> AgentSettings {
    if settings.directly_open_url
        && settings.initial_actions.is_empty()
        && let Some(url) = extract_start_url_from_task(task)
    {
        settings.initial_actions =
            vec![BrowserAction::Navigate(browser_use_tools::NavigateAction {
                url,
                new_tab: false,
            })];
    }
    settings
}

fn extract_start_url_from_task(task: &str) -> Option<String> {
    static EMAIL_RE: OnceLock<Regex> = OnceLock::new();
    static FULL_URL_RE: OnceLock<Regex> = OnceLock::new();
    static DOMAIN_URL_RE: OnceLock<Regex> = OnceLock::new();

    let email_re = EMAIL_RE.get_or_init(|| {
        Regex::new(r"\b[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Z|a-z]{2,}\b")
            .expect("valid email regex")
    });
    let full_url_re = FULL_URL_RE
        .get_or_init(|| Regex::new(r#"https?://[^\s<>"']+"#).expect("valid full URL regex"));
    let domain_url_re = DOMAIN_URL_RE.get_or_init(|| {
        Regex::new(r#"(?:www\.)?[a-zA-Z0-9-]+(?:\.[a-zA-Z0-9-]+)*\.[a-zA-Z]{2,}(?:/[^\s<>"']*)?"#)
            .expect("valid domain URL regex")
    });

    let task_without_emails = email_re.replace_all(task, "");
    let mut found_urls = BTreeSet::new();

    for pattern in [full_url_re, domain_url_re] {
        for matched in pattern.find_iter(&task_without_emails) {
            let original_position = matched.start();
            let url = trim_start_url_trailing_punctuation(matched.as_str());
            if url.is_empty() {
                continue;
            }

            let url_lower = url.to_ascii_lowercase();
            if contains_excluded_file_extension(&url_lower) {
                continue;
            }

            let context_start =
                char_boundary_n_chars_before(&task_without_emails, original_position, 20);
            let context = task_without_emails[context_start..original_position].to_lowercase();
            if START_URL_EXCLUDED_WORDS
                .iter()
                .any(|word| context.contains(word))
            {
                continue;
            }

            let url = if url.starts_with("http://") || url.starts_with("https://") {
                url.to_owned()
            } else {
                format!("https://{url}")
            };
            found_urls.insert(url);
        }
    }

    if found_urls.len() == 1 {
        found_urls.into_iter().next()
    } else {
        None
    }
}

fn trim_start_url_trailing_punctuation(url: &str) -> &str {
    url.trim_end_matches(['.', ',', ';', ':', '!', '?', '(', ')', '[', ']'])
}

fn contains_excluded_file_extension(url_lower: &str) -> bool {
    START_URL_EXCLUDED_EXTENSIONS
        .iter()
        .any(|ext| url_lower.contains(&format!(".{ext}")))
}

fn char_boundary_n_chars_before(text: &str, end: usize, chars: usize) -> usize {
    text[..end]
        .char_indices()
        .rev()
        .nth(chars.saturating_sub(1))
        .map(|(idx, _)| idx)
        .unwrap_or(0)
}

fn initial_actions_model_output(actions: Vec<BrowserAction>, flash_mode: bool) -> AgentOutput {
    if flash_mode {
        return AgentOutput {
            current_state: AgentCurrentState::default(),
            thinking: None,
            evaluation_previous_goal: None,
            memory: Some("Initial navigation".to_owned()),
            next_goal: None,
            current_plan_item: None,
            plan_update: None,
            action: actions,
        };
    }

    AgentOutput {
        current_state: AgentCurrentState::default(),
        thinking: None,
        evaluation_previous_goal: Some("Start".to_owned()),
        memory: None,
        next_goal: Some("Initial navigation".to_owned()),
        current_plan_item: None,
        plan_update: None,
        action: actions,
    }
}

fn initial_actions_state_history(actions: &[BrowserAction]) -> BrowserStateSummary {
    BrowserStateSummary {
        dom_state: Default::default(),
        url: initial_actions_url(actions).unwrap_or_default(),
        title: "Initial Actions".to_owned(),
        tabs: Vec::new(),
        screenshot: None,
        page_info: None,
        pixels_above: 0,
        pixels_below: 0,
        browser_errors: Vec::new(),
        is_pdf_viewer: false,
        recent_events: None,
        pending_network_requests: Vec::new(),
        pagination_buttons: Vec::new(),
        closed_popup_messages: Vec::new(),
    }
}

fn initial_actions_url(actions: &[BrowserAction]) -> Option<String> {
    actions.iter().find_map(|action| match action {
        BrowserAction::Navigate(params) => Some(params.url.clone()),
        BrowserAction::Search(params) => Some(search_url(&params.engine, &params.query)),
        _ => None,
    })
}

fn actions_for_execution(
    actions: &[BrowserAction],
    settings: &AgentSettings,
    current_url: &str,
) -> Vec<BrowserAction> {
    let sensitive_data = applicable_sensitive_data_values(&settings.sensitive_data, current_url);
    if sensitive_data.is_empty() {
        return actions.to_vec();
    }

    actions
        .iter()
        .map(|action| {
            let Ok(mut value) = serde_json::to_value(action) else {
                return action.clone();
            };
            replace_sensitive_placeholders_in_value(&mut value, &sensitive_data);
            serde_json::from_value(value).unwrap_or_else(|_| action.clone())
        })
        .collect()
}

fn replace_sensitive_placeholders_in_value(
    value: &mut Value,
    sensitive_data: &BTreeMap<String, String>,
) {
    match value {
        Value::String(text) => {
            *text = replace_sensitive_placeholders_in_string(text, sensitive_data);
        }
        Value::Array(items) => {
            for item in items {
                replace_sensitive_placeholders_in_value(item, sensitive_data);
            }
        }
        Value::Object(entries) => {
            for entry in entries.values_mut() {
                replace_sensitive_placeholders_in_value(entry, sensitive_data);
            }
        }
        Value::Null | Value::Bool(_) | Value::Number(_) => {}
    }
}

fn replace_sensitive_placeholders_in_string(
    text: &str,
    sensitive_data: &BTreeMap<String, String>,
) -> String {
    let secret_pattern =
        regex::Regex::new(r"<secret>(.*?)</secret>").expect("valid secret tag regex");
    let replaced = secret_pattern
        .replace_all(text, |captures: &regex::Captures<'_>| {
            let placeholder = captures.get(1).map(|match_| match_.as_str()).unwrap_or("");
            sensitive_replacement_value(placeholder, sensitive_data)
                .unwrap_or_else(|| captures[0].to_owned())
        })
        .into_owned();

    sensitive_replacement_value(&replaced, sensitive_data).unwrap_or(replaced)
}

fn sensitive_replacement_value(
    placeholder: &str,
    sensitive_data: &BTreeMap<String, String>,
) -> Option<String> {
    let secret = sensitive_data.get(placeholder)?;
    if placeholder.ends_with("bu_2fa_code") {
        return totp_code(secret, now_seconds() as u64);
    }

    Some(secret.clone())
}

fn totp_code(secret: &str, unix_seconds: u64) -> Option<String> {
    totp_code_at(secret, unix_seconds, 30, 6)
}

fn totp_code_at(
    secret: &str,
    unix_seconds: u64,
    period_seconds: u64,
    digits: u32,
) -> Option<String> {
    if period_seconds == 0 || digits == 0 || digits > 9 {
        return None;
    }

    let normalized_secret = secret
        .chars()
        .filter(|character| !character.is_whitespace())
        .collect::<String>()
        .to_ascii_uppercase();
    let unpadded_secret = normalized_secret.trim_end_matches('=');
    let key_bytes = data_encoding::BASE32_NOPAD
        .decode(unpadded_secret.as_bytes())
        .or_else(|_| data_encoding::BASE32.decode(normalized_secret.as_bytes()))
        .ok()?;
    let counter = unix_seconds / period_seconds;
    let message = counter.to_be_bytes();
    let key = ring::hmac::Key::new(ring::hmac::HMAC_SHA1_FOR_LEGACY_USE_ONLY, &key_bytes);
    let tag = ring::hmac::sign(&key, &message);
    let digest = tag.as_ref();
    let offset = usize::from(digest.last()? & 0x0f);
    let binary = (u32::from(digest.get(offset)? & 0x7f) << 24)
        | (u32::from(*digest.get(offset + 1)?) << 16)
        | (u32::from(*digest.get(offset + 2)?) << 8)
        | u32::from(*digest.get(offset + 3)?);
    let code = binary % 10_u32.pow(digits);

    Some(format!("{code:0width$}", width = digits as usize))
}

pub fn build_step_request(
    task: &str,
    state: &BrowserStateSummary,
    history: &AgentHistory,
    settings: &AgentSettings,
) -> Result<ChatRequest, AgentRunError> {
    build_step_request_with_file_system(task, state, history, settings, None)
}

pub fn build_step_request_with_file_system(
    task: &str,
    state: &BrowserStateSummary,
    history: &AgentHistory,
    settings: &AgentSettings,
    file_system: Option<&ManagedFileSystem>,
) -> Result<ChatRequest, AgentRunError> {
    let mut state_for_text = state.clone();
    state_for_text.screenshot = None;
    if !settings.include_recent_events {
        state_for_text.recent_events = None;
    }
    if !settings.include_attributes.is_empty() {
        state_for_text.dom_state.text = state
            .dom_state
            .llm_representation_with_attributes(&settings.include_attributes);
    }
    state_for_text.dom_state.text = truncate_clickable_elements_text(
        &state_for_text.dom_state.text,
        settings.max_clickable_elements_length,
    );
    let state_json = serde_json::to_string_pretty(&state_for_text)
        .map_err(|error| AgentRunError::InvalidOutput(error.to_string()))?;
    let agent_history = render_previous_results(history, settings.max_history_items);
    let page_stats = render_page_stats(state);
    let agent_state =
        render_agent_state_description(task, &page_stats, history, state, settings, file_system);
    let read_state = render_read_state_description(history)
        .map(|description| format!("\n<read_state>\n{description}\n</read_state>\n"))
        .unwrap_or_default();
    let sensitive_values = collect_sensitive_data_values(&settings.sensitive_data);
    let user_text = redact_sensitive_string(
        &format!(
            "<agent_history>\n{agent_history}\n</agent_history>\n\n<agent_state>\n{agent_state}\n</agent_state>\n<browser_state>\n{state_json}\n</browser_state>{read_state}"
        ),
        &sensitive_values,
    );
    let mut user_content = vec![ContentPart::Text { text: user_text }];
    user_content.extend(settings.sample_images.iter().cloned());
    if settings.use_vision.accepts_prompt_image()
        && let Some(screenshot) = state.screenshot.as_deref()
    {
        user_content.push(ContentPart::ImageUrl {
            image_url: screenshot_data_url(screenshot),
            detail: Some(settings.vision_detail_level),
        });
    }
    append_latest_action_result_images(&mut user_content, history, settings.vision_detail_level);
    Ok(ChatRequest {
        messages: vec![
            ChatMessage::text(MessageRole::System, render_system_message(settings)),
            ChatMessage {
                role: MessageRole::User,
                content: user_content,
            },
        ],
        output_schema: Some(schema_for_agent_output_with_settings(settings)),
    })
}

fn render_system_message(settings: &AgentSettings) -> String {
    let mut message = settings.override_system_message.clone().unwrap_or_else(|| {
        format!(
            "You are controlling a browser. Return a JSON object matching AgentOutput. \
	         Use at most {} actions in this step. Avoid repeating the same action \
	         sequence; if the browser is not changing, choose a different strategy \
	         or finish with done.",
            settings.max_actions_per_step
        )
    });
    if let Some(extension) = settings
        .extend_system_message
        .as_deref()
        .filter(|extension| !extension.is_empty())
    {
        message.push('\n');
        message.push_str(extension);
    }

    message
}

fn build_final_response_after_failure_request(
    task: &str,
    state: &BrowserStateSummary,
    history: &AgentHistory,
    settings: &AgentSettings,
    file_system: Option<&ManagedFileSystem>,
    failures: u32,
) -> Result<ChatRequest, AgentRunError> {
    let mut request =
        build_step_request_with_file_system(task, state, history, settings, file_system)?;
    request.output_schema = Some(schema_for_final_response_after_failure(settings));
    let instruction = format!(
        "You failed {failures} times. We are terminating the agent. Your only available action is done. Return exactly one done action. \
         If the task is not fully finished, set success to false. Include everything useful you found for the original task in done.text."
    );
    if let Some(message) = request
        .messages
        .iter_mut()
        .find(|message| message.role == MessageRole::User)
    {
        message
            .content
            .push(ContentPart::Text { text: instruction });
    }
    Ok(request)
}

fn should_inject_step_budget_warning(steps_used: usize, max_steps: usize) -> bool {
    max_steps > 0
        && steps_used < max_steps
        && steps_used.saturating_mul(4) >= max_steps.saturating_mul(3)
}

fn build_step_request_with_budget_warning(
    task: &str,
    state: &BrowserStateSummary,
    history: &AgentHistory,
    settings: &AgentSettings,
    file_system: Option<&ManagedFileSystem>,
    steps_used: usize,
    max_steps: usize,
) -> Result<ChatRequest, AgentRunError> {
    let mut request =
        build_step_request_with_file_system(task, state, history, settings, file_system)?;
    let steps_remaining = max_steps.saturating_sub(steps_used);
    let pct = steps_used
        .saturating_mul(100)
        .checked_div(max_steps)
        .unwrap_or(0);
    let instruction = format!(
        "BUDGET WARNING: You have used {steps_used}/{max_steps} steps ({pct}%). {steps_remaining} steps remaining. \
         If the task cannot be completed in the remaining steps, prioritize: \
         (1) consolidate your results (save to files if the file system is in use), \
         (2) call done with what you have. \
         Partial results are far more valuable than exhausting all steps with nothing saved."
    );
    if let Some(message) = request
        .messages
        .iter_mut()
        .find(|message| message.role == MessageRole::User)
    {
        message
            .content
            .push(ContentPart::Text { text: instruction });
    }
    Ok(request)
}

fn build_final_response_after_step_limit_request(
    task: &str,
    state: &BrowserStateSummary,
    history: &AgentHistory,
    settings: &AgentSettings,
    file_system: Option<&ManagedFileSystem>,
    max_steps: usize,
) -> Result<ChatRequest, AgentRunError> {
    let mut request =
        build_step_request_with_file_system(task, state, history, settings, file_system)?;
    request.output_schema = Some(schema_for_final_response_after_failure(settings));
    let instruction = format!(
        "You reached max_steps ({max_steps}) - this is your last step. Your only available action is done. \
         Return exactly one done action. If the task is not fully finished, set success to false. \
         Include everything useful you found for the original task in done.text."
    );
    if let Some(message) = request
        .messages
        .iter_mut()
        .find(|message| message.role == MessageRole::User)
    {
        message
            .content
            .push(ContentPart::Text { text: instruction });
    }
    Ok(request)
}

fn build_judge_request(
    task: &str,
    history: &AgentHistory,
    settings: &AgentSettings,
) -> ChatRequest {
    let final_result = history.final_result().unwrap_or_default();
    let trajectory = render_judge_trajectory(history);
    let ground_truth = settings
        .ground_truth
        .as_deref()
        .map(|ground_truth| {
            format!(
                "\n<ground_truth>\n{}\n</ground_truth>\n",
                truncate_judge_text(ground_truth)
            )
        })
        .unwrap_or_default();
    let user_prompt = format!(
        "<task>\n{}\n</task>\n{ground_truth}<agent_trajectory>\n{}\n</agent_trajectory>\n\n<final_result>\n{}\n</final_result>\n\nEvaluate this agent execution and respond with the exact JSON object requested.",
        truncate_judge_text(task),
        truncate_judge_text(&trajectory),
        truncate_judge_text(final_result)
    );
    let mut user_content = vec![ContentPart::Text { text: user_prompt }];
    if settings.use_vision.accepts_prompt_image() {
        for screenshot in history.screenshots(Some(10), false).into_iter().flatten() {
            user_content.push(ContentPart::ImageUrl {
                image_url: screenshot_data_url(screenshot),
                detail: Some(settings.vision_detail_level),
            });
        }
    }

    ChatRequest {
        messages: vec![
            ChatMessage::text(MessageRole::System, render_judge_system_message()),
            ChatMessage {
                role: MessageRole::User,
                content: user_content,
            },
        ],
        output_schema: Some(schema_for_judgement_result()),
    }
}

fn build_message_compaction_request(
    history: &AgentHistory,
    settings: &MessageCompactionSettings,
    sensitive_data: &BTreeMap<String, SensitiveDataValue>,
) -> ChatRequest {
    let full_history_text = render_history_items_for_compaction(history);
    let mut sections = Vec::new();
    if let Some(memory) = non_empty_prompt_text(history.compacted_memory.as_deref()) {
        sections.push(format!(
            "<previous_compacted_memory>\n{memory}\n</previous_compacted_memory>"
        ));
    }
    sections.push(format!(
        "<agent_history>\n{full_history_text}\n</agent_history>"
    ));
    if settings.include_read_state
        && let Some(read_state) = render_read_state_description(history)
    {
        sections.push(format!("<read_state>\n{read_state}\n</read_state>"));
    }
    let sensitive_values = collect_sensitive_data_values(sensitive_data);
    let user_prompt = redact_sensitive_string(&sections.join("\n\n"), &sensitive_values);

    let mut system_prompt = "You are summarizing an agent run for prompt compaction.\n\
         Capture task requirements, key facts, decisions, partial progress, errors, and next steps.\n\
         Preserve important entities, values, URLs, and file paths.\n\
         CRITICAL: Only mark a step as completed if you see explicit success confirmation in the history. \
         If a step was started but not explicitly confirmed complete, mark it as \"IN-PROGRESS\". \
         Never infer completion from context - only report what was confirmed.\n\
         Respond with exactly a JSON object matching MessageCompactionOutput: summary. Do not add prose outside JSON."
        .to_owned();
    if settings.summary_max_chars > 0 {
        system_prompt.push_str(&format!(
            " Keep summary under {} characters if possible.",
            settings.summary_max_chars
        ));
    }

    ChatRequest {
        messages: vec![
            ChatMessage::text(MessageRole::System, system_prompt),
            ChatMessage::text(MessageRole::User, user_prompt),
        ],
        output_schema: Some(schema_for_message_compaction_output()),
    }
}

fn render_judge_system_message() -> String {
    "You are an expert judge evaluating browser automation agent performance.\n\
     Decide whether the agent satisfied the user task, whether the final output is complete, \
     whether browser/tool actions appear effective, and whether any captcha or impossible-task \
     condition blocked success. Ground truth, when provided, has highest priority.\n\
     Respond with exactly a JSON object matching JudgementResult: reasoning, verdict, \
     failure_reason, impossible_task, reached_captcha. Do not add prose outside JSON."
        .to_owned()
}

fn render_judge_trajectory(history: &AgentHistory) -> String {
    history
        .items
        .iter()
        .enumerate()
        .map(|(index, item)| {
            let model_output = item
                .model_output
                .as_ref()
                .and_then(|output| serde_json::to_string_pretty(output).ok())
                .unwrap_or_else(|| "null".to_owned());
            let result =
                serde_json::to_string_pretty(&item.result).unwrap_or_else(|_| "[]".to_owned());
            format!(
                "Step {}\nURL: {}\nTitle: {}\nModel output:\n{}\nAction result:\n{}",
                index + 1,
                item.state.url,
                item.state.title,
                model_output,
                result
            )
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

fn truncate_judge_text(text: &str) -> String {
    const MAX_CHARS: usize = 40_000;
    if text.chars().count() <= MAX_CHARS {
        return text.to_owned();
    }
    let truncated = text
        .chars()
        .take(MAX_CHARS.saturating_sub(23))
        .collect::<String>();
    format!("{truncated}...[text truncated]...")
}

fn latest_history_step_number(history: &AgentHistory) -> Option<usize> {
    history.items.last()?;
    Some(
        history
            .items
            .iter()
            .filter_map(|item| item.metadata.as_ref().map(|metadata| metadata.step_number))
            .max()
            .unwrap_or(history.items.len()),
    )
}

fn retain_first_and_recent_history_items(history: &mut AgentHistory, keep_last_items: usize) {
    let keep_with_first = keep_last_items.saturating_add(1);
    if history.items.len() <= keep_with_first {
        return;
    }

    let first = history.items[0].clone();
    let mut retained = vec![first];
    if keep_last_items > 0 {
        let recent_start = history.items.len().saturating_sub(keep_last_items);
        retained.extend(history.items[recent_start..].iter().cloned());
    }
    history.items = retained;
}

fn screenshot_data_url(screenshot: &str) -> String {
    if screenshot.starts_with("data:image/") {
        screenshot.to_owned()
    } else {
        format!("data:image/png;base64,{screenshot}")
    }
}

fn append_latest_action_result_images(
    content: &mut Vec<ContentPart>,
    history: &AgentHistory,
    vision_detail_level: ImageDetailLevel,
) {
    let Some(latest) = history.items.last() else {
        return;
    };

    for image in latest.result.iter().flat_map(|result| result.images.iter()) {
        let Some(data) = image.get("data").and_then(Value::as_str) else {
            continue;
        };
        if data.is_empty() {
            continue;
        }
        let name = image
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or("unknown");

        content.push(ContentPart::Text {
            text: format!("Image from file: {name}"),
        });
        content.push(ContentPart::ImageUrl {
            image_url: action_result_image_data_url(name, data),
            detail: Some(vision_detail_level),
        });
    }
}

fn action_result_image_data_url(name: &str, data: &str) -> String {
    if data.starts_with("data:image/") {
        return data.to_owned();
    }

    let media_type = if name.to_ascii_lowercase().ends_with(".png") {
        "image/png"
    } else {
        "image/jpeg"
    };
    format!("data:{media_type};base64,{data}")
}

fn truncate_clickable_elements_text(text: &str, max_chars: usize) -> String {
    if max_chars == 0 {
        return "[clickable elements omitted by max_clickable_elements_length]".to_owned();
    }
    if text.chars().count() <= max_chars {
        return text.to_owned();
    }

    let truncated = text.chars().take(max_chars).collect::<String>();
    format!("{truncated}\n...[clickable elements truncated to {max_chars} chars]")
}

fn render_page_stats(state: &BrowserStateSummary) -> String {
    let stats = if state.dom_state.page_stats.is_empty() {
        fallback_page_stats(state)
    } else {
        state.dom_state.page_stats
    };
    let mut stats_text = "<page_stats>".to_owned();
    if stats.total_elements < 10 {
        stats_text.push_str("Page appears empty (SPA not loaded?) - ");
    } else if stats.total_elements > 20 && stats.text_chars < stats.total_elements.saturating_mul(5)
    {
        stats_text
            .push_str("Page appears to show skeleton/placeholder content (still loading?) - ");
    }
    stats_text.push_str(&format!(
        "{} links, {} interactive, {} iframes",
        stats.links, stats.interactive_elements, stats.iframes
    ));
    if stats.shadow_open > 0 || stats.shadow_closed > 0 {
        stats_text.push_str(&format!(
            ", {} shadow(open), {} shadow(closed)",
            stats.shadow_open, stats.shadow_closed
        ));
    }
    if stats.images > 0 {
        stats_text.push_str(&format!(", {} images", stats.images));
    }
    stats_text.push_str(&format!(
        ", {} scroll containers, {} total elements, {} text chars",
        stats.scroll_containers, stats.total_elements, stats.text_chars
    ));

    if let Some(page_info) = state.page_info {
        stats_text.push_str(&format!(
            ", {}px above, {}px below",
            page_info.pixels_above, page_info.pixels_below
        ));
    }

    stats_text.push_str("</page_stats>");
    stats_text
}

fn fallback_page_stats(state: &BrowserStateSummary) -> browser_use_dom::DomPageStats {
    let indexed_elements = state.dom_state.selector_map.values();
    browser_use_dom::DomPageStats {
        links: indexed_elements
            .clone()
            .filter(|element| element.tag_name == "a")
            .count() as u32,
        iframes: indexed_elements
            .clone()
            .filter(|element| matches!(element.tag_name.as_str(), "iframe" | "frame"))
            .count() as u32,
        scroll_containers: indexed_elements
            .clone()
            .filter(|element| element.is_scrollable)
            .count() as u32,
        interactive_elements: indexed_elements
            .clone()
            .filter(|element| element.is_interactive)
            .count() as u32,
        total_elements: state.dom_state.selector_map.len() as u32,
        text_chars: state.dom_state.text.chars().count() as u32,
        ..browser_use_dom::DomPageStats::default()
    }
}

fn render_agent_state_description(
    task: &str,
    page_stats: &str,
    history: &AgentHistory,
    state: &BrowserStateSummary,
    settings: &AgentSettings,
    file_system: Option<&ManagedFileSystem>,
) -> String {
    let mut description = format!("Task:\n{task}\n\nPage stats:\n{page_stats}");
    if let Some(file_system) = file_system {
        let todo_contents = file_system.get_todo_contents();
        let todo_contents = if todo_contents.is_empty() {
            "[empty todo.md, fill it when applicable]".to_owned()
        } else {
            todo_contents
        };
        description.push_str(&format!(
            "\n\n<file_system>\n{}\n</file_system>\n<todo_contents>\n{todo_contents}\n</todo_contents>",
            file_system.describe()
        ));
    }
    if let Some(message) = render_planning_context(history, settings) {
        description.push_str(&format!("\n\nPlanning:\n{message}"));
    }
    if let Some(message) = render_loop_awareness(history, state, settings) {
        description.push_str(&format!("\n\nLoop awareness:\n{message}"));
    }
    if let Some(message) = render_sensitive_data_description(&state.url, settings) {
        description.push_str(&format!("\n\n<sensitive_data>{message}</sensitive_data>"));
    }
    if !settings.available_file_paths.is_empty() {
        description.push_str(&format!(
            "\n\n<available_file_paths>{}\nUse with absolute paths</available_file_paths>",
            settings.available_file_paths.join("\n")
        ));
    }
    description
}

fn render_sensitive_data_description(
    current_url: &str,
    settings: &AgentSettings,
) -> Option<String> {
    let placeholders = sensitive_data_placeholders_for_url(&settings.sensitive_data, current_url);
    if placeholders.is_empty() {
        return None;
    }

    let first = placeholders.first().expect("placeholder exists");
    let formatted_placeholders = placeholders
        .iter()
        .map(|placeholder| format!("  - {placeholder}"))
        .collect::<Vec<_>>()
        .join("\n");

    Some(format!(
        "SENSITIVE DATA - Use these placeholders for secure input:\n{formatted_placeholders}\n\nIMPORTANT: When entering sensitive values, you MUST wrap the placeholder name in <secret> tags.\nExample: To enter the value for \"{first}\", use: <secret>{first}</secret>\nThe system will automatically replace these tags with the actual secret values."
    ))
}

fn sensitive_data_placeholders_for_url(
    sensitive_data: &BTreeMap<String, SensitiveDataValue>,
    current_url: &str,
) -> Vec<String> {
    let mut placeholders = BTreeSet::new();
    for (key_or_domain, value) in sensitive_data {
        match value {
            SensitiveDataValue::Value(_) => {
                placeholders.insert(key_or_domain.clone());
            }
            SensitiveDataValue::Domain(domain_values)
                if match_url_with_domain_pattern(current_url, key_or_domain) =>
            {
                placeholders.extend(domain_values.keys().cloned());
            }
            SensitiveDataValue::Domain(_) => {}
        }
    }

    placeholders.into_iter().collect()
}

fn collect_sensitive_data_values(
    sensitive_data: &BTreeMap<String, SensitiveDataValue>,
) -> BTreeMap<String, String> {
    let mut values = BTreeMap::new();
    for (key_or_domain, value) in sensitive_data {
        match value {
            SensitiveDataValue::Value(secret) if !secret.is_empty() => {
                values.insert(key_or_domain.clone(), secret.clone());
            }
            SensitiveDataValue::Value(_) => {}
            SensitiveDataValue::Domain(domain_values) => {
                for (placeholder, secret) in domain_values {
                    if !secret.is_empty() {
                        values.insert(placeholder.clone(), secret.clone());
                    }
                }
            }
        }
    }

    values
}

fn applicable_sensitive_data_values(
    sensitive_data: &BTreeMap<String, SensitiveDataValue>,
    current_url: &str,
) -> BTreeMap<String, String> {
    let mut values = BTreeMap::new();
    for (key_or_domain, value) in sensitive_data {
        match value {
            SensitiveDataValue::Value(secret) if !secret.is_empty() => {
                values.insert(key_or_domain.clone(), secret.clone());
            }
            SensitiveDataValue::Value(_) => {}
            SensitiveDataValue::Domain(secrets)
                if match_url_with_domain_pattern(current_url, key_or_domain) =>
            {
                for (placeholder, secret) in secrets {
                    if !secret.is_empty() {
                        values.insert(placeholder.clone(), secret.clone());
                    }
                }
            }
            SensitiveDataValue::Domain(_) => {}
        }
    }

    values
}

fn redact_sensitive_string(value: &str, sensitive_values: &BTreeMap<String, String>) -> String {
    let mut redacted = value.to_owned();
    let mut entries = sensitive_values.iter().collect::<Vec<_>>();
    entries.sort_by_key(|entry| std::cmp::Reverse(entry.1.len()));
    for (placeholder, secret) in entries {
        redacted = redacted.replace(secret, &format!("<secret>{placeholder}</secret>"));
    }

    redacted
}

fn match_url_with_domain_pattern(url: &str, domain_pattern: &str) -> bool {
    if is_new_tab_page(url) {
        return false;
    }

    let Ok(parsed_url) = url::Url::parse(url) else {
        return false;
    };
    let scheme = parsed_url.scheme().to_ascii_lowercase();
    let Some(domain) = parsed_url.host_str().map(str::to_ascii_lowercase) else {
        return false;
    };
    if scheme.is_empty() || domain.is_empty() {
        return false;
    }

    let domain_pattern = domain_pattern.to_ascii_lowercase();
    let (pattern_scheme, pattern_domain) = domain_pattern
        .split_once("://")
        .map_or(("https", domain_pattern.as_str()), |(scheme, domain)| {
            (scheme, domain)
        });
    let pattern_domain = pattern_domain
        .split_once(':')
        .map_or(pattern_domain, |(domain, _)| domain);

    if !glob_match(&scheme, pattern_scheme) {
        return false;
    }
    if pattern_domain == "*" || domain == pattern_domain {
        return true;
    }

    if !pattern_domain.contains('*') {
        return false;
    }
    if pattern_domain.matches("*.").count() > 1 || pattern_domain.matches(".*").count() > 1 {
        return false;
    }
    if pattern_domain.ends_with(".*") {
        return false;
    }
    let bare_domain = pattern_domain.replace("*.", "");
    if bare_domain.contains('*') {
        return false;
    }

    if let Some(parent_domain) = pattern_domain.strip_prefix("*.")
        && domain == parent_domain
    {
        return true;
    }

    glob_match(&domain, pattern_domain)
}

fn is_new_tab_page(url: &str) -> bool {
    matches!(
        url,
        "about:blank"
            | "chrome://new-tab-page/"
            | "chrome://new-tab-page"
            | "chrome://newtab/"
            | "chrome://newtab"
    )
}

fn glob_match(value: &str, pattern: &str) -> bool {
    let pattern = format!("^{}$", regex::escape(pattern).replace("\\*", ".*"));
    regex::Regex::new(&pattern)
        .map(|regex| regex.is_match(value))
        .unwrap_or(false)
}

fn schema_for_agent_output() -> Value {
    serde_json::to_value(schemars::schema_for!(AgentOutput)).unwrap_or(Value::Null)
}

fn schema_for_judgement_result() -> Value {
    serde_json::to_value(schemars::schema_for!(JudgementResult)).unwrap_or(Value::Null)
}

fn schema_for_message_compaction_output() -> Value {
    serde_json::to_value(schemars::schema_for!(MessageCompactionOutput)).unwrap_or(Value::Null)
}

fn schema_for_agent_output_with_settings(settings: &AgentSettings) -> Value {
    let mut schema = schema_for_agent_output();
    let mut remove_fields = vec!["current_state"];

    if !settings.use_thinking || settings.flash_mode {
        remove_fields.push("thinking");
    }
    if settings.flash_mode {
        remove_fields.extend([
            "current_state",
            "evaluation_previous_goal",
            "next_goal",
            "current_plan_item",
            "plan_update",
        ]);
    }

    if !remove_fields.is_empty() {
        prune_schema_properties(&mut schema, &remove_fields);
    }

    let excluded_actions = normalized_schema_excluded_actions(settings);
    if !excluded_actions.is_empty() {
        exclude_schema_actions(&mut schema, &excluded_actions);
    }

    if settings.flash_mode {
        require_schema_properties(&mut schema, &["memory", "action"]);
    } else {
        require_schema_properties(
            &mut schema,
            &["evaluation_previous_goal", "memory", "next_goal", "action"],
        );
    }
    require_non_empty_actions(&mut schema);

    schema
}

fn schema_for_final_response_after_failure(settings: &AgentSettings) -> Value {
    let mut schema = schema_for_agent_output_with_settings(settings);
    restrict_schema_actions_to_done(&mut schema);
    schema
}

fn normalized_excluded_actions(actions: &[String]) -> BTreeSet<String> {
    actions
        .iter()
        .map(|action| action.trim().replace('-', "_").to_ascii_lowercase())
        .filter(|action| !action.is_empty() && action != "done")
        .collect()
}

fn normalized_schema_excluded_actions(settings: &AgentSettings) -> BTreeSet<String> {
    let mut excluded_actions = normalized_excluded_actions(&settings.excluded_actions);
    if !settings.use_vision.allows_screenshot_action() {
        excluded_actions.insert("screenshot".to_owned());
    }
    excluded_actions
}

fn excluded_action_error(actions: &[BrowserAction], settings: &AgentSettings) -> Option<String> {
    if !settings.use_vision.allows_screenshot_action()
        && actions
            .iter()
            .any(|action| matches!(action, BrowserAction::Screenshot(_)))
    {
        return Some(
            "model output requested screenshot action, but AgentSettings.use_vision must be \"auto\""
                .to_owned(),
        );
    }

    let excluded_actions = normalized_excluded_actions(&settings.excluded_actions);
    if excluded_actions.is_empty() {
        return None;
    }

    actions
        .iter()
        .map(BrowserAction::name)
        .find(|name| excluded_actions.contains(*name))
        .map(|name| {
            format!(
                "model output requested excluded action `{name}`; remove it from the action list or update AgentSettings.excluded_actions"
            )
        })
}

fn exclude_schema_actions(schema: &mut Value, excluded_actions: &BTreeSet<String>) {
    for pointer in [
        "/$defs/BrowserAction/oneOf",
        "/$defs/BrowserAction/anyOf",
        "/definitions/BrowserAction/oneOf",
        "/definitions/BrowserAction/anyOf",
    ] {
        if let Some(actions) = schema.pointer_mut(pointer).and_then(Value::as_array_mut) {
            actions.retain(|action| {
                schema_variant_action_name(action)
                    .is_none_or(|name| !excluded_actions.contains(name))
            });
        }
    }
}

fn restrict_schema_actions_to_done(schema: &mut Value) {
    for pointer in [
        "/$defs/BrowserAction/oneOf",
        "/$defs/BrowserAction/anyOf",
        "/definitions/BrowserAction/oneOf",
        "/definitions/BrowserAction/anyOf",
    ] {
        if let Some(actions) = schema.pointer_mut(pointer).and_then(Value::as_array_mut) {
            actions.retain(schema_variant_is_done_action);
        }
    }
}

fn schema_variant_is_done_action(value: &Value) -> bool {
    schema_variant_action_name(value) == Some("done")
}

fn schema_variant_action_name(value: &Value) -> Option<&str> {
    let required_action_name = value
        .get("required")
        .and_then(Value::as_array)
        .and_then(|fields| fields.iter().find_map(Value::as_str));
    required_action_name.or_else(|| {
        value
            .get("properties")
            .and_then(Value::as_object)
            .and_then(|properties| properties.keys().next().map(String::as_str))
    })
}

fn prune_schema_properties(schema: &mut Value, remove_fields: &[&str]) {
    if let Some(properties) = schema.get_mut("properties").and_then(Value::as_object_mut) {
        for field in remove_fields {
            properties.remove(*field);
        }
    }

    if let Some(required) = schema.get_mut("required").and_then(Value::as_array_mut) {
        required.retain(|value| {
            value
                .as_str()
                .is_none_or(|field| !remove_fields.contains(&field))
        });
    }
}

fn require_schema_properties(schema: &mut Value, fields: &[&str]) {
    schema["required"] = Value::Array(
        fields
            .iter()
            .map(|field| Value::String((*field).to_owned()))
            .collect(),
    );
}

fn require_non_empty_actions(schema: &mut Value) {
    if let Some(action) = schema
        .get_mut("properties")
        .and_then(Value::as_object_mut)
        .and_then(|properties| properties.get_mut("action"))
        .and_then(Value::as_object_mut)
    {
        action.insert("minItems".to_owned(), Value::from(1));
    }
}

fn render_previous_results(history: &AgentHistory, max_history_items: Option<usize>) -> String {
    enum HistoryPromptEntry<'a> {
        Item(&'a AgentHistoryItem),
        Omitted(usize),
    }

    let total_items = history.items.len();
    let entries: Vec<HistoryPromptEntry<'_>> = match max_history_items {
        None => history.items.iter().map(HistoryPromptEntry::Item).collect(),
        Some(max_history_items) if total_items <= max_history_items => {
            history.items.iter().map(HistoryPromptEntry::Item).collect()
        }
        Some(0) => vec![HistoryPromptEntry::Omitted(total_items)],
        Some(max_history_items) => {
            let omitted_count = total_items - max_history_items;
            let recent_items_count = max_history_items - 1;
            let recent_start = total_items.saturating_sub(recent_items_count);
            let mut entries = vec![
                HistoryPromptEntry::Item(&history.items[0]),
                HistoryPromptEntry::Omitted(omitted_count),
            ];
            entries.extend(
                history
                    .items
                    .iter()
                    .skip(recent_start)
                    .map(HistoryPromptEntry::Item),
            );
            entries
        }
    };

    let mut rendered = Vec::new();
    if let Some(memory) = non_empty_prompt_text(history.compacted_memory.as_deref()) {
        rendered.push(render_compacted_memory(memory));
    }
    if history.items.is_empty() {
        rendered.push("Agent initialized".to_owned());
    }
    for entry in entries {
        match entry {
            HistoryPromptEntry::Item(item) => {
                if let Some(item_text) = render_history_item_for_prompt(item) {
                    rendered.push(item_text);
                }
            }
            HistoryPromptEntry::Omitted(omitted_count) if omitted_count > 0 => {
                rendered.push(format!(
                    "<sys>[... {omitted_count} previous steps omitted...]</sys>"
                ));
            }
            HistoryPromptEntry::Omitted(_) => {}
        }
    }

    truncate_prompt_content(rendered.join("\n"))
}

fn render_compacted_memory(memory: &str) -> String {
    format!(
        "<compacted_memory>\n\
         <!-- Summary of prior steps. Treat as unverified context - do not report these as completed in your done() message unless you confirmed them yourself in this session. -->\n\
         {memory}\n\
         </compacted_memory>"
    )
}

fn render_history_items_for_compaction(history: &AgentHistory) -> String {
    let mut rendered = if history.items.is_empty() {
        vec!["Agent initialized".to_owned()]
    } else {
        Vec::new()
    };
    for item in &history.items {
        if let Some(item_text) = render_history_item_for_prompt(item) {
            rendered.push(item_text);
        }
    }
    truncate_prompt_content(rendered.join("\n"))
}

fn render_history_item_for_prompt(item: &AgentHistoryItem) -> Option<String> {
    let mut content_parts = Vec::new();
    if let Some(output) = item.model_output.as_ref() {
        let brain = output.current_brain();
        if let Some(evaluation) = non_empty_prompt_text(brain.evaluation_previous_goal.as_deref()) {
            content_parts.push(evaluation.to_owned());
        }
        if let Some(memory) = non_empty_prompt_text(brain.memory.as_deref()) {
            content_parts.push(memory.to_owned());
        }
        if let Some(next_goal) = non_empty_prompt_text(brain.next_goal.as_deref()) {
            content_parts.push(next_goal.to_owned());
        }
    }
    if let Some(action_results) = render_action_results_for_prompt(&item.result) {
        content_parts.push(action_results);
    }

    (!content_parts.is_empty()).then(|| format!("<step>\n{}", content_parts.join("\n")))
}

fn render_action_results_for_prompt(results: &[ActionResult]) -> Option<String> {
    let mut lines = Vec::new();
    for result in results {
        if let Some(memory) = non_empty_prompt_text(result.long_term_memory.as_deref()) {
            lines.push(memory.to_owned());
        } else if !result.include_extracted_content_only_once
            && let Some(content) = non_empty_prompt_text(result.extracted_content.as_deref())
        {
            lines.push(content.to_owned());
        }

        if let Some(error) = non_empty_prompt_text(result.error.as_deref()) {
            lines.push(truncate_error_for_prompt(error));
        }
    }

    if lines.is_empty() {
        None
    } else {
        Some(truncate_prompt_content(format!(
            "Result\n{}",
            lines.join("\n")
        )))
    }
}

fn render_read_state_description(history: &AgentHistory) -> Option<String> {
    let latest = history.items.last()?;
    let mut blocks = Vec::new();
    for result in &latest.result {
        if result.include_extracted_content_only_once
            && let Some(extracted_content) =
                non_empty_prompt_text(result.extracted_content.as_deref())
        {
            let index = blocks.len();
            blocks.push(format!(
                "<read_state_{index}>\n{extracted_content}\n</read_state_{index}>"
            ));
        }
    }

    if blocks.is_empty() {
        None
    } else {
        Some(truncate_prompt_content(blocks.join("\n")))
    }
}

fn non_empty_prompt_text(text: Option<&str>) -> Option<&str> {
    text.filter(|value| !value.is_empty())
}

fn render_planning_context(history: &AgentHistory, settings: &AgentSettings) -> Option<String> {
    if !settings.enable_planning || settings.flash_mode {
        return None;
    }

    let steps_without_plan_update = history
        .items
        .iter()
        .rev()
        .take_while(|item| {
            item.model_output
                .as_ref()
                .and_then(|output| output.plan_update.as_ref())
                .is_none()
        })
        .count();
    let recent_failures = history
        .items
        .iter()
        .rev()
        .take_while(|item| item.result.iter().any(|result| result.error.is_some()))
        .count();

    let mut message = format!(
        "When useful, include `current_plan_item` and `plan_update` to keep multi-step work explicit. Replan after {} stalled/error steps; avoid exploring for more than {} steps without a plan update.",
        settings.planning_replan_on_stall, settings.planning_exploration_limit
    );

    if settings.planning_replan_on_stall > 0 && recent_failures >= settings.planning_replan_on_stall
    {
        message.push_str(
            " Recent steps have failed or stalled, so revise the plan before continuing.",
        );
    } else if settings.planning_exploration_limit > 0
        && steps_without_plan_update >= settings.planning_exploration_limit
    {
        message.push_str(" You have explored for several steps without updating the plan; provide a concise plan_update.");
    }

    Some(message)
}

fn render_loop_awareness(
    history: &AgentHistory,
    state: &BrowserStateSummary,
    settings: &AgentSettings,
) -> Option<String> {
    if !settings.loop_detection_enabled {
        return None;
    }

    let mut messages = Vec::new();
    if let Some((count, window)) = repeated_action_nudge(history, settings.loop_detection_window) {
        messages.push(format!(
            "Heads up: you have repeated a similar action {count} times in the last {window} actions. If this is intentional and making progress, carry on. If not, try a different approach."
        ));
    }

    let stagnant_pages = consecutive_stagnant_pages(history, state);
    if stagnant_pages >= 5 {
        messages.push(format!(
            "The page content has not changed across {stagnant_pages} consecutive observations. Your actions might not be having the intended effect."
        ));
    }

    if messages.is_empty() {
        None
    } else {
        Some(messages.join("\n\n"))
    }
}

fn repeated_action_nudge(history: &AgentHistory, window: usize) -> Option<(usize, usize)> {
    if window == 0 {
        return None;
    }

    let signatures = history
        .items
        .iter()
        .rev()
        .flat_map(|item| item.model_output.as_ref())
        .flat_map(|output| output.action.iter())
        .filter(|action| !matches!(action.name(), "wait" | "done" | "go_back"))
        .take(window)
        .filter_map(action_similarity_signature)
        .collect::<Vec<_>>();

    if signatures.len() < 5 {
        return None;
    }

    let mut counts = std::collections::BTreeMap::<String, usize>::new();
    for signature in &signatures {
        *counts.entry(signature.clone()).or_default() += 1;
    }
    let max_count = counts.values().copied().max().unwrap_or_default();
    (max_count >= 5).then_some((max_count, signatures.len()))
}

fn action_similarity_signature(action: &BrowserAction) -> Option<String> {
    match action {
        BrowserAction::Click(params) => params.index.map(|index| format!("click|{index}")),
        BrowserAction::Input(params) => Some(format!(
            "input|{}|{}",
            params.index,
            params.text.trim().to_ascii_lowercase()
        )),
        BrowserAction::Navigate(params) => Some(format!("navigate|{}", params.url)),
        BrowserAction::Search(params) => Some(format!(
            "search|{:?}|{}",
            params.engine,
            normalized_search_query(&params.query)
        )),
        BrowserAction::Scroll(params) => Some(format!("scroll|{}|{:?}", params.down, params.index)),
        other => serde_json::to_string(other).ok(),
    }
}

fn normalized_search_query(query: &str) -> String {
    let mut tokens = query
        .split(|character: char| !character.is_alphanumeric())
        .filter(|token| !token.is_empty())
        .map(str::to_ascii_lowercase)
        .collect::<Vec<_>>();
    tokens.sort();
    tokens.dedup();
    tokens.join("|")
}

fn consecutive_stagnant_pages(history: &AgentHistory, state: &BrowserStateSummary) -> usize {
    let mut count = 0;
    for item in history.items.iter().rev() {
        if item.state.url == state.url && item.state.dom_state.text == state.dom_state.text {
            count += 1;
        } else {
            break;
        }
    }
    count
}

fn truncate_prompt_content(content: String) -> String {
    if content.chars().count() <= MAX_PROMPT_CONTENT_CHARS {
        return content;
    }

    let truncated = content
        .chars()
        .take(MAX_PROMPT_CONTENT_CHARS)
        .collect::<String>();
    format!("{truncated}\n... [Content truncated at 60k characters]")
}

fn truncate_error_for_prompt(error: &str) -> String {
    if error.chars().count() <= MAX_PROMPT_ERROR_CHARS {
        return error.to_owned();
    }

    let prefix = error
        .chars()
        .take(PROMPT_ERROR_EDGE_CHARS)
        .collect::<String>();
    let suffix = error
        .chars()
        .rev()
        .take(PROMPT_ERROR_EDGE_CHARS)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<String>();
    format!("{prefix}......{suffix}")
}

fn repeated_action_loop(history: &AgentHistory, window: usize) -> bool {
    if window < 2 || history.items.len() < window {
        return false;
    }

    let signatures: Option<Vec<String>> = history
        .items
        .iter()
        .rev()
        .take(window)
        .map(|item| {
            item.model_output
                .as_ref()
                .and_then(|output| action_sequence_similarity_signature(&output.action))
        })
        .collect();

    let Some(signatures) = signatures else {
        return false;
    };
    let Some(first) = signatures.first() else {
        return false;
    };

    signatures.iter().all(|signature| signature == first)
}

fn action_sequence_similarity_signature(actions: &[BrowserAction]) -> Option<String> {
    let signatures = actions
        .iter()
        .filter(|action| !matches!(action.name(), "wait" | "done" | "go_back"))
        .filter_map(action_similarity_signature)
        .collect::<Vec<_>>();
    if signatures.is_empty() {
        None
    } else {
        Some(signatures.join("||"))
    }
}

/// Execute actions with the same high-level guard shape as browser-use:
/// stop on errors, stop on `done`, stop after sequence-terminating actions,
/// and refuse `done` when it is queued after another action.
pub async fn execute_action_sequence<E>(
    executor: &mut E,
    actions: &[BrowserAction],
) -> Vec<ActionResult>
where
    E: ActionExecutor + Send,
{
    let mut results = Vec::new();

    for (index, action) in actions.iter().enumerate() {
        if index > 0 && matches!(action, BrowserAction::Done(_)) {
            break;
        }

        let result = executor.execute(action).await;
        let should_stop = result.is_done || result.error.is_some() || action.terminates_sequence();
        results.push(result);

        if should_stop {
            break;
        }
    }

    results
}

/// Execute a rematched replay plan while preserving history coordinates and
/// the same generic stop rules used by action sequence execution.
pub async fn execute_history_replay_plan<E>(
    executor: &mut E,
    plan: &AgentHistoryReplayPlan,
) -> AgentHistoryReplayExecution
where
    E: ActionExecutor + Send,
{
    let mut items = Vec::new();
    let mut stop = None;

    for (plan_index, item) in plan.actions.iter().enumerate() {
        let action = &item.remapped_action;
        if plan_index > 0 && matches!(action, BrowserAction::Done(_)) {
            stop = Some(AgentHistoryReplayStop {
                step_index: item.step_index,
                action_index: item.action_index,
                reason: AgentHistoryReplayStopReason::DoneAfterPriorAction,
                diagnostic: None,
            });
            break;
        }

        let result = executor.execute(action).await;
        let stop_reason = replay_stop_reason(action, &result);
        let stop_diagnostic = result.error.clone();
        items.push(AgentHistoryReplayExecutionItem {
            step_index: item.step_index,
            action_index: item.action_index,
            original_action: item.original_action.clone(),
            executed_action: action.clone(),
            rematch: item.rematch.clone(),
            result,
        });

        if let Some(reason) = stop_reason {
            stop = Some(AgentHistoryReplayStop {
                step_index: item.step_index,
                action_index: item.action_index,
                reason,
                diagnostic: stop_diagnostic,
            });
            break;
        }
    }

    AgentHistoryReplayExecution { items, stop }
}

fn replay_stop_reason(
    action: &BrowserAction,
    result: &ActionResult,
) -> Option<AgentHistoryReplayStopReason> {
    if result.error.is_some() {
        Some(AgentHistoryReplayStopReason::Error)
    } else if result.is_done {
        Some(AgentHistoryReplayStopReason::Done)
    } else if action.terminates_sequence() {
        Some(AgentHistoryReplayStopReason::TerminatingAction)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use browser_use_cdp::{FoundElement, Pdf, Screenshot};
    use browser_use_dom::SerializedDomState;
    use browser_use_tools::{
        ClickElementAction, CloseTabAction, DoneAction, EvaluateAction, ExtractAction,
        FindElementsAction, FindTextAction, GetDropdownOptionsAction, InputTextAction,
        NavigateAction, NoParamsAction, ReadFileAction, ReplaceFileAction, SaveAsPdfAction,
        ScreenshotAction, ScrollAction, SearchPageAction, SelectDropdownOptionAction,
        SendKeysAction, SwitchTabAction, UploadFileAction, WaitAction, WriteFileAction,
    };
    use std::{
        collections::{BTreeMap, BTreeSet, VecDeque},
        sync::{Arc, Mutex},
    };

    static CWD_LOCK: Mutex<()> = Mutex::new(());

    struct CurrentDirGuard {
        original: std::path::PathBuf,
    }

    impl CurrentDirGuard {
        fn enter(path: &std::path::Path) -> Self {
            let original = std::env::current_dir().expect("current dir");
            std::env::set_current_dir(path).expect("set current dir");
            Self { original }
        }
    }

    impl Drop for CurrentDirGuard {
        fn drop(&mut self) {
            std::env::set_current_dir(&self.original).expect("restore current dir");
        }
    }

    #[test]
    fn target_commit_is_pinned() {
        assert_eq!(INITIAL_UPSTREAM_COMMIT.len(), 40);
    }

    #[test]
    fn settings_defaults_match_browser_use_shape() {
        let settings = AgentSettings::default();

        assert_eq!(settings.use_vision, VisionMode::Always);
        assert_eq!(settings.vision_detail_level, ImageDetailLevel::Auto);
        assert_eq!(settings.max_failures, 5);
        assert_eq!(settings.generate_gif, GenerateGif::Disabled);
        assert_eq!(settings.max_actions_per_step, 5);
        assert_eq!(settings.llm_timeout_seconds, 60);
        assert_eq!(settings.step_timeout_seconds, 180);
        assert_eq!(
            settings.action_timeout_seconds,
            default_action_timeout_seconds()
        );
        assert!(settings.directly_open_url);
        assert!(settings.final_response_after_failure);
        assert!(settings.display_files_in_done_text);
        assert_eq!(settings.loop_detection_window, 20);
        assert!(settings.loop_detection_enabled);
        assert_eq!(settings.max_history_items, None);
        assert_eq!(settings.max_clickable_elements_length, 40_000);
        assert!(!settings.include_recent_events);
        assert!(settings.sample_images.is_empty());
        assert!(settings.enable_planning);
        assert_eq!(settings.planning_replan_on_stall, 3);
        assert_eq!(settings.planning_exploration_limit, 5);
        assert!(settings.use_thinking);
        assert!(!settings.flash_mode);
        assert!(settings.use_judge);
        assert_eq!(settings.ground_truth, None);
        assert_eq!(settings.message_compaction, MessageCompaction::Enabled);
        assert!(!settings.calculate_cost);
        assert!(!settings.include_tool_call_examples);
        assert_eq!(settings.save_conversation_path, None);
        assert_eq!(
            settings.save_conversation_path_encoding.as_deref(),
            Some("utf-8")
        );
        assert!(settings.include_attributes.is_empty());
        assert!(settings.available_file_paths.is_empty());
        assert!(settings.initial_actions.is_empty());
        assert!(settings.excluded_actions.is_empty());
        assert!(settings.sensitive_data.is_empty());
        assert_eq!(settings.override_system_message, None);
        assert_eq!(settings.extend_system_message, None);
    }

    #[test]
    fn action_timeout_parser_accepts_only_finite_positive_seconds() {
        assert_eq!(parse_action_timeout_seconds(None), 180.0);
        assert_eq!(parse_action_timeout_seconds(Some("")), 180.0);
        assert_eq!(parse_action_timeout_seconds(Some("garbage")), 180.0);
        assert_eq!(parse_action_timeout_seconds(Some("nan")), 180.0);
        assert_eq!(parse_action_timeout_seconds(Some("inf")), 180.0);
        assert_eq!(parse_action_timeout_seconds(Some("-1")), 180.0);
        assert_eq!(parse_action_timeout_seconds(Some("0")), 180.0);
        assert_eq!(parse_action_timeout_seconds(Some("12.5")), 12.5);
    }

    #[test]
    fn generate_gif_preserves_upstream_json_shape() {
        assert_eq!(
            serde_json::to_value(GenerateGif::Disabled).expect("serialize disabled"),
            serde_json::json!(false)
        );
        assert_eq!(
            serde_json::to_value(GenerateGif::Enabled).expect("serialize enabled"),
            serde_json::json!(true)
        );
        assert_eq!(
            serde_json::to_value(GenerateGif::Path("trace.gif".to_owned()))
                .expect("serialize path"),
            serde_json::json!("trace.gif")
        );

        assert_eq!(
            serde_json::from_value::<GenerateGif>(serde_json::json!(false))
                .expect("deserialize false"),
            GenerateGif::Disabled
        );
        assert_eq!(
            serde_json::from_value::<GenerateGif>(serde_json::json!(true))
                .expect("deserialize true"),
            GenerateGif::Enabled
        );
        assert_eq!(
            serde_json::from_value::<GenerateGif>(serde_json::json!("trace.gif"))
                .expect("deserialize path"),
            GenerateGif::Path("trace.gif".to_owned())
        );
    }

    #[test]
    fn message_compaction_preserves_upstream_json_shape() {
        assert_eq!(
            serde_json::to_value(MessageCompaction::Disabled).expect("serialize disabled"),
            serde_json::json!(false)
        );
        assert_eq!(
            serde_json::to_value(MessageCompaction::Enabled).expect("serialize enabled"),
            serde_json::json!(true)
        );

        let settings = MessageCompactionSettings {
            compact_every_n_steps: 3,
            trigger_char_count: Some(1200),
            keep_last_items: 2,
            include_read_state: true,
            ..MessageCompactionSettings::default()
        };
        let serialized = serde_json::to_value(MessageCompaction::Settings(settings.clone()))
            .expect("serialize settings");
        assert_eq!(serialized["compact_every_n_steps"], 3);
        assert_eq!(serialized["trigger_char_count"], 1200);
        assert_eq!(serialized["keep_last_items"], 2);
        assert_eq!(serialized["include_read_state"], true);

        assert_eq!(
            serde_json::from_value::<MessageCompaction>(serde_json::json!(false))
                .expect("deserialize false"),
            MessageCompaction::Disabled
        );
        assert_eq!(
            serde_json::from_value::<MessageCompaction>(serde_json::json!(true))
                .expect("deserialize true"),
            MessageCompaction::Enabled
        );
        assert_eq!(
            serde_json::from_value::<MessageCompaction>(serde_json::Value::Null)
                .expect("deserialize null"),
            MessageCompaction::Disabled
        );
        assert_eq!(
            serde_json::from_value::<MessageCompaction>(serde_json::json!({
                "compact_every_n_steps": 3,
                "trigger_char_count": 1200,
                "keep_last_items": 2,
                "include_read_state": true
            }))
            .expect("deserialize settings"),
            MessageCompaction::Settings(settings)
        );

        let token_threshold =
            serde_json::from_value::<MessageCompactionSettings>(serde_json::json!({
                "trigger_token_count": 250,
                "chars_per_token": 3.5
            }))
            .expect("deserialize token threshold");
        assert_eq!(token_threshold.trigger_token_count, Some(250));
        assert_eq!(token_threshold.trigger_char_count, Some(875));

        let both_thresholds =
            serde_json::from_value::<MessageCompactionSettings>(serde_json::json!({
                "trigger_char_count": 100,
                "trigger_token_count": 25
            }));
        assert!(both_thresholds.is_err());
    }

    #[test]
    fn vision_mode_preserves_upstream_json_shape() {
        assert_eq!(
            serde_json::to_value(VisionMode::Always).expect("serialize always"),
            serde_json::json!(true)
        );
        assert_eq!(
            serde_json::to_value(VisionMode::Never).expect("serialize never"),
            serde_json::json!(false)
        );
        assert_eq!(
            serde_json::to_value(VisionMode::Auto).expect("serialize auto"),
            serde_json::json!("auto")
        );

        assert_eq!(
            serde_json::from_value::<VisionMode>(serde_json::json!(true))
                .expect("deserialize true"),
            VisionMode::Always
        );
        assert_eq!(
            serde_json::from_value::<VisionMode>(serde_json::json!(false))
                .expect("deserialize false"),
            VisionMode::Never
        );
        assert_eq!(
            serde_json::from_value::<VisionMode>(serde_json::json!("auto"))
                .expect("deserialize auto"),
            VisionMode::Auto
        );
    }

    #[test]
    fn action_result_rejects_success_true_without_done() {
        let error = serde_json::from_value::<ActionResult>(serde_json::json!({
            "extracted_content": "clicked",
            "success": true
        }))
        .expect_err("success=true without done should be rejected");

        assert!(error.to_string().contains("is_done=true"));
    }

    #[test]
    fn action_result_accepts_failed_non_done_status() {
        let result: ActionResult = serde_json::from_value(serde_json::json!({
            "error": "click failed",
            "success": false
        }))
        .expect("success=false remains valid for failed actions");

        assert!(!result.is_done);
        assert_eq!(result.success, Some(false));
    }

    #[test]
    fn action_result_accepts_successful_done_status() {
        let result: ActionResult = serde_json::from_value(serde_json::json!({
            "extracted_content": "complete",
            "is_done": true,
            "success": true
        }))
        .expect("done success should deserialize");

        assert!(result.is_done);
        assert_eq!(result.success, Some(true));
    }

    #[test]
    fn action_result_deserializes_trace_judgement() {
        let result: ActionResult = serde_json::from_value(serde_json::json!({
            "judgement": {
                "reasoning": "visual state matched expected result",
                "verdict": true,
                "failure_reason": null,
                "impossible_task": false,
                "reached_captcha": false
            }
        }))
        .expect("judgement should deserialize");

        let judgement = result.judgement.expect("judgement");
        assert!(judgement.verdict);
        assert_eq!(
            judgement.reasoning.as_deref(),
            Some("visual state matched expected result")
        );
    }

    #[test]
    fn action_result_deserializes_image_payloads() {
        let result: ActionResult = serde_json::from_value(serde_json::json!({
            "extracted_content": "Read image file chart.png.",
            "images": [
                {
                    "name": "chart.png",
                    "data": "abc123"
                }
            ]
        }))
        .expect("image payloads should deserialize");

        assert_eq!(result.images.len(), 1);
        assert_eq!(result.images[0]["name"], "chart.png");
        assert_eq!(result.images[0]["data"], "abc123");
    }

    #[test]
    fn agent_output_accepts_flattened_planning_shape() {
        let output: AgentOutput = serde_json::from_value(serde_json::json!({
            "thinking": "Need a plan",
            "evaluation_previous_goal": "No previous step",
            "memory": "Need to search",
            "next_goal": "Open search",
            "current_plan_item": 1,
            "plan_update": ["Find search box", "Submit query"],
            "action": [
                {
                    "done": {
                        "text": "planned",
                        "success": true,
                        "files_to_display": []
                    }
                }
            ]
        }))
        .expect("flattened agent output");

        assert!(output.current_state.is_empty());
        assert_eq!(output.current_plan_item, Some(1));
        assert_eq!(
            output.plan_update.as_ref().expect("plan update"),
            &vec!["Find search box".to_owned(), "Submit query".to_owned()]
        );
        let brain = output.current_brain();
        assert_eq!(brain.thinking.as_deref(), Some("Need a plan"));
        assert_eq!(
            brain.evaluation_previous_goal.as_deref(),
            Some("No previous step")
        );
        assert_eq!(brain.memory.as_deref(), Some("Need to search"));
        assert_eq!(brain.next_goal.as_deref(), Some("Open search"));

        let serialized = serde_json::to_value(&output).expect("serialize output");
        assert!(serialized.get("current_state").is_none());
        assert_eq!(serialized["current_plan_item"], 1);
        assert_eq!(serialized["plan_update"][0], "Find search box");
    }

    fn schema_required_fields(schema: &Value) -> Vec<&str> {
        schema["required"]
            .as_array()
            .expect("required fields")
            .iter()
            .map(|field| field.as_str().expect("required field string"))
            .collect()
    }

    fn schema_action_names(schema: &Value) -> BTreeSet<String> {
        for pointer in [
            "/$defs/BrowserAction/oneOf",
            "/$defs/BrowserAction/anyOf",
            "/definitions/BrowserAction/oneOf",
            "/definitions/BrowserAction/anyOf",
        ] {
            if let Some(actions) = schema.pointer(pointer).and_then(Value::as_array) {
                return actions
                    .iter()
                    .filter_map(schema_variant_action_name)
                    .map(ToOwned::to_owned)
                    .collect();
            }
        }
        BTreeSet::new()
    }

    #[test]
    fn agent_output_schema_exposes_planning_fields() {
        let schema = schema_for_agent_output();
        let schema_text = serde_json::to_string(&schema).expect("schema text");

        assert!(schema_text.contains("current_plan_item"));
        assert!(schema_text.contains("plan_update"));
        assert!(schema_text.contains("evaluation_previous_goal"));
    }

    #[test]
    fn agent_output_schema_filters_excluded_actions() {
        let settings = AgentSettings {
            excluded_actions: vec![
                "search".to_owned(),
                "scroll".to_owned(),
                "switch-tab".to_owned(),
                "done".to_owned(),
            ],
            ..AgentSettings::default()
        };
        let schema = schema_for_agent_output_with_settings(&settings);
        let action_names = schema_action_names(&schema);

        assert!(!action_names.contains("search"));
        assert!(!action_names.contains("scroll"));
        assert!(!action_names.contains("switch_tab"));
        assert!(action_names.contains("navigate"));
        assert!(action_names.contains("done"));
    }

    #[test]
    fn agent_output_schema_gates_screenshot_action_on_auto_vision() {
        let default_schema = schema_for_agent_output_with_settings(&AgentSettings::default());
        let default_actions = schema_action_names(&default_schema);
        assert!(!default_actions.contains("screenshot"));

        let auto_settings = AgentSettings {
            use_vision: VisionMode::Auto,
            ..AgentSettings::default()
        };
        let auto_schema = schema_for_agent_output_with_settings(&auto_settings);
        let auto_actions = schema_action_names(&auto_schema);
        assert!(auto_actions.contains("screenshot"));

        let disabled_settings = AgentSettings {
            use_vision: VisionMode::Never,
            ..AgentSettings::default()
        };
        let disabled_schema = schema_for_agent_output_with_settings(&disabled_settings);
        let disabled_actions = schema_action_names(&disabled_schema);
        assert!(!disabled_actions.contains("screenshot"));
    }

    #[test]
    fn final_response_schema_keeps_done_when_actions_are_excluded() {
        let settings = AgentSettings {
            excluded_actions: vec!["search".to_owned(), "done".to_owned()],
            ..AgentSettings::default()
        };
        let schema = schema_for_final_response_after_failure(&settings);
        let action_names = schema_action_names(&schema);

        assert_eq!(action_names, BTreeSet::from(["done".to_owned()]));
    }

    #[test]
    fn step_limit_final_response_request_uses_done_only_schema_and_instruction() {
        let request = build_final_response_after_step_limit_request(
            "finish the task",
            &blank_state(),
            &AgentHistory::default(),
            &AgentSettings::default(),
            None,
            3,
        )
        .expect("step limit final response request");
        let action_names =
            schema_action_names(request.output_schema.as_ref().expect("output schema"));

        assert_eq!(action_names, BTreeSet::from(["done".to_owned()]));
        let text = request_text(&request);
        assert!(text.contains("You reached max_steps (3)"));
        assert!(text.contains("only available action is done"));
        assert!(text.contains("set success to false"));
    }

    #[test]
    fn step_budget_warning_request_matches_upstream_threshold_text() {
        assert!(!should_inject_step_budget_warning(2, 4));
        assert!(should_inject_step_budget_warning(3, 4));
        assert!(!should_inject_step_budget_warning(4, 4));

        let request = build_step_request_with_budget_warning(
            "finish the task",
            &blank_state(),
            &AgentHistory::default(),
            &AgentSettings::default(),
            None,
            3,
            4,
        )
        .expect("budget warning request");
        let action_names =
            schema_action_names(request.output_schema.as_ref().expect("output schema"));

        assert!(action_names.contains("click"));
        assert!(action_names.contains("done"));
        let text = request_text(&request);
        assert!(text.contains("BUDGET WARNING: You have used 3/4 steps (75%). 1 steps remaining."));
        assert!(text.contains("Partial results are far more valuable"));
    }

    #[test]
    fn agent_output_schema_honors_thinking_and_flash_settings() {
        let no_thinking = AgentSettings {
            use_thinking: false,
            ..AgentSettings::default()
        };
        let schema = schema_for_agent_output_with_settings(&no_thinking);
        let properties = schema["properties"].as_object().expect("properties");
        assert!(!properties.contains_key("thinking"));
        assert!(!properties.contains_key("current_state"));
        assert!(properties.contains_key("current_plan_item"));
        assert_eq!(
            schema_required_fields(&schema),
            vec!["evaluation_previous_goal", "memory", "next_goal", "action"]
        );

        let flash = AgentSettings {
            flash_mode: true,
            ..AgentSettings::default()
        };
        let schema = schema_for_agent_output_with_settings(&flash);
        let properties = schema["properties"].as_object().expect("properties");
        assert!(!properties.contains_key("thinking"));
        assert!(!properties.contains_key("current_state"));
        assert!(!properties.contains_key("evaluation_previous_goal"));
        assert!(!properties.contains_key("next_goal"));
        assert!(!properties.contains_key("current_plan_item"));
        assert!(!properties.contains_key("plan_update"));
        assert!(properties.contains_key("memory"));
        assert!(properties.contains_key("action"));
        assert_eq!(schema_required_fields(&schema), vec!["memory", "action"]);
    }

    #[test]
    fn agent_output_schema_matches_upstream_required_fields() {
        let schema = schema_for_agent_output_with_settings(&AgentSettings::default());
        let properties = schema["properties"].as_object().expect("properties");

        assert!(!properties.contains_key("current_state"));
        assert_eq!(
            schema_required_fields(&schema),
            vec!["evaluation_previous_goal", "memory", "next_goal", "action"]
        );
        assert_eq!(schema["properties"]["action"]["minItems"], 1);
    }

    #[test]
    fn final_response_after_failure_schema_allows_only_done_action() {
        let schema = schema_for_final_response_after_failure(&AgentSettings::default());
        let variants = schema
            .pointer("/$defs/BrowserAction/oneOf")
            .or_else(|| schema.pointer("/$defs/BrowserAction/anyOf"))
            .or_else(|| schema.pointer("/definitions/BrowserAction/oneOf"))
            .or_else(|| schema.pointer("/definitions/BrowserAction/anyOf"))
            .expect("browser action variants")
            .as_array()
            .expect("variant array");

        assert_eq!(variants.len(), 1);
        let properties = variants[0]
            .get("properties")
            .and_then(Value::as_object)
            .expect("done action properties");
        assert_eq!(properties.len(), 1);
        assert!(properties.contains_key("done"));
    }

    #[test]
    fn history_returns_latest_done_result() {
        let history = AgentHistory {
            items: vec![AgentHistoryItem {
                model_output: None,
                result: vec![ActionResult::done("finished", true)],
                state: blank_state(),
                metadata: None,
            }],
            ..AgentHistory::default()
        };

        assert_eq!(history.final_result(), Some("finished"));
        assert!(history.is_done());
        assert_eq!(history.is_successful(), Some(true));
        assert_eq!(history.errors(), vec![None]);
        assert!(!history.has_errors());
    }

    #[test]
    fn history_collects_errors_and_failed_done_status() {
        let history = AgentHistory {
            items: vec![
                AgentHistoryItem {
                    model_output: None,
                    result: vec![ActionResult::error("first failure")],
                    state: blank_state(),
                    metadata: None,
                },
                AgentHistoryItem {
                    model_output: None,
                    result: vec![ActionResult::done("could not finish", false)],
                    state: blank_state(),
                    metadata: None,
                },
            ],
            ..AgentHistory::default()
        };

        assert_eq!(history.final_result(), Some("could not finish"));
        assert!(history.is_done());
        assert_eq!(history.is_successful(), Some(false));
        assert_eq!(history.errors(), vec![Some("first failure"), None]);
        assert!(history.has_errors());
    }

    #[test]
    fn history_judgement_helpers_use_last_result_like_browser_use() {
        let judgement = JudgementResult {
            reasoning: Some("trace satisfied the task".to_owned()),
            verdict: true,
            failure_reason: Some(String::new()),
            impossible_task: false,
            reached_captcha: false,
        };
        let history = AgentHistory {
            items: vec![AgentHistoryItem {
                model_output: None,
                result: vec![ActionResult {
                    extracted_content: Some("judged".to_owned()),
                    error: None,
                    judgement: Some(judgement.clone()),
                    long_term_memory: None,
                    include_extracted_content_only_once: false,
                    include_in_memory: false,
                    is_done: true,
                    success: Some(true),
                    attachments: Vec::new(),
                    images: Vec::new(),
                    metadata: None,
                }],
                state: blank_state(),
                metadata: None,
            }],
            ..AgentHistory::default()
        };

        assert_eq!(history.judgement(), Some(&judgement));
        assert!(history.is_judged());
        assert_eq!(history.is_validated(), Some(true));

        let serialized = serde_json::to_value(&history).expect("serialize history");
        assert_eq!(
            serialized["items"][0]["result"][0]["judgement"]["verdict"],
            true
        );
    }

    #[test]
    fn history_judgement_helpers_ignore_prior_non_terminal_judgement() {
        let history = AgentHistory {
            items: vec![
                AgentHistoryItem {
                    model_output: None,
                    result: vec![ActionResult {
                        extracted_content: Some("judged earlier".to_owned()),
                        error: None,
                        judgement: Some(JudgementResult {
                            reasoning: None,
                            verdict: false,
                            failure_reason: Some("missing final evidence".to_owned()),
                            impossible_task: false,
                            reached_captcha: false,
                        }),
                        long_term_memory: None,
                        include_extracted_content_only_once: false,
                        include_in_memory: false,
                        is_done: false,
                        success: None,
                        attachments: Vec::new(),
                        images: Vec::new(),
                        metadata: None,
                    }],
                    state: blank_state(),
                    metadata: None,
                },
                AgentHistoryItem {
                    model_output: None,
                    result: vec![ActionResult::extracted("latest result")],
                    state: blank_state(),
                    metadata: None,
                },
            ],
            ..AgentHistory::default()
        };

        assert_eq!(history.judgement(), None);
        assert!(!history.is_judged());
        assert_eq!(history.is_validated(), None);
    }

    #[test]
    fn history_completion_helpers_use_last_result_like_browser_use() {
        let history = AgentHistory {
            items: vec![
                AgentHistoryItem {
                    model_output: None,
                    result: vec![ActionResult::done("finished earlier", true)],
                    state: blank_state(),
                    metadata: None,
                },
                AgentHistoryItem {
                    model_output: None,
                    result: vec![ActionResult::extracted("latest non-terminal result")],
                    state: blank_state(),
                    metadata: None,
                },
            ],
            ..AgentHistory::default()
        };

        assert_eq!(history.final_result(), Some("latest non-terminal result"));
        assert!(!history.is_done());
        assert_eq!(history.is_successful(), None);
    }

    #[test]
    fn step_metadata_matches_browser_use_shape() {
        let metadata = StepMetadata {
            step_number: 2,
            step_start_time: 10.0,
            step_end_time: 13.5,
            step_interval: Some(2.0),
        };

        assert_eq!(metadata.duration_seconds(), 3.5);

        let serialized = serde_json::to_value(&metadata).expect("serialize metadata");
        assert_eq!(serialized["step_number"], 2);
        assert_eq!(serialized["step_start_time"], 10.0);
        assert_eq!(serialized["step_end_time"], 13.5);
        assert_eq!(serialized["step_interval"], 2.0);

        let without_interval: StepMetadata = serde_json::from_value(serde_json::json!({
            "step_number": 1,
            "step_start_time": 1.0,
            "step_end_time": 2.0
        }))
        .expect("deserialize old metadata");
        assert_eq!(without_interval.step_interval, None);

        let serialized_without_interval =
            serde_json::to_value(&without_interval).expect("serialize metadata");
        assert!(serialized_without_interval.get("step_interval").is_some());
        assert_eq!(serialized_without_interval["step_interval"], Value::Null);
    }

    #[test]
    fn history_helpers_match_browser_use_accessors() {
        let mut first_state = blank_state();
        first_state.url = "https://example.test/start".to_owned();
        let mut second_state = blank_state();
        second_state.url = "https://example.test/done".to_owned();
        let current_state = AgentCurrentState {
            thinking: None,
            evaluation_previous_goal: None,
            memory: None,
            next_goal: None,
        };
        let history = AgentHistory {
            items: vec![
                AgentHistoryItem {
                    model_output: Some(AgentOutput {
                        current_state: current_state.clone(),
                        thinking: None,
                        evaluation_previous_goal: None,
                        memory: None,
                        next_goal: None,
                        current_plan_item: None,
                        plan_update: None,
                        action: vec![BrowserAction::Click(ClickElementAction {
                            index: Some(1),
                            coordinate_x: None,
                            coordinate_y: None,
                        })],
                    }),
                    result: vec![ActionResult::extracted("Clicked element 1")],
                    state: first_state,
                    metadata: Some(StepMetadata {
                        step_number: 1,
                        step_start_time: 10.0,
                        step_end_time: 11.5,
                        step_interval: None,
                    }),
                },
                AgentHistoryItem {
                    model_output: Some(AgentOutput {
                        current_state,
                        thinking: None,
                        evaluation_previous_goal: None,
                        memory: None,
                        next_goal: None,
                        current_plan_item: None,
                        plan_update: None,
                        action: vec![BrowserAction::Done(DoneAction {
                            text: "finished".to_owned(),
                            success: true,
                            files_to_display: Vec::new(),
                        })],
                    }),
                    result: vec![ActionResult::done("finished", true)],
                    state: second_state,
                    metadata: Some(StepMetadata {
                        step_number: 2,
                        step_start_time: 20.0,
                        step_end_time: 22.0,
                        step_interval: Some(1.5),
                    }),
                },
            ],
            ..AgentHistory::default()
        };

        assert_eq!(history.number_of_steps(), 2);
        assert_eq!(history.action_results().len(), 2);
        assert_eq!(
            history.extracted_content(),
            vec!["Clicked element 1", "finished"]
        );
        assert_eq!(
            history.urls(),
            vec!["https://example.test/start", "https://example.test/done"]
        );
        assert_eq!(history.action_names(), vec!["click", "done"]);
        assert_eq!(history.model_outputs().len(), 2);
        assert_eq!(history.model_thoughts().len(), 2);
        assert_eq!(history.model_actions().len(), 2);
        assert_eq!(history.model_actions_filtered(&["click"]).len(), 1);
        assert_eq!(
            history.action_history(),
            vec![
                vec![serde_json::json!({
                    "click": {
                        "index": 1
                    },
                    "interacted_element": null,
                    "result": "Clicked element 1"
                })],
                vec![serde_json::json!({
                    "done": {
                        "text": "finished",
                        "success": true,
                        "files_to_display": []
                    },
                    "interacted_element": null,
                    "result": null
                })],
            ]
        );
        assert_eq!(
            history.last_action().expect("last action"),
            serde_json::json!({
                "done": {
                    "text": "finished",
                    "success": true,
                    "files_to_display": []
                }
            })
        );
        assert!((history.total_duration_seconds() - 3.5).abs() < f64::EPSILON);
    }

    #[test]
    fn action_history_includes_interacted_element_for_indexed_actions() {
        let element = browser_use_dom::DomElementRef {
            index: 1,
            target_id: "target-123".to_owned(),
            backend_node_id: 44,
            node_id: Some(9),
            tag_name: "input".to_owned(),
            role: Some("textbox".to_owned()),
            name: Some("Email".to_owned()),
            text: Some("ada@example.test".to_owned()),
            attributes: BTreeMap::from([
                ("id".to_owned(), "email".to_owned()),
                ("type".to_owned(), "email".to_owned()),
                ("ax_name".to_owned(), "Email".to_owned()),
            ]),
            bounds: Some(browser_use_dom::ElementBounds {
                x: 5,
                y: 10,
                width: 200,
                height: 30,
            }),
            is_visible: true,
            is_interactive: true,
            is_scrollable: false,
        };
        let mut state = blank_state();
        state.dom_state = browser_use_dom::SerializedDomState::from_elements(vec![element.clone()]);
        let history = AgentHistory {
            items: vec![AgentHistoryItem {
                model_output: Some(AgentOutput {
                    current_state: AgentCurrentState::default(),
                    thinking: None,
                    evaluation_previous_goal: None,
                    memory: None,
                    next_goal: None,
                    current_plan_item: None,
                    plan_update: None,
                    action: vec![
                        BrowserAction::Input(InputTextAction {
                            index: 1,
                            text: "ada@example.test".to_owned(),
                            clear: true,
                        }),
                        BrowserAction::Wait(WaitAction { seconds: 0 }),
                    ],
                }),
                result: vec![
                    ActionResult::extracted("Typed into element 1"),
                    ActionResult::extracted("Paused"),
                ],
                state,
                metadata: None,
            }],
            ..AgentHistory::default()
        };

        let action_history = history.action_history();
        assert_eq!(
            action_history[0][0]["interacted_element"],
            serde_json::to_value(DomInteractedElement::from_element(&element))
                .expect("interacted element")
        );
        assert_eq!(action_history[0][0]["result"], "Typed into element 1");
        assert_eq!(action_history[0][1]["interacted_element"], Value::Null);
        assert_eq!(action_history[0][1]["result"], "Paused");

        let model_actions = history.model_actions();
        assert_eq!(
            model_actions[0]["interacted_element"],
            serde_json::to_value(DomInteractedElement::from_element(&element))
                .expect("interacted element")
        );
        assert_eq!(model_actions[1]["interacted_element"], Value::Null);
    }

    #[test]
    fn replay_rematch_updates_supported_indexed_actions() {
        let historical_element = replay_dom_element(
            1,
            "input",
            BTreeMap::from([("id".to_owned(), "email".to_owned())]),
        );
        let interacted = DomInteractedElement::from_element(&historical_element);
        let current_dom = SerializedDomState::from_elements(vec![replay_dom_element(
            7,
            "input",
            BTreeMap::from([("id".to_owned(), "email".to_owned())]),
        )]);
        let actions = vec![
            BrowserAction::Click(ClickElementAction {
                index: Some(1),
                coordinate_x: None,
                coordinate_y: None,
            }),
            BrowserAction::Input(InputTextAction {
                index: 1,
                text: "ada@example.test".to_owned(),
                clear: true,
            }),
            BrowserAction::Scroll(ScrollAction {
                down: true,
                pages: 1.0,
                index: Some(1),
            }),
            BrowserAction::UploadFile(UploadFileAction {
                index: 1,
                path: "/tmp/file.txt".to_owned(),
            }),
            BrowserAction::GetDropdownOptions(GetDropdownOptionsAction { index: 1 }),
            BrowserAction::SelectDropdownOption(SelectDropdownOptionAction {
                index: 1,
                text: "Team".to_owned(),
            }),
        ];

        for action in actions {
            let rematch = rematch_action_for_replay(&action, Some(&interacted), &current_dom)
                .expect("rematched action");
            assert_eq!(rematch.original_index, Some(1));
            assert_eq!(rematch.rematched_index, Some(7));
            assert!(rematch.changed);
            assert_eq!(rematch.action.interacted_element_index(), Some(7));
            assert_eq!(
                rematch.match_result.expect("match result").level,
                DomInteractedElementMatchLevel::Exact
            );
        }
    }

    #[test]
    fn replay_rematch_preserves_non_indexed_or_missing_history_actions() {
        let current_dom = SerializedDomState::from_elements(vec![replay_dom_element(
            3,
            "button",
            BTreeMap::from([("id".to_owned(), "save".to_owned())]),
        )]);
        let coordinate_click = BrowserAction::Click(ClickElementAction {
            index: None,
            coordinate_x: Some(10),
            coordinate_y: Some(20),
        });
        let wait = BrowserAction::Wait(WaitAction { seconds: 0 });
        let input = BrowserAction::Input(InputTextAction {
            index: 1,
            text: "unchanged".to_owned(),
            clear: true,
        });

        let click_rematch =
            rematch_action_for_replay(&coordinate_click, None, &current_dom).expect("click");
        assert_eq!(click_rematch.action, coordinate_click);
        assert_eq!(click_rematch.original_index, None);
        assert!(!click_rematch.changed);

        let wait_rematch = rematch_action_for_replay(&wait, None, &current_dom).expect("wait");
        assert_eq!(wait_rematch.action, wait);
        assert_eq!(wait_rematch.original_index, None);
        assert!(!wait_rematch.changed);

        let input_rematch = rematch_action_for_replay(&input, None, &current_dom).expect("input");
        assert_eq!(input_rematch.action, input);
        assert_eq!(input_rematch.original_index, Some(1));
        assert_eq!(input_rematch.rematched_index, None);
        assert!(!input_rematch.changed);
    }

    #[test]
    fn replay_rematch_reports_noop_and_ambiguous_matches() {
        let historical_element = replay_dom_element(
            1,
            "button",
            BTreeMap::from([("id".to_owned(), "save".to_owned())]),
        );
        let interacted = DomInteractedElement::from_element(&historical_element);
        let current_dom = SerializedDomState::from_elements(vec![replay_dom_element(
            1,
            "button",
            BTreeMap::from([("id".to_owned(), "save".to_owned())]),
        )]);
        let action = BrowserAction::Click(ClickElementAction {
            index: Some(1),
            coordinate_x: None,
            coordinate_y: None,
        });

        let rematch = rematch_action_for_replay(&action, Some(&interacted), &current_dom)
            .expect("same-index match");
        assert_eq!(rematch.action, action);
        assert_eq!(rematch.original_index, Some(1));
        assert_eq!(rematch.rematched_index, Some(1));
        assert!(!rematch.changed);

        let mut ambiguous = DomInteractedElement::from_element(&historical_element);
        ambiguous.element_hash = 0;
        ambiguous.stable_hash = None;
        ambiguous.x_path = String::new();
        ambiguous.ax_name = Some("Duplicate".to_owned());
        let mut first = replay_dom_element(1, "button", BTreeMap::new());
        first.name = Some("Duplicate".to_owned());
        let mut second = replay_dom_element(2, "button", BTreeMap::new());
        second.name = Some("Duplicate".to_owned());
        let current_dom = SerializedDomState::from_elements(vec![first, second]);
        let error =
            rematch_action_for_replay(&action, Some(&ambiguous), &current_dom).expect_err("error");
        assert_eq!(
            error.reason,
            DomInteractedElementMatchFailureReason::Ambiguous
        );

        let missing = DomInteractedElement {
            element_hash: 0,
            stable_hash: None,
            x_path: String::new(),
            ax_name: Some("Missing".to_owned()),
            ..DomInteractedElement::from_element(&historical_element)
        };
        let current_dom = SerializedDomState::from_elements(vec![replay_dom_element(
            3,
            "button",
            BTreeMap::from([("id".to_owned(), "other".to_owned())]),
        )]);
        let error =
            rematch_action_for_replay(&action, Some(&missing), &current_dom).expect_err("missing");
        assert_eq!(
            error.reason,
            DomInteractedElementMatchFailureReason::NotFound
        );
    }

    #[test]
    fn history_replay_plan_remaps_actions_across_steps() {
        let mut first_state = blank_state();
        first_state.dom_state = SerializedDomState::from_elements(vec![replay_dom_element(
            1,
            "button",
            BTreeMap::from([("id".to_owned(), "save".to_owned())]),
        )]);
        let mut second_state = blank_state();
        second_state.dom_state = SerializedDomState::from_elements(vec![replay_dom_element(
            2,
            "input",
            BTreeMap::from([("name".to_owned(), "email".to_owned())]),
        )]);
        let history = AgentHistory {
            items: vec![
                history_item_with_state_actions(
                    first_state,
                    vec![BrowserAction::Click(ClickElementAction {
                        index: Some(1),
                        coordinate_x: None,
                        coordinate_y: None,
                    })],
                ),
                history_item_with_state_actions(
                    second_state,
                    vec![
                        BrowserAction::Input(InputTextAction {
                            index: 2,
                            text: "ada@example.test".to_owned(),
                            clear: true,
                        }),
                        BrowserAction::Wait(WaitAction { seconds: 0 }),
                    ],
                ),
            ],
            ..AgentHistory::default()
        };
        let current_dom = SerializedDomState::from_elements(vec![
            replay_dom_element(
                7,
                "button",
                BTreeMap::from([("id".to_owned(), "save".to_owned())]),
            ),
            replay_dom_element(
                8,
                "input",
                BTreeMap::from([("name".to_owned(), "email".to_owned())]),
            ),
        ]);

        let plan = history.replay_plan(&current_dom).expect("replay plan");

        assert_eq!(plan.actions.len(), 3);
        assert_eq!(plan.actions[0].step_index, 0);
        assert_eq!(plan.actions[0].action_index, 0);
        assert_eq!(
            plan.actions[0].remapped_action.interacted_element_index(),
            Some(7)
        );
        assert_eq!(
            plan.actions[0].rematch.action.interacted_element_index(),
            Some(7)
        );
        assert!(plan.actions[0].rematch.changed);
        assert_eq!(plan.actions[1].step_index, 1);
        assert_eq!(plan.actions[1].action_index, 0);
        assert_eq!(
            plan.actions[1].remapped_action.interacted_element_index(),
            Some(8)
        );
        assert_eq!(
            plan.actions[1].rematch.action.interacted_element_index(),
            Some(8)
        );
        assert!(plan.actions[1].rematch.changed);
        assert_eq!(plan.actions[2].step_index, 1);
        assert_eq!(plan.actions[2].action_index, 1);
        assert_eq!(
            plan.actions[2].original_action,
            plan.actions[2].remapped_action
        );
        assert_eq!(plan.actions[2].rematch.original_index, None);
        assert!(!plan.actions[2].rematch.changed);
    }

    #[test]
    fn history_replay_plan_preserves_missing_historical_selector_entries() {
        let history = AgentHistory {
            items: vec![history_item_with_state_actions(
                blank_state(),
                vec![BrowserAction::Input(InputTextAction {
                    index: 99,
                    text: "missing".to_owned(),
                    clear: true,
                })],
            )],
            ..AgentHistory::default()
        };
        let current_dom = SerializedDomState::from_elements(vec![replay_dom_element(
            1,
            "input",
            BTreeMap::from([("name".to_owned(), "email".to_owned())]),
        )]);

        let plan = history.replay_plan(&current_dom).expect("replay plan");

        assert_eq!(plan.actions.len(), 1);
        assert_eq!(
            plan.actions[0].original_action,
            plan.actions[0].remapped_action
        );
        assert_eq!(plan.actions[0].rematch.original_index, Some(99));
        assert_eq!(plan.actions[0].rematch.rematched_index, None);
        assert!(!plan.actions[0].rematch.changed);
    }

    #[test]
    fn history_replay_plan_attaches_step_coordinates_to_rematch_failures() {
        let mut old = replay_dom_element(1, "button", BTreeMap::new());
        old.name = Some("Duplicate".to_owned());
        let mut state = blank_state();
        state.dom_state = SerializedDomState::from_elements(vec![old]);
        let history = AgentHistory {
            items: vec![history_item_with_state_actions(
                state,
                vec![BrowserAction::Click(ClickElementAction {
                    index: Some(1),
                    coordinate_x: None,
                    coordinate_y: None,
                })],
            )],
            ..AgentHistory::default()
        };
        let mut first = replay_dom_element(2, "button", BTreeMap::new());
        first.name = Some("Duplicate".to_owned());
        let mut second = replay_dom_element(3, "button", BTreeMap::new());
        second.name = Some("Duplicate".to_owned());
        let current_dom = SerializedDomState::from_elements(vec![first, second]);

        let error = history.replay_plan(&current_dom).expect_err("ambiguous");

        assert_eq!(error.step_index, 0);
        assert_eq!(error.action_index, 0);
        assert_eq!(error.original_action.interacted_element_index(), Some(1));
        assert_eq!(error.original_index, Some(1));
        assert_eq!(
            error.failure.reason,
            DomInteractedElementMatchFailureReason::Ambiguous
        );

        let mut state = blank_state();
        state.dom_state = SerializedDomState::from_elements(vec![replay_dom_element(
            1,
            "button",
            BTreeMap::from([("data-testid".to_owned(), "missing".to_owned())]),
        )]);
        let history = AgentHistory {
            items: vec![history_item_with_state_actions(
                state,
                vec![BrowserAction::Click(ClickElementAction {
                    index: Some(1),
                    coordinate_x: None,
                    coordinate_y: None,
                })],
            )],
            ..AgentHistory::default()
        };
        let current_dom = SerializedDomState::from_elements(vec![replay_dom_element(
            9,
            "button",
            BTreeMap::from([("data-testid".to_owned(), "other".to_owned())]),
        )]);

        let error = history
            .replay_plan(&current_dom)
            .expect_err("missing current rematch");

        assert_eq!(
            error.failure.reason,
            DomInteractedElementMatchFailureReason::NotFound
        );
        assert_eq!(error.original_index, Some(1));
    }

    #[test]
    fn history_screenshots_match_browser_use_accessor() {
        let mut first_state = blank_state();
        first_state.screenshot = Some("first-shot".to_owned());
        let second_state = blank_state();
        let mut third_state = blank_state();
        third_state.screenshot = Some("third-shot".to_owned());

        let history = AgentHistory {
            items: vec![
                AgentHistoryItem {
                    model_output: None,
                    result: Vec::new(),
                    state: first_state,
                    metadata: None,
                },
                AgentHistoryItem {
                    model_output: None,
                    result: Vec::new(),
                    state: second_state,
                    metadata: None,
                },
                AgentHistoryItem {
                    model_output: None,
                    result: Vec::new(),
                    state: third_state,
                    metadata: None,
                },
            ],
            ..AgentHistory::default()
        };

        assert_eq!(
            history.screenshots(None, true),
            vec![Some("first-shot"), None, Some("third-shot")]
        );
        assert_eq!(
            history.screenshots(None, false),
            vec![Some("first-shot"), Some("third-shot")]
        );
        assert_eq!(
            history.screenshots(Some(2), true),
            vec![None, Some("third-shot")]
        );
        assert!(history.screenshots(Some(0), true).is_empty());
    }

    #[test]
    fn history_judgement_helpers_use_terminal_result() {
        let judgement = JudgementResult {
            reasoning: Some("visual state did not match".to_owned()),
            verdict: false,
            failure_reason: Some("missing confirmation".to_owned()),
            impossible_task: false,
            reached_captcha: false,
        };
        let history = AgentHistory {
            items: vec![AgentHistoryItem {
                model_output: None,
                result: vec![ActionResult {
                    extracted_content: Some("validated".to_owned()),
                    error: None,
                    judgement: Some(judgement),
                    long_term_memory: None,
                    include_extracted_content_only_once: false,
                    include_in_memory: false,
                    is_done: true,
                    success: Some(false),
                    attachments: Vec::new(),
                    images: Vec::new(),
                    metadata: None,
                }],
                state: blank_state(),
                metadata: None,
            }],
            ..AgentHistory::default()
        };

        assert!(history.is_judged());
        assert_eq!(history.is_validated(), Some(false));
        assert_eq!(
            history
                .judgement()
                .and_then(|judgement| judgement.failure_reason.as_deref()),
            Some("missing confirmation")
        );
    }

    fn blank_state() -> BrowserStateSummary {
        BrowserStateSummary {
            dom_state: Default::default(),
            url: "about:blank".to_owned(),
            title: "Blank".to_owned(),
            tabs: vec![],
            screenshot: None,
            page_info: None,
            pixels_above: 0,
            pixels_below: 0,
            browser_errors: vec![],
            is_pdf_viewer: false,
            recent_events: None,
            pending_network_requests: vec![],
            pagination_buttons: vec![],
            closed_popup_messages: vec![],
        }
    }

    fn test_png_base64(width: u32, height: u32) -> String {
        let image = image::RgbaImage::from_pixel(width, height, image::Rgba([12, 34, 56, 255]));
        let mut cursor = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(image)
            .write_to(&mut cursor, image::ImageFormat::Png)
            .expect("encode png");
        base64::engine::general_purpose::STANDARD.encode(cursor.into_inner())
    }

    fn replay_dom_element(
        index: u32,
        tag_name: &str,
        attributes: BTreeMap<String, String>,
    ) -> browser_use_dom::DomElementRef {
        browser_use_dom::DomElementRef {
            index,
            target_id: format!("target-{index}"),
            backend_node_id: u64::from(index),
            node_id: Some(u64::from(index)),
            tag_name: tag_name.to_owned(),
            role: None,
            name: None,
            text: None,
            attributes,
            bounds: None,
            is_visible: true,
            is_interactive: true,
            is_scrollable: false,
        }
    }

    fn history_item_with_actions(actions: Vec<BrowserAction>) -> AgentHistoryItem {
        history_item_with_state_actions(blank_state(), actions)
    }

    fn history_item_with_state_actions(
        state: BrowserStateSummary,
        actions: Vec<BrowserAction>,
    ) -> AgentHistoryItem {
        AgentHistoryItem {
            model_output: Some(AgentOutput {
                current_state: AgentCurrentState::default(),
                thinking: None,
                evaluation_previous_goal: None,
                memory: None,
                next_goal: None,
                current_plan_item: None,
                plan_update: None,
                action: actions,
            }),
            result: vec![ActionResult::extracted("ok")],
            state,
            metadata: None,
        }
    }

    struct RecordingExecutor {
        seen: Vec<&'static str>,
    }

    #[async_trait]
    impl ActionExecutor for RecordingExecutor {
        async fn execute(&mut self, action: &BrowserAction) -> ActionResult {
            self.seen.push(action.name());
            ActionResult {
                extracted_content: Some(action.name().to_owned()),
                error: None,
                judgement: None,
                long_term_memory: None,
                include_extracted_content_only_once: false,
                include_in_memory: false,
                is_done: matches!(action, BrowserAction::Done(_)),
                success: None,
                attachments: Vec::new(),
                images: Vec::new(),
                metadata: None,
            }
        }
    }

    struct ScriptedReplayExecutor {
        seen: Vec<BrowserAction>,
        results: VecDeque<ActionResult>,
    }

    #[async_trait]
    impl ActionExecutor for ScriptedReplayExecutor {
        async fn execute(&mut self, action: &BrowserAction) -> ActionResult {
            self.seen.push(action.clone());
            self.results
                .pop_front()
                .unwrap_or_else(|| ActionResult::extracted(action.name()))
        }
    }

    fn replay_plan_item(
        step_index: usize,
        action_index: usize,
        original_action: BrowserAction,
        executed_action: BrowserAction,
    ) -> AgentHistoryReplayPlanItem {
        AgentHistoryReplayPlanItem {
            step_index,
            action_index,
            original_action: original_action.clone(),
            remapped_action: executed_action.clone(),
            rematch: ActionReplayRematch {
                action: executed_action.clone(),
                original_index: original_action.interacted_element_index(),
                rematched_index: executed_action.interacted_element_index(),
                match_result: None,
                changed: original_action != executed_action,
            },
        }
    }

    #[tokio::test]
    async fn action_sequence_stops_after_navigation() {
        let actions = vec![
            BrowserAction::Navigate(NavigateAction {
                url: "https://example.com".to_owned(),
                new_tab: false,
            }),
            BrowserAction::Click(ClickElementAction {
                index: Some(1),
                coordinate_x: None,
                coordinate_y: None,
            }),
        ];
        let mut executor = RecordingExecutor { seen: Vec::new() };

        let results = execute_action_sequence(&mut executor, &actions).await;

        assert_eq!(executor.seen, vec!["navigate"]);
        assert_eq!(results.len(), 1);
    }

    #[tokio::test]
    async fn done_is_only_executed_when_first_action() {
        let actions = vec![
            BrowserAction::Click(ClickElementAction {
                index: Some(1),
                coordinate_x: None,
                coordinate_y: None,
            }),
            BrowserAction::Done(DoneAction {
                text: "finished".to_owned(),
                success: true,
                files_to_display: vec![],
            }),
        ];
        let mut executor = RecordingExecutor { seen: Vec::new() };

        let results = execute_action_sequence(&mut executor, &actions).await;

        assert_eq!(executor.seen, vec!["click"]);
        assert_eq!(results.len(), 1);
    }

    #[tokio::test]
    async fn history_replay_execution_runs_rematched_plan_actions() {
        let original_click = BrowserAction::Click(ClickElementAction {
            index: Some(1),
            coordinate_x: None,
            coordinate_y: None,
        });
        let remapped_click = BrowserAction::Click(ClickElementAction {
            index: Some(7),
            coordinate_x: None,
            coordinate_y: None,
        });
        let wait = BrowserAction::Wait(WaitAction { seconds: 0 });
        let plan = AgentHistoryReplayPlan {
            actions: vec![
                replay_plan_item(2, 0, original_click.clone(), remapped_click.clone()),
                replay_plan_item(2, 1, wait.clone(), wait.clone()),
            ],
        };
        let mut executor = ScriptedReplayExecutor {
            seen: Vec::new(),
            results: VecDeque::from([
                ActionResult::extracted("clicked remapped element"),
                ActionResult::extracted("waited"),
            ]),
        };

        let execution = execute_history_replay_plan(&mut executor, &plan).await;

        assert_eq!(executor.seen, vec![remapped_click.clone(), wait]);
        assert_eq!(execution.items.len(), 2);
        assert_eq!(execution.items[0].step_index, 2);
        assert_eq!(execution.items[0].action_index, 0);
        assert_eq!(execution.items[0].original_action, original_click);
        assert_eq!(execution.items[0].executed_action, remapped_click);
        assert_eq!(execution.items[0].rematch.original_index, Some(1));
        assert_eq!(execution.items[0].rematch.rematched_index, Some(7));
        assert_eq!(
            execution.items[0].result.extracted_content.as_deref(),
            Some("clicked remapped element")
        );
        assert_eq!(execution.stop, None);
    }

    #[tokio::test]
    async fn history_replay_execution_stops_on_action_errors_with_coordinates() {
        let click = BrowserAction::Click(ClickElementAction {
            index: Some(1),
            coordinate_x: None,
            coordinate_y: None,
        });
        let wait = BrowserAction::Wait(WaitAction { seconds: 0 });
        let plan = AgentHistoryReplayPlan {
            actions: vec![
                replay_plan_item(4, 2, click.clone(), click),
                replay_plan_item(4, 3, wait.clone(), wait),
            ],
        };
        let mut executor = ScriptedReplayExecutor {
            seen: Vec::new(),
            results: VecDeque::from([ActionResult::error("click failed")]),
        };

        let execution = execute_history_replay_plan(&mut executor, &plan).await;

        assert_eq!(executor.seen.len(), 1);
        assert_eq!(
            execution.stop,
            Some(AgentHistoryReplayStop {
                step_index: 4,
                action_index: 2,
                reason: AgentHistoryReplayStopReason::Error,
                diagnostic: Some("click failed".to_owned()),
            })
        );
        assert_eq!(
            execution.items[0].result.error.as_deref(),
            Some("click failed")
        );
    }

    #[tokio::test]
    async fn history_replay_execution_stops_after_terminating_actions() {
        let navigate = BrowserAction::Navigate(NavigateAction {
            url: "https://example.com".to_owned(),
            new_tab: false,
        });
        let click = BrowserAction::Click(ClickElementAction {
            index: Some(1),
            coordinate_x: None,
            coordinate_y: None,
        });
        let plan = AgentHistoryReplayPlan {
            actions: vec![
                replay_plan_item(0, 0, navigate.clone(), navigate.clone()),
                replay_plan_item(0, 1, click.clone(), click),
            ],
        };
        let mut executor = ScriptedReplayExecutor {
            seen: Vec::new(),
            results: VecDeque::from([ActionResult::extracted("navigated")]),
        };

        let execution = execute_history_replay_plan(&mut executor, &plan).await;

        assert_eq!(executor.seen, vec![navigate]);
        assert_eq!(execution.items.len(), 1);
        assert_eq!(
            execution.stop,
            Some(AgentHistoryReplayStop {
                step_index: 0,
                action_index: 0,
                reason: AgentHistoryReplayStopReason::TerminatingAction,
                diagnostic: None,
            })
        );
    }

    #[tokio::test]
    async fn history_replay_execution_matches_done_sequence_rules() {
        let done = BrowserAction::Done(DoneAction {
            text: "finished".to_owned(),
            success: true,
            files_to_display: vec![],
        });
        let first_done_plan = AgentHistoryReplayPlan {
            actions: vec![replay_plan_item(9, 0, done.clone(), done.clone())],
        };
        let mut executor = ScriptedReplayExecutor {
            seen: Vec::new(),
            results: VecDeque::from([ActionResult::done("finished", true)]),
        };

        let execution = execute_history_replay_plan(&mut executor, &first_done_plan).await;

        assert_eq!(executor.seen, vec![done.clone()]);
        assert_eq!(
            execution.stop,
            Some(AgentHistoryReplayStop {
                step_index: 9,
                action_index: 0,
                reason: AgentHistoryReplayStopReason::Done,
                diagnostic: None,
            })
        );

        let wait = BrowserAction::Wait(WaitAction { seconds: 0 });
        let delayed_done_plan = AgentHistoryReplayPlan {
            actions: vec![
                replay_plan_item(9, 0, wait.clone(), wait.clone()),
                replay_plan_item(9, 1, done.clone(), done),
            ],
        };
        let mut executor = ScriptedReplayExecutor {
            seen: Vec::new(),
            results: VecDeque::from([ActionResult::extracted("waited")]),
        };

        let execution = execute_history_replay_plan(&mut executor, &delayed_done_plan).await;

        assert_eq!(executor.seen, vec![wait]);
        assert_eq!(execution.items.len(), 1);
        assert_eq!(
            execution.stop,
            Some(AgentHistoryReplayStop {
                step_index: 9,
                action_index: 1,
                reason: AgentHistoryReplayStopReason::DoneAfterPriorAction,
                diagnostic: None,
            })
        );
    }

    struct MockSession {
        events: Mutex<Vec<String>>,
        states: Mutex<VecDeque<BrowserStateSummary>>,
        state_screenshot_requests: Mutex<Vec<bool>>,
        state_error: Mutex<Option<String>>,
        click_error: Mutex<Option<String>>,
        click_delay: Mutex<Option<Duration>>,
    }

    impl MockSession {
        fn new() -> Self {
            Self {
                events: Mutex::new(Vec::new()),
                states: Mutex::new(VecDeque::new()),
                state_screenshot_requests: Mutex::new(Vec::new()),
                state_error: Mutex::new(None),
                click_error: Mutex::new(None),
                click_delay: Mutex::new(None),
            }
        }

        fn with_states(states: Vec<BrowserStateSummary>) -> Self {
            Self {
                events: Mutex::new(Vec::new()),
                states: Mutex::new(VecDeque::from(states)),
                state_screenshot_requests: Mutex::new(Vec::new()),
                state_error: Mutex::new(None),
                click_error: Mutex::new(None),
                click_delay: Mutex::new(None),
            }
        }

        fn with_state_error(error: impl Into<String>) -> Self {
            Self {
                events: Mutex::new(Vec::new()),
                states: Mutex::new(VecDeque::new()),
                state_screenshot_requests: Mutex::new(Vec::new()),
                state_error: Mutex::new(Some(error.into())),
                click_error: Mutex::new(None),
                click_delay: Mutex::new(None),
            }
        }

        fn with_click_error(error: impl Into<String>) -> Self {
            Self {
                events: Mutex::new(Vec::new()),
                states: Mutex::new(VecDeque::new()),
                state_screenshot_requests: Mutex::new(Vec::new()),
                state_error: Mutex::new(None),
                click_error: Mutex::new(Some(error.into())),
                click_delay: Mutex::new(None),
            }
        }

        fn with_click_delay(delay: Duration) -> Self {
            Self {
                events: Mutex::new(Vec::new()),
                states: Mutex::new(VecDeque::new()),
                state_screenshot_requests: Mutex::new(Vec::new()),
                state_error: Mutex::new(None),
                click_error: Mutex::new(None),
                click_delay: Mutex::new(Some(delay)),
            }
        }

        fn events(&self) -> Vec<String> {
            self.events.lock().expect("events lock").clone()
        }

        fn state_screenshot_requests(&self) -> Vec<bool> {
            self.state_screenshot_requests
                .lock()
                .expect("state requests lock")
                .clone()
        }
    }

    #[async_trait]
    impl BrowserSession for MockSession {
        async fn state(
            &self,
            include_screenshot: bool,
        ) -> Result<BrowserStateSummary, BrowserError> {
            self.state_screenshot_requests
                .lock()
                .expect("state requests lock")
                .push(include_screenshot);
            if let Some(error) = self.state_error.lock().expect("state error lock").clone() {
                return Err(BrowserError::StateUnavailable(error));
            }
            if let Some(state) = self.states.lock().expect("states lock").pop_front() {
                return Ok(state);
            }
            Ok(BrowserStateSummary {
                dom_state: SerializedDomState::default(),
                ..blank_state()
            })
        }

        async fn navigate(&self, url: &str, new_tab: bool) -> Result<(), BrowserError> {
            self.events
                .lock()
                .expect("events lock")
                .push(format!("navigate:{url}:{new_tab}"));
            Ok(())
        }

        async fn go_back(&self) -> Result<(), BrowserError> {
            self.events
                .lock()
                .expect("events lock")
                .push("go_back".to_owned());
            Ok(())
        }

        async fn switch_tab(&self, target_id: &str) -> Result<(), BrowserError> {
            self.events
                .lock()
                .expect("events lock")
                .push(format!("switch_tab:{target_id}"));
            Ok(())
        }

        async fn close_tab(&self, target_id: &str) -> Result<(), BrowserError> {
            self.events
                .lock()
                .expect("events lock")
                .push(format!("close_tab:{target_id}"));
            Ok(())
        }

        async fn click(&self, index: u32) -> Result<(), BrowserError> {
            self.events
                .lock()
                .expect("events lock")
                .push(format!("click:{index}"));
            let delay = self.click_delay.lock().expect("click delay lock").take();
            if let Some(delay) = delay {
                sleep(delay).await;
            }
            if let Some(error) = self.click_error.lock().expect("click error lock").take() {
                return Err(BrowserError::ActionFailed(error));
            }
            Ok(())
        }

        async fn click_coordinates(&self, x: i32, y: i32) -> Result<(), BrowserError> {
            self.events
                .lock()
                .expect("events lock")
                .push(format!("click_coordinates:{x}:{y}"));
            Ok(())
        }

        async fn input_text(
            &self,
            index: u32,
            text: &str,
            clear: bool,
        ) -> Result<(), BrowserError> {
            self.events
                .lock()
                .expect("events lock")
                .push(format!("input:{index}:{text}:{clear}"));
            Ok(())
        }

        async fn scroll(
            &self,
            index: Option<u32>,
            down: bool,
            pages: f64,
        ) -> Result<(), BrowserError> {
            self.events
                .lock()
                .expect("events lock")
                .push(format!("scroll:{index:?}:{down}:{pages}"));
            Ok(())
        }

        async fn find_text(&self, text: &str) -> Result<bool, BrowserError> {
            self.events
                .lock()
                .expect("events lock")
                .push(format!("find_text:{text}"));
            Ok(true)
        }

        async fn evaluate(&self, code: &str) -> Result<String, BrowserError> {
            self.events
                .lock()
                .expect("events lock")
                .push(format!("evaluate:{code}"));
            Ok("EvalOps JS result".to_owned())
        }

        async fn dropdown_options(&self, index: u32) -> Result<Vec<String>, BrowserError> {
            self.events
                .lock()
                .expect("events lock")
                .push(format!("dropdown_options:{index}"));
            Ok(vec!["One".to_owned(), "Two".to_owned()])
        }

        async fn select_dropdown_option(&self, index: u32, text: &str) -> Result<(), BrowserError> {
            self.events
                .lock()
                .expect("events lock")
                .push(format!("select_dropdown_option:{index}:{text}"));
            Ok(())
        }

        async fn page_text(&self) -> Result<String, BrowserError> {
            self.events
                .lock()
                .expect("events lock")
                .push("page_text".to_owned());
            Ok("Alpha EvalOps Beta\nSecond EvalOps line".to_owned())
        }

        async fn find_elements(
            &self,
            selector: &str,
            attributes: &[String],
            max_results: usize,
            include_text: bool,
        ) -> Result<Vec<FoundElement>, BrowserError> {
            self.events.lock().expect("events lock").push(format!(
                "find_elements:{selector}:{}:{max_results}:{include_text}",
                attributes.join("|")
            ));
            if selector == "a[href]" {
                let mut attrs = BTreeMap::new();
                attrs.insert("href".to_owned(), "https://evalops.dev/run".to_owned());
                attrs.insert("title".to_owned(), "Run EvalOps".to_owned());
                return Ok(vec![FoundElement {
                    tag_name: "a".to_owned(),
                    text: include_text.then(|| "Run EvalOps".to_owned()),
                    attributes: attrs,
                }]);
            }
            if selector == "img[src], img[data-src], picture source[srcset]" {
                let mut attrs = BTreeMap::new();
                attrs.insert("src".to_owned(), "https://evalops.dev/hero.png".to_owned());
                attrs.insert("alt".to_owned(), "Hero shot".to_owned());
                return Ok(vec![FoundElement {
                    tag_name: "img".to_owned(),
                    text: include_text.then(|| "ignored".to_owned()),
                    attributes: attrs,
                }]);
            }
            let mut attrs = BTreeMap::new();
            attrs.insert("id".to_owned(), "run".to_owned());
            Ok(vec![FoundElement {
                tag_name: "button".to_owned(),
                text: include_text.then(|| "Run EvalOps".to_owned()),
                attributes: attrs,
            }])
        }

        async fn send_keys(&self, keys: &str) -> Result<(), BrowserError> {
            self.events
                .lock()
                .expect("events lock")
                .push(format!("send_keys:{keys}"));
            Ok(())
        }

        async fn upload_file(
            &self,
            index: u32,
            path: &std::path::Path,
        ) -> Result<(), BrowserError> {
            self.events
                .lock()
                .expect("events lock")
                .push(format!("upload_file:{index}:{}", path.display()));
            Ok(())
        }

        async fn screenshot(&self) -> Result<Screenshot, BrowserError> {
            self.events
                .lock()
                .expect("events lock")
                .push("screenshot".to_owned());
            Ok(Screenshot {
                base64_png: base64::engine::general_purpose::STANDARD.encode("PNGDATA"),
            })
        }

        async fn save_pdf(
            &self,
            print_background: bool,
            landscape: bool,
            scale: f64,
            paper_format: &str,
        ) -> Result<Pdf, BrowserError> {
            self.events.lock().expect("events lock").push(format!(
                "save_pdf:{print_background}:{landscape}:{scale}:{paper_format}"
            ));
            Ok(Pdf {
                base64_pdf: base64::engine::general_purpose::STANDARD.encode("%PDF-1.7"),
            })
        }
    }

    #[tokio::test]
    async fn browser_executor_maps_navigate_to_session() {
        let session = MockSession::new();
        let mut executor = BrowserActionExecutor::new(session);

        let result = executor
            .execute(&BrowserAction::Navigate(NavigateAction {
                url: "https://example.com".to_owned(),
                new_tab: false,
            }))
            .await;

        assert!(result.error.is_none());
        assert_eq!(
            executor.session().events(),
            vec!["navigate:https://example.com:false"]
        );
    }

    #[tokio::test]
    async fn browser_executor_returns_action_error_when_action_times_out() {
        let session = MockSession::with_click_delay(Duration::from_millis(50));
        let mut executor = BrowserActionExecutor::new(session);
        executor.set_action_timeout_seconds(0.005);

        let result = executor
            .execute(&BrowserAction::Click(ClickElementAction {
                index: Some(1),
                coordinate_x: None,
                coordinate_y: None,
            }))
            .await;

        let error = result.error.expect("timeout error");
        assert!(error.contains("Action click timed out after"));
        assert!(error.contains("dead CDP WebSocket"));
    }

    #[tokio::test]
    async fn terminating_sequence_action_does_not_need_pre_state() {
        let session = MockSession::with_state_error("state unavailable");
        let mut executor = BrowserActionExecutor::new(session);

        let results = executor
            .execute_sequence(&[BrowserAction::Navigate(NavigateAction {
                url: "https://example.com".to_owned(),
                new_tab: false,
            })])
            .await;

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].error, None);
        assert_eq!(
            executor.session().events(),
            vec!["navigate:https://example.com:false"]
        );
        assert!(executor.session().state_screenshot_requests().is_empty());
    }

    #[tokio::test]
    async fn browser_executor_stops_sequence_when_url_changes() {
        let mut first_state = blank_state();
        first_state.url = "https://example.com/one".to_owned();
        let mut second_state = blank_state();
        second_state.url = "https://example.com/two".to_owned();
        let session = MockSession::with_states(vec![first_state, second_state]);
        let mut executor = BrowserActionExecutor::new(session);

        let results = executor
            .execute_sequence(&[
                BrowserAction::Click(ClickElementAction {
                    index: Some(1),
                    coordinate_x: None,
                    coordinate_y: None,
                }),
                BrowserAction::Input(InputTextAction {
                    index: 2,
                    text: "should not type".to_owned(),
                    clear: true,
                }),
            ])
            .await;

        assert_eq!(results.len(), 1);
        assert_eq!(executor.session().events(), vec!["click:1"]);
    }

    #[tokio::test]
    async fn browser_replay_execution_stops_when_url_changes() {
        let mut first_state = blank_state();
        first_state.url = "https://example.com/one".to_owned();
        let mut second_state = blank_state();
        second_state.url = "https://example.com/two".to_owned();
        let session = MockSession::with_states(vec![first_state, second_state]);
        let mut executor = BrowserActionExecutor::new(session);
        let original_click = BrowserAction::Click(ClickElementAction {
            index: Some(1),
            coordinate_x: None,
            coordinate_y: None,
        });
        let remapped_click = BrowserAction::Click(ClickElementAction {
            index: Some(7),
            coordinate_x: None,
            coordinate_y: None,
        });
        let input = BrowserAction::Input(InputTextAction {
            index: 2,
            text: "should not type".to_owned(),
            clear: true,
        });
        let plan = AgentHistoryReplayPlan {
            actions: vec![
                replay_plan_item(3, 0, original_click, remapped_click.clone()),
                replay_plan_item(3, 1, input.clone(), input),
            ],
        };

        let execution = executor.execute_replay_plan(&plan).await;

        assert_eq!(executor.session().events(), vec!["click:7"]);
        assert_eq!(
            executor.session().state_screenshot_requests(),
            vec![false, false]
        );
        assert_eq!(execution.items.len(), 1);
        assert_eq!(execution.items[0].executed_action, remapped_click);
        assert_eq!(
            execution.stop,
            Some(AgentHistoryReplayStop {
                step_index: 3,
                action_index: 0,
                reason: AgentHistoryReplayStopReason::PageChanged,
                diagnostic: None,
            })
        );
    }

    #[tokio::test]
    async fn browser_replay_execution_continues_when_guarded_url_is_stable() {
        let mut state = blank_state();
        state.url = "https://example.com/stable".to_owned();
        let session = MockSession::with_states(vec![
            state.clone(),
            state.clone(),
            state.clone(),
            state.clone(),
        ]);
        let mut executor = BrowserActionExecutor::new(session);
        let click = BrowserAction::Click(ClickElementAction {
            index: Some(1),
            coordinate_x: None,
            coordinate_y: None,
        });
        let input = BrowserAction::Input(InputTextAction {
            index: 2,
            text: "hello".to_owned(),
            clear: true,
        });
        let plan = AgentHistoryReplayPlan {
            actions: vec![
                replay_plan_item(4, 0, click.clone(), click),
                replay_plan_item(4, 1, input.clone(), input),
            ],
        };

        let execution = executor.execute_replay_plan(&plan).await;

        assert_eq!(execution.items.len(), 2);
        assert_eq!(execution.stop, None);
        assert_eq!(
            executor.session().events(),
            vec!["click:1", "input:2:hello:true"]
        );
        assert_eq!(
            executor.session().state_screenshot_requests(),
            vec![false, false, false, false]
        );
    }

    #[tokio::test]
    async fn browser_replay_execution_reports_pre_state_failure_without_action() {
        let session = MockSession::with_state_error("state unavailable");
        let mut executor = BrowserActionExecutor::new(session);
        let click = BrowserAction::Click(ClickElementAction {
            index: Some(1),
            coordinate_x: None,
            coordinate_y: None,
        });
        let plan = AgentHistoryReplayPlan {
            actions: vec![replay_plan_item(5, 0, click.clone(), click)],
        };

        let execution = executor.execute_replay_plan(&plan).await;

        assert!(executor.session().events().is_empty());
        assert_eq!(executor.session().state_screenshot_requests(), vec![false]);
        assert_eq!(execution.items.len(), 1);
        assert!(
            execution.items[0]
                .result
                .error
                .as_deref()
                .is_some_and(|error| error.contains("state unavailable"))
        );
        let stop = execution.stop.expect("state failure stop");
        assert_eq!(stop.step_index, 5);
        assert_eq!(stop.action_index, 0);
        assert_eq!(stop.reason, AgentHistoryReplayStopReason::Error);
        assert!(
            stop.diagnostic
                .as_deref()
                .is_some_and(|error| error.contains("state unavailable"))
        );
    }

    #[tokio::test]
    async fn browser_replay_execution_preserves_done_and_error_stops() {
        let session = MockSession::new();
        let mut executor = BrowserActionExecutor::new(session);
        let done = BrowserAction::Done(DoneAction {
            text: "finished".to_owned(),
            success: true,
            files_to_display: vec![],
        });
        let plan = AgentHistoryReplayPlan {
            actions: vec![replay_plan_item(7, 0, done.clone(), done.clone())],
        };

        let execution = executor.execute_replay_plan(&plan).await;

        assert_eq!(execution.items.len(), 1);
        assert_eq!(execution.items[0].executed_action, done);
        assert_eq!(
            execution.stop,
            Some(AgentHistoryReplayStop {
                step_index: 7,
                action_index: 0,
                reason: AgentHistoryReplayStopReason::Done,
                diagnostic: None,
            })
        );
        assert!(executor.session().state_screenshot_requests().is_empty());

        let mut state = blank_state();
        state.url = "https://example.com/stable".to_owned();
        let session = MockSession::with_states(vec![state]);
        *session.click_error.lock().expect("click error lock") = Some("boom".to_owned());
        let mut executor = BrowserActionExecutor::new(session);
        let click = BrowserAction::Click(ClickElementAction {
            index: Some(1),
            coordinate_x: None,
            coordinate_y: None,
        });
        let plan = AgentHistoryReplayPlan {
            actions: vec![replay_plan_item(8, 0, click.clone(), click)],
        };

        let execution = executor.execute_replay_plan(&plan).await;

        assert_eq!(execution.items.len(), 1);
        assert!(
            execution.items[0]
                .result
                .error
                .as_deref()
                .is_some_and(|error| error.contains("boom"))
        );
        assert_eq!(
            execution.stop,
            Some(AgentHistoryReplayStop {
                step_index: 8,
                action_index: 0,
                reason: AgentHistoryReplayStopReason::Error,
                diagnostic: Some("action failed: boom".to_owned()),
            })
        );
        assert_eq!(executor.session().state_screenshot_requests(), vec![false]);
    }

    #[tokio::test]
    async fn browser_replay_execution_skips_pre_state_for_terminating_actions() {
        let session = MockSession::with_state_error("state unavailable");
        let mut executor = BrowserActionExecutor::new(session);
        let navigate = BrowserAction::Navigate(NavigateAction {
            url: "https://example.com".to_owned(),
            new_tab: false,
        });
        let click = BrowserAction::Click(ClickElementAction {
            index: Some(1),
            coordinate_x: None,
            coordinate_y: None,
        });
        let plan = AgentHistoryReplayPlan {
            actions: vec![
                replay_plan_item(6, 0, navigate.clone(), navigate.clone()),
                replay_plan_item(6, 1, click.clone(), click),
            ],
        };

        let execution = executor.execute_replay_plan(&plan).await;

        assert_eq!(
            executor.session().events(),
            vec!["navigate:https://example.com:false"]
        );
        assert!(executor.session().state_screenshot_requests().is_empty());
        assert_eq!(execution.items.len(), 1);
        assert_eq!(execution.items[0].executed_action, navigate);
        assert_eq!(
            execution.stop,
            Some(AgentHistoryReplayStop {
                step_index: 6,
                action_index: 0,
                reason: AgentHistoryReplayStopReason::TerminatingAction,
                diagnostic: None,
            })
        );
    }

    #[tokio::test]
    async fn browser_replay_history_captures_current_state_and_executes_plan() {
        let mut historical_state = blank_state();
        historical_state.dom_state = SerializedDomState::from_elements(vec![replay_dom_element(
            1,
            "button",
            BTreeMap::from([("id".to_owned(), "save".to_owned())]),
        )]);
        let history = AgentHistory {
            items: vec![history_item_with_state_actions(
                historical_state,
                vec![BrowserAction::Click(ClickElementAction {
                    index: Some(1),
                    coordinate_x: None,
                    coordinate_y: None,
                })],
            )],
            ..AgentHistory::default()
        };
        let mut current_state = blank_state();
        current_state.url = "https://example.com/current".to_owned();
        current_state.dom_state = SerializedDomState::from_elements(vec![replay_dom_element(
            7,
            "button",
            BTreeMap::from([("id".to_owned(), "save".to_owned())]),
        )]);
        let mut before = blank_state();
        before.url = current_state.url.clone();
        let after = before.clone();
        let session = MockSession::with_states(vec![current_state.clone(), before, after]);
        let mut executor = BrowserActionExecutor::new(session);

        let run = executor.replay_history(&history).await.expect("replay run");

        assert_eq!(run.current_state.url, current_state.url);
        assert_eq!(run.plan.actions.len(), 1);
        assert_eq!(
            run.plan.actions[0]
                .remapped_action
                .interacted_element_index(),
            Some(7)
        );
        assert_eq!(run.execution.items.len(), 1);
        assert_eq!(
            run.execution.items[0]
                .executed_action
                .interacted_element_index(),
            Some(7)
        );
        assert_eq!(run.execution.stop, None);
        assert_eq!(executor.session().events(), vec!["click:7"]);
        assert_eq!(
            executor.session().state_screenshot_requests(),
            vec![false, false]
        );
    }

    #[tokio::test]
    async fn browser_replay_history_recaptures_dom_between_actions() {
        let mut historical_click_state = blank_state();
        historical_click_state.dom_state =
            SerializedDomState::from_elements(vec![replay_dom_element(
                1,
                "button",
                BTreeMap::from([("id".to_owned(), "reveal-email".to_owned())]),
            )]);
        let mut historical_input_state = blank_state();
        historical_input_state.dom_state =
            SerializedDomState::from_elements(vec![replay_dom_element(
                2,
                "input",
                BTreeMap::from([("name".to_owned(), "email".to_owned())]),
            )]);
        let history = AgentHistory {
            items: vec![
                history_item_with_state_actions(
                    historical_click_state,
                    vec![BrowserAction::Click(ClickElementAction {
                        index: Some(1),
                        coordinate_x: None,
                        coordinate_y: None,
                    })],
                ),
                history_item_with_state_actions(
                    historical_input_state,
                    vec![BrowserAction::Input(InputTextAction {
                        index: 2,
                        text: "ada@example.test".to_owned(),
                        clear: true,
                    })],
                ),
            ],
            ..AgentHistory::default()
        };
        let mut initial_state = blank_state();
        initial_state.url = "https://example.com/start".to_owned();
        initial_state.dom_state = SerializedDomState::from_elements(vec![replay_dom_element(
            7,
            "button",
            BTreeMap::from([("id".to_owned(), "reveal-email".to_owned())]),
        )]);
        let mut after_click_state = blank_state();
        after_click_state.url = "https://example.com/form".to_owned();
        after_click_state.dom_state = SerializedDomState::from_elements(vec![replay_dom_element(
            8,
            "input",
            BTreeMap::from([("name".to_owned(), "email".to_owned())]),
        )]);
        let after_input_state = after_click_state.clone();
        let session = MockSession::with_states(vec![
            initial_state.clone(),
            after_click_state,
            after_input_state,
        ]);
        let mut executor = BrowserActionExecutor::new(session);

        let run = executor.replay_history(&history).await.expect("replay run");

        assert_eq!(run.current_state.url, initial_state.url);
        assert_eq!(run.plan.actions.len(), 2);
        assert_eq!(
            run.plan.actions[0]
                .remapped_action
                .interacted_element_index(),
            Some(7)
        );
        assert_eq!(
            run.plan.actions[1]
                .remapped_action
                .interacted_element_index(),
            Some(8)
        );
        assert_eq!(run.execution.stop, None);
        assert_eq!(
            executor.session().events(),
            vec!["click:7", "input:8:ada@example.test:true"]
        );
        assert_eq!(
            executor.session().state_screenshot_requests(),
            vec![false, false, false]
        );
    }

    #[tokio::test]
    async fn browser_replay_history_reports_current_state_failure() {
        let session = MockSession::with_state_error("state unavailable");
        let mut executor = BrowserActionExecutor::new(session);
        let history = AgentHistory::default();

        let error = executor
            .replay_history(&history)
            .await
            .expect_err("state error");

        assert!(matches!(
            error,
            AgentHistoryReplayRunError::CurrentState { .. }
        ));
        assert_eq!(executor.session().events(), Vec::<String>::new());
        assert_eq!(executor.session().state_screenshot_requests(), vec![false]);
    }

    #[tokio::test]
    async fn browser_replay_history_reports_plan_rematch_failure() {
        let mut historical_state = blank_state();
        historical_state.dom_state = SerializedDomState::from_elements(vec![replay_dom_element(
            1,
            "button",
            BTreeMap::from([("data-testid".to_owned(), "duplicate".to_owned())]),
        )]);
        let history = AgentHistory {
            items: vec![history_item_with_state_actions(
                historical_state,
                vec![BrowserAction::Click(ClickElementAction {
                    index: Some(1),
                    coordinate_x: None,
                    coordinate_y: None,
                })],
            )],
            ..AgentHistory::default()
        };
        let mut current_state = blank_state();
        current_state.dom_state = SerializedDomState::from_elements(vec![
            replay_dom_element(
                7,
                "button",
                BTreeMap::from([("data-testid".to_owned(), "duplicate".to_owned())]),
            ),
            replay_dom_element(
                8,
                "button",
                BTreeMap::from([("data-testid".to_owned(), "duplicate".to_owned())]),
            ),
        ]);
        let session = MockSession::with_states(vec![current_state]);
        let mut executor = BrowserActionExecutor::new(session);

        let error = executor
            .replay_history(&history)
            .await
            .expect_err("plan error");

        let AgentHistoryReplayRunError::Plan { error } = error else {
            panic!("expected plan error");
        };
        assert_eq!(error.step_index, 0);
        assert_eq!(error.action_index, 0);
        assert_eq!(
            error.failure.reason,
            DomInteractedElementMatchFailureReason::Ambiguous
        );
        assert_eq!(executor.session().events(), Vec::<String>::new());
        assert_eq!(executor.session().state_screenshot_requests(), vec![false]);
    }

    #[tokio::test]
    async fn browser_executor_rejects_zero_click_index() {
        let session = MockSession::new();
        let mut executor = BrowserActionExecutor::new(session);

        let result = executor
            .execute(&BrowserAction::Click(ClickElementAction {
                index: Some(0),
                coordinate_x: None,
                coordinate_y: None,
            }))
            .await;

        assert!(
            result
                .error
                .as_deref()
                .expect("click error")
                .contains("index 0")
        );
        assert_eq!(executor.session().events(), Vec::<String>::new());
    }

    #[tokio::test]
    async fn browser_executor_click_select_returns_dropdown_options() {
        let session = MockSession::with_click_error(
            "Cannot click on <select> elements. Use get_dropdown_options and select_dropdown_option instead.",
        );
        let mut executor = BrowserActionExecutor::new(session);

        let result = executor
            .execute(&BrowserAction::Click(ClickElementAction {
                index: Some(1),
                coordinate_x: None,
                coordinate_y: None,
            }))
            .await;

        assert_eq!(
            result.extracted_content.as_deref(),
            Some("Dropdown options: One, Two")
        );
        assert_eq!(
            executor.session().events(),
            vec!["click:1", "dropdown_options:1"]
        );
    }

    #[tokio::test]
    async fn browser_executor_click_file_input_points_to_upload_file() {
        let session = MockSession::with_click_error(
            "Cannot click on file input elements. Use upload_file instead.",
        );
        let mut executor = BrowserActionExecutor::new(session);

        let result = executor
            .execute(&BrowserAction::Click(ClickElementAction {
                index: Some(2),
                coordinate_x: None,
                coordinate_y: None,
            }))
            .await;

        assert!(
            result
                .error
                .as_deref()
                .expect("file input click error")
                .contains("upload_file")
        );
        assert_eq!(executor.session().events(), vec!["click:2"]);
    }

    #[tokio::test]
    async fn browser_executor_treats_zero_scroll_index_as_page_scroll() {
        let session = MockSession::new();
        let mut executor = BrowserActionExecutor::new(session);

        let result = executor
            .execute(&BrowserAction::Scroll(ScrollAction {
                down: true,
                pages: 1.0,
                index: Some(0),
            }))
            .await;

        assert_eq!(result.error, None);
        assert_eq!(executor.session().events(), vec!["scroll:None:true:1"]);
    }

    #[tokio::test]
    async fn browser_executor_maps_tab_actions_to_session() {
        let session = MockSession::new();
        let mut executor = BrowserActionExecutor::new(session);

        let switch_result = executor
            .execute(&BrowserAction::SwitchTab(SwitchTabAction {
                tab_id: "target-2".to_owned(),
            }))
            .await;
        let close_result = executor
            .execute(&BrowserAction::CloseTab(CloseTabAction {
                tab_id: "target-1".to_owned(),
            }))
            .await;

        assert_eq!(switch_result.error, None);
        assert_eq!(close_result.error, None);
        assert_eq!(
            executor.session().events(),
            vec!["switch_tab:target-2", "close_tab:target-1"]
        );
    }

    #[tokio::test]
    async fn browser_executor_maps_go_back_to_session() {
        let session = MockSession::new();
        let mut executor = BrowserActionExecutor::new(session);

        let result = executor
            .execute(&BrowserAction::GoBack(NoParamsAction { description: None }))
            .await;

        assert_eq!(result.error, None);
        assert_eq!(result.extracted_content.as_deref(), Some("Navigated back"));
        assert_eq!(executor.session().events(), vec!["go_back"]);
    }

    #[tokio::test]
    async fn browser_executor_maps_find_text_to_session() {
        let session = MockSession::new();
        let mut executor = BrowserActionExecutor::new(session);

        let result = executor
            .execute(&BrowserAction::FindText(FindTextAction {
                text: "Needle".to_owned(),
            }))
            .await;

        assert_eq!(result.error, None);
        assert_eq!(
            result.extracted_content.as_deref(),
            Some("Scrolled to text: Needle")
        );
        assert_eq!(executor.session().events(), vec!["find_text:Needle"]);
    }

    #[tokio::test]
    async fn browser_executor_maps_evaluate_to_session() {
        let session = MockSession::new();
        let mut executor = BrowserActionExecutor::new(session);

        let result = executor
            .execute(&BrowserAction::Evaluate(EvaluateAction {
                code: "document.title".to_owned(),
            }))
            .await;

        assert_eq!(result.error, None);
        assert_eq!(
            result.extracted_content.as_deref(),
            Some("EvalOps JS result")
        );
        assert_eq!(executor.session().events(), vec!["evaluate:document.title"]);
    }

    #[tokio::test]
    async fn browser_executor_handles_text_file_actions() {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let path = temp_dir.path().join("notes.md");
        let file_name = path.display().to_string();
        let session = MockSession::new();
        let mut executor = BrowserActionExecutor::new(session);

        let write_result = executor
            .execute(&BrowserAction::WriteFile(WriteFileAction {
                file_name: file_name.clone(),
                content: "hello world".to_owned(),
                append: false,
                trailing_newline: true,
                leading_newline: false,
            }))
            .await;
        let append_result = executor
            .execute(&BrowserAction::WriteFile(WriteFileAction {
                file_name: file_name.clone(),
                content: "world".to_owned(),
                append: true,
                trailing_newline: false,
                leading_newline: true,
            }))
            .await;
        let replace_result = executor
            .execute(&BrowserAction::ReplaceFile(ReplaceFileAction {
                file_name: file_name.clone(),
                old_str: "world".to_owned(),
                new_str: "EvalOps".to_owned(),
            }))
            .await;
        let read_result = executor
            .execute(&BrowserAction::ReadFile(ReadFileAction {
                file_name: file_name.clone(),
            }))
            .await;

        assert_eq!(write_result.error, None);
        assert_eq!(append_result.error, None);
        assert_eq!(replace_result.error, None);
        assert_eq!(
            std::fs::read_to_string(&path).expect("file content"),
            "hello EvalOps\n\nEvalOps"
        );
        assert!(
            read_result
                .extracted_content
                .as_deref()
                .expect("read content")
                .contains("hello EvalOps\n\nEvalOps")
        );
        assert!(read_result.include_extracted_content_only_once);
    }

    #[test]
    fn relative_file_action_paths_resolve_like_upstream_filesystem() {
        let resolved =
            resolve_file_action_path("../nested/test@file.MD", supported_text_extensions());

        assert_eq!(resolved.path, std::path::PathBuf::from("testfile.md"));
        assert_eq!(resolved.display_name, "testfile.md");
        assert!(resolved.was_corrected);

        let spaced = resolve_file_action_path("report (1).csv", supported_text_extensions());
        assert_eq!(spaced.path, std::path::PathBuf::from("report (1).csv"));
        assert!(!spaced.was_corrected);

        let absolute =
            resolve_file_action_path("/tmp/evalops/test@file.md", supported_text_extensions());
        assert_eq!(
            absolute.path,
            std::path::PathBuf::from("/tmp/evalops/test@file.md")
        );
        assert!(!absolute.was_corrected);
    }

    #[test]
    fn browser_executor_sanitizes_relative_file_actions_like_upstream() {
        let _lock = CWD_LOCK.lock().expect("cwd lock");
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let _cwd = CurrentDirGuard::enter(temp_dir.path());

        let write_result = write_file_action(&WriteFileAction {
            file_name: "nested/test@file.md".to_owned(),
            content: "old text".to_owned(),
            append: false,
            trailing_newline: false,
            leading_newline: false,
        })
        .expect("write result");
        let append_result = write_file_action(&WriteFileAction {
            file_name: "nested/test@file.md".to_owned(),
            content: "\nmore text".to_owned(),
            append: true,
            trailing_newline: false,
            leading_newline: false,
        })
        .expect("append result");
        let replace_result =
            replace_file_action("nested/test@file.md", "old", "new").expect("replace result");
        let read_result = read_file_action("nested/test@file.md").expect("read result");

        assert_eq!(
            write_result.extracted_content.as_deref(),
            Some("Wrote file testfile.md (auto-corrected from 'nested/test@file.md')")
        );
        assert_eq!(
            append_result.extracted_content.as_deref(),
            Some("Appended to file testfile.md (auto-corrected from 'nested/test@file.md')")
        );
        assert_eq!(
            replace_result.extracted_content.as_deref(),
            Some("Replaced text in file testfile.md (auto-corrected from 'nested/test@file.md')")
        );
        assert_eq!(
            std::fs::read_to_string(temp_dir.path().join("testfile.md")).expect("sanitized file"),
            "new text\nmore text"
        );
        assert!(!temp_dir.path().join("nested").exists());
        let read_content = read_result
            .extracted_content
            .as_deref()
            .expect("read content");
        assert!(read_content.contains(
            "Note: filename was auto-corrected from 'nested/test@file.md' to 'testfile.md'."
        ));
        assert!(read_content.contains("new text\nmore text"));
    }

    #[test]
    fn managed_file_system_initializes_default_sandbox() {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let data_dir = temp_dir.path().join(DEFAULT_FILE_SYSTEM_PATH);
        std::fs::create_dir_all(&data_dir).expect("seed data dir");
        std::fs::write(data_dir.join("stale.md"), "stale").expect("seed stale file");

        let file_system = ManagedFileSystem::new(temp_dir.path()).expect("managed file system");

        assert_eq!(file_system.base_dir(), temp_dir.path());
        assert_eq!(file_system.data_dir(), data_dir.as_path());
        assert_eq!(file_system.list_files(), vec!["todo.md"]);
        assert_eq!(file_system.get_todo_contents(), "");
        assert!(data_dir.join("todo.md").exists());
        assert!(!data_dir.join("stale.md").exists());
    }

    #[test]
    fn managed_file_system_round_trips_state_and_artifacts() {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let mut file_system = ManagedFileSystem::new(temp_dir.path()).expect("managed file system");
        let original = "nested/report@ data.CSV";
        let resolved = "report-data.csv";

        let write_result = file_system
            .write_file(&WriteFileAction {
                file_name: original.to_owned(),
                content: "name,value\nalice,one".to_owned(),
                append: false,
                trailing_newline: false,
                leading_newline: false,
            })
            .expect("managed write");
        assert_eq!(write_result.error, None);
        assert_eq!(
            write_result.extracted_content.as_deref(),
            Some("Wrote file report-data.csv (auto-corrected from 'nested/report@ data.CSV')")
        );

        let append_result = file_system
            .write_file(&WriteFileAction {
                file_name: original.to_owned(),
                content: "\nbob,two".to_owned(),
                append: true,
                trailing_newline: false,
                leading_newline: false,
            })
            .expect("managed append");
        assert_eq!(append_result.error, None);

        let replace_result = file_system
            .replace_file(original, "alice", "ada")
            .expect("managed replace");
        assert_eq!(replace_result.error, None);

        let read_result = file_system.read_file(original).expect("managed read");
        let read_content = read_result.extracted_content.as_deref().expect("read");
        assert!(read_content.contains(
            "Note: filename was auto-corrected from 'nested/report@ data.CSV' to 'report-data.csv'."
        ));
        assert!(read_content.contains("ada,one"));
        assert!(read_content.contains("bob,two"));

        let data_path = temp_dir
            .path()
            .join(DEFAULT_FILE_SYSTEM_PATH)
            .join(resolved);
        assert_eq!(
            std::fs::read_to_string(&data_path).expect("managed csv"),
            "name,value\nada,one\nbob,two"
        );
        assert_eq!(
            file_system.display_file(original).as_deref(),
            Some("name,value\nada,one\nbob,two")
        );
        let description = file_system.describe();
        assert!(description.contains("<file>\nreport-data.csv - 3 lines"));
        assert!(!description.contains("todo.md"));

        let extracted_0 = file_system
            .save_extracted_content("first extracted")
            .expect("first extracted");
        let extracted_1 = file_system
            .save_extracted_content("second extracted")
            .expect("second extracted");
        assert_eq!(extracted_0, "extracted_content_0.md");
        assert_eq!(extracted_1, "extracted_content_1.md");
        assert_eq!(file_system.get_state().extracted_content_count, 2);

        let state = file_system.get_state();
        let mut restored = ManagedFileSystem::from_state(state).expect("restore file system");
        assert_eq!(restored.get_state().extracted_content_count, 2);
        assert_eq!(
            restored
                .display_file("report-data.csv")
                .expect("restored report"),
            "name,value\nada,one\nbob,two"
        );
        assert_eq!(
            std::fs::read_to_string(
                temp_dir
                    .path()
                    .join(DEFAULT_FILE_SYSTEM_PATH)
                    .join("extracted_content_1.md")
            )
            .expect("restored extracted content"),
            "second extracted"
        );
        let extracted_2 = restored
            .save_extracted_content("third extracted")
            .expect("third extracted");
        assert_eq!(extracted_2, "extracted_content_2.md");

        restored.nuke().expect("nuke file system");
        assert!(restored.list_files().is_empty());
        assert!(!restored.data_dir().exists());
    }

    #[test]
    fn managed_file_system_syncs_pdf_and_docx_artifacts() {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let mut file_system = ManagedFileSystem::new(temp_dir.path()).expect("managed file system");

        file_system
            .write_file(&WriteFileAction {
                file_name: "report.pdf".to_owned(),
                content: "# Report\nPDF body".to_owned(),
                append: false,
                trailing_newline: false,
                leading_newline: false,
            })
            .expect("pdf write");
        file_system
            .write_file(&WriteFileAction {
                file_name: "brief.docx".to_owned(),
                content: "DOCX body".to_owned(),
                append: false,
                trailing_newline: false,
                leading_newline: false,
            })
            .expect("docx write");

        let pdf_path = file_system.data_dir().join("report.pdf");
        let docx_path = file_system.data_dir().join("brief.docx");
        assert!(
            std::fs::read(&pdf_path)
                .expect("pdf bytes")
                .starts_with(b"%PDF-1.4")
        );
        assert!(zip::ZipArchive::new(std::fs::File::open(docx_path).expect("docx file")).is_ok());
        assert_eq!(
            file_system
                .get_state()
                .files
                .get("report.pdf")
                .expect("pdf state")
                .data
                .content,
            "# Report\nPDF body"
        );
    }

    #[tokio::test]
    async fn browser_executor_routes_relative_file_actions_through_managed_sandbox() {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let unique = format!(
            "executor-sandbox-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system time")
                .as_nanos()
        );
        let original = format!("nested/{unique}@ report.MD");
        let resolved = format!("{unique}-report.md");
        let cwd_candidate = std::env::current_dir()
            .expect("current dir")
            .join(&resolved);
        assert!(!cwd_candidate.exists());

        let session = MockSession::new();
        let file_system = ManagedFileSystem::new(temp_dir.path()).expect("managed file system");
        let mut executor = BrowserActionExecutor::with_file_system(session, file_system);

        let write_result = executor
            .execute(&BrowserAction::WriteFile(WriteFileAction {
                file_name: original.clone(),
                content: "alpha".to_owned(),
                append: false,
                trailing_newline: false,
                leading_newline: false,
            }))
            .await;
        let append_result = executor
            .execute(&BrowserAction::WriteFile(WriteFileAction {
                file_name: original.clone(),
                content: "\nbeta".to_owned(),
                append: true,
                trailing_newline: false,
                leading_newline: false,
            }))
            .await;
        let replace_result = executor
            .execute(&BrowserAction::ReplaceFile(ReplaceFileAction {
                file_name: original.clone(),
                old_str: "alpha".to_owned(),
                new_str: "gamma".to_owned(),
            }))
            .await;
        let read_result = executor
            .execute(&BrowserAction::ReadFile(ReadFileAction {
                file_name: original.clone(),
            }))
            .await;
        let done_result = executor
            .execute(&BrowserAction::Done(DoneAction {
                text: "Done".to_owned(),
                success: true,
                files_to_display: vec![original.clone()],
            }))
            .await;

        assert_eq!(write_result.error, None);
        assert_eq!(append_result.error, None);
        assert_eq!(replace_result.error, None);
        assert_eq!(read_result.error, None);
        assert_eq!(done_result.error, None);

        let sandbox_file = executor.file_system().data_dir().join(&resolved);
        assert_eq!(
            std::fs::read_to_string(&sandbox_file).expect("sandbox file"),
            "gamma\nbeta"
        );
        assert!(!cwd_candidate.exists());

        let read_content = read_result.extracted_content.as_deref().expect("read");
        assert!(read_content.contains(&format!(
            "Note: filename was auto-corrected from '{original}' to '{resolved}'."
        )));
        assert!(read_content.contains("gamma\nbeta"));

        let done_content = done_result.extracted_content.as_deref().expect("done");
        assert!(done_content.contains("Done\n\nAttachments:"));
        assert!(done_content.contains(&format!("{resolved}:\ngamma\nbeta")));
        assert_eq!(
            done_result.attachments,
            vec![
                std::fs::canonicalize(sandbox_file)
                    .expect("canonical sandbox file")
                    .display()
                    .to_string()
            ]
        );
    }

    #[tokio::test]
    async fn browser_executor_preserves_absolute_file_action_paths() {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let file_system_dir = temp_dir.path().join("agent");
        let external_file = temp_dir.path().join("external.md");
        let external_file_name = external_file.display().to_string();
        let session = MockSession::new();
        let file_system = ManagedFileSystem::new(&file_system_dir).expect("managed file system");
        let mut executor = BrowserActionExecutor::with_file_system(session, file_system);

        let write_result = executor
            .execute(&BrowserAction::WriteFile(WriteFileAction {
                file_name: external_file_name.clone(),
                content: "external".to_owned(),
                append: false,
                trailing_newline: false,
                leading_newline: false,
            }))
            .await;
        let read_result = executor
            .execute(&BrowserAction::ReadFile(ReadFileAction {
                file_name: external_file_name.clone(),
            }))
            .await;

        assert_eq!(write_result.error, None);
        assert_eq!(read_result.error, None);
        assert_eq!(
            std::fs::read_to_string(&external_file).expect("external file"),
            "external"
        );
        assert!(
            !executor
                .file_system()
                .data_dir()
                .join("external.md")
                .exists()
        );
        assert!(
            read_result
                .extracted_content
                .as_deref()
                .expect("absolute read")
                .contains("external")
        );
    }

    #[tokio::test]
    async fn browser_executor_append_file_requires_existing_file_like_upstream() {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let text_path = temp_dir.path().join("missing.md");
        let csv_path = temp_dir.path().join("missing.csv");
        let pdf_path = temp_dir.path().join("missing.pdf");
        let docx_path = temp_dir.path().join("missing.docx");
        let session = MockSession::new();
        let mut executor = BrowserActionExecutor::new(session);

        let text_result = executor
            .execute(&BrowserAction::WriteFile(WriteFileAction {
                file_name: text_path.display().to_string(),
                content: "hello".to_owned(),
                append: true,
                trailing_newline: true,
                leading_newline: false,
            }))
            .await;
        let csv_result = executor
            .execute(&BrowserAction::WriteFile(WriteFileAction {
                file_name: csv_path.display().to_string(),
                content: "name,value".to_owned(),
                append: true,
                trailing_newline: true,
                leading_newline: false,
            }))
            .await;
        let pdf_result = executor
            .execute(&BrowserAction::WriteFile(WriteFileAction {
                file_name: pdf_path.display().to_string(),
                content: "pdf".to_owned(),
                append: true,
                trailing_newline: true,
                leading_newline: false,
            }))
            .await;
        let docx_result = executor
            .execute(&BrowserAction::WriteFile(WriteFileAction {
                file_name: docx_path.display().to_string(),
                content: "docx".to_owned(),
                append: true,
                trailing_newline: true,
                leading_newline: false,
            }))
            .await;

        let expected_text_error = format!("File '{}' not found.", text_path.display());
        let expected_csv_error = format!("File '{}' not found.", csv_path.display());
        let expected_pdf_error = format!("File '{}' not found.", pdf_path.display());
        let expected_docx_error = format!("File '{}' not found.", docx_path.display());
        assert_eq!(
            text_result.error.as_deref(),
            Some(expected_text_error.as_str())
        );
        assert_eq!(
            csv_result.error.as_deref(),
            Some(expected_csv_error.as_str())
        );
        assert_eq!(
            pdf_result.error.as_deref(),
            Some(expected_pdf_error.as_str())
        );
        assert_eq!(
            docx_result.error.as_deref(),
            Some(expected_docx_error.as_str())
        );
        assert!(!text_path.exists());
        assert!(!csv_path.exists());
        assert!(!pdf_path.exists());
        assert!(!docx_path.exists());
    }

    #[tokio::test]
    async fn browser_executor_normalizes_csv_file_actions_like_upstream() {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let path = temp_dir.path().join("contacts.csv");
        let file_name = path.display().to_string();
        let session = MockSession::new();
        let mut executor = BrowserActionExecutor::new(session);

        let write_result = executor
            .execute(&BrowserAction::WriteFile(WriteFileAction {
                file_name: file_name.clone(),
                content: r#"name,notes\nAda,\"likes, commas\""#.to_owned(),
                append: false,
                trailing_newline: true,
                leading_newline: false,
            }))
            .await;
        let append_result = executor
            .execute(&BrowserAction::WriteFile(WriteFileAction {
                file_name: file_name.clone(),
                content: r#"Grace,"ships, tests""#.to_owned(),
                append: true,
                trailing_newline: true,
                leading_newline: false,
            }))
            .await;
        let read_result = executor
            .execute(&BrowserAction::ReadFile(ReadFileAction {
                file_name: file_name.clone(),
            }))
            .await;

        assert_eq!(write_result.error, None);
        assert_eq!(append_result.error, None);
        assert_eq!(
            std::fs::read_to_string(&path).expect("csv content"),
            "name,notes\nAda,\"likes, commas\"\nGrace,\"ships, tests\""
        );
        assert!(
            read_result
                .extracted_content
                .as_deref()
                .expect("read content")
                .contains("Ada,\"likes, commas\"\nGrace,\"ships, tests\"")
        );
    }

    #[tokio::test]
    async fn browser_executor_reads_image_files_as_one_time_image_payloads() {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let path = temp_dir.path().join("chart.png");
        std::fs::write(&path, b"PNGDATA").expect("seed image");
        let file_name = path.display().to_string();
        let session = MockSession::new();
        let mut executor = BrowserActionExecutor::new(session);

        let result = executor
            .execute(&BrowserAction::ReadFile(ReadFileAction {
                file_name: file_name.clone(),
            }))
            .await;

        assert_eq!(result.error, None);
        assert_eq!(
            result.extracted_content.as_deref(),
            Some(format!("Read image file {file_name}.").as_str())
        );
        assert_eq!(
            result.long_term_memory.as_deref(),
            Some(format!("Read image file {file_name}").as_str())
        );
        assert!(result.include_extracted_content_only_once);
        assert_eq!(result.images.len(), 1);
        assert_eq!(result.images[0]["name"], "chart.png");
        assert_eq!(
            result.images[0]["data"],
            base64::engine::general_purpose::STANDARD.encode("PNGDATA")
        );
    }

    #[tokio::test]
    async fn browser_executor_reads_docx_files_as_text() {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let path = temp_dir.path().join("report.docx");
        write_minimal_docx(
            &path,
            r#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body><w:p><w:r><w:t>Alpha</w:t></w:r></w:p><w:p><w:r><w:t>Beta</w:t></w:r></w:p></w:body></w:document>"#,
        );
        let file_name = path.display().to_string();
        let session = MockSession::new();
        let mut executor = BrowserActionExecutor::new(session);

        let result = executor
            .execute(&BrowserAction::ReadFile(ReadFileAction {
                file_name: file_name.clone(),
            }))
            .await;

        assert_eq!(result.error, None);
        let content = result.extracted_content.as_deref().expect("docx content");
        assert!(content.contains(&format!("Read from file {file_name}.")));
        assert!(content.contains("<content>\nAlpha\nBeta\n</content>"));
        assert_eq!(result.long_term_memory.as_deref(), Some("Alpha\nBeta"));
        assert!(result.include_extracted_content_only_once);
    }

    #[tokio::test]
    async fn browser_executor_writes_and_appends_docx_files_as_text() {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let path = temp_dir.path().join("report.docx");
        let file_name = path.display().to_string();
        let session = MockSession::new();
        let mut executor = BrowserActionExecutor::new(session);

        let write_result = executor
            .execute(&BrowserAction::WriteFile(WriteFileAction {
                file_name: file_name.clone(),
                content: "Title <One>\nAlpha\tBeta".to_owned(),
                append: false,
                trailing_newline: false,
                leading_newline: false,
            }))
            .await;
        let append_result = executor
            .execute(&BrowserAction::WriteFile(WriteFileAction {
                file_name: file_name.clone(),
                content: "Gamma & Delta".to_owned(),
                append: true,
                trailing_newline: false,
                leading_newline: false,
            }))
            .await;
        let read_result = executor
            .execute(&BrowserAction::ReadFile(ReadFileAction {
                file_name: file_name.clone(),
            }))
            .await;

        assert_eq!(write_result.error, None);
        assert_eq!(append_result.error, None);
        assert_eq!(
            read_docx_text(&file_name).expect("docx text"),
            "Title <One>\nAlpha\tBeta\nGamma & Delta"
        );
        let bytes = std::fs::read(&path).expect("docx bytes");
        assert_eq!(&bytes[..2], b"PK");
        let content = read_result.extracted_content.as_deref().expect("docx read");
        assert!(content.contains(&format!("Read from file {file_name}.")));
        assert!(content.contains("Title <One>\nAlpha\tBeta\nGamma & Delta"));
    }

    #[tokio::test]
    async fn browser_executor_reads_pdf_files_as_text() {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let path = temp_dir.path().join("report.pdf");
        std::fs::write(&path, minimal_pdf("Hello PDF EvalOps")).expect("seed pdf");
        let file_name = path.display().to_string();
        let session = MockSession::new();
        let mut executor = BrowserActionExecutor::new(session);

        let result = executor
            .execute(&BrowserAction::ReadFile(ReadFileAction {
                file_name: file_name.clone(),
            }))
            .await;

        assert_eq!(result.error, None);
        let content = result.extracted_content.as_deref().expect("pdf content");
        assert!(content.contains(&format!("Read from file {file_name} (1 pages,")));
        assert!(content.contains("<content>\n--- Page 1 ---"));
        assert!(content.contains("Hello PDF EvalOps"));
        assert!(
            result
                .long_term_memory
                .as_deref()
                .expect("pdf memory")
                .contains("Hello PDF EvalOps")
        );
        assert!(result.include_extracted_content_only_once);
    }

    #[tokio::test]
    async fn browser_executor_writes_and_appends_pdf_files_as_text() {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let path = temp_dir.path().join("report.pdf");
        let file_name = path.display().to_string();
        let session = MockSession::new();
        let mut executor = BrowserActionExecutor::new(session);

        let write_result = executor
            .execute(&BrowserAction::WriteFile(WriteFileAction {
                file_name: file_name.clone(),
                content: "# Report\nAlpha\tBeta (Gamma)".to_owned(),
                append: false,
                trailing_newline: false,
                leading_newline: false,
            }))
            .await;
        let append_result = executor
            .execute(&BrowserAction::WriteFile(WriteFileAction {
                file_name: file_name.clone(),
                content: r"Second \ line".to_owned(),
                append: true,
                trailing_newline: false,
                leading_newline: false,
            }))
            .await;
        let read_result = executor
            .execute(&BrowserAction::ReadFile(ReadFileAction {
                file_name: file_name.clone(),
            }))
            .await;

        assert_eq!(write_result.error, None);
        assert_eq!(append_result.error, None);
        let bytes = std::fs::read(&path).expect("pdf bytes");
        assert!(bytes.starts_with(b"%PDF-1.4"));
        let extracted = pdf_extract::extract_text(&path).expect("pdf text");
        assert!(extracted.contains("Report"));
        assert!(!extracted.contains("# Report"));
        assert!(extracted.contains("Alpha"));
        assert!(extracted.contains("Beta (Gamma)"));
        assert!(extracted.contains(r"Second \ line"));
        let content = read_result.extracted_content.as_deref().expect("pdf read");
        assert!(content.contains(&format!("Read from file {file_name} (1 pages,")));
        assert!(content.contains("--- Page 1 ---"));
        assert!(content.contains("Report"));
        assert!(content.contains("Beta (Gamma)"));
        assert!(content.contains(r"Second \ line"));
    }

    #[test]
    fn pdf_document_bytes_paginates_long_content() {
        let content = (0..80)
            .map(|index| format!("Line {index}: PDF pagination parity"))
            .collect::<Vec<_>>()
            .join("\n");
        let bytes = pdf_document_bytes(&content);
        let pdf = String::from_utf8_lossy(&bytes);

        assert!(bytes.starts_with(b"%PDF-1.4"));
        assert!(pdf.matches("/Type /Page /Parent").count() > 1);
    }

    #[test]
    fn pdf_read_envelope_marks_pages_for_small_documents() {
        let pages = vec![
            "Cover text".to_owned(),
            String::new(),
            "Appendix text".to_owned(),
        ];
        let envelope = render_pdf_read_envelope("report.pdf", &pages);

        assert!(envelope.contains("Read from file report.pdf (3 pages, 23 chars)."));
        assert!(envelope.contains("--- Page 1 ---\nCover text"));
        assert!(!envelope.contains("--- Page 2 ---"));
        assert!(envelope.contains("--- Page 3 ---\nAppendix text"));
    }

    #[test]
    fn pdf_read_envelope_prioritizes_and_truncates_large_documents() {
        let pages = (1..=20)
            .map(|page_number| {
                let distinctive = if page_number == 17 {
                    "needleword uniqueword "
                } else {
                    ""
                };
                format!(
                    "Page {page_number} {distinctive}{}",
                    "commonword ".repeat(700)
                )
            })
            .collect::<Vec<_>>();

        let envelope = render_pdf_read_envelope("large.pdf", &pages);

        assert!(envelope.contains("Read from file large.pdf (20 pages,"));
        assert!(envelope.contains("chars total)."));
        assert!(envelope.contains("--- Page 1 ---"));
        assert!(envelope.contains("--- Page 17 ---"));
        assert!(envelope.contains("[Showing "));
        assert!(envelope.contains("Skipped pages: ["));
        assert!(envelope.chars().count() < PDF_READ_MAX_CHARS + 500);
    }

    fn minimal_pdf(text: &str) -> Vec<u8> {
        let escaped = text
            .replace('\\', r"\\")
            .replace('(', r"\(")
            .replace(')', r"\)");
        let stream = format!("BT /F1 24 Tf 72 720 Td ({escaped}) Tj ET");
        let objects = [
            "<< /Type /Catalog /Pages 2 0 R >>".to_owned(),
            "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_owned(),
            "<< /Type /Page /Parent 2 0 R /Resources << /Font << /F1 4 0 R >> >> /MediaBox [0 0 612 792] /Contents 5 0 R >>".to_owned(),
            "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_owned(),
            format!(
                "<< /Length {} >>\nstream\n{}\nendstream",
                stream.len(),
                stream
            ),
        ];

        let mut pdf = b"%PDF-1.4\n".to_vec();
        let mut offsets = Vec::new();
        for (index, object) in objects.iter().enumerate() {
            offsets.push(pdf.len());
            pdf.extend_from_slice(format!("{} 0 obj\n{}\nendobj\n", index + 1, object).as_bytes());
        }
        let xref_offset = pdf.len();
        pdf.extend_from_slice(format!("xref\n0 {}\n", objects.len() + 1).as_bytes());
        pdf.extend_from_slice(b"0000000000 65535 f \n");
        for offset in offsets {
            pdf.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
        }
        pdf.extend_from_slice(
            format!(
                "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{}\n%%EOF\n",
                objects.len() + 1,
                xref_offset
            )
            .as_bytes(),
        );
        pdf
    }

    #[tokio::test]
    async fn browser_executor_done_displays_requested_text_files() {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let path = temp_dir.path().join("report.md");
        std::fs::write(&path, "alpha\nbeta").expect("seed report");
        let file_name = path.display().to_string();
        let missing_file = temp_dir.path().join("missing.md").display().to_string();
        let session = MockSession::new();
        let mut executor = BrowserActionExecutor::new(session);

        let result = executor
            .execute(&BrowserAction::Done(DoneAction {
                text: "Complete".to_owned(),
                success: true,
                files_to_display: vec![file_name.clone(), missing_file],
            }))
            .await;

        assert_eq!(result.error, None);
        assert!(result.is_done);
        assert_eq!(result.success, Some(true));
        let content = result.extracted_content.as_deref().expect("done content");
        assert!(content.starts_with("Complete\n\nAttachments:"));
        assert!(content.contains(&format!("{file_name}:\nalpha\nbeta")));
        assert_eq!(
            result.attachments,
            vec![
                std::fs::canonicalize(&path)
                    .expect("canonical report path")
                    .display()
                    .to_string()
            ]
        );
    }

    #[tokio::test]
    async fn browser_executor_done_can_attach_without_displaying_file_text() {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let path = temp_dir.path().join("report.md");
        std::fs::write(&path, "alpha\nbeta").expect("seed report");
        let file_name = path.display().to_string();
        let session = MockSession::new();
        let mut executor = BrowserActionExecutor::new(session);
        executor.set_display_files_in_done_text(false);

        let result = executor
            .execute(&BrowserAction::Done(DoneAction {
                text: "Complete".to_owned(),
                success: true,
                files_to_display: vec![file_name],
            }))
            .await;

        assert_eq!(result.error, None);
        assert!(result.is_done);
        assert_eq!(result.extracted_content.as_deref(), Some("Complete"));
        assert_eq!(
            result.attachments,
            vec![
                std::fs::canonicalize(&path)
                    .expect("canonical report path")
                    .display()
                    .to_string()
            ]
        );
    }

    fn write_minimal_docx(path: &std::path::Path, document_xml: &str) {
        let file = std::fs::File::create(path).expect("docx file");
        let mut archive = zip::ZipWriter::new(file);
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);

        archive
            .start_file("[Content_Types].xml", options)
            .expect("content types entry");
        archive
            .write_all(
                br#"<?xml version="1.0" encoding="UTF-8"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="xml" ContentType="application/xml"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/></Types>"#,
            )
            .expect("content types xml");
        archive
            .start_file("word/document.xml", options)
            .expect("document entry");
        archive
            .write_all(document_xml.as_bytes())
            .expect("document xml");
        archive.finish().expect("finish docx");
    }

    #[tokio::test]
    async fn browser_executor_rejects_unsupported_text_file_actions() {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let session = MockSession::new();
        let mut executor = BrowserActionExecutor::new(session);

        let binary_result = executor
            .execute(&BrowserAction::WriteFile(WriteFileAction {
                file_name: temp_dir.path().join("image.png").display().to_string(),
                content: "not really an image".to_owned(),
                append: false,
                trailing_newline: true,
                leading_newline: false,
            }))
            .await;
        let svg_write_result = executor
            .execute(&BrowserAction::WriteFile(WriteFileAction {
                file_name: temp_dir.path().join("diagram.svg").display().to_string(),
                content: "<svg />".to_owned(),
                append: false,
                trailing_newline: true,
                leading_newline: false,
            }))
            .await;
        let audio_read_result = executor
            .execute(&BrowserAction::ReadFile(ReadFileAction {
                file_name: temp_dir.path().join("clip.mp3").display().to_string(),
            }))
            .await;
        let extensionless_result = executor
            .execute(&BrowserAction::ReadFile(ReadFileAction {
                file_name: temp_dir.path().join("notes").display().to_string(),
            }))
            .await;

        let editable_path = temp_dir.path().join("notes.md");
        std::fs::write(&editable_path, "hello").expect("seed editable file");
        let empty_replace_result = executor
            .execute(&BrowserAction::ReplaceFile(ReplaceFileAction {
                file_name: editable_path.display().to_string(),
                old_str: String::new(),
                new_str: "EvalOps".to_owned(),
            }))
            .await;
        let dll_replace_result = executor
            .execute(&BrowserAction::ReplaceFile(ReplaceFileAction {
                file_name: temp_dir.path().join("plugin.dll").display().to_string(),
                old_str: "old".to_owned(),
                new_str: "new".to_owned(),
            }))
            .await;

        assert!(
            binary_result
                .error
                .as_deref()
                .expect("binary error")
                .contains("binary/image file")
        );
        assert!(
            svg_write_result
                .error
                .as_deref()
                .expect("svg binary error")
                .contains("binary/image file")
        );
        assert!(
            audio_read_result
                .error
                .as_deref()
                .expect("mp3 binary error")
                .contains("binary/image file")
        );
        assert!(
            extensionless_result
                .error
                .as_deref()
                .expect("extension error")
                .contains("has no extension")
        );
        assert!(
            empty_replace_result
                .error
                .as_deref()
                .expect("empty replace error")
                .contains("Cannot replace empty string")
        );
        assert!(
            dll_replace_result
                .error
                .as_deref()
                .expect("dll binary error")
                .contains("binary/image file")
        );
        assert!(!temp_dir.path().join("diagram.svg").exists());
        assert_eq!(
            std::fs::read_to_string(editable_path).expect("editable content"),
            "hello"
        );
    }

    #[tokio::test]
    async fn browser_executor_maps_dropdown_actions_to_session() {
        let session = MockSession::new();
        let mut executor = BrowserActionExecutor::new(session);

        let options_result = executor
            .execute(&BrowserAction::GetDropdownOptions(
                GetDropdownOptionsAction { index: 1 },
            ))
            .await;
        let select_result = executor
            .execute(&BrowserAction::SelectDropdownOption(
                SelectDropdownOptionAction {
                    index: 1,
                    text: "Two".to_owned(),
                },
            ))
            .await;

        assert!(
            options_result
                .extracted_content
                .expect("options content")
                .contains("One, Two")
        );
        assert!(select_result.error.is_none());
        assert_eq!(
            executor.session().events(),
            vec!["dropdown_options:1", "select_dropdown_option:1:Two"]
        );
    }

    #[tokio::test]
    async fn browser_executor_maps_send_keys_to_session() {
        let session = MockSession::new();
        let mut executor = BrowserActionExecutor::new(session);

        let result = executor
            .execute(&BrowserAction::SendKeys(SendKeysAction {
                keys: "EvalOps".to_owned(),
            }))
            .await;

        assert_eq!(result.error, None);
        assert_eq!(executor.session().events(), vec!["send_keys:EvalOps"]);
    }

    #[tokio::test]
    async fn browser_executor_maps_wait_without_session_event() {
        let session = MockSession::new();
        let mut executor = BrowserActionExecutor::new(session);

        let result = executor
            .execute(&BrowserAction::Wait(WaitAction { seconds: 0 }))
            .await;

        assert_eq!(result.error, None);
        assert_eq!(
            result.extracted_content.as_deref(),
            Some("Waited for 0 seconds")
        );
        assert_eq!(executor.session().events(), Vec::<String>::new());
    }

    #[tokio::test]
    async fn browser_executor_requests_next_screenshot_observation() {
        let session = MockSession::new();
        let mut executor = BrowserActionExecutor::new(session);

        let result = executor
            .execute(&BrowserAction::Screenshot(ScreenshotAction {
                file_name: None,
            }))
            .await;

        assert_eq!(result.error, None);
        assert_eq!(
            result.extracted_content.as_deref(),
            Some("Requested screenshot for next observation")
        );
        assert_eq!(
            result
                .metadata
                .as_ref()
                .and_then(|metadata| metadata.get("include_screenshot"))
                .and_then(Value::as_bool),
            Some(true)
        );
        assert_eq!(executor.session().events(), Vec::<String>::new());
    }

    #[tokio::test]
    async fn browser_executor_saves_screenshot_when_file_name_is_present() {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let requested_output_path = temp_dir.path().join("shot");
        let output_path = temp_dir.path().join("shot.png");
        let session = MockSession::new();
        let mut executor = BrowserActionExecutor::new(session);

        let result = executor
            .execute(&BrowserAction::Screenshot(ScreenshotAction {
                file_name: Some(requested_output_path.display().to_string()),
            }))
            .await;

        assert_eq!(result.error, None);
        assert!(
            result
                .extracted_content
                .expect("screenshot content")
                .contains(&output_path.display().to_string())
        );
        assert_eq!(result.attachments, vec![output_path.display().to_string()]);
        assert_eq!(
            std::fs::read(&output_path).expect("screenshot file"),
            b"PNGDATA"
        );
        assert_eq!(executor.session().events(), vec!["screenshot"]);
    }

    #[test]
    fn screenshot_output_path_appends_png_extension() {
        assert_eq!(
            screenshot_output_path("/tmp/shot"),
            std::path::PathBuf::from("/tmp/shot.png")
        );
        assert_eq!(
            screenshot_output_path("/tmp/shot.PNG"),
            std::path::PathBuf::from("/tmp/shot.PNG")
        );
        assert_eq!(
            screenshot_output_path(""),
            std::path::PathBuf::from("screenshot.png")
        );
    }

    #[tokio::test]
    async fn browser_executor_maps_upload_file_to_session() {
        let session = MockSession::new();
        let mut executor = BrowserActionExecutor::new(session);

        let result = executor
            .execute(&BrowserAction::UploadFile(UploadFileAction {
                index: 3,
                path: "/tmp/evalops-upload.txt".to_owned(),
            }))
            .await;

        assert_eq!(result.error, None);
        assert_eq!(
            result.extracted_content.as_deref(),
            Some("Uploaded /tmp/evalops-upload.txt to element 3")
        );
        assert_eq!(
            executor.session().events(),
            vec!["upload_file:3:/tmp/evalops-upload.txt"]
        );
    }

    #[tokio::test]
    async fn browser_executor_enforces_available_upload_paths_when_enabled() {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let upload_path = temp_dir.path().join("allowed.txt");
        std::fs::write(&upload_path, "upload me").expect("upload file");
        let session = MockSession::new();
        let mut executor = BrowserActionExecutor::new(session);
        executor.set_upload_file_availability(true, vec![upload_path.display().to_string()]);

        let result = executor
            .execute(&BrowserAction::UploadFile(UploadFileAction {
                index: 3,
                path: upload_path.display().to_string(),
            }))
            .await;

        assert_eq!(result.error, None);
        assert_eq!(
            executor.session().events(),
            vec![format!("upload_file:3:{}", upload_path.display())]
        );
    }

    #[tokio::test]
    async fn browser_executor_rejects_unavailable_upload_paths_before_session_call() {
        let session = MockSession::new();
        let mut executor = BrowserActionExecutor::new(session);
        executor.set_upload_file_availability(true, Vec::new());

        let result = executor
            .execute(&BrowserAction::UploadFile(UploadFileAction {
                index: 3,
                path: "/tmp/not-declared.txt".to_owned(),
            }))
            .await;

        assert!(
            result
                .error
                .as_deref()
                .expect("upload error")
                .contains("AgentSettings.available_file_paths")
        );
        assert!(executor.session().events().is_empty());
    }

    #[tokio::test]
    async fn browser_executor_rejects_empty_available_upload_files() {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let upload_path = temp_dir.path().join("empty.txt");
        std::fs::write(&upload_path, "").expect("empty upload file");
        let session = MockSession::new();
        let mut executor = BrowserActionExecutor::new(session);
        executor.set_upload_file_availability(true, vec![upload_path.display().to_string()]);

        let result = executor
            .execute(&BrowserAction::UploadFile(UploadFileAction {
                index: 3,
                path: upload_path.display().to_string(),
            }))
            .await;

        assert!(
            result
                .error
                .as_deref()
                .expect("upload error")
                .contains("empty (0 bytes)")
        );
        assert!(executor.session().events().is_empty());
    }

    #[tokio::test]
    async fn browser_executor_uploads_managed_files_by_relative_name() {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let session = MockSession::new();
        let file_system = ManagedFileSystem::new(temp_dir.path()).expect("managed file system");
        let mut executor = BrowserActionExecutor::with_file_system(session, file_system);
        executor.set_upload_file_availability(true, Vec::new());
        executor
            .execute(&BrowserAction::WriteFile(WriteFileAction {
                file_name: "report.md".to_owned(),
                content: "upload me".to_owned(),
                append: false,
                trailing_newline: false,
                leading_newline: false,
            }))
            .await;
        let managed_path = executor.file_system().data_dir().join("report.md");

        let result = executor
            .execute(&BrowserAction::UploadFile(UploadFileAction {
                index: 3,
                path: "report.md".to_owned(),
            }))
            .await;

        assert_eq!(result.error, None);
        assert_eq!(
            executor.session().events(),
            vec![format!("upload_file:3:{}", managed_path.display())]
        );
    }

    #[tokio::test]
    async fn browser_executor_saves_pdf_when_file_name_is_present() {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let requested_output_path = temp_dir.path().join("out");
        let output_path = temp_dir.path().join("out.pdf");
        let output = requested_output_path.display().to_string();
        let session = MockSession::new();
        let mut executor = BrowserActionExecutor::new(session);

        let result = executor
            .execute(&BrowserAction::SaveAsPdf(SaveAsPdfAction {
                file_name: Some(output.clone()),
                print_background: true,
                landscape: false,
                scale: 1.0,
                paper_format: "Letter".to_owned(),
            }))
            .await;

        assert_eq!(result.error, None);
        assert!(
            result
                .extracted_content
                .expect("pdf content")
                .contains(&output_path.display().to_string())
        );
        assert_eq!(result.attachments, vec![output_path.display().to_string()]);
        assert!(
            std::fs::read(&output_path)
                .expect("pdf file")
                .starts_with(b"%PDF")
        );
        assert_eq!(
            executor.session().events(),
            vec!["save_pdf:true:false:1:Letter"]
        );
    }

    #[test]
    fn pdf_output_path_uses_sanitized_title_and_deduplicates() {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let existing = temp_dir.path().join("Quarterly Plan  2026.pdf");
        std::fs::write(&existing, b"existing").expect("existing pdf");

        let output = pdf_output_path(None, Some("Quarterly: Plan / 2026"));
        assert_eq!(output, std::path::PathBuf::from("Quarterly Plan  2026.pdf"));

        let duplicate = next_available_pdf_path(existing);
        assert_eq!(
            duplicate.file_name().and_then(std::ffi::OsStr::to_str),
            Some("Quarterly Plan  2026 (1).pdf")
        );
        assert_eq!(
            pdf_output_path(Some("/tmp/report"), None),
            std::path::PathBuf::from("/tmp/report.pdf")
        );
        assert_eq!(
            pdf_output_path(Some("/tmp/report.PDF"), None),
            std::path::PathBuf::from("/tmp/report.PDF")
        );
    }

    #[tokio::test]
    async fn browser_executor_extracts_page_text() {
        let session = MockSession::new();
        let mut executor = BrowserActionExecutor::new(session);

        let result = executor
            .execute(&BrowserAction::Extract(ExtractAction {
                query: "company".to_owned(),
                extract_links: false,
                extract_images: false,
                start_from_char: 6,
                output_schema: None,
                already_collected: vec![],
            }))
            .await;

        let content = result.extracted_content.expect("extracted content");
        assert!(content.contains("<url>\nabout:blank\n</url>"));
        assert!(content.contains("<query>\ncompany\n</query>"));
        assert!(content.contains("<content_stats>"));
        assert!(content.contains("started from char 6"));
        assert!(content.contains("<webpage_content>"));
        assert!(content.contains("EvalOps Beta"));
        assert_eq!(executor.session().events(), vec!["page_text"]);
    }

    #[tokio::test]
    async fn browser_executor_extracts_links_images_schema_and_dedupe_hints() {
        let session = MockSession::new();
        let mut executor = BrowserActionExecutor::new(session);

        let result = executor
            .execute(&BrowserAction::Extract(ExtractAction {
                query: "product image URLs".to_owned(),
                extract_links: true,
                extract_images: false,
                start_from_char: 0,
                output_schema: Some(serde_json::json!({
                    "type": "object",
                    "properties": {
                        "products": { "type": "array" }
                    }
                })),
                already_collected: vec!["Existing product".to_owned()],
            }))
            .await;

        let content = result.extracted_content.expect("extracted content");
        assert!(content.contains("<output_schema>"));
        assert!(content.contains(r#""type": "object""#));
        assert!(content.contains("extract_links=true, extract_images=true"));
        assert!(content.contains("<links>\n- Run EvalOps: https://evalops.dev/run\n</links>"));
        assert!(content.contains("<images>\n- Hero shot: https://evalops.dev/hero.png\n</images>"));
        assert!(content.contains("<already_collected>"));
        assert!(content.contains("- Existing product"));
        let metadata = result.metadata.expect("extract metadata");
        assert_eq!(metadata["structured_extraction"], true);
        assert_eq!(metadata["source_url"], "about:blank");
        assert_eq!(metadata["schema_used"]["type"], "object");
        assert_eq!(metadata["is_partial"], false);
        assert_eq!(metadata["content_stats"]["method"], "page_text");
        assert_eq!(metadata["options"]["extract_links"], true);
        assert_eq!(metadata["options"]["extract_images"], true);
        assert_eq!(metadata["options"]["links_count"], 1);
        assert_eq!(metadata["options"]["images_count"], 1);
        assert_eq!(
            executor.session().events(),
            vec![
                "page_text",
                "find_elements:a[href]:href|title|aria-label|rel:200:true",
                "find_elements:img[src], img[data-src], picture source[srcset]:src|data-src|srcset|alt|title|aria-label:200:false"
            ]
        );
    }

    #[tokio::test]
    async fn browser_executor_rejects_extract_start_beyond_content() {
        let session = MockSession::new();
        let mut executor = BrowserActionExecutor::new(session);

        let result = executor
            .execute(&BrowserAction::Extract(ExtractAction {
                query: "company".to_owned(),
                extract_links: false,
                extract_images: false,
                start_from_char: 999,
                output_schema: None,
                already_collected: vec![],
            }))
            .await;

        assert!(
            result
                .error
                .as_deref()
                .expect("extract error")
                .contains("exceeds content length")
        );
        assert_eq!(executor.session().events(), vec!["page_text"]);
    }

    #[tokio::test]
    async fn browser_executor_searches_page_text() {
        let session = MockSession::new();
        let mut executor = BrowserActionExecutor::new(session);

        let result = executor
            .execute(&BrowserAction::SearchPage(SearchPageAction {
                pattern: "evalops".to_owned(),
                regex: false,
                case_sensitive: false,
                context_chars: 5,
                css_scope: None,
                max_results: 2,
            }))
            .await;

        let content = result.extracted_content.expect("search content");
        assert!(content.contains("Found 2 matches for \"evalops\" on page:"));
        assert!(content.contains("[1] ...lpha EvalOps Beta..."));
        assert!(content.contains("[2] ...cond EvalOps line"));
        assert!(content.contains("EvalOps"));
        assert_eq!(content.matches("EvalOps").count(), 2);
    }

    #[tokio::test]
    async fn browser_executor_finds_css_elements() {
        let session = MockSession::new();
        let mut executor = BrowserActionExecutor::new(session);

        let result = executor
            .execute(&BrowserAction::FindElements(FindElementsAction {
                selector: "button".to_owned(),
                attributes: Some(vec!["id".to_owned()]),
                max_results: 3,
                include_text: true,
            }))
            .await;

        let content = result.extracted_content.expect("find content");
        assert!(content.contains("Found 1 element matching \"button\":"));
        assert!(content.contains("[0] <button> \"Run EvalOps\" {id=\"run\"}"));
        assert!(content.contains("Run EvalOps"));
        assert_eq!(
            executor.session().events(),
            vec!["find_elements:button:id:3:true"]
        );
    }

    struct QueueModel {
        outputs: Mutex<VecDeque<Result<Value, LlmError>>>,
        requests: Mutex<Vec<ChatRequest>>,
    }

    impl QueueModel {
        fn new(outputs: Vec<Value>) -> Self {
            Self::with_results(outputs.into_iter().map(Ok).collect())
        }

        fn with_results(outputs: Vec<Result<Value, LlmError>>) -> Self {
            Self {
                outputs: Mutex::new(outputs.into()),
                requests: Mutex::new(Vec::new()),
            }
        }
    }

    fn request_text(request: &ChatRequest) -> String {
        request
            .messages
            .iter()
            .flat_map(|message| message.content.iter())
            .filter_map(|part| match part {
                ContentPart::Text { text } => Some(text.as_str()),
                ContentPart::ImageUrl { .. } => None,
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[async_trait]
    impl ChatModel for QueueModel {
        fn provider(&self) -> &str {
            "test"
        }

        fn model(&self) -> &str {
            "static"
        }

        async fn invoke_json(
            &self,
            request: ChatRequest,
        ) -> Result<ChatCompletion<Value>, LlmError> {
            assert!(
                request
                    .messages
                    .iter()
                    .any(|message| message.role == MessageRole::User)
            );
            assert!(request.output_schema.is_some());
            self.requests.lock().expect("requests lock").push(request);
            let content = self
                .outputs
                .lock()
                .expect("outputs lock")
                .pop_front()
                .ok_or_else(|| LlmError::Provider("no queued model output".to_owned()))??;
            Ok(ChatCompletion {
                model: self.model().to_owned(),
                content,
                raw_response: None,
            })
        }
    }

    #[tokio::test]
    async fn agent_extract_action_uses_llm_result_sections() {
        let agent_output = serde_json::json!({
            "current_state": {
                "thinking": "extract"
            },
            "action": [
                {
                    "extract": {
                        "query": "company summary",
                        "extract_links": false,
                        "extract_images": false,
                        "start_from_char": 0,
                        "already_collected": []
                    }
                }
            ]
        });
        let extract_output = serde_json::json!({
            "result": "EvalOps appears in the page content."
        });
        let mut agent = Agent::new(
            "extract company summary",
            QueueModel::new(vec![agent_output, extract_output]),
            MockSession::new(),
        );

        let result = {
            let item = agent.step().await.expect("agent step");
            item.result[0].clone()
        };

        let content = result.extracted_content.as_deref().expect("extract result");
        assert!(content.contains("<url>\nabout:blank\n</url>"));
        assert!(content.contains("<query>\ncompany summary\n</query>"));
        assert!(content.contains("<result>\nEvalOps appears in the page content.\n</result>"));
        assert_eq!(result.long_term_memory.as_deref(), Some(content));
        assert!(!result.include_extracted_content_only_once);

        let requests = agent.llm.requests.lock().expect("requests lock").clone();
        assert_eq!(requests.len(), 2);
        let extract_request_text = request_text(&requests[1]);
        assert!(extract_request_text.contains("<webpage_content>"));
        assert!(extract_request_text.contains("Alpha EvalOps Beta"));
        assert!(
            requests[1]
                .output_schema
                .as_ref()
                .is_some_and(|schema| schema["properties"]["result"]["type"] == "string")
        );
    }

    #[tokio::test]
    async fn agent_extract_action_passes_structured_schema_to_llm() {
        let schema = serde_json::json!({
            "type": "object",
            "properties": {
                "company": { "type": "string" },
                "links": {
                    "type": "array",
                    "items": { "type": "string" }
                }
            },
            "required": ["company"]
        });
        let agent_output = serde_json::json!({
            "current_state": {},
            "action": [
                {
                    "extract": {
                        "query": "company and links",
                        "extract_links": true,
                        "extract_images": false,
                        "start_from_char": 0,
                        "output_schema": schema,
                        "already_collected": ["Old EvalOps"]
                    }
                }
            ]
        });
        let extract_output = serde_json::json!({
            "company": "EvalOps",
            "links": ["https://evalops.dev/run"]
        });
        let mut agent = Agent::new(
            "extract structured company data",
            QueueModel::new(vec![agent_output, extract_output]),
            MockSession::new(),
        );

        let result = {
            let item = agent.step().await.expect("agent step");
            item.result[0].clone()
        };

        let content = result.extracted_content.as_deref().expect("extract result");
        assert!(content.contains("<structured_result>"));
        assert!(content.contains(r#""company":"EvalOps""#));
        assert!(content.contains("https://evalops.dev/run"));

        let metadata = result.metadata.as_ref().expect("structured metadata");
        assert_eq!(metadata["structured_extraction"], true);
        assert_eq!(metadata["extraction_result"]["data"]["company"], "EvalOps");
        assert_eq!(
            metadata["extraction_result"]["schema_used"]["required"][0],
            "company"
        );
        assert_eq!(metadata["extraction_result"]["source_url"], "about:blank");

        let requests = agent.llm.requests.lock().expect("requests lock").clone();
        assert_eq!(requests.len(), 2);
        assert_eq!(
            requests[1].output_schema.as_ref().expect("extract schema"),
            &schema
        );
        let extract_request_text = request_text(&requests[1]);
        assert!(extract_request_text.contains("<links>"));
        assert!(extract_request_text.contains("<already_collected>"));
    }

    #[tokio::test]
    async fn agent_extract_action_records_llm_failure_as_action_error() {
        let agent_output = serde_json::json!({
            "current_state": {},
            "action": [
                {
                    "extract": {
                        "query": "company summary",
                        "extract_links": false,
                        "extract_images": false,
                        "start_from_char": 0,
                        "already_collected": []
                    }
                }
            ]
        });
        let mut agent = Agent::new(
            "extract with failing model",
            QueueModel::with_results(vec![
                Ok(agent_output),
                Err(LlmError::Provider("extract unavailable".to_owned())),
            ]),
            MockSession::new(),
        );

        let result = {
            let item = agent.step().await.expect("agent step");
            item.result[0].clone()
        };

        assert!(result.extracted_content.is_none());
        assert!(
            result
                .error
                .as_deref()
                .is_some_and(|error| error.contains("LLM-backed extract failed"))
        );
    }

    #[tokio::test]
    async fn agent_step_records_done_history() {
        let output = serde_json::json!({
            "current_state": {
                "thinking": "done"
            },
            "action": [
                {
                    "done": {
                        "text": "finished",
                        "success": true,
                        "files_to_display": []
                    }
                }
            ]
        });
        let mut agent = Agent::new(
            "finish the task",
            QueueModel::new(vec![output]),
            MockSession::new(),
        );

        let item = agent.step().await.expect("agent step");

        assert_eq!(item.result, vec![ActionResult::done("finished", true)]);
        let metadata = item.metadata.as_ref().expect("step metadata");
        assert_eq!(metadata.step_number, 1);
        assert_eq!(metadata.step_interval, None);
        assert!(metadata.step_end_time >= metadata.step_start_time);
        assert!(metadata.duration_seconds() >= 0.0);
        assert_eq!(agent.history().final_result(), Some("finished"));
    }

    #[tokio::test]
    async fn agent_file_system_state_survives_steps_and_prompts() {
        let write_output = serde_json::json!({
            "current_state": {
                "thinking": "write report"
            },
            "action": [
                {
                    "write_file": {
                        "file_name": "report.md",
                        "content": "alpha",
                        "append": false,
                        "trailing_newline": false,
                        "leading_newline": false
                    }
                }
            ]
        });
        let read_output = serde_json::json!({
            "current_state": {
                "thinking": "read report"
            },
            "action": [
                {
                    "read_file": {
                        "file_name": "report.md"
                    }
                }
            ]
        });
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let file_system = ManagedFileSystem::new(temp_dir.path()).expect("managed file system");
        let mut agent = Agent::with_settings_and_file_system(
            "write then read a report",
            AgentSettings::default(),
            QueueModel::new(vec![write_output, read_output]),
            MockSession::new(),
            file_system,
        );

        agent.step().await.expect("write step");
        assert_eq!(
            agent.file_system().display_file("report.md").as_deref(),
            Some("alpha")
        );

        let item = agent.step().await.expect("read step");
        assert!(
            item.result
                .first()
                .and_then(|result| result.extracted_content.as_deref())
                .expect("read result")
                .contains("alpha")
        );

        let requests = agent.llm.requests.lock().expect("requests lock");
        let second_prompt = request_text(&requests[1]);
        assert!(second_prompt.contains("<file_system>\n<file>\nreport.md - 1 lines"));
        assert!(second_prompt.contains("<content>\nalpha\n</content>"));

        let state = agent.file_system_state();
        assert!(state.files.contains_key("report.md"));
        let restored = ManagedFileSystem::from_state(state).expect("restore file system");
        assert_eq!(restored.display_file("report.md").as_deref(), Some("alpha"));
    }

    #[tokio::test]
    async fn agent_rejects_unavailable_upload_paths_before_browser_side_effects() {
        let upload_output = serde_json::json!({
            "current_state": {
                "thinking": "upload"
            },
            "action": [
                {
                    "upload_file": {
                        "index": 3,
                        "path": "/tmp/not-declared.txt"
                    }
                }
            ]
        });
        let mut agent = Agent::new(
            "upload a file",
            QueueModel::new(vec![upload_output]),
            MockSession::new(),
        );

        let item = agent.step().await.expect("agent step");

        assert!(
            item.result
                .first()
                .and_then(|result| result.error.as_deref())
                .expect("upload error")
                .contains("AgentSettings.available_file_paths")
        );
        assert!(agent.executor.session().events().is_empty());
    }

    #[tokio::test]
    async fn restored_agent_continues_with_serialized_file_system_state() {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let mut file_system = ManagedFileSystem::new(temp_dir.path()).expect("managed file system");
        file_system
            .write_file(&WriteFileAction {
                file_name: "todo.md".to_owned(),
                content: "- read the restored report".to_owned(),
                append: false,
                trailing_newline: false,
                leading_newline: false,
            })
            .expect("write todo");
        file_system
            .write_file(&WriteFileAction {
                file_name: "report.md".to_owned(),
                content: "alpha\nbeta".to_owned(),
                append: false,
                trailing_newline: false,
                leading_newline: false,
            })
            .expect("write report");

        let page_text = "EvalOps restored extracted content. ".repeat(700);
        let extract_result = extract_action_result(
            &ExtractAction {
                query: "restored context".to_owned(),
                extract_links: false,
                extract_images: false,
                start_from_char: 0,
                output_schema: None,
                already_collected: Vec::new(),
            },
            &page_text,
            Some("https://example.test/restored"),
            false,
            None,
            None,
            Some(&mut file_system),
        );
        assert!(
            extract_result
                .long_term_memory
                .as_deref()
                .expect("extract memory")
                .contains("Content in extracted_content_0.md and once in <read_state>.")
        );

        let state = file_system.get_state();
        assert_eq!(state.extracted_content_count, 1);
        let restored = ManagedFileSystem::from_state(state).expect("restore file system");
        let read_output = serde_json::json!({
            "current_state": {
                "thinking": "read restored report"
            },
            "action": [
                {
                    "read_file": {
                        "file_name": "report.md"
                    }
                }
            ]
        });
        let done_output = serde_json::json!({
            "current_state": {
                "thinking": "done"
            },
            "action": [
                {
                    "done": {
                        "text": "restored report read",
                        "success": true,
                        "files_to_display": []
                    }
                }
            ]
        });
        let mut agent = Agent::with_settings_and_file_system(
            "continue after restore",
            AgentSettings {
                use_judge: false,
                ..AgentSettings::default()
            },
            QueueModel::new(vec![read_output, done_output]),
            MockSession::new(),
            restored,
        );

        let history = agent.run(2).await.expect("restored agent run");
        assert!(history.is_done());
        assert_eq!(history.final_result(), Some("restored report read"));
        assert!(
            history.items[0].result[0]
                .extracted_content
                .as_deref()
                .expect("read result")
                .contains("<content>\nalpha\nbeta\n</content>")
        );

        let requests = agent.llm.requests.lock().expect("requests lock");
        assert_eq!(requests.len(), 2);
        let first_prompt = request_text(&requests[0]);
        assert!(first_prompt.contains("<file_system>\n<file>\nextracted_content_0.md -"));
        assert!(first_prompt.contains("<file>\nreport.md - 2 lines"));
        assert!(first_prompt.contains("<content>\nalpha\nbeta\n</content>"));
        assert!(
            first_prompt.contains("<todo_contents>\n- read the restored report\n</todo_contents>")
        );
        let second_prompt = request_text(&requests[1]);
        assert!(second_prompt.contains("<read_state_0>"));
        assert!(second_prompt.contains("Read from file report.md."));
        drop(requests);

        let next_file = agent
            .file_system_mut()
            .save_extracted_content("second restored extract")
            .expect("next extracted content");
        assert_eq!(next_file, "extracted_content_1.md");
        assert_eq!(agent.file_system_state().extracted_content_count, 2);
        assert_eq!(
            agent
                .file_system()
                .display_file("extracted_content_1.md")
                .as_deref(),
            Some("second restored extract")
        );
    }

    #[tokio::test]
    async fn agent_checkpoint_round_trips_and_resumes_state() {
        let write_output = serde_json::json!({
            "current_state": {
                "thinking": "write checkpoint files"
            },
            "action": [
                {
                    "write_file": {
                        "file_name": "todo.md",
                        "content": "- resume and read report",
                        "append": false,
                        "trailing_newline": false,
                        "leading_newline": false
                    }
                },
                {
                    "write_file": {
                        "file_name": "report.md",
                        "content": "alpha\nbeta",
                        "append": false,
                        "trailing_newline": false,
                        "leading_newline": false
                    }
                }
            ]
        });
        let settings = AgentSettings {
            initial_actions: vec![BrowserAction::Wait(WaitAction { seconds: 0 })],
            use_judge: false,
            ..AgentSettings::default()
        };
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let file_system = ManagedFileSystem::new(temp_dir.path()).expect("managed file system");
        let mut agent = Agent::with_settings_and_file_system(
            "checkpoint the agent",
            settings,
            QueueModel::new(vec![write_output]),
            MockSession::new(),
            file_system,
        );

        agent
            .execute_initial_actions()
            .await
            .expect("initial checkpoint action");
        agent.step().await.expect("write checkpoint files");
        assert_eq!(agent.history().items.len(), 2);
        assert_eq!(
            agent
                .history()
                .items
                .first()
                .and_then(|item| item.metadata.as_ref())
                .map(|metadata| metadata.step_number),
            Some(0)
        );
        assert_eq!(
            agent
                .file_system_mut()
                .save_extracted_content("checkpoint extract")
                .expect("seed extract"),
            "extracted_content_0.md"
        );

        let checkpoint = agent.checkpoint();
        assert!(checkpoint.initial_actions_executed);
        assert_eq!(checkpoint.task, "checkpoint the agent");
        assert_eq!(checkpoint.history.items.len(), 2);
        assert_eq!(checkpoint.file_system_state.extracted_content_count, 1);
        let checkpoint_json =
            serde_json::to_string_pretty(&checkpoint).expect("serialize checkpoint");
        let checkpoint: AgentCheckpoint =
            serde_json::from_str(&checkpoint_json).expect("deserialize checkpoint");
        assert!(checkpoint.initial_actions_executed);

        let read_output = serde_json::json!({
            "current_state": {
                "thinking": "read restored report"
            },
            "action": [
                {
                    "read_file": {
                        "file_name": "report.md"
                    }
                }
            ]
        });
        let done_output = serde_json::json!({
            "current_state": {
                "thinking": "done"
            },
            "action": [
                {
                    "done": {
                        "text": "checkpoint resumed",
                        "success": true,
                        "files_to_display": []
                    }
                }
            ]
        });
        let mut resumed = Agent::from_checkpoint(
            checkpoint,
            QueueModel::new(vec![read_output, done_output]),
            MockSession::new(),
        )
        .expect("resume from checkpoint");

        let history = resumed.run(2).await.expect("resumed run");
        assert!(history.is_done());
        assert_eq!(history.items.len(), 4);
        assert_eq!(history.final_result(), Some("checkpoint resumed"));
        assert!(
            history.items[2].result[0]
                .extracted_content
                .as_deref()
                .expect("read result")
                .contains("<content>\nalpha\nbeta\n</content>")
        );

        let requests = resumed.llm.requests.lock().expect("requests lock");
        assert_eq!(requests.len(), 2);
        let first_prompt = request_text(&requests[0]);
        assert!(first_prompt.contains("Wrote file report.md"));
        assert!(first_prompt.contains("<file_system>\n<file>\nextracted_content_0.md -"));
        assert!(
            first_prompt.contains("<todo_contents>\n- resume and read report\n</todo_contents>")
        );
        let second_prompt = request_text(&requests[1]);
        assert!(second_prompt.contains("<read_state_0>"));
        assert!(second_prompt.contains("Read from file report.md."));
        drop(requests);

        let next_file = resumed
            .file_system_mut()
            .save_extracted_content("after checkpoint")
            .expect("next extracted content");
        assert_eq!(next_file, "extracted_content_1.md");
        assert_eq!(resumed.file_system_state().extracted_content_count, 2);
    }

    #[tokio::test]
    async fn agent_replaces_sensitive_input_tags_for_execution_without_history_leak() {
        let output = serde_json::json!({
            "current_state": {},
            "action": [
                {
                    "input": {
                        "index": 1,
                        "text": "<secret>password</secret>",
                        "clear": true
                    }
                }
            ]
        });
        let settings = AgentSettings {
            sensitive_data: BTreeMap::from([(
                "password".to_owned(),
                SensitiveDataValue::Value("correct horse battery staple".to_owned()),
            )]),
            ..AgentSettings::default()
        };
        let mut agent = Agent::with_settings(
            "enter password",
            settings,
            QueueModel::new(vec![output]),
            MockSession::new(),
        );

        let recorded_action_text = {
            let item = agent.step().await.expect("agent step");
            let model_output = item.model_output.as_ref().expect("model output");
            let BrowserAction::Input(params) = &model_output.action[0] else {
                panic!("expected input action");
            };
            params.text.clone()
        };

        assert_eq!(
            agent.executor.session().events(),
            vec!["input:1:correct horse battery staple:true"]
        );
        assert_eq!(recorded_action_text, "<secret>password</secret>");

        let request = agent.llm.requests.lock().expect("requests lock");
        let prompt_text = request_text(&request[0]);
        assert!(prompt_text.contains("<sensitive_data>SENSITIVE DATA"));
        assert!(prompt_text.contains("<secret>password</secret>"));
        assert!(!prompt_text.contains("correct horse battery staple"));
    }

    #[test]
    fn actions_for_execution_replaces_sensitive_tags_and_literals_across_params() {
        let settings = AgentSettings {
            sensitive_data: BTreeMap::from([
                (
                    "api_key".to_owned(),
                    SensitiveDataValue::Value("sk-live-123".to_owned()),
                ),
                (
                    "username".to_owned(),
                    SensitiveDataValue::Value("evalops-user".to_owned()),
                ),
            ]),
            ..AgentSettings::default()
        };
        let actions = vec![
            BrowserAction::WriteFile(WriteFileAction {
                file_name: "request.txt".to_owned(),
                content: "Authorization: Bearer <secret>api_key</secret>".to_owned(),
                append: false,
                trailing_newline: true,
                leading_newline: false,
            }),
            BrowserAction::Input(InputTextAction {
                index: 1,
                text: "username".to_owned(),
                clear: true,
            }),
        ];

        let replaced = actions_for_execution(&actions, &settings, "https://example.test");

        assert_eq!(
            replaced[0],
            BrowserAction::WriteFile(WriteFileAction {
                file_name: "request.txt".to_owned(),
                content: "Authorization: Bearer sk-live-123".to_owned(),
                append: false,
                trailing_newline: true,
                leading_newline: false,
            })
        );
        assert_eq!(
            replaced[1],
            BrowserAction::Input(InputTextAction {
                index: 1,
                text: "evalops-user".to_owned(),
                clear: true,
            })
        );
    }

    #[test]
    fn sensitive_bu_2fa_code_placeholders_generate_totp_values() {
        let settings = AgentSettings {
            sensitive_data: BTreeMap::from([(
                "login_bu_2fa_code".to_owned(),
                SensitiveDataValue::Value("GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ".to_owned()),
            )]),
            ..AgentSettings::default()
        };
        let actions = vec![BrowserAction::Input(InputTextAction {
            index: 1,
            text: "<secret>login_bu_2fa_code</secret>".to_owned(),
            clear: true,
        })];

        assert_eq!(
            totp_code_at("GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ", 59, 30, 8),
            Some("94287082".to_owned())
        );

        let replaced = actions_for_execution(&actions, &settings, "https://example.test");
        let BrowserAction::Input(params) = &replaced[0] else {
            panic!("expected input action");
        };

        assert_eq!(params.text.len(), 6);
        assert!(
            params
                .text
                .chars()
                .all(|character| character.is_ascii_digit())
        );
        assert_ne!(params.text, "GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ");
    }

    struct SlowModel;

    struct QueueThenPendingModel {
        first_output: Mutex<Option<Value>>,
    }

    impl QueueThenPendingModel {
        fn new(first_output: Value) -> Self {
            Self {
                first_output: Mutex::new(Some(first_output)),
            }
        }
    }

    #[async_trait]
    impl ChatModel for SlowModel {
        fn provider(&self) -> &str {
            "test"
        }

        fn model(&self) -> &str {
            "slow"
        }

        async fn invoke_json(
            &self,
            _request: ChatRequest,
        ) -> Result<ChatCompletion<Value>, LlmError> {
            std::future::pending::<()>().await;
            unreachable!("pending model should be cancelled by timeout")
        }
    }

    #[async_trait]
    impl ChatModel for QueueThenPendingModel {
        fn provider(&self) -> &str {
            "test"
        }

        fn model(&self) -> &str {
            "queue-then-pending"
        }

        async fn invoke_json(
            &self,
            _request: ChatRequest,
        ) -> Result<ChatCompletion<Value>, LlmError> {
            let first_output = self.first_output.lock().expect("first output lock").take();
            if let Some(content) = first_output {
                return Ok(ChatCompletion {
                    model: self.model().to_owned(),
                    content,
                    raw_response: None,
                });
            }
            std::future::pending::<()>().await;
            unreachable!("pending model should be cancelled by timeout")
        }
    }

    #[tokio::test]
    async fn agent_step_enforces_llm_timeout() {
        let settings = AgentSettings {
            llm_timeout_seconds: 0,
            ..AgentSettings::default()
        };
        let mut agent = Agent::with_settings("timeout", settings, SlowModel, MockSession::new());

        let error = agent.step().await.expect_err("LLM timeout");

        assert!(matches!(error, AgentRunError::LlmTimedOut { seconds: 0 }));
    }

    #[tokio::test]
    async fn agent_step_enforces_step_timeout() {
        let settings = AgentSettings {
            step_timeout_seconds: 0,
            ..AgentSettings::default()
        };
        let mut agent =
            Agent::with_settings("step timeout", settings, SlowModel, MockSession::new());

        let error = agent.step().await.expect_err("step timeout");

        assert!(matches!(error, AgentRunError::StepTimedOut { seconds: 0 }));
        assert!(agent.history().items.is_empty());
    }

    #[tokio::test]
    async fn agent_action_timeout_bounds_llm_backed_extract_resolution() {
        let output = serde_json::json!({
            "current_state": {},
            "action": [
                {
                    "extract": {
                        "query": "company summary",
                        "extract_links": false,
                        "extract_images": false,
                        "include_source": false
                    }
                }
            ]
        });
        let settings = AgentSettings {
            action_timeout_seconds: 0.005,
            llm_timeout_seconds: 30,
            ..AgentSettings::default()
        };
        let mut agent = Agent::with_settings(
            "extract with bounded action timeout",
            settings,
            QueueThenPendingModel::new(output),
            MockSession::new(),
        );

        let item = agent.step().await.expect("step records action timeout");

        let error = item.result[0].error.as_deref().expect("timeout error");
        assert!(error.contains("Action extract timed out after"));
        assert!(error.contains("dead CDP WebSocket"));
    }

    #[tokio::test]
    async fn agent_step_respects_use_vision_setting() {
        let output = serde_json::json!({
            "current_state": {},
            "action": [
                {
                    "done": {
                        "text": "complete",
                        "success": true,
                        "files_to_display": []
                    }
                }
            ]
        });
        let settings = AgentSettings {
            use_vision: VisionMode::Never,
            ..AgentSettings::default()
        };
        let mut agent = Agent::with_settings(
            "finish without screenshot",
            settings,
            QueueModel::new(vec![output]),
            MockSession::new(),
        );

        agent.step().await.expect("agent step");

        let state_requests = agent.executor.session().state_screenshot_requests();
        assert_eq!(state_requests.first(), Some(&false));
        assert!(state_requests.iter().all(|include| !include));
    }

    #[tokio::test]
    async fn agent_step_saves_conversation_transcript() {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let transcript_dir = temp_dir.path().join("conversations");
        let output = serde_json::json!({
            "current_state": {},
            "action": [
                {
                    "done": {
                        "text": "complete",
                        "success": true,
                        "files_to_display": []
                    }
                }
            ]
        });
        let mut state = blank_state();
        state.screenshot = Some("abc123".to_owned());
        let settings = AgentSettings {
            save_conversation_path: Some(transcript_dir.display().to_string()),
            ..AgentSettings::default()
        };
        let mut agent = Agent::with_settings(
            "save transcript",
            settings,
            QueueModel::new(vec![output]),
            MockSession::with_states(vec![state]),
        );
        let agent_id = agent.id();

        agent.step().await.expect("agent step");

        let transcript_path = transcript_dir.join(format!("conversation_{agent_id}_1.txt"));
        let transcript = std::fs::read_to_string(&transcript_path).expect("transcript file");
        assert!(transcript.contains(" system "));
        assert!(transcript.contains(" user "));
        assert!(transcript.contains("save transcript"));
        assert!(transcript.contains("<image_url detail=\"auto\">"));
        assert!(transcript.contains("data:image/png;base64,abc123"));
        assert!(transcript.contains("\"done\""));
        assert!(transcript.contains("\"text\": \"complete\""));
    }

    #[test]
    fn conversation_snapshot_encoding_honors_labels() {
        assert_eq!(
            encode_conversation_snapshot("hello", Some("utf8")).expect("utf8 bytes"),
            b"hello"
        );
        assert_eq!(
            encode_conversation_snapshot("£", Some("windows-1252")).expect("windows bytes"),
            vec![0xa3]
        );
    }

    #[test]
    fn conversation_snapshot_encoding_rejects_invalid_or_lossy_labels() {
        let invalid = encode_conversation_snapshot("hello", Some("not-real")).expect_err("invalid");
        assert!(matches!(
            invalid,
            AgentRunError::ConversationEncoding { encoding } if encoding == "not-real"
        ));

        let lossy = encode_conversation_snapshot("😀", Some("windows-1252")).expect_err("lossy");
        assert!(matches!(
            lossy,
            AgentRunError::ConversationEncodingLossy { encoding } if encoding == "windows-1252"
        ));
    }

    #[tokio::test]
    async fn agent_run_honors_screenshot_action_next_observation() {
        let screenshot_output = serde_json::json!({
            "current_state": {},
            "action": [
                {
                    "screenshot": {}
                }
            ]
        });
        let done_output = serde_json::json!({
            "current_state": {},
            "action": [
                {
                    "done": {
                        "text": "saw screenshot",
                        "success": true,
                        "files_to_display": []
                    }
                }
            ]
        });
        let settings = AgentSettings {
            use_vision: VisionMode::Auto,
            ..AgentSettings::default()
        };
        let mut agent = Agent::with_settings(
            "request screenshot then finish",
            settings,
            QueueModel::new(vec![screenshot_output, done_output]),
            MockSession::new(),
        );

        agent.run(2).await.expect("agent run");

        let state_requests = agent.executor.session().state_screenshot_requests();
        assert_eq!(state_requests.first(), Some(&false));
        assert_eq!(state_requests.last(), Some(&true));
        assert!(
            state_requests.iter().any(|include| *include),
            "expected screenshot request override: {state_requests:?}"
        );
    }

    #[tokio::test]
    async fn agent_step_defaults_to_screenshot_observation() {
        let output = serde_json::json!({
            "current_state": {},
            "action": [
                {
                    "done": {
                        "text": "complete",
                        "success": true,
                        "files_to_display": []
                    }
                }
            ]
        });
        let mut agent = Agent::with_settings(
            "finish with default vision",
            AgentSettings::default(),
            QueueModel::new(vec![output]),
            MockSession::new(),
        );

        agent.step().await.expect("agent step");

        let state_requests = agent.executor.session().state_screenshot_requests();
        assert_eq!(state_requests.first(), Some(&true));
    }

    #[tokio::test]
    async fn agent_step_rejects_screenshot_action_outside_auto_vision() {
        let output = serde_json::json!({
            "current_state": {},
            "action": [
                {
                    "screenshot": {}
                }
            ]
        });
        let mut agent = Agent::with_settings(
            "do not allow screenshot tool by default",
            AgentSettings::default(),
            QueueModel::new(vec![output]),
            MockSession::new(),
        );

        let error = {
            let item = agent.step().await.expect("agent step");
            assert_eq!(item.result.len(), 1);
            item.result[0]
                .error
                .as_deref()
                .expect("screenshot gating error")
                .to_owned()
        };

        assert!(agent.executor.session().events().is_empty());
        assert_eq!(
            agent.executor.session().state_screenshot_requests(),
            vec![true]
        );
        assert!(
            error.contains("use_vision") && error.contains("auto"),
            "unexpected error: {error}"
        );
    }

    #[tokio::test]
    async fn agent_step_never_requests_screenshot_when_vision_disabled() {
        let output = serde_json::json!({
            "current_state": {},
            "action": [
                {
                    "screenshot": {}
                }
            ]
        });
        let settings = AgentSettings {
            use_vision: VisionMode::Never,
            ..AgentSettings::default()
        };
        let mut agent = Agent::with_settings(
            "vision disabled blocks screenshot tool",
            settings,
            QueueModel::new(vec![output]),
            MockSession::new(),
        );

        let item = agent.step().await.expect("agent step");

        assert!(
            item.result[0]
                .error
                .as_deref()
                .is_some_and(|error| { error.contains("use_vision") && error.contains("auto") })
        );
        assert!(agent.executor.session().events().is_empty());
        assert_eq!(
            agent.executor.session().state_screenshot_requests(),
            vec![false]
        );
    }

    #[tokio::test]
    async fn agent_step_truncates_too_many_actions_like_upstream() {
        let output = serde_json::json!({
            "current_state": {},
            "action": [
                {
                    "click": {
                        "index": 1
                    }
                },
                {
                    "click": {
                        "index": 2
                    }
                }
            ]
        });
        let settings = AgentSettings {
            max_actions_per_step: 1,
            ..AgentSettings::default()
        };
        let mut agent = Agent::with_settings(
            "click only once",
            settings,
            QueueModel::new(vec![output]),
            MockSession::new(),
        );

        let item = agent.step().await.expect("agent step");

        assert_eq!(item.result.len(), 1);
        assert_eq!(item.result[0].error, None);
        assert_eq!(
            item.model_output
                .as_ref()
                .expect("model output")
                .action
                .len(),
            1
        );
        assert_eq!(agent.executor.session().events(), vec!["click:1"]);
    }

    #[tokio::test]
    async fn agent_step_rejects_excluded_actions_before_side_effects() {
        let output = serde_json::json!({
            "current_state": {},
            "action": [
                {
                    "click": {
                        "index": 1
                    }
                }
            ]
        });
        let settings = AgentSettings {
            excluded_actions: vec!["click".to_owned()],
            ..AgentSettings::default()
        };
        let mut agent = Agent::with_settings(
            "do not click",
            settings,
            QueueModel::new(vec![output]),
            MockSession::new(),
        );

        let error = {
            let item = agent.step().await.expect("agent step");
            assert_eq!(item.result.len(), 1);
            item.result[0]
                .error
                .as_deref()
                .expect("excluded error")
                .to_owned()
        };

        assert!(agent.executor.session().events().is_empty());
        assert!(
            error.contains("excluded action `click`"),
            "unexpected error: {error}"
        );
    }

    #[tokio::test]
    async fn agent_step_allows_non_excluded_actions_to_execute() {
        let output = serde_json::json!({
            "current_state": {},
            "action": [
                {
                    "click": {
                        "index": 1
                    }
                }
            ]
        });
        let settings = AgentSettings {
            excluded_actions: vec!["search".to_owned(), "switch-tab".to_owned()],
            ..AgentSettings::default()
        };
        let mut agent = Agent::with_settings(
            "click still allowed",
            settings,
            QueueModel::new(vec![output]),
            MockSession::new(),
        );

        {
            let item = agent.step().await.expect("agent step");
            assert_eq!(item.result.len(), 1);
            assert_eq!(item.result[0].error, None);
        }

        assert_eq!(agent.executor.session().events(), vec!["click:1"]);
    }

    #[tokio::test]
    async fn excluded_actions_never_block_done_outputs() {
        let output = serde_json::json!({
            "current_state": {},
            "action": [
                {
                    "done": {
                        "text": "complete",
                        "success": true,
                        "files_to_display": []
                    }
                }
            ]
        });
        let settings = AgentSettings {
            excluded_actions: vec!["done".to_owned()],
            ..AgentSettings::default()
        };
        let mut agent = Agent::with_settings(
            "finish",
            settings,
            QueueModel::new(vec![output]),
            MockSession::new(),
        );

        let history = agent.run(1).await.expect("agent run");

        assert_eq!(history.final_result(), Some("complete"));
        assert_eq!(history.is_successful(), Some(true));
    }

    #[tokio::test]
    async fn agent_run_executes_initial_actions_as_step_zero() {
        let done_output = serde_json::json!({
            "current_state": {},
            "action": [
                {
                    "done": {
                        "text": "ready",
                        "success": true,
                        "files_to_display": []
                    }
                }
            ]
        });
        let settings = AgentSettings {
            initial_actions: vec![BrowserAction::Navigate(NavigateAction {
                url: "https://example.test/start".to_owned(),
                new_tab: false,
            })],
            ..AgentSettings::default()
        };
        let mut agent = Agent::with_settings(
            "continue after preload",
            settings,
            QueueModel::new(vec![done_output]),
            MockSession::new(),
        );

        let history = agent.run(1).await.expect("agent run").clone();

        assert_eq!(
            agent.executor.session().events(),
            vec!["navigate:https://example.test/start:false"]
        );
        assert_eq!(history.items.len(), 2);
        assert_eq!(
            history.items[0]
                .metadata
                .as_ref()
                .expect("initial metadata")
                .step_number,
            0
        );
        assert_eq!(history.items[0].state.url, "https://example.test/start");
        assert_eq!(history.items[0].state.title, "Initial Actions");
        assert_eq!(
            history.items[0]
                .model_output
                .as_ref()
                .expect("initial output")
                .next_goal
                .as_deref(),
            Some("Initial navigation")
        );
        assert_eq!(
            history.items[1]
                .metadata
                .as_ref()
                .expect("step metadata")
                .step_number,
            1
        );

        let request = agent.llm.requests.lock().expect("requests lock");
        let prompt_text = request_text(&request[0]);
        assert!(prompt_text.contains("Initial navigation"));
        assert!(prompt_text.contains("Navigated to https://example.test/start"));
    }

    #[test]
    fn extract_start_url_from_task_matches_upstream_filters() {
        assert_eq!(
            extract_start_url_from_task("Open example.com and summarize it").as_deref(),
            Some("https://example.com")
        );
        assert_eq!(
            extract_start_url_from_task("Open https://example.com/path?q=1.").as_deref(),
            Some("https://example.com/path?q=1")
        );
        assert_eq!(
            extract_start_url_from_task("Email support@example.com for the status"),
            None
        );
        assert_eq!(
            extract_start_url_from_task("Read https://example.com/report.pdf"),
            None
        );
        assert_eq!(
            extract_start_url_from_task("Do not open example.com during this task"),
            None
        );
        assert_eq!(
            extract_start_url_from_task("Compare example.com and example.org"),
            None
        );
    }

    #[tokio::test]
    async fn agent_run_auto_navigates_single_task_url_as_initial_action() {
        let done_output = serde_json::json!({
            "current_state": {},
            "action": [
                {
                    "done": {
                        "text": "ready",
                        "success": true,
                        "files_to_display": []
                    }
                }
            ]
        });
        let mut agent = Agent::with_settings(
            "Summarize example.test/start before answering",
            AgentSettings {
                use_judge: false,
                ..AgentSettings::default()
            },
            QueueModel::new(vec![done_output]),
            MockSession::new(),
        );

        let history = agent.run(1).await.expect("agent run").clone();

        assert_eq!(
            agent.settings.initial_actions,
            vec![BrowserAction::Navigate(NavigateAction {
                url: "https://example.test/start".to_owned(),
                new_tab: false,
            })]
        );
        assert_eq!(
            agent.executor.session().events(),
            vec!["navigate:https://example.test/start:false"]
        );
        assert_eq!(history.items[0].state.url, "https://example.test/start");
        assert_eq!(history.items[0].state.title, "Initial Actions");
    }

    #[tokio::test]
    async fn agent_run_respects_directly_open_url_opt_out_and_explicit_initial_actions() {
        let done_output = serde_json::json!({
            "current_state": {},
            "action": [
                {
                    "done": {
                        "text": "ready",
                        "success": true,
                        "files_to_display": []
                    }
                }
            ]
        });
        let mut disabled_agent = Agent::with_settings(
            "Summarize example.test/start before answering",
            AgentSettings {
                directly_open_url: false,
                use_judge: false,
                ..AgentSettings::default()
            },
            QueueModel::new(vec![done_output.clone()]),
            MockSession::new(),
        );
        disabled_agent
            .run(1)
            .await
            .expect("disabled directly_open_url run");
        assert!(disabled_agent.settings.initial_actions.is_empty());
        assert!(disabled_agent.executor.session().events().is_empty());

        let explicit_action = BrowserAction::Navigate(NavigateAction {
            url: "https://explicit.example/start".to_owned(),
            new_tab: false,
        });
        let mut explicit_agent = Agent::with_settings(
            "Summarize example.test/start before answering",
            AgentSettings {
                initial_actions: vec![explicit_action.clone()],
                use_judge: false,
                ..AgentSettings::default()
            },
            QueueModel::new(vec![done_output]),
            MockSession::new(),
        );
        explicit_agent
            .run(1)
            .await
            .expect("explicit initial action run");
        assert_eq!(
            explicit_agent.settings.initial_actions,
            vec![explicit_action]
        );
        assert_eq!(
            explicit_agent.executor.session().events(),
            vec!["navigate:https://explicit.example/start:false"]
        );
    }

    #[tokio::test]
    async fn agent_run_invokes_step_and_done_callbacks() {
        let output = serde_json::json!({
            "current_state": {},
            "action": [
                {
                    "done": {
                        "text": "complete",
                        "success": true,
                        "files_to_display": []
                    }
                }
            ]
        });
        let events = Arc::new(Mutex::new(Vec::new()));
        let mut agent = Agent::with_settings(
            "complete",
            AgentSettings {
                use_judge: false,
                ..AgentSettings::default()
            },
            QueueModel::new(vec![output]),
            MockSession::new(),
        );

        let step_events = Arc::clone(&events);
        agent.register_new_step_callback(move |_state, output, step_number| {
            let action_name = output
                .action
                .first()
                .map(BrowserAction::name)
                .unwrap_or("none");
            step_events
                .lock()
                .expect("step events lock")
                .push(format!("step:{step_number}:{action_name}"));
            Ok::<(), String>(())
        });

        let done_events = Arc::clone(&events);
        agent.register_done_callback(move |history| {
            done_events
                .lock()
                .expect("done events lock")
                .push(format!("done:{}", history.items.len()));
            Ok::<(), String>(())
        });

        let history = agent.run(1).await.expect("agent run");

        assert_eq!(history.final_result(), Some("complete"));
        assert_eq!(
            events.lock().expect("events lock").as_slice(),
            ["step:1:done", "done:1"]
        );
    }

    #[tokio::test]
    async fn agent_run_invokes_async_step_callback() {
        let output = serde_json::json!({
            "current_state": {},
            "action": [
                {
                    "done": {
                        "text": "async complete",
                        "success": true,
                        "files_to_display": []
                    }
                }
            ]
        });
        let events = Arc::new(Mutex::new(Vec::new()));
        let mut agent = Agent::with_settings(
            "complete async",
            AgentSettings {
                use_judge: false,
                ..AgentSettings::default()
            },
            QueueModel::new(vec![output]),
            MockSession::new(),
        );

        let step_events = Arc::clone(&events);
        agent.register_new_step_callback_async(move |_state, output, step_number| {
            let step_events = Arc::clone(&step_events);
            let action_name = output
                .action
                .first()
                .map(BrowserAction::name)
                .unwrap_or("none")
                .to_owned();
            Box::pin(async move {
                step_events
                    .lock()
                    .expect("async step events lock")
                    .push(format!("async-step:{step_number}:{action_name}"));
                Ok(())
            })
        });

        let history = agent.run(1).await.expect("agent run");

        assert_eq!(history.final_result(), Some("async complete"));
        assert_eq!(
            events.lock().expect("events lock").as_slice(),
            ["async-step:1:done"]
        );
    }

    #[tokio::test]
    async fn agent_done_callback_sees_judged_history() {
        let done_output = serde_json::json!({
            "current_state": {},
            "action": [
                {
                    "done": {
                        "text": "judge before callback",
                        "success": true,
                        "files_to_display": []
                    }
                }
            ]
        });
        let judge_output = serde_json::json!({
            "reasoning": "Checked the terminal result before invoking callbacks.",
            "verdict": true,
            "failure_reason": null,
            "impossible_task": false,
            "reached_captcha": false
        });
        let seen = Arc::new(Mutex::new(Vec::new()));
        let callback_seen = Arc::clone(&seen);
        let mut agent = Agent::new(
            "finish with judged history",
            QueueModel::new(vec![done_output, judge_output]),
            MockSession::new(),
        );
        agent.register_done_callback(move |history| {
            let verdict = history
                .items
                .last()
                .and_then(|item| item.result.last())
                .and_then(|result| result.judgement.as_ref())
                .map(|judgement| judgement.verdict);
            callback_seen
                .lock()
                .expect("done callback log")
                .push(format!("items:{}:verdict:{verdict:?}", history.items.len()));
            Ok::<(), String>(())
        });

        agent.run(1).await.expect("agent run");

        assert_eq!(
            seen.lock().expect("seen log").as_slice(),
            ["items:1:verdict:Some(true)"]
        );
    }

    #[tokio::test]
    async fn agent_run_stops_before_model_when_stop_requested() {
        let mut agent = Agent::new("complete", QueueModel::new(vec![]), MockSession::new());

        agent.stop();
        let error = agent.run(1).await.expect_err("run should stop");

        assert!(matches!(
            error,
            AgentRunError::Stopped { reason } if reason == "stop requested"
        ));
        assert!(agent.is_stopped());
        assert!(agent.llm.requests.lock().expect("requests lock").is_empty());
        assert!(agent.executor.session().events().is_empty());
    }

    #[tokio::test]
    async fn agent_run_stops_before_model_when_should_stop_callback_requests_it() {
        let calls = Arc::new(Mutex::new(0usize));
        let mut agent = Agent::new("complete", QueueModel::new(vec![]), MockSession::new());
        let callback_calls = Arc::clone(&calls);
        agent.register_should_stop_callback(move || {
            let mut calls = callback_calls.lock().expect("callback calls lock");
            *calls += 1;
            Ok::<_, String>(*calls >= 1)
        });

        let error = agent.run(1).await.expect_err("run should stop");

        assert!(matches!(
            error,
            AgentRunError::Stopped { reason } if reason == "should_stop callback requested stop"
        ));
        assert!(agent.is_stopped());
        assert_eq!(*calls.lock().expect("calls lock"), 1);
        assert!(agent.llm.requests.lock().expect("requests lock").is_empty());
    }

    #[tokio::test]
    async fn agent_run_reports_callback_failures_without_side_effects() {
        let output = serde_json::json!({
            "current_state": {},
            "action": [
                {
                    "click": {
                        "index": 1
                    }
                }
            ]
        });
        let mut agent = Agent::with_settings(
            "click after callback",
            AgentSettings {
                use_judge: false,
                ..AgentSettings::default()
            },
            QueueModel::new(vec![output]),
            MockSession::new(),
        );
        agent.register_new_step_callback(|_state, _output, _step_number| {
            Err::<(), _>("callback refused step")
        });

        let error = agent.run(2).await.expect_err("callback should fail");

        assert!(matches!(
            error,
            AgentRunError::Callback {
                callback: "new_step",
                message
            } if message == "callback refused step"
        ));
        assert!(agent.history.items.is_empty());
        assert!(agent.executor.session().events().is_empty());
    }

    #[tokio::test]
    async fn agent_run_stops_on_done() {
        let output = serde_json::json!({
            "current_state": {},
            "action": [
                {
                    "done": {
                        "text": "complete",
                        "success": true,
                        "files_to_display": []
                    }
                }
            ]
        });
        let mut agent = Agent::new(
            "complete",
            QueueModel::new(vec![output]),
            MockSession::new(),
        );

        let history = agent.run(3).await.expect("agent run");

        assert_eq!(history.items.len(), 1);
        assert_eq!(history.final_result(), Some("complete"));
    }

    #[tokio::test]
    async fn agent_run_attaches_judge_result_to_done() {
        let done_output = serde_json::json!({
            "current_state": {},
            "action": [
                {
                    "done": {
                        "text": "complete",
                        "success": true,
                        "files_to_display": []
                    }
                }
            ]
        });
        let judge_output = serde_json::json!({
            "reasoning": "The task asked for stricter evidence.",
            "verdict": false,
            "failure_reason": "Missing required citation.",
            "impossible_task": false,
            "reached_captcha": false
        });
        let mut state = blank_state();
        state.screenshot = Some("judge-shot".to_owned());
        let settings = AgentSettings {
            ground_truth: Some("Must include a source citation.".to_owned()),
            ..AgentSettings::default()
        };
        let mut agent = Agent::with_settings(
            "complete with proof",
            settings,
            QueueModel::new(vec![done_output, judge_output]),
            MockSession::with_states(vec![state]),
        );

        let history = agent.run(1).await.expect("agent run").clone();

        assert_eq!(history.final_result(), Some("complete"));
        assert_eq!(history.is_successful(), Some(true));
        assert_eq!(history.is_validated(), Some(false));
        let judgement = history.judgement().expect("judgement");
        assert_eq!(
            judgement.failure_reason.as_deref(),
            Some("Missing required citation.")
        );

        let requests = agent.llm.requests.lock().expect("requests lock");
        assert_eq!(requests.len(), 2);
        let judge_text = request_text(&requests[1]);
        assert!(judge_text.contains("<ground_truth>"));
        assert!(judge_text.contains("Must include a source citation."));
        assert!(judge_text.contains("<final_result>\ncomplete\n</final_result>"));
        assert!(judge_text.contains("complete with proof"));
        assert!(
            requests[1].messages.iter().any(|message| {
                message.content.iter().any(|part| {
                    matches!(
                        part,
                        ContentPart::ImageUrl { image_url, .. }
                            if image_url == "data:image/png;base64,judge-shot"
                    )
                })
            }),
            "judge request should include recent screenshots"
        );
    }

    #[tokio::test]
    async fn agent_run_skips_judge_when_disabled() {
        let output = serde_json::json!({
            "current_state": {},
            "action": [
                {
                    "done": {
                        "text": "complete",
                        "success": true,
                        "files_to_display": []
                    }
                }
            ]
        });
        let settings = AgentSettings {
            use_judge: false,
            ..AgentSettings::default()
        };
        let mut agent = Agent::with_settings(
            "skip judge",
            settings,
            QueueModel::new(vec![output]),
            MockSession::new(),
        );

        let history = agent.run(1).await.expect("agent run").clone();

        assert_eq!(history.final_result(), Some("complete"));
        assert_eq!(history.judgement(), None);
        let requests = agent.llm.requests.lock().expect("requests lock");
        assert_eq!(requests.len(), 1);
    }

    #[tokio::test]
    async fn agent_run_compacts_history_between_steps() {
        let first_output = serde_json::json!({
            "current_state": {
                "memory": "Opened the checkout and started collecting receipt data",
                "next_goal": "Confirm the receipt details"
            },
            "action": [
                {
                    "wait": {
                        "seconds": 0
                    }
                }
            ]
        });
        let compaction_output = serde_json::json!({
            "summary": "Task: collect receipt data. IN-PROGRESS: checkout was opened but receipt details still need confirmation."
        });
        let done_output = serde_json::json!({
            "current_state": {
                "thinking": "done"
            },
            "action": [
                {
                    "done": {
                        "text": "receipt confirmed",
                        "success": true,
                        "files_to_display": []
                    }
                }
            ]
        });
        let settings = AgentSettings {
            use_judge: false,
            message_compaction: MessageCompaction::Settings(MessageCompactionSettings {
                compact_every_n_steps: 1,
                trigger_char_count: Some(1),
                keep_last_items: 1,
                summary_max_chars: 200,
                ..MessageCompactionSettings::default()
            }),
            ..AgentSettings::default()
        };
        let mut agent = Agent::with_settings(
            "collect receipt data",
            settings,
            QueueModel::new(vec![first_output, compaction_output, done_output]),
            MockSession::new(),
        );

        let history = agent.run(2).await.expect("agent run").clone();

        assert!(history.is_done());
        assert_eq!(
            history.compacted_memory.as_deref(),
            Some(
                "Task: collect receipt data. IN-PROGRESS: checkout was opened but receipt details still need confirmation."
            )
        );
        assert_eq!(history.compaction_count, 1);
        assert_eq!(history.last_compaction_step, Some(1));

        let checkpoint_json =
            serde_json::to_string_pretty(&agent.checkpoint()).expect("serialize checkpoint");
        let checkpoint: AgentCheckpoint =
            serde_json::from_str(&checkpoint_json).expect("deserialize checkpoint");
        assert_eq!(
            checkpoint.history.compacted_memory,
            history.compacted_memory
        );
        assert_eq!(checkpoint.history.compaction_count, 1);
        assert_eq!(checkpoint.history.last_compaction_step, Some(1));

        let requests = agent.llm.requests.lock().expect("requests lock");
        assert_eq!(requests.len(), 3);
        let compaction_text = request_text(&requests[1]);
        assert!(compaction_text.contains("Only mark a step as completed"));
        assert!(compaction_text.contains("<agent_history>"));
        assert!(compaction_text.contains("Opened the checkout"));
        assert!(compaction_text.contains("Waited for 0 seconds"));
        let second_step_prompt = request_text(&requests[2]);
        assert!(second_step_prompt.contains("<compacted_memory>"));
        assert!(second_step_prompt.contains("Treat as unverified context"));
        assert!(second_step_prompt.contains("receipt details still need confirmation"));
    }

    #[tokio::test]
    async fn agent_run_skips_message_compaction_when_disabled() {
        let wait_output = serde_json::json!({
            "current_state": {
                "memory": "Still working"
            },
            "action": [
                {
                    "wait": {
                        "seconds": 0
                    }
                }
            ]
        });
        let done_output = serde_json::json!({
            "current_state": {
                "thinking": "done"
            },
            "action": [
                {
                    "done": {
                        "text": "finished without compaction",
                        "success": true,
                        "files_to_display": []
                    }
                }
            ]
        });
        let settings = AgentSettings {
            use_judge: false,
            message_compaction: MessageCompaction::Disabled,
            ..AgentSettings::default()
        };
        let mut agent = Agent::with_settings(
            "skip compaction",
            settings,
            QueueModel::new(vec![wait_output, done_output]),
            MockSession::new(),
        );

        let history = agent.run(2).await.expect("agent run");

        assert!(history.is_done());
        assert_eq!(history.compacted_memory, None);
        assert_eq!(history.compaction_count, 0);
        assert_eq!(agent.llm.requests.lock().expect("requests lock").len(), 2);
    }

    #[test]
    fn generate_gif_output_path_preserves_upstream_default_and_custom_shapes() {
        assert_eq!(
            generate_gif_output_path(&GenerateGif::Enabled),
            Some(std::path::PathBuf::from("agent_history.gif"))
        );
        assert_eq!(
            generate_gif_output_path(&GenerateGif::Path("~/trace.gif".to_owned())),
            Some(expand_user_path("~/trace.gif"))
        );
        assert_eq!(generate_gif_output_path(&GenerateGif::Disabled), None);
    }

    #[tokio::test]
    async fn agent_run_writes_generate_gif_custom_path_from_screenshots() {
        let mut state = blank_state();
        state.screenshot = Some(test_png_base64(8, 8));
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let gif_path = temp_dir.path().join("history.gif");
        let output = serde_json::json!({
            "current_state": {},
            "action": [
                {
                    "done": {
                        "text": "with gif",
                        "success": true,
                        "files_to_display": []
                    }
                }
            ]
        });
        let settings = AgentSettings {
            use_judge: false,
            generate_gif: GenerateGif::Path(gif_path.display().to_string()),
            ..AgentSettings::default()
        };
        let mut agent = Agent::with_settings(
            "write gif",
            settings,
            QueueModel::new(vec![output]),
            MockSession::with_states(vec![state]),
        );

        agent.run(1).await.expect("agent run");

        let gif_bytes = std::fs::read(&gif_path).expect("gif bytes");
        assert!(gif_bytes.starts_with(b"GIF8"));
        assert!(gif_bytes.len() > 20);
    }

    #[tokio::test]
    async fn agent_run_does_not_write_generate_gif_without_screenshots() {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let gif_path = temp_dir.path().join("empty.gif");
        let output = serde_json::json!({
            "current_state": {},
            "action": [
                {
                    "done": {
                        "text": "no gif",
                        "success": true,
                        "files_to_display": []
                    }
                }
            ]
        });
        let settings = AgentSettings {
            use_judge: false,
            generate_gif: GenerateGif::Path(gif_path.display().to_string()),
            ..AgentSettings::default()
        };
        let mut agent = Agent::with_settings(
            "skip empty gif",
            settings,
            QueueModel::new(vec![output]),
            MockSession::new(),
        );

        agent.run(1).await.expect("agent run");

        assert!(!gif_path.exists());
    }

    #[tokio::test]
    async fn agent_run_ignores_message_compaction_failures() {
        let wait_output = serde_json::json!({
            "current_state": {
                "memory": "Collected partial evidence"
            },
            "action": [
                {
                    "wait": {
                        "seconds": 0
                    }
                }
            ]
        });
        let done_output = serde_json::json!({
            "current_state": {
                "thinking": "done"
            },
            "action": [
                {
                    "done": {
                        "text": "finished after failed compaction",
                        "success": true,
                        "files_to_display": []
                    }
                }
            ]
        });
        let settings = AgentSettings {
            use_judge: false,
            message_compaction: MessageCompaction::Settings(MessageCompactionSettings {
                compact_every_n_steps: 1,
                trigger_char_count: Some(1),
                ..MessageCompactionSettings::default()
            }),
            ..AgentSettings::default()
        };
        let mut agent = Agent::with_settings(
            "ignore compaction failure",
            settings,
            QueueModel::with_results(vec![
                Ok(wait_output),
                Err(LlmError::Provider("summary model unavailable".to_owned())),
                Ok(done_output),
            ]),
            MockSession::new(),
        );

        let history = agent.run(2).await.expect("agent run");

        assert!(history.is_done());
        assert_eq!(history.compacted_memory, None);
        assert_eq!(history.compaction_count, 0);
        let requests = agent.llm.requests.lock().expect("requests lock");
        assert_eq!(requests.len(), 3);
        assert!(request_text(&requests[1]).contains("<agent_history>"));
        assert!(!request_text(&requests[2]).contains("<compacted_memory>"));
    }

    #[tokio::test]
    async fn agent_setting_can_attach_done_files_without_displaying_text() {
        let output = serde_json::json!({
            "current_state": {},
            "action": [
                {
                    "done": {
                        "text": "Complete",
                        "success": true,
                        "files_to_display": ["report.md"]
                    }
                }
            ]
        });
        let settings = AgentSettings {
            display_files_in_done_text: false,
            ..AgentSettings::default()
        };
        let mut agent = Agent::with_settings(
            "finish with report attachment",
            settings,
            QueueModel::new(vec![output]),
            MockSession::new(),
        );
        agent
            .file_system_mut()
            .write_file(&WriteFileAction {
                file_name: "report.md".to_owned(),
                content: "alpha\nbeta".to_owned(),
                append: false,
                trailing_newline: true,
                leading_newline: false,
            })
            .expect("write report");

        let history = agent.run(1).await.expect("agent run");

        assert_eq!(history.final_result(), Some("Complete"));
        let result = &history.items[0].result[0];
        assert_eq!(result.extracted_content.as_deref(), Some("Complete"));
        assert_eq!(result.attachments.len(), 1);
        assert!(result.attachments[0].ends_with("browseruse_agent_data/report.md"));
    }

    #[tokio::test]
    async fn agent_run_recovers_from_invalid_model_output() {
        let invalid_output = serde_json::json!({
            "not_agent_output": true
        });
        let done_output = serde_json::json!({
            "current_state": {},
            "action": [
                {
                    "done": {
                        "text": "recovered",
                        "success": true,
                        "files_to_display": []
                    }
                }
            ]
        });
        let settings = AgentSettings {
            max_failures: 2,
            final_response_after_failure: false,
            ..AgentSettings::default()
        };
        let mut agent = Agent::with_settings(
            "recover",
            settings,
            QueueModel::new(vec![invalid_output, done_output]),
            MockSession::new(),
        );

        let history = agent.run(3).await.expect("agent run");

        assert_eq!(history.items.len(), 2);
        let first_metadata = history.items[0].metadata.as_ref().expect("first metadata");
        assert_eq!(first_metadata.step_number, 1);
        assert_eq!(first_metadata.step_interval, None);
        let second_metadata = history.items[1].metadata.as_ref().expect("second metadata");
        assert_eq!(second_metadata.step_number, 2);
        assert!(second_metadata.step_interval.unwrap_or(-1.0) >= 0.0);
        assert_eq!(history.final_result(), Some("recovered"));
        assert_eq!(history.is_successful(), Some(true));
        let errors = history.errors();
        assert_eq!(errors.len(), 2);
        let first_error = errors[0].expect("first step error");
        assert!(
            first_error.contains("invalid agent output"),
            "unexpected error: {}",
            first_error
        );
        assert_eq!(errors[1], None);
        assert!(history.has_errors());
    }

    #[tokio::test]
    async fn agent_run_recovers_from_provider_structured_output_errors() {
        let done_output = serde_json::json!({
            "current_state": {},
            "action": [
                {
                    "done": {
                        "text": "recovered from provider errors",
                        "success": true,
                        "files_to_display": []
                    }
                }
            ]
        });
        let settings = AgentSettings {
            max_failures: 4,
            final_response_after_failure: false,
            ..AgentSettings::default()
        };
        let mut agent = Agent::with_settings(
            "recover provider failures",
            settings,
            QueueModel::with_results(vec![
                Err(LlmError::InvalidStructuredOutput(
                    "chat completion tool call arguments: expected value".to_owned(),
                )),
                Err(LlmError::Provider(
                    "model refused request: safety policy".to_owned(),
                )),
                Err(LlmError::Provider(
                    "chat completion stopped with length before completing structured output"
                        .to_owned(),
                )),
                Ok(done_output),
            ]),
            MockSession::new(),
        );

        let history = agent.run(5).await.expect("agent run");

        assert_eq!(history.items.len(), 4);
        assert_eq!(
            history.final_result(),
            Some("recovered from provider errors")
        );
        let errors = history.errors();
        assert!(
            errors[0]
                .expect("malformed error")
                .contains("chat completion tool call arguments"),
            "unexpected error: {:?}",
            errors[0]
        );
        assert!(
            errors[1]
                .expect("refusal error")
                .contains("model refused request"),
            "unexpected error: {:?}",
            errors[1]
        );
        assert!(
            errors[2]
                .expect("truncation error")
                .contains("stopped with length"),
            "unexpected error: {:?}",
            errors[2]
        );
        assert_eq!(errors[3], None);
    }

    #[tokio::test]
    async fn agent_run_enforces_max_failures() {
        let output = serde_json::json!({
            "current_state": {},
            "action": [
                {
                    "click": {
                        "coordinate_x": 10
                    }
                }
            ]
        });
        let settings = AgentSettings {
            max_failures: 2,
            final_response_after_failure: false,
            ..AgentSettings::default()
        };
        let mut agent = Agent::with_settings(
            "find buttons",
            settings,
            QueueModel::new(vec![output.clone(), output]),
            MockSession::new(),
        );

        let error = agent.run(5).await.expect_err("max failures");

        assert!(matches!(
            error,
            AgentRunError::MaxFailuresExceeded { failures: 2 }
        ));
        assert_eq!(agent.history().items.len(), 2);
    }

    #[tokio::test]
    async fn agent_run_requests_final_response_after_max_failures() {
        let invalid_output = serde_json::json!({
            "not_agent_output": true
        });
        let final_done = serde_json::json!({
            "current_state": {},
            "action": [
                {
                    "done": {
                        "text": "could not finish, but here is what I found",
                        "success": false,
                        "files_to_display": []
                    }
                }
            ]
        });
        let settings = AgentSettings {
            max_failures: 2,
            use_judge: false,
            ..AgentSettings::default()
        };
        let mut agent = Agent::with_settings(
            "recover with final answer",
            settings,
            QueueModel::new(vec![invalid_output.clone(), invalid_output, final_done]),
            MockSession::new(),
        );

        let history = agent.run(5).await.expect("final response");

        assert_eq!(history.items.len(), 3);
        assert_eq!(
            history.final_result(),
            Some("could not finish, but here is what I found")
        );
        assert_eq!(history.is_successful(), Some(false));
        assert_eq!(history.errors().len(), 3);
        assert!(history.errors()[0].is_some());
        assert!(history.errors()[1].is_some());
        assert_eq!(history.errors()[2], None);

        let requests = agent.llm.requests.lock().expect("requests lock");
        assert_eq!(requests.len(), 3);
        let final_request_text = request_text(&requests[2]);
        assert!(final_request_text.contains("You failed 2 times"));
        assert!(final_request_text.contains("Your only available action is done"));
        assert!(final_request_text.contains("set success to false"));
    }

    #[tokio::test]
    async fn final_response_after_failure_rejects_non_done_actions_before_side_effects() {
        let invalid_output = serde_json::json!({
            "not_agent_output": true
        });
        let final_click = serde_json::json!({
            "current_state": {},
            "action": [
                {
                    "click": {
                        "index": 1
                    }
                }
            ]
        });
        let settings = AgentSettings {
            max_failures: 1,
            ..AgentSettings::default()
        };
        let mut agent = Agent::with_settings(
            "avoid side effects after failure",
            settings,
            QueueModel::new(vec![invalid_output, final_click]),
            MockSession::new(),
        );

        let error = agent.run(5).await.expect_err("max failures");

        assert!(matches!(
            error,
            AgentRunError::MaxFailuresExceeded { failures: 1 }
        ));
        assert!(agent.executor.session().events().is_empty());
        assert_eq!(agent.history().items.len(), 2);
        let errors = agent.history().errors();
        assert!(
            errors[1]
                .expect("final response error")
                .contains("exactly one done action")
        );
    }

    #[tokio::test]
    async fn agent_run_reports_step_limit() {
        let output = serde_json::json!({
            "current_state": {},
            "action": [
                {
                    "click": {
                        "index": 1
                    }
                }
            ]
        });
        let mut agent = Agent::new(
            "click once",
            QueueModel::new(vec![output]),
            MockSession::new(),
        );

        let error = agent.run(1).await.expect_err("step limit");

        assert!(matches!(
            error,
            AgentRunError::StepLimitReached { max_steps: 1 }
        ));
        assert_eq!(agent.history().items.len(), 1);
    }

    #[tokio::test]
    async fn agent_run_uses_final_step_done_response_to_finish() {
        let output = serde_json::json!({
            "current_state": {},
            "action": [
                {
                    "done": {
                        "text": "finished with final step",
                        "success": true,
                        "files_to_display": []
                    }
                }
            ]
        });
        let mut agent = Agent::new(
            "finish on final step",
            QueueModel::new(vec![output]),
            MockSession::new(),
        );

        let history = agent.run(1).await.expect("final step done");

        assert_eq!(history.final_result(), Some("finished with final step"));
        assert_eq!(history.items.len(), 1);
    }

    #[tokio::test]
    async fn agent_run_rejects_non_done_final_step_without_side_effects() {
        let output = serde_json::json!({
            "current_state": {},
            "action": [
                {
                    "click": {
                        "index": 1
                    }
                }
            ]
        });
        let mut agent = Agent::new(
            "do not click on final step",
            QueueModel::new(vec![output]),
            MockSession::new(),
        );

        let error = agent.run(1).await.expect_err("step limit");

        assert!(matches!(
            error,
            AgentRunError::StepLimitReached { max_steps: 1 }
        ));
        assert!(agent.executor.session().events().is_empty());
        assert_eq!(
            agent.history().errors()[0],
            Some("final response at step limit must return exactly one done action")
        );
    }

    #[tokio::test]
    async fn agent_run_injects_budget_warning_before_final_step_only() {
        let click_output = serde_json::json!({
            "current_state": {},
            "action": [
                {
                    "click": {
                        "index": 1
                    }
                }
            ]
        });
        let done_output = serde_json::json!({
            "current_state": {},
            "action": [
                {
                    "done": {
                        "text": "finished after warning",
                        "success": true,
                        "files_to_display": []
                    }
                }
            ]
        });
        let settings = AgentSettings {
            use_judge: false,
            ..AgentSettings::default()
        };
        let mut agent = Agent::with_settings(
            "warn before final step",
            settings,
            QueueModel::new(vec![
                click_output.clone(),
                click_output.clone(),
                click_output,
                done_output,
            ]),
            MockSession::new(),
        );

        agent.run(4).await.expect("final step done");

        let requests = agent.llm.requests.lock().expect("requests lock");
        assert_eq!(requests.len(), 4);
        assert!(!request_text(&requests[0]).contains("BUDGET WARNING"));
        assert!(!request_text(&requests[1]).contains("BUDGET WARNING"));
        assert!(
            request_text(&requests[2])
                .contains("BUDGET WARNING: You have used 3/4 steps (75%). 1 steps remaining.")
        );
        let final_request = request_text(&requests[3]);
        assert!(final_request.contains("You reached max_steps (4)"));
        assert!(!final_request.contains("BUDGET WARNING"));
    }

    #[tokio::test]
    async fn agent_run_detects_repeated_action_loop() {
        let output = serde_json::json!({
            "current_state": {},
            "action": [
                {
                    "click": {
                        "index": 1
                    }
                }
            ]
        });
        let settings = AgentSettings {
            loop_detection_window: 2,
            ..AgentSettings::default()
        };
        let mut agent = Agent::with_settings(
            "do not loop",
            settings,
            QueueModel::new(vec![output.clone(), output]),
            MockSession::new(),
        );

        let error = agent.run(5).await.expect_err("loop detected");

        assert!(matches!(error, AgentRunError::LoopDetected { window: 2 }));
        assert_eq!(agent.history().items.len(), 2);
    }

    #[tokio::test]
    async fn agent_loop_detection_can_be_disabled() {
        let output = serde_json::json!({
            "current_state": {},
            "action": [
                {
                    "click": {
                        "index": 1
                    }
                }
            ]
        });
        let settings = AgentSettings {
            loop_detection_window: 2,
            loop_detection_enabled: false,
            ..AgentSettings::default()
        };
        let mut agent = Agent::with_settings(
            "loop if allowed",
            settings,
            QueueModel::new(vec![output.clone(), output]),
            MockSession::new(),
        );

        let error = agent.run(2).await.expect_err("step limit");

        assert!(matches!(
            error,
            AgentRunError::StepLimitReached { max_steps: 2 }
        ));
        assert_eq!(agent.history().items.len(), 2);
    }

    #[test]
    fn repeated_action_loop_uses_normalized_action_signatures() {
        let history = AgentHistory {
            items: vec![
                history_item_with_actions(vec![BrowserAction::Search(
                    browser_use_tools::SearchAction {
                        query: "EvalOps browser-use".to_owned(),
                        engine: SearchEngine::Google,
                    },
                )]),
                history_item_with_actions(vec![BrowserAction::Search(
                    browser_use_tools::SearchAction {
                        query: "browser use EvalOps!!!".to_owned(),
                        engine: SearchEngine::Google,
                    },
                )]),
            ],
            ..AgentHistory::default()
        };

        assert!(repeated_action_loop(&history, 2));
    }

    #[test]
    fn repeated_action_loop_ignores_wait_only_steps() {
        let history = AgentHistory {
            items: vec![
                history_item_with_actions(vec![BrowserAction::Wait(
                    browser_use_tools::WaitAction { seconds: 3 },
                )]),
                history_item_with_actions(vec![BrowserAction::Wait(
                    browser_use_tools::WaitAction { seconds: 3 },
                )]),
            ],
            ..AgentHistory::default()
        };

        assert!(!repeated_action_loop(&history, 2));
    }

    #[test]
    fn step_request_includes_previous_results() {
        let mut state = blank_state();
        state.dom_state = SerializedDomState::from_elements(vec![
            browser_use_dom::DomElementRef {
                index: 1,
                target_id: "target".to_owned(),
                backend_node_id: 1,
                node_id: None,
                tag_name: "a".to_owned(),
                role: Some("link".to_owned()),
                name: Some("Docs".to_owned()),
                text: Some("Docs".to_owned()),
                attributes: BTreeMap::from([("href".to_owned(), "/docs".to_owned())]),
                bounds: None,
                is_visible: true,
                is_interactive: true,
                is_scrollable: false,
            },
            browser_use_dom::DomElementRef {
                index: 2,
                target_id: "target".to_owned(),
                backend_node_id: 2,
                node_id: None,
                tag_name: "div".to_owned(),
                role: None,
                name: Some("Results".to_owned()),
                text: None,
                attributes: BTreeMap::new(),
                bounds: None,
                is_visible: true,
                is_interactive: true,
                is_scrollable: true,
            },
        ])
        .with_page_stats(browser_use_dom::DomPageStats {
            links: 1,
            iframes: 1,
            shadow_open: 1,
            shadow_closed: 0,
            scroll_containers: 1,
            images: 3,
            interactive_elements: 2,
            total_elements: 30,
            text_chars: 120,
        });
        state.tabs = vec![browser_use_dom::TabInfo {
            url: "https://example.com/docs".to_owned(),
            title: "Docs".to_owned(),
            tab_id: browser_use_dom::TabInfo::tab_id_for_target("target-1234abcd"),
            target_id: "target-1234abcd".to_owned(),
            parent_target_id: None,
        }];
        let history = AgentHistory {
            items: vec![AgentHistoryItem {
                model_output: None,
                result: vec![ActionResult::extracted("Clicked element 1")],
                state: blank_state(),
                metadata: None,
            }],
            ..AgentHistory::default()
        };

        let request = build_step_request("keep going", &state, &history, &AgentSettings::default())
            .expect("step request");
        let request_text = serde_json::to_string(&request.messages).expect("messages json");
        let user_message = request
            .messages
            .iter()
            .find(|message| message.role == MessageRole::User)
            .expect("user message");
        let user_text = match &user_message.content[0] {
            ContentPart::Text { text } => text,
            other => panic!("unexpected first content part: {other:?}"),
        };

        assert!(user_text.contains("<agent_history>"));
        assert!(user_text.contains("<step>\nResult\nClicked element 1"));
        assert!(user_text.contains("</agent_history>"));
        assert!(user_text.contains("<agent_state>"));
        assert!(user_text.contains("Page stats"));
        assert!(user_text.contains("1 links, 2 interactive, 1 iframes"));
        assert!(user_text.contains("1 shadow(open), 0 shadow(closed)"));
        assert!(user_text.contains("3 images"));
        assert!(user_text.contains("1 scroll containers"));
        assert!(user_text.contains("30 total elements, 120 text chars"));
        assert!(user_text.contains("</agent_state>"));
        assert!(user_text.contains("<browser_state>"));
        assert!(user_text.contains("</browser_state>"));
        assert!(user_text.contains(r#""tab_id": "abcd""#));
        assert!(request_text.contains("Avoid repeating the same action sequence"));
    }

    #[test]
    fn step_request_includes_loading_page_stats_hint_like_upstream() {
        let mut state = blank_state();
        state.dom_state =
            SerializedDomState::default().with_page_stats(browser_use_dom::DomPageStats {
                links: 0,
                iframes: 0,
                shadow_open: 0,
                shadow_closed: 0,
                scroll_containers: 0,
                images: 0,
                interactive_elements: 0,
                total_elements: 25,
                text_chars: 10,
            });
        let request = build_step_request(
            "wait for app",
            &state,
            &AgentHistory::default(),
            &AgentSettings::default(),
        )
        .expect("step request");
        let user_message = request
            .messages
            .iter()
            .find(|message| message.role == MessageRole::User)
            .expect("user message");
        let user_text = match &user_message.content[0] {
            ContentPart::Text { text } => text,
            other => panic!("unexpected first content part: {other:?}"),
        };

        assert!(
            user_text
                .contains("Page appears to show skeleton/placeholder content (still loading?)")
        );
        assert!(user_text.contains("25 total elements"));
    }

    #[test]
    fn step_request_omits_recent_events_by_default() {
        let mut state = blank_state();
        state.recent_events = Some("Blocked popup https://tracker.example.test".to_owned());

        let request = build_step_request(
            "inspect page",
            &state,
            &AgentHistory::default(),
            &AgentSettings::default(),
        )
        .expect("step request");
        let user_text = request_text(&request);

        assert!(!user_text.contains("recent_events"));
        assert!(!user_text.contains("tracker.example.test"));
    }

    #[test]
    fn step_request_includes_recent_events_when_enabled() {
        let mut state = blank_state();
        state.recent_events = Some("Blocked popup https://tracker.example.test".to_owned());
        let settings = AgentSettings {
            include_recent_events: true,
            ..AgentSettings::default()
        };

        let request =
            build_step_request("inspect page", &state, &AgentHistory::default(), &settings)
                .expect("step request");
        let user_text = request_text(&request);

        assert!(
            user_text.contains(r#""recent_events": "Blocked popup https://tracker.example.test""#)
        );
    }

    #[test]
    fn step_request_includes_loop_awareness_nudge() {
        let mut state = blank_state();
        state.dom_state.text = "[1] <button> Refresh".to_owned();
        let history = AgentHistory {
            items: (0..5)
                .map(|_| AgentHistoryItem {
                    model_output: Some(AgentOutput {
                        current_state: AgentCurrentState {
                            thinking: None,
                            evaluation_previous_goal: None,
                            memory: None,
                            next_goal: None,
                        },
                        thinking: None,
                        evaluation_previous_goal: None,
                        memory: None,
                        next_goal: None,
                        current_plan_item: None,
                        plan_update: None,
                        action: vec![BrowserAction::Click(ClickElementAction {
                            index: Some(1),
                            coordinate_x: None,
                            coordinate_y: None,
                        })],
                    }),
                    result: vec![ActionResult::extracted("Clicked element 1")],
                    state: state.clone(),
                    metadata: None,
                })
                .collect(),
            ..AgentHistory::default()
        };

        let request = build_step_request("unstick", &state, &history, &AgentSettings::default())
            .expect("step request");
        let request_text = serde_json::to_string(&request.messages).expect("messages json");

        assert!(request_text.contains("Loop awareness"));
        assert!(request_text.contains("repeated a similar action 5 times"));
        assert!(request_text.contains("page content has not changed across 5"));

        let disabled_settings = AgentSettings {
            loop_detection_enabled: false,
            ..AgentSettings::default()
        };
        let request = build_step_request("unstick", &state, &history, &disabled_settings)
            .expect("step request");
        let request_text = serde_json::to_string(&request.messages).expect("messages json");
        assert!(!request_text.contains("Loop awareness"));
    }

    #[test]
    fn step_request_includes_planning_context_and_replan_nudge() {
        let state = blank_state();
        let history = AgentHistory {
            items: (0..3)
                .map(|_| AgentHistoryItem {
                    model_output: None,
                    result: vec![ActionResult::error("stalled")],
                    state: blank_state(),
                    metadata: None,
                })
                .collect(),
            ..AgentHistory::default()
        };

        let request = build_step_request("recover", &state, &history, &AgentSettings::default())
            .expect("step request");
        let request_text = serde_json::to_string(&request.messages).expect("messages json");

        assert!(request_text.contains("Planning"));
        assert!(request_text.contains("current_plan_item"));
        assert!(request_text.contains("plan_update"));
        assert!(request_text.contains("revise the plan"));

        let disabled_settings = AgentSettings {
            enable_planning: false,
            ..AgentSettings::default()
        };
        let request = build_step_request("recover", &state, &history, &disabled_settings)
            .expect("step request");
        let request_text = serde_json::to_string(&request.messages).expect("messages json");
        assert!(!request_text.contains("Planning"));

        let flash_settings = AgentSettings {
            flash_mode: true,
            ..AgentSettings::default()
        };
        let request =
            build_step_request("recover", &state, &history, &flash_settings).expect("step request");
        let request_text = serde_json::to_string(&request.messages).expect("messages json");
        assert!(!request_text.contains("Planning"));
    }

    #[test]
    fn step_request_includes_available_file_paths_like_upstream() {
        let settings = AgentSettings {
            available_file_paths: vec!["/tmp/report.pdf".to_owned(), "/tmp/chart.png".to_owned()],
            ..AgentSettings::default()
        };

        let request = build_step_request(
            "inspect files",
            &blank_state(),
            &AgentHistory::default(),
            &settings,
        )
        .expect("step request");
        let user_message = request
            .messages
            .iter()
            .find(|message| message.role == MessageRole::User)
            .expect("user message");
        let user_text = match &user_message.content[0] {
            ContentPart::Text { text } => text,
            other => panic!("unexpected first content part: {other:?}"),
        };

        assert!(user_text.contains(
            "<available_file_paths>/tmp/report.pdf\n/tmp/chart.png\nUse with absolute paths</available_file_paths>"
        ));
        assert!(user_text.contains("<agent_state>"));
        assert!(user_text.contains("</agent_state>"));
    }

    #[test]
    fn step_request_includes_managed_file_system_context() {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let mut file_system = ManagedFileSystem::new(temp_dir.path()).expect("managed file system");
        let empty_request = build_step_request_with_file_system(
            "use the files",
            &blank_state(),
            &AgentHistory::default(),
            &AgentSettings::default(),
            Some(&file_system),
        )
        .expect("empty file system request");
        let empty_text = request_text(&empty_request);

        assert!(empty_text.contains("<file_system>\n\n</file_system>"));
        assert!(empty_text.contains(
            "<todo_contents>\n[empty todo.md, fill it when applicable]\n</todo_contents>"
        ));

        file_system
            .write_file(&WriteFileAction {
                file_name: "todo.md".to_owned(),
                content: "- inspect report".to_owned(),
                append: false,
                trailing_newline: false,
                leading_newline: false,
            })
            .expect("write todo");
        file_system
            .write_file(&WriteFileAction {
                file_name: "report.md".to_owned(),
                content: "alpha\nbeta".to_owned(),
                append: false,
                trailing_newline: false,
                leading_newline: false,
            })
            .expect("write report");

        let request = build_step_request_with_file_system(
            "use the files",
            &blank_state(),
            &AgentHistory::default(),
            &AgentSettings::default(),
            Some(&file_system),
        )
        .expect("file system request");
        let text = request_text(&request);

        assert!(text.contains("<file_system>\n<file>\nreport.md - 2 lines"));
        assert!(text.contains("<content>\nalpha\nbeta\n</content>"));
        assert!(text.contains("<todo_contents>\n- inspect report\n</todo_contents>"));
        assert!(!text.contains("todo.md -"));
    }

    #[test]
    fn large_extract_results_save_to_managed_file_system() {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let mut file_system = ManagedFileSystem::new(temp_dir.path()).expect("managed file system");
        let page_text = "EvalOps extracted content. ".repeat(700);

        let result = extract_action_result(
            &ExtractAction {
                query: "summarize".to_owned(),
                extract_links: false,
                extract_images: false,
                start_from_char: 0,
                output_schema: None,
                already_collected: Vec::new(),
            },
            &page_text,
            Some("https://example.test/report"),
            false,
            None,
            None,
            Some(&mut file_system),
        );

        assert!(
            result
                .long_term_memory
                .as_deref()
                .expect("memory")
                .contains("Content in extracted_content_0.md and once in <read_state>.")
        );
        let saved = file_system
            .display_file("extracted_content_0.md")
            .expect("saved extracted content");
        assert!(saved.contains("<query>\nsummarize\n</query>"));
        assert!(saved.contains("EvalOps extracted content."));
        assert_eq!(file_system.get_state().extracted_content_count, 1);
    }

    #[test]
    fn step_request_honors_system_message_override_and_extension() {
        let extended_settings = AgentSettings {
            max_actions_per_step: 2,
            extend_system_message: Some("Prefer stable selectors when possible.".to_owned()),
            ..AgentSettings::default()
        };
        let request = build_step_request(
            "inspect",
            &blank_state(),
            &AgentHistory::default(),
            &extended_settings,
        )
        .expect("step request");
        let system_text = request
            .messages
            .iter()
            .find(|message| message.role == MessageRole::System)
            .and_then(|message| match &message.content[0] {
                ContentPart::Text { text } => Some(text.as_str()),
                ContentPart::ImageUrl { .. } => None,
            })
            .expect("system message");

        assert!(system_text.contains("Use at most 2 actions"));
        assert!(system_text.contains("Prefer stable selectors when possible."));

        let override_settings = AgentSettings {
            override_system_message: Some("Return only the agreed JSON contract.".to_owned()),
            extend_system_message: Some("Prefer compact actions.".to_owned()),
            ..AgentSettings::default()
        };
        let request = build_step_request(
            "inspect",
            &blank_state(),
            &AgentHistory::default(),
            &override_settings,
        )
        .expect("step request");
        let system_text = request
            .messages
            .iter()
            .find(|message| message.role == MessageRole::System)
            .and_then(|message| match &message.content[0] {
                ContentPart::Text { text } => Some(text.as_str()),
                ContentPart::ImageUrl { .. } => None,
            })
            .expect("system message");

        assert_eq!(
            system_text,
            "Return only the agreed JSON contract.\nPrefer compact actions."
        );
    }

    #[test]
    fn step_request_includes_sensitive_data_placeholders_like_upstream() {
        let mut state = blank_state();
        state.url = "https://secure.example.test/login".to_owned();
        let settings = AgentSettings {
            sensitive_data: BTreeMap::from([
                (
                    "*.example.test".to_owned(),
                    SensitiveDataValue::Domain(BTreeMap::from([(
                        "password".to_owned(),
                        "super-secret".to_owned(),
                    )])),
                ),
                (
                    "username".to_owned(),
                    SensitiveDataValue::Value("evalops@example.test".to_owned()),
                ),
            ]),
            ..AgentSettings::default()
        };

        let request = build_step_request("log in", &state, &AgentHistory::default(), &settings)
            .expect("step request");
        let user_message = request
            .messages
            .iter()
            .find(|message| message.role == MessageRole::User)
            .expect("user message");
        let user_text = match &user_message.content[0] {
            ContentPart::Text { text } => text,
            other => panic!("unexpected first content part: {other:?}"),
        };

        assert!(user_text.contains("<sensitive_data>SENSITIVE DATA"));
        assert!(user_text.contains("  - password"));
        assert!(user_text.contains("  - username"));
        assert!(user_text.contains("use: <secret>password</secret>"));
        assert!(!user_text.contains("super-secret"));
        assert!(!user_text.contains("evalops@example.test"));
    }

    #[test]
    fn step_request_filters_domain_scoped_sensitive_data_by_url() {
        let mut state = blank_state();
        state.url = "https://other.example.test/login".to_owned();
        let settings = AgentSettings {
            sensitive_data: BTreeMap::from([
                (
                    "secure.example.test".to_owned(),
                    SensitiveDataValue::Domain(BTreeMap::from([(
                        "password".to_owned(),
                        "super-secret".to_owned(),
                    )])),
                ),
                (
                    "username".to_owned(),
                    SensitiveDataValue::Value("evalops@example.test".to_owned()),
                ),
            ]),
            ..AgentSettings::default()
        };

        let request = build_step_request("log in", &state, &AgentHistory::default(), &settings)
            .expect("step request");
        let user_text = request_text(&request);

        assert!(user_text.contains("<sensitive_data>SENSITIVE DATA"));
        assert!(user_text.contains("  - username"));
        assert!(!user_text.contains("  - password"));
        assert!(!user_text.contains("super-secret"));
        assert!(!user_text.contains("evalops@example.test"));
    }

    #[test]
    fn sensitive_data_domain_pattern_matching_follows_upstream_security_defaults() {
        assert!(match_url_with_domain_pattern(
            "https://secure.example.test/login",
            "secure.example.test"
        ));
        assert!(!match_url_with_domain_pattern(
            "http://secure.example.test/login",
            "secure.example.test"
        ));
        assert!(match_url_with_domain_pattern(
            "http://secure.example.test/login",
            "http*://secure.example.test"
        ));
        assert!(match_url_with_domain_pattern(
            "https://child.example.test",
            "*.example.test"
        ));
        assert!(match_url_with_domain_pattern(
            "https://example.test",
            "*.example.test"
        ));
        assert!(match_url_with_domain_pattern(
            "chrome-extension://aaaaaaaaaaaa/options",
            "chrome-extension://*"
        ));
        assert!(!match_url_with_domain_pattern("about:blank", "*"));
        assert!(!match_url_with_domain_pattern(
            "https://deep.example.test",
            "*.*.example.test"
        ));
    }

    #[test]
    fn step_request_redacts_sensitive_values_from_state_and_history() {
        let mut state = blank_state();
        state.url = "https://secure.example.test/login".to_owned();
        state.dom_state.text = "Password field currently contains super-secret".to_owned();
        let history = AgentHistory {
            items: vec![AgentHistoryItem {
                model_output: None,
                result: vec![ActionResult::extracted(
                    "Previous action saw token sk-live-123",
                )],
                state: blank_state(),
                metadata: None,
            }],
            ..AgentHistory::default()
        };
        let settings = AgentSettings {
            sensitive_data: BTreeMap::from([
                (
                    "password".to_owned(),
                    SensitiveDataValue::Value("super-secret".to_owned()),
                ),
                (
                    "api_key".to_owned(),
                    SensitiveDataValue::Value("sk-live-123".to_owned()),
                ),
            ]),
            ..AgentSettings::default()
        };

        let request =
            build_step_request("continue", &state, &history, &settings).expect("step request");
        let user_text = request_text(&request);

        assert!(user_text.contains("<secret>password</secret>"));
        assert!(user_text.contains("<secret>api_key</secret>"));
        assert!(!user_text.contains("super-secret"));
        assert!(!user_text.contains("sk-live-123"));
    }

    #[test]
    fn step_request_attaches_screenshot_as_image_part() {
        let mut state = blank_state();
        state.screenshot = Some("abc123".to_owned());

        let request = build_step_request(
            "inspect screenshot",
            &state,
            &AgentHistory::default(),
            &AgentSettings::default(),
        )
        .expect("step request");
        let user_message = request
            .messages
            .iter()
            .find(|message| message.role == MessageRole::User)
            .expect("user message");

        assert!(matches!(
            &user_message.content[1],
            ContentPart::ImageUrl { image_url, detail } if image_url == "data:image/png;base64,abc123"
                && *detail == Some(ImageDetailLevel::Auto)
        ));
        let text = match &user_message.content[0] {
            ContentPart::Text { text } => text,
            other => panic!("unexpected first content part: {other:?}"),
        };
        assert!(text.contains("<browser_state>"));
        assert!(!text.contains("abc123"));
    }

    #[test]
    fn step_request_attaches_latest_action_result_images_as_image_parts() {
        let history = AgentHistory {
            items: vec![
                AgentHistoryItem {
                    model_output: None,
                    result: vec![ActionResult {
                        extracted_content: Some("old image".to_owned()),
                        error: None,
                        judgement: None,
                        long_term_memory: Some("old image".to_owned()),
                        include_extracted_content_only_once: true,
                        include_in_memory: true,
                        is_done: false,
                        success: None,
                        attachments: Vec::new(),
                        images: vec![serde_json::json!({
                            "name": "old.jpg",
                            "data": "old-data",
                        })],
                        metadata: None,
                    }],
                    state: blank_state(),
                    metadata: None,
                },
                AgentHistoryItem {
                    model_output: None,
                    result: vec![ActionResult {
                        extracted_content: Some("Read image file chart.png.".to_owned()),
                        error: None,
                        judgement: None,
                        long_term_memory: Some("Read image file chart.png".to_owned()),
                        include_extracted_content_only_once: true,
                        include_in_memory: true,
                        is_done: false,
                        success: None,
                        attachments: Vec::new(),
                        images: vec![serde_json::json!({
                            "name": "chart.png",
                            "data": "abc123",
                        })],
                        metadata: None,
                    }],
                    state: blank_state(),
                    metadata: None,
                },
            ],
            ..AgentHistory::default()
        };
        let settings = AgentSettings {
            use_vision: VisionMode::Never,
            vision_detail_level: ImageDetailLevel::High,
            ..AgentSettings::default()
        };

        let request = build_step_request("inspect file image", &blank_state(), &history, &settings)
            .expect("step request");
        let user_message = request
            .messages
            .iter()
            .find(|message| message.role == MessageRole::User)
            .expect("user message");

        assert!(matches!(
            &user_message.content[1],
            ContentPart::Text { text } if text == "Image from file: chart.png"
        ));
        assert!(matches!(
            &user_message.content[2],
            ContentPart::ImageUrl { image_url, detail } if image_url == "data:image/png;base64,abc123"
                && *detail == Some(ImageDetailLevel::High)
        ));
        let request_text = serde_json::to_string(user_message).expect("message json");
        assert!(!request_text.contains("old-data"));
    }

    #[test]
    fn step_request_inserts_sample_images_before_runtime_images() {
        let mut state = blank_state();
        state.screenshot = Some("screen-data".to_owned());
        let history = AgentHistory {
            items: vec![AgentHistoryItem {
                model_output: None,
                result: vec![ActionResult {
                    extracted_content: Some("Read image file chart.png.".to_owned()),
                    error: None,
                    judgement: None,
                    long_term_memory: Some("Read image file chart.png".to_owned()),
                    include_extracted_content_only_once: true,
                    include_in_memory: true,
                    is_done: false,
                    success: None,
                    attachments: Vec::new(),
                    images: vec![serde_json::json!({
                        "name": "chart.png",
                        "data": "chart-data",
                    })],
                    metadata: None,
                }],
                state: blank_state(),
                metadata: None,
            }],
            ..AgentHistory::default()
        };
        let settings = AgentSettings {
            vision_detail_level: ImageDetailLevel::High,
            sample_images: vec![
                ContentPart::Text {
                    text: "Sample reference:".to_owned(),
                },
                ContentPart::ImageUrl {
                    image_url: "data:image/png;base64,sample-data".to_owned(),
                    detail: Some(ImageDetailLevel::Low),
                },
            ],
            ..AgentSettings::default()
        };

        let request = build_step_request("inspect samples", &state, &history, &settings)
            .expect("step request");
        let user_message = request
            .messages
            .iter()
            .find(|message| message.role == MessageRole::User)
            .expect("user message");

        assert!(matches!(
            &user_message.content[0],
            ContentPart::Text { text } if text.contains("<browser_state>")
        ));
        assert!(matches!(
            &user_message.content[1],
            ContentPart::Text { text } if text == "Sample reference:"
        ));
        assert!(matches!(
            &user_message.content[2],
            ContentPart::ImageUrl { image_url, detail }
                if image_url == "data:image/png;base64,sample-data"
                    && *detail == Some(ImageDetailLevel::Low)
        ));
        assert!(matches!(
            &user_message.content[3],
            ContentPart::ImageUrl { image_url, detail }
                if image_url == "data:image/png;base64,screen-data"
                    && *detail == Some(ImageDetailLevel::High)
        ));
        assert!(matches!(
            &user_message.content[4],
            ContentPart::Text { text } if text == "Image from file: chart.png"
        ));
        assert!(matches!(
            &user_message.content[5],
            ContentPart::ImageUrl { image_url, detail }
                if image_url == "data:image/png;base64,chart-data"
                    && *detail == Some(ImageDetailLevel::High)
        ));
    }

    #[test]
    fn step_request_uses_custom_dom_include_attributes() {
        let mut state = blank_state();
        state.dom_state = SerializedDomState::from_elements(vec![browser_use_dom::DomElementRef {
            index: 1,
            target_id: "target".to_owned(),
            backend_node_id: 1,
            node_id: None,
            tag_name: "button".to_owned(),
            role: None,
            name: Some("Run".to_owned()),
            text: None,
            attributes: BTreeMap::from([
                ("data-testid".to_owned(), "run-action".to_owned()),
                ("id".to_owned(), "run".to_owned()),
            ]),
            bounds: None,
            is_visible: true,
            is_interactive: true,
            is_scrollable: false,
        }]);
        let settings = AgentSettings {
            include_attributes: vec!["data-testid".to_owned()],
            ..AgentSettings::default()
        };

        let request = build_step_request("click run", &state, &AgentHistory::default(), &settings)
            .expect("step request");
        let user_message = request
            .messages
            .iter()
            .find(|message| message.role == MessageRole::User)
            .expect("user message");
        let text = match &user_message.content[0] {
            ContentPart::Text { text } => text,
            other => panic!("unexpected first content part: {other:?}"),
        };

        assert!(text.contains(r#""text": "[1] <button data-testid=run-action> Run""#));
        assert!(!text.contains(r#""text": "[1] <button id=run> Run""#));
    }

    #[test]
    fn step_request_truncates_large_clickable_element_text() {
        let mut state = blank_state();
        state.dom_state.text = "abcdef".repeat(20);
        let settings = AgentSettings {
            max_clickable_elements_length: 12,
            ..AgentSettings::default()
        };

        let request = build_step_request("summarize", &state, &AgentHistory::default(), &settings)
            .expect("step request");
        let user_message = request
            .messages
            .iter()
            .find(|message| message.role == MessageRole::User)
            .expect("user message");
        let text = match &user_message.content[0] {
            ContentPart::Text { text } => text,
            other => panic!("unexpected first content part: {other:?}"),
        };

        assert!(text.contains("abcdefabcdef"));
        assert!(text.contains("[clickable elements truncated to 12 chars]"));
        assert!(!text.contains("abcdefabcdefabcdef"));
    }

    #[test]
    fn step_request_omits_screenshot_when_vision_disabled() {
        let mut state = blank_state();
        state.screenshot = Some("abc123".to_owned());
        let settings = AgentSettings {
            use_vision: VisionMode::Never,
            ..AgentSettings::default()
        };

        let request =
            build_step_request("no screenshot", &state, &AgentHistory::default(), &settings)
                .expect("step request");
        let user_message = request
            .messages
            .iter()
            .find(|message| message.role == MessageRole::User)
            .expect("user message");

        assert_eq!(user_message.content.len(), 1);
        let request_text = serde_json::to_string(user_message).expect("message json");
        assert!(!request_text.contains("abc123"));
    }

    #[test]
    fn step_request_includes_screenshot_when_auto_vision_state_has_one() {
        let mut state = blank_state();
        state.screenshot = Some("abc123".to_owned());
        let settings = AgentSettings {
            use_vision: VisionMode::Auto,
            ..AgentSettings::default()
        };

        let request = build_step_request(
            "include requested screenshot",
            &state,
            &AgentHistory::default(),
            &settings,
        )
        .expect("step request");
        let user_message = request
            .messages
            .iter()
            .find(|message| message.role == MessageRole::User)
            .expect("user message");

        assert!(matches!(
            &user_message.content[1],
            ContentPart::ImageUrl { image_url, detail }
                if image_url == "data:image/png;base64,abc123"
                    && *detail == Some(ImageDetailLevel::Auto)
        ));
    }

    #[test]
    fn previous_results_only_include_one_time_extractions_once() {
        let history = AgentHistory {
            items: vec![
                AgentHistoryItem {
                    model_output: None,
                    result: vec![ActionResult {
                        extracted_content: Some("very large extracted payload".to_owned()),
                        error: None,
                        judgement: None,
                        long_term_memory: Some("Large extraction saved to file".to_owned()),
                        include_extracted_content_only_once: true,
                        include_in_memory: true,
                        is_done: false,
                        success: None,
                        attachments: Vec::new(),
                        images: Vec::new(),
                        metadata: None,
                    }],
                    state: blank_state(),
                    metadata: None,
                },
                AgentHistoryItem {
                    model_output: None,
                    result: vec![ActionResult::extracted("Clicked element 1")],
                    state: blank_state(),
                    metadata: None,
                },
            ],
            ..AgentHistory::default()
        };

        let rendered = render_previous_results(&history, None);

        assert!(!rendered.contains("very large extracted payload"));
        assert!(rendered.contains("Large extraction saved to file"));
        assert!(rendered.contains("Clicked element 1"));
    }

    #[test]
    fn latest_one_time_extraction_moves_to_read_state_section() {
        let history = AgentHistory {
            items: vec![AgentHistoryItem {
                model_output: None,
                result: vec![ActionResult {
                    extracted_content: Some("fresh extracted payload".to_owned()),
                    error: None,
                    judgement: None,
                    long_term_memory: Some("Extraction saved to file".to_owned()),
                    include_extracted_content_only_once: true,
                    include_in_memory: true,
                    is_done: false,
                    success: None,
                    attachments: Vec::new(),
                    images: Vec::new(),
                    metadata: None,
                }],
                state: blank_state(),
                metadata: None,
            }],
            ..AgentHistory::default()
        };

        let rendered = render_previous_results(&history, None);
        let read_state = render_read_state_description(&history).expect("read state");

        assert!(!rendered.contains("fresh extracted payload"));
        assert!(rendered.contains("Result\nExtraction saved to file"));
        assert_eq!(
            read_state,
            "<read_state_0>\nfresh extracted payload\n</read_state_0>"
        );

        let request = build_step_request(
            "inspect read state",
            &blank_state(),
            &history,
            &AgentSettings::default(),
        )
        .expect("step request");
        let user_message = request
            .messages
            .iter()
            .find(|message| message.role == MessageRole::User)
            .expect("user message");
        let user_text = match &user_message.content[0] {
            ContentPart::Text { text } => text,
            other => panic!("unexpected first content part: {other:?}"),
        };
        assert!(user_text.contains(
            "<read_state>\n<read_state_0>\nfresh extracted payload\n</read_state_0>\n</read_state>"
        ));
        assert!(user_text.contains(
            "<agent_history>\n<step>\nResult\nExtraction saved to file\n</agent_history>"
        ));
    }

    #[test]
    fn latest_read_state_blocks_are_numbered_like_upstream() {
        let history = AgentHistory {
            items: vec![AgentHistoryItem {
                model_output: None,
                result: vec![
                    ActionResult {
                        extracted_content: Some("first payload".to_owned()),
                        error: None,
                        judgement: None,
                        long_term_memory: Some("first summary".to_owned()),
                        include_extracted_content_only_once: true,
                        include_in_memory: true,
                        is_done: false,
                        success: None,
                        attachments: Vec::new(),
                        images: Vec::new(),
                        metadata: None,
                    },
                    ActionResult {
                        extracted_content: Some("second payload".to_owned()),
                        error: None,
                        judgement: None,
                        long_term_memory: Some("second summary".to_owned()),
                        include_extracted_content_only_once: true,
                        include_in_memory: true,
                        is_done: false,
                        success: None,
                        attachments: Vec::new(),
                        images: Vec::new(),
                        metadata: None,
                    },
                ],
                state: blank_state(),
                metadata: None,
            }],
            ..AgentHistory::default()
        };

        let rendered = render_previous_results(&history, None);
        let read_state = render_read_state_description(&history).expect("read state");

        assert!(!rendered.contains("first payload"));
        assert!(!rendered.contains("second payload"));
        assert!(rendered.contains("first summary"));
        assert!(rendered.contains("second summary"));
        assert!(read_state.contains("<read_state_0>\nfirst payload\n</read_state_0>"));
        assert!(read_state.contains("<read_state_1>\nsecond payload\n</read_state_1>"));
    }

    #[test]
    fn previous_results_include_all_history_when_limit_is_absent_like_upstream() {
        let history = AgentHistory {
            items: ["first", "second", "third", "fourth", "fifth", "sixth"]
                .into_iter()
                .map(|text| AgentHistoryItem {
                    model_output: None,
                    result: vec![ActionResult::extracted(text)],
                    state: blank_state(),
                    metadata: None,
                })
                .collect(),
            ..AgentHistory::default()
        };

        let rendered = render_previous_results(&history, None);

        assert!(rendered.contains("Result\nfirst"));
        assert!(rendered.contains("Result\nsixth"));
        assert!(!rendered.contains("previous steps omitted"));
    }

    #[test]
    fn previous_results_prefix_compacted_memory_like_upstream() {
        let history = AgentHistory {
            compacted_memory: Some(
                "Earlier summary; checkout was started but not confirmed.".to_owned(),
            ),
            compaction_count: 1,
            last_compaction_step: Some(25),
            items: vec![AgentHistoryItem {
                model_output: None,
                result: vec![ActionResult::extracted("Latest verified step")],
                state: blank_state(),
                metadata: None,
            }],
        };

        let rendered = render_previous_results(&history, Some(1));

        assert!(rendered.starts_with("<compacted_memory>"));
        assert!(rendered.contains("Treat as unverified context"));
        assert!(rendered.contains("Earlier summary"));
        assert!(rendered.contains("</compacted_memory>\n<step>\nResult\nLatest verified step"));
    }

    #[test]
    fn previous_results_include_model_brain_fields_like_upstream() {
        let history = AgentHistory {
            items: vec![AgentHistoryItem {
                model_output: Some(AgentOutput {
                    current_state: AgentCurrentState::default(),
                    thinking: None,
                    evaluation_previous_goal: Some("Previous goal succeeded".to_owned()),
                    memory: Some("Remembered page context".to_owned()),
                    next_goal: Some("Click next result".to_owned()),
                    current_plan_item: None,
                    plan_update: None,
                    action: vec![BrowserAction::Wait(browser_use_tools::WaitAction {
                        seconds: 1,
                    })],
                }),
                result: vec![ActionResult::extracted("Waited for 1 seconds")],
                state: blank_state(),
                metadata: None,
            }],
            ..AgentHistory::default()
        };

        let rendered = render_previous_results(&history, None);

        assert!(rendered.starts_with("<step>\nPrevious goal succeeded"));
        assert!(rendered.contains("Remembered page context"));
        assert!(rendered.contains("Click next result"));
        assert!(rendered.contains("Result\nWaited for 1 seconds"));
    }

    #[test]
    fn previous_results_limit_preserves_initial_and_recent_tail_like_upstream() {
        let history = AgentHistory {
            items: ["first", "second", "third"]
                .into_iter()
                .map(|text| AgentHistoryItem {
                    model_output: None,
                    result: vec![ActionResult::extracted(text)],
                    state: blank_state(),
                    metadata: None,
                })
                .collect(),
            ..AgentHistory::default()
        };

        let rendered = render_previous_results(&history, Some(2));

        assert!(rendered.contains("Result\nfirst"));
        assert!(rendered.contains("<sys>[... 1 previous steps omitted...]</sys>"));
        assert!(!rendered.contains("Result\nsecond"));
        assert!(rendered.contains("Result\nthird"));
    }

    #[test]
    fn previous_results_truncate_long_errors_like_upstream() {
        let error = format!("{}{}", "a".repeat(120), "b".repeat(120));
        let history = AgentHistory {
            items: vec![AgentHistoryItem {
                model_output: None,
                result: vec![ActionResult::error(error.clone())],
                state: blank_state(),
                metadata: None,
            }],
            ..AgentHistory::default()
        };

        let rendered = render_previous_results(&history, None);
        let expected = format!("{}......{}", "a".repeat(100), "b".repeat(100));

        assert!(rendered.contains(&expected));
        assert!(!rendered.contains(&error));
    }

    #[test]
    fn previous_results_truncate_large_prompt_context_like_upstream() {
        let huge_result = "x".repeat(MAX_PROMPT_CONTENT_CHARS + 100);
        let history = AgentHistory {
            items: vec![AgentHistoryItem {
                model_output: None,
                result: vec![ActionResult::extracted(huge_result)],
                state: blank_state(),
                metadata: None,
            }],
            ..AgentHistory::default()
        };

        let rendered = render_previous_results(&history, None);

        assert_eq!(
            rendered.chars().count(),
            MAX_PROMPT_CONTENT_CHARS + "\n... [Content truncated at 60k characters]".len()
        );
        assert!(rendered.ends_with("\n... [Content truncated at 60k characters]"));
    }

    #[test]
    fn search_url_matches_browser_use_engines() {
        assert_eq!(
            search_url(&SearchEngine::Google, "browser use rust"),
            "https://www.google.com/search?q=browser+use+rust&udm=14"
        );
        assert_eq!(
            search_url(&SearchEngine::DuckDuckGo, "browser use rust"),
            "https://duckduckgo.com/?q=browser+use+rust"
        );
    }
}
