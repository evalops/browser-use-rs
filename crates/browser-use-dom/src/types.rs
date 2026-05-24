use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use url::Url;

use crate::SerializedDomState;

/// Browser target identifier. In Chrome this is a CDP `TargetID`.
pub type TargetId = String;

/// DOM backend node identifier, stable enough for CDP follow-up actions.
pub type BackendNodeId = u64;

/// Node identifier scoped to a CDP session.
pub type NodeId = u64;

/// Prompt text used when the browser could not produce a useful DOM snapshot.
pub const EMPTY_DOM_TREE_MESSAGE: &str =
    "Empty DOM tree (you might have to wait for the page to load)";

/// Information about an open tab or page target.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct TabInfo {
    /// Current URL of the tab.
    pub url: String,
    /// Browser title for the tab.
    pub title: String,
    /// Short human-facing identifier shown to the model and CLI.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub tab_id: String,
    /// Full Chrome DevTools Protocol target id.
    pub target_id: TargetId,
    /// Parent target when this tab-like entry represents an attached child target.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_target_id: Option<TargetId>,
}

impl TabInfo {
    /// Returns the short id the agent should use when switching tabs.
    #[must_use]
    pub fn short_target_id(&self) -> &str {
        if !self.tab_id.is_empty() {
            return &self.tab_id;
        }
        Self::short_target_id_for(&self.target_id)
    }

    /// Returns the last four characters of a CDP target id.
    ///
    /// Python browser-use exposes compact tab ids in prompts; this helper keeps
    /// the Rust display shape compatible without discarding the full target id.
    #[must_use]
    pub fn short_target_id_for(target_id: &str) -> &str {
        let len = target_id.len();
        &target_id[len.saturating_sub(4)..]
    }

    /// Builds the stable short tab id for a target.
    #[must_use]
    pub fn tab_id_for_target(target_id: &str) -> String {
        Self::short_target_id_for(target_id).to_owned()
    }
}

/// Viewport and scroll metrics used to help the agent reason about page shape.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PageInfo {
    /// Visible viewport width in CSS pixels.
    pub viewport_width: u32,
    /// Visible viewport height in CSS pixels.
    pub viewport_height: u32,
    /// Full document width in CSS pixels.
    pub page_width: u32,
    /// Full document height in CSS pixels.
    pub page_height: u32,
    /// Horizontal scroll offset.
    pub scroll_x: i32,
    /// Vertical scroll offset.
    pub scroll_y: i32,
    /// Pixels above the current viewport.
    pub pixels_above: u32,
    /// Pixels below the current viewport.
    pub pixels_below: u32,
    /// Pixels left of the current viewport.
    pub pixels_left: u32,
    /// Pixels right of the current viewport.
    pub pixels_right: u32,
}

/// A network request that is still in flight when browser state is captured.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct NetworkRequest {
    /// Request URL.
    pub url: String,
    /// HTTP method, defaulting to `GET` for compatibility with older snapshots.
    #[serde(default = "default_method")]
    pub method: String,
    /// Duration the request has been loading when state is captured.
    #[serde(default)]
    pub loading_duration_ms: f64,
    /// CDP resource type such as `Document`, `XHR`, or `Fetch`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resource_type: Option<String>,
}

fn default_method() -> String {
    "GET".to_owned()
}

/// Pagination affordance detected from the current page.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PaginationButton {
    /// Kind of pagination control detected.
    pub button_type: PaginationButtonType,
    /// CDP backend node id used to click the control.
    pub backend_node_id: BackendNodeId,
    /// Visible text or label for the control.
    pub text: String,
    /// Best-effort selector for diagnostics and replay hints.
    pub selector: String,
    /// Whether the control is disabled.
    #[serde(default)]
    pub is_disabled: bool,
}

/// Semantic category for a detected pagination control.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PaginationButtonType {
    /// Advances to the next page.
    Next,
    /// Returns to the previous page.
    Prev,
    /// Jumps to the first page.
    First,
    /// Jumps to the last page.
    Last,
    /// Direct link to a numbered page.
    PageNumber,
}

/// Viewport-relative integer bounds for an indexed element.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ElementBounds {
    /// Left coordinate relative to the viewport.
    pub x: i32,
    /// Top coordinate relative to the viewport.
    pub y: i32,
    /// Element width in CSS pixels.
    pub width: u32,
    /// Element height in CSS pixels.
    pub height: u32,
}

/// A compact node reference addressable by an action index.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct DomElementRef {
    /// One-based index shown in the prompt and used by indexed actions.
    pub index: u32,
    /// Target containing the element.
    pub target_id: TargetId,
    /// CDP backend node id used for follow-up DOM resolution.
    pub backend_node_id: BackendNodeId,
    /// Optional frontend node id when available in the active CDP session.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node_id: Option<NodeId>,
    /// Lower-level DOM tag name such as `button`, `input`, or `a`.
    pub tag_name: String,
    /// Accessibility role when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    /// Accessible name or primary label.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Visible text associated with the element.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    /// Attribute subset retained for rendering, matching, and diagnostics.
    #[serde(default)]
    pub attributes: BTreeMap<String, String>,
    /// Viewport-relative bounds when layout information is available.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bounds: Option<ElementBounds>,
    /// Whether the element is visible enough to present to the model.
    #[serde(default)]
    pub is_visible: bool,
    /// Whether the element can plausibly receive an action.
    #[serde(default)]
    pub is_interactive: bool,
    /// Whether the element has scrollable overflow.
    #[serde(default, skip_serializing_if = "is_false")]
    pub is_scrollable: bool,
}

/// Compact page-shape statistics rendered into the agent prompt.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
pub struct DomPageStats {
    /// Number of links discovered.
    #[serde(default)]
    pub links: u32,
    /// Number of frames or iframes discovered.
    #[serde(default)]
    pub iframes: u32,
    /// Number of open shadow roots discovered.
    #[serde(default)]
    pub shadow_open: u32,
    /// Number of closed shadow roots observed.
    #[serde(default)]
    pub shadow_closed: u32,
    /// Number of scrollable containers discovered.
    #[serde(default)]
    pub scroll_containers: u32,
    /// Number of images discovered.
    #[serde(default)]
    pub images: u32,
    /// Number of interactive elements indexed for actions.
    #[serde(default)]
    pub interactive_elements: u32,
    /// Total number of DOM elements visited.
    #[serde(default)]
    pub total_elements: u32,
    /// Number of text characters considered during DOM capture.
    #[serde(default)]
    pub text_chars: u32,
}

impl DomPageStats {
    /// Returns true when all counters are zero.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }
}

/// Node kind used by the evaluation-focused DOM tree serializer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DomEvalNodeType {
    /// Synthetic root used for shadow DOM and frame fragments.
    DocumentFragment,
    /// Regular element node.
    Element,
    /// Text node used to build inline text for nearby elements.
    Text,
}

/// Tree-shaped DOM node used for browser-use's evaluation/judge representation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct DomEvalNode {
    /// Kind of node represented by this struct.
    pub node_type: DomEvalNodeType,
    /// Element tag name, empty for text and document-fragment nodes.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub tag_name: String,
    /// Text node value, empty for element nodes.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub node_value: String,
    /// Attributes retained for judge/evaluation rendering.
    #[serde(default)]
    pub attributes: BTreeMap<String, String>,
    /// Child nodes in document order.
    #[serde(default)]
    pub children: Vec<DomEvalNode>,
    /// Backend node id displayed in `[i_N]` markers for interactive nodes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backend_node_id: Option<BackendNodeId>,
    /// Whether the serializer should display this node directly.
    #[serde(default = "default_true", skip_serializing_if = "is_true")]
    pub should_display: bool,
    /// Whether a hidden parent suppressed this node.
    #[serde(default, skip_serializing_if = "is_false")]
    pub excluded_by_parent: bool,
    /// Whether this node is visible.
    #[serde(default = "default_true", skip_serializing_if = "is_true")]
    pub is_visible: bool,
    /// Whether this node can be interacted with.
    #[serde(default, skip_serializing_if = "is_false")]
    pub is_interactive: bool,
    /// Whether this node has scrollable overflow.
    #[serde(default, skip_serializing_if = "is_false")]
    pub is_scrollable: bool,
    /// Human-readable scroll bounds or position summary.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scroll_info: Option<String>,
}

impl DomEvalNode {
    /// Creates a visible element node with no attributes or children.
    #[must_use]
    pub fn element(tag_name: impl Into<String>) -> Self {
        Self {
            node_type: DomEvalNodeType::Element,
            tag_name: tag_name.into(),
            node_value: String::new(),
            attributes: BTreeMap::new(),
            children: Vec::new(),
            backend_node_id: None,
            should_display: true,
            excluded_by_parent: false,
            is_visible: true,
            is_interactive: false,
            is_scrollable: false,
            scroll_info: None,
        }
    }

    /// Creates a text node.
    #[must_use]
    pub fn text(node_value: impl Into<String>) -> Self {
        Self {
            node_type: DomEvalNodeType::Text,
            tag_name: String::new(),
            node_value: node_value.into(),
            attributes: BTreeMap::new(),
            children: Vec::new(),
            backend_node_id: None,
            should_display: true,
            excluded_by_parent: false,
            is_visible: true,
            is_interactive: false,
            is_scrollable: false,
            scroll_info: None,
        }
    }

    /// Creates a document fragment containing `children`.
    #[must_use]
    pub fn document_fragment(children: Vec<DomEvalNode>) -> Self {
        Self {
            node_type: DomEvalNodeType::DocumentFragment,
            tag_name: String::new(),
            node_value: String::new(),
            attributes: BTreeMap::new(),
            children,
            backend_node_id: None,
            should_display: true,
            excluded_by_parent: false,
            is_visible: true,
            is_interactive: false,
            is_scrollable: false,
            scroll_info: None,
        }
    }

    /// Adds or replaces an attribute and returns the updated node.
    #[must_use]
    pub fn with_attribute(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.attributes.insert(name.into(), value.into());
        self
    }

    /// Replaces children and returns the updated node.
    #[must_use]
    pub fn with_children(mut self, children: Vec<DomEvalNode>) -> Self {
        self.children = children;
        self
    }

    /// Marks the node interactive and records the backend id used in output.
    #[must_use]
    pub fn interactive(mut self, backend_node_id: BackendNodeId) -> Self {
        self.backend_node_id = Some(backend_node_id);
        self.is_interactive = true;
        self
    }

    /// Marks the node hidden.
    #[must_use]
    pub fn hidden(mut self) -> Self {
        self.is_visible = false;
        self
    }

    /// Marks the node as omitted because an ancestor hid or filtered it.
    #[must_use]
    pub fn excluded_by_parent(mut self) -> Self {
        self.excluded_by_parent = true;
        self
    }

    /// Marks the node scrollable with a compact prompt-facing description.
    #[must_use]
    pub fn scrollable(mut self, scroll_info: impl Into<String>) -> Self {
        self.is_scrollable = true;
        self.scroll_info = Some(scroll_info.into());
        self
    }
}

fn default_true() -> bool {
    true
}

fn is_true(value: &bool) -> bool {
    *value
}

fn is_false(value: &bool) -> bool {
    !*value
}

/// Browser state summary compatible with the browser-use agent step contract.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct BrowserStateSummary {
    /// Serialized DOM and indexed element map for the active page.
    pub dom_state: SerializedDomState,
    /// Current active-tab URL.
    pub url: String,
    /// Current active-tab title.
    pub title: String,
    /// Known tabs and page targets.
    #[serde(default)]
    pub tabs: Vec<TabInfo>,
    /// Optional screenshot as a data URL or provider-ready image URL.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub screenshot: Option<String>,
    /// Viewport, page, and scroll metrics for the active page.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub page_info: Option<PageInfo>,
    /// Pixels above the viewport, duplicated for upstream prompt compatibility.
    #[serde(default)]
    pub pixels_above: u32,
    /// Pixels below the viewport, duplicated for upstream prompt compatibility.
    #[serde(default)]
    pub pixels_below: u32,
    /// Browser-state collection errors that should be visible to the agent.
    #[serde(default)]
    pub browser_errors: Vec<String>,
    /// Whether the active page appears to be Chrome's PDF viewer.
    #[serde(default)]
    pub is_pdf_viewer: bool,
    /// Optional recent browser event summary.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recent_events: Option<String>,
    /// Network requests still in flight at capture time.
    #[serde(default)]
    pub pending_network_requests: Vec<NetworkRequest>,
    /// Detected pagination controls.
    #[serde(default)]
    pub pagination_buttons: Vec<PaginationButton>,
    /// Popup messages closed during capture or watchdog handling.
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
