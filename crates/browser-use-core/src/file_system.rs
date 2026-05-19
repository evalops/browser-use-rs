//! Managed file sandbox and file-action helpers.
//!
//! Browser-use lets the model create, read, replace, upload, and attach files.
//! This module keeps relative file actions inside a managed sandbox while still
//! allowing explicit absolute paths for trusted caller-provided files.
//!
//! ```mermaid
//! flowchart LR
//!     Action["write/read/replace/upload action"] --> Resolve["resolve_file_action_path"]
//!     Resolve --> Sandbox["managed sandbox path"]
//!     Resolve --> Absolute["trusted absolute path"]
//!     Sandbox --> Result["ActionResult + attachments"]
//!     Absolute --> Result
//!     Result --> Prompt["future prompt/read state"]
//!     Result --> Done["done files_to_display"]
//! ```

use crate::ActionResult;
use base64::Engine;
use browser_use_cdp::BrowserError;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::io::{Read, Write};
use uuid::Uuid;

pub(crate) fn display_done_file(file_name: &str) -> Option<(String, String)> {
    if validate_text_file_name(file_name).is_some() {
        return None;
    }

    let content = std::fs::read_to_string(file_name).ok()?;
    let attachment = std::fs::canonicalize(file_name)
        .unwrap_or_else(|_| std::path::PathBuf::from(file_name))
        .display()
        .to_string();
    Some((format!("{file_name}:\n{content}"), attachment))
}

pub(crate) fn write_file_action(
    params: &browser_use_tools::WriteFileAction,
) -> Result<ActionResult, BrowserError> {
    let resolved_file = resolve_file_action_path(&params.file_name, supported_write_extensions());
    if let Some(result) = validate_write_file_name(&resolved_file.display_name) {
        return Ok(result);
    }
    let path = resolved_file.path.as_path();
    if params.append && !path.exists() {
        return Ok(ActionResult::error(format!(
            "File '{}' not found.",
            resolved_file.display_name
        )));
    }
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent)
            .map_err(|error| BrowserError::ActionFailed(error.to_string()))?;
    }

    let mut content = params.content.clone();
    if params.trailing_newline {
        content.push('\n');
    }
    if params.leading_newline {
        content.insert(0, '\n');
    }

    if is_csv_file(&resolved_file.display_name) {
        // CSV is normalized before writing so appends and replacements behave
        // consistently whether the model provided raw rows or pretty text.
        content = normalize_csv_content(&content);
    }

    if params.append {
        // PDF and DOCX are binary containers, so appending means read existing
        // text, merge in memory, and write a fresh container rather than using
        // byte-level append.
        if is_pdf_file(&resolved_file.display_name) {
            let existing = pdf_extract::extract_text(path)
                .map_err(|error| BrowserError::ActionFailed(error.to_string()))?;
            let merged = merge_pdf_append_content(&existing, &content);
            write_pdf_text(path, &merged)
                .map_err(|error| BrowserError::ActionFailed(error.to_string()))?;
        } else if is_docx_file(&resolved_file.display_name) {
            let existing = read_docx_text(&resolved_file.path_string())
                .map_err(|error| BrowserError::ActionFailed(error.to_string()))?;
            let merged = merge_docx_append_content(&existing, &content);
            write_docx_text(path, &merged)
                .map_err(|error| BrowserError::ActionFailed(error.to_string()))?;
        } else if is_csv_file(&resolved_file.display_name) {
            let existing = std::fs::read_to_string(path)
                .map_err(|error| BrowserError::ActionFailed(error.to_string()))?;
            let merged = merge_csv_append_content(&existing, &content);
            std::fs::write(path, merged)
                .map_err(|error| BrowserError::ActionFailed(error.to_string()))?;
        } else {
            let mut file = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)
                .map_err(|error| BrowserError::ActionFailed(error.to_string()))?;
            file.write_all(content.as_bytes())
                .map_err(|error| BrowserError::ActionFailed(error.to_string()))?;
        }
        Ok(ActionResult::extracted(format!(
            "Appended to file {}{}",
            resolved_file.display_name,
            resolved_file.correction_suffix()
        )))
    } else {
        if is_pdf_file(&resolved_file.display_name) {
            write_pdf_text(path, &content)
                .map_err(|error| BrowserError::ActionFailed(error.to_string()))?;
        } else if is_docx_file(&resolved_file.display_name) {
            write_docx_text(path, &content)
                .map_err(|error| BrowserError::ActionFailed(error.to_string()))?;
        } else {
            std::fs::write(path, content)
                .map_err(|error| BrowserError::ActionFailed(error.to_string()))?;
        }
        Ok(ActionResult::extracted(format!(
            "Wrote file {}{}",
            resolved_file.display_name,
            resolved_file.correction_suffix()
        )))
    }
}

fn is_csv_file(file_name: &str) -> bool {
    file_extension(file_name).as_deref() == Some("csv")
}

fn is_pdf_file(file_name: &str) -> bool {
    file_extension(file_name).as_deref() == Some("pdf")
}

fn is_docx_file(file_name: &str) -> bool {
    file_extension(file_name).as_deref() == Some("docx")
}

fn merge_pdf_append_content(existing: &str, new_content: &str) -> String {
    let existing = existing.trim_end_matches(['\n', '\r', '\u{c}']);
    merge_document_append_content(existing, new_content)
}

fn merge_docx_append_content(existing: &str, new_content: &str) -> String {
    merge_document_append_content(existing, new_content)
}

fn merge_document_append_content(existing: &str, new_content: &str) -> String {
    if new_content
        .trim_matches(|char| char == '\n' || char == '\r')
        .is_empty()
    {
        return existing.to_owned();
    }

    let mut merged = existing.to_owned();
    if !merged.is_empty() && !new_content.starts_with('\n') {
        merged.push('\n');
    }
    merged.push_str(new_content);
    merged
}

fn merge_csv_append_content(existing: &str, new_content: &str) -> String {
    if new_content
        .trim_matches(|char| char == '\n' || char == '\r')
        .is_empty()
    {
        return existing.to_owned();
    }

    let mut merged = existing.to_owned();
    if !merged.is_empty() && !merged.ends_with('\n') {
        merged.push('\n');
    }
    merged.push_str(new_content);
    normalize_csv_content(&merged)
}

fn normalize_csv_content(raw: &str) -> String {
    let mut content = raw
        .trim_matches(|char| char == '\n' || char == '\r')
        .to_owned();
    if content.is_empty() {
        return raw.to_owned();
    }

    if !content.contains('\n') && content.contains("\\n") {
        content = content.replace("\\\"", "\"").replace("\\n", "\n");
    }

    let mut reader = csv::ReaderBuilder::new()
        .has_headers(false)
        .flexible(true)
        .from_reader(content.as_bytes());
    let mut rows = Vec::new();
    for record in reader.records() {
        let Ok(record) = record else {
            return raw.to_owned();
        };
        if !record.is_empty() {
            rows.push(record);
        }
    }

    if rows.is_empty() {
        return raw.to_owned();
    }

    let mut output = Vec::new();
    {
        let mut writer = csv::WriterBuilder::new()
            .has_headers(false)
            .terminator(csv::Terminator::Any(b'\n'))
            .from_writer(&mut output);
        for row in rows {
            if writer.write_record(&row).is_err() {
                return raw.to_owned();
            }
        }
        if writer.flush().is_err() {
            return raw.to_owned();
        }
    }

    let Ok(mut normalized) = String::from_utf8(output) else {
        return raw.to_owned();
    };
    while normalized.ends_with('\n') {
        normalized.pop();
    }
    normalized
}

pub(crate) fn read_file_action(file_name: &str) -> Result<ActionResult, BrowserError> {
    let read_extensions = supported_read_extensions();
    let resolved_file = resolve_file_action_path(file_name, &read_extensions);
    if let Some(result) = validate_read_file_name(&resolved_file.display_name) {
        return Ok(result);
    }
    let path_string = resolved_file.path_string();
    if is_supported_read_image_file(&resolved_file.display_name) {
        let mut result = read_image_file_action(&path_string)?;
        apply_file_name_correction_note(&mut result, &resolved_file);
        return Ok(result);
    }
    if is_supported_read_document_file(&resolved_file.display_name) {
        let mut result = read_document_file_action(&path_string)?;
        apply_file_name_correction_note(&mut result, &resolved_file);
        return Ok(result);
    }
    let content = std::fs::read_to_string(&resolved_file.path)
        .map_err(|error| BrowserError::ActionFailed(error.to_string()))?;
    let memory = read_file_memory(&content);
    Ok(ActionResult {
        extracted_content: Some(format!(
            "{}Read file {}:\n{content}",
            resolved_file.correction_prefix(),
            resolved_file.display_name
        )),
        error: None,
        judgement: None,
        long_term_memory: Some(memory),
        include_extracted_content_only_once: true,
        include_in_memory: true,
        is_done: false,
        success: None,
        attachments: Vec::new(),
        images: Vec::new(),
        metadata: None,
    })
}

fn read_document_file_action(file_name: &str) -> Result<ActionResult, BrowserError> {
    if is_pdf_file(file_name) {
        return read_pdf_file_action(file_name);
    }

    let content = match read_document_file_content(file_name) {
        Ok(content) => content,
        Err(error) => {
            return Ok(ActionResult::error(format!(
                "Error: Could not read file '{file_name}'. {error}"
            )));
        }
    };
    let content = truncate_read_document_content(&content);
    let memory = read_file_memory(&content);
    Ok(ActionResult {
        extracted_content: Some(format!(
            "Read from file {file_name}.\n<content>\n{content}\n</content>"
        )),
        error: None,
        judgement: None,
        long_term_memory: Some(memory),
        include_extracted_content_only_once: true,
        include_in_memory: true,
        is_done: false,
        success: None,
        attachments: Vec::new(),
        images: Vec::new(),
        metadata: None,
    })
}

fn read_document_file_content(file_name: &str) -> Result<String, String> {
    match file_extension(file_name).as_deref() {
        Some("docx") => read_docx_text(file_name),
        _ => Err("unsupported document extension".to_owned()),
    }
}

fn read_pdf_file_action(file_name: &str) -> Result<ActionResult, BrowserError> {
    let pages = match pdf_extract::extract_text_by_pages(file_name) {
        Ok(pages) => pdf_pages_or_empty_page(pages),
        Err(error) => {
            return Ok(ActionResult::error(format!(
                "Error: Could not read file '{file_name}'. {error}"
            )));
        }
    };
    let envelope = render_pdf_read_envelope(file_name, &pages);
    let memory = read_file_memory(&pages.join("\n"));

    Ok(ActionResult {
        extracted_content: Some(envelope),
        error: None,
        judgement: None,
        long_term_memory: Some(memory),
        include_extracted_content_only_once: true,
        include_in_memory: true,
        is_done: false,
        success: None,
        attachments: Vec::new(),
        images: Vec::new(),
        metadata: None,
    })
}

fn pdf_pages_or_empty_page(pages: Vec<String>) -> Vec<String> {
    if pages.is_empty() {
        vec![String::new()]
    } else {
        pages
    }
}

pub(crate) const PDF_READ_MAX_CHARS: usize = 60_000;

pub(crate) fn render_pdf_read_envelope(file_name: &str, pages: &[String]) -> String {
    let total_pages = pages.len();
    let total_chars: usize = pages.iter().map(|page| page.chars().count()).sum();
    if total_chars <= PDF_READ_MAX_CHARS {
        let content = render_pdf_page_markers(
            pages
                .iter()
                .enumerate()
                .filter(|(_, text)| !text.trim().is_empty())
                .map(|(index, text)| (index + 1, text.as_str())),
        );
        return format!(
            "Read from file {file_name} ({total_pages} pages, {} chars).\n<content>\n{content}\n</content>",
            format_usize_with_commas(total_chars)
        );
    }

    let mut content_parts = Vec::new();
    let mut pages_included = BTreeSet::new();
    let mut chars_used = 0usize;
    for page_number in pdf_priority_pages(pages) {
        let text = &pages[page_number - 1];
        if text.trim().is_empty() {
            continue;
        }

        let header = format!("--- Page {page_number} ---\n");
        let truncation_suffix = "\n[...truncated]";
        let remaining = PDF_READ_MAX_CHARS.saturating_sub(chars_used);
        let min_useful = header.chars().count() + truncation_suffix.chars().count() + 50;
        if remaining < min_useful {
            break;
        }

        let mut page_content = format!("{header}{text}");
        if page_content.chars().count() > remaining {
            let kept_chars = remaining.saturating_sub(truncation_suffix.chars().count());
            page_content = format!(
                "{}{truncation_suffix}",
                page_content.chars().take(kept_chars).collect::<String>()
            );
        }
        chars_used += page_content.chars().count();
        pages_included.insert(page_number);
        content_parts.push((page_number, page_content));
        if chars_used >= PDF_READ_MAX_CHARS {
            break;
        }
    }

    content_parts.sort_by_key(|(page_number, _)| *page_number);
    let mut content = content_parts
        .into_iter()
        .map(|(_, content)| content)
        .collect::<Vec<_>>()
        .join("\n\n");
    if pages_included.len() < total_pages {
        let skipped = (1..=total_pages)
            .filter(|page_number| !pages_included.contains(page_number))
            .collect::<Vec<_>>();
        let skipped_preview = skipped
            .iter()
            .take(10)
            .map(usize::to_string)
            .collect::<Vec<_>>()
            .join(", ");
        let ellipsis = if skipped.len() > 10 { "..." } else { "" };
        content.push_str(&format!(
            "\n\n[Showing {} of {total_pages} pages. Skipped pages: [{skipped_preview}]{ellipsis}. Use extract with start_from_char to read further into the file.]",
            pages_included.len()
        ));
    }

    format!(
        "Read from file {file_name} ({total_pages} pages, {} chars total).\n<content>\n{content}\n</content>",
        format_usize_with_commas(total_chars)
    )
}

fn render_pdf_page_markers<'a>(pages: impl Iterator<Item = (usize, &'a str)>) -> String {
    pages
        .map(|(page_number, text)| format!("--- Page {page_number} ---\n{text}"))
        .collect::<Vec<_>>()
        .join("\n\n")
}

fn pdf_priority_pages(pages: &[String]) -> Vec<usize> {
    let word_pattern = regex::Regex::new(r"\b[a-zA-Z]{4,}\b").expect("valid word regex");
    let total_pages = pages.len();
    let mut page_words = BTreeMap::<usize, BTreeSet<String>>::new();
    let mut word_to_pages = BTreeMap::<String, BTreeSet<usize>>::new();

    for (index, text) in pages.iter().enumerate() {
        let page_number = index + 1;
        let words = word_pattern
            .find_iter(text)
            .map(|word| word.as_str().to_ascii_lowercase())
            .collect::<BTreeSet<_>>();
        for word in &words {
            word_to_pages
                .entry(word.clone())
                .or_default()
                .insert(page_number);
        }
        page_words.insert(page_number, words);
    }

    let mut scored_pages = page_words
        .iter()
        .map(|(page_number, words)| {
            let score = words
                .iter()
                .filter_map(|word| word_to_pages.get(word))
                .map(|pages_with_word| (total_pages as f64 / pages_with_word.len() as f64).ln())
                .sum::<f64>();
            (*page_number, score)
        })
        .collect::<Vec<_>>();
    scored_pages.sort_by(|(left_page, left_score), (right_page, right_score)| {
        right_score
            .partial_cmp(left_score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| left_page.cmp(right_page))
    });

    let mut priority_pages = Vec::new();
    if total_pages > 0 {
        priority_pages.push(1);
    }
    for (page_number, _) in scored_pages {
        if !priority_pages.contains(&page_number) {
            priority_pages.push(page_number);
        }
    }
    for page_number in 1..=total_pages {
        if !priority_pages.contains(&page_number) {
            priority_pages.push(page_number);
        }
    }
    priority_pages
}

fn format_usize_with_commas(value: usize) -> String {
    let text = value.to_string();
    let mut formatted = String::with_capacity(text.len() + text.len() / 3);
    for (index, character) in text.chars().rev().enumerate() {
        if index > 0 && index % 3 == 0 {
            formatted.push(',');
        }
        formatted.push(character);
    }
    formatted.chars().rev().collect()
}

fn write_pdf_text(path: &std::path::Path, content: &str) -> Result<(), String> {
    std::fs::write(path, pdf_document_bytes(content)).map_err(|error| error.to_string())
}

pub(crate) fn pdf_document_bytes(content: &str) -> Vec<u8> {
    let streams = pdf_page_streams(content);
    let page_count = streams.len();
    let font_object_id = 3usize;
    let first_page_object_id = 4usize;
    let first_content_object_id = first_page_object_id + page_count;
    let kids = (0..page_count)
        .map(|index| format!("{} 0 R", first_page_object_id + index))
        .collect::<Vec<_>>()
        .join(" ");

    let mut objects = vec![
        "<< /Type /Catalog /Pages 2 0 R >>".to_owned(),
        format!("<< /Type /Pages /Kids [{kids}] /Count {page_count} >>"),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_owned(),
    ];

    for (index, _) in streams.iter().enumerate() {
        let page_object_id = first_page_object_id + index;
        let content_object_id = first_content_object_id + index;
        objects.push(format!(
            "<< /Type /Page /Parent 2 0 R /Resources << /Font << /F1 {font_object_id} 0 R >> >> /MediaBox [0 0 612 792] /Contents {content_object_id} 0 R >>"
        ));
        debug_assert_eq!(objects.len(), page_object_id);
    }

    for stream in streams {
        objects.push(format!(
            "<< /Length {} >>\nstream\n{}endstream",
            stream.len(),
            stream
        ));
    }

    pdf_objects_to_bytes(&objects)
}

fn pdf_page_streams(content: &str) -> Vec<String> {
    let mut streams = Vec::new();
    let mut stream = String::new();
    let mut y = 720i32;
    let mut has_text_on_page = false;

    for line in content.split('\n') {
        let line = pdf_line_style(line);
        if y - line.advance < 72 && has_text_on_page {
            streams.push(stream);
            stream = String::new();
            y = 720;
            has_text_on_page = false;
        }

        if let Some(text) = line.text {
            stream.push_str("BT\n");
            stream.push_str(&format!("/F1 {} Tf\n", line.font_size));
            stream.push_str(&format!("72 {y} Td\n"));
            stream.push_str(&format!("({}) Tj\n", pdf_escape_literal_text(&text)));
            stream.push_str("ET\n");
            has_text_on_page = true;
        }
        y -= line.advance;
    }

    streams.push(stream);
    streams
}

struct PdfLineStyle {
    text: Option<String>,
    font_size: u32,
    advance: i32,
}

fn pdf_line_style(line: &str) -> PdfLineStyle {
    if line.trim().is_empty() {
        return PdfLineStyle {
            text: None,
            font_size: 12,
            advance: 6,
        };
    }

    if let Some(text) = line.strip_prefix("# ") {
        return PdfLineStyle {
            text: Some(text.to_owned()),
            font_size: 24,
            advance: 34,
        };
    }
    if let Some(text) = line.strip_prefix("## ") {
        return PdfLineStyle {
            text: Some(text.to_owned()),
            font_size: 18,
            advance: 26,
        };
    }
    if let Some(text) = line.strip_prefix("### ") {
        return PdfLineStyle {
            text: Some(text.to_owned()),
            font_size: 14,
            advance: 20,
        };
    }

    PdfLineStyle {
        text: Some(line.to_owned()),
        font_size: 12,
        advance: 17,
    }
}

fn pdf_escape_literal_text(text: &str) -> String {
    let mut escaped = String::new();
    for character in text.chars() {
        match character {
            '\\' => escaped.push_str(r"\\"),
            '(' => escaped.push_str(r"\("),
            ')' => escaped.push_str(r"\)"),
            '\t' => escaped.push_str(r"\t"),
            '\r' => {}
            character if character.is_control() => {
                escaped.push_str(&format!(r"\{:03o}", character as u32));
            }
            character => escaped.push(character),
        }
    }
    escaped
}

fn pdf_objects_to_bytes(objects: &[String]) -> Vec<u8> {
    let mut pdf = b"%PDF-1.4\n".to_vec();
    let mut offsets = Vec::with_capacity(objects.len());
    for (index, object) in objects.iter().enumerate() {
        offsets.push(pdf.len());
        pdf.extend_from_slice(format!("{} 0 obj\n{}\nendobj\n", index + 1, object).as_bytes());
    }
    let xref_offset = pdf.len();
    pdf.extend_from_slice(format!("xref\n0 {}\n", objects.len() + 1).as_bytes());
    pdf.extend_from_slice(b"0000000000 65535 f \n");
    for offset in offsets {
        pdf.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    pdf.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{}\n%%EOF\n",
            objects.len() + 1,
            xref_offset
        )
        .as_bytes(),
    );
    pdf
}

pub(crate) fn read_docx_text(file_name: &str) -> Result<String, String> {
    let file = std::fs::File::open(file_name).map_err(|error| error.to_string())?;
    let mut archive = zip::ZipArchive::new(file).map_err(|error| error.to_string())?;
    let mut document = archive
        .by_name("word/document.xml")
        .map_err(|error| error.to_string())?;
    let mut xml = String::new();
    document
        .read_to_string(&mut xml)
        .map_err(|error| error.to_string())?;
    docx_document_xml_to_text(&xml)
}

fn write_docx_text(path: &std::path::Path, content: &str) -> Result<(), String> {
    let file = std::fs::File::create(path).map_err(|error| error.to_string())?;
    let mut archive = zip::ZipWriter::new(file);
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);

    archive
        .start_file("[Content_Types].xml", options)
        .map_err(|error| error.to_string())?;
    archive
        .write_all(docx_content_types_xml().as_bytes())
        .map_err(|error| error.to_string())?;
    archive
        .start_file("_rels/.rels", options)
        .map_err(|error| error.to_string())?;
    archive
        .write_all(docx_root_relationships_xml().as_bytes())
        .map_err(|error| error.to_string())?;
    archive
        .start_file("word/document.xml", options)
        .map_err(|error| error.to_string())?;
    archive
        .write_all(docx_document_xml(content).as_bytes())
        .map_err(|error| error.to_string())?;
    archive.finish().map_err(|error| error.to_string())?;
    Ok(())
}

fn docx_content_types_xml() -> &'static str {
    r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/></Types>"#
}

fn docx_root_relationships_xml() -> &'static str {
    r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#
}

fn docx_document_xml(content: &str) -> String {
    let mut xml = String::from(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body>"#,
    );
    for paragraph in content.split('\n') {
        xml.push_str("<w:p>");
        xml.push_str(&docx_paragraph_runs(paragraph));
        xml.push_str("</w:p>");
    }
    xml.push_str(r#"<w:sectPr><w:pgSz w:w="12240" w:h="15840"/><w:pgMar w:top="1440" w:right="1440" w:bottom="1440" w:left="1440" w:header="720" w:footer="720" w:gutter="0"/></w:sectPr></w:body></w:document>"#);
    xml
}

fn docx_paragraph_runs(paragraph: &str) -> String {
    let mut runs = String::new();
    let mut text = String::new();
    for character in paragraph.chars() {
        if character == '\t' {
            push_docx_text_run(&mut runs, &text);
            text.clear();
            runs.push_str("<w:r><w:tab/></w:r>");
        } else {
            text.push(character);
        }
    }
    push_docx_text_run(&mut runs, &text);
    runs
}

fn push_docx_text_run(runs: &mut String, text: &str) {
    if text.is_empty() {
        return;
    }
    runs.push_str(r#"<w:r><w:t xml:space="preserve">"#);
    push_xml_escaped(runs, text);
    runs.push_str("</w:t></w:r>");
}

fn push_xml_escaped(output: &mut String, text: &str) {
    for character in text.chars() {
        match character {
            '&' => output.push_str("&amp;"),
            '<' => output.push_str("&lt;"),
            '>' => output.push_str("&gt;"),
            _ => output.push(character),
        }
    }
}

fn docx_document_xml_to_text(xml: &str) -> Result<String, String> {
    let mut reader = quick_xml::Reader::from_str(xml);
    reader.config_mut().trim_text(false);
    let mut text = String::new();
    let mut in_text = false;

    loop {
        match reader.read_event() {
            Ok(quick_xml::events::Event::Eof) => break,
            Ok(quick_xml::events::Event::Start(event))
                if local_xml_name(event.name().as_ref()) == b"t" =>
            {
                in_text = true;
            }
            Ok(quick_xml::events::Event::End(event))
                if local_xml_name(event.name().as_ref()) == b"t" =>
            {
                in_text = false;
            }
            Ok(quick_xml::events::Event::Text(event)) => {
                if in_text {
                    let decoded = event.decode().map_err(|error| error.to_string())?;
                    text.push_str(&decoded);
                }
            }
            Ok(quick_xml::events::Event::CData(event)) => {
                if in_text {
                    let decoded = event.decode().map_err(|error| error.to_string())?;
                    text.push_str(&decoded);
                }
            }
            Ok(quick_xml::events::Event::GeneralRef(event)) => {
                if in_text {
                    let decoded = event.decode().map_err(|error| error.to_string())?;
                    text.push_str(&decode_xml_general_ref(&decoded)?);
                }
            }
            Ok(quick_xml::events::Event::Empty(event))
                if local_xml_name(event.name().as_ref()) == b"tab" =>
            {
                text.push('\t');
            }
            Ok(quick_xml::events::Event::Empty(event))
                if local_xml_name(event.name().as_ref()) == b"br" =>
            {
                push_docx_newline(&mut text);
            }
            Ok(quick_xml::events::Event::End(event))
                if local_xml_name(event.name().as_ref()) == b"p" =>
            {
                push_docx_newline(&mut text);
            }
            Ok(_) => {}
            Err(error) => return Err(error.to_string()),
        }
    }

    Ok(text.trim_end_matches('\n').to_owned())
}

fn decode_xml_general_ref(reference: &str) -> Result<String, String> {
    match reference {
        "amp" => return Ok("&".to_owned()),
        "lt" => return Ok("<".to_owned()),
        "gt" => return Ok(">".to_owned()),
        "quot" => return Ok("\"".to_owned()),
        "apos" => return Ok("'".to_owned()),
        _ => {}
    }

    let value = if let Some(hex) = reference.strip_prefix("#x") {
        u32::from_str_radix(hex, 16).map_err(|error| error.to_string())?
    } else if let Some(decimal) = reference.strip_prefix('#') {
        decimal.parse::<u32>().map_err(|error| error.to_string())?
    } else {
        return Err(format!("unsupported XML entity reference '&{reference};'"));
    };
    char::from_u32(value)
        .map(|character| character.to_string())
        .ok_or_else(|| format!("invalid XML character reference '&{reference};'"))
}

fn local_xml_name(name: &[u8]) -> &[u8] {
    name.rsplit(|byte| *byte == b':').next().unwrap_or(name)
}

fn push_docx_newline(text: &mut String) {
    if !text.ends_with('\n') {
        text.push('\n');
    }
}

fn truncate_read_document_content(content: &str) -> String {
    const MAX_CHARS: usize = 60_000;
    if content.chars().count() <= MAX_CHARS {
        return content.to_owned();
    }

    let mut truncated = content.chars().take(MAX_CHARS).collect::<String>();
    truncated.push_str("\n[...truncated]");
    truncated
}

fn read_image_file_action(file_name: &str) -> Result<ActionResult, BrowserError> {
    let bytes =
        std::fs::read(file_name).map_err(|error| BrowserError::ActionFailed(error.to_string()))?;
    let data = base64::engine::general_purpose::STANDARD.encode(bytes);
    let image_name = std::path::Path::new(file_name)
        .file_name()
        .and_then(std::ffi::OsStr::to_str)
        .unwrap_or(file_name)
        .to_owned();

    Ok(ActionResult {
        extracted_content: Some(format!("Read image file {file_name}.")),
        error: None,
        judgement: None,
        long_term_memory: Some(format!("Read image file {file_name}")),
        include_extracted_content_only_once: true,
        include_in_memory: true,
        is_done: false,
        success: None,
        attachments: Vec::new(),
        images: vec![serde_json::json!({
            "name": image_name,
            "data": data,
        })],
        metadata: None,
    })
}

pub(crate) fn replace_file_action(
    file_name: &str,
    old_str: &str,
    new_str: &str,
) -> Result<ActionResult, BrowserError> {
    let resolved_file = resolve_file_action_path(file_name, supported_text_extensions());
    if let Some(result) = validate_text_file_name(&resolved_file.display_name) {
        return Ok(result);
    }
    if old_str.is_empty() {
        return Ok(ActionResult::error(
            "Cannot replace empty string. Please provide a non-empty string to replace.",
        ));
    }
    let content = std::fs::read_to_string(&resolved_file.path)
        .map_err(|error| BrowserError::ActionFailed(error.to_string()))?;
    if !content.contains(old_str) {
        return Ok(ActionResult::error(format!(
            "Could not find text to replace in {}",
            resolved_file.display_name
        )));
    }
    let updated = content.replace(old_str, new_str);
    std::fs::write(&resolved_file.path, updated)
        .map_err(|error| BrowserError::ActionFailed(error.to_string()))?;
    Ok(ActionResult::extracted(format!(
        "Replaced text in file {}{}",
        resolved_file.display_name,
        resolved_file.correction_suffix()
    )))
}

/// Default directory name created inside each managed file-system base dir.
pub const DEFAULT_FILE_SYSTEM_PATH: &str = "browseruse_agent_data";

/// Serializable snapshot of the managed file system.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct FileSystemState {
    /// Stored files keyed by managed file name.
    pub files: BTreeMap<String, FileSystemStoredFile>,
    /// Base directory containing the managed data directory.
    pub base_dir: String,
    /// Counter used to name extracted-content files.
    pub extracted_content_count: usize,
}

/// One file stored in the managed file system.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct FileSystemStoredFile {
    /// Browser-use file type label, for example `MarkdownFile`.
    #[serde(rename = "type")]
    pub file_type: String,
    /// File payload.
    pub data: FileSystemFileData,
}

/// Data payload for a managed file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct FileSystemFileData {
    /// File stem without extension.
    pub name: String,
    /// Text representation of the file contents.
    pub content: String,
}

/// In-memory and on-disk managed file sandbox.
#[derive(Debug, Clone)]
pub struct ManagedFileSystem {
    base_dir: std::path::PathBuf,
    data_dir: std::path::PathBuf,
    files: BTreeMap<String, FileSystemStoredFile>,
    extracted_content_count: usize,
}

impl ManagedFileSystem {
    /// Creates a managed file system in a fresh temporary base directory.
    pub fn new_in_temp() -> Result<Self, BrowserError> {
        let base_dir = std::env::temp_dir().join(format!("browser_use_agent_{}", Uuid::now_v7()));
        Self::new(base_dir)
    }

    /// Creates a managed file system under `base_dir`.
    pub fn new(base_dir: impl Into<std::path::PathBuf>) -> Result<Self, BrowserError> {
        let base_dir = base_dir.into();
        std::fs::create_dir_all(&base_dir)
            .map_err(|error| BrowserError::ActionFailed(error.to_string()))?;
        let data_dir = base_dir.join(DEFAULT_FILE_SYSTEM_PATH);
        if data_dir.exists() {
            std::fs::remove_dir_all(&data_dir)
                .map_err(|error| BrowserError::ActionFailed(error.to_string()))?;
        }
        std::fs::create_dir_all(&data_dir)
            .map_err(|error| BrowserError::ActionFailed(error.to_string()))?;

        let mut file_system = Self {
            base_dir,
            data_dir,
            files: BTreeMap::new(),
            extracted_content_count: 0,
        };
        file_system.write_stored_file("todo.md", "")?;
        Ok(file_system)
    }

    /// Restores a managed file system from a serialized state snapshot.
    pub fn from_state(state: FileSystemState) -> Result<Self, BrowserError> {
        let base_dir = std::path::PathBuf::from(&state.base_dir);
        std::fs::create_dir_all(&base_dir)
            .map_err(|error| BrowserError::ActionFailed(error.to_string()))?;
        let data_dir = base_dir.join(DEFAULT_FILE_SYSTEM_PATH);
        if data_dir.exists() {
            std::fs::remove_dir_all(&data_dir)
                .map_err(|error| BrowserError::ActionFailed(error.to_string()))?;
        }
        std::fs::create_dir_all(&data_dir)
            .map_err(|error| BrowserError::ActionFailed(error.to_string()))?;

        let mut file_system = Self {
            base_dir,
            data_dir,
            files: BTreeMap::new(),
            extracted_content_count: state.extracted_content_count,
        };
        for (file_name, file) in state.files {
            if validate_write_file_name(&file_name).is_some() {
                continue;
            }
            file_system.sync_stored_file_to_disk(&file_name, &file.data.content)?;
            file_system.files.insert(file_name, file);
        }
        Ok(file_system)
    }

    /// Returns the base directory.
    pub fn base_dir(&self) -> &std::path::Path {
        &self.base_dir
    }

    /// Returns the managed data directory.
    pub fn data_dir(&self) -> &std::path::Path {
        &self.data_dir
    }

    /// Lists managed file names.
    pub fn list_files(&self) -> Vec<String> {
        self.files.keys().cloned().collect()
    }

    /// Returns current `todo.md` contents.
    pub fn get_todo_contents(&self) -> String {
        self.files
            .get("todo.md")
            .map(|file| file.data.content.clone())
            .unwrap_or_default()
    }

    /// Returns a serializable state snapshot.
    pub fn get_state(&self) -> FileSystemState {
        FileSystemState {
            files: self.files.clone(),
            base_dir: self.base_dir.display().to_string(),
            extracted_content_count: self.extracted_content_count,
        }
    }

    /// Deletes managed files from disk and clears in-memory state.
    pub fn nuke(&mut self) -> Result<(), BrowserError> {
        if self.data_dir.exists() {
            std::fs::remove_dir_all(&self.data_dir)
                .map_err(|error| BrowserError::ActionFailed(error.to_string()))?;
        }
        self.files.clear();
        Ok(())
    }

    /// Saves extracted content into a numbered managed markdown file.
    pub fn save_extracted_content(&mut self, content: &str) -> Result<String, BrowserError> {
        let stem = format!("extracted_content_{}", self.extracted_content_count);
        let file_name = format!("{stem}.md");
        self.write_stored_file(&file_name, content)?;
        self.extracted_content_count += 1;
        Ok(file_name)
    }

    /// Renders a compact XML-like description of managed files for prompts.
    pub fn describe(&self) -> String {
        const DISPLAY_CHARS: usize = 400;
        let mut description = String::new();
        for (file_name, file) in &self.files {
            if file_name == "todo.md" {
                continue;
            }

            let content = &file.data.content;
            if content.is_empty() {
                description.push_str(&format!("<file>\n{file_name} - [empty file]\n</file>\n"));
                continue;
            }

            let lines = content.lines().collect::<Vec<_>>();
            let line_count = lines.len();
            let whole_file_description = format!(
                "<file>\n{file_name} - {line_count} lines\n<content>\n{content}\n</content>\n</file>\n"
            );
            if content.chars().count() < DISPLAY_CHARS * 3 / 2 {
                description.push_str(&whole_file_description);
                continue;
            }

            let (start_preview, start_line_count) =
                preview_lines(lines.iter().copied(), DISPLAY_CHARS / 2);
            let (end_preview, end_line_count) =
                preview_lines(lines.iter().rev().copied(), DISPLAY_CHARS / 2);
            let middle_line_count = line_count.saturating_sub(start_line_count + end_line_count);
            if middle_line_count == 0 {
                description.push_str(&whole_file_description);
                continue;
            }

            let end_preview = end_preview
                .lines()
                .rev()
                .collect::<Vec<_>>()
                .join("\n")
                .trim()
                .to_owned();
            description.push_str(&format!(
                "<file>\n{file_name} - {line_count} lines\n<content>\n{}\n... {middle_line_count} more lines ...\n{end_preview}\n</content>\n</file>\n",
                start_preview.trim()
            ));
        }
        description.trim_end_matches('\n').to_owned()
    }

    /// Returns text for a managed file if it can be displayed inline.
    pub fn display_file(&self, file_name: &str) -> Option<String> {
        let resolved_file = resolve_file_action_path_at(
            file_name,
            supported_text_extensions(),
            Some(&self.data_dir),
        );
        if validate_text_file_name(&resolved_file.display_name).is_some() {
            return None;
        }
        self.files
            .get(&resolved_file.display_name)
            .map(|file| file.data.content.clone())
    }

    /// Returns final-answer display text and attachment path for a file.
    pub fn display_done_file(&self, file_name: &str) -> Option<(String, String)> {
        if std::path::Path::new(file_name).is_absolute() {
            return display_done_file(file_name);
        }

        let resolved_file = resolve_file_action_path_at(
            file_name,
            supported_text_extensions(),
            Some(&self.data_dir),
        );
        if validate_text_file_name(&resolved_file.display_name).is_some() {
            return None;
        }

        let content = self
            .files
            .get(&resolved_file.display_name)
            .map(|file| file.data.content.clone())?;
        let attachment = std::fs::canonicalize(&resolved_file.path)
            .unwrap_or_else(|_| resolved_file.path.clone())
            .display()
            .to_string();
        Some((
            format!("{}:\n{content}", resolved_file.display_name),
            attachment,
        ))
    }

    /// Resolves a managed file path for upload actions.
    pub fn upload_file_path(&self, file_name: &str) -> Option<std::path::PathBuf> {
        if std::path::Path::new(file_name).is_absolute() {
            return None;
        }

        let resolved_file = resolve_file_action_path_at(
            file_name,
            supported_write_extensions(),
            Some(&self.data_dir),
        );
        if validate_write_file_name(&resolved_file.display_name).is_some()
            || !self.files.contains_key(&resolved_file.display_name)
        {
            return None;
        }

        Some(resolved_file.path)
    }

    /// Executes a write-file action against the managed sandbox or absolute path.
    pub fn write_file(
        &mut self,
        params: &browser_use_tools::WriteFileAction,
    ) -> Result<ActionResult, BrowserError> {
        if std::path::Path::new(&params.file_name).is_absolute() {
            return write_file_action(params);
        }

        let resolved_file = resolve_file_action_path_at(
            &params.file_name,
            supported_write_extensions(),
            Some(&self.data_dir),
        );
        if let Some(result) = validate_write_file_name(&resolved_file.display_name) {
            return Ok(result);
        }
        if params.append && !self.files.contains_key(&resolved_file.display_name) {
            return Ok(ActionResult::error(format!(
                "File '{}' not found.",
                resolved_file.display_name
            )));
        }

        let mut content = params.content.clone();
        if params.trailing_newline {
            content.push('\n');
        }
        if params.leading_newline {
            content.insert(0, '\n');
        }

        let stored_content = if params.append {
            let existing = self
                .files
                .get(&resolved_file.display_name)
                .map(|file| file.data.content.as_str())
                .unwrap_or_default();
            merge_managed_append_content(&resolved_file.display_name, existing, &content)
        } else if is_csv_file(&resolved_file.display_name) {
            normalize_csv_content(&content)
        } else {
            content
        };

        self.write_stored_file(&resolved_file.display_name, &stored_content)?;
        Ok(ActionResult::extracted(format!(
            "{} file {}{}",
            if params.append {
                "Appended to"
            } else {
                "Wrote"
            },
            resolved_file.display_name,
            resolved_file.correction_suffix()
        )))
    }

    /// Executes a read-file action against the managed sandbox or absolute path.
    pub fn read_file(&self, file_name: &str) -> Result<ActionResult, BrowserError> {
        if std::path::Path::new(file_name).is_absolute() {
            return read_file_action(file_name);
        }

        let read_extensions = supported_read_extensions();
        let resolved_file =
            resolve_file_action_path_at(file_name, &read_extensions, Some(&self.data_dir));
        if let Some(result) = validate_read_file_name(&resolved_file.display_name) {
            return Ok(result);
        }
        let Some(file) = self.files.get(&resolved_file.display_name) else {
            return Ok(ActionResult::error(format!(
                "File '{}' not found.{}",
                resolved_file.display_name,
                if resolved_file.was_corrected {
                    format!(
                        " (Filename was auto-corrected from '{}')",
                        resolved_file.original_name
                    )
                } else {
                    String::new()
                }
            )));
        };
        let content = &file.data.content;
        let memory = read_file_memory(content);
        Ok(ActionResult {
            extracted_content: Some(format!(
                "{}Read from file {}.\n<content>\n{content}\n</content>",
                resolved_file.correction_prefix(),
                resolved_file.display_name
            )),
            error: None,
            judgement: None,
            long_term_memory: Some(memory),
            include_extracted_content_only_once: true,
            include_in_memory: true,
            is_done: false,
            success: None,
            attachments: Vec::new(),
            images: Vec::new(),
            metadata: None,
        })
    }

    /// Executes a replace-file action against the managed sandbox or absolute path.
    pub fn replace_file(
        &mut self,
        file_name: &str,
        old_str: &str,
        new_str: &str,
    ) -> Result<ActionResult, BrowserError> {
        if std::path::Path::new(file_name).is_absolute() {
            return replace_file_action(file_name, old_str, new_str);
        }

        let resolved_file = resolve_file_action_path_at(
            file_name,
            supported_text_extensions(),
            Some(&self.data_dir),
        );
        if let Some(result) = validate_text_file_name(&resolved_file.display_name) {
            return Ok(result);
        }
        if old_str.is_empty() {
            return Ok(ActionResult::error(
                "Cannot replace empty string. Please provide a non-empty string to replace.",
            ));
        }
        let Some(existing) = self.files.get(&resolved_file.display_name) else {
            return Ok(ActionResult::error(format!(
                "File '{}' not found.",
                resolved_file.display_name
            )));
        };
        if !existing.data.content.contains(old_str) {
            return Ok(ActionResult::error(format!(
                "Could not find text to replace in {}",
                resolved_file.display_name
            )));
        }
        let updated = existing.data.content.replace(old_str, new_str);
        self.write_stored_file(&resolved_file.display_name, &updated)?;
        Ok(ActionResult::extracted(format!(
            "Replaced text in file {}{}",
            resolved_file.display_name,
            resolved_file.correction_suffix()
        )))
    }

    fn write_stored_file(&mut self, file_name: &str, content: &str) -> Result<(), BrowserError> {
        self.sync_stored_file_to_disk(file_name, content)?;
        self.files
            .insert(file_name.to_owned(), stored_file_state(file_name, content)?);
        Ok(())
    }

    fn sync_stored_file_to_disk(&self, file_name: &str, content: &str) -> Result<(), BrowserError> {
        let path = self.data_dir.join(file_name);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|error| BrowserError::ActionFailed(error.to_string()))?;
        }
        write_supported_artifact(&path, file_name, content)
    }
}

fn preview_lines<'a>(lines: impl Iterator<Item = &'a str>, max_chars: usize) -> (String, usize) {
    let mut preview = String::new();
    let mut line_count = 0;
    let mut chars_count = 0;
    for line in lines {
        let next = line.chars().count() + 1;
        if chars_count + next > max_chars {
            break;
        }
        preview.push_str(line);
        preview.push('\n');
        chars_count += next;
        line_count += 1;
    }
    (preview, line_count)
}

fn merge_managed_append_content(file_name: &str, existing: &str, new_content: &str) -> String {
    if is_pdf_file(file_name) {
        merge_pdf_append_content(existing, new_content)
    } else if is_docx_file(file_name) {
        merge_docx_append_content(existing, new_content)
    } else if is_csv_file(file_name) {
        merge_csv_append_content(existing, new_content)
    } else {
        let mut merged = existing.to_owned();
        merged.push_str(new_content);
        merged
    }
}

fn stored_file_state(file_name: &str, content: &str) -> Result<FileSystemStoredFile, BrowserError> {
    let Some((name, extension)) = file_name.rsplit_once('.') else {
        return Err(BrowserError::ActionFailed(format!(
            "Filename '{file_name}' has no extension"
        )));
    };
    let file_type = file_type_for_extension(extension).ok_or_else(|| {
        BrowserError::ActionFailed(format!("Unsupported managed file extension '.{extension}'"))
    })?;
    Ok(FileSystemStoredFile {
        file_type: file_type.to_owned(),
        data: FileSystemFileData {
            name: name.to_owned(),
            content: content.to_owned(),
        },
    })
}

fn file_type_for_extension(extension: &str) -> Option<&'static str> {
    match extension.to_ascii_lowercase().as_str() {
        "md" => Some("MarkdownFile"),
        "txt" => Some("TxtFile"),
        "json" => Some("JsonFile"),
        "jsonl" => Some("JsonlFile"),
        "csv" => Some("CsvFile"),
        "pdf" => Some("PdfFile"),
        "docx" => Some("DocxFile"),
        "html" => Some("HtmlFile"),
        "xml" => Some("XmlFile"),
        _ => None,
    }
}

fn write_supported_artifact(
    path: &std::path::Path,
    file_name: &str,
    content: &str,
) -> Result<(), BrowserError> {
    if is_pdf_file(file_name) {
        write_pdf_text(path, content).map_err(BrowserError::ActionFailed)
    } else if is_docx_file(file_name) {
        write_docx_text(path, content).map_err(BrowserError::ActionFailed)
    } else {
        std::fs::write(path, content).map_err(|error| BrowserError::ActionFailed(error.to_string()))
    }
}

#[derive(Debug, Clone)]
pub(crate) struct ResolvedFileActionPath {
    pub(crate) path: std::path::PathBuf,
    pub(crate) display_name: String,
    pub(crate) original_name: String,
    pub(crate) was_corrected: bool,
}

impl ResolvedFileActionPath {
    fn path_string(&self) -> String {
        self.path.to_string_lossy().into_owned()
    }

    fn correction_suffix(&self) -> String {
        if self.was_corrected {
            format!(" (auto-corrected from '{}')", self.original_name)
        } else {
            String::new()
        }
    }

    fn correction_prefix(&self) -> String {
        if self.was_corrected {
            format!(
                "Note: filename was auto-corrected from '{}' to '{}'. ",
                self.original_name, self.display_name
            )
        } else {
            String::new()
        }
    }
}

fn apply_file_name_correction_note(
    result: &mut ActionResult,
    resolved_file: &ResolvedFileActionPath,
) {
    if !resolved_file.was_corrected {
        return;
    }
    if let Some(content) = result.extracted_content.as_mut() {
        content.insert_str(0, &resolved_file.correction_prefix());
    }
    if let Some(memory) = result.long_term_memory.as_mut() {
        memory.insert_str(0, &resolved_file.correction_prefix());
    }
}

pub(crate) fn resolve_file_action_path(
    file_name: &str,
    supported_extensions: &[&str],
) -> ResolvedFileActionPath {
    resolve_file_action_path_at(file_name, supported_extensions, None)
}

fn resolve_file_action_path_at(
    file_name: &str,
    supported_extensions: &[&str],
    relative_root: Option<&std::path::Path>,
) -> ResolvedFileActionPath {
    let path = std::path::Path::new(file_name);
    if path.is_absolute() {
        return ResolvedFileActionPath {
            path: path.to_path_buf(),
            display_name: file_name.to_owned(),
            original_name: file_name.to_owned(),
            was_corrected: false,
        };
    }

    let base_name = path
        .file_name()
        .and_then(std::ffi::OsStr::to_str)
        .unwrap_or(file_name);
    let mut display_name = base_name.to_owned();
    let mut was_corrected = base_name != file_name;

    if !is_valid_action_file_name(&display_name, supported_extensions) {
        let sanitized = sanitize_action_file_name(&display_name);
        if sanitized != display_name && is_valid_action_file_name(&sanitized, supported_extensions)
        {
            display_name = sanitized;
            was_corrected = true;
        }
    }

    let path = relative_root
        .map(|root| root.join(&display_name))
        .unwrap_or_else(|| std::path::PathBuf::from(&display_name));

    ResolvedFileActionPath {
        path,
        display_name,
        original_name: file_name.to_owned(),
        was_corrected,
    }
}

fn is_valid_action_file_name(file_name: &str, supported_extensions: &[&str]) -> bool {
    let Some((name, extension)) = file_name.rsplit_once('.') else {
        return false;
    };
    if name.trim().is_empty() {
        return false;
    }
    let extension = extension.to_ascii_lowercase();
    if !supported_extensions.contains(&extension.as_str()) {
        return false;
    }
    name.chars().all(is_valid_action_file_name_char)
}

fn is_valid_action_file_name_char(character: char) -> bool {
    character.is_ascii_alphanumeric()
        || matches!(character, '_' | '-' | '.' | '(' | ')' | ' ')
        || ('\u{4e00}'..='\u{9fff}').contains(&character)
}

fn sanitize_action_file_name(file_name: &str) -> String {
    let Some((name, extension)) = file_name.rsplit_once('.') else {
        return file_name.to_owned();
    };
    let mut sanitized_name = name
        .replace(' ', "-")
        .chars()
        .filter(|character| {
            character.is_ascii_alphanumeric()
                || matches!(*character, '_' | '-' | '.' | '(' | ')')
                || ('\u{4e00}'..='\u{9fff}').contains(character)
        })
        .collect::<String>();
    while sanitized_name.contains("--") {
        sanitized_name = sanitized_name.replace("--", "-");
    }
    sanitized_name = sanitized_name
        .trim_matches(|character| character == '-' || character == '.')
        .to_owned();
    if sanitized_name.is_empty() {
        sanitized_name = "file".to_owned();
    }
    format!("{}.{}", sanitized_name, extension.to_ascii_lowercase())
}

fn validate_write_file_name(file_name: &str) -> Option<ActionResult> {
    let path = std::path::Path::new(file_name);
    let base_name = path
        .file_name()
        .and_then(std::ffi::OsStr::to_str)
        .unwrap_or(file_name);
    let Some(extension) = path.extension().and_then(std::ffi::OsStr::to_str) else {
        return Some(ActionResult::error(format!(
            "Filename '{base_name}' has no extension. Supported extensions: {}.",
            supported_write_extensions_message()
        )));
    };
    let extension = extension.to_ascii_lowercase();

    if unsupported_binary_extensions().contains(&extension.as_str()) {
        return Some(ActionResult::error(format!(
            "Cannot write binary/image file '{base_name}'. The write_file action supports text files and PDF/DOCX documents. Supported extensions: {}.",
            supported_write_extensions_message()
        )));
    }

    if !supported_write_extensions().contains(&extension.as_str()) {
        return Some(ActionResult::error(format!(
            "Unsupported file extension '.{extension}' in '{base_name}'. Supported extensions: {}.",
            supported_write_extensions_message()
        )));
    }

    None
}

fn validate_text_file_name(file_name: &str) -> Option<ActionResult> {
    let path = std::path::Path::new(file_name);
    let base_name = path
        .file_name()
        .and_then(std::ffi::OsStr::to_str)
        .unwrap_or(file_name);
    let Some(extension) = path.extension().and_then(std::ffi::OsStr::to_str) else {
        return Some(ActionResult::error(format!(
            "Filename '{base_name}' has no extension. Supported extensions: {}.",
            supported_text_extensions_message()
        )));
    };
    let extension = extension.to_ascii_lowercase();

    if unsupported_binary_extensions().contains(&extension.as_str()) {
        return Some(ActionResult::error(format!(
            "Cannot write binary/image file '{base_name}'. The file actions only support text-based files. Supported extensions: {}.",
            supported_text_extensions_message()
        )));
    }

    if !supported_text_extensions().contains(&extension.as_str()) {
        return Some(ActionResult::error(format!(
            "Unsupported file extension '.{extension}' in '{base_name}'. Supported extensions: {}.",
            supported_text_extensions_message()
        )));
    }

    None
}

fn validate_read_file_name(file_name: &str) -> Option<ActionResult> {
    let path = std::path::Path::new(file_name);
    let base_name = path
        .file_name()
        .and_then(std::ffi::OsStr::to_str)
        .unwrap_or(file_name);
    let Some(extension) = path.extension().and_then(std::ffi::OsStr::to_str) else {
        return Some(ActionResult::error(format!(
            "Filename '{base_name}' has no extension. Supported extensions: {}.",
            supported_read_extensions_message()
        )));
    };
    let extension = extension.to_ascii_lowercase();

    if supported_text_extensions().contains(&extension.as_str())
        || supported_read_image_extensions().contains(&extension.as_str())
        || supported_read_document_extensions().contains(&extension.as_str())
    {
        return None;
    }

    if unsupported_binary_extensions().contains(&extension.as_str()) {
        return Some(ActionResult::error(format!(
            "Cannot read binary/image file '{base_name}'. The read_file action supports text files, PDF/DOCX documents, and PNG/JPEG images. Supported extensions: {}.",
            supported_read_extensions_message()
        )));
    }

    Some(ActionResult::error(format!(
        "Unsupported file extension '.{extension}' in '{base_name}'. Supported extensions: {}.",
        supported_read_extensions_message()
    )))
}

pub(crate) fn supported_text_extensions() -> &'static [&'static str] {
    &["txt", "md", "json", "jsonl", "csv", "html", "xml"]
}

fn supported_write_extensions() -> &'static [&'static str] {
    &[
        "txt", "md", "json", "jsonl", "csv", "html", "xml", "pdf", "docx",
    ]
}

fn supported_read_image_extensions() -> &'static [&'static str] {
    &["png", "jpg", "jpeg"]
}

fn supported_read_document_extensions() -> &'static [&'static str] {
    &["pdf", "docx"]
}

fn supported_read_extensions() -> Vec<&'static str> {
    supported_text_extensions()
        .iter()
        .chain(supported_read_document_extensions().iter())
        .chain(supported_read_image_extensions().iter())
        .copied()
        .collect()
}

fn is_supported_read_image_file(file_name: &str) -> bool {
    file_extension(file_name)
        .is_some_and(|extension| supported_read_image_extensions().contains(&extension.as_str()))
}

fn is_supported_read_document_file(file_name: &str) -> bool {
    file_extension(file_name)
        .is_some_and(|extension| supported_read_document_extensions().contains(&extension.as_str()))
}

fn file_extension(file_name: &str) -> Option<String> {
    std::path::Path::new(file_name)
        .extension()
        .and_then(std::ffi::OsStr::to_str)
        .map(str::to_ascii_lowercase)
}

fn unsupported_binary_extensions() -> &'static [&'static str] {
    &[
        "png", "jpg", "jpeg", "gif", "bmp", "svg", "webp", "ico", "mp3", "mp4", "wav", "avi",
        "mov", "zip", "tar", "gz", "rar", "exe", "bin", "dll", "so",
    ]
}

fn supported_text_extensions_message() -> String {
    supported_text_extensions()
        .iter()
        .map(|extension| format!(".{extension}"))
        .collect::<Vec<_>>()
        .join(", ")
}

fn supported_write_extensions_message() -> String {
    supported_write_extensions()
        .iter()
        .map(|extension| format!(".{extension}"))
        .collect::<Vec<_>>()
        .join(", ")
}

fn supported_read_extensions_message() -> String {
    supported_text_extensions()
        .iter()
        .chain(supported_read_document_extensions().iter())
        .chain(supported_read_image_extensions().iter())
        .map(|extension| format!(".{extension}"))
        .collect::<Vec<_>>()
        .join(", ")
}

fn read_file_memory(content: &str) -> String {
    const MAX_MEMORY_SIZE: usize = 1_000;
    if content.len() <= MAX_MEMORY_SIZE {
        return content.to_owned();
    }

    let mut display = String::new();
    let mut lines_count = 0;
    let lines: Vec<&str> = content.lines().collect();
    for line in &lines {
        if display.len() + line.len() + 1 < MAX_MEMORY_SIZE {
            display.push_str(line);
            display.push('\n');
            lines_count += 1;
        } else {
            break;
        }
    }
    let remaining_lines = lines.len().saturating_sub(lines_count);
    if remaining_lines > 0 {
        format!("{display}{remaining_lines} more lines...")
    } else {
        display
    }
}
