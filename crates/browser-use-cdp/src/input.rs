//! Keyboard input normalization for CDP.
//!
//! Browser-use accepts human-readable key strings such as `Enter`,
//! `Control+A`, or text. This module normalizes those strings into the CDP
//! `Input.dispatchKeyEvent` parameters and modifier bitmasks expected by
//! Chrome.

use serde_json::{Value, json};

pub(crate) fn normalize_send_keys(keys: &str) -> String {
    if keys.contains('+') {
        return keys
            .split('+')
            .map(normalize_key_alias)
            .collect::<Vec<_>>()
            .join("+");
    }

    normalize_key_or_text(keys)
}

fn normalize_key_alias(key: &str) -> String {
    key_alias(key).unwrap_or_else(|| key.trim().to_owned())
}

fn normalize_key_or_text(key: &str) -> String {
    key_alias(key).unwrap_or_else(|| key.to_owned())
}

fn key_alias(key: &str) -> Option<String> {
    Some(match key.trim().to_ascii_lowercase().as_str() {
        "ctrl" | "control" => "Control".to_owned(),
        "alt" | "option" => "Alt".to_owned(),
        "meta" | "cmd" | "command" => "Meta".to_owned(),
        "shift" => "Shift".to_owned(),
        "enter" | "return" => "Enter".to_owned(),
        "tab" => "Tab".to_owned(),
        "delete" => "Delete".to_owned(),
        "backspace" => "Backspace".to_owned(),
        "escape" | "esc" => "Escape".to_owned(),
        "space" => " ".to_owned(),
        "up" => "ArrowUp".to_owned(),
        "down" => "ArrowDown".to_owned(),
        "left" => "ArrowLeft".to_owned(),
        "right" => "ArrowRight".to_owned(),
        "pageup" => "PageUp".to_owned(),
        "pagedown" => "PageDown".to_owned(),
        "home" => "Home".to_owned(),
        "end" => "End".to_owned(),
        _ => return None,
    })
}

pub(crate) fn is_special_key(key: &str) -> bool {
    matches!(
        key,
        "Enter"
            | "Tab"
            | "Delete"
            | "Backspace"
            | "Escape"
            | "ArrowUp"
            | "ArrowDown"
            | "ArrowLeft"
            | "ArrowRight"
            | "PageUp"
            | "PageDown"
            | "Home"
            | "End"
            | "Control"
            | "Alt"
            | "Meta"
            | "Shift"
            | "F1"
            | "F2"
            | "F3"
            | "F4"
            | "F5"
            | "F6"
            | "F7"
            | "F8"
            | "F9"
            | "F10"
            | "F11"
            | "F12"
    )
}

pub(crate) fn modifier_mask(modifiers: &[String]) -> i64 {
    modifiers.iter().fold(0, |mask, modifier| {
        mask | match modifier.as_str() {
            "Alt" => 1,
            "Control" => 2,
            "Meta" => 4,
            "Shift" => 8,
            _ => 0,
        }
    })
}

fn key_info(key: &str) -> (String, Option<i64>) {
    match key {
        "Enter" => ("Enter".to_owned(), Some(13)),
        "Tab" => ("Tab".to_owned(), Some(9)),
        "Delete" => ("Delete".to_owned(), Some(46)),
        "Backspace" => ("Backspace".to_owned(), Some(8)),
        "Escape" => ("Escape".to_owned(), Some(27)),
        "ArrowUp" => ("ArrowUp".to_owned(), Some(38)),
        "ArrowDown" => ("ArrowDown".to_owned(), Some(40)),
        "ArrowLeft" => ("ArrowLeft".to_owned(), Some(37)),
        "ArrowRight" => ("ArrowRight".to_owned(), Some(39)),
        "PageUp" => ("PageUp".to_owned(), Some(33)),
        "PageDown" => ("PageDown".to_owned(), Some(34)),
        "Home" => ("Home".to_owned(), Some(36)),
        "End" => ("End".to_owned(), Some(35)),
        "Control" => ("ControlLeft".to_owned(), Some(17)),
        "Alt" => ("AltLeft".to_owned(), Some(18)),
        "Meta" => ("MetaLeft".to_owned(), Some(91)),
        "Shift" => ("ShiftLeft".to_owned(), Some(16)),
        " " => ("Space".to_owned(), Some(32)),
        function_key if function_key.starts_with('F') => {
            let number = function_key[1..].parse::<i64>().ok();
            if let Some(number @ 1..=12) = number {
                (function_key.to_owned(), Some(111 + number))
            } else {
                (function_key.to_owned(), None)
            }
        }
        single if single.chars().count() == 1 => {
            let lower = single.to_ascii_lowercase();
            let upper = lower.to_ascii_uppercase();
            let vk = upper.as_bytes().first().copied().map(i64::from);
            (format!("Key{upper}"), vk)
        }
        other => (other.to_owned(), None),
    }
}

pub(crate) fn key_event_params(event_type: &str, key: &str, modifiers: i64) -> Value {
    let key = if key.chars().count() == 1 {
        key.to_ascii_lowercase()
    } else {
        key.to_owned()
    };
    let (code, vk_code) = key_info(&key);
    let mut params = serde_json::Map::new();
    params.insert("type".to_owned(), json!(event_type));
    params.insert("key".to_owned(), json!(key));
    params.insert("code".to_owned(), json!(code));
    if modifiers != 0 {
        params.insert("modifiers".to_owned(), json!(modifiers));
    }
    if let Some(vk_code) = vk_code {
        params.insert("windowsVirtualKeyCode".to_owned(), json!(vk_code));
    }
    Value::Object(params)
}
