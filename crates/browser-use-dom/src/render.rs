use std::collections::BTreeMap;

use crate::DomElementRef;

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
