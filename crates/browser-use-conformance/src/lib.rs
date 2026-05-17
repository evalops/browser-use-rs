//! Golden fixtures and parity utilities for browser-use-rs.

use browser_use_dom::{DomElementRef, ElementBounds, SerializedDomState};
use std::collections::BTreeMap;

#[must_use]
pub fn simple_interactive_state() -> SerializedDomState {
    SerializedDomState::from_elements(vec![
        DomElementRef {
            index: 1,
            target_id: "fixture-target".to_owned(),
            backend_node_id: 0,
            node_id: None,
            tag_name: "button".to_owned(),
            role: None,
            name: Some("Run".to_owned()),
            text: Some("Run".to_owned()),
            attributes: BTreeMap::new(),
            bounds: None,
            is_visible: true,
            is_interactive: true,
            is_scrollable: false,
        },
        DomElementRef {
            index: 2,
            target_id: "fixture-target".to_owned(),
            backend_node_id: 0,
            node_id: None,
            tag_name: "input".to_owned(),
            role: None,
            name: Some("Name".to_owned()),
            text: Some("EvalOps".to_owned()),
            attributes: BTreeMap::new(),
            bounds: None,
            is_visible: true,
            is_interactive: true,
            is_scrollable: false,
        },
    ])
}

#[must_use]
pub fn mixed_interactive_state() -> SerializedDomState {
    SerializedDomState::from_elements(vec![
        DomElementRef {
            index: 1,
            target_id: "fixture-target".to_owned(),
            backend_node_id: 101,
            node_id: Some(201),
            tag_name: "button".to_owned(),
            role: Some("button".to_owned()),
            name: Some("Submit request".to_owned()),
            text: Some("Ignored visual label".to_owned()),
            attributes: BTreeMap::from([
                ("aria-labelledby".to_owned(), "submit-name".to_owned()),
                ("data-testid".to_owned(), "submit-request".to_owned()),
                ("id".to_owned(), "submit".to_owned()),
            ]),
            bounds: Some(ElementBounds {
                x: 12,
                y: 18,
                width: 140,
                height: 32,
            }),
            is_visible: true,
            is_interactive: true,
            is_scrollable: false,
        },
        DomElementRef {
            index: 2,
            target_id: "fixture-target".to_owned(),
            backend_node_id: 102,
            node_id: Some(202),
            tag_name: "input".to_owned(),
            role: None,
            name: Some("Email address".to_owned()),
            text: Some("user@example.com".to_owned()),
            attributes: BTreeMap::from([
                ("aria-required".to_owned(), "true".to_owned()),
                ("id".to_owned(), "email".to_owned()),
                ("placeholder".to_owned(), "name@example.com".to_owned()),
                ("type".to_owned(), "email".to_owned()),
            ]),
            bounds: Some(ElementBounds {
                x: 12,
                y: 64,
                width: 260,
                height: 36,
            }),
            is_visible: true,
            is_interactive: true,
            is_scrollable: false,
        },
        DomElementRef {
            index: 3,
            target_id: "fixture-target".to_owned(),
            backend_node_id: 103,
            node_id: Some(203),
            tag_name: "select".to_owned(),
            role: None,
            name: Some("Plan".to_owned()),
            text: Some("Enterprise".to_owned()),
            attributes: BTreeMap::from([("name".to_owned(), "plan".to_owned())]),
            bounds: Some(ElementBounds {
                x: 12,
                y: 112,
                width: 180,
                height: 34,
            }),
            is_visible: true,
            is_interactive: true,
            is_scrollable: false,
        },
        DomElementRef {
            index: 4,
            target_id: "fixture-target".to_owned(),
            backend_node_id: 104,
            node_id: Some(204),
            tag_name: "div".to_owned(),
            role: None,
            name: Some("Results panel".to_owned()),
            text: None,
            attributes: BTreeMap::from([
                ("id".to_owned(), "results".to_owned()),
                ("tabindex".to_owned(), "0".to_owned()),
            ]),
            bounds: Some(ElementBounds {
                x: 12,
                y: 160,
                width: 320,
                height: 120,
            }),
            is_visible: true,
            is_interactive: true,
            is_scrollable: true,
        },
    ])
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use browser_use_cdp::{BrowserError, BrowserSession, FoundElement, Pdf, Screenshot};
    use browser_use_core::{
        ActionExecutor, ActionResult, Agent, AgentSettings, ChatCompletion, ChatModel, ChatRequest,
        LlmError, execute_action_sequence,
    };
    use browser_use_dom::BrowserStateSummary;
    use browser_use_tools::BrowserAction;
    use schemars::schema_for;
    use serde_json::{Value, json};
    use std::{
        collections::VecDeque,
        path::Path,
        sync::{Arc, Mutex},
    };

    #[test]
    fn simple_interactive_state_matches_golden_fixture() {
        let expected: Value =
            serde_json::from_str(include_str!("../fixtures/simple_interactive_state.json"))
                .expect("golden fixture");
        let actual = serde_json::to_value(simple_interactive_state()).expect("serialize state");

        assert_eq!(actual, expected);
    }

    #[test]
    fn mixed_interactive_state_matches_golden_fixture() {
        let expected: Value =
            serde_json::from_str(include_str!("../fixtures/mixed_interactive_state.json"))
                .expect("golden fixture");
        let actual = serde_json::to_value(mixed_interactive_state()).expect("serialize state");

        assert_eq!(actual, expected);
    }

    #[test]
    fn action_schema_exposes_browser_use_action_names() {
        let schema = serde_json::to_value(schema_for!(BrowserAction)).expect("serialize schema");
        let schema_text = serde_json::to_string(&schema).expect("schema text");

        for action in [
            "navigate",
            "search",
            "click",
            "input",
            "scroll",
            "find_text",
            "evaluate",
            "wait",
            "screenshot",
            "send_keys",
            "upload_file",
            "write_file",
            "read_file",
            "replace_file",
            "save_as_pdf",
            "extract",
            "search_page",
            "find_elements",
            "switch_tab",
            "close_tab",
            "go_back",
            "get_dropdown_options",
            "select_dropdown_option",
            "done",
        ] {
            assert!(schema_text.contains(action), "schema missing {action}");
        }
    }

    struct FixtureExecutor;

    #[async_trait]
    impl ActionExecutor for FixtureExecutor {
        async fn execute(&mut self, action: &BrowserAction) -> ActionResult {
            match action {
                BrowserAction::Click(params) => {
                    ActionResult::extracted(format!("Clicked element {}", params.index.unwrap()))
                }
                BrowserAction::Input(params) => {
                    ActionResult::extracted(format!("Typed text into element {}", params.index))
                }
                other => {
                    ActionResult::error(format!("unexpected fixture action: {}", other.name()))
                }
            }
        }
    }

    #[tokio::test]
    async fn simple_action_sequence_matches_golden_results() {
        let actions: Vec<BrowserAction> =
            serde_json::from_str(include_str!("../fixtures/simple_action_sequence.json"))
                .expect("action fixture");
        let expected: Value =
            serde_json::from_str(include_str!("../fixtures/simple_action_results.json"))
                .expect("result fixture");
        let mut executor = FixtureExecutor;

        let results = execute_action_sequence(&mut executor, &actions).await;
        let actual = serde_json::to_value(results).expect("serialize results");

        assert_eq!(actual, expected);
    }

    #[tokio::test]
    async fn simple_agent_run_matches_golden_history_fixture() {
        let clicked = Arc::new(Mutex::new(Vec::new()));
        let (llm, requests) = ScriptedChatModel::new(vec![
            json!({
                "current_state": {
                    "thinking": "Click the Run button",
                    "evaluation_previous_goal": "No previous goal",
                    "memory": "Need to trigger the form",
                    "next_goal": "Click Run"
                },
                "action": [
                    {
                        "click": {
                            "index": 1
                        }
                    }
                ]
            }),
            json!({
                "current_state": {
                    "thinking": "The requested click succeeded",
                    "evaluation_previous_goal": "Clicked element 1",
                    "memory": "Run was clicked",
                    "next_goal": "Finish"
                },
                "action": [
                    {
                        "done": {
                            "text": "Clicked Run",
                            "success": true
                        }
                    }
                ]
            }),
        ]);
        let session = FixtureSession {
            state: fixture_browser_state(),
            clicked: Arc::clone(&clicked),
        };
        let settings = AgentSettings {
            max_actions_per_step: 1,
            ..AgentSettings::default()
        };
        let mut agent = Agent::with_settings("Click the Run button", settings, llm, session);

        let mut actual = {
            let history = agent.run(2).await.expect("agent run");
            assert!(history.is_done());
            assert_eq!(history.final_result(), Some("Clicked Run"));
            assert_eq!(history.is_successful(), Some(true));
            serde_json::to_value(history).expect("serialize history")
        };

        assert_eq!(*clicked.lock().expect("clicked lock"), vec![1]);
        assert_agent_step_metadata(&actual);
        strip_agent_step_metadata(&mut actual);
        let expected: Value =
            serde_json::from_str(include_str!("../fixtures/simple_agent_history.json"))
                .expect("agent history fixture");
        assert_eq!(actual, expected);

        let requests = requests.lock().expect("requests lock");
        assert_eq!(requests.len(), 2);
        assert!(requests[0].output_schema.is_some());
        let request_text = serde_json::to_string(&requests[1]).expect("request text");
        assert!(request_text.contains("Previous action results"));
        assert!(request_text.contains("Clicked element 1"));
    }

    fn assert_agent_step_metadata(history: &Value) {
        let items = history["items"].as_array().expect("history items");
        assert_eq!(items.len(), 2);

        let first = &items[0]["metadata"];
        assert_eq!(first["step_number"], 1);
        assert!(first["step_start_time"].as_f64().expect("first start") > 0.0);
        assert!(
            first["step_end_time"].as_f64().expect("first end")
                >= first["step_start_time"].as_f64().unwrap()
        );
        assert!(first["step_interval"].is_null());

        let second = &items[1]["metadata"];
        assert_eq!(second["step_number"], 2);
        assert!(second["step_start_time"].as_f64().expect("second start") > 0.0);
        assert!(
            second["step_end_time"].as_f64().expect("second end")
                >= second["step_start_time"].as_f64().unwrap()
        );
        assert!(second["step_interval"].as_f64().expect("second interval") >= 0.0);
    }

    fn strip_agent_step_metadata(history: &mut Value) {
        let items = history["items"].as_array_mut().expect("history items");
        for item in items {
            item.as_object_mut()
                .expect("history item object")
                .remove("metadata");
        }
    }

    fn fixture_browser_state() -> BrowserStateSummary {
        BrowserStateSummary {
            dom_state: simple_interactive_state(),
            url: "https://example.test/form".to_owned(),
            title: "Fixture".to_owned(),
            tabs: Vec::new(),
            screenshot: None,
            page_info: None,
            pixels_above: 0,
            pixels_below: 0,
            browser_errors: Vec::new(),
            is_pdf_viewer: false,
            recent_events: None,
            pending_network_requests: Vec::new(),
            pagination_buttons: Vec::new(),
            closed_popup_messages: Vec::new(),
        }
    }

    #[derive(Clone)]
    struct ScriptedChatModel {
        outputs: Arc<Mutex<VecDeque<Value>>>,
        requests: Arc<Mutex<Vec<ChatRequest>>>,
    }

    impl ScriptedChatModel {
        fn new(outputs: Vec<Value>) -> (Self, Arc<Mutex<Vec<ChatRequest>>>) {
            let requests = Arc::new(Mutex::new(Vec::new()));
            (
                Self {
                    outputs: Arc::new(Mutex::new(outputs.into())),
                    requests: Arc::clone(&requests),
                },
                requests,
            )
        }
    }

    #[async_trait]
    impl ChatModel for ScriptedChatModel {
        fn provider(&self) -> &str {
            "fixture"
        }

        fn model(&self) -> &str {
            "fixture-script"
        }

        async fn invoke_json(
            &self,
            request: ChatRequest,
        ) -> Result<ChatCompletion<Value>, LlmError> {
            self.requests.lock().expect("requests lock").push(request);
            let content = self
                .outputs
                .lock()
                .expect("outputs lock")
                .pop_front()
                .ok_or_else(|| LlmError::Provider("script exhausted".to_owned()))?;
            Ok(ChatCompletion {
                model: self.model().to_owned(),
                content,
                raw_response: None,
            })
        }
    }

    #[derive(Clone)]
    struct FixtureSession {
        state: BrowserStateSummary,
        clicked: Arc<Mutex<Vec<u32>>>,
    }

    #[async_trait]
    impl BrowserSession for FixtureSession {
        async fn state(
            &self,
            _include_screenshot: bool,
        ) -> Result<BrowserStateSummary, BrowserError> {
            Ok(self.state.clone())
        }

        async fn navigate(&self, _url: &str, _new_tab: bool) -> Result<(), BrowserError> {
            Err(unsupported_action("navigate"))
        }

        async fn go_back(&self) -> Result<(), BrowserError> {
            Err(unsupported_action("go_back"))
        }

        async fn switch_tab(&self, _target_id: &str) -> Result<(), BrowserError> {
            Err(unsupported_action("switch_tab"))
        }

        async fn close_tab(&self, _target_id: &str) -> Result<(), BrowserError> {
            Err(unsupported_action("close_tab"))
        }

        async fn click(&self, index: u32) -> Result<(), BrowserError> {
            self.clicked.lock().expect("clicked lock").push(index);
            Ok(())
        }

        async fn click_coordinates(&self, _x: i32, _y: i32) -> Result<(), BrowserError> {
            Err(unsupported_action("click_coordinates"))
        }

        async fn input_text(
            &self,
            _index: u32,
            _text: &str,
            _clear: bool,
        ) -> Result<(), BrowserError> {
            Err(unsupported_action("input_text"))
        }

        async fn scroll(
            &self,
            _index: Option<u32>,
            _down: bool,
            _pages: f64,
        ) -> Result<(), BrowserError> {
            Err(unsupported_action("scroll"))
        }

        async fn find_text(&self, _text: &str) -> Result<bool, BrowserError> {
            Err(unsupported_action("find_text"))
        }

        async fn evaluate(&self, _code: &str) -> Result<String, BrowserError> {
            Err(unsupported_action("evaluate"))
        }

        async fn dropdown_options(&self, _index: u32) -> Result<Vec<String>, BrowserError> {
            Err(unsupported_action("dropdown_options"))
        }

        async fn select_dropdown_option(
            &self,
            _index: u32,
            _text: &str,
        ) -> Result<(), BrowserError> {
            Err(unsupported_action("select_dropdown_option"))
        }

        async fn page_text(&self) -> Result<String, BrowserError> {
            Err(unsupported_action("page_text"))
        }

        async fn find_elements(
            &self,
            _selector: &str,
            _attributes: &[String],
            _max_results: usize,
            _include_text: bool,
        ) -> Result<Vec<FoundElement>, BrowserError> {
            Err(unsupported_action("find_elements"))
        }

        async fn send_keys(&self, _keys: &str) -> Result<(), BrowserError> {
            Err(unsupported_action("send_keys"))
        }

        async fn upload_file(&self, _index: u32, _path: &Path) -> Result<(), BrowserError> {
            Err(unsupported_action("upload_file"))
        }

        async fn screenshot(&self) -> Result<Screenshot, BrowserError> {
            Err(unsupported_action("screenshot"))
        }

        async fn save_pdf(
            &self,
            _print_background: bool,
            _landscape: bool,
            _scale: f64,
            _paper_format: &str,
        ) -> Result<Pdf, BrowserError> {
            Err(unsupported_action("save_pdf"))
        }
    }

    fn unsupported_action(action: &str) -> BrowserError {
        BrowserError::ActionFailed(format!("fixture session does not support {action}"))
    }
}
