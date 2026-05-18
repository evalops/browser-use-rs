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
    #[schemars(range(min = 1))]
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
    #[schemars(length(min = 4, max = 4))]
    pub tab_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CloseTabAction {
    #[schemars(length(min = 4, max = 4))]
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
pub struct FindTextAction {
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct EvaluateAction {
    pub code: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct WaitAction {
    #[serde(default = "default_wait_seconds")]
    pub seconds: i64,
}

fn default_wait_seconds() -> i64 {
    3
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
pub struct WriteFileAction {
    pub file_name: String,
    pub content: String,
    #[serde(default)]
    pub append: bool,
    #[serde(default = "default_true")]
    pub trailing_newline: bool,
    #[serde(default)]
    pub leading_newline: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ReadFileAction {
    pub file_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ReplaceFileAction {
    pub file_name: String,
    pub old_str: String,
    pub new_str: String,
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
    GoBack(NoParamsAction),
    Click(ClickElementAction),
    Input(InputTextAction),
    Done(DoneAction),
    SwitchTab(SwitchTabAction),
    CloseTab(CloseTabAction),
    Scroll(ScrollAction),
    FindText(FindTextAction),
    Evaluate(EvaluateAction),
    Wait(WaitAction),
    SendKeys(SendKeysAction),
    UploadFile(UploadFileAction),
    WriteFile(WriteFileAction),
    ReadFile(ReadFileAction),
    ReplaceFile(ReplaceFileAction),
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
            Self::GoBack(_) => "go_back",
            Self::Click(_) => "click",
            Self::Input(_) => "input",
            Self::Done(_) => "done",
            Self::SwitchTab(_) => "switch_tab",
            Self::CloseTab(_) => "close_tab",
            Self::Scroll(_) => "scroll",
            Self::FindText(_) => "find_text",
            Self::Evaluate(_) => "evaluate",
            Self::Wait(_) => "wait",
            Self::SendKeys(_) => "send_keys",
            Self::UploadFile(_) => "upload_file",
            Self::WriteFile(_) => "write_file",
            Self::ReadFile(_) => "read_file",
            Self::ReplaceFile(_) => "replace_file",
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
                | Self::GoBack(_)
                | Self::SwitchTab(_)
                | Self::CloseTab(_)
                | Self::Evaluate(_)
                | Self::Done(_)
        )
    }

    #[must_use]
    pub fn interacted_element_index(&self) -> Option<u32> {
        match self {
            Self::Click(params) => params.index,
            Self::Input(params) => Some(params.index),
            Self::Scroll(params) => params.index,
            Self::UploadFile(params) => Some(params.index),
            Self::GetDropdownOptions(params) => Some(params.index),
            Self::SelectDropdownOption(params) => Some(params.index),
            _ => None,
        }
    }

    pub fn set_interacted_element_index(&mut self, index: u32) -> bool {
        match self {
            Self::Click(params) if params.index.is_some() => {
                params.index = Some(index);
                true
            }
            Self::Input(params) => {
                params.index = index;
                true
            }
            Self::Scroll(params) if params.index.is_some() => {
                params.index = Some(index);
                true
            }
            Self::UploadFile(params) => {
                params.index = index;
                true
            }
            Self::GetDropdownOptions(params) => {
                params.index = index;
                true
            }
            Self::SelectDropdownOption(params) => {
                params.index = index;
                true
            }
            _ => false,
        }
    }

    #[must_use]
    pub fn with_interacted_element_index(&self, index: u32) -> Option<Self> {
        let mut action = self.clone();
        action.set_interacted_element_index(index).then_some(action)
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
    fn action_reports_interacted_element_index_for_indexed_actions() {
        assert_eq!(
            BrowserAction::Click(ClickElementAction {
                index: Some(3),
                coordinate_x: None,
                coordinate_y: None,
            })
            .interacted_element_index(),
            Some(3)
        );
        assert_eq!(
            BrowserAction::Click(ClickElementAction {
                index: None,
                coordinate_x: Some(10),
                coordinate_y: Some(20),
            })
            .interacted_element_index(),
            None
        );
        assert_eq!(
            BrowserAction::Input(InputTextAction {
                index: 4,
                text: "hello".to_owned(),
                clear: true,
            })
            .interacted_element_index(),
            Some(4)
        );
        assert_eq!(
            BrowserAction::Wait(WaitAction { seconds: 1 }).interacted_element_index(),
            None
        );
    }

    #[test]
    fn action_rewrites_interacted_element_index_for_replayable_actions() {
        let actions = vec![
            BrowserAction::Click(ClickElementAction {
                index: Some(3),
                coordinate_x: None,
                coordinate_y: None,
            }),
            BrowserAction::Input(InputTextAction {
                index: 3,
                text: "hello".to_owned(),
                clear: true,
            }),
            BrowserAction::Scroll(ScrollAction {
                down: true,
                pages: 1.0,
                index: Some(3),
            }),
            BrowserAction::UploadFile(UploadFileAction {
                index: 3,
                path: "/tmp/report.pdf".to_owned(),
            }),
            BrowserAction::GetDropdownOptions(GetDropdownOptionsAction { index: 3 }),
            BrowserAction::SelectDropdownOption(SelectDropdownOptionAction {
                index: 3,
                text: "Enterprise".to_owned(),
            }),
        ];

        for action in actions {
            let updated = action
                .with_interacted_element_index(9)
                .expect("indexed action can be rewritten");

            assert_eq!(updated.interacted_element_index(), Some(9));
            assert_eq!(action.interacted_element_index(), Some(3));
        }
    }

    #[test]
    fn action_does_not_rewrite_coordinate_or_non_indexed_actions() {
        assert!(
            BrowserAction::Click(ClickElementAction {
                index: None,
                coordinate_x: Some(10),
                coordinate_y: Some(20),
            })
            .with_interacted_element_index(9)
            .is_none()
        );

        assert!(
            BrowserAction::Scroll(ScrollAction {
                down: true,
                pages: 1.0,
                index: None,
            })
            .with_interacted_element_index(9)
            .is_none()
        );

        assert!(
            BrowserAction::Wait(WaitAction { seconds: 1 })
                .with_interacted_element_index(9)
                .is_none()
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

    #[test]
    fn wait_action_defaults_to_three_seconds() {
        let action: BrowserAction =
            serde_json::from_value(serde_json::json!({ "wait": {} })).expect("wait action");

        assert_eq!(action.name(), "wait");
        assert_eq!(action, BrowserAction::Wait(WaitAction { seconds: 3 }));
        assert!(!action.terminates_sequence());
    }

    #[test]
    fn go_back_action_uses_no_params_and_terminates() {
        let action: BrowserAction =
            serde_json::from_value(serde_json::json!({ "go_back": {} })).expect("go_back action");

        assert_eq!(action.name(), "go_back");
        assert_eq!(
            action,
            BrowserAction::GoBack(NoParamsAction { description: None })
        );
        assert!(action.terminates_sequence());
    }

    #[test]
    fn find_text_action_uses_text_param() {
        let action: BrowserAction =
            serde_json::from_value(serde_json::json!({ "find_text": { "text": "Needle" } }))
                .expect("find_text action");

        assert_eq!(action.name(), "find_text");
        assert_eq!(
            action,
            BrowserAction::FindText(FindTextAction {
                text: "Needle".to_owned()
            })
        );
        assert!(!action.terminates_sequence());
    }

    #[test]
    fn evaluate_action_uses_code_param_and_terminates() {
        let action: BrowserAction =
            serde_json::from_value(serde_json::json!({ "evaluate": { "code": "document.title" } }))
                .expect("evaluate action");

        assert_eq!(action.name(), "evaluate");
        assert_eq!(
            action,
            BrowserAction::Evaluate(EvaluateAction {
                code: "document.title".to_owned()
            })
        );
        assert!(action.terminates_sequence());
    }

    #[test]
    fn write_file_action_defaults_to_overwrite_with_trailing_newline() {
        let action: BrowserAction = serde_json::from_value(serde_json::json!({
            "write_file": {
                "file_name": "notes.md",
                "content": "hello"
            }
        }))
        .expect("write_file action");

        assert_eq!(action.name(), "write_file");
        assert_eq!(
            action,
            BrowserAction::WriteFile(WriteFileAction {
                file_name: "notes.md".to_owned(),
                content: "hello".to_owned(),
                append: false,
                trailing_newline: true,
                leading_newline: false,
            })
        );
        assert!(!action.terminates_sequence());
    }
}
