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
        }
    }

    #[must_use]
    pub fn element_count(&self) -> usize {
        self.selector_map.len()
    }

    #[must_use]
    pub fn llm_representation(&self) -> &str {
        self.text.as_str()
    }
}

#[must_use]
pub fn render_element_line(element: &DomElementRef) -> String {
    let attributes = render_element_attributes(element);
    let tag = if attributes.is_empty() {
        format!("<{}>", element.tag_name)
    } else {
        format!("<{} {attributes}>", element.tag_name)
    };
    let text = render_element_text(element);
    if text.is_empty() {
        format!("[{}] {tag}", element.index)
    } else {
        format!("[{}] {tag} {text}", element.index)
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
];

#[must_use]
pub fn render_element_attributes(element: &DomElementRef) -> String {
    let is_password_field = element.tag_name.eq_ignore_ascii_case("input")
        && element
            .attributes
            .get("type")
            .is_some_and(|value| value.eq_ignore_ascii_case("password"));
    let text = render_element_text(element);

    DEFAULT_RENDER_ATTRIBUTES
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
            if matches!(
                *attribute,
                "required" | "checked" | "selected" | "expanded" | "pressed" | "disabled"
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
            Some(format!("{attribute}={}", cap_attribute_value(&value)))
        })
        .collect::<Vec<_>>()
        .join(" ")
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
        "disabled" => "aria-disabled",
        "haspopup" => "aria-haspopup",
        "invalid" => "aria-invalid",
        "pressed" => "aria-pressed",
        "required" => "aria-required",
        "selected" => "aria-selected",
        _ => return None,
    };
    element.attributes.get(alias)
}

fn synthetic_render_attribute(element: &DomElementRef, attribute: &str) -> Option<String> {
    let input_type = element.attributes.get("type")?.to_ascii_lowercase();
    if !element.tag_name.eq_ignore_ascii_case("input") {
        return None;
    }

    match attribute {
        "format" => native_input_format(&input_type).map(str::to_owned),
        "placeholder" => {
            native_input_placeholder(&input_type, &element.attributes).map(str::to_owned)
        }
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
        assert_eq!(dom.llm_representation(), "");
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
}
