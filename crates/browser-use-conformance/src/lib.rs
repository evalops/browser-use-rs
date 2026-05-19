//! Golden fixtures and parity utilities for browser-use-rs.

use browser_use_dom::{
    BrowserStateSummary, DomElementRef, DomEvalNode, DomPageStats, ElementBounds, NetworkRequest,
    PageInfo, PaginationButton, PaginationButtonType, SerializedDomState, TabInfo,
};
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
                ("aria-busy".to_owned(), "true".to_owned()),
                ("aria-keyshortcuts".to_owned(), "Control+Enter".to_owned()),
                ("aria-labelledby".to_owned(), "submit-name".to_owned()),
                ("aria-live".to_owned(), "polite".to_owned()),
                (
                    "ax_description".to_owned(),
                    "Submits the completed request".to_owned(),
                ),
                ("ax_name".to_owned(), "Submit request".to_owned()),
                ("data-testid".to_owned(), "submit-request".to_owned()),
                (
                    "description".to_owned(),
                    "Sends the completed form".to_owned(),
                ),
                ("id".to_owned(), "submit".to_owned()),
                ("role".to_owned(), "button".to_owned()),
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
                ("readonly".to_owned(), "true".to_owned()),
                ("id".to_owned(), "email".to_owned()),
                ("placeholder".to_owned(), "name@example.com".to_owned()),
                ("type".to_owned(), "email".to_owned()),
                ("value".to_owned(), "user@example.com".to_owned()),
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
            attributes: BTreeMap::from([
                ("aria-multiselectable".to_owned(), "true".to_owned()),
                (
                    "compound_components".to_owned(),
                    "(name=Dropdown Toggle,role=button),(name=Options,role=listbox,count=3,options=Starter|Enterprise|Internal)"
                        .to_owned(),
                ),
                ("name".to_owned(), "plan".to_owned()),
            ]),
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
        DomElementRef {
            index: 5,
            target_id: "fixture-target".to_owned(),
            backend_node_id: 105,
            node_id: Some(205),
            tag_name: "div".to_owned(),
            role: None,
            name: Some("Draft note".to_owned()),
            text: Some("Draft body".to_owned()),
            attributes: BTreeMap::from([
                ("contenteditable".to_owned(), "plaintext-only".to_owned()),
                ("editable".to_owned(), "true".to_owned()),
                ("focusable".to_owned(), "true".to_owned()),
                ("id".to_owned(), "notes".to_owned()),
                ("settable".to_owned(), "true".to_owned()),
            ]),
            bounds: Some(ElementBounds {
                x: 12,
                y: 300,
                width: 300,
                height: 80,
            }),
            is_visible: true,
            is_interactive: true,
            is_scrollable: false,
        },
        DomElementRef {
            index: 6,
            target_id: "fixture-target".to_owned(),
            backend_node_id: 106,
            node_id: Some(206),
            tag_name: "audio".to_owned(),
            role: None,
            name: Some("Audio sample".to_owned()),
            text: None,
            attributes: BTreeMap::from([
                (
                    "compound_components".to_owned(),
                    "(name=Play/Pause,role=button),(name=Progress,role=slider,min=0,max=100),(name=Mute,role=button),(name=Volume,role=slider,min=0,max=100)"
                        .to_owned(),
                ),
                ("id".to_owned(), "audio-player".to_owned()),
            ]),
            bounds: Some(ElementBounds {
                x: 12,
                y: 400,
                width: 320,
                height: 54,
            }),
            is_visible: true,
            is_interactive: true,
            is_scrollable: false,
        },
    ])
}

#[must_use]
pub fn frame_shadow_state() -> SerializedDomState {
    SerializedDomState::from_elements(vec![
        DomElementRef {
            index: 1,
            target_id: "fixture-root".to_owned(),
            backend_node_id: 301,
            node_id: Some(401),
            tag_name: "iframe".to_owned(),
            role: None,
            name: Some("Checkout frame".to_owned()),
            text: None,
            attributes: BTreeMap::from([
                ("id".to_owned(), "checkout-frame".to_owned()),
                (
                    "src".to_owned(),
                    "https://child.example/checkout".to_owned(),
                ),
            ]),
            bounds: Some(ElementBounds {
                x: 24,
                y: 120,
                width: 640,
                height: 420,
            }),
            is_visible: true,
            is_interactive: true,
            is_scrollable: false,
        },
        DomElementRef {
            index: 2,
            target_id: "fixture-child".to_owned(),
            backend_node_id: 302,
            node_id: Some(402),
            tag_name: "input".to_owned(),
            role: None,
            name: Some("Child email".to_owned()),
            text: Some("agent@example.com".to_owned()),
            attributes: BTreeMap::from([
                ("id".to_owned(), "child-email".to_owned()),
                ("type".to_owned(), "email".to_owned()),
            ]),
            bounds: Some(ElementBounds {
                x: 48,
                y: 188,
                width: 280,
                height: 34,
            }),
            is_visible: true,
            is_interactive: true,
            is_scrollable: false,
        },
        DomElementRef {
            index: 3,
            target_id: "fixture-root".to_owned(),
            backend_node_id: 303,
            node_id: Some(403),
            tag_name: "button".to_owned(),
            role: None,
            name: Some("Shadow save".to_owned()),
            text: None,
            attributes: BTreeMap::from([("id".to_owned(), "shadow-save".to_owned())]),
            bounds: Some(ElementBounds {
                x: 700,
                y: 160,
                width: 112,
                height: 36,
            }),
            is_visible: true,
            is_interactive: true,
            is_scrollable: false,
        },
    ])
}

#[must_use]
pub fn eval_tree_state() -> SerializedDomState {
    let root = DomEvalNode::element("html").with_children(vec![
        DomEvalNode::element("body").with_children(vec![
            DomEvalNode::element("main").with_children(vec![
                DomEvalNode::element("h1").with_children(vec![DomEvalNode::text("Checkout")]),
                DomEvalNode::element("form").with_children(vec![
                    DomEvalNode::element("label")
                        .with_attribute("for", "email")
                        .with_children(vec![DomEvalNode::text("Email")]),
                    DomEvalNode::element("input")
                        .with_attribute("id", "email")
                        .with_attribute("name", "email")
                        .with_attribute("type", "email")
                        .with_attribute("placeholder", "agent@example.com")
                        .with_attribute("required", "true")
                        .interactive(901),
                    DomEvalNode::element("button")
                        .with_attribute("ax_description", "Completes checkout")
                        .with_attribute("ax_name", "Pay now")
                        .with_attribute("data-testid", "checkout-submit")
                        .with_children(vec![DomEvalNode::text("Pay now")])
                        .interactive(902),
                ]),
                DomEvalNode::document_fragment(vec![
                    DomEvalNode::element("button")
                        .with_attribute("id", "shadow-help")
                        .with_children(vec![DomEvalNode::text("Help")])
                        .interactive(903),
                ]),
                DomEvalNode::element("iframe")
                    .with_attribute("title", "Receipt")
                    .with_children(vec![
                        DomEvalNode::element("a")
                            .with_attribute("href", "/receipt")
                            .with_children(vec![DomEvalNode::text("Receipt link")])
                            .interactive(904),
                    ]),
            ]),
        ]),
    ]);

    SerializedDomState::default().with_eval_root(root)
}

#[must_use]
pub fn rich_browser_state_summary() -> BrowserStateSummary {
    let dom_state = simple_interactive_state().with_page_stats(DomPageStats {
        links: 2,
        iframes: 1,
        shadow_open: 1,
        shadow_closed: 0,
        scroll_containers: 1,
        images: 3,
        interactive_elements: 2,
        total_elements: 42,
        text_chars: 512,
    });

    BrowserStateSummary {
        dom_state,
        url: "https://example.test/dashboard?page=2".to_owned(),
        title: "Fixture Dashboard".to_owned(),
        tabs: vec![
            TabInfo {
                url: "https://example.test/dashboard?page=2".to_owned(),
                title: "Fixture Dashboard".to_owned(),
                tab_id: TabInfo::tab_id_for_target("target-main-abcd"),
                target_id: "target-main-abcd".to_owned(),
                parent_target_id: None,
            },
            TabInfo {
                url: "https://child.example/frame".to_owned(),
                title: "Child Frame".to_owned(),
                tab_id: TabInfo::tab_id_for_target("target-child-efgh"),
                target_id: "target-child-efgh".to_owned(),
                parent_target_id: Some("target-main-abcd".to_owned()),
            },
        ],
        screenshot: Some("fixture-base64-png".to_owned()),
        page_info: Some(PageInfo {
            viewport_width: 1280,
            viewport_height: 720,
            page_width: 1280,
            page_height: 1800,
            scroll_x: 0,
            scroll_y: 240,
            pixels_above: 240,
            pixels_below: 840,
            pixels_left: 0,
            pixels_right: 0,
        }),
        pixels_above: 240,
        pixels_below: 840,
        browser_errors: vec!["console error: fixture warning".to_owned()],
        is_pdf_viewer: false,
        recent_events: Some("Clicked element 1".to_owned()),
        pending_network_requests: vec![NetworkRequest {
            url: "https://api.example.test/data".to_owned(),
            method: "POST".to_owned(),
            loading_duration_ms: 123.5,
            resource_type: Some("fetch".to_owned()),
        }],
        pagination_buttons: vec![PaginationButton {
            button_type: PaginationButtonType::Next,
            backend_node_id: 9001,
            text: "Next".to_owned(),
            selector: "#next".to_owned(),
            is_disabled: false,
        }],
        closed_popup_messages: vec!["Closed popup https://ads.example.test".to_owned()],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use browser_use_cdp::{
        BrowserError, BrowserLifecycleEvent, BrowserLifecycleEventKind, BrowserSession,
        FoundElement, Pdf, Screenshot, browser_lifecycle_adapter_events,
    };
    use browser_use_core::{
        ActionExecutor, ActionReplayRematch, ActionResult, Agent, AgentCurrentState, AgentHistory,
        AgentHistoryItem, AgentHistoryReplayExecution, AgentHistoryReplayExecutionItem,
        AgentHistoryReplayPlan, AgentHistoryReplayPlanItem, AgentHistoryReplayRun,
        AgentHistoryReplayStop, AgentHistoryReplayStopReason, AgentOutput, AgentRunError,
        AgentSettings, BrowserActionExecutor, ChatCompletion, ChatModel, ChatRequest, ContentPart,
        FileSystemState, LlmError, ManagedFileSystem, MessageRole, execute_action_sequence,
    };
    use browser_use_dom::{DomInteractedElementMatch, DomInteractedElementMatchLevel};
    use browser_use_tools::{BrowserAction, ClickElementAction, InputTextAction, WaitAction};
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
    fn mixed_interactive_state_can_render_opt_in_ax_description() {
        let rendered = mixed_interactive_state()
            .llm_representation_with_attributes(&["description".to_owned()]);

        assert_eq!(
            rendered.lines().next(),
            Some(
                "[1] <button description=Sends the completed form> Submit request Ignored visual label"
            )
        );
    }

    #[test]
    fn mixed_interactive_state_preserves_quiet_ax_clickability_metadata() {
        let state = mixed_interactive_state();
        let notes = state.selector_map.get(&5).expect("notes element");

        assert_eq!(
            notes.attributes.get("focusable").map(String::as_str),
            Some("true")
        );
        assert_eq!(
            notes.attributes.get("editable").map(String::as_str),
            Some("true")
        );
        assert_eq!(
            notes.attributes.get("settable").map(String::as_str),
            Some("true")
        );
        assert!(!state.llm_representation().contains("focusable=true"));
        assert!(
            state
                .llm_representation_with_attributes(&[
                    "focusable".to_owned(),
                    "editable".to_owned(),
                    "settable".to_owned()
                ])
                .contains(
                    "[5] <div focusable=true editable=true settable=true> Draft note Draft body"
                )
        );
    }

    #[test]
    fn frame_shadow_state_matches_golden_fixture() {
        let expected: Value =
            serde_json::from_str(include_str!("../fixtures/frame_shadow_state.json"))
                .expect("golden fixture");
        let actual = serde_json::to_value(frame_shadow_state()).expect("serialize state");

        assert_eq!(actual, expected);
    }

    #[test]
    fn browser_action_schema_matches_golden_fixture() {
        let actual = serde_json::to_value(schema_for!(BrowserAction)).expect("serialize schema");

        assert_matches_fixture(
            actual,
            include_str!("../fixtures/browser_action_schema.json"),
        );
    }

    #[test]
    fn browser_state_summary_schema_matches_golden_fixture() {
        let actual =
            serde_json::to_value(schema_for!(BrowserStateSummary)).expect("serialize schema");

        assert_matches_fixture(
            actual,
            include_str!("../fixtures/browser_state_summary_schema.json"),
        );
    }

    #[test]
    fn agent_history_replay_run_schema_matches_golden_fixture() {
        let actual =
            serde_json::to_value(schema_for!(AgentHistoryReplayRun)).expect("serialize schema");

        assert_matches_fixture(
            actual,
            include_str!("../fixtures/agent_history_replay_run_schema.json"),
        );
    }

    #[test]
    fn eval_tree_state_matches_golden_fixture() {
        assert_eq!(
            eval_tree_state().eval_representation(),
            include_str!("../fixtures/eval_tree_state.txt").trim_end_matches('\n')
        );
    }

    #[test]
    fn rich_browser_state_summary_matches_golden_fixture() {
        let actual =
            serde_json::to_value(rich_browser_state_summary()).expect("serialize browser state");

        assert_matches_fixture(
            actual,
            include_str!("../fixtures/rich_browser_state_summary.json"),
        );
    }

    #[test]
    fn agent_history_replay_run_matches_golden_fixture() {
        let actual =
            serde_json::to_value(agent_history_replay_run_fixture()).expect("serialize replay run");

        assert_matches_fixture(
            actual,
            include_str!("../fixtures/agent_history_replay_run.json"),
        );
    }

    #[tokio::test]
    async fn agent_history_replay_recapture_run_matches_golden_fixture() {
        let session = ReplayRecaptureFixtureSession::new(vec![
            replay_recapture_state(
                "https://example.test/start",
                vec![replay_recapture_element(
                    7,
                    "button",
                    Some("Reveal email"),
                    Some("Reveal email"),
                    BTreeMap::from([("id".to_owned(), "reveal-email".to_owned())]),
                )],
            ),
            replay_recapture_state(
                "https://example.test/form",
                vec![replay_recapture_element(
                    8,
                    "input",
                    Some("Email"),
                    None,
                    BTreeMap::from([("name".to_owned(), "email".to_owned())]),
                )],
            ),
            replay_recapture_state(
                "https://example.test/form",
                vec![replay_recapture_element(
                    8,
                    "input",
                    Some("Email"),
                    Some("ada@example.test"),
                    BTreeMap::from([("name".to_owned(), "email".to_owned())]),
                )],
            ),
        ]);
        let mut executor = BrowserActionExecutor::new(session);

        let run = executor
            .replay_history(&replay_recapture_history())
            .await
            .expect("recapture replay run");

        assert_eq!(
            executor.session().events(),
            vec!["click:7", "input:8:ada@example.test:true"]
        );
        let actual = serde_json::to_value(run).expect("serialize recapture replay run");
        assert_matches_fixture(
            actual,
            include_str!("../fixtures/agent_history_replay_recapture_run.json"),
        );
    }

    #[test]
    fn agent_history_replay_run_schema_exposes_stop_diagnostics() {
        let schema =
            serde_json::to_value(schema_for!(AgentHistoryReplayRun)).expect("serialize schema");
        let schema_text = serde_json::to_string(&schema).expect("schema text");

        for field in [
            "current_state",
            "plan",
            "execution",
            "diagnostic",
            "page_changed",
        ] {
            assert!(schema_text.contains(field), "schema missing {field}");
        }
    }

    #[test]
    fn browser_lifecycle_events_match_golden_fixture() {
        let actual = serde_json::to_value(vec![
            BrowserLifecycleEvent {
                kind: BrowserLifecycleEventKind::BrowserConnected,
                target_id: None,
                url: Some("http://127.0.0.1:9222".to_owned()),
                reason: None,
                error: None,
                details: BTreeMap::new(),
                message: "Browser connected at http://127.0.0.1:9222".to_owned(),
            },
            BrowserLifecycleEvent::permissions_grant_failed(
                &["clipboardReadWrite".to_owned(), "notifications".to_owned()],
                "permission grant denied",
            ),
            BrowserLifecycleEvent {
                kind: BrowserLifecycleEventKind::TargetCreated,
                target_id: Some("target-1".to_owned()),
                url: Some("https://example.test/report".to_owned()),
                reason: None,
                error: None,
                details: BTreeMap::new(),
                message: "Target target-1 created for https://example.test/report".to_owned(),
            },
            BrowserLifecycleEvent {
                kind: BrowserLifecycleEventKind::NavigationCompleted,
                target_id: Some("target-1".to_owned()),
                url: Some("https://example.test/report".to_owned()),
                reason: None,
                error: None,
                details: BTreeMap::new(),
                message:
                    "Navigation completed on target target-1 to https://example.test/report"
                        .to_owned(),
            },
            BrowserLifecycleEvent {
                kind: BrowserLifecycleEventKind::PopupCloseFailed,
                target_id: None,
                url: Some("https://blocked.test/popup".to_owned()),
                reason: Some("in_prohibited_domains".to_owned()),
                error: Some("No target with given id found".to_owned()),
                details: BTreeMap::new(),
                message:
                    "Failed to close popup https://blocked.test/popup (in_prohibited_domains): No target with given id found"
                        .to_owned(),
            },
        ])
        .expect("serialize lifecycle events");

        assert_matches_fixture(
            actual,
            include_str!("../fixtures/browser_lifecycle_events.json"),
        );
    }

    #[test]
    fn browser_lifecycle_exception_events_match_golden_fixture() {
        let actual = serde_json::to_value(vec![
            BrowserLifecycleEvent {
                kind: BrowserLifecycleEventKind::BrowserReconnecting,
                target_id: None,
                url: Some("http://127.0.0.1:9222".to_owned()),
                reason: None,
                error: None,
                details: BTreeMap::from([
                    ("attempt".to_owned(), "2".to_owned()),
                    ("max_attempts".to_owned(), "3".to_owned()),
                ]),
                message: "Browser reconnecting to http://127.0.0.1:9222 (attempt 2/3)".to_owned(),
            },
            BrowserLifecycleEvent {
                kind: BrowserLifecycleEventKind::TargetCrashed,
                target_id: Some("target-1".to_owned()),
                url: None,
                reason: None,
                error: Some("Inspector target crashed".to_owned()),
                details: BTreeMap::new(),
                message: "Target target-1 crashed: Inspector target crashed".to_owned(),
            },
            BrowserLifecycleEvent {
                kind: BrowserLifecycleEventKind::JavaScriptDialogHandled,
                target_id: None,
                url: Some("https://example.test".to_owned()),
                reason: Some("confirm".to_owned()),
                error: None,
                details: BTreeMap::from([
                    ("action".to_owned(), "accepted".to_owned()),
                    ("dialog_message".to_owned(), "Continue?".to_owned()),
                    ("dialog_type".to_owned(), "confirm".to_owned()),
                ]),
                message:
                    "JavaScript confirm dialog on https://example.test was accepted: Continue?"
                        .to_owned(),
            },
            BrowserLifecycleEvent {
                kind: BrowserLifecycleEventKind::DownloadProgress,
                target_id: None,
                url: None,
                reason: Some("completed".to_owned()),
                error: None,
                details: BTreeMap::from([
                    ("guid".to_owned(), "download-guid".to_owned()),
                    ("received_bytes".to_owned(), "4096".to_owned()),
                    ("state".to_owned(), "completed".to_owned()),
                    ("total_bytes".to_owned(), "4096".to_owned()),
                ]),
                message: "Download download-guid progress: completed (4096 bytes received)"
                    .to_owned(),
            },
            BrowserLifecycleEvent {
                kind: BrowserLifecycleEventKind::StorageStateSaved,
                target_id: None,
                url: None,
                reason: Some("storage_state".to_owned()),
                error: None,
                details: BTreeMap::from([
                    ("cookies_count".to_owned(), "4".to_owned()),
                    ("origins_count".to_owned(), "2".to_owned()),
                    ("path".to_owned(), "/tmp/storage.json".to_owned()),
                ]),
                message: "Storage state saved to /tmp/storage.json (4 cookies, 2 origins)"
                    .to_owned(),
            },
        ])
        .expect("serialize lifecycle events");

        assert_matches_fixture(
            actual,
            include_str!("../fixtures/browser_lifecycle_exception_events.json"),
        );
    }

    #[test]
    fn browser_lifecycle_adapter_events_match_golden_fixture() {
        let lifecycle_events = vec![
            BrowserLifecycleEvent::browser_connected("http://127.0.0.1:9222"),
            BrowserLifecycleEvent::permissions_grant_failed(
                &["clipboardReadWrite".to_owned(), "notifications".to_owned()],
                "permission grant denied",
            ),
            BrowserLifecycleEvent::target_created("target-1", "https://example.test/report"),
            BrowserLifecycleEvent::target_switched("target-1"),
            BrowserLifecycleEvent::navigation_completed(
                "target-1",
                "https://example.test/report",
            ),
            BrowserLifecycleEvent {
                kind: BrowserLifecycleEventKind::CurrentTargetReset,
                target_id: None,
                url: Some("https://blocked.test".to_owned()),
                reason: Some("not_in_allowed_domains".to_owned()),
                error: None,
                details: BTreeMap::new(),
                message:
                    "Blocked navigation to https://blocked.test (not_in_allowed_domains); reset current tab to about:blank"
                        .to_owned(),
            },
            BrowserLifecycleEvent {
                kind: BrowserLifecycleEventKind::PopupCloseFailed,
                target_id: None,
                url: Some("https://blocked.test/popup".to_owned()),
                reason: Some("in_prohibited_domains".to_owned()),
                error: Some("No target with given id found".to_owned()),
                details: BTreeMap::new(),
                message:
                    "Failed to close popup https://blocked.test/popup (in_prohibited_domains): No target with given id found"
                        .to_owned(),
            },
            BrowserLifecycleEvent::storage_state_saved("/tmp/storage.json", 4, 2),
        ];
        let actual = serde_json::to_value(browser_lifecycle_adapter_events(&lifecycle_events))
            .expect("serialize lifecycle adapter events");

        assert_matches_fixture(
            actual,
            include_str!("../fixtures/browser_lifecycle_adapter_events.json"),
        );
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

    #[tokio::test]
    async fn agent_output_schema_matches_golden_fixture() {
        let (llm, requests) = ScriptedChatModel::new(vec![json!({
            "evaluation_previous_goal": "No previous goal",
            "memory": "Need to finish",
            "next_goal": "Finish",
            "action": [
                {
                    "done": {
                        "text": "Done",
                        "success": true
                    }
                }
            ]
        })]);
        let settings = AgentSettings::default();
        let mut agent = Agent::with_settings(
            "Finish",
            settings,
            llm,
            FixtureSession {
                state: fixture_browser_state(),
                clicked: Arc::new(Mutex::new(Vec::new())),
            },
        );

        let history = agent.run(2).await.expect("agent run");
        assert!(history.is_done());

        let requests = requests.lock().expect("requests lock");
        let actual = requests[0]
            .output_schema
            .clone()
            .expect("agent output schema");
        assert_matches_fixture(actual, include_str!("../fixtures/agent_output_schema.json"));
    }

    fn assert_matches_fixture(actual: Value, expected_json: &str) {
        let expected: Value = serde_json::from_str(expected_json).expect("golden fixture");

        assert_eq!(actual, expected);
    }

    fn agent_history_replay_run_fixture() -> AgentHistoryReplayRun {
        let original_action = BrowserAction::Click(ClickElementAction {
            index: Some(1),
            coordinate_x: None,
            coordinate_y: None,
        });
        let remapped_action = BrowserAction::Click(ClickElementAction {
            index: Some(2),
            coordinate_x: None,
            coordinate_y: None,
        });
        let rematch = ActionReplayRematch {
            action: remapped_action.clone(),
            original_index: Some(1),
            rematched_index: Some(2),
            match_result: Some(DomInteractedElementMatch {
                index: 2,
                level: DomInteractedElementMatchLevel::Exact,
                attribute: None,
            }),
            changed: true,
        };
        let plan = AgentHistoryReplayPlan {
            actions: vec![AgentHistoryReplayPlanItem {
                step_index: 1,
                action_index: 0,
                original_action: original_action.clone(),
                remapped_action: remapped_action.clone(),
                rematch: rematch.clone(),
            }],
        };
        let execution = AgentHistoryReplayExecution {
            items: vec![AgentHistoryReplayExecutionItem {
                step_index: 1,
                action_index: 0,
                original_action,
                executed_action: remapped_action,
                rematch,
                result: ActionResult::extracted("Clicked element 2"),
            }],
            stop: Some(AgentHistoryReplayStop {
                step_index: 1,
                action_index: 0,
                reason: AgentHistoryReplayStopReason::PageChanged,
                diagnostic: None,
            }),
        };

        AgentHistoryReplayRun {
            current_state: fixture_browser_state(),
            plan,
            execution,
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
        assert!(request_text.contains("<agent_history>"));
        assert!(request_text.contains("<agent_state>"));
        assert!(request_text.contains("<browser_state>"));
        assert!(request_text.contains("Clicked element 1"));
    }

    #[tokio::test]
    async fn long_task_agent_replay_matches_golden_fixture() {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let file_system = ManagedFileSystem::new(temp_dir.path()).expect("managed file system");
        let clicked = Arc::new(Mutex::new(Vec::new()));
        let (llm, requests) = ScriptedChatModel::new(vec![
            json!({
                "current_state": {
                    "thinking": "Open the workflow",
                    "evaluation_previous_goal": "No previous goal",
                    "memory": "Need to complete a report workflow",
                    "next_goal": "Click Run"
                },
                "current_plan_item": 1,
                "plan_update": [
                    "Trigger the workflow",
                    "Draft the report",
                    "Review the report",
                    "Display the report and finish"
                ],
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
                    "thinking": "Check for status text",
                    "evaluation_previous_goal": "Clicked element 1",
                    "memory": "Workflow was triggered",
                    "next_goal": "Find status"
                },
                "current_plan_item": 2,
                "action": [
                    {
                        "find_text": {
                            "text": "Ready"
                        }
                    }
                ]
            }),
            json!({
                "current_state": {
                    "thinking": "Recover from unsupported text lookup by writing the report",
                    "evaluation_previous_goal": "find_text failed",
                    "memory": "Use the report artifact to preserve progress",
                    "next_goal": "Write report"
                },
                "current_plan_item": 2,
                "plan_update": [
                    "Triggered the workflow",
                    "Skipped unsupported status lookup",
                    "Draft the report",
                    "Review and display the report"
                ],
                "action": [
                    {
                        "write_file": {
                            "file_name": "report.md",
                            "content": "Run clicked",
                            "append": false,
                            "trailing_newline": false,
                            "leading_newline": false
                        }
                    }
                ]
            }),
            json!({
                "current_state": {
                    "thinking": "Read the draft back",
                    "evaluation_previous_goal": "Wrote report.md",
                    "memory": "Report draft exists",
                    "next_goal": "Read report"
                },
                "current_plan_item": 3,
                "action": [
                    {
                        "read_file": {
                            "file_name": "report.md"
                        }
                    }
                ]
            }),
            json!({
                "current_state": {
                    "thinking": "Append review notes",
                    "evaluation_previous_goal": "Read report.md",
                    "memory": "Report says Run clicked",
                    "next_goal": "Append verification"
                },
                "current_plan_item": 3,
                "action": [
                    {
                        "write_file": {
                            "file_name": "report.md",
                            "content": "Report verified",
                            "append": true,
                            "trailing_newline": false,
                            "leading_newline": true
                        }
                    }
                ]
            }),
            json!({
                "current_state": {
                    "thinking": "Read final report",
                    "evaluation_previous_goal": "Appended report.md",
                    "memory": "Report has verification",
                    "next_goal": "Read final report"
                },
                "current_plan_item": 3,
                "action": [
                    {
                        "read_file": {
                            "file_name": "report.md"
                        }
                    }
                ]
            }),
            json!({
                "current_state": {
                    "thinking": "Finish with displayed report",
                    "evaluation_previous_goal": "Read final report",
                    "memory": "Report is ready",
                    "next_goal": "Finish"
                },
                "current_plan_item": 4,
                "plan_update": [
                    "Triggered workflow",
                    "Drafted report",
                    "Reviewed report",
                    "Displayed report"
                ],
                "action": [
                    {
                        "done": {
                            "text": "Long task complete",
                            "success": true,
                            "files_to_display": ["report.md"]
                        }
                    }
                ]
            }),
        ]);
        let settings = AgentSettings {
            max_actions_per_step: 1,
            max_history_items: Some(4),
            planning_replan_on_stall: 1,
            planning_exploration_limit: 2,
            ..AgentSettings::default()
        };
        let mut agent = Agent::with_settings_and_file_system(
            "Complete the long report workflow",
            settings,
            llm,
            FixtureSession {
                state: fixture_browser_state(),
                clicked: Arc::clone(&clicked),
            },
            file_system,
        );

        agent.run(7).await.expect("long task run");
        let history = agent.history();
        assert!(history.is_done());
        let prompt_texts = {
            let requests = requests.lock().expect("requests lock");
            assert_eq!(requests.len(), 7);
            requests.iter().map(request_text).collect::<Vec<_>>()
        };
        let final_result = history.final_result().unwrap_or_default();
        let actual = json!({
            "history_summary": {
                "steps": history.items.len(),
                "is_done": history.is_done(),
                "is_successful": history.is_successful(),
                "action_names": history_action_names(history),
                "clicked_indices": clicked.lock().expect("clicked lock").clone(),
                "final_file_names": agent.file_system().list_files()
            },
            "prompt_checks": {
                "first_prompt_has_planning_guidance": prompt_texts[0].contains("When useful, include `current_plan_item` and `plan_update`"),
                "third_prompt_requests_recovery_replan": prompt_texts[2].contains("Recent steps have failed or stalled"),
                "sixth_prompt_requests_plan_update": prompt_texts[5].contains("explored for several steps without updating the plan"),
                "seventh_prompt_reports_stagnant_page": prompt_texts[6].contains("page content has not changed across 6 consecutive observations"),
                "seventh_prompt_preserves_initial_click_result": prompt_texts[6].contains("Clicked element 1"),
                "seventh_prompt_omits_recovered_find_error": !prompt_texts[6].contains("fixture session does not support find_text"),
                "seventh_prompt_keeps_recent_report_read": prompt_texts[6].contains("Report verified")
            },
            "final_result_checks": {
                "contains_done_text": final_result.contains("Long task complete"),
                "displays_report_heading": final_result.contains("report.md"),
                "displays_report_body": final_result.contains("Run clicked") && final_result.contains("Report verified")
            }
        });

        assert_matches_fixture(
            actual,
            include_str!("../fixtures/long_task_agent_replay.json"),
        );
    }

    #[tokio::test]
    async fn managed_file_system_replay_matches_golden_fixture() {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let file_system = ManagedFileSystem::new(temp_dir.path()).expect("managed file system");
        let (writer_llm, _writer_requests) = ScriptedChatModel::new(vec![json!({
            "current_state": {
                "thinking": "write restored files",
                "evaluation_previous_goal": "No previous goal",
                "memory": "Need filesystem replay",
                "next_goal": "Seed managed files"
            },
            "action": [
                {
                    "write_file": {
                        "file_name": "todo.md",
                        "content": "- inspect restored report",
                        "append": false,
                        "trailing_newline": false,
                        "leading_newline": false
                    }
                },
                {
                    "write_file": {
                        "file_name": "report.md",
                        "content": "alpha\nbeta",
                        "append": false,
                        "trailing_newline": false,
                        "leading_newline": false
                    }
                }
            ]
        })]);
        let mut writer = Agent::with_settings_and_file_system(
            "seed managed files",
            AgentSettings::default(),
            writer_llm,
            FixtureSession {
                state: fixture_browser_state(),
                clicked: Arc::new(Mutex::new(Vec::new())),
            },
            file_system,
        );

        writer.step().await.expect("writer step");
        let first_extracted = writer
            .file_system_mut()
            .save_extracted_content("<extraction>\nseed restored extract\n</extraction>")
            .expect("seed extracted content");
        assert_eq!(first_extracted, "extracted_content_0.md");
        let saved_state = writer.file_system_state();

        let restored = ManagedFileSystem::from_state(saved_state.clone()).expect("restore state");
        let (reader_llm, reader_requests) = ScriptedChatModel::new(vec![
            json!({
                "current_state": {
                    "thinking": "read restored report",
                    "evaluation_previous_goal": "Seeded managed files",
                    "memory": "Need report contents",
                    "next_goal": "Read report"
                },
                "action": [
                    {
                        "read_file": {
                            "file_name": "report.md"
                        }
                    }
                ]
            }),
            json!({
                "current_state": {
                    "thinking": "finish replay",
                    "evaluation_previous_goal": "Read report",
                    "memory": "Report contained alpha and beta",
                    "next_goal": "Finish"
                },
                "action": [
                    {
                        "done": {
                            "text": "restored replay complete",
                            "success": true,
                            "files_to_display": []
                        }
                    }
                ]
            }),
        ]);
        let mut reader = Agent::with_settings_and_file_system(
            "continue managed filesystem replay",
            AgentSettings::default(),
            reader_llm,
            FixtureSession {
                state: fixture_browser_state(),
                clicked: Arc::new(Mutex::new(Vec::new())),
            },
            restored,
        );

        reader.run(2).await.expect("reader run");
        assert!(reader.history().is_done());
        let (first_prompt, second_prompt) = {
            let requests = reader_requests.lock().expect("requests lock");
            assert_eq!(requests.len(), 2);
            (request_text(&requests[0]), request_text(&requests[1]))
        };
        let next_extracted = reader
            .file_system_mut()
            .save_extracted_content("after restored replay")
            .expect("next extracted content");

        let first_result = &reader.history().items[0].result[0];
        let second_result = &reader.history().items[1].result[0];
        let actual = json!({
            "state_after_writer": normalize_file_system_state(saved_state),
            "restored_first_prompt": {
                "file_system": tagged_section(&first_prompt, "file_system"),
                "todo_contents": tagged_section(&first_prompt, "todo_contents")
            },
            "restored_second_prompt": {
                "read_state": tagged_section(&second_prompt, "read_state")
            },
            "restored_history_summary": {
                "steps": reader.history().items.len(),
                "is_done": reader.history().is_done(),
                "final_result": reader.history().final_result(),
                "first_step_result": first_result.extracted_content.as_deref(),
                "first_step_long_term_memory": first_result.long_term_memory.as_deref(),
                "second_step_result": second_result.extracted_content.as_deref()
            },
            "next_extracted_file": next_extracted,
            "final_file_names": reader.file_system().list_files()
        });

        assert_matches_fixture(
            actual,
            include_str!("../fixtures/managed_file_system_replay.json"),
        );
    }

    #[tokio::test]
    async fn agent_checkpoint_resume_matches_golden_fixture() {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let file_system = ManagedFileSystem::new(temp_dir.path()).expect("managed file system");
        let settings = AgentSettings {
            initial_actions: vec![BrowserAction::Wait(WaitAction { seconds: 0 })],
            ..AgentSettings::default()
        };
        let (writer_llm, _writer_requests) = ScriptedChatModel::new(vec![json!({
            "current_state": {
                "thinking": "write checkpoint files",
                "evaluation_previous_goal": "No previous goal",
                "memory": "Need a checkpoint",
                "next_goal": "Seed files"
            },
            "action": [
                {
                    "write_file": {
                        "file_name": "todo.md",
                        "content": "- checkpoint read report",
                        "append": false,
                        "trailing_newline": false,
                        "leading_newline": false
                    }
                },
                {
                    "write_file": {
                        "file_name": "report.md",
                        "content": "alpha\nbeta",
                        "append": false,
                        "trailing_newline": false,
                        "leading_newline": false
                    }
                }
            ]
        })]);
        let mut writer = Agent::with_settings_and_file_system(
            "checkpoint conformance",
            settings,
            writer_llm,
            FixtureSession {
                state: fixture_browser_state(),
                clicked: Arc::new(Mutex::new(Vec::new())),
            },
            file_system,
        );

        assert!(matches!(
            writer.run(0).await,
            Err(AgentRunError::StepLimitReached { max_steps: 0 })
        ));
        writer.step().await.expect("write checkpoint files");
        writer
            .file_system_mut()
            .save_extracted_content("checkpoint conformance extract")
            .expect("seed extracted content");
        let checkpoint = writer.checkpoint();
        let checkpoint_json =
            serde_json::to_string_pretty(&checkpoint).expect("serialize checkpoint");
        let checkpoint = serde_json::from_str(&checkpoint_json).expect("deserialize checkpoint");

        let (reader_llm, reader_requests) = ScriptedChatModel::new(vec![
            json!({
                "current_state": {
                    "thinking": "read checkpoint report",
                    "evaluation_previous_goal": "Seeded checkpoint",
                    "memory": "Need report contents",
                    "next_goal": "Read report"
                },
                "action": [
                    {
                        "read_file": {
                            "file_name": "report.md"
                        }
                    }
                ]
            }),
            json!({
                "current_state": {
                    "thinking": "finish checkpoint replay",
                    "evaluation_previous_goal": "Read report",
                    "memory": "Report contained alpha and beta",
                    "next_goal": "Finish"
                },
                "action": [
                    {
                        "done": {
                            "text": "checkpoint replay complete",
                            "success": true,
                            "files_to_display": []
                        }
                    }
                ]
            }),
        ]);
        let mut reader = Agent::from_checkpoint(
            checkpoint,
            reader_llm,
            FixtureSession {
                state: fixture_browser_state(),
                clicked: Arc::new(Mutex::new(Vec::new())),
            },
        )
        .expect("resume checkpoint");

        reader.run(2).await.expect("reader run");
        assert!(reader.history().is_done());
        let (first_prompt, second_prompt) = {
            let requests = reader_requests.lock().expect("requests lock");
            assert_eq!(requests.len(), 2);
            (request_text(&requests[0]), request_text(&requests[1]))
        };
        let next_extracted = reader
            .file_system_mut()
            .save_extracted_content("after checkpoint replay")
            .expect("next extracted content");

        let checkpoint = reader.checkpoint();
        let checkpoint_history = &checkpoint.history.items;
        let actual = json!({
            "checkpoint": {
                "task": checkpoint.task.as_str(),
                "initial_actions_executed": checkpoint.initial_actions_executed,
                "initial_actions": &checkpoint.settings.initial_actions,
                "history_action_names": history_action_names(&checkpoint.history),
                "history_results": checkpoint_history
                    .iter()
                    .map(|item| item.result.iter()
                        .filter_map(|result| result.extracted_content.as_deref())
                        .collect::<Vec<_>>())
                    .collect::<Vec<_>>(),
                "file_system_state": normalize_file_system_state(checkpoint.file_system_state.clone())
            },
            "resumed_first_prompt": {
                "file_system": tagged_section(&first_prompt, "file_system"),
                "todo_contents": tagged_section(&first_prompt, "todo_contents"),
                "contains_prior_initial_action": first_prompt.contains("Waited for 0 seconds"),
                "contains_prior_write_result": first_prompt.contains("Wrote file report.md")
            },
            "resumed_second_prompt": {
                "read_state": tagged_section(&second_prompt, "read_state")
            },
            "resumed_history_summary": {
                "steps": checkpoint_history.len(),
                "is_done": reader.history().is_done(),
                "final_result": reader.history().final_result(),
                "first_resumed_result": checkpoint_history[2].result[0].extracted_content.as_deref(),
                "second_resumed_result": checkpoint_history[3].result[0].extracted_content.as_deref()
            },
            "next_extracted_file": next_extracted,
            "final_file_names": reader.file_system().list_files()
        });

        assert_matches_fixture(
            actual,
            include_str!("../fixtures/agent_checkpoint_replay.json"),
        );
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

    fn normalize_file_system_state(state: FileSystemState) -> Value {
        let mut value = serde_json::to_value(state).expect("serialize file system state");
        value["base_dir"] = Value::String("__BASE_DIR__".to_owned());
        value
    }

    fn history_action_names(history: &browser_use_core::AgentHistory) -> Vec<Vec<String>> {
        history
            .items
            .iter()
            .map(|item| {
                item.model_output
                    .as_ref()
                    .map(|output| {
                        output
                            .action
                            .iter()
                            .map(|action| action.name().to_owned())
                            .collect()
                    })
                    .unwrap_or_default()
            })
            .collect()
    }

    fn request_text(request: &ChatRequest) -> String {
        request
            .messages
            .iter()
            .filter(|message| message.role == MessageRole::User)
            .flat_map(|message| message.content.iter())
            .filter_map(|part| match part {
                ContentPart::Text { text } => Some(text.as_str()),
                ContentPart::ImageUrl { .. } => None,
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn tagged_section(text: &str, tag: &str) -> String {
        let start = format!("<{tag}>\n");
        let end = format!("\n</{tag}>");
        let start_index = text.find(&start).expect("tag start") + start.len();
        let end_index = text[start_index..]
            .find(&end)
            .map(|index| start_index + index)
            .expect("tag end");
        text[start_index..end_index].to_owned()
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

    fn replay_recapture_history() -> AgentHistory {
        AgentHistory {
            items: vec![
                AgentHistoryItem {
                    model_output: Some(AgentOutput {
                        current_state: AgentCurrentState::default(),
                        thinking: None,
                        evaluation_previous_goal: None,
                        memory: None,
                        next_goal: Some("Reveal the email field".to_owned()),
                        current_plan_item: None,
                        plan_update: None,
                        action: vec![BrowserAction::Click(ClickElementAction {
                            index: Some(1),
                            coordinate_x: None,
                            coordinate_y: None,
                        })],
                    }),
                    result: vec![ActionResult::extracted("Clicked element 1")],
                    state: replay_recapture_state(
                        "https://example.test/start",
                        vec![replay_recapture_element(
                            1,
                            "button",
                            Some("Reveal email"),
                            Some("Reveal email"),
                            BTreeMap::from([("id".to_owned(), "reveal-email".to_owned())]),
                        )],
                    ),
                    metadata: None,
                },
                AgentHistoryItem {
                    model_output: Some(AgentOutput {
                        current_state: AgentCurrentState::default(),
                        thinking: None,
                        evaluation_previous_goal: None,
                        memory: None,
                        next_goal: Some("Fill the email field".to_owned()),
                        current_plan_item: None,
                        plan_update: None,
                        action: vec![BrowserAction::Input(InputTextAction {
                            index: 2,
                            text: "ada@example.test".to_owned(),
                            clear: true,
                        })],
                    }),
                    result: vec![ActionResult::extracted("Typed text into element 2")],
                    state: replay_recapture_state(
                        "https://example.test/form",
                        vec![replay_recapture_element(
                            2,
                            "input",
                            Some("Email"),
                            None,
                            BTreeMap::from([("name".to_owned(), "email".to_owned())]),
                        )],
                    ),
                    metadata: None,
                },
            ],
            ..AgentHistory::default()
        }
    }

    fn replay_recapture_state(url: &str, elements: Vec<DomElementRef>) -> BrowserStateSummary {
        BrowserStateSummary {
            dom_state: SerializedDomState::from_elements(elements),
            url: url.to_owned(),
            title: "Replay Recapture".to_owned(),
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

    fn replay_recapture_element(
        index: u32,
        tag_name: &str,
        name: Option<&str>,
        text: Option<&str>,
        attributes: BTreeMap<String, String>,
    ) -> DomElementRef {
        DomElementRef {
            index,
            target_id: "recapture-target".to_owned(),
            backend_node_id: u64::from(index),
            node_id: Some(u64::from(index)),
            tag_name: tag_name.to_owned(),
            role: None,
            name: name.map(str::to_owned),
            text: text.map(str::to_owned),
            attributes,
            bounds: None,
            is_visible: true,
            is_interactive: true,
            is_scrollable: false,
        }
    }

    #[derive(Clone)]
    struct ReplayRecaptureFixtureSession {
        states: Arc<Mutex<VecDeque<BrowserStateSummary>>>,
        events: Arc<Mutex<Vec<String>>>,
    }

    impl ReplayRecaptureFixtureSession {
        fn new(states: Vec<BrowserStateSummary>) -> Self {
            Self {
                states: Arc::new(Mutex::new(states.into())),
                events: Arc::new(Mutex::new(Vec::new())),
            }
        }

        fn events(&self) -> Vec<String> {
            self.events.lock().expect("events lock").clone()
        }
    }

    #[async_trait]
    impl BrowserSession for ReplayRecaptureFixtureSession {
        async fn state(
            &self,
            _include_screenshot: bool,
        ) -> Result<BrowserStateSummary, BrowserError> {
            self.states
                .lock()
                .expect("states lock")
                .pop_front()
                .ok_or_else(|| {
                    BrowserError::StateUnavailable("fixture states exhausted".to_owned())
                })
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
            self.events
                .lock()
                .expect("events lock")
                .push(format!("click:{index}"));
            Ok(())
        }

        async fn click_coordinates(&self, _x: i32, _y: i32) -> Result<(), BrowserError> {
            Err(unsupported_action("click_coordinates"))
        }

        async fn input_text(
            &self,
            index: u32,
            text: &str,
            clear: bool,
        ) -> Result<(), BrowserError> {
            self.events
                .lock()
                .expect("events lock")
                .push(format!("input:{index}:{text}:{clear}"));
            Ok(())
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
            let content = self
                .outputs
                .lock()
                .expect("outputs lock")
                .pop_front()
                .ok_or_else(|| LlmError::Provider("script exhausted".to_owned()))?;
            self.requests.lock().expect("requests lock").push(request);
            Ok(ChatCompletion {
                model: self.model().to_owned(),
                content,
                usage: None,
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
