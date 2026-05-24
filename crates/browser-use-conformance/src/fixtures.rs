use super::*;

#[must_use]
/// Returns a tiny two-element DOM state used by baseline prompt/render tests.
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
/// Returns a DOM state covering rich attributes, scrollable elements, and media widgets.
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
/// Returns a DOM state with root and child-frame targets plus shadow-like elements.
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
/// Returns a tree-shaped DOM state for judge/evaluation rendering tests.
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
/// Returns a full browser-state summary with tabs, metrics, network, and pagination fixtures.
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
