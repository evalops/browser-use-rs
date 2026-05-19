use crate::AgentOutput;
use browser_use_llm::{ChatRequest, ContentPart, MessageRole};
use browser_use_tools::SearchEngine;
use md5::{Digest, Md5};
use regex::Regex;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::OnceLock;
use url::form_urlencoded;

#[must_use]
pub fn search_url(engine: &SearchEngine, query: &str) -> String {
    let encoded: String = form_urlencoded::byte_serialize(query.as_bytes()).collect();
    match engine {
        SearchEngine::DuckDuckGo => format!("https://duckduckgo.com/?q={encoded}"),
        SearchEngine::Google => format!("https://www.google.com/search?q={encoded}&udm=14"),
        SearchEngine::Bing => format!("https://www.bing.com/search?q={encoded}"),
    }
}

const START_URL_EXCLUDED_EXTENSIONS: &[&str] = &[
    "pdf",
    "doc",
    "docx",
    "xls",
    "xlsx",
    "ppt",
    "pptx",
    "odt",
    "ods",
    "odp",
    "txt",
    "md",
    "csv",
    "json",
    "xml",
    "yaml",
    "yml",
    "zip",
    "rar",
    "7z",
    "tar",
    "gz",
    "bz2",
    "xz",
    "jpg",
    "jpeg",
    "png",
    "gif",
    "bmp",
    "svg",
    "webp",
    "ico",
    "mp3",
    "mp4",
    "avi",
    "mkv",
    "mov",
    "wav",
    "flac",
    "ogg",
    "py",
    "js",
    "css",
    "java",
    "cpp",
    "bib",
    "bibtex",
    "tex",
    "latex",
    "cls",
    "sty",
    "exe",
    "msi",
    "dmg",
    "pkg",
    "deb",
    "rpm",
    "iso",
    "polynomial",
];

const START_URL_EXCLUDED_WORDS: &[&str] = &["never", "dont", "not", "don't"];

pub(crate) fn request_with_shortened_urls(
    mut request: ChatRequest,
    limit: Option<usize>,
) -> (ChatRequest, BTreeMap<String, String>) {
    let Some(limit) = limit.filter(|limit| *limit > 0) else {
        return (request, BTreeMap::new());
    };

    let mut replacements = BTreeMap::new();
    for message in &mut request.messages {
        if !matches!(&message.role, MessageRole::User | MessageRole::Assistant) {
            continue;
        }

        for part in &mut message.content {
            let ContentPart::Text { text } = part else {
                continue;
            };
            let (rewritten, part_replacements) = shorten_urls_in_text(text, limit);
            *text = rewritten;
            replacements.extend(part_replacements);
        }
    }

    (request, replacements)
}

pub(crate) fn shorten_urls_in_text(text: &str, limit: usize) -> (String, BTreeMap<String, String>) {
    static URL_PATTERN: OnceLock<Regex> = OnceLock::new();

    if limit == 0 || text.is_empty() {
        return (text.to_owned(), BTreeMap::new());
    }

    let pattern = URL_PATTERN.get_or_init(|| {
        Regex::new(
            r#"(?i)https?://[^\s<>"']+|www\.[^\s<>"']+|[^\s<>"']+\.[a-z]{2,}(?:/[^\s<>"']*)?"#,
        )
        .expect("valid browser-use URL regex")
    });
    let mut replacements = BTreeMap::new();
    let rewritten = pattern
        .replace_all(text, |captures: &regex::Captures<'_>| {
            let original_url = captures.get(0).expect("URL regex capture").as_str();
            shorten_url(original_url, limit, &mut replacements)
                .unwrap_or_else(|| original_url.to_owned())
        })
        .into_owned();

    (rewritten, replacements)
}

fn shorten_url(
    original_url: &str,
    limit: usize,
    replacements: &mut BTreeMap<String, String>,
) -> Option<String> {
    let query_start = original_url.find('?');
    let fragment_start = original_url.find('#');
    let after_path_start = [query_start, fragment_start]
        .into_iter()
        .flatten()
        .min()
        .unwrap_or(original_url.len());
    let base_url = &original_url[..after_path_start];
    let after_path = &original_url[after_path_start..];

    if after_path.chars().count() <= limit {
        return None;
    }

    let truncated_after_path = after_path.chars().take(limit).collect::<String>();
    let mut hasher = Md5::new();
    hasher.update(after_path.as_bytes());
    let digest = hasher.finalize();
    let short_hash = format!("{digest:x}");
    let shortened = format!("{base_url}{truncated_after_path}...{}", &short_hash[..7]);

    if shortened.len() < original_url.len() {
        replacements.insert(shortened.clone(), original_url.to_owned());
        Some(shortened)
    } else {
        None
    }
}

pub(crate) fn restore_shortened_urls_in_agent_output(
    output: AgentOutput,
    replacements: &BTreeMap<String, String>,
) -> Result<AgentOutput, serde_json::Error> {
    if replacements.is_empty() {
        return Ok(output);
    }

    let mut value = serde_json::to_value(output)?;
    replace_shortened_urls_in_value(&mut value, replacements);
    serde_json::from_value(value)
}

fn replace_shortened_urls_in_value(value: &mut Value, replacements: &BTreeMap<String, String>) {
    match value {
        Value::String(text) => {
            for (shortened, original) in replacements {
                if text.contains(shortened) {
                    *text = text.replace(shortened, original);
                }
            }
        }
        Value::Array(items) => {
            for item in items {
                replace_shortened_urls_in_value(item, replacements);
            }
        }
        Value::Object(object) => {
            for item in object.values_mut() {
                replace_shortened_urls_in_value(item, replacements);
            }
        }
        Value::Null | Value::Bool(_) | Value::Number(_) => {}
    }
}

pub(crate) fn extract_start_url_from_task(task: &str) -> Option<String> {
    static EMAIL_RE: OnceLock<Regex> = OnceLock::new();
    static FULL_URL_RE: OnceLock<Regex> = OnceLock::new();
    static DOMAIN_URL_RE: OnceLock<Regex> = OnceLock::new();

    let email_re = EMAIL_RE.get_or_init(|| {
        Regex::new(r"\b[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Z|a-z]{2,}\b")
            .expect("valid email regex")
    });
    let full_url_re = FULL_URL_RE
        .get_or_init(|| Regex::new(r#"https?://[^\s<>"']+"#).expect("valid full URL regex"));
    let domain_url_re = DOMAIN_URL_RE.get_or_init(|| {
        Regex::new(r#"(?:www\.)?[a-zA-Z0-9-]+(?:\.[a-zA-Z0-9-]+)*\.[a-zA-Z]{2,}(?:/[^\s<>"']*)?"#)
            .expect("valid domain URL regex")
    });

    let task_without_emails = email_re.replace_all(task, "");
    let mut found_urls = BTreeSet::new();

    for pattern in [full_url_re, domain_url_re] {
        for matched in pattern.find_iter(&task_without_emails) {
            let original_position = matched.start();
            let url = trim_start_url_trailing_punctuation(matched.as_str());
            if url.is_empty() {
                continue;
            }

            let url_lower = url.to_ascii_lowercase();
            if contains_excluded_file_extension(&url_lower) {
                continue;
            }

            let context_start =
                char_boundary_n_chars_before(&task_without_emails, original_position, 20);
            let context = task_without_emails[context_start..original_position].to_lowercase();
            if START_URL_EXCLUDED_WORDS
                .iter()
                .any(|word| context.contains(word))
            {
                continue;
            }

            let url = if url.starts_with("http://") || url.starts_with("https://") {
                url.to_owned()
            } else {
                format!("https://{url}")
            };
            found_urls.insert(url);
        }
    }

    if found_urls.len() == 1 {
        found_urls.into_iter().next()
    } else {
        None
    }
}

fn trim_start_url_trailing_punctuation(url: &str) -> &str {
    url.trim_end_matches(['.', ',', ';', ':', '!', '?', '(', ')', '[', ']'])
}

fn contains_excluded_file_extension(url_lower: &str) -> bool {
    START_URL_EXCLUDED_EXTENSIONS
        .iter()
        .any(|ext| url_lower.contains(&format!(".{ext}")))
}

fn char_boundary_n_chars_before(text: &str, end: usize, chars: usize) -> usize {
    text[..end]
        .char_indices()
        .rev()
        .nth(chars.saturating_sub(1))
        .map(|(idx, _)| idx)
        .unwrap_or(0)
}
