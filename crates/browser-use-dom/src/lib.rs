//! DOM and accessibility-state serialization contracts.

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use url::Url;

/// Browser target identifier. In Chrome this is a CDP `TargetID`.
pub type TargetId = String;

/// DOM backend node identifier, stable enough for CDP follow-up actions.
pub type BackendNodeId = u64;

/// Node identifier scoped to a CDP session.
pub type NodeId = u64;

/// Information about an open tab or page target.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct TabInfo {
    pub url: String,
    pub title: String,
    pub target_id: TargetId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_target_id: Option<TargetId>,
}

impl TabInfo {
    #[must_use]
    pub fn short_target_id(&self) -> &str {
        let len = self.target_id.len();
        &self.target_id[len.saturating_sub(4)..]
    }
}

/// Viewport and scroll metrics used to help the agent reason about page shape.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PageInfo {
    pub viewport_width: u32,
    pub viewport_height: u32,
    pub page_width: u32,
    pub page_height: u32,
    pub scroll_x: i32,
    pub scroll_y: i32,
    pub pixels_above: u32,
    pub pixels_below: u32,
    pub pixels_left: u32,
    pub pixels_right: u32,
}

/// A network request that is still in flight when browser state is captured.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct NetworkRequest {
    pub url: String,
    #[serde(default = "default_method")]
    pub method: String,
    #[serde(default)]
    pub loading_duration_ms: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resource_type: Option<String>,
}

fn default_method() -> String {
    "GET".to_owned()
}

/// Pagination affordance detected from the current page.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PaginationButton {
    pub button_type: PaginationButtonType,
    pub backend_node_id: BackendNodeId,
    pub text: String,
    pub selector: String,
    #[serde(default)]
    pub is_disabled: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PaginationButtonType {
    Next,
    Prev,
    First,
    Last,
    PageNumber,
}

/// A compact node reference addressable by an action index.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct DomElementRef {
    pub index: u32,
    pub target_id: TargetId,
    pub backend_node_id: BackendNodeId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node_id: Option<NodeId>,
    pub tag_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(default)]
    pub attributes: BTreeMap<String, String>,
    #[serde(default)]
    pub is_visible: bool,
    #[serde(default)]
    pub is_interactive: bool,
}

/// Serialized DOM state in the form the agent consumes.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
pub struct SerializedDomState {
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub selector_map: BTreeMap<u32, DomElementRef>,
}

impl SerializedDomState {
    #[must_use]
    pub fn element_count(&self) -> usize {
        self.selector_map.len()
    }

    #[must_use]
    pub fn llm_representation(&self) -> &str {
        self.text.as_str()
    }
}

/// Browser state summary compatible with the browser-use agent step contract.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct BrowserStateSummary {
    pub dom_state: SerializedDomState,
    pub url: String,
    pub title: String,
    #[serde(default)]
    pub tabs: Vec<TabInfo>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub screenshot: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub page_info: Option<PageInfo>,
    #[serde(default)]
    pub pixels_above: u32,
    #[serde(default)]
    pub pixels_below: u32,
    #[serde(default)]
    pub browser_errors: Vec<String>,
    #[serde(default)]
    pub is_pdf_viewer: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recent_events: Option<String>,
    #[serde(default)]
    pub pending_network_requests: Vec<NetworkRequest>,
    #[serde(default)]
    pub pagination_buttons: Vec<PaginationButton>,
    #[serde(default)]
    pub closed_popup_messages: Vec<String>,
}

impl BrowserStateSummary {
    /// Returns the parsed current URL when it is absolute.
    #[must_use]
    pub fn parsed_url(&self) -> Option<Url> {
        Url::parse(&self.url).ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_target_id_matches_browser_use_display_shape() {
        let tab = TabInfo {
            url: "about:blank".to_owned(),
            title: "Blank".to_owned(),
            target_id: "123456abcdef".to_owned(),
            parent_target_id: None,
        };

        assert_eq!(tab.short_target_id(), "cdef");
    }

    #[test]
    fn empty_dom_state_has_zero_elements() {
        let dom = SerializedDomState::default();
        assert_eq!(dom.element_count(), 0);
        assert_eq!(dom.llm_representation(), "");
    }
}
