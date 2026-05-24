use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{BackendNodeId, DomElementRef, ElementBounds, NodeId, SerializedDomState, TargetId};

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
