//! Built-in browser action schemas and registry contracts.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Free-text or schema-guided page extraction.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ExtractAction {
    pub query: String,
    #[serde(default)]
    pub extract_links: bool,
    #[serde(default)]
    pub extract_images: bool,
    #[serde(default)]
    pub start_from_char: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_schema: Option<Value>,
    #[serde(default)]
    pub already_collected: Vec<String>,
}

/// Text or regex search against page content.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SearchPageAction {
    pub pattern: String,
    #[serde(default)]
    pub regex: bool,
    #[serde(default)]
    pub case_sensitive: bool,
    #[serde(default = "default_context_chars")]
    pub context_chars: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub css_scope: Option<String>,
    #[serde(default = "default_search_page_max_results")]
    pub max_results: usize,
}

fn default_context_chars() -> usize {
    150
}

fn default_search_page_max_results() -> usize {
    25
}

/// CSS selector lookup against the page.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct FindElementsAction {
    pub selector: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attributes: Option<Vec<String>>,
    #[serde(default = "default_find_elements_max_results")]
    pub max_results: usize,
    #[serde(default = "default_true")]
    pub include_text: bool,
}

fn default_find_elements_max_results() -> usize {
    50
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum SearchEngine {
    #[default]
    DuckDuckGo,
    Google,
    Bing,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SearchAction {
    pub query: String,
    #[serde(default)]
    pub engine: SearchEngine,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct NavigateAction {
    pub url: String,
    #[serde(default)]
    pub new_tab: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ClickElementAction {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub index: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub coordinate_x: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub coordinate_y: Option<i32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct InputTextAction {
    pub index: u32,
    pub text: String,
    #[serde(default = "default_true")]
    pub clear: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct DoneAction {
    pub text: String,
    #[serde(default = "default_true")]
    pub success: bool,
    #[serde(default)]
    pub files_to_display: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SwitchTabAction {
    pub tab_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CloseTabAction {
    pub tab_id: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ScrollAction {
    #[serde(default = "default_true")]
    pub down: bool,
    #[serde(default = "default_scroll_pages")]
    pub pages: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub index: Option<u32>,
}

fn default_scroll_pages() -> f64 {
    1.0
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SendKeysAction {
    pub keys: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct UploadFileAction {
    pub index: u32,
    pub path: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct NoParamsAction {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ScreenshotAction {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SaveAsPdfAction {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_name: Option<String>,
    #[serde(default = "default_true")]
    pub print_background: bool,
    #[serde(default)]
    pub landscape: bool,
    #[serde(default = "default_pdf_scale")]
    pub scale: f64,
    #[serde(default = "default_paper_format")]
    pub paper_format: String,
}

fn default_pdf_scale() -> f64 {
    1.0
}

fn default_paper_format() -> String {
    "Letter".to_owned()
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct GetDropdownOptionsAction {
    pub index: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SelectDropdownOptionAction {
    pub index: u32,
    pub text: String,
}

/// Browser-use action model: each serialized action is a one-key object.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum BrowserAction {
    Extract(ExtractAction),
    SearchPage(SearchPageAction),
    FindElements(FindElementsAction),
    Search(SearchAction),
    Navigate(NavigateAction),
    Click(ClickElementAction),
    Input(InputTextAction),
    Done(DoneAction),
    SwitchTab(SwitchTabAction),
    CloseTab(CloseTabAction),
    Scroll(ScrollAction),
    SendKeys(SendKeysAction),
    UploadFile(UploadFileAction),
    Screenshot(ScreenshotAction),
    SaveAsPdf(SaveAsPdfAction),
    GetDropdownOptions(GetDropdownOptionsAction),
    SelectDropdownOption(SelectDropdownOptionAction),
}

impl BrowserAction {
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::Extract(_) => "extract",
            Self::SearchPage(_) => "search_page",
            Self::FindElements(_) => "find_elements",
            Self::Search(_) => "search",
            Self::Navigate(_) => "navigate",
            Self::Click(_) => "click",
            Self::Input(_) => "input",
            Self::Done(_) => "done",
            Self::SwitchTab(_) => "switch_tab",
            Self::CloseTab(_) => "close_tab",
            Self::Scroll(_) => "scroll",
            Self::SendKeys(_) => "send_keys",
            Self::UploadFile(_) => "upload_file",
            Self::Screenshot(_) => "screenshot",
            Self::SaveAsPdf(_) => "save_as_pdf",
            Self::GetDropdownOptions(_) => "get_dropdown_options",
            Self::SelectDropdownOption(_) => "select_dropdown_option",
        }
    }

    #[must_use]
    pub fn terminates_sequence(&self) -> bool {
        matches!(
            self,
            Self::Search(_)
                | Self::Navigate(_)
                | Self::SwitchTab(_)
                | Self::CloseTab(_)
                | Self::Done(_)
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn action_serializes_as_one_key_object() {
        let action = BrowserAction::Navigate(NavigateAction {
            url: "https://example.com".to_owned(),
            new_tab: false,
        });

        let value = serde_json::to_value(action).expect("serialize action");

        assert_eq!(
            value,
            serde_json::json!({
                "navigate": {
                    "url": "https://example.com",
                    "new_tab": false
                }
            })
        );
    }

    #[test]
    fn navigation_terminates_multi_action_sequences() {
        let action = BrowserAction::Navigate(NavigateAction {
            url: "https://example.com".to_owned(),
            new_tab: false,
        });

        assert!(action.terminates_sequence());
    }
}
