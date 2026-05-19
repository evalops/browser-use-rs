//! DOM and accessibility-state serialization contracts.
//!
//! The agent never receives a live browser DOM directly. Instead, the CDP
//! crate distills the current page into the structs in this crate: tab
//! metadata, page metrics, indexed interactive elements, and compact text
//! renderings that can be placed in an LLM prompt. The same structs are also
//! used by history replay to rematch old element indexes against a fresh DOM.
//!
//! ```mermaid
//! flowchart LR
//!     CDP["CDP DOM / AX / runtime snapshots"] --> Ref["DomElementRef selector_map"]
//!     Ref --> Text["llm_representation"]
//!     Ref --> Eval["DomEvalNode eval tree"]
//!     Ref --> Fingerprint["DomInteractedElement fingerprints"]
//!     Text --> Prompt["Agent prompt"]
//!     Fingerprint --> Replay["history replay rematch"]
//!     Eval --> Judge["judge/evaluation prompt"]
//! ```

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use url::Url;

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

/// Upstream-style compact record for an element targeted by an action.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct DomInteractedElement {
    /// Target that originally contained the interacted element.
    pub target_id: TargetId,
    /// Frontend node id recorded at interaction time.
    pub node_id: NodeId,
    /// Backend node id recorded at interaction time.
    pub backend_node_id: BackendNodeId,
    /// Frame id when the interaction happened inside a frame.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frame_id: Option<String>,
    /// DOM node type number, using browser DOM constants.
    pub node_type: u8,
    /// Node value captured for compatibility with upstream history shape.
    pub node_value: String,
    /// Node name or tag captured at interaction time.
    pub node_name: String,
    /// Attributes captured at interaction time.
    #[serde(default)]
    pub attributes: BTreeMap<String, String>,
    /// Bounds captured at interaction time.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bounds: Option<ElementBounds>,
    /// Synthetic XPath used as one replay matching strategy.
    pub x_path: String,
    /// Exact hash of stable element identity inputs.
    pub element_hash: u64,
    /// More forgiving hash that ignores volatile class tokens.
    pub stable_hash: Option<u64>,
    /// Accessibility name used as a fallback matching strategy.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ax_name: Option<String>,
}

/// Strategy that successfully matched a historical element to the current DOM.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DomInteractedElementMatchLevel {
    /// Full element hash matched.
    Exact,
    /// Stable hash matched after filtering dynamic class tokens.
    Stable,
    /// Synthetic XPath matched.
    XPath,
    /// Accessibility name matched.
    AxName,
    /// A selected attribute such as `id`, `name`, or `aria-label` matched.
    Attribute,
}

/// Reason a historical interacted element could not be safely remapped.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DomInteractedElementMatchFailureReason {
    /// The current snapshot has no indexed elements to match against.
    EmptySelectorMap,
    /// More than one current element matched at the same confidence level.
    Ambiguous,
    /// No current element matched.
    NotFound,
}

/// Successful rematch from a historical interacted element to a current index.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct DomInteractedElementMatch {
    /// Current one-based element index.
    pub index: u32,
    /// Matching strategy that produced this unique match.
    pub level: DomInteractedElementMatchLevel,
    /// Attribute name when `level` is [`DomInteractedElementMatchLevel::Attribute`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attribute: Option<String>,
}

/// Detailed failure returned when replay cannot safely remap an element.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct DomInteractedElementMatchFailure {
    /// High-level failure category.
    pub reason: DomInteractedElementMatchFailureReason,
    /// Matching strategy that failed, when relevant.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub level: Option<DomInteractedElementMatchLevel>,
    /// Attribute involved in an attribute-level ambiguity.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attribute: Option<String>,
    /// Candidate element indexes when matching was ambiguous.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub candidate_indices: Vec<u32>,
    /// Human-readable explanation suitable for replay diagnostics.
    pub message: String,
}

impl DomInteractedElement {
    /// Builds a history/replay identity record from a currently indexed element.
    #[must_use]
    pub fn from_element(element: &DomElementRef) -> Self {
        let x_path = synthetic_x_path(element);
        Self {
            target_id: element.target_id.clone(),
            node_id: element.node_id.unwrap_or_default(),
            backend_node_id: element.backend_node_id,
            frame_id: None,
            node_type: 1,
            node_value: element.text.clone().unwrap_or_default(),
            node_name: element.tag_name.clone(),
            attributes: element.attributes.clone(),
            bounds: element.bounds,
            element_hash: interacted_element_hash(element, &x_path, HashClassMode::Exact),
            stable_hash: Some(interacted_element_hash(
                element,
                &x_path,
                HashClassMode::Stable,
            )),
            x_path,
            ax_name: element
                .attributes
                .get("ax_name")
                .filter(|value| !value.is_empty())
                .cloned()
                .or_else(|| {
                    element
                        .name
                        .as_ref()
                        .filter(|value| !value.is_empty())
                        .cloned()
                }),
        }
    }

    /// Attempts to find this historical element in a fresh DOM snapshot.
    pub fn rematch(
        &self,
        state: &SerializedDomState,
    ) -> Result<DomInteractedElementMatch, DomInteractedElementMatchFailure> {
        rematch_interacted_element(self, state)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HashClassMode {
    Exact,
    Stable,
}

const STATIC_HASH_ATTRIBUTES: &[&str] = &[
    "class",
    "id",
    "name",
    "type",
    "placeholder",
    "aria-label",
    "title",
    "role",
    "data-testid",
    "data-test",
    "data-cy",
    "data-selenium",
    "for",
    "required",
    "disabled",
    "readonly",
    "checked",
    "selected",
    "multiple",
    "accept",
    "href",
    "target",
    "rel",
    "aria-describedby",
    "aria-labelledby",
    "aria-controls",
    "aria-owns",
    "aria-live",
    "aria-atomic",
    "aria-busy",
    "aria-disabled",
    "aria-hidden",
    "aria-pressed",
    "aria-autocomplete",
    "aria-checked",
    "aria-selected",
    "list",
    "tabindex",
    "alt",
    "src",
    "lang",
    "itemscope",
    "itemtype",
    "itemprop",
    "pseudo",
    "aria-valuemin",
    "aria-valuemax",
    "aria-valuenow",
    "aria-placeholder",
];

const DYNAMIC_CLASS_PATTERNS: &[&str] = &[
    "focus",
    "hover",
    "active",
    "selected",
    "disabled",
    "animation",
    "transition",
    "loading",
    "open",
    "closed",
    "expanded",
    "collapsed",
    "visible",
    "hidden",
    "pressed",
    "checked",
    "highlighted",
    "current",
    "entering",
    "leaving",
];

fn interacted_element_hash(
    element: &DomElementRef,
    x_path: &str,
    class_mode: HashClassMode,
) -> u64 {
    // The hash intentionally mixes structural hints and accessibility naming
    // rather than only the transient browser-use index. That gives replay a
    // stable identity when the page inserts or removes unrelated elements.
    let attributes = hash_attributes(element, class_mode);
    let attributes_string = attributes
        .iter()
        .map(|(key, value)| format!("{key}={value}"))
        .collect::<Vec<_>>()
        .join("");
    let ax_name = element
        .attributes
        .get("ax_name")
        .filter(|value| !value.is_empty())
        .map(|value| format!("|ax_name={value}"))
        .unwrap_or_default();
    let source = format!(
        "{}|{}{}",
        element.tag_name.to_ascii_lowercase(),
        attributes_string,
        ax_name
    );
    sha256_u64(&format!("{source}|x_path={x_path}"))
}

fn hash_attributes(element: &DomElementRef, class_mode: HashClassMode) -> BTreeMap<String, String> {
    element
        .attributes
        .iter()
        .filter(|(key, _)| STATIC_HASH_ATTRIBUTES.contains(&key.as_str()))
        .filter_map(|(key, value)| {
            let value = if key == "class" && class_mode == HashClassMode::Stable {
                filter_dynamic_classes(value)
            } else {
                value.clone()
            };
            if value.is_empty() {
                None
            } else {
                Some((key.clone(), value))
            }
        })
        .collect()
}

fn filter_dynamic_classes(class_value: &str) -> String {
    // Utility CSS classes often describe state rather than identity. Filtering
    // them for the stable hash lets replay survive hover/loading/open state
    // changes without accepting a completely different element.
    let mut classes = class_value
        .split_whitespace()
        .filter(|class_name| {
            let class_name = class_name.to_ascii_lowercase();
            !DYNAMIC_CLASS_PATTERNS
                .iter()
                .any(|pattern| class_name.contains(pattern))
        })
        .collect::<Vec<_>>();
    classes.sort_unstable();
    classes.join(" ")
}

fn sha256_u64(value: &str) -> u64 {
    let digest = Sha256::digest(value.as_bytes());
    let mut bytes = [0_u8; 8];
    bytes.copy_from_slice(&digest[..8]);
    u64::from_be_bytes(bytes)
}

fn synthetic_x_path(element: &DomElementRef) -> String {
    let tag = element.tag_name.to_ascii_lowercase();
    for attribute in [
        "id",
        "data-testid",
        "data-test",
        "data-cy",
        "name",
        "aria-label",
        "title",
    ] {
        if let Some(value) = element
            .attributes
            .get(attribute)
            .filter(|value| !value.is_empty())
        {
            return format!("//{tag}[@{attribute}={}]", xpath_literal(value));
        }
    }
    format!("//{tag}[@browser-use-index='{}']", element.index)
}

fn xpath_literal(value: &str) -> String {
    if !value.contains('\'') {
        return format!("'{value}'");
    }
    if !value.contains('"') {
        return format!("\"{value}\"");
    }
    let parts = value
        .split('\'')
        .map(|part| format!("'{part}'"))
        .collect::<Vec<_>>();
    format!("concat({})", parts.join(", \"'\", "))
}

/// Remaps a historical interacted element to the current DOM selector map.
///
/// Browser-use history stores the element that was acted on, not just the
/// transient numeric index shown to the model. When replaying history on a
/// changed page, this function tries progressively weaker identity strategies
/// and only returns a new index if the match is unique.
pub fn rematch_interacted_element(
    historical: &DomInteractedElement,
    state: &SerializedDomState,
) -> Result<DomInteractedElementMatch, DomInteractedElementMatchFailure> {
    if state.selector_map.is_empty() {
        return Err(DomInteractedElementMatchFailure {
            reason: DomInteractedElementMatchFailureReason::EmptySelectorMap,
            level: None,
            attribute: None,
            candidate_indices: Vec::new(),
            message: "cannot rematch interacted element against an empty selector map".to_owned(),
        });
    }

    // Matching proceeds from strongest to weakest signal. Each strategy must
    // produce exactly one candidate; ambiguity is reported instead of hidden so
    // replay tooling can explain why a historical action cannot be trusted.
    if let Some(match_result) = unique_match(
        DomInteractedElementMatchLevel::Exact,
        None,
        matching_indices(state, |element| {
            DomInteractedElement::from_element(element).element_hash == historical.element_hash
        }),
    )? {
        return Ok(match_result);
    }

    if let Some(stable_hash) = historical.stable_hash
        && let Some(match_result) = unique_match(
            DomInteractedElementMatchLevel::Stable,
            None,
            matching_indices(state, |element| {
                DomInteractedElement::from_element(element).stable_hash == Some(stable_hash)
            }),
        )?
    {
        return Ok(match_result);
    }

    if !historical.x_path.is_empty()
        && let Some(match_result) = unique_match(
            DomInteractedElementMatchLevel::XPath,
            None,
            matching_indices(state, |element| {
                DomInteractedElement::from_element(element).x_path == historical.x_path
            }),
        )?
    {
        return Ok(match_result);
    }

    if let Some(ax_name) = historical
        .ax_name
        .as_deref()
        .filter(|value| !value.is_empty())
        && let Some(match_result) = unique_match(
            DomInteractedElementMatchLevel::AxName,
            None,
            matching_indices(state, |element| {
                element
                    .tag_name
                    .eq_ignore_ascii_case(historical.node_name.as_str())
                    && DomInteractedElement::from_element(element)
                        .ax_name
                        .as_deref()
                        == Some(ax_name)
            }),
        )?
    {
        return Ok(match_result);
    }

    for attribute in ["name", "id", "aria-label"] {
        let Some(expected) = historical
            .attributes
            .get(attribute)
            .filter(|value| !value.is_empty())
        else {
            continue;
        };
        if let Some(match_result) = unique_match(
            DomInteractedElementMatchLevel::Attribute,
            Some(attribute),
            matching_indices(state, |element| {
                element
                    .tag_name
                    .eq_ignore_ascii_case(historical.node_name.as_str())
                    && element.attributes.get(attribute) == Some(expected)
            }),
        )? {
            return Ok(match_result);
        }
    }

    Err(DomInteractedElementMatchFailure {
        reason: DomInteractedElementMatchFailureReason::NotFound,
        level: None,
        attribute: None,
        candidate_indices: Vec::new(),
        message: format!(
            "no current selector-map element matched historical <{}> interacted element",
            historical.node_name
        ),
    })
}

fn matching_indices(
    state: &SerializedDomState,
    mut predicate: impl FnMut(&DomElementRef) -> bool,
) -> Vec<u32> {
    state
        .selector_map
        .iter()
        .filter_map(|(index, element)| predicate(element).then_some(*index))
        .collect()
}

fn unique_match(
    level: DomInteractedElementMatchLevel,
    attribute: Option<&str>,
    indices: Vec<u32>,
) -> Result<Option<DomInteractedElementMatch>, DomInteractedElementMatchFailure> {
    match indices.as_slice() {
        [] => Ok(None),
        [index] => Ok(Some(DomInteractedElementMatch {
            index: *index,
            level,
            attribute: attribute.map(str::to_owned),
        })),
        _ => Err(DomInteractedElementMatchFailure {
            reason: DomInteractedElementMatchFailureReason::Ambiguous,
            level: Some(level),
            attribute: attribute.map(str::to_owned),
            candidate_indices: indices,
            message: format!("multiple selector-map elements matched at {level:?} level"),
        }),
    }
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

/// Serialized DOM state in the form the agent consumes.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
pub struct SerializedDomState {
    /// Pre-rendered indexed text shown to the model.
    #[serde(default)]
    pub text: String,
    /// Map from browser-use element index to the element reference.
    #[serde(default)]
    pub selector_map: BTreeMap<u32, DomElementRef>,
    /// Optional aggregate page-shape counters.
    #[serde(default, skip_serializing_if = "DomPageStats::is_empty")]
    pub page_stats: DomPageStats,
    /// Optional tree representation used by evaluation/judge flows.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub eval_root: Option<DomEvalNode>,
}

impl SerializedDomState {
    /// Builds a state from indexed elements and renders the default prompt text.
    #[must_use]
    pub fn from_elements(elements: Vec<DomElementRef>) -> Self {
        let mut selector_map = BTreeMap::new();
        let mut lines = Vec::new();

        for element in elements {
            lines.push(render_element_line(&element));
            selector_map.insert(element.index, element);
        }

        Self {
            text: lines.join("\n"),
            selector_map,
            page_stats: DomPageStats::default(),
            eval_root: None,
        }
    }

    /// Attaches page statistics to the state.
    #[must_use]
    pub fn with_page_stats(mut self, page_stats: DomPageStats) -> Self {
        self.page_stats = page_stats;
        self
    }

    /// Attaches an evaluation tree root to the state.
    #[must_use]
    pub fn with_eval_root(mut self, eval_root: DomEvalNode) -> Self {
        self.eval_root = Some(eval_root);
        self
    }

    /// Returns the number of indexed elements available for actions.
    #[must_use]
    pub fn element_count(&self) -> usize {
        self.selector_map.len()
    }

    /// Returns prompt text for the LLM, falling back to the empty-DOM message.
    #[must_use]
    pub fn llm_representation(&self) -> &str {
        if self.text.is_empty() && self.selector_map.is_empty() {
            return EMPTY_DOM_TREE_MESSAGE;
        }
        self.text.as_str()
    }

    /// Renders prompt text with an explicit attribute allow-list.
    #[must_use]
    pub fn llm_representation_with_attributes(&self, include_attributes: &[String]) -> String {
        if self.selector_map.is_empty() {
            return EMPTY_DOM_TREE_MESSAGE.to_owned();
        }
        if include_attributes.is_empty() {
            return self.llm_representation().to_owned();
        }

        self.selector_map
            .values()
            .map(|element| render_element_line_with_attributes(element, include_attributes))
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Renders the evaluation tree with the default attribute policy.
    #[must_use]
    pub fn eval_representation(&self) -> String {
        self.eval_representation_with_attributes(&[])
    }

    /// Renders the evaluation tree with caller-provided attributes.
    ///
    /// The current serializer keeps an upstream-compatible fixed attribute set,
    /// so this argument is accepted for API symmetry but is not yet used.
    #[must_use]
    pub fn eval_representation_with_attributes(&self, _include_attributes: &[String]) -> String {
        self.eval_root
            .as_ref()
            .map(|root| serialize_eval_tree(root, 0))
            .filter(|text| !text.is_empty())
            .unwrap_or_else(|| EMPTY_DOM_TREE_MESSAGE.to_owned())
    }
}

const EVAL_KEY_ATTRIBUTES: &[&str] = &[
    "id",
    "class",
    "name",
    "type",
    "placeholder",
    "aria-label",
    "role",
    "value",
    "data-testid",
    "alt",
    "title",
    "checked",
    "selected",
    "disabled",
    "required",
    "readonly",
    "aria-expanded",
    "aria-pressed",
    "aria-checked",
    "aria-selected",
    "aria-invalid",
    "pattern",
    "min",
    "max",
    "minlength",
    "maxlength",
    "step",
    "aria-valuemin",
    "aria-valuemax",
    "aria-valuenow",
    "ax_name",
    "ax_description",
];

const EVAL_CONTAINER_ELEMENTS: &[&str] = &[
    "html", "body", "div", "main", "section", "article", "aside", "header", "footer", "nav",
];

const EVAL_SVG_CHILD_ELEMENTS: &[&str] = &[
    "path", "rect", "g", "circle", "ellipse", "line", "polyline", "polygon", "use", "defs",
    "clippath", "mask", "pattern", "image", "text", "tspan",
];

fn serialize_eval_tree(node: &DomEvalNode, depth: usize) -> String {
    if node.excluded_by_parent || !node.should_display {
        return serialize_eval_children(node, depth);
    }

    match node.node_type {
        DomEvalNodeType::Element => serialize_eval_element(node, depth),
        DomEvalNodeType::Text => String::new(),
        DomEvalNodeType::DocumentFragment => serialize_eval_document_fragment(node, depth),
    }
}

fn serialize_eval_element(node: &DomEvalNode, depth: usize) -> String {
    let tag = node.tag_name.to_ascii_lowercase();

    if !node.is_visible
        && !EVAL_CONTAINER_ELEMENTS.contains(&tag.as_str())
        && !matches!(tag.as_str(), "iframe" | "frame")
    {
        return serialize_eval_children(node, depth);
    }

    if matches!(tag.as_str(), "iframe" | "frame") {
        return serialize_eval_iframe(node, depth);
    }

    if tag == "svg" {
        let mut line = eval_depth_prefix(depth);
        if node.is_interactive
            && let Some(backend_node_id) = node.backend_node_id
        {
            line.push_str(&format!("[i_{backend_node_id}] "));
        }
        line.push_str("<svg");
        let attributes = build_eval_attributes(node);
        if !attributes.is_empty() {
            line.push(' ');
            line.push_str(&attributes);
        }
        line.push_str(" /> <!-- SVG content collapsed -->");
        return line;
    }

    if EVAL_SVG_CHILD_ELEMENTS.contains(&tag.as_str()) {
        return String::new();
    }

    let attributes = build_eval_attributes(node);
    let inline_text = eval_inline_text(node);
    let is_container = EVAL_CONTAINER_ELEMENTS.contains(&tag.as_str());

    let mut output = Vec::new();
    let mut line = eval_depth_prefix(depth);
    if node.is_interactive
        && let Some(backend_node_id) = node.backend_node_id
    {
        line.push_str(&format!("[i_{backend_node_id}] "));
    }
    line.push('<');
    line.push_str(&tag);
    if !attributes.is_empty() {
        line.push(' ');
        line.push_str(&attributes);
    }
    if node.is_scrollable
        && let Some(scroll_info) = node
            .scroll_info
            .as_deref()
            .filter(|value| !value.is_empty())
    {
        line.push_str(" scroll=\"");
        line.push_str(scroll_info);
        line.push('"');
    }
    if inline_text.is_empty() || is_container {
        line.push_str(" />");
    } else {
        line.push('>');
        line.push_str(&inline_text);
    }
    output.push(line);

    if !node.children.is_empty() && (is_container || inline_text.is_empty()) {
        let children = serialize_eval_children(node, depth + 1);
        if !children.is_empty() {
            output.push(children);
        }
    }

    output.join("\n")
}

fn serialize_eval_document_fragment(node: &DomEvalNode, depth: usize) -> String {
    if node.children.is_empty() {
        return String::new();
    }
    let children = serialize_eval_children(node, depth + 1);
    if children.is_empty() {
        return String::new();
    }
    format!("{}#shadow\n{children}", eval_depth_prefix(depth))
}

fn serialize_eval_iframe(node: &DomEvalNode, depth: usize) -> String {
    let tag = node.tag_name.to_ascii_lowercase();
    let mut output = Vec::new();
    let mut line = eval_depth_prefix(depth);
    line.push('<');
    line.push_str(&tag);

    let attributes = build_eval_attributes(node);
    if !attributes.is_empty() {
        line.push(' ');
        line.push_str(&attributes);
    }
    if node.is_scrollable
        && let Some(scroll_info) = node
            .scroll_info
            .as_deref()
            .filter(|value| !value.is_empty())
    {
        line.push_str(" scroll=\"");
        line.push_str(scroll_info);
        line.push('"');
    }
    line.push_str(" />");
    output.push(line);

    let children = serialize_eval_children(node, depth + 2);
    if !children.is_empty() {
        output.push(format!("{}\t#iframe-content", eval_depth_prefix(depth)));
        output.push(children);
    }

    output.join("\n")
}

fn serialize_eval_children(node: &DomEvalNode, depth: usize) -> String {
    let is_list_container = node.node_type == DomEvalNodeType::Element
        && matches!(node.tag_name.to_ascii_lowercase().as_str(), "ul" | "ol");
    let mut children_output = Vec::new();
    let mut li_count = 0_u32;
    let max_list_items = 50_u32;
    let mut consecutive_link_count = 0_u32;
    let max_consecutive_links = 50_u32;
    let mut total_links_skipped = 0_u32;

    for child in &node.children {
        let current_tag = (child.node_type == DomEvalNodeType::Element)
            .then(|| child.tag_name.to_ascii_lowercase());

        if is_list_container && current_tag.as_deref() == Some("li") {
            li_count += 1;
            if li_count > max_list_items {
                continue;
            }
        }

        if current_tag.as_deref() == Some("a") {
            consecutive_link_count += 1;
            if consecutive_link_count > max_consecutive_links {
                total_links_skipped += 1;
                continue;
            }
        } else {
            if total_links_skipped > 0 {
                children_output.push(format!(
                    "{}... ({total_links_skipped} more links in this list)",
                    eval_depth_prefix(depth)
                ));
                total_links_skipped = 0;
            }
            consecutive_link_count = 0;
        }

        let child_text = serialize_eval_tree(child, depth);
        if !child_text.is_empty() {
            children_output.push(child_text);
        }
    }

    if is_list_container && li_count > max_list_items {
        children_output.push(format!(
            "{}... ({} more items in this list (truncated) use evaluate to get more.",
            eval_depth_prefix(depth),
            li_count - max_list_items
        ));
    }
    if total_links_skipped > 0 {
        children_output.push(format!(
            "{}... ({total_links_skipped} more links in this list) (truncated) use evaluate to get more.",
            eval_depth_prefix(depth)
        ));
    }

    children_output.join("\n")
}

fn build_eval_attributes(node: &DomEvalNode) -> String {
    EVAL_KEY_ATTRIBUTES
        .iter()
        .filter_map(|attribute| {
            let value = node.attributes.get(*attribute)?.trim();
            if value.is_empty() {
                return None;
            }
            let value = if *attribute == "class" {
                value
                    .split_whitespace()
                    .take(3)
                    .collect::<Vec<_>>()
                    .join(" ")
            } else {
                cap_eval_text(value, 80)
            };
            Some(format!("{attribute}=\"{value}\""))
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn eval_inline_text(node: &DomEvalNode) -> String {
    let text_parts = node
        .children
        .iter()
        .filter(|child| child.node_type == DomEvalNodeType::Text)
        .map(|child| child.node_value.trim())
        .filter(|text| text.chars().count() > 1)
        .collect::<Vec<_>>();

    if text_parts.is_empty() {
        return String::new();
    }

    cap_eval_text(&text_parts.join(" "), 80)
}

fn eval_depth_prefix(depth: usize) -> String {
    "\t".repeat(depth)
}

fn cap_eval_text(value: &str, max_chars: usize) -> String {
    if value.chars().count() <= max_chars {
        return value.to_owned();
    }
    let mut capped = value.chars().take(max_chars).collect::<String>();
    capped.push_str("...");
    capped
}

#[must_use]
/// Renders one indexed element in the compact prompt format.
///
/// The result starts with `[index]`, includes a minimal tag/attribute string,
/// and appends accessible text when present.
pub fn render_element_line(element: &DomElementRef) -> String {
    render_element_line_with_attribute_names(element, DEFAULT_RENDER_ATTRIBUTES)
}

#[must_use]
/// Renders one indexed element with an explicit attribute allow-list.
pub fn render_element_line_with_attributes(
    element: &DomElementRef,
    include_attributes: &[String],
) -> String {
    let include_attributes = include_attributes
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>();
    render_element_line_with_attribute_names(element, &include_attributes)
}

fn render_element_line_with_attribute_names(
    element: &DomElementRef,
    include_attributes: &[&str],
) -> String {
    let attributes = render_element_attributes_with_attribute_names(element, include_attributes);
    let tag = if attributes.is_empty() {
        format!("<{}>", element.tag_name)
    } else {
        format!("<{} {attributes}>", element.tag_name)
    };
    let text = render_element_text(element);
    let prefix = if element.is_scrollable {
        format!("[{}] |scroll element|", element.index)
    } else {
        format!("[{}]", element.index)
    };
    let scroll_info = element
        .attributes
        .get("scroll")
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
        .map(|value| format!(" ({value})"))
        .unwrap_or_default();
    match (scroll_info.is_empty(), text.is_empty()) {
        (true, true) => format!("{prefix} {tag}"),
        (true, false) => format!("{prefix} {tag} {text}"),
        (false, true) => format!("{prefix} {tag}{scroll_info}"),
        (false, false) => format!("{prefix} {tag}{scroll_info} {text}"),
    }
}

#[must_use]
/// Returns the text label used beside an indexed element in prompt output.
pub fn render_element_text(element: &DomElementRef) -> String {
    match (element.name.as_deref(), element.text.as_deref()) {
        (Some(name), Some(value)) if !value.is_empty() && name != value => {
            format!("{name} {value}")
        }
        (Some(name), _) => name.to_owned(),
        (_, Some(value)) => value.to_owned(),
        _ => String::new(),
    }
}

const DEFAULT_RENDER_ATTRIBUTES: &[&str] = &[
    "title",
    "type",
    "checked",
    "id",
    "name",
    "role",
    "value",
    "placeholder",
    "data-date-format",
    "alt",
    "aria-label",
    "aria-expanded",
    "data-state",
    "aria-checked",
    "aria-valuemin",
    "aria-valuemax",
    "aria-valuenow",
    "aria-placeholder",
    "pattern",
    "min",
    "max",
    "minlength",
    "maxlength",
    "step",
    "accept",
    "multiple",
    "inputmode",
    "autocomplete",
    "aria-autocomplete",
    "list",
    "data-mask",
    "data-inputmask",
    "data-datepicker",
    "format",
    "expected_format",
    "contenteditable",
    "pseudo",
    "selected",
    "compound_components",
    "expanded",
    "pressed",
    "disabled",
    "invalid",
    "valuemin",
    "valuemax",
    "valuenow",
    "keyshortcuts",
    "haspopup",
    "multiselectable",
    "required",
    "valuetext",
    "level",
    "busy",
    "live",
    "ax_name",
    "ax_description",
];

#[must_use]
/// Renders the default prompt-facing attribute list for an element.
pub fn render_element_attributes(element: &DomElementRef) -> String {
    render_element_attributes_with_attribute_names(element, DEFAULT_RENDER_ATTRIBUTES)
}

#[must_use]
/// Renders an element attribute string with an explicit allow-list.
pub fn render_element_attributes_with_attributes(
    element: &DomElementRef,
    include_attributes: &[String],
) -> String {
    let include_attributes = include_attributes
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>();
    render_element_attributes_with_attribute_names(element, &include_attributes)
}

fn render_element_attributes_with_attribute_names(
    element: &DomElementRef,
    include_attributes: &[&str],
) -> String {
    let is_password_field = element.tag_name.eq_ignore_ascii_case("input")
        && element
            .attributes
            .get("type")
            .is_some_and(|value| value.eq_ignore_ascii_case("password"));
    let text = render_element_text(element);

    // Attribute rendering is intentionally lossy. The model needs useful
    // interaction hints, not a full HTML dump, and prompt output must avoid
    // duplicating labels or leaking password values.
    let rendered_attributes = include_attributes
        .iter()
        .filter_map(|attribute| {
            let value = render_attribute_value(element, attribute)?;
            if value.is_empty() {
                return None;
            }
            if is_password_field && matches!(*attribute, "value" | "valuetext") {
                return None;
            }
            if *attribute == "type" && value.eq_ignore_ascii_case(&element.tag_name) {
                return None;
            }
            if *attribute == "role" && value.eq_ignore_ascii_case(&element.tag_name) {
                return None;
            }
            if *attribute == "invalid" && value.eq_ignore_ascii_case("false") {
                return None;
            }
            if let Some(alias_target) = aliased_suppression_target(attribute)
                && include_attributes.contains(&alias_target)
                && render_attribute_value(element, alias_target).is_some()
            {
                return None;
            }
            if *attribute == "required"
                && matches!(value.to_ascii_lowercase().as_str(), "false" | "0" | "no")
            {
                return None;
            }
            if matches!(*attribute, "aria-label" | "placeholder" | "title")
                && !text.is_empty()
                && value.eq_ignore_ascii_case(text.trim())
            {
                return None;
            }
            if matches!(*attribute, "ax_name" | "ax_description")
                && element_label_values(element).any(|label| value.eq_ignore_ascii_case(label))
            {
                return None;
            }
            Some((*attribute, value))
        })
        .collect::<Vec<_>>();

    remove_duplicate_attribute_values(rendered_attributes)
        .into_iter()
        .map(|(attribute, value)| format!("{attribute}={}", cap_attribute_value(&value)))
        .collect::<Vec<_>>()
        .join(" ")
}

fn remove_duplicate_attribute_values(attributes: Vec<(&str, String)>) -> Vec<(&str, String)> {
    let mut seen_values = BTreeMap::new();
    attributes
        .into_iter()
        .filter(|(attribute, value)| {
            if value.chars().count() <= 5 {
                return true;
            }
            if seen_values.contains_key(value) && !is_duplicate_protected_attribute(attribute) {
                return false;
            }
            seen_values.insert(value.clone(), ());
            true
        })
        .collect()
}

fn element_label_values(element: &DomElementRef) -> impl Iterator<Item = &str> {
    element
        .name
        .as_deref()
        .into_iter()
        .chain(element.text.as_deref())
}

fn is_duplicate_protected_attribute(attribute: &str) -> bool {
    matches!(
        attribute,
        "format" | "expected_format" | "placeholder" | "value" | "aria-label" | "title"
    )
}

fn render_attribute_value(element: &DomElementRef, attribute: &str) -> Option<String> {
    element
        .attributes
        .get(attribute)
        .or_else(|| aliased_render_attribute(element, attribute))
        .map(|value| value.trim().to_owned())
        .or_else(|| synthetic_render_attribute(element, attribute))
        .filter(|value| !value.is_empty())
}

fn aliased_render_attribute<'a>(element: &'a DomElementRef, attribute: &str) -> Option<&'a String> {
    let alias = match attribute {
        "busy" => "aria-busy",
        "disabled" => "aria-disabled",
        "haspopup" => "aria-haspopup",
        "invalid" => "aria-invalid",
        "keyshortcuts" => "aria-keyshortcuts",
        "level" => "aria-level",
        "live" => "aria-live",
        "multiselectable" => "aria-multiselectable",
        "pressed" => "aria-pressed",
        "readonly" => "aria-readonly",
        "required" => "aria-required",
        "selected" => "aria-selected",
        "valuemax" => "aria-valuemax",
        "valuemin" => "aria-valuemin",
        "valuenow" => "aria-valuenow",
        "valuetext" => "aria-valuetext",
        _ => return None,
    };
    element.attributes.get(alias)
}

fn aliased_suppression_target(attribute: &str) -> Option<&'static str> {
    match attribute {
        "aria-expanded" => Some("expanded"),
        "aria-valuemax" => Some("valuemax"),
        "aria-valuemin" => Some("valuemin"),
        "aria-valuenow" => Some("valuenow"),
        "aria-valuetext" => Some("valuetext"),
        _ => None,
    }
}

fn synthetic_render_attribute(element: &DomElementRef, attribute: &str) -> Option<String> {
    if !element.tag_name.eq_ignore_ascii_case("input") {
        return None;
    }
    let input_type = element
        .attributes
        .get("type")
        .map(|value| value.to_ascii_lowercase())
        .unwrap_or_default();

    match attribute {
        "format" => native_input_format(&input_type)
            .map(str::to_owned)
            .or_else(|| text_datepicker_format(&input_type, &element.attributes)),
        "expected_format" => text_datepicker_expected_format(&input_type, &element.attributes),
        "placeholder" => native_input_placeholder(&input_type, &element.attributes)
            .map(str::to_owned)
            .or_else(|| text_datepicker_placeholder(&input_type, &element.attributes)),
        _ => None,
    }
}

fn native_input_format(input_type: &str) -> Option<&'static str> {
    match input_type {
        "date" => Some("YYYY-MM-DD"),
        "time" => Some("HH:MM"),
        "datetime-local" => Some("YYYY-MM-DDTHH:MM"),
        "month" => Some("YYYY-MM"),
        "week" => Some("YYYY-W##"),
        _ => None,
    }
}

fn native_input_placeholder(
    input_type: &str,
    attributes: &BTreeMap<String, String>,
) -> Option<&'static str> {
    match input_type {
        "date" => Some("YYYY-MM-DD"),
        "time" => Some("HH:MM"),
        "datetime-local" => Some("YYYY-MM-DDTHH:MM"),
        "month" => Some("YYYY-MM"),
        "week" => Some("YYYY-W##"),
        "tel" if !attributes.contains_key("pattern") => Some("123-456-7890"),
        _ => None,
    }
}

fn text_datepicker_expected_format(
    input_type: &str,
    attributes: &BTreeMap<String, String>,
) -> Option<String> {
    if !matches!(input_type, "" | "text") {
        return None;
    }
    nonempty_attribute(attributes, "uib-datepicker-popup").map(str::to_owned)
}

fn text_datepicker_format(
    input_type: &str,
    attributes: &BTreeMap<String, String>,
) -> Option<String> {
    if let Some(format) = text_datepicker_expected_format(input_type, attributes) {
        return Some(format);
    }
    text_datepicker_placeholder(input_type, attributes)
}

fn text_datepicker_placeholder(
    input_type: &str,
    attributes: &BTreeMap<String, String>,
) -> Option<String> {
    if !matches!(input_type, "" | "text") {
        return None;
    }
    if !has_text_datepicker_signal(attributes) {
        return None;
    }
    Some(
        nonempty_attribute(attributes, "data-date-format")
            .unwrap_or("mm/dd/yyyy")
            .to_owned(),
    )
}

fn has_text_datepicker_signal(attributes: &BTreeMap<String, String>) -> bool {
    attributes
        .get("class")
        .map(|value| {
            let value = value.to_ascii_lowercase();
            ["datepicker", "datetimepicker", "daterangepicker"]
                .iter()
                .any(|indicator| value.contains(indicator))
        })
        .unwrap_or(false)
        || attributes.contains_key("data-datepicker")
}

fn nonempty_attribute<'a>(attributes: &'a BTreeMap<String, String>, name: &str) -> Option<&'a str> {
    attributes
        .get(name)
        .map(String::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn cap_attribute_value(value: &str) -> String {
    const MAX_ATTRIBUTE_CHARS: usize = 100;
    if value.chars().count() <= MAX_ATTRIBUTE_CHARS {
        return value.to_owned();
    }
    let mut capped = value.chars().take(MAX_ATTRIBUTE_CHARS).collect::<String>();
    capped.push_str("...");
    capped
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

#[cfg(test)]
mod tests {
    use super::*;

    fn test_element(
        index: u32,
        tag_name: &str,
        attributes: BTreeMap<String, String>,
    ) -> DomElementRef {
        DomElementRef {
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

    #[test]
    fn short_target_id_matches_browser_use_display_shape() {
        let tab = TabInfo {
            url: "about:blank".to_owned(),
            title: "Blank".to_owned(),
            tab_id: TabInfo::tab_id_for_target("123456abcdef"),
            target_id: "123456abcdef".to_owned(),
            parent_target_id: None,
        };

        assert_eq!(tab.short_target_id(), "cdef");
    }

    #[test]
    fn tab_info_serializes_browser_use_short_tab_id() {
        let tab = TabInfo {
            url: "about:blank".to_owned(),
            title: "Blank".to_owned(),
            tab_id: TabInfo::tab_id_for_target("123456abcdef"),
            target_id: "123456abcdef".to_owned(),
            parent_target_id: None,
        };

        let value = serde_json::to_value(tab).expect("tab json");

        assert_eq!(value["tab_id"], "cdef");
        assert_eq!(value["target_id"], "123456abcdef");
    }

    #[test]
    fn empty_dom_state_has_zero_elements() {
        let dom = SerializedDomState::default();
        assert_eq!(dom.element_count(), 0);
        assert_eq!(dom.llm_representation(), EMPTY_DOM_TREE_MESSAGE);
        assert_eq!(
            dom.llm_representation_with_attributes(&["data-testid".to_owned()]),
            EMPTY_DOM_TREE_MESSAGE
        );
        assert_eq!(dom.eval_representation(), EMPTY_DOM_TREE_MESSAGE);
    }

    #[test]
    fn eval_representation_preserves_tree_structure_without_action_indexes() {
        let root = DomEvalNode::element("html").with_children(vec![
            DomEvalNode::element("body").with_children(vec![
                DomEvalNode::element("main").with_children(vec![
                    DomEvalNode::element("h1")
                        .with_children(vec![DomEvalNode::text("Account setup")]),
                    DomEvalNode::element("form").with_children(vec![
                        DomEvalNode::element("label")
                            .with_attribute("for", "email")
                            .with_children(vec![DomEvalNode::text("Email")]),
                        DomEvalNode::element("input")
                            .with_attribute("id", "email")
                            .with_attribute("name", "email")
                            .with_attribute("type", "email")
                            .with_attribute("placeholder", "you@example.com")
                            .with_attribute("required", "true")
                            .interactive(42),
                        DomEvalNode::element("button")
                            .with_attribute("data-testid", "submit-account")
                            .with_children(vec![DomEvalNode::text("Continue")])
                            .interactive(43),
                    ]),
                ]),
            ]),
        ]);
        let state = SerializedDomState::default().with_eval_root(root);

        assert_eq!(
            state.eval_representation(),
            "<html />\n\t<body />\n\t\t<main />\n\t\t\t<h1>Account setup\n\t\t\t<form />\n\t\t\t\t<label>Email\n\t\t\t\t[i_42] <input id=\"email\" name=\"email\" type=\"email\" placeholder=\"you@example.com\" required=\"true\" />\n\t\t\t\t[i_43] <button data-testid=\"submit-account\">Continue"
        );
    }

    #[test]
    fn eval_representation_handles_shadow_iframe_svg_and_scroll_markers() {
        let root = DomEvalNode::element("body").with_children(vec![
            DomEvalNode::element("section")
                .scrollable("0.0 pages above, 2.0 pages below")
                .with_children(vec![
                    DomEvalNode::element("button")
                        .with_attribute("aria-expanded", "false")
                        .with_children(vec![DomEvalNode::text("Open menu")])
                        .interactive(51),
                ]),
            DomEvalNode::document_fragment(vec![
                DomEvalNode::element("button")
                    .with_attribute("id", "shadow-save")
                    .with_children(vec![DomEvalNode::text("Save")])
                    .interactive(52),
            ]),
            DomEvalNode::element("iframe")
                .with_attribute("title", "Checkout")
                .with_children(vec![
                    DomEvalNode::element("button")
                        .with_children(vec![DomEvalNode::text("Pay now")])
                        .interactive(53),
                ]),
            DomEvalNode::element("svg")
                .with_attribute("aria-label", "Decorative chart")
                .with_children(vec![
                    DomEvalNode::element("path").with_attribute("d", "M0 0"),
                ]),
        ]);
        let state = SerializedDomState::default().with_eval_root(root);

        assert_eq!(
            state.eval_representation(),
            "<body />\n\t<section scroll=\"0.0 pages above, 2.0 pages below\" />\n\t\t[i_51] <button aria-expanded=\"false\">Open menu\n\t#shadow\n\t\t[i_52] <button id=\"shadow-save\">Save\n\t<iframe title=\"Checkout\" />\n\t\t#iframe-content\n\t\t\t[i_53] <button>Pay now\n\t<svg aria-label=\"Decorative chart\" /> <!-- SVG content collapsed -->"
        );
    }

    #[test]
    fn serialized_state_renders_label_and_current_value() {
        let element = DomElementRef {
            index: 1,
            target_id: "target".to_owned(),
            backend_node_id: 0,
            node_id: None,
            tag_name: "input".to_owned(),
            role: None,
            name: Some("Name".to_owned()),
            text: Some("EvalOps".to_owned()),
            attributes: BTreeMap::new(),
            bounds: Some(ElementBounds {
                x: 10,
                y: 20,
                width: 120,
                height: 32,
            }),
            is_visible: true,
            is_interactive: true,
            is_scrollable: false,
        };

        let state = SerializedDomState::from_elements(vec![element]);

        assert_eq!(state.llm_representation(), "[1] <input> Name EvalOps");
        assert_eq!(state.element_count(), 1);
        assert_eq!(state.selector_map[&1].bounds.expect("bounds").width, 120);
    }

    #[test]
    fn interacted_element_records_upstream_style_metadata() {
        let element = DomElementRef {
            index: 7,
            target_id: "target-abc123".to_owned(),
            backend_node_id: 42,
            node_id: Some(11),
            tag_name: "button".to_owned(),
            role: Some("button".to_owned()),
            name: Some("Save".to_owned()),
            text: Some("Save changes".to_owned()),
            attributes: BTreeMap::from([
                ("id".to_owned(), "save-button".to_owned()),
                ("class".to_owned(), "btn primary hover".to_owned()),
                ("ax_name".to_owned(), "Save".to_owned()),
            ]),
            bounds: Some(ElementBounds {
                x: 10,
                y: 20,
                width: 120,
                height: 32,
            }),
            is_visible: true,
            is_interactive: true,
            is_scrollable: false,
        };

        let interacted = DomInteractedElement::from_element(&element);

        assert_eq!(interacted.target_id, "target-abc123");
        assert_eq!(interacted.node_id, 11);
        assert_eq!(interacted.backend_node_id, 42);
        assert_eq!(interacted.node_type, 1);
        assert_eq!(interacted.node_name, "button");
        assert_eq!(interacted.node_value, "Save changes");
        assert_eq!(interacted.x_path, "//button[@id='save-button']");
        assert_eq!(interacted.ax_name.as_deref(), Some("Save"));
        assert_eq!(interacted.bounds.expect("bounds").width, 120);
        assert_ne!(interacted.element_hash, 0);
        assert!(interacted.stable_hash.is_some());
    }

    #[test]
    fn interacted_element_stable_hash_filters_dynamic_classes() {
        let first = DomElementRef {
            index: 1,
            target_id: "target".to_owned(),
            backend_node_id: 1,
            node_id: Some(1),
            tag_name: "button".to_owned(),
            role: None,
            name: Some("Save".to_owned()),
            text: Some("Save".to_owned()),
            attributes: BTreeMap::from([
                ("class".to_owned(), "btn primary hover selected".to_owned()),
                ("data-testid".to_owned(), "save".to_owned()),
                ("ax_name".to_owned(), "Save".to_owned()),
            ]),
            bounds: None,
            is_visible: true,
            is_interactive: true,
            is_scrollable: false,
        };
        let second = DomElementRef {
            attributes: BTreeMap::from([
                ("class".to_owned(), "active primary btn loading".to_owned()),
                ("data-testid".to_owned(), "save".to_owned()),
                ("ax_name".to_owned(), "Save".to_owned()),
            ]),
            ..first.clone()
        };

        let first = DomInteractedElement::from_element(&first);
        let second = DomInteractedElement::from_element(&second);

        assert_ne!(first.element_hash, second.element_hash);
        assert_eq!(first.stable_hash, second.stable_hash);
        assert_eq!(first.x_path, second.x_path);
    }

    #[test]
    fn interacted_element_uses_element_name_as_ax_fallback() {
        let element = DomElementRef {
            index: 3,
            target_id: "target".to_owned(),
            backend_node_id: 3,
            node_id: Some(3),
            tag_name: "button".to_owned(),
            role: Some("button".to_owned()),
            name: Some("Fallback Label".to_owned()),
            text: Some("Fallback Label".to_owned()),
            attributes: BTreeMap::from([("id".to_owned(), "fallback".to_owned())]),
            bounds: None,
            is_visible: true,
            is_interactive: true,
            is_scrollable: false,
        };

        let interacted = DomInteractedElement::from_element(&element);

        assert_eq!(interacted.ax_name.as_deref(), Some("Fallback Label"));
    }

    #[test]
    fn interacted_element_rematches_exact_hash_across_targets() {
        let historical = DomInteractedElement::from_element(&test_element(
            1,
            "button",
            BTreeMap::from([
                ("id".to_owned(), "save".to_owned()),
                ("class".to_owned(), "btn primary".to_owned()),
            ]),
        ));
        let current = test_element(
            9,
            "button",
            BTreeMap::from([
                ("id".to_owned(), "save".to_owned()),
                ("class".to_owned(), "btn primary".to_owned()),
            ]),
        );
        let state = SerializedDomState::from_elements(vec![current]);

        assert_eq!(
            historical.rematch(&state).expect("exact match"),
            DomInteractedElementMatch {
                index: 9,
                level: DomInteractedElementMatchLevel::Exact,
                attribute: None,
            }
        );
    }

    #[test]
    fn interacted_element_rematches_stable_hash_after_dynamic_class_change() {
        let historical = DomInteractedElement::from_element(&test_element(
            1,
            "button",
            BTreeMap::from([
                ("data-testid".to_owned(), "save".to_owned()),
                ("class".to_owned(), "btn primary hover selected".to_owned()),
            ]),
        ));
        let current = test_element(
            4,
            "button",
            BTreeMap::from([
                ("data-testid".to_owned(), "save".to_owned()),
                ("class".to_owned(), "active primary btn loading".to_owned()),
            ]),
        );
        let state = SerializedDomState::from_elements(vec![current]);

        assert_eq!(
            historical.rematch(&state).expect("stable match").level,
            DomInteractedElementMatchLevel::Stable
        );
        assert_eq!(historical.rematch(&state).expect("stable match").index, 4);
    }

    #[test]
    fn interacted_element_rematches_xpath_when_hashes_are_unavailable() {
        let current = test_element(
            5,
            "a",
            BTreeMap::from([("id".to_owned(), "docs".to_owned())]),
        );
        let mut historical = DomInteractedElement::from_element(&current);
        historical.element_hash = 0;
        historical.stable_hash = None;
        let state = SerializedDomState::from_elements(vec![current]);

        assert_eq!(
            historical.rematch(&state).expect("xpath match"),
            DomInteractedElementMatch {
                index: 5,
                level: DomInteractedElementMatchLevel::XPath,
                attribute: None,
            }
        );
    }

    #[test]
    fn interacted_element_rematches_ax_name_when_structure_changes() {
        let mut historical =
            DomInteractedElement::from_element(&test_element(1, "button", BTreeMap::new()));
        historical.element_hash = 0;
        historical.stable_hash = None;
        historical.x_path = String::new();
        historical.ax_name = Some("Open menu".to_owned());
        let mut current = test_element(8, "button", BTreeMap::new());
        current.name = Some("Open menu".to_owned());
        let state = SerializedDomState::from_elements(vec![current]);

        assert_eq!(
            historical.rematch(&state).expect("ax match").level,
            DomInteractedElementMatchLevel::AxName
        );
    }

    #[test]
    fn interacted_element_rematches_unique_attribute_fallback() {
        let mut historical = DomInteractedElement::from_element(&test_element(
            1,
            "input",
            BTreeMap::from([("name".to_owned(), "email".to_owned())]),
        ));
        historical.element_hash = 0;
        historical.stable_hash = None;
        historical.x_path = String::new();
        historical.ax_name = None;
        let state = SerializedDomState::from_elements(vec![
            test_element(
                3,
                "input",
                BTreeMap::from([("name".to_owned(), "email".to_owned())]),
            ),
            test_element(
                4,
                "input",
                BTreeMap::from([("name".to_owned(), "password".to_owned())]),
            ),
        ]);

        assert_eq!(
            historical.rematch(&state).expect("attribute match"),
            DomInteractedElementMatch {
                index: 3,
                level: DomInteractedElementMatchLevel::Attribute,
                attribute: Some("name".to_owned()),
            }
        );
    }

    #[test]
    fn interacted_element_rematch_reports_ambiguous_and_missing_matches() {
        let mut historical =
            DomInteractedElement::from_element(&test_element(1, "button", BTreeMap::new()));
        historical.element_hash = 0;
        historical.stable_hash = None;
        historical.x_path = String::new();
        historical.ax_name = Some("Duplicate".to_owned());
        let mut first = test_element(1, "button", BTreeMap::new());
        first.name = Some("Duplicate".to_owned());
        let mut second = test_element(2, "button", BTreeMap::new());
        second.name = Some("Duplicate".to_owned());
        let state = SerializedDomState::from_elements(vec![first, second]);

        let error = historical.rematch(&state).expect_err("ambiguous match");
        assert_eq!(
            error.reason,
            DomInteractedElementMatchFailureReason::Ambiguous
        );
        assert_eq!(error.level, Some(DomInteractedElementMatchLevel::AxName));
        assert_eq!(error.candidate_indices, vec![1, 2]);

        let empty = historical
            .rematch(&SerializedDomState::default())
            .expect_err("empty selector map");
        assert_eq!(
            empty.reason,
            DomInteractedElementMatchFailureReason::EmptySelectorMap
        );

        historical.ax_name = Some("Missing".to_owned());
        let missing = historical.rematch(&state).expect_err("missing match");
        assert_eq!(
            missing.reason,
            DomInteractedElementMatchFailureReason::NotFound
        );
    }

    #[test]
    fn serialized_state_renders_compound_component_metadata() {
        let element = DomElementRef {
            index: 1,
            target_id: "target".to_owned(),
            backend_node_id: 0,
            node_id: None,
            tag_name: "select".to_owned(),
            role: None,
            name: Some("Plan".to_owned()),
            text: Some("Enterprise".to_owned()),
            attributes: BTreeMap::from([
                ("id".to_owned(), "plan".to_owned()),
                ("value".to_owned(), "Enterprise".to_owned()),
                (
                    "compound_components".to_owned(),
                    "(name=Dropdown Toggle,role=button),(name=Options,role=listbox,count=2,options=Starter|Enterprise)"
                        .to_owned(),
                ),
            ]),
            bounds: None,
            is_visible: true,
            is_interactive: true,
            is_scrollable: false,
        };

        let state = SerializedDomState::from_elements(vec![element]);

        assert_eq!(
            state.llm_representation(),
            "[1] <select id=plan value=Enterprise compound_components=(name=Dropdown Toggle,role=button),(name=Options,role=listbox,count=2,options=Starter|Enterprise)> Plan Enterprise"
        );
    }

    #[test]
    fn serialized_state_renders_included_attributes_without_password_values() {
        let visible = DomElementRef {
            index: 1,
            target_id: "target".to_owned(),
            backend_node_id: 0,
            node_id: None,
            tag_name: "input".to_owned(),
            role: None,
            name: Some("Email".to_owned()),
            text: Some("user@example.com".to_owned()),
            attributes: BTreeMap::from([
                ("aria-required".to_owned(), "true".to_owned()),
                ("id".to_owned(), "email".to_owned()),
                ("placeholder".to_owned(), "name@example.com".to_owned()),
                ("type".to_owned(), "email".to_owned()),
                ("value".to_owned(), "user@example.com".to_owned()),
            ]),
            bounds: None,
            is_visible: true,
            is_interactive: true,
            is_scrollable: false,
        };
        let password = DomElementRef {
            index: 2,
            target_id: "target".to_owned(),
            backend_node_id: 0,
            node_id: None,
            tag_name: "input".to_owned(),
            role: None,
            name: Some("Password".to_owned()),
            text: None,
            attributes: BTreeMap::from([
                ("placeholder".to_owned(), "Password".to_owned()),
                ("type".to_owned(), "password".to_owned()),
                ("value".to_owned(), "secret".to_owned()),
            ]),
            bounds: None,
            is_visible: true,
            is_interactive: true,
            is_scrollable: false,
        };

        let state = SerializedDomState::from_elements(vec![visible, password]);

        assert!(state.llm_representation().contains(
            "[1] <input type=email id=email value=user@example.com placeholder=name@example.com required=true> Email user@example.com"
        ));
        assert!(
            state
                .llm_representation()
                .contains("[2] <input type=password> Password")
        );
        assert!(!state.llm_representation().contains("secret"));
    }

    #[test]
    fn rendered_attributes_do_not_duplicate_direct_aria_state() {
        let element = DomElementRef {
            index: 1,
            target_id: "target".to_owned(),
            backend_node_id: 0,
            node_id: None,
            tag_name: "button".to_owned(),
            role: None,
            name: Some("Toggle details".to_owned()),
            text: None,
            attributes: BTreeMap::from([
                ("aria-checked".to_owned(), "true".to_owned()),
                ("aria-expanded".to_owned(), "true".to_owned()),
            ]),
            bounds: None,
            is_visible: true,
            is_interactive: true,
            is_scrollable: false,
        };

        let attributes = render_element_attributes(&element);

        assert_eq!(attributes, "aria-expanded=true aria-checked=true");
    }

    #[test]
    fn rendered_attributes_prefer_ax_expanded_over_aria_expanded() {
        let element = DomElementRef {
            index: 1,
            target_id: "target".to_owned(),
            backend_node_id: 0,
            node_id: None,
            tag_name: "button".to_owned(),
            role: None,
            name: Some("Toggle details".to_owned()),
            text: None,
            attributes: BTreeMap::from([
                ("aria-expanded".to_owned(), "false".to_owned()),
                ("expanded".to_owned(), "true".to_owned()),
            ]),
            bounds: None,
            is_visible: true,
            is_interactive: true,
            is_scrollable: false,
        };

        let attributes = render_element_attributes(&element);

        assert_eq!(attributes, "expanded=true");
    }

    #[test]
    fn rendered_attributes_drop_redundant_role_matching_tag_name() {
        let native_role = DomElementRef {
            index: 1,
            target_id: "target".to_owned(),
            backend_node_id: 0,
            node_id: None,
            tag_name: "button".to_owned(),
            role: Some("button".to_owned()),
            name: Some("Submit request".to_owned()),
            text: None,
            attributes: BTreeMap::from([("role".to_owned(), "button".to_owned())]),
            bounds: None,
            is_visible: true,
            is_interactive: true,
            is_scrollable: false,
        };
        let semantic_override = DomElementRef {
            tag_name: "div".to_owned(),
            attributes: BTreeMap::from([("role".to_owned(), "button".to_owned())]),
            ..native_role.clone()
        };

        assert_eq!(render_element_attributes(&native_role), "");
        assert_eq!(render_element_attributes(&semantic_override), "role=button");
    }

    #[test]
    fn rendered_attributes_alias_aria_keyshortcuts_to_ax_shape() {
        let element = DomElementRef {
            index: 1,
            target_id: "target".to_owned(),
            backend_node_id: 0,
            node_id: None,
            tag_name: "div".to_owned(),
            role: None,
            name: Some("Submit request".to_owned()),
            text: None,
            attributes: BTreeMap::from([(
                "aria-keyshortcuts".to_owned(),
                "Control+Enter".to_owned(),
            )]),
            bounds: None,
            is_visible: true,
            is_interactive: true,
            is_scrollable: false,
        };

        let attributes = render_element_attributes(&element);

        assert_eq!(attributes, "keyshortcuts=Control+Enter");
    }

    #[test]
    fn rendered_attributes_alias_aria_live_region_metadata() {
        let element = DomElementRef {
            index: 1,
            target_id: "target".to_owned(),
            backend_node_id: 0,
            node_id: None,
            tag_name: "section".to_owned(),
            role: Some("listbox".to_owned()),
            name: Some("Results".to_owned()),
            text: None,
            attributes: BTreeMap::from([
                ("aria-busy".to_owned(), "true".to_owned()),
                ("aria-level".to_owned(), "2".to_owned()),
                ("aria-live".to_owned(), "polite".to_owned()),
                ("aria-multiselectable".to_owned(), "true".to_owned()),
            ]),
            bounds: None,
            is_visible: true,
            is_interactive: true,
            is_scrollable: false,
        };

        let attributes = render_element_attributes(&element);

        assert_eq!(
            attributes,
            "multiselectable=true level=2 busy=true live=polite"
        );
    }

    #[test]
    fn rendered_attributes_omit_readonly_by_default_but_allow_custom_include() {
        let read_only = DomElementRef {
            index: 1,
            target_id: "target".to_owned(),
            backend_node_id: 0,
            node_id: None,
            tag_name: "input".to_owned(),
            role: None,
            name: Some("Invoice id".to_owned()),
            text: Some("INV-123".to_owned()),
            attributes: BTreeMap::from([("aria-readonly".to_owned(), "true".to_owned())]),
            bounds: None,
            is_visible: true,
            is_interactive: true,
            is_scrollable: false,
        };
        let writable = DomElementRef {
            index: 2,
            target_id: "target".to_owned(),
            backend_node_id: 0,
            node_id: None,
            tag_name: "input".to_owned(),
            role: None,
            name: Some("Notes".to_owned()),
            text: None,
            attributes: BTreeMap::from([("aria-readonly".to_owned(), "false".to_owned())]),
            bounds: None,
            is_visible: true,
            is_interactive: true,
            is_scrollable: false,
        };

        let state = SerializedDomState::from_elements(vec![read_only.clone(), writable.clone()]);

        assert!(
            state
                .llm_representation()
                .contains("[1] <input> Invoice id INV-123")
        );
        assert!(state.llm_representation().contains("[2] <input> Notes"));
        assert_eq!(
            render_element_attributes_with_attributes(&read_only, &["readonly".to_owned()]),
            "readonly=true"
        );
        assert_eq!(
            render_element_attributes_with_attributes(&writable, &["readonly".to_owned()]),
            "readonly=false"
        );
    }

    #[test]
    fn rendered_attributes_keep_false_ax_states_except_required_and_invalid() {
        let element = DomElementRef {
            index: 1,
            target_id: "target".to_owned(),
            backend_node_id: 0,
            node_id: None,
            tag_name: "button".to_owned(),
            role: None,
            name: Some("Toggle filters".to_owned()),
            text: None,
            attributes: BTreeMap::from([
                ("disabled".to_owned(), "false".to_owned()),
                ("expanded".to_owned(), "false".to_owned()),
                ("invalid".to_owned(), "false".to_owned()),
                ("pressed".to_owned(), "false".to_owned()),
                ("required".to_owned(), "false".to_owned()),
                ("selected".to_owned(), "false".to_owned()),
            ]),
            bounds: None,
            is_visible: true,
            is_interactive: true,
            is_scrollable: false,
        };

        assert_eq!(
            render_element_attributes(&element),
            "selected=false expanded=false pressed=false disabled=false"
        );
    }

    #[test]
    fn rendered_attributes_drop_duplicate_long_values() {
        let element = DomElementRef {
            index: 1,
            target_id: "target".to_owned(),
            backend_node_id: 0,
            node_id: None,
            tag_name: "button".to_owned(),
            role: None,
            name: Some("Checkout".to_owned()),
            text: None,
            attributes: BTreeMap::from([
                ("id".to_owned(), "customer-checkout-button".to_owned()),
                ("name".to_owned(), "customer-checkout-button".to_owned()),
                ("value".to_owned(), "customer-checkout-button".to_owned()),
            ]),
            bounds: None,
            is_visible: true,
            is_interactive: true,
            is_scrollable: false,
        };

        let attributes = render_element_attributes(&element);

        assert_eq!(
            attributes,
            "id=customer-checkout-button value=customer-checkout-button"
        );
    }

    #[test]
    fn rendered_attributes_alias_aria_value_metadata() {
        let element = DomElementRef {
            index: 1,
            target_id: "target".to_owned(),
            backend_node_id: 0,
            node_id: None,
            tag_name: "div".to_owned(),
            role: Some("slider".to_owned()),
            name: Some("Volume".to_owned()),
            text: None,
            attributes: BTreeMap::from([
                ("aria-valuemax".to_owned(), "10".to_owned()),
                ("aria-valuemin".to_owned(), "0".to_owned()),
                ("aria-valuenow".to_owned(), "7".to_owned()),
                ("aria-valuetext".to_owned(), "Seven".to_owned()),
            ]),
            bounds: None,
            is_visible: true,
            is_interactive: true,
            is_scrollable: false,
        };

        let attributes = render_element_attributes(&element);

        assert_eq!(
            attributes,
            "valuemin=0 valuemax=10 valuenow=7 valuetext=Seven"
        );
    }

    #[test]
    fn rendered_attributes_add_native_input_format_hints() {
        let date = DomElementRef {
            index: 1,
            target_id: "target".to_owned(),
            backend_node_id: 0,
            node_id: None,
            tag_name: "input".to_owned(),
            role: None,
            name: Some("Start date".to_owned()),
            text: None,
            attributes: BTreeMap::from([
                ("id".to_owned(), "start".to_owned()),
                ("type".to_owned(), "date".to_owned()),
            ]),
            bounds: None,
            is_visible: true,
            is_interactive: true,
            is_scrollable: false,
        };
        let tel = DomElementRef {
            index: 2,
            target_id: "target".to_owned(),
            backend_node_id: 0,
            node_id: None,
            tag_name: "input".to_owned(),
            role: None,
            name: Some("Phone".to_owned()),
            text: None,
            attributes: BTreeMap::from([("type".to_owned(), "tel".to_owned())]),
            bounds: None,
            is_visible: true,
            is_interactive: true,
            is_scrollable: false,
        };

        let state = SerializedDomState::from_elements(vec![date, tel]);

        assert!(state.llm_representation().contains(
            "[1] <input type=date id=start placeholder=YYYY-MM-DD format=YYYY-MM-DD> Start date"
        ));
        assert!(
            state
                .llm_representation()
                .contains("[2] <input type=tel placeholder=123-456-7890> Phone")
        );
    }

    #[test]
    fn rendered_attributes_add_text_datepicker_format_hints() {
        let jquery_datepicker = DomElementRef {
            index: 1,
            target_id: "target".to_owned(),
            backend_node_id: 0,
            node_id: None,
            tag_name: "input".to_owned(),
            role: None,
            name: Some("Travel date".to_owned()),
            text: None,
            attributes: BTreeMap::from([
                ("class".to_owned(), "form-control datepicker".to_owned()),
                ("data-date-format".to_owned(), "dd/mm/yyyy".to_owned()),
                ("type".to_owned(), "text".to_owned()),
            ]),
            bounds: None,
            is_visible: true,
            is_interactive: true,
            is_scrollable: false,
        };
        let angular_datepicker = DomElementRef {
            index: 2,
            target_id: "target".to_owned(),
            backend_node_id: 0,
            node_id: None,
            tag_name: "input".to_owned(),
            role: None,
            name: Some("Start".to_owned()),
            text: None,
            attributes: BTreeMap::from([(
                "uib-datepicker-popup".to_owned(),
                "MM/dd/yyyy".to_owned(),
            )]),
            bounds: None,
            is_visible: true,
            is_interactive: true,
            is_scrollable: false,
        };
        let default_datepicker = DomElementRef {
            index: 3,
            target_id: "target".to_owned(),
            backend_node_id: 0,
            node_id: None,
            tag_name: "input".to_owned(),
            role: None,
            name: Some("Fallback".to_owned()),
            text: None,
            attributes: BTreeMap::from([("data-datepicker".to_owned(), "true".to_owned())]),
            bounds: None,
            is_visible: true,
            is_interactive: true,
            is_scrollable: false,
        };
        let state = SerializedDomState::from_elements(vec![
            jquery_datepicker,
            angular_datepicker,
            default_datepicker,
        ]);

        assert!(state.llm_representation().contains(
            "[1] <input type=text placeholder=dd/mm/yyyy format=dd/mm/yyyy> Travel date"
        ));
        assert!(
            state
                .llm_representation()
                .contains("[2] <input format=MM/dd/yyyy expected_format=MM/dd/yyyy> Start")
        );
        assert!(state.llm_representation().contains(
            "[3] <input placeholder=mm/dd/yyyy data-datepicker=true format=mm/dd/yyyy> Fallback"
        ));
    }

    #[test]
    fn serialized_state_marks_scrollable_elements_for_agent() {
        let element = DomElementRef {
            index: 7,
            target_id: "target".to_owned(),
            backend_node_id: 0,
            node_id: None,
            tag_name: "section".to_owned(),
            role: None,
            name: Some("Results".to_owned()),
            text: None,
            attributes: BTreeMap::from([("id".to_owned(), "results".to_owned())]),
            bounds: None,
            is_visible: true,
            is_interactive: true,
            is_scrollable: true,
        };

        let state = SerializedDomState::from_elements(vec![element]);

        assert_eq!(
            state.llm_representation(),
            "[7] |scroll element| <section id=results> Results"
        );
    }

    #[test]
    fn serialized_state_renders_scroll_context_for_agent() {
        let element = DomElementRef {
            index: 7,
            target_id: "target".to_owned(),
            backend_node_id: 0,
            node_id: None,
            tag_name: "section".to_owned(),
            role: None,
            name: Some("Results".to_owned()),
            text: None,
            attributes: BTreeMap::from([
                ("id".to_owned(), "results".to_owned()),
                (
                    "scroll".to_owned(),
                    "0.0 pages above, 5.7 pages below".to_owned(),
                ),
            ]),
            bounds: None,
            is_visible: true,
            is_interactive: true,
            is_scrollable: true,
        };

        let state = SerializedDomState::from_elements(vec![element]);

        assert_eq!(
            state.llm_representation(),
            "[7] |scroll element| <section id=results> (0.0 pages above, 5.7 pages below) Results"
        );
    }

    #[test]
    fn ax_description_requires_explicit_include_attribute() {
        let element = DomElementRef {
            index: 1,
            target_id: "target".to_owned(),
            backend_node_id: 0,
            node_id: None,
            tag_name: "button".to_owned(),
            role: None,
            name: Some("Submit".to_owned()),
            text: None,
            attributes: BTreeMap::from([(
                "description".to_owned(),
                "Sends the completed form".to_owned(),
            )]),
            bounds: None,
            is_visible: true,
            is_interactive: true,
            is_scrollable: false,
        };

        assert_eq!(render_element_attributes(&element), "");
        assert_eq!(
            render_element_attributes_with_attributes(&element, &["description".to_owned()]),
            "description=Sends the completed form"
        );
    }

    #[test]
    fn rendered_attributes_include_distinct_accessibility_snapshot_metadata() {
        let element = DomElementRef {
            index: 1,
            target_id: "target".to_owned(),
            backend_node_id: 0,
            node_id: None,
            tag_name: "button".to_owned(),
            role: None,
            name: Some("Submit".to_owned()),
            text: None,
            attributes: BTreeMap::from([
                ("ax_name".to_owned(), "Submit".to_owned()),
                (
                    "ax_description".to_owned(),
                    "Sends the completed checkout form".to_owned(),
                ),
            ]),
            bounds: None,
            is_visible: true,
            is_interactive: true,
            is_scrollable: false,
        };

        assert_eq!(
            render_element_attributes(&element),
            "ax_description=Sends the completed checkout form"
        );
    }

    #[test]
    fn llm_representation_can_use_custom_include_attributes() {
        let element = DomElementRef {
            index: 1,
            target_id: "target".to_owned(),
            backend_node_id: 0,
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
        };
        let state = SerializedDomState::from_elements(vec![element]);

        assert_eq!(state.llm_representation(), "[1] <button id=run> Run");
        assert_eq!(
            state.llm_representation_with_attributes(&["data-testid".to_owned()]),
            "[1] <button data-testid=run-action> Run"
        );
    }
}
