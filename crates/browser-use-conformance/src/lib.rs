//! Golden fixtures and parity utilities for browser-use-rs.

use browser_use_dom::{DomElementRef, SerializedDomState};
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
            is_visible: true,
            is_interactive: true,
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
            is_visible: true,
            is_interactive: true,
        },
    ])
}

#[cfg(test)]
mod tests {
    use super::*;
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
    fn action_schema_exposes_browser_use_action_names() {
        let schema = serde_json::to_value(schema_for!(BrowserAction)).expect("serialize schema");
        let schema_text = serde_json::to_string(&schema).expect("schema text");

        for action in ["navigate", "click", "input", "scroll", "screenshot", "done"] {
            assert!(schema_text.contains(action), "schema missing {action}");
        }
    }
}
