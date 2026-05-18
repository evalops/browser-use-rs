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

pub const EMPTY_DOM_TREE_MESSAGE: &str =
    "Empty DOM tree (you might have to wait for the page to load)";

/// Information about an open tab or page target.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct TabInfo {
    pub url: String,
    pub title: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub tab_id: String,
    pub target_id: TargetId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_target_id: Option<TargetId>,
}

impl TabInfo {
    #[must_use]
    pub fn short_target_id(&self) -> &str {
        if !self.tab_id.is_empty() {
            return &self.tab_id;
        }
        Self::short_target_id_for(&self.target_id)
    }

    #[must_use]
    pub fn short_target_id_for(target_id: &str) -> &str {
        let len = target_id.len();
        &target_id[len.saturating_sub(4)..]
    }

    #[must_use]
    pub fn tab_id_for_target(target_id: &str) -> String {
        Self::short_target_id_for(target_id).to_owned()
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

/// Viewport-relative integer bounds for an indexed element.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ElementBounds {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bounds: Option<ElementBounds>,
    #[serde(default)]
    pub is_visible: bool,
    #[serde(default)]
    pub is_interactive: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub is_scrollable: bool,
}

/// Compact page-shape statistics rendered into the agent prompt.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
pub struct DomPageStats {
    #[serde(default)]
    pub links: u32,
    #[serde(default)]
    pub iframes: u32,
    #[serde(default)]
    pub shadow_open: u32,
    #[serde(default)]
    pub shadow_closed: u32,
    #[serde(default)]
    pub scroll_containers: u32,
    #[serde(default)]
    pub images: u32,
    #[serde(default)]
    pub interactive_elements: u32,
    #[serde(default)]
    pub total_elements: u32,
    #[serde(default)]
    pub text_chars: u32,
}

impl DomPageStats {
    #[must_use]
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }
}

/// Serialized DOM state in the form the agent consumes.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
pub struct SerializedDomState {
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub selector_map: BTreeMap<u32, DomElementRef>,
    #[serde(default, skip_serializing_if = "DomPageStats::is_empty")]
    pub page_stats: DomPageStats,
}

impl SerializedDomState {
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
        }
    }

    #[must_use]
    pub fn with_page_stats(mut self, page_stats: DomPageStats) -> Self {
        self.page_stats = page_stats;
        self
    }

    #[must_use]
    pub fn element_count(&self) -> usize {
        self.selector_map.len()
    }

    #[must_use]
    pub fn llm_representation(&self) -> &str {
        if self.text.is_empty() && self.selector_map.is_empty() {
            return EMPTY_DOM_TREE_MESSAGE;
        }
        self.text.as_str()
    }

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
}

#[must_use]
pub fn render_element_line(element: &DomElementRef) -> String {
    render_element_line_with_attribute_names(element, DEFAULT_RENDER_ATTRIBUTES)
}

#[must_use]
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
    "readonly",
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
];

#[must_use]
pub fn render_element_attributes(element: &DomElementRef) -> String {
    render_element_attributes_with_attribute_names(element, DEFAULT_RENDER_ATTRIBUTES)
}

#[must_use]
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
            if *attribute == "invalid" && value.eq_ignore_ascii_case("false") {
                return None;
            }
            if *attribute == "aria-expanded"
                && render_attribute_value(element, "expanded").is_some()
            {
                return None;
            }
            if matches!(
                *attribute,
                "required"
                    | "checked"
                    | "selected"
                    | "expanded"
                    | "pressed"
                    | "disabled"
                    | "readonly"
            ) && matches!(value.to_ascii_lowercase().as_str(), "false" | "0" | "no")
            {
                return None;
            }
            if matches!(*attribute, "aria-label" | "placeholder" | "title")
                && !text.is_empty()
                && value.eq_ignore_ascii_case(text.trim())
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
        "valuetext" => "aria-valuetext",
        _ => return None,
    };
    element.attributes.get(alias)
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

fn is_false(value: &bool) -> bool {
    !*value
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
    fn rendered_attributes_alias_aria_readonly_to_ax_shape() {
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

        let state = SerializedDomState::from_elements(vec![read_only, writable]);

        assert!(
            state
                .llm_representation()
                .contains("[1] <input readonly=true> Invoice id INV-123")
        );
        assert!(state.llm_representation().contains("[2] <input> Notes"));
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
    fn rendered_attributes_alias_aria_value_text() {
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
                ("aria-valuenow".to_owned(), "7".to_owned()),
                ("aria-valuetext".to_owned(), "Seven".to_owned()),
            ]),
            bounds: None,
            is_visible: true,
            is_interactive: true,
            is_scrollable: false,
        };

        let attributes = render_element_attributes(&element);

        assert_eq!(attributes, "aria-valuenow=7 valuetext=Seven");
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
