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
    use browser_use_core::{ActionExecutor, ActionResult, execute_action_sequence};
    use browser_use_tools::BrowserAction;
    use schemars::schema_for;
    use serde_json::Value;

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
            "screenshot",
            "send_keys",
            "upload_file",
            "save_as_pdf",
            "extract",
            "search_page",
            "find_elements",
            "switch_tab",
            "close_tab",
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
}
