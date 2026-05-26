use crate::{ActionResult, ManagedFileSystem, display_done_file};
use std::collections::BTreeSet;
use std::path::PathBuf;

pub(super) fn done_action_result(
    params: &browser_use_tools::DoneAction,
    file_system: Option<&ManagedFileSystem>,
    display_files_in_done_text: bool,
) -> ActionResult {
    let mut user_message = params.text.clone();
    let mut file_sections = Vec::new();
    let mut attachments = Vec::new();

    for file_name in &params.files_to_display {
        let displayed_file = file_system
            .and_then(|file_system| file_system.display_done_file(file_name))
            .or_else(|| display_done_file(file_name));
        if let Some((section, attachment)) = displayed_file {
            if display_files_in_done_text {
                file_sections.push(section);
            }
            attachments.push(attachment);
        }
    }

    if !file_sections.is_empty() {
        user_message.push_str("\n\nAttachments:");
        for section in file_sections {
            user_message.push_str("\n\n");
            user_message.push_str(&section);
        }
    }

    ActionResult::done_with_attachments(user_message, params.success, attachments)
}

pub(super) fn upload_file_action_path(
    params: &browser_use_tools::UploadFileAction,
    file_system: &ManagedFileSystem,
    enforce_upload_file_availability: bool,
    available_file_paths: &BTreeSet<String>,
) -> Result<PathBuf, String> {
    if !enforce_upload_file_availability {
        return Ok(PathBuf::from(&params.path));
    }

    let path = if available_file_paths.contains(&params.path) {
        PathBuf::from(&params.path)
    } else if let Some(path) = file_system.upload_file_path(&params.path) {
        path
    } else {
        return Err(format!(
            "File path {} is not available. Add it to AgentSettings.available_file_paths before using upload_file.",
            params.path
        ));
    };

    if !path.exists() {
        return Err(format!("File {} does not exist", path.display()));
    }
    if path.metadata().map(|metadata| metadata.len()).unwrap_or(0) == 0 {
        return Err(format!(
            "File {} is empty (0 bytes). The file may not have been saved correctly.",
            path.display()
        ));
    }

    Ok(path)
}

pub(crate) fn pdf_output_path(file_name: Option<&str>, page_title: Option<&str>) -> PathBuf {
    let raw_name = file_name
        .filter(|name| !name.trim().is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| {
            page_title
                .map(sanitize_pdf_title)
                .filter(|title| !title.is_empty())
                .unwrap_or_else(|| "page".to_owned())
        });
    let path = PathBuf::from(raw_name);
    ensure_pdf_extension(path)
}

fn sanitize_pdf_title(title: &str) -> String {
    title
        .chars()
        .filter(|character| {
            character.is_alphanumeric()
                || *character == '_'
                || *character == ' '
                || *character == '-'
        })
        .collect::<String>()
        .trim()
        .chars()
        .take(50)
        .collect()
}

fn ensure_pdf_extension(mut path: PathBuf) -> PathBuf {
    let has_pdf_extension = path
        .file_name()
        .and_then(std::ffi::OsStr::to_str)
        .is_some_and(|file_name| file_name.to_ascii_lowercase().ends_with(".pdf"));
    if has_pdf_extension {
        return path;
    }

    let Some(file_name) = path
        .file_name()
        .and_then(std::ffi::OsStr::to_str)
        .map(str::to_owned)
    else {
        return PathBuf::from("page.pdf");
    };
    path.set_file_name(format!("{file_name}.pdf"));
    path
}

pub(crate) fn next_available_pdf_path(path: PathBuf) -> PathBuf {
    if !path.exists() {
        return path;
    }

    let parent = path.parent().map(std::path::Path::to_path_buf);
    let stem = path
        .file_stem()
        .and_then(std::ffi::OsStr::to_str)
        .unwrap_or("page");
    let extension = path
        .extension()
        .and_then(std::ffi::OsStr::to_str)
        .unwrap_or("pdf");

    for counter in 1.. {
        let candidate_name = format!("{stem} ({counter}).{extension}");
        let candidate = parent.as_ref().map_or_else(
            || PathBuf::from(&candidate_name),
            |parent| parent.join(&candidate_name),
        );
        if !candidate.exists() {
            return candidate;
        }
    }
    unreachable!("unbounded PDF filename counter should always return")
}

pub(crate) fn screenshot_output_path(file_name: &str) -> PathBuf {
    let path = PathBuf::from(if file_name.trim().is_empty() {
        "screenshot".to_owned()
    } else {
        file_name.to_owned()
    });
    ensure_png_extension(path)
}

fn ensure_png_extension(mut path: PathBuf) -> PathBuf {
    let has_png_extension = path
        .file_name()
        .and_then(std::ffi::OsStr::to_str)
        .is_some_and(|file_name| file_name.to_ascii_lowercase().ends_with(".png"));
    if has_png_extension {
        return path;
    }

    let Some(file_name) = path
        .file_name()
        .and_then(std::ffi::OsStr::to_str)
        .map(str::to_owned)
    else {
        return PathBuf::from("screenshot.png");
    };
    path.set_file_name(format!("{file_name}.png"));
    path
}
