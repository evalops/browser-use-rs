use crate::ActionResult;
use browser_use_cdp::BrowserError;
use std::collections::{BTreeMap, BTreeSet};
use std::io::{Read, Write};

use super::{file_extension, is_pdf_file, read_file_memory};

pub(super) fn merge_pdf_append_content(existing: &str, new_content: &str) -> String {
    let existing = existing.trim_end_matches(['\n', '\r', '\u{c}']);
    merge_document_append_content(existing, new_content)
}

pub(super) fn merge_docx_append_content(existing: &str, new_content: &str) -> String {
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

pub(super) fn read_document_file_action(file_name: &str) -> Result<ActionResult, BrowserError> {
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

pub(super) fn write_pdf_text(path: &std::path::Path, content: &str) -> Result<(), String> {
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

pub(super) fn write_docx_text(path: &std::path::Path, content: &str) -> Result<(), String> {
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
