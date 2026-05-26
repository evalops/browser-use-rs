use crate::ActionResult;

#[derive(Debug, Clone)]
pub(crate) struct ResolvedFileActionPath {
    pub(crate) path: std::path::PathBuf,
    pub(crate) display_name: String,
    pub(crate) original_name: String,
    pub(crate) was_corrected: bool,
}

impl ResolvedFileActionPath {
    pub(super) fn path_string(&self) -> String {
        self.path.to_string_lossy().into_owned()
    }

    pub(super) fn correction_suffix(&self) -> String {
        if self.was_corrected {
            format!(" (auto-corrected from '{}')", self.original_name)
        } else {
            String::new()
        }
    }

    pub(super) fn correction_prefix(&self) -> String {
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

pub(super) fn apply_file_name_correction_note(
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

pub(super) fn resolve_file_action_path_at(
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

pub(super) fn validate_write_file_name(file_name: &str) -> Option<ActionResult> {
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

pub(super) fn validate_text_file_name(file_name: &str) -> Option<ActionResult> {
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

pub(super) fn validate_read_file_name(file_name: &str) -> Option<ActionResult> {
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

pub(super) fn supported_write_extensions() -> &'static [&'static str] {
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

pub(super) fn supported_read_extensions() -> Vec<&'static str> {
    supported_text_extensions()
        .iter()
        .chain(supported_read_document_extensions().iter())
        .chain(supported_read_image_extensions().iter())
        .copied()
        .collect()
}

pub(super) fn is_supported_read_image_file(file_name: &str) -> bool {
    file_extension(file_name)
        .is_some_and(|extension| supported_read_image_extensions().contains(&extension.as_str()))
}

pub(super) fn is_supported_read_document_file(file_name: &str) -> bool {
    file_extension(file_name)
        .is_some_and(|extension| supported_read_document_extensions().contains(&extension.as_str()))
}

pub(crate) fn file_extension(file_name: &str) -> Option<String> {
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

pub(crate) fn read_file_memory(content: &str) -> String {
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
