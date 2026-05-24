use super::*;
use std::collections::BTreeMap;

fn test_element(index: u32, tag_name: &str, attributes: BTreeMap<String, String>) -> DomElementRef {
    DomElementRef {
        index,
        target_id: format!("target-{index}"),
        backend_node_id: u64::from(index),
        node_id: Some(u64::from(index)),
        tag_name: tag_name.to_owned(),
        role: None,
        name: None,
        text: None,
        attributes,
        bounds: None,
        is_visible: true,
        is_interactive: true,
        is_scrollable: false,
    }
}

#[test]
fn short_target_id_matches_browser_use_display_shape() {
    let tab = TabInfo {
        url: "about:blank".to_owned(),
        title: "Blank".to_owned(),
        tab_id: TabInfo::tab_id_for_target("123456abcdef"),
        target_id: "123456abcdef".to_owned(),
        parent_target_id: None,
    };

    assert_eq!(tab.short_target_id(), "cdef");
}

#[test]
fn tab_info_serializes_browser_use_short_tab_id() {
    let tab = TabInfo {
        url: "about:blank".to_owned(),
        title: "Blank".to_owned(),
        tab_id: TabInfo::tab_id_for_target("123456abcdef"),
        target_id: "123456abcdef".to_owned(),
        parent_target_id: None,
    };

    let value = serde_json::to_value(tab).expect("tab json");

    assert_eq!(value["tab_id"], "cdef");
    assert_eq!(value["target_id"], "123456abcdef");
}

#[test]
fn empty_dom_state_has_zero_elements() {
    let dom = SerializedDomState::default();
    assert_eq!(dom.element_count(), 0);
    assert_eq!(dom.llm_representation(), EMPTY_DOM_TREE_MESSAGE);
    assert_eq!(
        dom.llm_representation_with_attributes(&["data-testid".to_owned()]),
        EMPTY_DOM_TREE_MESSAGE
    );
    assert_eq!(dom.eval_representation(), EMPTY_DOM_TREE_MESSAGE);
}

#[test]
fn eval_representation_preserves_tree_structure_without_action_indexes() {
    let root = DomEvalNode::element("html").with_children(vec![
        DomEvalNode::element("body").with_children(vec![
            DomEvalNode::element("main").with_children(vec![
                DomEvalNode::element("h1").with_children(vec![DomEvalNode::text("Account setup")]),
                DomEvalNode::element("form").with_children(vec![
                    DomEvalNode::element("label")
                        .with_attribute("for", "email")
                        .with_children(vec![DomEvalNode::text("Email")]),
                    DomEvalNode::element("input")
                        .with_attribute("id", "email")
                        .with_attribute("name", "email")
                        .with_attribute("type", "email")
                        .with_attribute("placeholder", "you@example.com")
                        .with_attribute("required", "true")
                        .interactive(42),
                    DomEvalNode::element("button")
                        .with_attribute("data-testid", "submit-account")
                        .with_children(vec![DomEvalNode::text("Continue")])
                        .interactive(43),
                ]),
            ]),
        ]),
    ]);
    let state = SerializedDomState::default().with_eval_root(root);

    assert_eq!(
        state.eval_representation(),
        "<html />\n\t<body />\n\t\t<main />\n\t\t\t<h1>Account setup\n\t\t\t<form />\n\t\t\t\t<label>Email\n\t\t\t\t[i_42] <input id=\"email\" name=\"email\" type=\"email\" placeholder=\"you@example.com\" required=\"true\" />\n\t\t\t\t[i_43] <button data-testid=\"submit-account\">Continue"
    );
}

#[test]
fn eval_representation_handles_shadow_iframe_svg_and_scroll_markers() {
    let root = DomEvalNode::element("body").with_children(vec![
        DomEvalNode::element("section")
            .scrollable("0.0 pages above, 2.0 pages below")
            .with_children(vec![
                DomEvalNode::element("button")
                    .with_attribute("aria-expanded", "false")
                    .with_children(vec![DomEvalNode::text("Open menu")])
                    .interactive(51),
            ]),
        DomEvalNode::document_fragment(vec![
            DomEvalNode::element("button")
                .with_attribute("id", "shadow-save")
                .with_children(vec![DomEvalNode::text("Save")])
                .interactive(52),
        ]),
        DomEvalNode::element("iframe")
            .with_attribute("title", "Checkout")
            .with_children(vec![
                DomEvalNode::element("button")
                    .with_children(vec![DomEvalNode::text("Pay now")])
                    .interactive(53),
            ]),
        DomEvalNode::element("svg")
            .with_attribute("aria-label", "Decorative chart")
            .with_children(vec![
                DomEvalNode::element("path").with_attribute("d", "M0 0"),
            ]),
    ]);
    let state = SerializedDomState::default().with_eval_root(root);

    assert_eq!(
        state.eval_representation(),
        "<body />\n\t<section scroll=\"0.0 pages above, 2.0 pages below\" />\n\t\t[i_51] <button aria-expanded=\"false\">Open menu\n\t#shadow\n\t\t[i_52] <button id=\"shadow-save\">Save\n\t<iframe title=\"Checkout\" />\n\t\t#iframe-content\n\t\t\t[i_53] <button>Pay now\n\t<svg aria-label=\"Decorative chart\" /> <!-- SVG content collapsed -->"
    );
}

#[test]
fn serialized_state_renders_label_and_current_value() {
    let element = DomElementRef {
        index: 1,
        target_id: "target".to_owned(),
        backend_node_id: 0,
        node_id: None,
        tag_name: "input".to_owned(),
        role: None,
        name: Some("Name".to_owned()),
        text: Some("EvalOps".to_owned()),
        attributes: BTreeMap::new(),
        bounds: Some(ElementBounds {
            x: 10,
            y: 20,
            width: 120,
            height: 32,
        }),
        is_visible: true,
        is_interactive: true,
        is_scrollable: false,
    };

    let state = SerializedDomState::from_elements(vec![element]);

    assert_eq!(state.llm_representation(), "[1] <input> Name EvalOps");
    assert_eq!(state.element_count(), 1);
    assert_eq!(state.selector_map[&1].bounds.expect("bounds").width, 120);
}

#[test]
fn interacted_element_records_upstream_style_metadata() {
    let element = DomElementRef {
        index: 7,
        target_id: "target-abc123".to_owned(),
        backend_node_id: 42,
        node_id: Some(11),
        tag_name: "button".to_owned(),
        role: Some("button".to_owned()),
        name: Some("Save".to_owned()),
        text: Some("Save changes".to_owned()),
        attributes: BTreeMap::from([
            ("id".to_owned(), "save-button".to_owned()),
            ("class".to_owned(), "btn primary hover".to_owned()),
            ("ax_name".to_owned(), "Save".to_owned()),
        ]),
        bounds: Some(ElementBounds {
            x: 10,
            y: 20,
            width: 120,
            height: 32,
        }),
        is_visible: true,
        is_interactive: true,
        is_scrollable: false,
    };

    let interacted = DomInteractedElement::from_element(&element);

    assert_eq!(interacted.target_id, "target-abc123");
    assert_eq!(interacted.node_id, 11);
    assert_eq!(interacted.backend_node_id, 42);
    assert_eq!(interacted.node_type, 1);
    assert_eq!(interacted.node_name, "button");
    assert_eq!(interacted.node_value, "Save changes");
    assert_eq!(interacted.x_path, "//button[@id='save-button']");
    assert_eq!(interacted.ax_name.as_deref(), Some("Save"));
    assert_eq!(interacted.bounds.expect("bounds").width, 120);
    assert_ne!(interacted.element_hash, 0);
    assert!(interacted.stable_hash.is_some());
}

#[test]
fn interacted_element_stable_hash_filters_dynamic_classes() {
    let first = DomElementRef {
        index: 1,
        target_id: "target".to_owned(),
        backend_node_id: 1,
        node_id: Some(1),
        tag_name: "button".to_owned(),
        role: None,
        name: Some("Save".to_owned()),
        text: Some("Save".to_owned()),
        attributes: BTreeMap::from([
            ("class".to_owned(), "btn primary hover selected".to_owned()),
            ("data-testid".to_owned(), "save".to_owned()),
            ("ax_name".to_owned(), "Save".to_owned()),
        ]),
        bounds: None,
        is_visible: true,
        is_interactive: true,
        is_scrollable: false,
    };
    let second = DomElementRef {
        attributes: BTreeMap::from([
            ("class".to_owned(), "active primary btn loading".to_owned()),
            ("data-testid".to_owned(), "save".to_owned()),
            ("ax_name".to_owned(), "Save".to_owned()),
        ]),
        ..first.clone()
    };

    let first = DomInteractedElement::from_element(&first);
    let second = DomInteractedElement::from_element(&second);

    assert_ne!(first.element_hash, second.element_hash);
    assert_eq!(first.stable_hash, second.stable_hash);
    assert_eq!(first.x_path, second.x_path);
}

#[test]
fn interacted_element_uses_element_name_as_ax_fallback() {
    let element = DomElementRef {
        index: 3,
        target_id: "target".to_owned(),
        backend_node_id: 3,
        node_id: Some(3),
        tag_name: "button".to_owned(),
        role: Some("button".to_owned()),
        name: Some("Fallback Label".to_owned()),
        text: Some("Fallback Label".to_owned()),
        attributes: BTreeMap::from([("id".to_owned(), "fallback".to_owned())]),
        bounds: None,
        is_visible: true,
        is_interactive: true,
        is_scrollable: false,
    };

    let interacted = DomInteractedElement::from_element(&element);

    assert_eq!(interacted.ax_name.as_deref(), Some("Fallback Label"));
}

#[test]
fn interacted_element_rematches_exact_hash_across_targets() {
    let historical = DomInteractedElement::from_element(&test_element(
        1,
        "button",
        BTreeMap::from([
            ("id".to_owned(), "save".to_owned()),
            ("class".to_owned(), "btn primary".to_owned()),
        ]),
    ));
    let current = test_element(
        9,
        "button",
        BTreeMap::from([
            ("id".to_owned(), "save".to_owned()),
            ("class".to_owned(), "btn primary".to_owned()),
        ]),
    );
    let state = SerializedDomState::from_elements(vec![current]);

    assert_eq!(
        historical.rematch(&state).expect("exact match"),
        DomInteractedElementMatch {
            index: 9,
            level: DomInteractedElementMatchLevel::Exact,
            attribute: None,
        }
    );
}

#[test]
fn interacted_element_rematches_stable_hash_after_dynamic_class_change() {
    let historical = DomInteractedElement::from_element(&test_element(
        1,
        "button",
        BTreeMap::from([
            ("data-testid".to_owned(), "save".to_owned()),
            ("class".to_owned(), "btn primary hover selected".to_owned()),
        ]),
    ));
    let current = test_element(
        4,
        "button",
        BTreeMap::from([
            ("data-testid".to_owned(), "save".to_owned()),
            ("class".to_owned(), "active primary btn loading".to_owned()),
        ]),
    );
    let state = SerializedDomState::from_elements(vec![current]);

    assert_eq!(
        historical.rematch(&state).expect("stable match").level,
        DomInteractedElementMatchLevel::Stable
    );
    assert_eq!(historical.rematch(&state).expect("stable match").index, 4);
}

#[test]
fn interacted_element_rematches_xpath_when_hashes_are_unavailable() {
    let current = test_element(
        5,
        "a",
        BTreeMap::from([("id".to_owned(), "docs".to_owned())]),
    );
    let mut historical = DomInteractedElement::from_element(&current);
    historical.element_hash = 0;
    historical.stable_hash = None;
    let state = SerializedDomState::from_elements(vec![current]);

    assert_eq!(
        historical.rematch(&state).expect("xpath match"),
        DomInteractedElementMatch {
            index: 5,
            level: DomInteractedElementMatchLevel::XPath,
            attribute: None,
        }
    );
}

#[test]
fn interacted_element_rematches_ax_name_when_structure_changes() {
    let mut historical =
        DomInteractedElement::from_element(&test_element(1, "button", BTreeMap::new()));
    historical.element_hash = 0;
    historical.stable_hash = None;
    historical.x_path = String::new();
    historical.ax_name = Some("Open menu".to_owned());
    let mut current = test_element(8, "button", BTreeMap::new());
    current.name = Some("Open menu".to_owned());
    let state = SerializedDomState::from_elements(vec![current]);

    assert_eq!(
        historical.rematch(&state).expect("ax match").level,
        DomInteractedElementMatchLevel::AxName
    );
}

#[test]
fn interacted_element_rematches_unique_attribute_fallback() {
    let mut historical = DomInteractedElement::from_element(&test_element(
        1,
        "input",
        BTreeMap::from([("name".to_owned(), "email".to_owned())]),
    ));
    historical.element_hash = 0;
    historical.stable_hash = None;
    historical.x_path = String::new();
    historical.ax_name = None;
    let state = SerializedDomState::from_elements(vec![
        test_element(
            3,
            "input",
            BTreeMap::from([("name".to_owned(), "email".to_owned())]),
        ),
        test_element(
            4,
            "input",
            BTreeMap::from([("name".to_owned(), "password".to_owned())]),
        ),
    ]);

    assert_eq!(
        historical.rematch(&state).expect("attribute match"),
        DomInteractedElementMatch {
            index: 3,
            level: DomInteractedElementMatchLevel::Attribute,
            attribute: Some("name".to_owned()),
        }
    );
}

#[test]
fn interacted_element_rematch_reports_ambiguous_and_missing_matches() {
    let mut historical =
        DomInteractedElement::from_element(&test_element(1, "button", BTreeMap::new()));
    historical.element_hash = 0;
    historical.stable_hash = None;
    historical.x_path = String::new();
    historical.ax_name = Some("Duplicate".to_owned());
    let mut first = test_element(1, "button", BTreeMap::new());
    first.name = Some("Duplicate".to_owned());
    let mut second = test_element(2, "button", BTreeMap::new());
    second.name = Some("Duplicate".to_owned());
    let state = SerializedDomState::from_elements(vec![first, second]);

    let error = historical.rematch(&state).expect_err("ambiguous match");
    assert_eq!(
        error.reason,
        DomInteractedElementMatchFailureReason::Ambiguous
    );
    assert_eq!(error.level, Some(DomInteractedElementMatchLevel::AxName));
    assert_eq!(error.candidate_indices, vec![1, 2]);

    let empty = historical
        .rematch(&SerializedDomState::default())
        .expect_err("empty selector map");
    assert_eq!(
        empty.reason,
        DomInteractedElementMatchFailureReason::EmptySelectorMap
    );

    historical.ax_name = Some("Missing".to_owned());
    let missing = historical.rematch(&state).expect_err("missing match");
    assert_eq!(
        missing.reason,
        DomInteractedElementMatchFailureReason::NotFound
    );
}

#[test]
fn serialized_state_renders_compound_component_metadata() {
    let element = DomElementRef {
        index: 1,
        target_id: "target".to_owned(),
        backend_node_id: 0,
        node_id: None,
        tag_name: "select".to_owned(),
        role: None,
        name: Some("Plan".to_owned()),
        text: Some("Enterprise".to_owned()),
        attributes: BTreeMap::from([
            ("id".to_owned(), "plan".to_owned()),
            ("value".to_owned(), "Enterprise".to_owned()),
            (
                "compound_components".to_owned(),
                "(name=Dropdown Toggle,role=button),(name=Options,role=listbox,count=2,options=Starter|Enterprise)"
                    .to_owned(),
            ),
        ]),
        bounds: None,
        is_visible: true,
        is_interactive: true,
        is_scrollable: false,
    };

    let state = SerializedDomState::from_elements(vec![element]);

    assert_eq!(
        state.llm_representation(),
        "[1] <select id=plan value=Enterprise compound_components=(name=Dropdown Toggle,role=button),(name=Options,role=listbox,count=2,options=Starter|Enterprise)> Plan Enterprise"
    );
}

#[test]
fn serialized_state_renders_included_attributes_without_password_values() {
    let visible = DomElementRef {
        index: 1,
        target_id: "target".to_owned(),
        backend_node_id: 0,
        node_id: None,
        tag_name: "input".to_owned(),
        role: None,
        name: Some("Email".to_owned()),
        text: Some("user@example.com".to_owned()),
        attributes: BTreeMap::from([
            ("aria-required".to_owned(), "true".to_owned()),
            ("id".to_owned(), "email".to_owned()),
            ("placeholder".to_owned(), "name@example.com".to_owned()),
            ("type".to_owned(), "email".to_owned()),
            ("value".to_owned(), "user@example.com".to_owned()),
        ]),
        bounds: None,
        is_visible: true,
        is_interactive: true,
        is_scrollable: false,
    };
    let password = DomElementRef {
        index: 2,
        target_id: "target".to_owned(),
        backend_node_id: 0,
        node_id: None,
        tag_name: "input".to_owned(),
        role: None,
        name: Some("Password".to_owned()),
        text: None,
        attributes: BTreeMap::from([
            ("placeholder".to_owned(), "Password".to_owned()),
            ("type".to_owned(), "password".to_owned()),
            ("value".to_owned(), "secret".to_owned()),
        ]),
        bounds: None,
        is_visible: true,
        is_interactive: true,
        is_scrollable: false,
    };

    let state = SerializedDomState::from_elements(vec![visible, password]);

    assert!(state.llm_representation().contains(
        "[1] <input type=email id=email value=user@example.com placeholder=name@example.com required=true> Email user@example.com"
    ));
    assert!(
        state
            .llm_representation()
            .contains("[2] <input type=password> Password")
    );
    assert!(!state.llm_representation().contains("secret"));
}

#[test]
fn rendered_attributes_do_not_duplicate_direct_aria_state() {
    let element = DomElementRef {
        index: 1,
        target_id: "target".to_owned(),
        backend_node_id: 0,
        node_id: None,
        tag_name: "button".to_owned(),
        role: None,
        name: Some("Toggle details".to_owned()),
        text: None,
        attributes: BTreeMap::from([
            ("aria-checked".to_owned(), "true".to_owned()),
            ("aria-expanded".to_owned(), "true".to_owned()),
        ]),
        bounds: None,
        is_visible: true,
        is_interactive: true,
        is_scrollable: false,
    };

    let attributes = render_element_attributes(&element);

    assert_eq!(attributes, "aria-expanded=true aria-checked=true");
}

#[test]
fn rendered_attributes_prefer_ax_expanded_over_aria_expanded() {
    let element = DomElementRef {
        index: 1,
        target_id: "target".to_owned(),
        backend_node_id: 0,
        node_id: None,
        tag_name: "button".to_owned(),
        role: None,
        name: Some("Toggle details".to_owned()),
        text: None,
        attributes: BTreeMap::from([
            ("aria-expanded".to_owned(), "false".to_owned()),
            ("expanded".to_owned(), "true".to_owned()),
        ]),
        bounds: None,
        is_visible: true,
        is_interactive: true,
        is_scrollable: false,
    };

    let attributes = render_element_attributes(&element);

    assert_eq!(attributes, "expanded=true");
}

#[test]
fn rendered_attributes_drop_redundant_role_matching_tag_name() {
    let native_role = DomElementRef {
        index: 1,
        target_id: "target".to_owned(),
        backend_node_id: 0,
        node_id: None,
        tag_name: "button".to_owned(),
        role: Some("button".to_owned()),
        name: Some("Submit request".to_owned()),
        text: None,
        attributes: BTreeMap::from([("role".to_owned(), "button".to_owned())]),
        bounds: None,
        is_visible: true,
        is_interactive: true,
        is_scrollable: false,
    };
    let semantic_override = DomElementRef {
        tag_name: "div".to_owned(),
        attributes: BTreeMap::from([("role".to_owned(), "button".to_owned())]),
        ..native_role.clone()
    };

    assert_eq!(render_element_attributes(&native_role), "");
    assert_eq!(render_element_attributes(&semantic_override), "role=button");
}

#[test]
fn rendered_attributes_alias_aria_keyshortcuts_to_ax_shape() {
    let element = DomElementRef {
        index: 1,
        target_id: "target".to_owned(),
        backend_node_id: 0,
        node_id: None,
        tag_name: "div".to_owned(),
        role: None,
        name: Some("Submit request".to_owned()),
        text: None,
        attributes: BTreeMap::from([("aria-keyshortcuts".to_owned(), "Control+Enter".to_owned())]),
        bounds: None,
        is_visible: true,
        is_interactive: true,
        is_scrollable: false,
    };

    let attributes = render_element_attributes(&element);

    assert_eq!(attributes, "keyshortcuts=Control+Enter");
}

#[test]
fn rendered_attributes_alias_aria_live_region_metadata() {
    let element = DomElementRef {
        index: 1,
        target_id: "target".to_owned(),
        backend_node_id: 0,
        node_id: None,
        tag_name: "section".to_owned(),
        role: Some("listbox".to_owned()),
        name: Some("Results".to_owned()),
        text: None,
        attributes: BTreeMap::from([
            ("aria-busy".to_owned(), "true".to_owned()),
            ("aria-level".to_owned(), "2".to_owned()),
            ("aria-live".to_owned(), "polite".to_owned()),
            ("aria-multiselectable".to_owned(), "true".to_owned()),
        ]),
        bounds: None,
        is_visible: true,
        is_interactive: true,
        is_scrollable: false,
    };

    let attributes = render_element_attributes(&element);

    assert_eq!(
        attributes,
        "multiselectable=true level=2 busy=true live=polite"
    );
}

#[test]
fn rendered_attributes_omit_readonly_by_default_but_allow_custom_include() {
    let read_only = DomElementRef {
        index: 1,
        target_id: "target".to_owned(),
        backend_node_id: 0,
        node_id: None,
        tag_name: "input".to_owned(),
        role: None,
        name: Some("Invoice id".to_owned()),
        text: Some("INV-123".to_owned()),
        attributes: BTreeMap::from([("aria-readonly".to_owned(), "true".to_owned())]),
        bounds: None,
        is_visible: true,
        is_interactive: true,
        is_scrollable: false,
    };
    let writable = DomElementRef {
        index: 2,
        target_id: "target".to_owned(),
        backend_node_id: 0,
        node_id: None,
        tag_name: "input".to_owned(),
        role: None,
        name: Some("Notes".to_owned()),
        text: None,
        attributes: BTreeMap::from([("aria-readonly".to_owned(), "false".to_owned())]),
        bounds: None,
        is_visible: true,
        is_interactive: true,
        is_scrollable: false,
    };

    let state = SerializedDomState::from_elements(vec![read_only.clone(), writable.clone()]);

    assert!(
        state
            .llm_representation()
            .contains("[1] <input> Invoice id INV-123")
    );
    assert!(state.llm_representation().contains("[2] <input> Notes"));
    assert_eq!(
        render_element_attributes_with_attributes(&read_only, &["readonly".to_owned()]),
        "readonly=true"
    );
    assert_eq!(
        render_element_attributes_with_attributes(&writable, &["readonly".to_owned()]),
        "readonly=false"
    );
}

#[test]
fn rendered_attributes_keep_false_ax_states_except_required_and_invalid() {
    let element = DomElementRef {
        index: 1,
        target_id: "target".to_owned(),
        backend_node_id: 0,
        node_id: None,
        tag_name: "button".to_owned(),
        role: None,
        name: Some("Toggle filters".to_owned()),
        text: None,
        attributes: BTreeMap::from([
            ("disabled".to_owned(), "false".to_owned()),
            ("expanded".to_owned(), "false".to_owned()),
            ("invalid".to_owned(), "false".to_owned()),
            ("pressed".to_owned(), "false".to_owned()),
            ("required".to_owned(), "false".to_owned()),
            ("selected".to_owned(), "false".to_owned()),
        ]),
        bounds: None,
        is_visible: true,
        is_interactive: true,
        is_scrollable: false,
    };

    assert_eq!(
        render_element_attributes(&element),
        "selected=false expanded=false pressed=false disabled=false"
    );
}

#[test]
fn rendered_attributes_drop_duplicate_long_values() {
    let element = DomElementRef {
        index: 1,
        target_id: "target".to_owned(),
        backend_node_id: 0,
        node_id: None,
        tag_name: "button".to_owned(),
        role: None,
        name: Some("Checkout".to_owned()),
        text: None,
        attributes: BTreeMap::from([
            ("id".to_owned(), "customer-checkout-button".to_owned()),
            ("name".to_owned(), "customer-checkout-button".to_owned()),
            ("value".to_owned(), "customer-checkout-button".to_owned()),
        ]),
        bounds: None,
        is_visible: true,
        is_interactive: true,
        is_scrollable: false,
    };

    let attributes = render_element_attributes(&element);

    assert_eq!(
        attributes,
        "id=customer-checkout-button value=customer-checkout-button"
    );
}

#[test]
fn rendered_attributes_alias_aria_value_metadata() {
    let element = DomElementRef {
        index: 1,
        target_id: "target".to_owned(),
        backend_node_id: 0,
        node_id: None,
        tag_name: "div".to_owned(),
        role: Some("slider".to_owned()),
        name: Some("Volume".to_owned()),
        text: None,
        attributes: BTreeMap::from([
            ("aria-valuemax".to_owned(), "10".to_owned()),
            ("aria-valuemin".to_owned(), "0".to_owned()),
            ("aria-valuenow".to_owned(), "7".to_owned()),
            ("aria-valuetext".to_owned(), "Seven".to_owned()),
        ]),
        bounds: None,
        is_visible: true,
        is_interactive: true,
        is_scrollable: false,
    };

    let attributes = render_element_attributes(&element);

    assert_eq!(
        attributes,
        "valuemin=0 valuemax=10 valuenow=7 valuetext=Seven"
    );
}

#[test]
fn rendered_attributes_add_native_input_format_hints() {
    let date = DomElementRef {
        index: 1,
        target_id: "target".to_owned(),
        backend_node_id: 0,
        node_id: None,
        tag_name: "input".to_owned(),
        role: None,
        name: Some("Start date".to_owned()),
        text: None,
        attributes: BTreeMap::from([
            ("id".to_owned(), "start".to_owned()),
            ("type".to_owned(), "date".to_owned()),
        ]),
        bounds: None,
        is_visible: true,
        is_interactive: true,
        is_scrollable: false,
    };
    let tel = DomElementRef {
        index: 2,
        target_id: "target".to_owned(),
        backend_node_id: 0,
        node_id: None,
        tag_name: "input".to_owned(),
        role: None,
        name: Some("Phone".to_owned()),
        text: None,
        attributes: BTreeMap::from([("type".to_owned(), "tel".to_owned())]),
        bounds: None,
        is_visible: true,
        is_interactive: true,
        is_scrollable: false,
    };

    let state = SerializedDomState::from_elements(vec![date, tel]);

    assert!(state.llm_representation().contains(
        "[1] <input type=date id=start placeholder=YYYY-MM-DD format=YYYY-MM-DD> Start date"
    ));
    assert!(
        state
            .llm_representation()
            .contains("[2] <input type=tel placeholder=123-456-7890> Phone")
    );
}

#[test]
fn rendered_attributes_add_text_datepicker_format_hints() {
    let jquery_datepicker = DomElementRef {
        index: 1,
        target_id: "target".to_owned(),
        backend_node_id: 0,
        node_id: None,
        tag_name: "input".to_owned(),
        role: None,
        name: Some("Travel date".to_owned()),
        text: None,
        attributes: BTreeMap::from([
            ("class".to_owned(), "form-control datepicker".to_owned()),
            ("data-date-format".to_owned(), "dd/mm/yyyy".to_owned()),
            ("type".to_owned(), "text".to_owned()),
        ]),
        bounds: None,
        is_visible: true,
        is_interactive: true,
        is_scrollable: false,
    };
    let angular_datepicker = DomElementRef {
        index: 2,
        target_id: "target".to_owned(),
        backend_node_id: 0,
        node_id: None,
        tag_name: "input".to_owned(),
        role: None,
        name: Some("Start".to_owned()),
        text: None,
        attributes: BTreeMap::from([("uib-datepicker-popup".to_owned(), "MM/dd/yyyy".to_owned())]),
        bounds: None,
        is_visible: true,
        is_interactive: true,
        is_scrollable: false,
    };
    let default_datepicker = DomElementRef {
        index: 3,
        target_id: "target".to_owned(),
        backend_node_id: 0,
        node_id: None,
        tag_name: "input".to_owned(),
        role: None,
        name: Some("Fallback".to_owned()),
        text: None,
        attributes: BTreeMap::from([("data-datepicker".to_owned(), "true".to_owned())]),
        bounds: None,
        is_visible: true,
        is_interactive: true,
        is_scrollable: false,
    };
    let state = SerializedDomState::from_elements(vec![
        jquery_datepicker,
        angular_datepicker,
        default_datepicker,
    ]);

    assert!(
        state
            .llm_representation()
            .contains("[1] <input type=text placeholder=dd/mm/yyyy format=dd/mm/yyyy> Travel date")
    );
    assert!(
        state
            .llm_representation()
            .contains("[2] <input format=MM/dd/yyyy expected_format=MM/dd/yyyy> Start")
    );
    assert!(state.llm_representation().contains(
        "[3] <input placeholder=mm/dd/yyyy data-datepicker=true format=mm/dd/yyyy> Fallback"
    ));
}

#[test]
fn serialized_state_marks_scrollable_elements_for_agent() {
    let element = DomElementRef {
        index: 7,
        target_id: "target".to_owned(),
        backend_node_id: 0,
        node_id: None,
        tag_name: "section".to_owned(),
        role: None,
        name: Some("Results".to_owned()),
        text: None,
        attributes: BTreeMap::from([("id".to_owned(), "results".to_owned())]),
        bounds: None,
        is_visible: true,
        is_interactive: true,
        is_scrollable: true,
    };

    let state = SerializedDomState::from_elements(vec![element]);

    assert_eq!(
        state.llm_representation(),
        "[7] |scroll element| <section id=results> Results"
    );
}

#[test]
fn serialized_state_renders_scroll_context_for_agent() {
    let element = DomElementRef {
        index: 7,
        target_id: "target".to_owned(),
        backend_node_id: 0,
        node_id: None,
        tag_name: "section".to_owned(),
        role: None,
        name: Some("Results".to_owned()),
        text: None,
        attributes: BTreeMap::from([
            ("id".to_owned(), "results".to_owned()),
            (
                "scroll".to_owned(),
                "0.0 pages above, 5.7 pages below".to_owned(),
            ),
        ]),
        bounds: None,
        is_visible: true,
        is_interactive: true,
        is_scrollable: true,
    };

    let state = SerializedDomState::from_elements(vec![element]);

    assert_eq!(
        state.llm_representation(),
        "[7] |scroll element| <section id=results> (0.0 pages above, 5.7 pages below) Results"
    );
}

#[test]
fn ax_description_requires_explicit_include_attribute() {
    let element = DomElementRef {
        index: 1,
        target_id: "target".to_owned(),
        backend_node_id: 0,
        node_id: None,
        tag_name: "button".to_owned(),
        role: None,
        name: Some("Submit".to_owned()),
        text: None,
        attributes: BTreeMap::from([(
            "description".to_owned(),
            "Sends the completed form".to_owned(),
        )]),
        bounds: None,
        is_visible: true,
        is_interactive: true,
        is_scrollable: false,
    };

    assert_eq!(render_element_attributes(&element), "");
    assert_eq!(
        render_element_attributes_with_attributes(&element, &["description".to_owned()]),
        "description=Sends the completed form"
    );
}

#[test]
fn rendered_attributes_include_distinct_accessibility_snapshot_metadata() {
    let element = DomElementRef {
        index: 1,
        target_id: "target".to_owned(),
        backend_node_id: 0,
        node_id: None,
        tag_name: "button".to_owned(),
        role: None,
        name: Some("Submit".to_owned()),
        text: None,
        attributes: BTreeMap::from([
            ("ax_name".to_owned(), "Submit".to_owned()),
            (
                "ax_description".to_owned(),
                "Sends the completed checkout form".to_owned(),
            ),
        ]),
        bounds: None,
        is_visible: true,
        is_interactive: true,
        is_scrollable: false,
    };

    assert_eq!(
        render_element_attributes(&element),
        "ax_description=Sends the completed checkout form"
    );
}

#[test]
fn llm_representation_can_use_custom_include_attributes() {
    let element = DomElementRef {
        index: 1,
        target_id: "target".to_owned(),
        backend_node_id: 0,
        node_id: None,
        tag_name: "button".to_owned(),
        role: None,
        name: Some("Run".to_owned()),
        text: None,
        attributes: BTreeMap::from([
            ("data-testid".to_owned(), "run-action".to_owned()),
            ("id".to_owned(), "run".to_owned()),
        ]),
        bounds: None,
        is_visible: true,
        is_interactive: true,
        is_scrollable: false,
    };
    let state = SerializedDomState::from_elements(vec![element]);

    assert_eq!(state.llm_representation(), "[1] <button id=run> Run");
    assert_eq!(
        state.llm_representation_with_attributes(&["data-testid".to_owned()]),
        "[1] <button data-testid=run-action> Run"
    );
}
