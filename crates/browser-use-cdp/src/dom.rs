//! DOM, accessibility, and page-state extraction over CDP.
//!
//! This module is the bridge from raw Chrome JSON to prompt-ready
//! [`browser_use_dom`] structures. It evaluates browser scripts, gathers
//! accessibility metadata, merges iframe/shadow states, highlights elements,
//! detects pagination, and keeps selector-map indexes stable enough for action
//! execution and replay.

use crate::{
    AttachedPage, BrowserError, CachedDomElementRef, FrameElementInfo, FrameOffset,
    IframeTargetInfo, IframeTraversalConfig,
};
use browser_use_dom::{
    DomElementRef, DomEvalNode, DomEvalNodeType, DomPageStats, ElementBounds, PageInfo,
    PaginationButton, PaginationButtonType, SerializedDomState, render_element_text,
};
use serde::Serialize;
use serde_json::{Value, json};
use std::collections::{BTreeMap, VecDeque};

mod scripts;

pub(crate) use scripts::{
    AX_REF_ATTRIBUTE, CLEANUP_AX_REFS_JS, CLICK_ELEMENT_ACTION_JS, DROPDOWN_OPTIONS_BODY_JS,
    FRAME_ELEMENTS_JS, PAGE_INFO_JS, click_element_js, dom_highlight_overlay_elements,
    dom_highlight_overlay_script, dropdown_options_js, element_action_function_js,
    element_action_js, element_eval_js, element_function_js, interaction_coordinate_highlight_script,
    interaction_element_highlight_script, interactive_elements_js, scroll_to_text_js,
    select_dropdown_option_body_js, select_dropdown_option_js,
};
#[cfg(test)]
pub(crate) use scripts::INTERACTIVE_ELEMENTS_JS;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct AccessibilityNodeInfo {
    pub(crate) backend_node_id: u64,
    pub(crate) node_id: Option<u64>,
    pub(crate) role: Option<String>,
    pub(crate) name: Option<String>,
    pub(crate) properties: BTreeMap<String, String>,
}

pub(crate) fn dom_state_from_interactive_value(
    target_id: &str,
    value: &Value,
    accessibility: &BTreeMap<String, AccessibilityNodeInfo>,
) -> Result<SerializedDomState, BrowserError> {
    let stats = value
        .get("stats")
        .and_then(dom_page_stats_from_value)
        .unwrap_or_default();
    let element_values = value
        .as_array()
        .or_else(|| value.get("elements").and_then(Value::as_array))
        .ok_or_else(|| BrowserError::MissingResponseData("interactive element array".to_owned()))?;
    let elements = element_values
        .iter()
        .map(|element| dom_element_from_value(target_id, element, accessibility))
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .filter(|element| !is_ax_suppressed_interactive_element(element))
        .collect::<Vec<_>>();
    let eval_root = value
        .get("eval_tree")
        .filter(|value| !value.is_null())
        .map(|value| dom_eval_node_from_value(value, accessibility))
        .transpose()?;

    let state = SerializedDomState::from_elements(elements).with_page_stats(stats);
    Ok(match eval_root {
        Some(eval_root) => state.with_eval_root(eval_root),
        None => state,
    })
}

fn dom_eval_node_from_value(
    value: &Value,
    accessibility: &BTreeMap<String, AccessibilityNodeInfo>,
) -> Result<DomEvalNode, BrowserError> {
    let node_type = match value.get("node_type").and_then(Value::as_str) {
        Some("document_fragment") => DomEvalNodeType::DocumentFragment,
        Some("element") => DomEvalNodeType::Element,
        Some("text") => DomEvalNodeType::Text,
        Some(other) => {
            return Err(BrowserError::MissingResponseData(format!(
                "unsupported eval node type {other}"
            )));
        }
        None => {
            return Err(BrowserError::MissingResponseData(
                "eval node type".to_owned(),
            ));
        }
    };
    let ax_info = accessibility_info_for_value(value, accessibility);
    let attributes = enriched_attributes_from_value(value, ax_info);
    let children = value
        .get("children")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|child| dom_eval_node_from_value(child, accessibility))
        .collect::<Result<Vec<_>, _>>()?;
    let backend_node_id = value
        .get("backend_node_id")
        .and_then(Value::as_u64)
        .filter(|backend_node_id| *backend_node_id != 0)
        .or_else(|| ax_info.map(|info| info.backend_node_id));

    Ok(DomEvalNode {
        node_type,
        tag_name: value
            .get("tag_name")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned(),
        node_value: value
            .get("node_value")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned(),
        attributes,
        children,
        backend_node_id,
        should_display: value
            .get("should_display")
            .and_then(Value::as_bool)
            .unwrap_or(true),
        excluded_by_parent: value
            .get("excluded_by_parent")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        is_visible: value
            .get("is_visible")
            .and_then(Value::as_bool)
            .unwrap_or(true),
        is_interactive: value
            .get("is_interactive")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        is_scrollable: value
            .get("is_scrollable")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        scroll_info: value
            .get("scroll_info")
            .and_then(Value::as_str)
            .map(str::to_owned),
    })
}

fn dom_page_stats_from_value(value: &Value) -> Option<DomPageStats> {
    Some(DomPageStats {
        links: u32_field(value, "links").unwrap_or_default(),
        iframes: u32_field(value, "iframes").unwrap_or_default(),
        shadow_open: u32_field(value, "shadow_open").unwrap_or_default(),
        shadow_closed: u32_field(value, "shadow_closed").unwrap_or_default(),
        scroll_containers: u32_field(value, "scroll_containers").unwrap_or_default(),
        images: u32_field(value, "images").unwrap_or_default(),
        interactive_elements: u32_field(value, "interactive_elements").unwrap_or_default(),
        total_elements: u32_field(value, "total_elements").unwrap_or_default(),
        text_chars: u32_field(value, "text_chars").unwrap_or_default(),
    })
}

pub(crate) fn frame_element_infos_from_value(
    value: &Value,
) -> Result<Vec<FrameElementInfo>, BrowserError> {
    let encoded = value.as_str().ok_or_else(|| {
        BrowserError::MissingResponseData("iframe element info string".to_owned())
    })?;
    let frames: Value = serde_json::from_str(encoded)
        .map_err(|error| BrowserError::Transport(error.to_string()))?;
    let frames = frames
        .as_array()
        .ok_or_else(|| BrowserError::MissingResponseData("iframe element info array".to_owned()))?;

    Ok(frames
        .iter()
        .filter_map(|frame| {
            let url = frame.get("url")?.as_str()?.to_owned();
            let offset = FrameOffset {
                x: i32_field(frame, "x").unwrap_or_default(),
                y: i32_field(frame, "y").unwrap_or_default(),
            };
            Some(FrameElementInfo { url, offset })
        })
        .collect())
}

pub(crate) fn iframe_target_infos_from_targets(
    targets: &Value,
    parent_target_id: &str,
    frame_infos: &[FrameElementInfo],
    config: IframeTraversalConfig,
) -> Vec<IframeTargetInfo> {
    let depth = 1;
    if !config.allows_cross_origin_depth(depth) {
        return Vec::new();
    }
    let mut used_frames = Vec::new();
    targets
        .get("targetInfos")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|target| target.get("type").and_then(Value::as_str) == Some("iframe"))
        .filter(|target| {
            target
                .get("parentId")
                .and_then(Value::as_str)
                .is_none_or(|parent_id| parent_id == parent_target_id)
        })
        .filter_map(|target| {
            let target_id = target.get("targetId")?.as_str()?.to_owned();
            let target_url = target.get("url").and_then(Value::as_str).unwrap_or("");
            let offset = frame_offset_for_target_url(target_url, frame_infos, &mut used_frames)?;
            Some(IframeTargetInfo {
                target_id,
                offset,
                depth,
            })
        })
        .take(config.max_iframes)
        .collect()
}

pub(crate) fn frame_offset_for_target_url(
    target_url: &str,
    frame_infos: &[FrameElementInfo],
    used_frames: &mut Vec<usize>,
) -> Option<FrameOffset> {
    if frame_infos.is_empty() {
        return Some(FrameOffset::default());
    }

    let index = frame_infos
        .iter()
        .enumerate()
        .find(|(index, frame)| {
            !used_frames.contains(index) && frame_url_matches(&frame.url, target_url)
        })
        .map(|(index, _)| index)?;
    used_frames.push(index);
    Some(frame_infos[index].offset)
}

fn frame_url_matches(frame_url: &str, target_url: &str) -> bool {
    if frame_url == target_url {
        return true;
    }

    let Some(frame_url) = comparable_url(frame_url) else {
        return false;
    };
    let Some(target_url) = comparable_url(target_url) else {
        return false;
    };
    frame_url == target_url
}

fn comparable_url(value: &str) -> Option<String> {
    let mut url = url::Url::parse(value).ok()?;
    url.set_fragment(None);
    Some(url.to_string())
}

pub(crate) fn offset_dom_state_bounds(state: &mut SerializedDomState, offset: FrameOffset) {
    for element in state.selector_map.values_mut() {
        if let Some(bounds) = &mut element.bounds {
            bounds.x += offset.x;
            bounds.y += offset.y;
        }
    }
}

pub(crate) fn merge_dom_states(
    root_state: SerializedDomState,
    child_states: Vec<SerializedDomState>,
) -> SerializedDomState {
    if child_states.is_empty() {
        return root_state;
    }

    let mut root_state = root_state;
    let mut page_stats = root_state.page_stats;
    let mut eval_root = root_state.eval_root.take();
    let mut elements = dom_state_elements(root_state);
    for mut child_state in child_states {
        add_dom_page_stats(&mut page_stats, child_state.page_stats);
        if let Some(child_eval_root) = child_state.eval_root.take() {
            attach_child_eval_root(&mut eval_root, child_eval_root);
        }
        elements.extend(dom_state_elements(child_state));
    }

    for (index, element) in elements.iter_mut().enumerate() {
        element.index = u32::try_from(index + 1).unwrap_or(u32::MAX);
    }

    let state = SerializedDomState::from_elements(elements).with_page_stats(page_stats);
    match eval_root {
        Some(eval_root) => state.with_eval_root(eval_root),
        None => state,
    }
}

pub(crate) fn dom_state_elements(state: SerializedDomState) -> Vec<DomElementRef> {
    state.selector_map.into_values().collect()
}

pub(crate) fn target_local_index_for_global_index(
    selector_map: &BTreeMap<u32, DomElementRef>,
    global_index: u32,
    target_id: &str,
) -> u32 {
    let mut local_index = 0_u32;
    for (candidate_index, element) in selector_map {
        if element.target_id != target_id {
            continue;
        }
        local_index = local_index.saturating_add(1);
        if *candidate_index == global_index {
            return local_index;
        }
    }

    global_index
}

pub(crate) fn index_fallback_target_id<'a>(
    current_page: &'a AttachedPage,
    cached_element: Option<&'a CachedDomElementRef>,
) -> &'a str {
    cached_element
        .map(|cached| cached.element.target_id.as_str())
        .filter(|target_id| !target_id.is_empty())
        .unwrap_or(current_page.target_id.as_str())
}

fn attach_child_eval_root(eval_root: &mut Option<DomEvalNode>, child_eval_root: DomEvalNode) {
    let Some(root) = eval_root else {
        *eval_root = Some(child_eval_root);
        return;
    };

    let mut child_roots = VecDeque::from([child_eval_root]);
    attach_eval_roots_to_iframes(root, &mut child_roots);
    while let Some(child_root) = child_roots.pop_front() {
        root.children
            .extend(eval_iframe_content_children(child_root));
    }
}

fn attach_eval_roots_to_iframes(node: &mut DomEvalNode, child_roots: &mut VecDeque<DomEvalNode>) {
    if child_roots.is_empty() {
        return;
    }
    if node.node_type == DomEvalNodeType::Element
        && matches!(node.tag_name.as_str(), "iframe" | "frame")
        && node.children.is_empty()
        && let Some(child_root) = child_roots.pop_front()
    {
        node.children
            .extend(eval_iframe_content_children(child_root));
        return;
    }

    for child in &mut node.children {
        attach_eval_roots_to_iframes(child, child_roots);
        if child_roots.is_empty() {
            return;
        }
    }
}

fn eval_iframe_content_children(child_root: DomEvalNode) -> Vec<DomEvalNode> {
    if child_root.node_type == DomEvalNodeType::Element && child_root.tag_name == "html" {
        if let Some(body) = child_root
            .children
            .into_iter()
            .find(|child| child.node_type == DomEvalNodeType::Element && child.tag_name == "body")
        {
            return body.children;
        }
        return Vec::new();
    }
    if child_root.node_type == DomEvalNodeType::Element && child_root.tag_name == "body" {
        return child_root.children;
    }
    vec![child_root]
}

pub(crate) fn add_dom_page_stats(total: &mut DomPageStats, next: DomPageStats) {
    total.links = total.links.saturating_add(next.links);
    total.iframes = total.iframes.saturating_add(next.iframes);
    total.shadow_open = total.shadow_open.saturating_add(next.shadow_open);
    total.shadow_closed = total.shadow_closed.saturating_add(next.shadow_closed);
    total.scroll_containers = total
        .scroll_containers
        .saturating_add(next.scroll_containers);
    total.images = total.images.saturating_add(next.images);
    total.interactive_elements = total
        .interactive_elements
        .saturating_add(next.interactive_elements);
    total.total_elements = total.total_elements.saturating_add(next.total_elements);
    total.text_chars = total.text_chars.saturating_add(next.text_chars);
}

pub(crate) fn dom_element_from_value(
    target_id: &str,
    value: &Value,
    accessibility: &BTreeMap<String, AccessibilityNodeInfo>,
) -> Result<DomElementRef, BrowserError> {
    let index = value
        .get("index")
        .and_then(Value::as_u64)
        .and_then(|index| u32::try_from(index).ok())
        .ok_or_else(|| BrowserError::MissingResponseData("element index".to_owned()))?;
    let ax_info = accessibility_info_for_value(value, accessibility);
    let attributes = enriched_attributes_from_value(value, ax_info);
    let ax_role = ax_info.and_then(|info| info.role.as_deref());
    let dom_role = value.get("role").and_then(Value::as_str).map(str::to_owned);
    let role = dom_role.or_else(|| {
        ax_role
            .filter(|role| is_useful_ax_role(role))
            .map(str::to_owned)
    });
    let name = ax_info
        .and_then(|info| nonempty_value(info.name.as_deref()))
        .map(str::to_owned)
        .or_else(|| value.get("name").and_then(Value::as_str).map(str::to_owned));

    Ok(DomElementRef {
        index,
        target_id: target_id.to_owned(),
        backend_node_id: ax_info.map(|info| info.backend_node_id).unwrap_or_default(),
        node_id: ax_info.and_then(|info| info.node_id),
        tag_name: value
            .get("tag_name")
            .and_then(Value::as_str)
            .unwrap_or("element")
            .to_owned(),
        role,
        name,
        text: value.get("text").and_then(Value::as_str).map(str::to_owned),
        attributes,
        bounds: element_bounds_from_value(value),
        is_visible: value
            .get("is_visible")
            .and_then(Value::as_bool)
            .unwrap_or(true),
        is_interactive: value
            .get("is_interactive")
            .and_then(Value::as_bool)
            .unwrap_or(true),
        is_scrollable: value
            .get("is_scrollable")
            .and_then(Value::as_bool)
            .unwrap_or(false),
    })
}

pub(crate) fn snapshot_backend_ids_by_ax_ref(snapshot: &Value) -> BTreeMap<String, u64> {
    let strings = snapshot
        .get("strings")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut backend_by_ref = BTreeMap::new();

    for document in snapshot
        .get("documents")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let Some(nodes) = document.get("nodes") else {
            continue;
        };
        let backend_node_ids = nodes
            .get("backendNodeId")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let attributes = nodes
            .get("attributes")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();

        for (node_index, node_attributes) in attributes.iter().enumerate() {
            let Some(backend_node_id) = backend_node_ids.get(node_index).and_then(Value::as_u64)
            else {
                continue;
            };
            if let Some(ax_ref) =
                snapshot_attribute_value(node_attributes, &strings, AX_REF_ATTRIBUTE)
            {
                backend_by_ref.insert(ax_ref.to_owned(), backend_node_id);
            }
        }
    }

    backend_by_ref
}

fn snapshot_attribute_value<'a>(
    attributes: &'a Value,
    strings: &'a [Value],
    attribute_name: &str,
) -> Option<&'a str> {
    let attributes = attributes.as_array()?;
    for pair in attributes.chunks(2) {
        let [name, value] = pair else {
            continue;
        };
        if snapshot_string(strings, name) == Some(attribute_name) {
            return snapshot_string(strings, value);
        }
    }
    None
}

fn snapshot_string<'a>(strings: &'a [Value], index: &Value) -> Option<&'a str> {
    let index = usize::try_from(index.as_u64()?).ok()?;
    strings.get(index)?.as_str()
}

pub(crate) fn accessibility_nodes_by_backend_id(
    tree: &Value,
) -> BTreeMap<u64, AccessibilityNodeInfo> {
    tree.get("nodes")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|node| node.get("ignored").and_then(Value::as_bool) != Some(true))
        .filter_map(|node| {
            let backend_node_id = node.get("backendDOMNodeId").and_then(Value::as_u64)?;
            let mut properties = ax_node_properties(node);
            for field in ["value", "description"] {
                if let Some(value) = ax_node_field_to_string(node, field) {
                    properties.entry(field.to_owned()).or_insert(value);
                }
            }
            Some((
                backend_node_id,
                AccessibilityNodeInfo {
                    backend_node_id,
                    node_id: None,
                    role: ax_property_value(node, "role").map(str::to_owned),
                    name: ax_property_value(node, "name").map(str::to_owned),
                    properties,
                },
            ))
        })
        .collect()
}

fn accessibility_info_for_value<'a>(
    value: &Value,
    accessibility: &'a BTreeMap<String, AccessibilityNodeInfo>,
) -> Option<&'a AccessibilityNodeInfo> {
    value
        .get("ax_ref")
        .and_then(Value::as_str)
        .and_then(|ax_ref| accessibility.get(ax_ref))
}

pub(crate) fn enriched_attributes_from_value(
    value: &Value,
    ax_info: Option<&AccessibilityNodeInfo>,
) -> BTreeMap<String, String> {
    let mut attributes: BTreeMap<String, String> = value
        .get("attributes")
        .and_then(Value::as_object)
        .map(|attrs| {
            attrs
                .iter()
                .filter_map(|(key, value)| {
                    value.as_str().map(|value| (key.clone(), value.to_owned()))
                })
                .collect()
        })
        .unwrap_or_default();

    if let Some(ax_info) = ax_info {
        attributes.extend(ax_info.properties.clone());
        if let Some(name) = nonempty_value(ax_info.name.as_deref()) {
            attributes.insert("ax_name".to_owned(), name.to_owned());
        }
        if let Some(description) = ax_info
            .properties
            .get("description")
            .and_then(|value| nonempty_value(Some(value)))
        {
            attributes.insert("ax_description".to_owned(), description.to_owned());
        }
    }

    attributes
}

fn ax_node_properties(node: &Value) -> BTreeMap<String, String> {
    node.get("properties")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|property| {
            let name = property.get("name")?.as_str()?.to_owned();
            let value = ax_property_to_string(property)?;
            Some((name, value))
        })
        .collect()
}

fn ax_property_value<'a>(node: &'a Value, property: &str) -> Option<&'a str> {
    nonempty_value(node.get(property)?.get("value")?.as_str())
}

fn ax_property_to_string(property: &Value) -> Option<String> {
    ax_value_to_string(property.get("value")?)
}

fn ax_node_field_to_string(node: &Value, field: &str) -> Option<String> {
    ax_value_to_string(node.get(field)?)
}

fn ax_value_to_string(value: &Value) -> Option<String> {
    match value.get("value")? {
        Value::Bool(value) => Some(value.to_string()),
        Value::Number(value) => Some(value.to_string()),
        Value::String(value) => nonempty_value(Some(value)).map(str::to_owned),
        _ => None,
    }
}

fn nonempty_value(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|value| !value.is_empty())
}

pub(crate) fn is_ax_suppressed_interactive_element(element: &DomElementRef) -> bool {
    ["disabled", "hidden"].into_iter().any(|attribute| {
        element
            .attributes
            .get(attribute)
            .is_some_and(|value| is_truthy_accessibility_value(value))
    })
}

fn is_truthy_accessibility_value(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "true" | "1" | "yes"
    )
}

fn is_useful_ax_role(role: &str) -> bool {
    !matches!(role, "generic" | "none" | "presentation" | "StaticText")
}

pub(crate) fn should_fallback_to_index_traversal(error: &BrowserError) -> bool {
    match error {
        BrowserError::MissingResponseData(message) => message.contains("cached element node id"),
        BrowserError::CommandFailed { method, message } => {
            (method == "DOM.resolveNode"
                && (message.contains("No node")
                    || message.contains("Could not find")
                    || message.contains("Invalid remote object id")))
                || (method == "Runtime.callFunctionOn"
                    && message.contains("cached element is detached from DOM"))
        }
        _ => false,
    }
}

pub(crate) fn is_missing_target_error(error: &BrowserError) -> bool {
    matches!(
        error,
        BrowserError::CommandFailed { method, message }
            if matches!(method.as_str(), "Target.attachToTarget" | "Target.closeTarget")
                && message.contains("No target with given id found")
    )
}

pub(crate) fn parse_dropdown_options_value(value: Value) -> Result<Vec<String>, BrowserError> {
    let encoded = value
        .as_str()
        .ok_or_else(|| BrowserError::MissingResponseData("dropdown options string".to_owned()))?;
    serde_json::from_str(encoded).map_err(|error| BrowserError::Transport(error.to_string()))
}

pub(crate) fn element_bounds_from_value(value: &Value) -> Option<ElementBounds> {
    let bounds = value.get("bounds")?;
    Some(ElementBounds {
        x: bounds
            .get("x")?
            .as_i64()
            .and_then(|x| i32::try_from(x).ok())?,
        y: bounds
            .get("y")?
            .as_i64()
            .and_then(|y| i32::try_from(y).ok())?,
        width: bounds
            .get("width")?
            .as_u64()
            .and_then(|width| u32::try_from(width).ok())?,
        height: bounds
            .get("height")?
            .as_u64()
            .and_then(|height| u32::try_from(height).ok())?,
    })
}

pub(crate) fn page_info_from_value(value: &Value) -> Option<PageInfo> {
    Some(PageInfo {
        viewport_width: u32_field(value, "viewport_width")?,
        viewport_height: u32_field(value, "viewport_height")?,
        page_width: u32_field(value, "page_width")?,
        page_height: u32_field(value, "page_height")?,
        scroll_x: i32_field(value, "scroll_x")?,
        scroll_y: i32_field(value, "scroll_y")?,
        pixels_above: u32_field(value, "pixels_above")?,
        pixels_below: u32_field(value, "pixels_below")?,
        pixels_left: u32_field(value, "pixels_left")?,
        pixels_right: u32_field(value, "pixels_right")?,
    })
}

pub(crate) fn detect_pagination_buttons(dom_state: &SerializedDomState) -> Vec<PaginationButton> {
    let mut buttons = Vec::new();

    for element in dom_state.selector_map.values() {
        if !element.is_interactive {
            continue;
        }

        let label = pagination_label_text(element);
        let label_lower = label.to_lowercase();
        let role = element
            .role
            .as_deref()
            .or_else(|| element.attributes.get("role").map(String::as_str))
            .unwrap_or("")
            .to_ascii_lowercase();

        let button_type = if contains_any(
            &label_lower,
            &["first", "⇤", "primera", "première", "erste", "eerste"],
        ) {
            Some(PaginationButtonType::First)
        } else if contains_any(
            &label_lower,
            &["last", "⇥", "última", "dernier", "letzte", "laatste"],
        ) {
            Some(PaginationButtonType::Last)
        } else if contains_any(
            &label_lower,
            &[
                "next",
                ">",
                "›",
                "→",
                "»",
                "siguiente",
                "suivant",
                "volgende",
            ],
        ) {
            Some(PaginationButtonType::Next)
        } else if contains_any(
            &label_lower,
            &[
                "prev",
                "previous",
                "<",
                "‹",
                "←",
                "«",
                "anterior",
                "précédent",
                "vorige",
            ],
        ) {
            Some(PaginationButtonType::Prev)
        } else if label_lower.trim().len() <= 2
            && label_lower
                .trim()
                .chars()
                .all(|character| character.is_ascii_digit())
            && matches!(role.as_str(), "" | "button" | "link")
        {
            Some(PaginationButtonType::PageNumber)
        } else {
            None
        };

        let Some(button_type) = button_type else {
            continue;
        };

        buttons.push(PaginationButton {
            button_type,
            backend_node_id: if element.backend_node_id == 0 {
                u64::from(element.index)
            } else {
                element.backend_node_id
            },
            text: label.trim().to_owned(),
            selector: pagination_selector(element),
            is_disabled: pagination_is_disabled(element),
        });
    }

    buttons
}

fn pagination_label_text(element: &DomElementRef) -> String {
    let mut parts = vec![render_element_text(element)];
    for attribute in ["aria-label", "title", "class"] {
        if let Some(value) = element.attributes.get(attribute) {
            parts.push(value.clone());
        }
    }
    parts
        .into_iter()
        .filter(|part| !part.trim().is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

fn pagination_is_disabled(element: &DomElementRef) -> bool {
    element
        .attributes
        .get("disabled")
        .is_some_and(|value| value == "true" || value.is_empty())
        || element
            .attributes
            .get("aria-disabled")
            .is_some_and(|value| value == "true")
        || element
            .attributes
            .get("class")
            .is_some_and(|value| value.to_lowercase().contains("disabled"))
}

fn pagination_selector(element: &DomElementRef) -> String {
    if let Some(id) = element.attributes.get("id") {
        format!("#{id}")
    } else if let Some(name) = element.attributes.get("name") {
        format!("{}[name=\"{}\"]", element.tag_name, name)
    } else {
        format!("{}:nth-index({})", element.tag_name, element.index)
    }
}

fn contains_any(haystack: &str, needles: &[&str]) -> bool {
    needles.iter().any(|needle| haystack.contains(needle))
}

pub(crate) fn u32_field(value: &Value, field: &str) -> Option<u32> {
    value
        .get(field)?
        .as_u64()
        .and_then(|number| u32::try_from(number).ok())
}

pub(crate) fn i32_field(value: &Value, field: &str) -> Option<i32> {
    value
        .get(field)?
        .as_i64()
        .and_then(|number| i32::try_from(number).ok())
}
