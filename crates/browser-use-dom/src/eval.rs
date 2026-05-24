use crate::{DomEvalNode, DomEvalNodeType};

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

pub(crate) fn serialize_eval_tree(node: &DomEvalNode, depth: usize) -> String {
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
