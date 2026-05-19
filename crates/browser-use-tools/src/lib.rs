//! Built-in browser action schemas and registry contracts.
//!
//! This crate is intentionally data-heavy: each public struct below is a
//! serializable Rust representation of one action the language model may ask
//! the browser executor to perform. The `serde` derives define the JSON shape
//! on the wire, while the `schemars` derives produce the JSON Schema that is
//! sent to schema-capable model providers.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Free-text or schema-guided page extraction.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ExtractAction {
    /// Natural-language instruction describing what should be extracted.
    pub query: String,
    /// Whether extracted records should include links discovered on the page.
    #[serde(default)]
    pub extract_links: bool,
    /// Whether extracted records should include image references from the page.
    #[serde(default)]
    pub extract_images: bool,
    /// Character offset used by long-page extraction to continue from earlier work.
    #[serde(default)]
    pub start_from_char: usize,
    /// Optional schema describing the structured extraction result expected from the model.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_schema: Option<Value>,
    /// Items already collected in a previous extraction pass.
    #[serde(default)]
    pub already_collected: Vec<String>,
}

/// Text or regex search against page content.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SearchPageAction {
    /// Literal text or regular expression to find in the current page.
    pub pattern: String,
    /// Treats `pattern` as a regular expression when true.
    #[serde(default)]
    pub regex: bool,
    /// Makes matching case-sensitive when true.
    #[serde(default)]
    pub case_sensitive: bool,
    /// Number of neighboring characters to include around each match.
    #[serde(default = "default_context_chars")]
    pub context_chars: usize,
    /// Optional CSS selector limiting the search to a sub-tree.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub css_scope: Option<String>,
    /// Upper bound on matches returned to the model.
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
    /// CSS selector used to find matching elements in the active page.
    pub selector: String,
    /// Optional attribute names to include for each result.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attributes: Option<Vec<String>>,
    /// Upper bound on elements returned to the model.
    #[serde(default = "default_find_elements_max_results")]
    pub max_results: usize,
    /// Whether element text content should be included in results.
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
/// Search provider used by [`SearchAction`].
pub enum SearchEngine {
    /// Use DuckDuckGo, the default provider because it works without API keys.
    #[default]
    DuckDuckGo,
    /// Use Google search.
    Google,
    /// Use Bing search.
    Bing,
}

/// Web-search action that opens a result page for the requested query.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SearchAction {
    /// Search terms supplied by the model.
    pub query: String,
    /// Search engine used to construct the navigation URL.
    #[serde(default)]
    pub engine: SearchEngine,
}

/// Navigation action for loading a URL in the current tab or a new tab.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct NavigateAction {
    /// Absolute or user-entered URL to open.
    pub url: String,
    /// Opens the URL in a fresh tab when true.
    #[serde(default)]
    pub new_tab: bool,
}

/// Click action against an indexed DOM element or an explicit coordinate.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ClickElementAction {
    /// One-based browser-use element index from the current DOM summary.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(range(min = 1))]
    pub index: Option<u32>,
    /// Optional viewport x coordinate for coordinate-based clicks.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub coordinate_x: Option<i32>,
    /// Optional viewport y coordinate for coordinate-based clicks.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub coordinate_y: Option<i32>,
}

/// Text-entry action for an indexed input-like element.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct InputTextAction {
    /// One-based browser-use element index to focus before typing.
    pub index: u32,
    /// Text to type into the focused element.
    pub text: String,
    /// Clears existing value before typing when true.
    #[serde(default = "default_true")]
    pub clear: bool,
}

/// Terminal response action returned when the agent believes the task is done.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct DoneAction {
    /// Final text shown to the caller.
    pub text: String,
    /// Whether the model believes the task completed successfully.
    #[serde(default = "default_true")]
    pub success: bool,
    /// Managed files whose contents should be included with the final answer.
    #[serde(default)]
    pub files_to_display: Vec<String>,
}

/// Switches the active browser target to an existing tab.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SwitchTabAction {
    /// Four-character tab identifier shown in browser state.
    #[schemars(length(min = 4, max = 4))]
    pub tab_id: String,
}

/// Closes an existing browser tab.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CloseTabAction {
    /// Four-character tab identifier shown in browser state.
    #[schemars(length(min = 4, max = 4))]
    pub tab_id: String,
}

/// Scrolls the page or a scrollable indexed element.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ScrollAction {
    /// Scrolls downward when true and upward when false.
    #[serde(default = "default_true")]
    pub down: bool,
    /// Number of viewport pages, or equivalent element pages, to scroll.
    #[serde(default = "default_scroll_pages")]
    pub pages: f64,
    /// Optional one-based element index for element-scoped scrolling.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub index: Option<u32>,
}

fn default_scroll_pages() -> f64 {
    1.0
}

/// Finds text on the current page using browser-native find behavior.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct FindTextAction {
    /// Text to locate in the current page.
    pub text: String,
}

/// Executes JavaScript in the active page.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct EvaluateAction {
    /// JavaScript source code to evaluate.
    pub code: String,
}

/// Sleeps for a bounded amount of time so page work can settle.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct WaitAction {
    /// Number of seconds to wait.
    #[serde(default = "default_wait_seconds")]
    pub seconds: i64,
}

fn default_wait_seconds() -> i64 {
    3
}

/// Sends keyboard shortcuts or raw key sequences to the active page.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SendKeysAction {
    /// Key sequence, for example `Enter`, `Tab`, or provider-specific chords.
    pub keys: String,
}

/// Uploads a local file through an indexed file input element.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct UploadFileAction {
    /// One-based browser-use element index for the file input.
    pub index: u32,
    /// Local path to upload.
    pub path: String,
}

/// Writes text content into the managed file sandbox.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct WriteFileAction {
    /// Managed file name or relative path.
    pub file_name: String,
    /// Text content to write.
    pub content: String,
    /// Appends to an existing file instead of replacing it when true.
    #[serde(default)]
    pub append: bool,
    /// Adds a newline after `content` when true.
    #[serde(default = "default_true")]
    pub trailing_newline: bool,
    /// Adds a newline before `content` when true.
    #[serde(default)]
    pub leading_newline: bool,
}

/// Reads text content from the managed file sandbox.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ReadFileAction {
    /// Managed file name or relative path.
    pub file_name: String,
}

/// Replaces text inside a managed file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ReplaceFileAction {
    /// Managed file name or relative path.
    pub file_name: String,
    /// Existing text that must be present.
    pub old_str: String,
    /// Replacement text.
    pub new_str: String,
}

/// Placeholder payload for actions that do not require parameters.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct NoParamsAction {
    /// Optional human-readable note retained for compatibility with flexible callers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// Captures a screenshot of the active page.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ScreenshotAction {
    /// Optional managed output file name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_name: Option<String>,
}

/// Saves the active page as a PDF.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SaveAsPdfAction {
    /// Optional managed output file name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_name: Option<String>,
    /// Includes CSS backgrounds in the generated PDF when true.
    #[serde(default = "default_true")]
    pub print_background: bool,
    /// Uses landscape orientation when true.
    #[serde(default)]
    pub landscape: bool,
    /// Browser print scale, where `1.0` is the default page scale.
    #[serde(default = "default_pdf_scale")]
    pub scale: f64,
    /// Paper format understood by Chrome print-to-PDF, for example `Letter`.
    #[serde(default = "default_paper_format")]
    pub paper_format: String,
}

fn default_pdf_scale() -> f64 {
    1.0
}

fn default_paper_format() -> String {
    "Letter".to_owned()
}

/// Lists options for an indexed `<select>` element.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct GetDropdownOptionsAction {
    /// One-based browser-use element index for the dropdown.
    pub index: u32,
}

/// Selects an option in an indexed dropdown.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SelectDropdownOptionAction {
    /// One-based browser-use element index for the dropdown.
    pub index: u32,
    /// Visible option text to select.
    pub text: String,
}

/// Browser-use action model: each serialized action is a one-key object.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum BrowserAction {
    /// Ask the model to extract structured or unstructured information from the page.
    Extract(ExtractAction),
    /// Search within the current page text or DOM.
    SearchPage(SearchPageAction),
    /// Find elements by CSS selector.
    FindElements(FindElementsAction),
    /// Run a web search.
    Search(SearchAction),
    /// Navigate to a URL.
    Navigate(NavigateAction),
    /// Navigate the active tab backward in history.
    GoBack(NoParamsAction),
    /// Click an element or coordinate.
    Click(ClickElementAction),
    /// Type text into an element.
    Input(InputTextAction),
    /// End the task with a final answer.
    Done(DoneAction),
    /// Switch to another tab.
    SwitchTab(SwitchTabAction),
    /// Close a tab.
    CloseTab(CloseTabAction),
    /// Scroll the page or a scrollable element.
    Scroll(ScrollAction),
    /// Use browser find for text.
    FindText(FindTextAction),
    /// Evaluate JavaScript.
    Evaluate(EvaluateAction),
    /// Wait for page activity to settle.
    Wait(WaitAction),
    /// Send keyboard input.
    SendKeys(SendKeysAction),
    /// Upload a file.
    UploadFile(UploadFileAction),
    /// Write a managed file.
    WriteFile(WriteFileAction),
    /// Read a managed file.
    ReadFile(ReadFileAction),
    /// Replace text in a managed file.
    ReplaceFile(ReplaceFileAction),
    /// Capture a screenshot.
    Screenshot(ScreenshotAction),
    /// Save the page as a PDF.
    SaveAsPdf(SaveAsPdfAction),
    /// Inspect options for a dropdown.
    GetDropdownOptions(GetDropdownOptionsAction),
    /// Select a dropdown option.
    SelectDropdownOption(SelectDropdownOptionAction),
}

impl BrowserAction {
    /// Returns the stable snake_case action name used in prompts, JSON, and logs.
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

    /// Returns true when this action should stop an action batch after it runs.
    ///
    /// Navigation-like actions can change the page, tab, or task state so much
    /// that following actions would be based on stale DOM indexes. The executor
    /// uses this helper to preserve browser-use's step-by-step semantics.
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

    /// Returns the DOM element index targeted by actions that use one.
    ///
    /// Coordinate clicks intentionally return `None`; they are not tied to a
    /// DOM element and therefore cannot be remapped during history replay.
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

    /// Updates the targeted DOM index when the action type supports remapping.
    ///
    /// History replay uses this after matching a previously interacted element
    /// against the current DOM. The boolean return value tells callers whether
    /// the action actually had an index slot to update.
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

    /// Clones the action with a remapped DOM index when remapping is supported.
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
