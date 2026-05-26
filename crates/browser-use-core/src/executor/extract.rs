use crate::{ActionResult, ManagedFileSystem};
use browser_use_cdp::FoundElement;
use browser_use_llm::{ChatMessage, ChatRequest, MessageRole};
use browser_use_tools::ExtractAction;
use serde_json::Value;

const MAX_EXTRACT_CHAR_LIMIT: usize = 100_000;
pub(super) const MAX_EXTRACT_RELATED_ELEMENTS: usize = 200;
const MAX_EXTRACT_MEMORY_LENGTH: usize = 10_000;
const IMAGE_QUERY_KEYWORDS: &[&str] = &[
    "image",
    "photo",
    "picture",
    "thumbnail",
    "img url",
    "image url",
    "photo url",
    "product image",
];

pub(super) fn should_extract_images(query: &str, requested: bool) -> bool {
    let query = query.to_ascii_lowercase();
    requested
        || IMAGE_QUERY_KEYWORDS
            .iter()
            .any(|keyword| query.contains(keyword))
}

pub(crate) fn extract_action_result(
    params: &ExtractAction,
    page_text: &str,
    source_url: Option<&str>,
    extract_images: bool,
    links: Option<&[FoundElement]>,
    images: Option<&[FoundElement]>,
    file_system: Option<&mut ManagedFileSystem>,
) -> ActionResult {
    let total_chars = page_text.chars().count();
    if params.start_from_char > total_chars {
        return ActionResult::error(format!(
            "start_from_char ({}) exceeds content length {total_chars} characters.",
            params.start_from_char
        ));
    }

    let available_chars = total_chars.saturating_sub(params.start_from_char);
    let truncated = available_chars > MAX_EXTRACT_CHAR_LIMIT;
    let content: String = page_text
        .chars()
        .skip(params.start_from_char)
        .take(MAX_EXTRACT_CHAR_LIMIT)
        .collect();
    let next_start_char = params.start_from_char + content.chars().count();
    let content_stats = extract_content_stats(
        total_chars,
        params.start_from_char,
        content.chars().count(),
        truncated,
        next_start_char,
        params.extract_links,
        extract_images,
    );
    let rendered =
        render_extract_envelope(params, source_url, &content, &content_stats, links, images);
    let memory = if rendered.chars().count() < 10_000 {
        rendered.clone()
    } else if let Some(file_name) =
        file_system.and_then(|file_system| file_system.save_extracted_content(&rendered).ok())
    {
        format!(
            "Query: {}\nContent in {file_name} and once in <read_state>.",
            params.query
        )
    } else {
        format!(
            "Query: {}\nContent prepared for extraction, length: {} characters.",
            params.query,
            content.chars().count()
        )
    };

    ActionResult {
        extracted_content: Some(rendered),
        error: None,
        judgement: None,
        long_term_memory: Some(memory),
        include_extracted_content_only_once: true,
        include_in_memory: true,
        is_done: false,
        success: None,
        attachments: Vec::new(),
        images: Vec::new(),
        metadata: extract_metadata(
            params,
            source_url,
            total_chars,
            params.start_from_char,
            content.chars().count(),
            truncated,
            next_start_char,
            extract_images,
            links.map_or(0, <[FoundElement]>::len),
            images.map_or(0, <[FoundElement]>::len),
        ),
    }
}

fn extract_content_stats(
    total_chars: usize,
    start_from_char: usize,
    content_chars: usize,
    truncated: bool,
    next_start_char: usize,
    extract_links: bool,
    extract_images: bool,
) -> String {
    let mut stats =
        format!("Content processed: {total_chars} text chars -> {total_chars} filtered text chars");
    if start_from_char > 0 {
        stats.push_str(&format!(" (started from char {start_from_char})"));
    }
    if truncated {
        stats.push_str(&format!(
            " -> {content_chars} final chars (use start_from_char={next_start_char} to continue)"
        ));
    }
    if extract_links || extract_images {
        stats.push_str(&format!(
            "\nExtraction options: extract_links={extract_links}, extract_images={extract_images}"
        ));
    }
    stats
}

#[allow(clippy::too_many_arguments)]
fn extract_metadata(
    params: &ExtractAction,
    source_url: Option<&str>,
    original_chars: usize,
    start_from_char: usize,
    content_chars: usize,
    truncated: bool,
    next_start_char: usize,
    extract_images: bool,
    links_count: usize,
    images_count: usize,
) -> Option<Value> {
    let schema = params.output_schema.as_ref()?;
    Some(serde_json::json!({
        "structured_extraction": true,
        "schema_used": schema,
        "is_partial": truncated,
        "source_url": source_url,
        "content_stats": {
            "method": "page_text",
            "original_text_chars": original_chars,
            "final_filtered_chars": original_chars,
            "started_from_char": start_from_char,
            "returned_chars": content_chars,
            "next_start_char": if truncated { Some(next_start_char) } else { None::<usize> },
        },
        "options": {
            "extract_links": params.extract_links,
            "extract_images": extract_images,
            "links_count": links_count,
            "images_count": images_count,
            "already_collected_count": params.already_collected.len(),
        }
    }))
}

fn render_extract_envelope(
    params: &ExtractAction,
    source_url: Option<&str>,
    content: &str,
    content_stats: &str,
    links: Option<&[FoundElement]>,
    images: Option<&[FoundElement]>,
) -> String {
    let mut rendered = source_url
        .map(|url| format!("<url>\n{url}\n</url>\n"))
        .unwrap_or_default();
    rendered.push_str(&format!("<query>\n{}\n</query>\n\n", params.query));

    if let Some(schema) = &params.output_schema {
        let schema = serde_json::to_string_pretty(schema).unwrap_or_else(|_| schema.to_string());
        rendered.push_str(&format!("<output_schema>\n{schema}\n</output_schema>\n\n"));
    }

    rendered.push_str(&format!(
        "<content_stats>\n{content_stats}\n</content_stats>\n\n<webpage_content>\n{content}\n</webpage_content>"
    ));

    if let Some(links) = links.and_then(render_link_appendix) {
        rendered.push_str(&format!("\n\n<links>\n{links}\n</links>"));
    }

    if let Some(images) = images.and_then(render_image_appendix) {
        rendered.push_str(&format!("\n\n<images>\n{images}\n</images>"));
    }

    if !params.already_collected.is_empty() {
        let items = params
            .already_collected
            .iter()
            .take(100)
            .map(|item| format!("- {item}"))
            .collect::<Vec<_>>()
            .join("\n");
        rendered.push_str(&format!(
            "\n\n<already_collected>\nSkip items whose name/title/URL matches any of these already-collected identifiers:\n{items}\n</already_collected>"
        ));
    }

    rendered
}

pub(crate) fn build_extract_llm_request(params: &ExtractAction, raw_envelope: &str) -> ChatRequest {
    let structured_schema = params.output_schema.clone();
    let system_prompt = if structured_schema.is_some() {
        "You are an expert at extracting structured data from webpage markdown. Extract only information present in the webpage. Return data that conforms exactly to the provided JSON Schema."
    } else {
        "You are an expert at extracting data from webpage markdown. Extract only information relevant to the query. Do not guess or use outside knowledge."
    };
    let user_prompt = format!(
        "Use the prepared extraction envelope below. It contains the query, content statistics, webpage markdown, and any link/image/already-collected context.\n\n{raw_envelope}"
    );

    ChatRequest {
        messages: vec![
            ChatMessage::text(MessageRole::System, system_prompt),
            ChatMessage::text(MessageRole::User, user_prompt),
        ],
        output_schema: Some(structured_schema.unwrap_or_else(schema_for_extract_text_result)),
    }
}

fn schema_for_extract_text_result() -> Value {
    serde_json::json!({
        "type": "object",
        "properties": {
            "result": { "type": "string" }
        },
        "required": ["result"],
        "additionalProperties": false
    })
}

pub(crate) fn complete_llm_extract_result(
    params: &ExtractAction,
    raw_envelope: &str,
    raw_metadata: Option<&Value>,
    completion: Value,
    file_system: &mut ManagedFileSystem,
) -> ActionResult {
    let source_url = tagged_section(raw_envelope, "url").unwrap_or("about:blank");
    let metadata = params
        .output_schema
        .as_ref()
        .map(|schema| structured_extract_metadata(schema, raw_metadata, source_url, &completion));
    let extracted_content = if params.output_schema.is_some() {
        format!(
            "<url>\n{source_url}\n</url>\n<query>\n{}\n</query>\n<structured_result>\n{}\n</structured_result>",
            params.query,
            serde_json::to_string(&completion).unwrap_or_else(|_| completion.to_string())
        )
    } else {
        format!(
            "<url>\n{source_url}\n</url>\n<query>\n{}\n</query>\n<result>\n{}\n</result>",
            params.query,
            extract_text_completion(completion)
        )
    };
    let (long_term_memory, include_extracted_content_only_once) =
        extract_memory_fields(&params.query, &extracted_content, file_system);

    ActionResult {
        extracted_content: Some(extracted_content),
        error: None,
        judgement: None,
        long_term_memory: Some(long_term_memory),
        include_extracted_content_only_once,
        include_in_memory: true,
        is_done: false,
        success: None,
        attachments: Vec::new(),
        images: Vec::new(),
        metadata,
    }
}

fn extract_text_completion(completion: Value) -> String {
    completion
        .get("result")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
        .or_else(|| completion.as_str().map(ToOwned::to_owned))
        .unwrap_or_else(|| completion.to_string())
}

fn extract_memory_fields(
    query: &str,
    extracted_content: &str,
    file_system: &mut ManagedFileSystem,
) -> (String, bool) {
    if extracted_content.chars().count() < MAX_EXTRACT_MEMORY_LENGTH {
        return (extracted_content.to_owned(), false);
    }

    if let Ok(file_name) = file_system.save_extracted_content(extracted_content) {
        (
            format!("Query: {query}\nContent in {file_name} and once in <read_state>."),
            true,
        )
    } else {
        (
            format!(
                "Query: {query}\nExtracted result length: {} characters.",
                extracted_content.chars().count()
            ),
            true,
        )
    }
}

fn structured_extract_metadata(
    schema: &Value,
    raw_metadata: Option<&Value>,
    source_url: &str,
    data: &Value,
) -> Value {
    serde_json::json!({
        "structured_extraction": true,
        "extraction_result": {
            "data": data,
            "schema_used": schema,
            "is_partial": raw_metadata
                .and_then(|metadata| metadata.get("is_partial"))
                .cloned()
                .unwrap_or(Value::Bool(false)),
            "source_url": source_url,
            "content_stats": raw_metadata
                .and_then(|metadata| metadata.get("content_stats"))
                .cloned()
                .unwrap_or(Value::Null),
        }
    })
}

fn tagged_section<'a>(text: &'a str, tag: &str) -> Option<&'a str> {
    let start_tag = format!("<{tag}>\n");
    let end_tag = format!("\n</{tag}>");
    let start = text.find(&start_tag)? + start_tag.len();
    let end = text[start..].find(&end_tag)? + start;
    Some(&text[start..end])
}

fn render_link_appendix(elements: &[FoundElement]) -> Option<String> {
    let lines = elements
        .iter()
        .filter_map(|element| {
            let href = element.attributes.get("href")?.trim();
            if href.is_empty() {
                return None;
            }
            let label = element_label(element)
                .filter(|label| !label.is_empty())
                .unwrap_or(href);
            Some(format!("- {label}: {href}"))
        })
        .collect::<Vec<_>>();

    (!lines.is_empty()).then(|| lines.join("\n"))
}

fn render_image_appendix(elements: &[FoundElement]) -> Option<String> {
    let lines = elements
        .iter()
        .filter_map(|element| {
            let src = element
                .attributes
                .get("src")
                .or_else(|| element.attributes.get("data-src"))
                .or_else(|| element.attributes.get("srcset"))?
                .trim();
            if src.is_empty() {
                return None;
            }
            let label = element_label(element)
                .filter(|label| !label.is_empty())
                .unwrap_or("image");
            Some(format!("- {label}: {src}"))
        })
        .collect::<Vec<_>>();

    (!lines.is_empty()).then(|| lines.join("\n"))
}

fn element_label(element: &FoundElement) -> Option<&str> {
    element
        .text
        .as_deref()
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .or_else(|| attr_label(element, "alt"))
        .or_else(|| attr_label(element, "title"))
        .or_else(|| attr_label(element, "aria-label"))
}

fn attr_label<'a>(element: &'a FoundElement, name: &str) -> Option<&'a str> {
    element
        .attributes
        .get(name)
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
}

pub(super) fn extract_link_attributes() -> Vec<String> {
    ["href", "title", "aria-label", "rel"]
        .into_iter()
        .map(str::to_owned)
        .collect()
}

pub(super) fn extract_image_attributes() -> Vec<String> {
    ["src", "data-src", "srcset", "alt", "title", "aria-label"]
        .into_iter()
        .map(str::to_owned)
        .collect()
}
