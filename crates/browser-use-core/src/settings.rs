//! Agent configuration, compatibility enums, and timeout helpers.
//!
//! The settings in this file are serialized for CLI/MCP callers and consumed
//! directly by the agent run loop. Several enums intentionally preserve Python
//! browser-use wire shapes, such as booleans-or-strings, while exposing clearer
//! Rust variants to the rest of the code.
//!
//! ```mermaid
//! flowchart TD
//!     CLI["CLI flags / MCP input / Rust API"] --> Settings["AgentSettings"]
//!     Settings --> Prompt["prompt and schema shaping"]
//!     Settings --> RunLoop["timeouts, loop detection, failure policy"]
//!     Settings --> Executor["action limits, uploads, files"]
//!     Settings --> Browser["vision and screenshot behavior"]
//! ```

use std::collections::BTreeMap;
use std::fmt;
use std::time::Duration;

use browser_use_llm::{ContentPart, ImageDetailLevel};
use browser_use_tools::BrowserAction;
use schemars::JsonSchema;
use serde::{Deserialize, Deserializer, Serialize, Serializer, de};
use serde_json::Value;

use crate::ActionResult;

const ACTION_TIMEOUT_ENV_VAR: &str = "BROWSER_USE_ACTION_TIMEOUT_S";
const ACTION_TIMEOUT_FALLBACK_SECONDS: f64 = 180.0;
const WAIT_BETWEEN_ACTIONS_FALLBACK_SECONDS: f64 = 0.1;

/// Optional resize target for screenshots sent to the LLM.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
pub struct LlmScreenshotSize {
    pub(crate) width: u32,
    pub(crate) height: u32,
}

impl LlmScreenshotSize {
    /// Minimum allowed width or height.
    pub const MIN_DIMENSION: u32 = 100;

    /// Creates a validated screenshot size.
    pub fn new(width: u32, height: u32) -> Result<Self, String> {
        if width < Self::MIN_DIMENSION || height < Self::MIN_DIMENSION {
            return Err("llm_screenshot_size dimensions must be at least 100 pixels".to_owned());
        }
        Ok(Self { width, height })
    }

    /// Screenshot width in pixels.
    #[must_use]
    pub fn width(self) -> u32 {
        self.width
    }

    /// Screenshot height in pixels.
    #[must_use]
    pub fn height(self) -> u32 {
        self.height
    }
}

impl<'de> Deserialize<'de> for LlmScreenshotSize {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        // Accept all shapes observed around Python/browser-use callers while
        // normalizing them into a small Rust value object for internal code.
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Wire {
            Tuple(u32, u32),
            Array([u32; 2]),
            Object { width: u32, height: u32 },
        }

        let (width, height) = match Wire::deserialize(deserializer)? {
            Wire::Tuple(width, height) | Wire::Object { width, height } => (width, height),
            Wire::Array([width, height]) => (width, height),
        };
        Self::new(width, height).map_err(de::Error::custom)
    }
}

/// Runtime configuration for one agent task.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct AgentSettings {
    /// Controls when screenshots are included in model observations.
    #[serde(default = "default_use_vision")]
    pub use_vision: VisionMode,
    /// Provider image-detail hint for screenshots.
    #[serde(default = "default_vision_detail_level")]
    pub vision_detail_level: ImageDetailLevel,
    /// Optional screenshot resize size before sending images to the model.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub llm_screenshot_size: Option<LlmScreenshotSize>,
    /// Maximum query/fragment characters before prompt URLs are shortened.
    #[serde(default = "default_url_shortening_limit")]
    pub url_shortening_limit: Option<usize>,
    /// Consecutive failure count before the agent stops.
    #[serde(default = "default_max_failures")]
    pub max_failures: u32,
    /// GIF recording setting for the final history artifact.
    #[serde(default)]
    pub generate_gif: GenerateGif,
    /// Maximum browser actions the model may return in one step.
    #[serde(default = "default_max_actions_per_step")]
    pub max_actions_per_step: usize,
    /// Timeout for a single LLM call.
    #[serde(default = "default_llm_timeout_seconds")]
    pub llm_timeout_seconds: u64,
    /// Timeout for one complete agent step.
    #[serde(default = "default_step_timeout_seconds")]
    pub step_timeout_seconds: u64,
    /// Timeout for a single browser action.
    #[serde(default = "default_action_timeout_seconds")]
    pub action_timeout_seconds: f64,
    /// Delay inserted between browser actions after the first action.
    #[serde(default = "default_wait_between_actions_seconds")]
    pub wait_between_actions_seconds: f64,
    /// Automatically opens a URL detected in the task before the first step.
    #[serde(default = "default_true")]
    pub directly_open_url: bool,
    /// Asks for a final response after terminal failures when possible.
    #[serde(default = "default_final_response_after_failure")]
    pub final_response_after_failure: bool,
    /// Includes requested managed file contents in `done` text.
    #[serde(default = "default_display_files_in_done_text")]
    pub display_files_in_done_text: bool,
    /// Number of recent action names used for loop detection.
    #[serde(default = "default_loop_detection_window")]
    pub loop_detection_window: usize,
    /// Enables repeated-action loop detection.
    #[serde(default = "default_loop_detection_enabled")]
    pub loop_detection_enabled: bool,
    /// Optional cap on history items included in prompts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_history_items: Option<usize>,
    /// Maximum characters of clickable-element text included in prompts.
    #[serde(default = "default_max_clickable_elements_length")]
    pub max_clickable_elements_length: usize,
    /// Includes recent browser lifecycle events in state prompts.
    #[serde(default)]
    pub include_recent_events: bool,
    /// Example images supplied to the prompt before live observations.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sample_images: Vec<ContentPart>,
    /// Enables planning fields in the model output schema and prompts.
    #[serde(default = "default_enable_planning")]
    pub enable_planning: bool,
    /// Step count after which planning may replan on stalls.
    #[serde(default = "default_planning_replan_on_stall")]
    pub planning_replan_on_stall: usize,
    /// Maximum exploratory steps before plan pressure increases.
    #[serde(default = "default_planning_exploration_limit")]
    pub planning_exploration_limit: usize,
    /// Enables model `thinking` fields in output.
    #[serde(default = "default_use_thinking")]
    pub use_thinking: bool,
    /// Uses compact Flash-provider prompt/schema shape when true.
    #[serde(default)]
    pub flash_mode: bool,
    /// Enables final judge/validation request.
    #[serde(default = "default_use_judge")]
    pub use_judge: bool,
    /// Optional ground truth supplied to judge prompts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ground_truth: Option<String>,
    /// Default schema applied to extract actions without their own schema.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extraction_schema: Option<Value>,
    /// Controls history/message compaction.
    #[serde(default, skip_serializing_if = "is_default_message_compaction")]
    pub message_compaction: MessageCompaction,
    /// Enables model cost estimation when usage is available.
    #[serde(default)]
    pub calculate_cost: bool,
    /// Includes tool-call examples in prompts.
    #[serde(default)]
    pub include_tool_call_examples: bool,
    /// Optional path for saving the conversation transcript.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub save_conversation_path: Option<String>,
    /// Encoding used when saving conversation transcripts.
    #[serde(
        default = "default_save_conversation_path_encoding",
        skip_serializing_if = "is_default_save_conversation_path_encoding"
    )]
    pub save_conversation_path_encoding: Option<String>,
    /// Base directory for the managed file sandbox.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_system_path: Option<String>,
    /// Extra DOM attributes to include in prompt-visible element lines.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub include_attributes: Vec<String>,
    /// File paths the upload action is allowed to use.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub available_file_paths: Vec<String>,
    /// Actions executed before the first model step.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub initial_actions: Vec<BrowserAction>,
    /// Action names removed from the model output schema.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub excluded_actions: Vec<String>,
    /// Sensitive placeholders available to replace in model-selected actions.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub sensitive_data: BTreeMap<String, SensitiveDataValue>,
    /// Optional full replacement for the system message.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub override_system_message: Option<String>,
    /// Optional text appended to the system message.
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
    /// Include screenshots by default.
    #[default]
    Always,
    /// Never include screenshots or prompt images.
    Never,
    /// Include screenshots only when the model requests the screenshot action.
    Auto,
}

impl VisionMode {
    /// Returns true when each normal observation should include a screenshot.
    #[must_use]
    pub fn includes_screenshot_by_default(self) -> bool {
        matches!(self, Self::Always)
    }

    /// Returns true when the screenshot action is available to the model.
    #[must_use]
    pub fn allows_screenshot_action(self) -> bool {
        matches!(self, Self::Auto)
    }

    /// Resolves whether the next observation should include a screenshot.
    #[must_use]
    pub fn should_include_screenshot(self, action_requested_screenshot: bool) -> bool {
        match self {
            Self::Always => true,
            Self::Never => false,
            Self::Auto => action_requested_screenshot,
        }
    }

    /// Returns true when prompt-provided images are allowed.
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
        // The wire schema stays intentionally loose: existing users send either
        // booleans or the string "auto", even though Rust code works with the
        // clearer `VisionMode` enum.
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
            llm_screenshot_size: None,
            url_shortening_limit: default_url_shortening_limit(),
            max_failures: default_max_failures(),
            generate_gif: GenerateGif::default(),
            max_actions_per_step: default_max_actions_per_step(),
            llm_timeout_seconds: default_llm_timeout_seconds(),
            step_timeout_seconds: default_step_timeout_seconds(),
            action_timeout_seconds: default_action_timeout_seconds(),
            wait_between_actions_seconds: default_wait_between_actions_seconds(),
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
            extraction_schema: None,
            message_compaction: MessageCompaction::default(),
            calculate_cost: false,
            include_tool_call_examples: false,
            save_conversation_path: None,
            save_conversation_path_encoding: default_save_conversation_path_encoding(),
            file_system_path: None,
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
    /// Returns a validated action timeout, falling back when the stored value is invalid.
    #[must_use]
    pub fn effective_action_timeout_seconds(&self) -> f64 {
        coerce_valid_action_timeout_seconds(self.action_timeout_seconds)
    }

    /// Returns a validated inter-action delay, falling back when the stored value is invalid.
    #[must_use]
    pub fn effective_wait_between_actions_seconds(&self) -> f64 {
        coerce_valid_wait_between_actions_seconds(self.wait_between_actions_seconds)
    }
}

/// Detailed configuration for summarizing old prompt history.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct MessageCompactionSettings {
    /// Enables compaction when true.
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Compacts at most every N steps.
    #[serde(default = "default_compact_every_n_steps")]
    pub compact_every_n_steps: usize,
    /// Prompt character threshold that triggers compaction.
    #[serde(
        default = "default_trigger_char_count",
        skip_serializing_if = "Option::is_none"
    )]
    pub trigger_char_count: Option<usize>,
    /// Alternative token threshold converted to characters.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trigger_token_count: Option<usize>,
    /// Conversion ratio used for token thresholds.
    #[serde(default = "default_chars_per_token")]
    pub chars_per_token: f64,
    /// Number of latest history items kept uncompressed.
    #[serde(default = "default_keep_last_items")]
    pub keep_last_items: usize,
    /// Maximum characters in the compaction summary.
    #[serde(default = "default_summary_max_chars")]
    pub summary_max_chars: usize,
    /// Includes managed-file read state in compaction prompts.
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
    /// Returns the configured character trigger or the default threshold.
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

/// Upstream-compatible message compaction toggle or settings object.
#[derive(Debug, Clone, Default, PartialEq)]
pub enum MessageCompaction {
    /// Disable compaction.
    Disabled,
    /// Enable compaction with default settings.
    #[default]
    Enabled,
    /// Enable compaction with explicit settings.
    Settings(MessageCompactionSettings),
}

impl MessageCompaction {
    /// Returns true when compaction should run.
    #[must_use]
    pub fn is_enabled(&self) -> bool {
        match self {
            Self::Disabled => false,
            Self::Enabled => true,
            Self::Settings(settings) => settings.enabled,
        }
    }

    /// Returns concrete compaction settings when enabled.
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

pub(crate) fn is_zero(value: &usize) -> bool {
    *value == 0
}

/// Upstream-compatible GIF generation setting.
///
/// Python browser-use accepts `False`, `True`, or a string output path. The
/// Rust runtime preserves that public shape even before GIF rendering side
/// effects are implemented.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum GenerateGif {
    /// Do not write a GIF.
    #[default]
    Disabled,
    /// Write a GIF at the default output path.
    Enabled,
    /// Write a GIF to the supplied path.
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

fn default_url_shortening_limit() -> Option<usize> {
    Some(25)
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

pub(crate) fn default_action_timeout_seconds() -> f64 {
    parse_action_timeout_seconds(std::env::var(ACTION_TIMEOUT_ENV_VAR).ok().as_deref())
}

pub(crate) fn default_wait_between_actions_seconds() -> f64 {
    WAIT_BETWEEN_ACTIONS_FALLBACK_SECONDS
}

pub(crate) fn parse_action_timeout_seconds(raw: Option<&str>) -> f64 {
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

pub(crate) fn coerce_valid_action_timeout_seconds(value: f64) -> f64 {
    if value.is_finite() && value > 0.0 && Duration::try_from_secs_f64(value).is_ok() {
        value
    } else {
        ACTION_TIMEOUT_FALLBACK_SECONDS
    }
}

pub(crate) fn coerce_valid_wait_between_actions_seconds(value: f64) -> f64 {
    if value.is_finite() && value >= 0.0 && Duration::try_from_secs_f64(value).is_ok() {
        value
    } else {
        WAIT_BETWEEN_ACTIONS_FALLBACK_SECONDS
    }
}

pub(crate) fn action_timeout_duration(seconds: f64) -> Duration {
    Duration::try_from_secs_f64(coerce_valid_action_timeout_seconds(seconds))
        .unwrap_or_else(|_| Duration::from_secs(ACTION_TIMEOUT_FALLBACK_SECONDS as u64))
}

pub(crate) fn wait_between_actions_duration(seconds: f64) -> Duration {
    Duration::try_from_secs_f64(coerce_valid_wait_between_actions_seconds(seconds))
        .unwrap_or_else(|_| Duration::from_millis(100))
}

pub(crate) fn timed_out_action_result(
    action: &BrowserAction,
    timeout_seconds: f64,
) -> ActionResult {
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

/// Sensitive data value used by placeholder replacement.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum SensitiveDataValue {
    /// One global placeholder value.
    Value(String),
    /// Domain-specific placeholder values.
    Domain(BTreeMap<String, String>),
}
