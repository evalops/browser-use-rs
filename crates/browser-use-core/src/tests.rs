use super::*;
use browser_use_cdp::{FoundElement, Pdf, Screenshot};
use browser_use_dom::SerializedDomState;
use browser_use_tools::{
    ClickElementAction, CloseTabAction, DoneAction, EvaluateAction, ExtractAction,
    FindElementsAction, FindTextAction, GetDropdownOptionsAction, InputTextAction, NavigateAction,
    NoParamsAction, ReadFileAction, ReplaceFileAction, SaveAsPdfAction, ScreenshotAction,
    ScrollAction, SearchPageAction, SelectDropdownOptionAction, SendKeysAction, SwitchTabAction,
    UploadFileAction, WaitAction, WriteFileAction,
};
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    io::Write,
    sync::{Arc, Mutex},
};

static CWD_LOCK: Mutex<()> = Mutex::new(());

struct CurrentDirGuard {
    original: std::path::PathBuf,
}

impl CurrentDirGuard {
    fn enter(path: &std::path::Path) -> Self {
        let original = std::env::current_dir().expect("current dir");
        std::env::set_current_dir(path).expect("set current dir");
        Self { original }
    }
}

impl Drop for CurrentDirGuard {
    fn drop(&mut self) {
        std::env::set_current_dir(&self.original).expect("restore current dir");
    }
}

#[test]
fn target_commit_is_pinned() {
    assert_eq!(
        INITIAL_UPSTREAM_COMMIT,
        "157779338afdcc03023010ec3c24ad63d820453c"
    );
}

#[test]
fn settings_defaults_match_browser_use_shape() {
    let settings = AgentSettings::default();

    assert_eq!(settings.use_vision, VisionMode::Always);
    assert_eq!(settings.vision_detail_level, ImageDetailLevel::Auto);
    assert_eq!(settings.llm_screenshot_size, None);
    assert_eq!(settings.url_shortening_limit, Some(25));
    assert_eq!(settings.max_failures, 5);
    assert_eq!(settings.generate_gif, GenerateGif::Disabled);
    assert_eq!(settings.max_actions_per_step, 5);
    assert_eq!(settings.llm_timeout_seconds, 60);
    assert_eq!(settings.step_timeout_seconds, 180);
    assert_eq!(
        settings.action_timeout_seconds,
        default_action_timeout_seconds()
    );
    assert_eq!(settings.wait_between_actions_seconds, 0.1);
    assert!(settings.directly_open_url);
    assert!(settings.final_response_after_failure);
    assert!(settings.display_files_in_done_text);
    assert_eq!(settings.loop_detection_window, 20);
    assert!(settings.loop_detection_enabled);
    assert_eq!(settings.max_history_items, None);
    assert_eq!(settings.max_clickable_elements_length, 40_000);
    assert!(!settings.include_recent_events);
    assert!(settings.sample_images.is_empty());
    assert!(settings.enable_planning);
    assert_eq!(settings.planning_replan_on_stall, 3);
    assert_eq!(settings.planning_exploration_limit, 5);
    assert!(settings.use_thinking);
    assert!(!settings.flash_mode);
    assert!(settings.use_judge);
    assert_eq!(settings.ground_truth, None);
    assert_eq!(settings.extraction_schema, None);
    assert_eq!(settings.message_compaction, MessageCompaction::Enabled);
    assert!(!settings.calculate_cost);
    assert!(!settings.include_tool_call_examples);
    assert_eq!(settings.save_conversation_path, None);
    assert_eq!(
        settings.save_conversation_path_encoding.as_deref(),
        Some("utf-8")
    );
    assert_eq!(settings.file_system_path, None);
    assert!(settings.include_attributes.is_empty());
    assert!(settings.available_file_paths.is_empty());
    assert!(settings.initial_actions.is_empty());
    assert!(settings.excluded_actions.is_empty());
    assert!(settings.sensitive_data.is_empty());
    assert_eq!(settings.override_system_message, None);
    assert_eq!(settings.extend_system_message, None);
}

#[test]
fn url_shortening_matches_upstream_query_hash_shape() {
    let short_url = "https://example.test/path?q=short";
    let long_url = "https://example.test/path?abcdefghijklmnopqrstuvwxyz0123456789";

    let (rewritten, replacements) =
        shorten_urls_in_text(&format!("Open {short_url} then {long_url}"), 10);

    let shortened = "https://example.test/path?abcdefghi...0cd4b05";
    assert!(rewritten.contains(short_url));
    assert!(rewritten.contains(shortened));
    assert!(!rewritten.contains(long_url));
    assert_eq!(
        replacements.get(shortened).map(String::as_str),
        Some(long_url)
    );
}

#[test]
fn url_shortening_rewrites_only_user_and_assistant_text_parts() {
    let long_url = "https://example.test/path?abcdefghijklmnopqrstuvwxyz0123456789";
    let request = ChatRequest {
        messages: vec![
            ChatMessage::text(MessageRole::System, format!("System keeps {long_url}")),
            ChatMessage {
                role: MessageRole::User,
                content: vec![
                    ContentPart::Text {
                        text: format!("User opens {long_url}"),
                    },
                    ContentPart::ImageUrl {
                        image_url: long_url.to_owned(),
                        detail: None,
                    },
                ],
            },
            ChatMessage::text(MessageRole::Assistant, format!("Assistant saw {long_url}")),
        ],
        output_schema: None,
    };

    let (request, replacements) = request_with_shortened_urls(request, Some(10));
    let shortened = "https://example.test/path?abcdefghi...0cd4b05";

    assert_eq!(
        replacements.get(shortened).map(String::as_str),
        Some(long_url)
    );
    let system_text = match &request.messages[0].content[0] {
        ContentPart::Text { text } => text,
        other => panic!("unexpected system content: {other:?}"),
    };
    let user_text = match &request.messages[1].content[0] {
        ContentPart::Text { text } => text,
        other => panic!("unexpected user content: {other:?}"),
    };
    let user_image = match &request.messages[1].content[1] {
        ContentPart::ImageUrl { image_url, .. } => image_url,
        other => panic!("unexpected user image content: {other:?}"),
    };
    let assistant_text = match &request.messages[2].content[0] {
        ContentPart::Text { text } => text,
        other => panic!("unexpected assistant content: {other:?}"),
    };

    assert!(system_text.contains(long_url));
    assert!(user_text.contains(shortened));
    assert_eq!(user_image, long_url);
    assert!(assistant_text.contains(shortened));

    let (disabled_request, disabled_replacements) =
        request_with_shortened_urls(request.clone(), None);
    assert!(disabled_replacements.is_empty());
    assert_eq!(disabled_request, request);
}

#[test]
fn llm_screenshot_size_validates_and_deserializes_wire_shapes() {
    assert_eq!(
        LlmScreenshotSize::new(1400, 850).expect("valid size"),
        LlmScreenshotSize {
            width: 1400,
            height: 850
        }
    );
    assert!(LlmScreenshotSize::new(99, 850).is_err());
    assert!(LlmScreenshotSize::new(1400, 99).is_err());

    let tuple_settings: AgentSettings =
        serde_json::from_value(serde_json::json!({"llm_screenshot_size": [1400, 850]}))
            .expect("tuple screenshot size");
    assert_eq!(
        tuple_settings.llm_screenshot_size,
        Some(LlmScreenshotSize::new(1400, 850).expect("valid tuple size"))
    );

    let object_settings: AgentSettings = serde_json::from_value(serde_json::json!({
        "llm_screenshot_size": {"width": 1200, "height": 900}
    }))
    .expect("object screenshot size");
    assert_eq!(
        object_settings.llm_screenshot_size,
        Some(LlmScreenshotSize::new(1200, 900).expect("valid object size"))
    );

    let error = serde_json::from_value::<AgentSettings>(
        serde_json::json!({"llm_screenshot_size": [80, 850]}),
    )
    .expect_err("invalid screenshot size");
    assert!(error.to_string().contains("at least 100"));
}

#[test]
fn action_timeout_parser_accepts_only_finite_positive_seconds() {
    assert_eq!(parse_action_timeout_seconds(None), 180.0);
    assert_eq!(parse_action_timeout_seconds(Some("")), 180.0);
    assert_eq!(parse_action_timeout_seconds(Some("garbage")), 180.0);
    assert_eq!(parse_action_timeout_seconds(Some("nan")), 180.0);
    assert_eq!(parse_action_timeout_seconds(Some("inf")), 180.0);
    assert_eq!(parse_action_timeout_seconds(Some("-1")), 180.0);
    assert_eq!(parse_action_timeout_seconds(Some("0")), 180.0);
    assert_eq!(parse_action_timeout_seconds(Some("12.5")), 12.5);
}

#[test]
fn wait_between_actions_coerces_invalid_values_but_preserves_zero() {
    assert_eq!(coerce_valid_wait_between_actions_seconds(0.0), 0.0);
    assert_eq!(coerce_valid_wait_between_actions_seconds(0.25), 0.25);
    assert_eq!(
        coerce_valid_wait_between_actions_seconds(-1.0),
        default_wait_between_actions_seconds()
    );
    assert_eq!(
        coerce_valid_wait_between_actions_seconds(f64::NAN),
        default_wait_between_actions_seconds()
    );
    assert_eq!(
        coerce_valid_wait_between_actions_seconds(f64::INFINITY),
        default_wait_between_actions_seconds()
    );
}

#[test]
fn generate_gif_preserves_upstream_json_shape() {
    assert_eq!(
        serde_json::to_value(GenerateGif::Disabled).expect("serialize disabled"),
        serde_json::json!(false)
    );
    assert_eq!(
        serde_json::to_value(GenerateGif::Enabled).expect("serialize enabled"),
        serde_json::json!(true)
    );
    assert_eq!(
        serde_json::to_value(GenerateGif::Path("trace.gif".to_owned())).expect("serialize path"),
        serde_json::json!("trace.gif")
    );

    assert_eq!(
        serde_json::from_value::<GenerateGif>(serde_json::json!(false)).expect("deserialize false"),
        GenerateGif::Disabled
    );
    assert_eq!(
        serde_json::from_value::<GenerateGif>(serde_json::json!(true)).expect("deserialize true"),
        GenerateGif::Enabled
    );
    assert_eq!(
        serde_json::from_value::<GenerateGif>(serde_json::json!("trace.gif"))
            .expect("deserialize path"),
        GenerateGif::Path("trace.gif".to_owned())
    );
}

#[test]
fn message_compaction_preserves_upstream_json_shape() {
    assert_eq!(
        serde_json::to_value(MessageCompaction::Disabled).expect("serialize disabled"),
        serde_json::json!(false)
    );
    assert_eq!(
        serde_json::to_value(MessageCompaction::Enabled).expect("serialize enabled"),
        serde_json::json!(true)
    );

    let settings = MessageCompactionSettings {
        compact_every_n_steps: 3,
        trigger_char_count: Some(1200),
        keep_last_items: 2,
        include_read_state: true,
        ..MessageCompactionSettings::default()
    };
    let serialized = serde_json::to_value(MessageCompaction::Settings(settings.clone()))
        .expect("serialize settings");
    assert_eq!(serialized["compact_every_n_steps"], 3);
    assert_eq!(serialized["trigger_char_count"], 1200);
    assert_eq!(serialized["keep_last_items"], 2);
    assert_eq!(serialized["include_read_state"], true);

    assert_eq!(
        serde_json::from_value::<MessageCompaction>(serde_json::json!(false))
            .expect("deserialize false"),
        MessageCompaction::Disabled
    );
    assert_eq!(
        serde_json::from_value::<MessageCompaction>(serde_json::json!(true))
            .expect("deserialize true"),
        MessageCompaction::Enabled
    );
    assert_eq!(
        serde_json::from_value::<MessageCompaction>(serde_json::Value::Null)
            .expect("deserialize null"),
        MessageCompaction::Disabled
    );
    assert_eq!(
        serde_json::from_value::<MessageCompaction>(serde_json::json!({
            "compact_every_n_steps": 3,
            "trigger_char_count": 1200,
            "keep_last_items": 2,
            "include_read_state": true
        }))
        .expect("deserialize settings"),
        MessageCompaction::Settings(settings)
    );

    let token_threshold = serde_json::from_value::<MessageCompactionSettings>(serde_json::json!({
        "trigger_token_count": 250,
        "chars_per_token": 3.5
    }))
    .expect("deserialize token threshold");
    assert_eq!(token_threshold.trigger_token_count, Some(250));
    assert_eq!(token_threshold.trigger_char_count, Some(875));

    let both_thresholds = serde_json::from_value::<MessageCompactionSettings>(serde_json::json!({
        "trigger_char_count": 100,
        "trigger_token_count": 25
    }));
    assert!(both_thresholds.is_err());
}

#[test]
fn vision_mode_preserves_upstream_json_shape() {
    assert_eq!(
        serde_json::to_value(VisionMode::Always).expect("serialize always"),
        serde_json::json!(true)
    );
    assert_eq!(
        serde_json::to_value(VisionMode::Never).expect("serialize never"),
        serde_json::json!(false)
    );
    assert_eq!(
        serde_json::to_value(VisionMode::Auto).expect("serialize auto"),
        serde_json::json!("auto")
    );

    assert_eq!(
        serde_json::from_value::<VisionMode>(serde_json::json!(true)).expect("deserialize true"),
        VisionMode::Always
    );
    assert_eq!(
        serde_json::from_value::<VisionMode>(serde_json::json!(false)).expect("deserialize false"),
        VisionMode::Never
    );
    assert_eq!(
        serde_json::from_value::<VisionMode>(serde_json::json!("auto")).expect("deserialize auto"),
        VisionMode::Auto
    );
}

#[test]
fn action_result_rejects_success_true_without_done() {
    let error = serde_json::from_value::<ActionResult>(serde_json::json!({
        "extracted_content": "clicked",
        "success": true
    }))
    .expect_err("success=true without done should be rejected");

    assert!(error.to_string().contains("is_done=true"));
}

#[test]
fn action_result_accepts_failed_non_done_status() {
    let result: ActionResult = serde_json::from_value(serde_json::json!({
        "error": "click failed",
        "success": false
    }))
    .expect("success=false remains valid for failed actions");

    assert!(!result.is_done);
    assert_eq!(result.success, Some(false));
}

#[test]
fn action_result_accepts_successful_done_status() {
    let result: ActionResult = serde_json::from_value(serde_json::json!({
        "extracted_content": "complete",
        "is_done": true,
        "success": true
    }))
    .expect("done success should deserialize");

    assert!(result.is_done);
    assert_eq!(result.success, Some(true));
}

#[test]
fn action_result_deserializes_trace_judgement() {
    let result: ActionResult = serde_json::from_value(serde_json::json!({
        "judgement": {
            "reasoning": "visual state matched expected result",
            "verdict": true,
            "failure_reason": null,
            "impossible_task": false,
            "reached_captcha": false
        }
    }))
    .expect("judgement should deserialize");

    let judgement = result.judgement.expect("judgement");
    assert!(judgement.verdict);
    assert_eq!(
        judgement.reasoning.as_deref(),
        Some("visual state matched expected result")
    );
}

#[test]
fn action_result_deserializes_image_payloads() {
    let result: ActionResult = serde_json::from_value(serde_json::json!({
        "extracted_content": "Read image file chart.png.",
        "images": [
            {
                "name": "chart.png",
                "data": "abc123"
            }
        ]
    }))
    .expect("image payloads should deserialize");

    assert_eq!(result.images.len(), 1);
    assert_eq!(result.images[0]["name"], "chart.png");
    assert_eq!(result.images[0]["data"], "abc123");
}

#[test]
fn agent_output_accepts_flattened_planning_shape() {
    let output: AgentOutput = serde_json::from_value(serde_json::json!({
        "thinking": "Need a plan",
        "evaluation_previous_goal": "No previous step",
        "memory": "Need to search",
        "next_goal": "Open search",
        "current_plan_item": 1,
        "plan_update": ["Find search box", "Submit query"],
        "action": [
            {
                "done": {
                    "text": "planned",
                    "success": true,
                    "files_to_display": []
                }
            }
        ]
    }))
    .expect("flattened agent output");

    assert!(output.current_state.is_empty());
    assert_eq!(output.current_plan_item, Some(1));
    assert_eq!(
        output.plan_update.as_ref().expect("plan update"),
        &vec!["Find search box".to_owned(), "Submit query".to_owned()]
    );
    let brain = output.current_brain();
    assert_eq!(brain.thinking.as_deref(), Some("Need a plan"));
    assert_eq!(
        brain.evaluation_previous_goal.as_deref(),
        Some("No previous step")
    );
    assert_eq!(brain.memory.as_deref(), Some("Need to search"));
    assert_eq!(brain.next_goal.as_deref(), Some("Open search"));

    let serialized = serde_json::to_value(&output).expect("serialize output");
    assert!(serialized.get("current_state").is_none());
    assert_eq!(serialized["current_plan_item"], 1);
    assert_eq!(serialized["plan_update"][0], "Find search box");
}

fn schema_required_fields(schema: &Value) -> Vec<&str> {
    schema["required"]
        .as_array()
        .expect("required fields")
        .iter()
        .map(|field| field.as_str().expect("required field string"))
        .collect()
}

fn schema_action_names(schema: &Value) -> BTreeSet<String> {
    for pointer in [
        "/$defs/BrowserAction/oneOf",
        "/$defs/BrowserAction/anyOf",
        "/definitions/BrowserAction/oneOf",
        "/definitions/BrowserAction/anyOf",
    ] {
        if let Some(actions) = schema.pointer(pointer).and_then(Value::as_array) {
            return actions
                .iter()
                .filter_map(schema_variant_action_name)
                .map(ToOwned::to_owned)
                .collect();
        }
    }
    BTreeSet::new()
}

#[test]
fn agent_output_schema_exposes_planning_fields() {
    let schema = schema_for_agent_output();
    let schema_text = serde_json::to_string(&schema).expect("schema text");

    assert!(schema_text.contains("current_plan_item"));
    assert!(schema_text.contains("plan_update"));
    assert!(schema_text.contains("evaluation_previous_goal"));
}

#[test]
fn agent_output_schema_filters_excluded_actions() {
    let settings = AgentSettings {
        excluded_actions: vec![
            "search".to_owned(),
            "scroll".to_owned(),
            "switch-tab".to_owned(),
            "done".to_owned(),
        ],
        ..AgentSettings::default()
    };
    let schema = schema_for_agent_output_with_settings(&settings);
    let action_names = schema_action_names(&schema);

    assert!(!action_names.contains("search"));
    assert!(!action_names.contains("scroll"));
    assert!(!action_names.contains("switch_tab"));
    assert!(action_names.contains("navigate"));
    assert!(action_names.contains("done"));
}

#[test]
fn agent_output_schema_gates_screenshot_action_on_auto_vision() {
    let default_schema = schema_for_agent_output_with_settings(&AgentSettings::default());
    let default_actions = schema_action_names(&default_schema);
    assert!(!default_actions.contains("screenshot"));

    let auto_settings = AgentSettings {
        use_vision: VisionMode::Auto,
        ..AgentSettings::default()
    };
    let auto_schema = schema_for_agent_output_with_settings(&auto_settings);
    let auto_actions = schema_action_names(&auto_schema);
    assert!(auto_actions.contains("screenshot"));

    let disabled_settings = AgentSettings {
        use_vision: VisionMode::Never,
        ..AgentSettings::default()
    };
    let disabled_schema = schema_for_agent_output_with_settings(&disabled_settings);
    let disabled_actions = schema_action_names(&disabled_schema);
    assert!(!disabled_actions.contains("screenshot"));
}

#[test]
fn final_response_schema_keeps_done_when_actions_are_excluded() {
    let settings = AgentSettings {
        excluded_actions: vec!["search".to_owned(), "done".to_owned()],
        ..AgentSettings::default()
    };
    let schema = schema_for_final_response_after_failure(&settings);
    let action_names = schema_action_names(&schema);

    assert_eq!(action_names, BTreeSet::from(["done".to_owned()]));
}

#[test]
fn step_limit_final_response_request_uses_done_only_schema_and_instruction() {
    let request = build_final_response_after_step_limit_request(
        "finish the task",
        &blank_state(),
        &AgentHistory::default(),
        &AgentSettings::default(),
        None,
        3,
    )
    .expect("step limit final response request");
    let action_names = schema_action_names(request.output_schema.as_ref().expect("output schema"));

    assert_eq!(action_names, BTreeSet::from(["done".to_owned()]));
    let text = request_text(&request);
    assert!(text.contains("You reached max_steps (3)"));
    assert!(text.contains("only available action is done"));
    assert!(text.contains("set success to false"));
}

#[test]
fn step_budget_warning_request_matches_upstream_threshold_text() {
    assert!(!should_inject_step_budget_warning(2, 4));
    assert!(should_inject_step_budget_warning(3, 4));
    assert!(!should_inject_step_budget_warning(4, 4));

    let request = build_step_request_with_budget_warning(
        "finish the task",
        &blank_state(),
        &AgentHistory::default(),
        &AgentSettings::default(),
        None,
        3,
        4,
    )
    .expect("budget warning request");
    let action_names = schema_action_names(request.output_schema.as_ref().expect("output schema"));

    assert!(action_names.contains("click"));
    assert!(action_names.contains("done"));
    let text = request_text(&request);
    assert!(text.contains("BUDGET WARNING: You have used 3/4 steps (75%). 1 steps remaining."));
    assert!(text.contains("Partial results are far more valuable"));
}

#[test]
fn agent_output_schema_honors_thinking_and_flash_settings() {
    let no_thinking = AgentSettings {
        use_thinking: false,
        ..AgentSettings::default()
    };
    let schema = schema_for_agent_output_with_settings(&no_thinking);
    let properties = schema["properties"].as_object().expect("properties");
    assert!(!properties.contains_key("thinking"));
    assert!(!properties.contains_key("current_state"));
    assert!(properties.contains_key("current_plan_item"));
    assert_eq!(
        schema_required_fields(&schema),
        vec!["evaluation_previous_goal", "memory", "next_goal", "action"]
    );

    let flash = AgentSettings {
        flash_mode: true,
        ..AgentSettings::default()
    };
    let schema = schema_for_agent_output_with_settings(&flash);
    let properties = schema["properties"].as_object().expect("properties");
    assert!(!properties.contains_key("thinking"));
    assert!(!properties.contains_key("current_state"));
    assert!(!properties.contains_key("evaluation_previous_goal"));
    assert!(!properties.contains_key("next_goal"));
    assert!(!properties.contains_key("current_plan_item"));
    assert!(!properties.contains_key("plan_update"));
    assert!(properties.contains_key("memory"));
    assert!(properties.contains_key("action"));
    assert_eq!(schema_required_fields(&schema), vec!["memory", "action"]);
}

#[test]
fn agent_output_schema_matches_upstream_required_fields() {
    let schema = schema_for_agent_output_with_settings(&AgentSettings::default());
    let properties = schema["properties"].as_object().expect("properties");

    assert!(!properties.contains_key("current_state"));
    assert_eq!(
        schema_required_fields(&schema),
        vec!["evaluation_previous_goal", "memory", "next_goal", "action"]
    );
    assert_eq!(schema["properties"]["action"]["minItems"], 1);
}

#[test]
fn final_response_after_failure_schema_allows_only_done_action() {
    let schema = schema_for_final_response_after_failure(&AgentSettings::default());
    let variants = schema
        .pointer("/$defs/BrowserAction/oneOf")
        .or_else(|| schema.pointer("/$defs/BrowserAction/anyOf"))
        .or_else(|| schema.pointer("/definitions/BrowserAction/oneOf"))
        .or_else(|| schema.pointer("/definitions/BrowserAction/anyOf"))
        .expect("browser action variants")
        .as_array()
        .expect("variant array");

    assert_eq!(variants.len(), 1);
    let properties = variants[0]
        .get("properties")
        .and_then(Value::as_object)
        .expect("done action properties");
    assert_eq!(properties.len(), 1);
    assert!(properties.contains_key("done"));
}

#[test]
fn history_returns_latest_done_result() {
    let history = AgentHistory {
        items: vec![AgentHistoryItem {
            model_output: None,
            result: vec![ActionResult::done("finished", true)],
            state: blank_state(),
            metadata: None,
        }],
        ..AgentHistory::default()
    };

    assert_eq!(history.final_result(), Some("finished"));
    assert!(history.is_done());
    assert_eq!(history.is_successful(), Some(true));
    assert_eq!(history.errors(), vec![None]);
    assert!(!history.has_errors());
}

#[test]
fn history_collects_errors_and_failed_done_status() {
    let history = AgentHistory {
        items: vec![
            AgentHistoryItem {
                model_output: None,
                result: vec![ActionResult::error("first failure")],
                state: blank_state(),
                metadata: None,
            },
            AgentHistoryItem {
                model_output: None,
                result: vec![ActionResult::done("could not finish", false)],
                state: blank_state(),
                metadata: None,
            },
        ],
        ..AgentHistory::default()
    };

    assert_eq!(history.final_result(), Some("could not finish"));
    assert!(history.is_done());
    assert_eq!(history.is_successful(), Some(false));
    assert_eq!(history.errors(), vec![Some("first failure"), None]);
    assert!(history.has_errors());
}

#[test]
fn history_judgement_helpers_use_last_result_like_browser_use() {
    let judgement = JudgementResult {
        reasoning: Some("trace satisfied the task".to_owned()),
        verdict: true,
        failure_reason: Some(String::new()),
        impossible_task: false,
        reached_captcha: false,
    };
    let history = AgentHistory {
        items: vec![AgentHistoryItem {
            model_output: None,
            result: vec![ActionResult {
                extracted_content: Some("judged".to_owned()),
                error: None,
                judgement: Some(judgement.clone()),
                long_term_memory: None,
                include_extracted_content_only_once: false,
                include_in_memory: false,
                is_done: true,
                success: Some(true),
                attachments: Vec::new(),
                images: Vec::new(),
                metadata: None,
            }],
            state: blank_state(),
            metadata: None,
        }],
        ..AgentHistory::default()
    };

    assert_eq!(history.judgement(), Some(&judgement));
    assert!(history.is_judged());
    assert_eq!(history.is_validated(), Some(true));

    let serialized = serde_json::to_value(&history).expect("serialize history");
    assert_eq!(
        serialized["items"][0]["result"][0]["judgement"]["verdict"],
        true
    );
}

#[test]
fn history_judgement_helpers_ignore_prior_non_terminal_judgement() {
    let history = AgentHistory {
        items: vec![
            AgentHistoryItem {
                model_output: None,
                result: vec![ActionResult {
                    extracted_content: Some("judged earlier".to_owned()),
                    error: None,
                    judgement: Some(JudgementResult {
                        reasoning: None,
                        verdict: false,
                        failure_reason: Some("missing final evidence".to_owned()),
                        impossible_task: false,
                        reached_captcha: false,
                    }),
                    long_term_memory: None,
                    include_extracted_content_only_once: false,
                    include_in_memory: false,
                    is_done: false,
                    success: None,
                    attachments: Vec::new(),
                    images: Vec::new(),
                    metadata: None,
                }],
                state: blank_state(),
                metadata: None,
            },
            AgentHistoryItem {
                model_output: None,
                result: vec![ActionResult::extracted("latest result")],
                state: blank_state(),
                metadata: None,
            },
        ],
        ..AgentHistory::default()
    };

    assert_eq!(history.judgement(), None);
    assert!(!history.is_judged());
    assert_eq!(history.is_validated(), None);
}

#[test]
fn history_completion_helpers_use_last_result_like_browser_use() {
    let history = AgentHistory {
        items: vec![
            AgentHistoryItem {
                model_output: None,
                result: vec![ActionResult::done("finished earlier", true)],
                state: blank_state(),
                metadata: None,
            },
            AgentHistoryItem {
                model_output: None,
                result: vec![ActionResult::extracted("latest non-terminal result")],
                state: blank_state(),
                metadata: None,
            },
        ],
        ..AgentHistory::default()
    };

    assert_eq!(history.final_result(), Some("latest non-terminal result"));
    assert!(!history.is_done());
    assert_eq!(history.is_successful(), None);
}

#[test]
fn step_metadata_matches_browser_use_shape() {
    let metadata = StepMetadata {
        step_number: 2,
        step_start_time: 10.0,
        step_end_time: 13.5,
        step_interval: Some(2.0),
    };

    assert_eq!(metadata.duration_seconds(), 3.5);

    let serialized = serde_json::to_value(&metadata).expect("serialize metadata");
    assert_eq!(serialized["step_number"], 2);
    assert_eq!(serialized["step_start_time"], 10.0);
    assert_eq!(serialized["step_end_time"], 13.5);
    assert_eq!(serialized["step_interval"], 2.0);

    let without_interval: StepMetadata = serde_json::from_value(serde_json::json!({
        "step_number": 1,
        "step_start_time": 1.0,
        "step_end_time": 2.0
    }))
    .expect("deserialize old metadata");
    assert_eq!(without_interval.step_interval, None);

    let serialized_without_interval =
        serde_json::to_value(&without_interval).expect("serialize metadata");
    assert!(serialized_without_interval.get("step_interval").is_some());
    assert_eq!(serialized_without_interval["step_interval"], Value::Null);
}

#[test]
fn history_helpers_match_browser_use_accessors() {
    let mut first_state = blank_state();
    first_state.url = "https://example.test/start".to_owned();
    let mut second_state = blank_state();
    second_state.url = "https://example.test/done".to_owned();
    let current_state = AgentCurrentState {
        thinking: None,
        evaluation_previous_goal: None,
        memory: None,
        next_goal: None,
    };
    let history = AgentHistory {
        items: vec![
            AgentHistoryItem {
                model_output: Some(AgentOutput {
                    current_state: current_state.clone(),
                    thinking: None,
                    evaluation_previous_goal: None,
                    memory: None,
                    next_goal: None,
                    current_plan_item: None,
                    plan_update: None,
                    action: vec![BrowserAction::Click(ClickElementAction {
                        index: Some(1),
                        coordinate_x: None,
                        coordinate_y: None,
                    })],
                }),
                result: vec![ActionResult::extracted("Clicked element 1")],
                state: first_state,
                metadata: Some(StepMetadata {
                    step_number: 1,
                    step_start_time: 10.0,
                    step_end_time: 11.5,
                    step_interval: None,
                }),
            },
            AgentHistoryItem {
                model_output: Some(AgentOutput {
                    current_state,
                    thinking: None,
                    evaluation_previous_goal: None,
                    memory: None,
                    next_goal: None,
                    current_plan_item: None,
                    plan_update: None,
                    action: vec![BrowserAction::Done(DoneAction {
                        text: "finished".to_owned(),
                        success: true,
                        files_to_display: Vec::new(),
                    })],
                }),
                result: vec![ActionResult::done("finished", true)],
                state: second_state,
                metadata: Some(StepMetadata {
                    step_number: 2,
                    step_start_time: 20.0,
                    step_end_time: 22.0,
                    step_interval: Some(1.5),
                }),
            },
        ],
        ..AgentHistory::default()
    };

    assert_eq!(history.number_of_steps(), 2);
    assert_eq!(history.action_results().len(), 2);
    assert_eq!(
        history.extracted_content(),
        vec!["Clicked element 1", "finished"]
    );
    assert_eq!(
        history.urls(),
        vec!["https://example.test/start", "https://example.test/done"]
    );
    assert_eq!(history.action_names(), vec!["click", "done"]);
    assert_eq!(history.model_outputs().len(), 2);
    assert_eq!(history.model_thoughts().len(), 2);
    assert_eq!(history.model_actions().len(), 2);
    assert_eq!(history.model_actions_filtered(&["click"]).len(), 1);
    assert_eq!(
        history.action_history(),
        vec![
            vec![serde_json::json!({
                "click": {
                    "index": 1
                },
                "interacted_element": null,
                "result": "Clicked element 1"
            })],
            vec![serde_json::json!({
                "done": {
                    "text": "finished",
                    "success": true,
                    "files_to_display": []
                },
                "interacted_element": null,
                "result": null
            })],
        ]
    );
    assert_eq!(
        history.last_action().expect("last action"),
        serde_json::json!({
            "done": {
                "text": "finished",
                "success": true,
                "files_to_display": []
            }
        })
    );
    assert!((history.total_duration_seconds() - 3.5).abs() < f64::EPSILON);
}

#[test]
fn action_history_includes_interacted_element_for_indexed_actions() {
    let element = browser_use_dom::DomElementRef {
        index: 1,
        target_id: "target-123".to_owned(),
        backend_node_id: 44,
        node_id: Some(9),
        tag_name: "input".to_owned(),
        role: Some("textbox".to_owned()),
        name: Some("Email".to_owned()),
        text: Some("ada@example.test".to_owned()),
        attributes: BTreeMap::from([
            ("id".to_owned(), "email".to_owned()),
            ("type".to_owned(), "email".to_owned()),
            ("ax_name".to_owned(), "Email".to_owned()),
        ]),
        bounds: Some(browser_use_dom::ElementBounds {
            x: 5,
            y: 10,
            width: 200,
            height: 30,
        }),
        is_visible: true,
        is_interactive: true,
        is_scrollable: false,
    };
    let mut state = blank_state();
    state.dom_state = browser_use_dom::SerializedDomState::from_elements(vec![element.clone()]);
    let history = AgentHistory {
        items: vec![AgentHistoryItem {
            model_output: Some(AgentOutput {
                current_state: AgentCurrentState::default(),
                thinking: None,
                evaluation_previous_goal: None,
                memory: None,
                next_goal: None,
                current_plan_item: None,
                plan_update: None,
                action: vec![
                    BrowserAction::Input(InputTextAction {
                        index: 1,
                        text: "ada@example.test".to_owned(),
                        clear: true,
                    }),
                    BrowserAction::Wait(WaitAction { seconds: 0 }),
                ],
            }),
            result: vec![
                ActionResult::extracted("Typed into element 1"),
                ActionResult::extracted("Paused"),
            ],
            state,
            metadata: None,
        }],
        ..AgentHistory::default()
    };

    let action_history = history.action_history();
    assert_eq!(
        action_history[0][0]["interacted_element"],
        serde_json::to_value(DomInteractedElement::from_element(&element))
            .expect("interacted element")
    );
    assert_eq!(action_history[0][0]["result"], "Typed into element 1");
    assert_eq!(action_history[0][1]["interacted_element"], Value::Null);
    assert_eq!(action_history[0][1]["result"], "Paused");

    let model_actions = history.model_actions();
    assert_eq!(
        model_actions[0]["interacted_element"],
        serde_json::to_value(DomInteractedElement::from_element(&element))
            .expect("interacted element")
    );
    assert_eq!(model_actions[1]["interacted_element"], Value::Null);
}

#[test]
fn replay_rematch_updates_supported_indexed_actions() {
    let historical_element = replay_dom_element(
        1,
        "input",
        BTreeMap::from([("id".to_owned(), "email".to_owned())]),
    );
    let interacted = DomInteractedElement::from_element(&historical_element);
    let current_dom = SerializedDomState::from_elements(vec![replay_dom_element(
        7,
        "input",
        BTreeMap::from([("id".to_owned(), "email".to_owned())]),
    )]);
    let actions = vec![
        BrowserAction::Click(ClickElementAction {
            index: Some(1),
            coordinate_x: None,
            coordinate_y: None,
        }),
        BrowserAction::Input(InputTextAction {
            index: 1,
            text: "ada@example.test".to_owned(),
            clear: true,
        }),
        BrowserAction::Scroll(ScrollAction {
            down: true,
            pages: 1.0,
            index: Some(1),
        }),
        BrowserAction::UploadFile(UploadFileAction {
            index: 1,
            path: "/tmp/file.txt".to_owned(),
        }),
        BrowserAction::GetDropdownOptions(GetDropdownOptionsAction { index: 1 }),
        BrowserAction::SelectDropdownOption(SelectDropdownOptionAction {
            index: 1,
            text: "Team".to_owned(),
        }),
    ];

    for action in actions {
        let rematch = rematch_action_for_replay(&action, Some(&interacted), &current_dom)
            .expect("rematched action");
        assert_eq!(rematch.original_index, Some(1));
        assert_eq!(rematch.rematched_index, Some(7));
        assert!(rematch.changed);
        assert_eq!(rematch.action.interacted_element_index(), Some(7));
        assert_eq!(
            rematch.match_result.expect("match result").level,
            DomInteractedElementMatchLevel::Exact
        );
    }
}

#[test]
fn replay_rematch_preserves_non_indexed_or_missing_history_actions() {
    let current_dom = SerializedDomState::from_elements(vec![replay_dom_element(
        3,
        "button",
        BTreeMap::from([("id".to_owned(), "save".to_owned())]),
    )]);
    let coordinate_click = BrowserAction::Click(ClickElementAction {
        index: None,
        coordinate_x: Some(10),
        coordinate_y: Some(20),
    });
    let wait = BrowserAction::Wait(WaitAction { seconds: 0 });
    let input = BrowserAction::Input(InputTextAction {
        index: 1,
        text: "unchanged".to_owned(),
        clear: true,
    });

    let click_rematch =
        rematch_action_for_replay(&coordinate_click, None, &current_dom).expect("click");
    assert_eq!(click_rematch.action, coordinate_click);
    assert_eq!(click_rematch.original_index, None);
    assert!(!click_rematch.changed);

    let wait_rematch = rematch_action_for_replay(&wait, None, &current_dom).expect("wait");
    assert_eq!(wait_rematch.action, wait);
    assert_eq!(wait_rematch.original_index, None);
    assert!(!wait_rematch.changed);

    let input_rematch = rematch_action_for_replay(&input, None, &current_dom).expect("input");
    assert_eq!(input_rematch.action, input);
    assert_eq!(input_rematch.original_index, Some(1));
    assert_eq!(input_rematch.rematched_index, None);
    assert!(!input_rematch.changed);
}

#[test]
fn replay_rematch_reports_noop_and_ambiguous_matches() {
    let historical_element = replay_dom_element(
        1,
        "button",
        BTreeMap::from([("id".to_owned(), "save".to_owned())]),
    );
    let interacted = DomInteractedElement::from_element(&historical_element);
    let current_dom = SerializedDomState::from_elements(vec![replay_dom_element(
        1,
        "button",
        BTreeMap::from([("id".to_owned(), "save".to_owned())]),
    )]);
    let action = BrowserAction::Click(ClickElementAction {
        index: Some(1),
        coordinate_x: None,
        coordinate_y: None,
    });

    let rematch = rematch_action_for_replay(&action, Some(&interacted), &current_dom)
        .expect("same-index match");
    assert_eq!(rematch.action, action);
    assert_eq!(rematch.original_index, Some(1));
    assert_eq!(rematch.rematched_index, Some(1));
    assert!(!rematch.changed);

    let mut ambiguous = DomInteractedElement::from_element(&historical_element);
    ambiguous.element_hash = 0;
    ambiguous.stable_hash = None;
    ambiguous.x_path = String::new();
    ambiguous.ax_name = Some("Duplicate".to_owned());
    let mut first = replay_dom_element(1, "button", BTreeMap::new());
    first.name = Some("Duplicate".to_owned());
    let mut second = replay_dom_element(2, "button", BTreeMap::new());
    second.name = Some("Duplicate".to_owned());
    let current_dom = SerializedDomState::from_elements(vec![first, second]);
    let error =
        rematch_action_for_replay(&action, Some(&ambiguous), &current_dom).expect_err("error");
    assert_eq!(
        error.reason,
        DomInteractedElementMatchFailureReason::Ambiguous
    );

    let missing = DomInteractedElement {
        element_hash: 0,
        stable_hash: None,
        x_path: String::new(),
        ax_name: Some("Missing".to_owned()),
        ..DomInteractedElement::from_element(&historical_element)
    };
    let current_dom = SerializedDomState::from_elements(vec![replay_dom_element(
        3,
        "button",
        BTreeMap::from([("id".to_owned(), "other".to_owned())]),
    )]);
    let error =
        rematch_action_for_replay(&action, Some(&missing), &current_dom).expect_err("missing");
    assert_eq!(
        error.reason,
        DomInteractedElementMatchFailureReason::NotFound
    );
}

#[test]
fn history_replay_plan_remaps_actions_across_steps() {
    let mut first_state = blank_state();
    first_state.dom_state = SerializedDomState::from_elements(vec![replay_dom_element(
        1,
        "button",
        BTreeMap::from([("id".to_owned(), "save".to_owned())]),
    )]);
    let mut second_state = blank_state();
    second_state.dom_state = SerializedDomState::from_elements(vec![replay_dom_element(
        2,
        "input",
        BTreeMap::from([("name".to_owned(), "email".to_owned())]),
    )]);
    let history = AgentHistory {
        items: vec![
            history_item_with_state_actions(
                first_state,
                vec![BrowserAction::Click(ClickElementAction {
                    index: Some(1),
                    coordinate_x: None,
                    coordinate_y: None,
                })],
            ),
            history_item_with_state_actions(
                second_state,
                vec![
                    BrowserAction::Input(InputTextAction {
                        index: 2,
                        text: "ada@example.test".to_owned(),
                        clear: true,
                    }),
                    BrowserAction::Wait(WaitAction { seconds: 0 }),
                ],
            ),
        ],
        ..AgentHistory::default()
    };
    let current_dom = SerializedDomState::from_elements(vec![
        replay_dom_element(
            7,
            "button",
            BTreeMap::from([("id".to_owned(), "save".to_owned())]),
        ),
        replay_dom_element(
            8,
            "input",
            BTreeMap::from([("name".to_owned(), "email".to_owned())]),
        ),
    ]);

    let plan = history.replay_plan(&current_dom).expect("replay plan");

    assert_eq!(plan.actions.len(), 3);
    assert_eq!(plan.actions[0].step_index, 0);
    assert_eq!(plan.actions[0].action_index, 0);
    assert_eq!(
        plan.actions[0].remapped_action.interacted_element_index(),
        Some(7)
    );
    assert_eq!(
        plan.actions[0].rematch.action.interacted_element_index(),
        Some(7)
    );
    assert!(plan.actions[0].rematch.changed);
    assert_eq!(plan.actions[1].step_index, 1);
    assert_eq!(plan.actions[1].action_index, 0);
    assert_eq!(
        plan.actions[1].remapped_action.interacted_element_index(),
        Some(8)
    );
    assert_eq!(
        plan.actions[1].rematch.action.interacted_element_index(),
        Some(8)
    );
    assert!(plan.actions[1].rematch.changed);
    assert_eq!(plan.actions[2].step_index, 1);
    assert_eq!(plan.actions[2].action_index, 1);
    assert_eq!(
        plan.actions[2].original_action,
        plan.actions[2].remapped_action
    );
    assert_eq!(plan.actions[2].rematch.original_index, None);
    assert!(!plan.actions[2].rematch.changed);
}

#[test]
fn history_replay_plan_preserves_missing_historical_selector_entries() {
    let history = AgentHistory {
        items: vec![history_item_with_state_actions(
            blank_state(),
            vec![BrowserAction::Input(InputTextAction {
                index: 99,
                text: "missing".to_owned(),
                clear: true,
            })],
        )],
        ..AgentHistory::default()
    };
    let current_dom = SerializedDomState::from_elements(vec![replay_dom_element(
        1,
        "input",
        BTreeMap::from([("name".to_owned(), "email".to_owned())]),
    )]);

    let plan = history.replay_plan(&current_dom).expect("replay plan");

    assert_eq!(plan.actions.len(), 1);
    assert_eq!(
        plan.actions[0].original_action,
        plan.actions[0].remapped_action
    );
    assert_eq!(plan.actions[0].rematch.original_index, Some(99));
    assert_eq!(plan.actions[0].rematch.rematched_index, None);
    assert!(!plan.actions[0].rematch.changed);
}

#[test]
fn history_replay_plan_attaches_step_coordinates_to_rematch_failures() {
    let mut old = replay_dom_element(1, "button", BTreeMap::new());
    old.name = Some("Duplicate".to_owned());
    let mut state = blank_state();
    state.dom_state = SerializedDomState::from_elements(vec![old]);
    let history = AgentHistory {
        items: vec![history_item_with_state_actions(
            state,
            vec![BrowserAction::Click(ClickElementAction {
                index: Some(1),
                coordinate_x: None,
                coordinate_y: None,
            })],
        )],
        ..AgentHistory::default()
    };
    let mut first = replay_dom_element(2, "button", BTreeMap::new());
    first.name = Some("Duplicate".to_owned());
    let mut second = replay_dom_element(3, "button", BTreeMap::new());
    second.name = Some("Duplicate".to_owned());
    let current_dom = SerializedDomState::from_elements(vec![first, second]);

    let error = history.replay_plan(&current_dom).expect_err("ambiguous");

    assert_eq!(error.step_index, 0);
    assert_eq!(error.action_index, 0);
    assert_eq!(error.original_action.interacted_element_index(), Some(1));
    assert_eq!(error.original_index, Some(1));
    assert_eq!(
        error.failure.reason,
        DomInteractedElementMatchFailureReason::Ambiguous
    );

    let mut state = blank_state();
    state.dom_state = SerializedDomState::from_elements(vec![replay_dom_element(
        1,
        "button",
        BTreeMap::from([("data-testid".to_owned(), "missing".to_owned())]),
    )]);
    let history = AgentHistory {
        items: vec![history_item_with_state_actions(
            state,
            vec![BrowserAction::Click(ClickElementAction {
                index: Some(1),
                coordinate_x: None,
                coordinate_y: None,
            })],
        )],
        ..AgentHistory::default()
    };
    let current_dom = SerializedDomState::from_elements(vec![replay_dom_element(
        9,
        "button",
        BTreeMap::from([("data-testid".to_owned(), "other".to_owned())]),
    )]);

    let error = history
        .replay_plan(&current_dom)
        .expect_err("missing current rematch");

    assert_eq!(
        error.failure.reason,
        DomInteractedElementMatchFailureReason::NotFound
    );
    assert_eq!(error.original_index, Some(1));
}

#[test]
fn history_screenshots_match_browser_use_accessor() {
    let mut first_state = blank_state();
    first_state.screenshot = Some("first-shot".to_owned());
    let second_state = blank_state();
    let mut third_state = blank_state();
    third_state.screenshot = Some("third-shot".to_owned());

    let history = AgentHistory {
        items: vec![
            AgentHistoryItem {
                model_output: None,
                result: Vec::new(),
                state: first_state,
                metadata: None,
            },
            AgentHistoryItem {
                model_output: None,
                result: Vec::new(),
                state: second_state,
                metadata: None,
            },
            AgentHistoryItem {
                model_output: None,
                result: Vec::new(),
                state: third_state,
                metadata: None,
            },
        ],
        ..AgentHistory::default()
    };

    assert_eq!(
        history.screenshots(None, true),
        vec![Some("first-shot"), None, Some("third-shot")]
    );
    assert_eq!(
        history.screenshots(None, false),
        vec![Some("first-shot"), Some("third-shot")]
    );
    assert_eq!(
        history.screenshots(Some(2), true),
        vec![None, Some("third-shot")]
    );
    assert!(history.screenshots(Some(0), true).is_empty());
}

#[test]
fn history_judgement_helpers_use_terminal_result() {
    let judgement = JudgementResult {
        reasoning: Some("visual state did not match".to_owned()),
        verdict: false,
        failure_reason: Some("missing confirmation".to_owned()),
        impossible_task: false,
        reached_captcha: false,
    };
    let history = AgentHistory {
        items: vec![AgentHistoryItem {
            model_output: None,
            result: vec![ActionResult {
                extracted_content: Some("validated".to_owned()),
                error: None,
                judgement: Some(judgement),
                long_term_memory: None,
                include_extracted_content_only_once: false,
                include_in_memory: false,
                is_done: true,
                success: Some(false),
                attachments: Vec::new(),
                images: Vec::new(),
                metadata: None,
            }],
            state: blank_state(),
            metadata: None,
        }],
        ..AgentHistory::default()
    };

    assert!(history.is_judged());
    assert_eq!(history.is_validated(), Some(false));
    assert_eq!(
        history
            .judgement()
            .and_then(|judgement| judgement.failure_reason.as_deref()),
        Some("missing confirmation")
    );
}

fn blank_state() -> BrowserStateSummary {
    BrowserStateSummary {
        dom_state: Default::default(),
        url: "about:blank".to_owned(),
        title: "Blank".to_owned(),
        tabs: vec![],
        screenshot: None,
        page_info: None,
        pixels_above: 0,
        pixels_below: 0,
        browser_errors: vec![],
        is_pdf_viewer: false,
        recent_events: None,
        pending_network_requests: vec![],
        pagination_buttons: vec![],
        closed_popup_messages: vec![],
    }
}

fn test_png_base64(width: u32, height: u32) -> String {
    let image = image::RgbaImage::from_pixel(width, height, image::Rgba([12, 34, 56, 255]));
    let mut cursor = std::io::Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba8(image)
        .write_to(&mut cursor, image::ImageFormat::Png)
        .expect("encode png");
    base64::engine::general_purpose::STANDARD.encode(cursor.into_inner())
}

fn png_dimensions_from_data_url(data_url: &str) -> (u32, u32) {
    let payload = data_url
        .strip_prefix("data:image/png;base64,")
        .expect("png data url");
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(payload)
        .expect("decode png");
    let image = image::load_from_memory(&bytes).expect("load png");
    (image.width(), image.height())
}

fn replay_dom_element(
    index: u32,
    tag_name: &str,
    attributes: BTreeMap<String, String>,
) -> browser_use_dom::DomElementRef {
    browser_use_dom::DomElementRef {
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

fn history_item_with_actions(actions: Vec<BrowserAction>) -> AgentHistoryItem {
    history_item_with_state_actions(blank_state(), actions)
}

fn history_item_with_state_actions(
    state: BrowserStateSummary,
    actions: Vec<BrowserAction>,
) -> AgentHistoryItem {
    AgentHistoryItem {
        model_output: Some(AgentOutput {
            current_state: AgentCurrentState::default(),
            thinking: None,
            evaluation_previous_goal: None,
            memory: None,
            next_goal: None,
            current_plan_item: None,
            plan_update: None,
            action: actions,
        }),
        result: vec![ActionResult::extracted("ok")],
        state,
        metadata: None,
    }
}

struct RecordingExecutor {
    seen: Vec<&'static str>,
}

#[async_trait]
impl ActionExecutor for RecordingExecutor {
    async fn execute(&mut self, action: &BrowserAction) -> ActionResult {
        self.seen.push(action.name());
        ActionResult {
            extracted_content: Some(action.name().to_owned()),
            error: None,
            judgement: None,
            long_term_memory: None,
            include_extracted_content_only_once: false,
            include_in_memory: false,
            is_done: matches!(action, BrowserAction::Done(_)),
            success: None,
            attachments: Vec::new(),
            images: Vec::new(),
            metadata: None,
        }
    }
}

struct ScriptedReplayExecutor {
    seen: Vec<BrowserAction>,
    results: VecDeque<ActionResult>,
}

#[async_trait]
impl ActionExecutor for ScriptedReplayExecutor {
    async fn execute(&mut self, action: &BrowserAction) -> ActionResult {
        self.seen.push(action.clone());
        self.results
            .pop_front()
            .unwrap_or_else(|| ActionResult::extracted(action.name()))
    }
}

fn replay_plan_item(
    step_index: usize,
    action_index: usize,
    original_action: BrowserAction,
    executed_action: BrowserAction,
) -> AgentHistoryReplayPlanItem {
    AgentHistoryReplayPlanItem {
        step_index,
        action_index,
        original_action: original_action.clone(),
        remapped_action: executed_action.clone(),
        rematch: ActionReplayRematch {
            action: executed_action.clone(),
            original_index: original_action.interacted_element_index(),
            rematched_index: executed_action.interacted_element_index(),
            match_result: None,
            changed: original_action != executed_action,
        },
    }
}

#[tokio::test]
async fn action_sequence_stops_after_navigation() {
    let actions = vec![
        BrowserAction::Navigate(NavigateAction {
            url: "https://example.com".to_owned(),
            new_tab: false,
        }),
        BrowserAction::Click(ClickElementAction {
            index: Some(1),
            coordinate_x: None,
            coordinate_y: None,
        }),
    ];
    let mut executor = RecordingExecutor { seen: Vec::new() };

    let results = execute_action_sequence(&mut executor, &actions).await;

    assert_eq!(executor.seen, vec!["navigate"]);
    assert_eq!(results.len(), 1);
}

#[tokio::test]
async fn done_is_only_executed_when_first_action() {
    let actions = vec![
        BrowserAction::Click(ClickElementAction {
            index: Some(1),
            coordinate_x: None,
            coordinate_y: None,
        }),
        BrowserAction::Done(DoneAction {
            text: "finished".to_owned(),
            success: true,
            files_to_display: vec![],
        }),
    ];
    let mut executor = RecordingExecutor { seen: Vec::new() };

    let results = execute_action_sequence(&mut executor, &actions).await;

    assert_eq!(executor.seen, vec!["click"]);
    assert_eq!(results.len(), 1);
}

#[tokio::test]
async fn agent_wait_between_actions_does_not_delay_first_action() {
    let settings = AgentSettings {
        wait_between_actions_seconds: 5.0,
        ..AgentSettings::default()
    };
    let mut agent = Agent::with_settings(
        "single action",
        settings,
        QueueModel::new(vec![]),
        MockSession::new(),
    );
    let actions = vec![BrowserAction::Wait(WaitAction { seconds: 0 })];

    let results = timeout(
        Duration::from_millis(100),
        agent.execute_agent_sequence(&actions),
    )
    .await
    .expect("first action should not wait")
    .expect("execute actions");

    assert_eq!(results.len(), 1);
    assert_eq!(
        results[0].extracted_content.as_deref(),
        Some("Waited for 0 seconds")
    );
}

#[tokio::test]
async fn agent_wait_between_actions_delays_second_action() {
    let settings = AgentSettings {
        wait_between_actions_seconds: 0.05,
        ..AgentSettings::default()
    };
    let mut agent = Agent::with_settings(
        "two actions",
        settings,
        QueueModel::new(vec![]),
        MockSession::new(),
    );
    let actions = vec![
        BrowserAction::Wait(WaitAction { seconds: 0 }),
        BrowserAction::Wait(WaitAction { seconds: 0 }),
    ];
    let started = std::time::Instant::now();

    let results = agent
        .execute_agent_sequence(&actions)
        .await
        .expect("execute actions");

    assert!(started.elapsed() >= Duration::from_millis(40));
    assert_eq!(results.len(), 2);
    assert_eq!(
        results[1].extracted_content.as_deref(),
        Some("Waited for 0 seconds")
    );
}

#[tokio::test]
async fn history_replay_execution_runs_rematched_plan_actions() {
    let original_click = BrowserAction::Click(ClickElementAction {
        index: Some(1),
        coordinate_x: None,
        coordinate_y: None,
    });
    let remapped_click = BrowserAction::Click(ClickElementAction {
        index: Some(7),
        coordinate_x: None,
        coordinate_y: None,
    });
    let wait = BrowserAction::Wait(WaitAction { seconds: 0 });
    let plan = AgentHistoryReplayPlan {
        actions: vec![
            replay_plan_item(2, 0, original_click.clone(), remapped_click.clone()),
            replay_plan_item(2, 1, wait.clone(), wait.clone()),
        ],
    };
    let mut executor = ScriptedReplayExecutor {
        seen: Vec::new(),
        results: VecDeque::from([
            ActionResult::extracted("clicked remapped element"),
            ActionResult::extracted("waited"),
        ]),
    };

    let execution = execute_history_replay_plan(&mut executor, &plan).await;

    assert_eq!(executor.seen, vec![remapped_click.clone(), wait]);
    assert_eq!(execution.items.len(), 2);
    assert_eq!(execution.items[0].step_index, 2);
    assert_eq!(execution.items[0].action_index, 0);
    assert_eq!(execution.items[0].original_action, original_click);
    assert_eq!(execution.items[0].executed_action, remapped_click);
    assert_eq!(execution.items[0].rematch.original_index, Some(1));
    assert_eq!(execution.items[0].rematch.rematched_index, Some(7));
    assert_eq!(
        execution.items[0].result.extracted_content.as_deref(),
        Some("clicked remapped element")
    );
    assert_eq!(execution.stop, None);
}

#[tokio::test]
async fn history_replay_execution_stops_on_action_errors_with_coordinates() {
    let click = BrowserAction::Click(ClickElementAction {
        index: Some(1),
        coordinate_x: None,
        coordinate_y: None,
    });
    let wait = BrowserAction::Wait(WaitAction { seconds: 0 });
    let plan = AgentHistoryReplayPlan {
        actions: vec![
            replay_plan_item(4, 2, click.clone(), click),
            replay_plan_item(4, 3, wait.clone(), wait),
        ],
    };
    let mut executor = ScriptedReplayExecutor {
        seen: Vec::new(),
        results: VecDeque::from([ActionResult::error("click failed")]),
    };

    let execution = execute_history_replay_plan(&mut executor, &plan).await;

    assert_eq!(executor.seen.len(), 1);
    assert_eq!(
        execution.stop,
        Some(AgentHistoryReplayStop {
            step_index: 4,
            action_index: 2,
            reason: AgentHistoryReplayStopReason::Error,
            diagnostic: Some("click failed".to_owned()),
        })
    );
    assert_eq!(
        execution.items[0].result.error.as_deref(),
        Some("click failed")
    );
}

#[tokio::test]
async fn history_replay_execution_stops_after_terminating_actions() {
    let navigate = BrowserAction::Navigate(NavigateAction {
        url: "https://example.com".to_owned(),
        new_tab: false,
    });
    let click = BrowserAction::Click(ClickElementAction {
        index: Some(1),
        coordinate_x: None,
        coordinate_y: None,
    });
    let plan = AgentHistoryReplayPlan {
        actions: vec![
            replay_plan_item(0, 0, navigate.clone(), navigate.clone()),
            replay_plan_item(0, 1, click.clone(), click),
        ],
    };
    let mut executor = ScriptedReplayExecutor {
        seen: Vec::new(),
        results: VecDeque::from([ActionResult::extracted("navigated")]),
    };

    let execution = execute_history_replay_plan(&mut executor, &plan).await;

    assert_eq!(executor.seen, vec![navigate]);
    assert_eq!(execution.items.len(), 1);
    assert_eq!(
        execution.stop,
        Some(AgentHistoryReplayStop {
            step_index: 0,
            action_index: 0,
            reason: AgentHistoryReplayStopReason::TerminatingAction,
            diagnostic: None,
        })
    );
}

#[tokio::test]
async fn history_replay_execution_matches_done_sequence_rules() {
    let done = BrowserAction::Done(DoneAction {
        text: "finished".to_owned(),
        success: true,
        files_to_display: vec![],
    });
    let first_done_plan = AgentHistoryReplayPlan {
        actions: vec![replay_plan_item(9, 0, done.clone(), done.clone())],
    };
    let mut executor = ScriptedReplayExecutor {
        seen: Vec::new(),
        results: VecDeque::from([ActionResult::done("finished", true)]),
    };

    let execution = execute_history_replay_plan(&mut executor, &first_done_plan).await;

    assert_eq!(executor.seen, vec![done.clone()]);
    assert_eq!(
        execution.stop,
        Some(AgentHistoryReplayStop {
            step_index: 9,
            action_index: 0,
            reason: AgentHistoryReplayStopReason::Done,
            diagnostic: None,
        })
    );

    let wait = BrowserAction::Wait(WaitAction { seconds: 0 });
    let delayed_done_plan = AgentHistoryReplayPlan {
        actions: vec![
            replay_plan_item(9, 0, wait.clone(), wait.clone()),
            replay_plan_item(9, 1, done.clone(), done),
        ],
    };
    let mut executor = ScriptedReplayExecutor {
        seen: Vec::new(),
        results: VecDeque::from([ActionResult::extracted("waited")]),
    };

    let execution = execute_history_replay_plan(&mut executor, &delayed_done_plan).await;

    assert_eq!(executor.seen, vec![wait]);
    assert_eq!(execution.items.len(), 1);
    assert_eq!(
        execution.stop,
        Some(AgentHistoryReplayStop {
            step_index: 9,
            action_index: 1,
            reason: AgentHistoryReplayStopReason::DoneAfterPriorAction,
            diagnostic: None,
        })
    );
}

struct MockSession {
    events: Mutex<Vec<String>>,
    states: Mutex<VecDeque<BrowserStateSummary>>,
    state_screenshot_requests: Mutex<Vec<bool>>,
    state_error: Mutex<Option<String>>,
    click_error: Mutex<Option<String>>,
    click_delay: Mutex<Option<Duration>>,
}

impl MockSession {
    fn new() -> Self {
        Self {
            events: Mutex::new(Vec::new()),
            states: Mutex::new(VecDeque::new()),
            state_screenshot_requests: Mutex::new(Vec::new()),
            state_error: Mutex::new(None),
            click_error: Mutex::new(None),
            click_delay: Mutex::new(None),
        }
    }

    fn with_states(states: Vec<BrowserStateSummary>) -> Self {
        Self {
            events: Mutex::new(Vec::new()),
            states: Mutex::new(VecDeque::from(states)),
            state_screenshot_requests: Mutex::new(Vec::new()),
            state_error: Mutex::new(None),
            click_error: Mutex::new(None),
            click_delay: Mutex::new(None),
        }
    }

    fn with_state_error(error: impl Into<String>) -> Self {
        Self {
            events: Mutex::new(Vec::new()),
            states: Mutex::new(VecDeque::new()),
            state_screenshot_requests: Mutex::new(Vec::new()),
            state_error: Mutex::new(Some(error.into())),
            click_error: Mutex::new(None),
            click_delay: Mutex::new(None),
        }
    }

    fn with_click_error(error: impl Into<String>) -> Self {
        Self {
            events: Mutex::new(Vec::new()),
            states: Mutex::new(VecDeque::new()),
            state_screenshot_requests: Mutex::new(Vec::new()),
            state_error: Mutex::new(None),
            click_error: Mutex::new(Some(error.into())),
            click_delay: Mutex::new(None),
        }
    }

    fn with_click_delay(delay: Duration) -> Self {
        Self {
            events: Mutex::new(Vec::new()),
            states: Mutex::new(VecDeque::new()),
            state_screenshot_requests: Mutex::new(Vec::new()),
            state_error: Mutex::new(None),
            click_error: Mutex::new(None),
            click_delay: Mutex::new(Some(delay)),
        }
    }

    fn events(&self) -> Vec<String> {
        self.events.lock().expect("events lock").clone()
    }

    fn state_screenshot_requests(&self) -> Vec<bool> {
        self.state_screenshot_requests
            .lock()
            .expect("state requests lock")
            .clone()
    }
}

#[async_trait]
impl BrowserSession for MockSession {
    async fn state(&self, include_screenshot: bool) -> Result<BrowserStateSummary, BrowserError> {
        self.state_screenshot_requests
            .lock()
            .expect("state requests lock")
            .push(include_screenshot);
        if let Some(error) = self.state_error.lock().expect("state error lock").clone() {
            return Err(BrowserError::StateUnavailable(error));
        }
        if let Some(state) = self.states.lock().expect("states lock").pop_front() {
            return Ok(state);
        }
        Ok(BrowserStateSummary {
            dom_state: SerializedDomState::default(),
            ..blank_state()
        })
    }

    async fn navigate(&self, url: &str, new_tab: bool) -> Result<(), BrowserError> {
        self.events
            .lock()
            .expect("events lock")
            .push(format!("navigate:{url}:{new_tab}"));
        Ok(())
    }

    async fn go_back(&self) -> Result<(), BrowserError> {
        self.events
            .lock()
            .expect("events lock")
            .push("go_back".to_owned());
        Ok(())
    }

    async fn switch_tab(&self, target_id: &str) -> Result<(), BrowserError> {
        self.events
            .lock()
            .expect("events lock")
            .push(format!("switch_tab:{target_id}"));
        Ok(())
    }

    async fn close_tab(&self, target_id: &str) -> Result<(), BrowserError> {
        self.events
            .lock()
            .expect("events lock")
            .push(format!("close_tab:{target_id}"));
        Ok(())
    }

    async fn click(&self, index: u32) -> Result<(), BrowserError> {
        self.events
            .lock()
            .expect("events lock")
            .push(format!("click:{index}"));
        let delay = self.click_delay.lock().expect("click delay lock").take();
        if let Some(delay) = delay {
            sleep(delay).await;
        }
        if let Some(error) = self.click_error.lock().expect("click error lock").take() {
            return Err(BrowserError::ActionFailed(error));
        }
        Ok(())
    }

    async fn click_coordinates(&self, x: i32, y: i32) -> Result<(), BrowserError> {
        self.events
            .lock()
            .expect("events lock")
            .push(format!("click_coordinates:{x}:{y}"));
        Ok(())
    }

    async fn input_text(&self, index: u32, text: &str, clear: bool) -> Result<(), BrowserError> {
        self.events
            .lock()
            .expect("events lock")
            .push(format!("input:{index}:{text}:{clear}"));
        Ok(())
    }

    async fn scroll(&self, index: Option<u32>, down: bool, pages: f64) -> Result<(), BrowserError> {
        self.events
            .lock()
            .expect("events lock")
            .push(format!("scroll:{index:?}:{down}:{pages}"));
        Ok(())
    }

    async fn find_text(&self, text: &str) -> Result<bool, BrowserError> {
        self.events
            .lock()
            .expect("events lock")
            .push(format!("find_text:{text}"));
        Ok(true)
    }

    async fn evaluate(&self, code: &str) -> Result<String, BrowserError> {
        self.events
            .lock()
            .expect("events lock")
            .push(format!("evaluate:{code}"));
        Ok("EvalOps JS result".to_owned())
    }

    async fn dropdown_options(&self, index: u32) -> Result<Vec<String>, BrowserError> {
        self.events
            .lock()
            .expect("events lock")
            .push(format!("dropdown_options:{index}"));
        Ok(vec!["One".to_owned(), "Two".to_owned()])
    }

    async fn select_dropdown_option(&self, index: u32, text: &str) -> Result<(), BrowserError> {
        self.events
            .lock()
            .expect("events lock")
            .push(format!("select_dropdown_option:{index}:{text}"));
        Ok(())
    }

    async fn page_text(&self) -> Result<String, BrowserError> {
        self.events
            .lock()
            .expect("events lock")
            .push("page_text".to_owned());
        Ok("Alpha EvalOps Beta\nSecond EvalOps line".to_owned())
    }

    async fn find_elements(
        &self,
        selector: &str,
        attributes: &[String],
        max_results: usize,
        include_text: bool,
    ) -> Result<Vec<FoundElement>, BrowserError> {
        self.events.lock().expect("events lock").push(format!(
            "find_elements:{selector}:{}:{max_results}:{include_text}",
            attributes.join("|")
        ));
        if selector == "a[href]" {
            let mut attrs = BTreeMap::new();
            attrs.insert("href".to_owned(), "https://evalops.dev/run".to_owned());
            attrs.insert("title".to_owned(), "Run EvalOps".to_owned());
            return Ok(vec![FoundElement {
                tag_name: "a".to_owned(),
                text: include_text.then(|| "Run EvalOps".to_owned()),
                attributes: attrs,
            }]);
        }
        if selector == "img[src], img[data-src], picture source[srcset]" {
            let mut attrs = BTreeMap::new();
            attrs.insert("src".to_owned(), "https://evalops.dev/hero.png".to_owned());
            attrs.insert("alt".to_owned(), "Hero shot".to_owned());
            return Ok(vec![FoundElement {
                tag_name: "img".to_owned(),
                text: include_text.then(|| "ignored".to_owned()),
                attributes: attrs,
            }]);
        }
        let mut attrs = BTreeMap::new();
        attrs.insert("id".to_owned(), "run".to_owned());
        Ok(vec![FoundElement {
            tag_name: "button".to_owned(),
            text: include_text.then(|| "Run EvalOps".to_owned()),
            attributes: attrs,
        }])
    }

    async fn send_keys(&self, keys: &str) -> Result<(), BrowserError> {
        self.events
            .lock()
            .expect("events lock")
            .push(format!("send_keys:{keys}"));
        Ok(())
    }

    async fn upload_file(&self, index: u32, path: &std::path::Path) -> Result<(), BrowserError> {
        self.events
            .lock()
            .expect("events lock")
            .push(format!("upload_file:{index}:{}", path.display()));
        Ok(())
    }

    async fn screenshot(&self) -> Result<Screenshot, BrowserError> {
        self.events
            .lock()
            .expect("events lock")
            .push("screenshot".to_owned());
        Ok(Screenshot {
            base64_png: base64::engine::general_purpose::STANDARD.encode("PNGDATA"),
        })
    }

    async fn save_pdf(
        &self,
        print_background: bool,
        landscape: bool,
        scale: f64,
        paper_format: &str,
    ) -> Result<Pdf, BrowserError> {
        self.events.lock().expect("events lock").push(format!(
            "save_pdf:{print_background}:{landscape}:{scale}:{paper_format}"
        ));
        Ok(Pdf {
            base64_pdf: base64::engine::general_purpose::STANDARD.encode("%PDF-1.7"),
        })
    }
}

#[tokio::test]
async fn browser_executor_maps_navigate_to_session() {
    let session = MockSession::new();
    let mut executor = BrowserActionExecutor::new(session);

    let result = executor
        .execute(&BrowserAction::Navigate(NavigateAction {
            url: "https://example.com".to_owned(),
            new_tab: false,
        }))
        .await;

    assert!(result.error.is_none());
    assert_eq!(
        executor.session().events(),
        vec!["navigate:https://example.com:false"]
    );
}

#[tokio::test]
async fn browser_executor_returns_action_error_when_action_times_out() {
    let session = MockSession::with_click_delay(Duration::from_millis(50));
    let mut executor = BrowserActionExecutor::new(session);
    executor.set_action_timeout_seconds(0.005);

    let result = executor
        .execute(&BrowserAction::Click(ClickElementAction {
            index: Some(1),
            coordinate_x: None,
            coordinate_y: None,
        }))
        .await;

    let error = result.error.expect("timeout error");
    assert!(error.contains("Action click timed out after"));
    assert!(error.contains("dead CDP WebSocket"));
}

#[tokio::test]
async fn terminating_sequence_action_does_not_need_pre_state() {
    let session = MockSession::with_state_error("state unavailable");
    let mut executor = BrowserActionExecutor::new(session);

    let results = executor
        .execute_sequence(&[BrowserAction::Navigate(NavigateAction {
            url: "https://example.com".to_owned(),
            new_tab: false,
        })])
        .await;

    assert_eq!(results.len(), 1);
    assert_eq!(results[0].error, None);
    assert_eq!(
        executor.session().events(),
        vec!["navigate:https://example.com:false"]
    );
    assert!(executor.session().state_screenshot_requests().is_empty());
}

#[tokio::test]
async fn browser_executor_stops_sequence_when_url_changes() {
    let mut first_state = blank_state();
    first_state.url = "https://example.com/one".to_owned();
    let mut second_state = blank_state();
    second_state.url = "https://example.com/two".to_owned();
    let session = MockSession::with_states(vec![first_state, second_state]);
    let mut executor = BrowserActionExecutor::new(session);

    let results = executor
        .execute_sequence(&[
            BrowserAction::Click(ClickElementAction {
                index: Some(1),
                coordinate_x: None,
                coordinate_y: None,
            }),
            BrowserAction::Input(InputTextAction {
                index: 2,
                text: "should not type".to_owned(),
                clear: true,
            }),
        ])
        .await;

    assert_eq!(results.len(), 1);
    assert_eq!(executor.session().events(), vec!["click:1"]);
}

#[tokio::test]
async fn browser_replay_execution_stops_when_url_changes() {
    let mut first_state = blank_state();
    first_state.url = "https://example.com/one".to_owned();
    let mut second_state = blank_state();
    second_state.url = "https://example.com/two".to_owned();
    let session = MockSession::with_states(vec![first_state, second_state]);
    let mut executor = BrowserActionExecutor::new(session);
    let original_click = BrowserAction::Click(ClickElementAction {
        index: Some(1),
        coordinate_x: None,
        coordinate_y: None,
    });
    let remapped_click = BrowserAction::Click(ClickElementAction {
        index: Some(7),
        coordinate_x: None,
        coordinate_y: None,
    });
    let input = BrowserAction::Input(InputTextAction {
        index: 2,
        text: "should not type".to_owned(),
        clear: true,
    });
    let plan = AgentHistoryReplayPlan {
        actions: vec![
            replay_plan_item(3, 0, original_click, remapped_click.clone()),
            replay_plan_item(3, 1, input.clone(), input),
        ],
    };

    let execution = executor.execute_replay_plan(&plan).await;

    assert_eq!(executor.session().events(), vec!["click:7"]);
    assert_eq!(
        executor.session().state_screenshot_requests(),
        vec![false, false]
    );
    assert_eq!(execution.items.len(), 1);
    assert_eq!(execution.items[0].executed_action, remapped_click);
    assert_eq!(
        execution.stop,
        Some(AgentHistoryReplayStop {
            step_index: 3,
            action_index: 0,
            reason: AgentHistoryReplayStopReason::PageChanged,
            diagnostic: None,
        })
    );
}

#[tokio::test]
async fn browser_replay_execution_continues_when_guarded_url_is_stable() {
    let mut state = blank_state();
    state.url = "https://example.com/stable".to_owned();
    let session = MockSession::with_states(vec![
        state.clone(),
        state.clone(),
        state.clone(),
        state.clone(),
    ]);
    let mut executor = BrowserActionExecutor::new(session);
    let click = BrowserAction::Click(ClickElementAction {
        index: Some(1),
        coordinate_x: None,
        coordinate_y: None,
    });
    let input = BrowserAction::Input(InputTextAction {
        index: 2,
        text: "hello".to_owned(),
        clear: true,
    });
    let plan = AgentHistoryReplayPlan {
        actions: vec![
            replay_plan_item(4, 0, click.clone(), click),
            replay_plan_item(4, 1, input.clone(), input),
        ],
    };

    let execution = executor.execute_replay_plan(&plan).await;

    assert_eq!(execution.items.len(), 2);
    assert_eq!(execution.stop, None);
    assert_eq!(
        executor.session().events(),
        vec!["click:1", "input:2:hello:true"]
    );
    assert_eq!(
        executor.session().state_screenshot_requests(),
        vec![false, false, false, false]
    );
}

#[tokio::test]
async fn browser_replay_execution_reports_pre_state_failure_without_action() {
    let session = MockSession::with_state_error("state unavailable");
    let mut executor = BrowserActionExecutor::new(session);
    let click = BrowserAction::Click(ClickElementAction {
        index: Some(1),
        coordinate_x: None,
        coordinate_y: None,
    });
    let plan = AgentHistoryReplayPlan {
        actions: vec![replay_plan_item(5, 0, click.clone(), click)],
    };

    let execution = executor.execute_replay_plan(&plan).await;

    assert!(executor.session().events().is_empty());
    assert_eq!(executor.session().state_screenshot_requests(), vec![false]);
    assert_eq!(execution.items.len(), 1);
    assert!(
        execution.items[0]
            .result
            .error
            .as_deref()
            .is_some_and(|error| error.contains("state unavailable"))
    );
    let stop = execution.stop.expect("state failure stop");
    assert_eq!(stop.step_index, 5);
    assert_eq!(stop.action_index, 0);
    assert_eq!(stop.reason, AgentHistoryReplayStopReason::Error);
    assert!(
        stop.diagnostic
            .as_deref()
            .is_some_and(|error| error.contains("state unavailable"))
    );
}

#[tokio::test]
async fn browser_replay_execution_preserves_done_and_error_stops() {
    let session = MockSession::new();
    let mut executor = BrowserActionExecutor::new(session);
    let done = BrowserAction::Done(DoneAction {
        text: "finished".to_owned(),
        success: true,
        files_to_display: vec![],
    });
    let plan = AgentHistoryReplayPlan {
        actions: vec![replay_plan_item(7, 0, done.clone(), done.clone())],
    };

    let execution = executor.execute_replay_plan(&plan).await;

    assert_eq!(execution.items.len(), 1);
    assert_eq!(execution.items[0].executed_action, done);
    assert_eq!(
        execution.stop,
        Some(AgentHistoryReplayStop {
            step_index: 7,
            action_index: 0,
            reason: AgentHistoryReplayStopReason::Done,
            diagnostic: None,
        })
    );
    assert!(executor.session().state_screenshot_requests().is_empty());

    let mut state = blank_state();
    state.url = "https://example.com/stable".to_owned();
    let session = MockSession::with_states(vec![state]);
    *session.click_error.lock().expect("click error lock") = Some("boom".to_owned());
    let mut executor = BrowserActionExecutor::new(session);
    let click = BrowserAction::Click(ClickElementAction {
        index: Some(1),
        coordinate_x: None,
        coordinate_y: None,
    });
    let plan = AgentHistoryReplayPlan {
        actions: vec![replay_plan_item(8, 0, click.clone(), click)],
    };

    let execution = executor.execute_replay_plan(&plan).await;

    assert_eq!(execution.items.len(), 1);
    assert!(
        execution.items[0]
            .result
            .error
            .as_deref()
            .is_some_and(|error| error.contains("boom"))
    );
    assert_eq!(
        execution.stop,
        Some(AgentHistoryReplayStop {
            step_index: 8,
            action_index: 0,
            reason: AgentHistoryReplayStopReason::Error,
            diagnostic: Some("action failed: boom".to_owned()),
        })
    );
    assert_eq!(executor.session().state_screenshot_requests(), vec![false]);
}

#[tokio::test]
async fn browser_replay_execution_skips_pre_state_for_terminating_actions() {
    let session = MockSession::with_state_error("state unavailable");
    let mut executor = BrowserActionExecutor::new(session);
    let navigate = BrowserAction::Navigate(NavigateAction {
        url: "https://example.com".to_owned(),
        new_tab: false,
    });
    let click = BrowserAction::Click(ClickElementAction {
        index: Some(1),
        coordinate_x: None,
        coordinate_y: None,
    });
    let plan = AgentHistoryReplayPlan {
        actions: vec![
            replay_plan_item(6, 0, navigate.clone(), navigate.clone()),
            replay_plan_item(6, 1, click.clone(), click),
        ],
    };

    let execution = executor.execute_replay_plan(&plan).await;

    assert_eq!(
        executor.session().events(),
        vec!["navigate:https://example.com:false"]
    );
    assert!(executor.session().state_screenshot_requests().is_empty());
    assert_eq!(execution.items.len(), 1);
    assert_eq!(execution.items[0].executed_action, navigate);
    assert_eq!(
        execution.stop,
        Some(AgentHistoryReplayStop {
            step_index: 6,
            action_index: 0,
            reason: AgentHistoryReplayStopReason::TerminatingAction,
            diagnostic: None,
        })
    );
}

#[tokio::test]
async fn browser_replay_history_captures_current_state_and_executes_plan() {
    let mut historical_state = blank_state();
    historical_state.dom_state = SerializedDomState::from_elements(vec![replay_dom_element(
        1,
        "button",
        BTreeMap::from([("id".to_owned(), "save".to_owned())]),
    )]);
    let history = AgentHistory {
        items: vec![history_item_with_state_actions(
            historical_state,
            vec![BrowserAction::Click(ClickElementAction {
                index: Some(1),
                coordinate_x: None,
                coordinate_y: None,
            })],
        )],
        ..AgentHistory::default()
    };
    let mut current_state = blank_state();
    current_state.url = "https://example.com/current".to_owned();
    current_state.dom_state = SerializedDomState::from_elements(vec![replay_dom_element(
        7,
        "button",
        BTreeMap::from([("id".to_owned(), "save".to_owned())]),
    )]);
    let mut before = blank_state();
    before.url = current_state.url.clone();
    let after = before.clone();
    let session = MockSession::with_states(vec![current_state.clone(), before, after]);
    let mut executor = BrowserActionExecutor::new(session);

    let run = executor.replay_history(&history).await.expect("replay run");

    assert_eq!(run.current_state.url, current_state.url);
    assert_eq!(run.plan.actions.len(), 1);
    assert_eq!(
        run.plan.actions[0]
            .remapped_action
            .interacted_element_index(),
        Some(7)
    );
    assert_eq!(run.execution.items.len(), 1);
    assert_eq!(
        run.execution.items[0]
            .executed_action
            .interacted_element_index(),
        Some(7)
    );
    assert_eq!(run.execution.stop, None);
    assert_eq!(executor.session().events(), vec!["click:7"]);
    assert_eq!(
        executor.session().state_screenshot_requests(),
        vec![false, false]
    );
}

#[tokio::test]
async fn browser_replay_history_recaptures_dom_between_actions() {
    let mut historical_click_state = blank_state();
    historical_click_state.dom_state = SerializedDomState::from_elements(vec![replay_dom_element(
        1,
        "button",
        BTreeMap::from([("id".to_owned(), "reveal-email".to_owned())]),
    )]);
    let mut historical_input_state = blank_state();
    historical_input_state.dom_state = SerializedDomState::from_elements(vec![replay_dom_element(
        2,
        "input",
        BTreeMap::from([("name".to_owned(), "email".to_owned())]),
    )]);
    let history = AgentHistory {
        items: vec![
            history_item_with_state_actions(
                historical_click_state,
                vec![BrowserAction::Click(ClickElementAction {
                    index: Some(1),
                    coordinate_x: None,
                    coordinate_y: None,
                })],
            ),
            history_item_with_state_actions(
                historical_input_state,
                vec![BrowserAction::Input(InputTextAction {
                    index: 2,
                    text: "ada@example.test".to_owned(),
                    clear: true,
                })],
            ),
        ],
        ..AgentHistory::default()
    };
    let mut initial_state = blank_state();
    initial_state.url = "https://example.com/start".to_owned();
    initial_state.dom_state = SerializedDomState::from_elements(vec![replay_dom_element(
        7,
        "button",
        BTreeMap::from([("id".to_owned(), "reveal-email".to_owned())]),
    )]);
    let mut after_click_state = blank_state();
    after_click_state.url = "https://example.com/form".to_owned();
    after_click_state.dom_state = SerializedDomState::from_elements(vec![replay_dom_element(
        8,
        "input",
        BTreeMap::from([("name".to_owned(), "email".to_owned())]),
    )]);
    let after_input_state = after_click_state.clone();
    let session = MockSession::with_states(vec![
        initial_state.clone(),
        after_click_state,
        after_input_state,
    ]);
    let mut executor = BrowserActionExecutor::new(session);

    let run = executor.replay_history(&history).await.expect("replay run");

    assert_eq!(run.current_state.url, initial_state.url);
    assert_eq!(run.plan.actions.len(), 2);
    assert_eq!(
        run.plan.actions[0]
            .remapped_action
            .interacted_element_index(),
        Some(7)
    );
    assert_eq!(
        run.plan.actions[1]
            .remapped_action
            .interacted_element_index(),
        Some(8)
    );
    assert_eq!(run.execution.stop, None);
    assert_eq!(
        executor.session().events(),
        vec!["click:7", "input:8:ada@example.test:true"]
    );
    assert_eq!(
        executor.session().state_screenshot_requests(),
        vec![false, false, false]
    );
}

#[tokio::test]
async fn browser_replay_history_reports_current_state_failure() {
    let session = MockSession::with_state_error("state unavailable");
    let mut executor = BrowserActionExecutor::new(session);
    let history = AgentHistory::default();

    let error = executor
        .replay_history(&history)
        .await
        .expect_err("state error");

    assert!(matches!(
        error,
        AgentHistoryReplayRunError::CurrentState { .. }
    ));
    assert_eq!(executor.session().events(), Vec::<String>::new());
    assert_eq!(executor.session().state_screenshot_requests(), vec![false]);
}

#[tokio::test]
async fn browser_replay_history_reports_plan_rematch_failure() {
    let mut historical_state = blank_state();
    historical_state.dom_state = SerializedDomState::from_elements(vec![replay_dom_element(
        1,
        "button",
        BTreeMap::from([("data-testid".to_owned(), "duplicate".to_owned())]),
    )]);
    let history = AgentHistory {
        items: vec![history_item_with_state_actions(
            historical_state,
            vec![BrowserAction::Click(ClickElementAction {
                index: Some(1),
                coordinate_x: None,
                coordinate_y: None,
            })],
        )],
        ..AgentHistory::default()
    };
    let mut current_state = blank_state();
    current_state.dom_state = SerializedDomState::from_elements(vec![
        replay_dom_element(
            7,
            "button",
            BTreeMap::from([("data-testid".to_owned(), "duplicate".to_owned())]),
        ),
        replay_dom_element(
            8,
            "button",
            BTreeMap::from([("data-testid".to_owned(), "duplicate".to_owned())]),
        ),
    ]);
    let session = MockSession::with_states(vec![current_state]);
    let mut executor = BrowserActionExecutor::new(session);

    let error = executor
        .replay_history(&history)
        .await
        .expect_err("plan error");

    let AgentHistoryReplayRunError::Plan { error } = error else {
        panic!("expected plan error");
    };
    assert_eq!(error.step_index, 0);
    assert_eq!(error.action_index, 0);
    assert_eq!(
        error.failure.reason,
        DomInteractedElementMatchFailureReason::Ambiguous
    );
    assert_eq!(executor.session().events(), Vec::<String>::new());
    assert_eq!(executor.session().state_screenshot_requests(), vec![false]);
}

#[tokio::test]
async fn browser_executor_rejects_zero_click_index() {
    let session = MockSession::new();
    let mut executor = BrowserActionExecutor::new(session);

    let result = executor
        .execute(&BrowserAction::Click(ClickElementAction {
            index: Some(0),
            coordinate_x: None,
            coordinate_y: None,
        }))
        .await;

    assert!(
        result
            .error
            .as_deref()
            .expect("click error")
            .contains("index 0")
    );
    assert_eq!(executor.session().events(), Vec::<String>::new());
}

#[tokio::test]
async fn browser_executor_click_select_returns_dropdown_options() {
    let session = MockSession::with_click_error(
        "Cannot click on <select> elements. Use get_dropdown_options and select_dropdown_option instead.",
    );
    let mut executor = BrowserActionExecutor::new(session);

    let result = executor
        .execute(&BrowserAction::Click(ClickElementAction {
            index: Some(1),
            coordinate_x: None,
            coordinate_y: None,
        }))
        .await;

    assert_eq!(
        result.extracted_content.as_deref(),
        Some("Dropdown options: One, Two")
    );
    assert_eq!(
        executor.session().events(),
        vec!["click:1", "dropdown_options:1"]
    );
}

#[tokio::test]
async fn browser_executor_click_file_input_points_to_upload_file() {
    let session = MockSession::with_click_error(
        "Cannot click on file input elements. Use upload_file instead.",
    );
    let mut executor = BrowserActionExecutor::new(session);

    let result = executor
        .execute(&BrowserAction::Click(ClickElementAction {
            index: Some(2),
            coordinate_x: None,
            coordinate_y: None,
        }))
        .await;

    assert!(
        result
            .error
            .as_deref()
            .expect("file input click error")
            .contains("upload_file")
    );
    assert_eq!(executor.session().events(), vec!["click:2"]);
}

#[tokio::test]
async fn browser_executor_treats_zero_scroll_index_as_page_scroll() {
    let session = MockSession::new();
    let mut executor = BrowserActionExecutor::new(session);

    let result = executor
        .execute(&BrowserAction::Scroll(ScrollAction {
            down: true,
            pages: 1.0,
            index: Some(0),
        }))
        .await;

    assert_eq!(result.error, None);
    assert_eq!(executor.session().events(), vec!["scroll:None:true:1"]);
}

#[tokio::test]
async fn browser_executor_maps_tab_actions_to_session() {
    let session = MockSession::new();
    let mut executor = BrowserActionExecutor::new(session);

    let switch_result = executor
        .execute(&BrowserAction::SwitchTab(SwitchTabAction {
            tab_id: "target-2".to_owned(),
        }))
        .await;
    let close_result = executor
        .execute(&BrowserAction::CloseTab(CloseTabAction {
            tab_id: "target-1".to_owned(),
        }))
        .await;

    assert_eq!(switch_result.error, None);
    assert_eq!(close_result.error, None);
    assert_eq!(
        executor.session().events(),
        vec!["switch_tab:target-2", "close_tab:target-1"]
    );
}

#[tokio::test]
async fn browser_executor_maps_go_back_to_session() {
    let session = MockSession::new();
    let mut executor = BrowserActionExecutor::new(session);

    let result = executor
        .execute(&BrowserAction::GoBack(NoParamsAction { description: None }))
        .await;

    assert_eq!(result.error, None);
    assert_eq!(result.extracted_content.as_deref(), Some("Navigated back"));
    assert_eq!(executor.session().events(), vec!["go_back"]);
}

#[tokio::test]
async fn browser_executor_maps_find_text_to_session() {
    let session = MockSession::new();
    let mut executor = BrowserActionExecutor::new(session);

    let result = executor
        .execute(&BrowserAction::FindText(FindTextAction {
            text: "Needle".to_owned(),
        }))
        .await;

    assert_eq!(result.error, None);
    assert_eq!(
        result.extracted_content.as_deref(),
        Some("Scrolled to text: Needle")
    );
    assert_eq!(executor.session().events(), vec!["find_text:Needle"]);
}

#[tokio::test]
async fn browser_executor_maps_evaluate_to_session() {
    let session = MockSession::new();
    let mut executor = BrowserActionExecutor::new(session);

    let result = executor
        .execute(&BrowserAction::Evaluate(EvaluateAction {
            code: "document.title".to_owned(),
        }))
        .await;

    assert_eq!(result.error, None);
    assert_eq!(
        result.extracted_content.as_deref(),
        Some("EvalOps JS result")
    );
    assert_eq!(executor.session().events(), vec!["evaluate:document.title"]);
}

#[tokio::test]
async fn browser_executor_handles_text_file_actions() {
    let temp_dir = tempfile::tempdir().expect("temp dir");
    let path = temp_dir.path().join("notes.md");
    let file_name = path.display().to_string();
    let session = MockSession::new();
    let mut executor = BrowserActionExecutor::new(session);

    let write_result = executor
        .execute(&BrowserAction::WriteFile(WriteFileAction {
            file_name: file_name.clone(),
            content: "hello world".to_owned(),
            append: false,
            trailing_newline: true,
            leading_newline: false,
        }))
        .await;
    let append_result = executor
        .execute(&BrowserAction::WriteFile(WriteFileAction {
            file_name: file_name.clone(),
            content: "world".to_owned(),
            append: true,
            trailing_newline: false,
            leading_newline: true,
        }))
        .await;
    let replace_result = executor
        .execute(&BrowserAction::ReplaceFile(ReplaceFileAction {
            file_name: file_name.clone(),
            old_str: "world".to_owned(),
            new_str: "EvalOps".to_owned(),
        }))
        .await;
    let read_result = executor
        .execute(&BrowserAction::ReadFile(ReadFileAction {
            file_name: file_name.clone(),
        }))
        .await;

    assert_eq!(write_result.error, None);
    assert_eq!(append_result.error, None);
    assert_eq!(replace_result.error, None);
    assert_eq!(
        std::fs::read_to_string(&path).expect("file content"),
        "hello EvalOps\n\nEvalOps"
    );
    assert!(
        read_result
            .extracted_content
            .as_deref()
            .expect("read content")
            .contains("hello EvalOps\n\nEvalOps")
    );
    assert!(read_result.include_extracted_content_only_once);
}

#[test]
fn relative_file_action_paths_resolve_like_upstream_filesystem() {
    let resolved = resolve_file_action_path("../nested/test@file.MD", supported_text_extensions());

    assert_eq!(resolved.path, std::path::PathBuf::from("testfile.md"));
    assert_eq!(resolved.display_name, "testfile.md");
    assert!(resolved.was_corrected);

    let spaced = resolve_file_action_path("report (1).csv", supported_text_extensions());
    assert_eq!(spaced.path, std::path::PathBuf::from("report (1).csv"));
    assert!(!spaced.was_corrected);

    let absolute =
        resolve_file_action_path("/tmp/evalops/test@file.md", supported_text_extensions());
    assert_eq!(
        absolute.path,
        std::path::PathBuf::from("/tmp/evalops/test@file.md")
    );
    assert!(!absolute.was_corrected);
}

#[test]
fn browser_executor_sanitizes_relative_file_actions_like_upstream() {
    let _lock = CWD_LOCK.lock().expect("cwd lock");
    let temp_dir = tempfile::tempdir().expect("temp dir");
    let _cwd = CurrentDirGuard::enter(temp_dir.path());

    let write_result = write_file_action(&WriteFileAction {
        file_name: "nested/test@file.md".to_owned(),
        content: "old text".to_owned(),
        append: false,
        trailing_newline: false,
        leading_newline: false,
    })
    .expect("write result");
    let append_result = write_file_action(&WriteFileAction {
        file_name: "nested/test@file.md".to_owned(),
        content: "\nmore text".to_owned(),
        append: true,
        trailing_newline: false,
        leading_newline: false,
    })
    .expect("append result");
    let replace_result =
        replace_file_action("nested/test@file.md", "old", "new").expect("replace result");
    let read_result = read_file_action("nested/test@file.md").expect("read result");

    assert_eq!(
        write_result.extracted_content.as_deref(),
        Some("Wrote file testfile.md (auto-corrected from 'nested/test@file.md')")
    );
    assert_eq!(
        append_result.extracted_content.as_deref(),
        Some("Appended to file testfile.md (auto-corrected from 'nested/test@file.md')")
    );
    assert_eq!(
        replace_result.extracted_content.as_deref(),
        Some("Replaced text in file testfile.md (auto-corrected from 'nested/test@file.md')")
    );
    assert_eq!(
        std::fs::read_to_string(temp_dir.path().join("testfile.md")).expect("sanitized file"),
        "new text\nmore text"
    );
    assert!(!temp_dir.path().join("nested").exists());
    let read_content = read_result
        .extracted_content
        .as_deref()
        .expect("read content");
    assert!(read_content.contains(
        "Note: filename was auto-corrected from 'nested/test@file.md' to 'testfile.md'."
    ));
    assert!(read_content.contains("new text\nmore text"));
}

#[test]
fn managed_file_system_initializes_default_sandbox() {
    let temp_dir = tempfile::tempdir().expect("temp dir");
    let data_dir = temp_dir.path().join(DEFAULT_FILE_SYSTEM_PATH);
    std::fs::create_dir_all(&data_dir).expect("seed data dir");
    std::fs::write(data_dir.join("stale.md"), "stale").expect("seed stale file");

    let file_system = ManagedFileSystem::new(temp_dir.path()).expect("managed file system");

    assert_eq!(file_system.base_dir(), temp_dir.path());
    assert_eq!(file_system.data_dir(), data_dir.as_path());
    assert_eq!(file_system.list_files(), vec!["todo.md"]);
    assert_eq!(file_system.get_todo_contents(), "");
    assert!(data_dir.join("todo.md").exists());
    assert!(!data_dir.join("stale.md").exists());
}

#[test]
fn managed_file_system_round_trips_state_and_artifacts() {
    let temp_dir = tempfile::tempdir().expect("temp dir");
    let mut file_system = ManagedFileSystem::new(temp_dir.path()).expect("managed file system");
    let original = "nested/report@ data.CSV";
    let resolved = "report-data.csv";

    let write_result = file_system
        .write_file(&WriteFileAction {
            file_name: original.to_owned(),
            content: "name,value\nalice,one".to_owned(),
            append: false,
            trailing_newline: false,
            leading_newline: false,
        })
        .expect("managed write");
    assert_eq!(write_result.error, None);
    assert_eq!(
        write_result.extracted_content.as_deref(),
        Some("Wrote file report-data.csv (auto-corrected from 'nested/report@ data.CSV')")
    );

    let append_result = file_system
        .write_file(&WriteFileAction {
            file_name: original.to_owned(),
            content: "\nbob,two".to_owned(),
            append: true,
            trailing_newline: false,
            leading_newline: false,
        })
        .expect("managed append");
    assert_eq!(append_result.error, None);

    let replace_result = file_system
        .replace_file(original, "alice", "ada")
        .expect("managed replace");
    assert_eq!(replace_result.error, None);

    let read_result = file_system.read_file(original).expect("managed read");
    let read_content = read_result.extracted_content.as_deref().expect("read");
    assert!(read_content.contains(
        "Note: filename was auto-corrected from 'nested/report@ data.CSV' to 'report-data.csv'."
    ));
    assert!(read_content.contains("ada,one"));
    assert!(read_content.contains("bob,two"));

    let data_path = temp_dir
        .path()
        .join(DEFAULT_FILE_SYSTEM_PATH)
        .join(resolved);
    assert_eq!(
        std::fs::read_to_string(&data_path).expect("managed csv"),
        "name,value\nada,one\nbob,two"
    );
    assert_eq!(
        file_system.display_file(original).as_deref(),
        Some("name,value\nada,one\nbob,two")
    );
    let description = file_system.describe();
    assert!(description.contains("<file>\nreport-data.csv - 3 lines"));
    assert!(!description.contains("todo.md"));

    let extracted_0 = file_system
        .save_extracted_content("first extracted")
        .expect("first extracted");
    let extracted_1 = file_system
        .save_extracted_content("second extracted")
        .expect("second extracted");
    assert_eq!(extracted_0, "extracted_content_0.md");
    assert_eq!(extracted_1, "extracted_content_1.md");
    assert_eq!(file_system.get_state().extracted_content_count, 2);

    let state = file_system.get_state();
    let mut restored = ManagedFileSystem::from_state(state).expect("restore file system");
    assert_eq!(restored.get_state().extracted_content_count, 2);
    assert_eq!(
        restored
            .display_file("report-data.csv")
            .expect("restored report"),
        "name,value\nada,one\nbob,two"
    );
    assert_eq!(
        std::fs::read_to_string(
            temp_dir
                .path()
                .join(DEFAULT_FILE_SYSTEM_PATH)
                .join("extracted_content_1.md")
        )
        .expect("restored extracted content"),
        "second extracted"
    );
    let extracted_2 = restored
        .save_extracted_content("third extracted")
        .expect("third extracted");
    assert_eq!(extracted_2, "extracted_content_2.md");

    restored.nuke().expect("nuke file system");
    assert!(restored.list_files().is_empty());
    assert!(!restored.data_dir().exists());
}

#[test]
fn managed_file_system_syncs_pdf_and_docx_artifacts() {
    let temp_dir = tempfile::tempdir().expect("temp dir");
    let mut file_system = ManagedFileSystem::new(temp_dir.path()).expect("managed file system");

    file_system
        .write_file(&WriteFileAction {
            file_name: "report.pdf".to_owned(),
            content: "# Report\nPDF body".to_owned(),
            append: false,
            trailing_newline: false,
            leading_newline: false,
        })
        .expect("pdf write");
    file_system
        .write_file(&WriteFileAction {
            file_name: "brief.docx".to_owned(),
            content: "DOCX body".to_owned(),
            append: false,
            trailing_newline: false,
            leading_newline: false,
        })
        .expect("docx write");

    let pdf_path = file_system.data_dir().join("report.pdf");
    let docx_path = file_system.data_dir().join("brief.docx");
    assert!(
        std::fs::read(&pdf_path)
            .expect("pdf bytes")
            .starts_with(b"%PDF-1.4")
    );
    assert!(zip::ZipArchive::new(std::fs::File::open(docx_path).expect("docx file")).is_ok());
    assert_eq!(
        file_system
            .get_state()
            .files
            .get("report.pdf")
            .expect("pdf state")
            .data
            .content,
        "# Report\nPDF body"
    );
}

#[tokio::test]
async fn browser_executor_routes_relative_file_actions_through_managed_sandbox() {
    let temp_dir = tempfile::tempdir().expect("temp dir");
    let unique = format!(
        "executor-sandbox-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system time")
            .as_nanos()
    );
    let original = format!("nested/{unique}@ report.MD");
    let resolved = format!("{unique}-report.md");
    let cwd_candidate = std::env::current_dir()
        .expect("current dir")
        .join(&resolved);
    assert!(!cwd_candidate.exists());

    let session = MockSession::new();
    let file_system = ManagedFileSystem::new(temp_dir.path()).expect("managed file system");
    let mut executor = BrowserActionExecutor::with_file_system(session, file_system);

    let write_result = executor
        .execute(&BrowserAction::WriteFile(WriteFileAction {
            file_name: original.clone(),
            content: "alpha".to_owned(),
            append: false,
            trailing_newline: false,
            leading_newline: false,
        }))
        .await;
    let append_result = executor
        .execute(&BrowserAction::WriteFile(WriteFileAction {
            file_name: original.clone(),
            content: "\nbeta".to_owned(),
            append: true,
            trailing_newline: false,
            leading_newline: false,
        }))
        .await;
    let replace_result = executor
        .execute(&BrowserAction::ReplaceFile(ReplaceFileAction {
            file_name: original.clone(),
            old_str: "alpha".to_owned(),
            new_str: "gamma".to_owned(),
        }))
        .await;
    let read_result = executor
        .execute(&BrowserAction::ReadFile(ReadFileAction {
            file_name: original.clone(),
        }))
        .await;
    let done_result = executor
        .execute(&BrowserAction::Done(DoneAction {
            text: "Done".to_owned(),
            success: true,
            files_to_display: vec![original.clone()],
        }))
        .await;

    assert_eq!(write_result.error, None);
    assert_eq!(append_result.error, None);
    assert_eq!(replace_result.error, None);
    assert_eq!(read_result.error, None);
    assert_eq!(done_result.error, None);

    let sandbox_file = executor.file_system().data_dir().join(&resolved);
    assert_eq!(
        std::fs::read_to_string(&sandbox_file).expect("sandbox file"),
        "gamma\nbeta"
    );
    assert!(!cwd_candidate.exists());

    let read_content = read_result.extracted_content.as_deref().expect("read");
    assert!(read_content.contains(&format!(
        "Note: filename was auto-corrected from '{original}' to '{resolved}'."
    )));
    assert!(read_content.contains("gamma\nbeta"));

    let done_content = done_result.extracted_content.as_deref().expect("done");
    assert!(done_content.contains("Done\n\nAttachments:"));
    assert!(done_content.contains(&format!("{resolved}:\ngamma\nbeta")));
    assert_eq!(
        done_result.attachments,
        vec![
            std::fs::canonicalize(sandbox_file)
                .expect("canonical sandbox file")
                .display()
                .to_string()
        ]
    );
}

#[tokio::test]
async fn browser_executor_preserves_absolute_file_action_paths() {
    let temp_dir = tempfile::tempdir().expect("temp dir");
    let file_system_dir = temp_dir.path().join("agent");
    let external_file = temp_dir.path().join("external.md");
    let external_file_name = external_file.display().to_string();
    let session = MockSession::new();
    let file_system = ManagedFileSystem::new(&file_system_dir).expect("managed file system");
    let mut executor = BrowserActionExecutor::with_file_system(session, file_system);

    let write_result = executor
        .execute(&BrowserAction::WriteFile(WriteFileAction {
            file_name: external_file_name.clone(),
            content: "external".to_owned(),
            append: false,
            trailing_newline: false,
            leading_newline: false,
        }))
        .await;
    let read_result = executor
        .execute(&BrowserAction::ReadFile(ReadFileAction {
            file_name: external_file_name.clone(),
        }))
        .await;

    assert_eq!(write_result.error, None);
    assert_eq!(read_result.error, None);
    assert_eq!(
        std::fs::read_to_string(&external_file).expect("external file"),
        "external"
    );
    assert!(
        !executor
            .file_system()
            .data_dir()
            .join("external.md")
            .exists()
    );
    assert!(
        read_result
            .extracted_content
            .as_deref()
            .expect("absolute read")
            .contains("external")
    );
}

#[tokio::test]
async fn browser_executor_append_file_requires_existing_file_like_upstream() {
    let temp_dir = tempfile::tempdir().expect("temp dir");
    let text_path = temp_dir.path().join("missing.md");
    let csv_path = temp_dir.path().join("missing.csv");
    let pdf_path = temp_dir.path().join("missing.pdf");
    let docx_path = temp_dir.path().join("missing.docx");
    let session = MockSession::new();
    let mut executor = BrowserActionExecutor::new(session);

    let text_result = executor
        .execute(&BrowserAction::WriteFile(WriteFileAction {
            file_name: text_path.display().to_string(),
            content: "hello".to_owned(),
            append: true,
            trailing_newline: true,
            leading_newline: false,
        }))
        .await;
    let csv_result = executor
        .execute(&BrowserAction::WriteFile(WriteFileAction {
            file_name: csv_path.display().to_string(),
            content: "name,value".to_owned(),
            append: true,
            trailing_newline: true,
            leading_newline: false,
        }))
        .await;
    let pdf_result = executor
        .execute(&BrowserAction::WriteFile(WriteFileAction {
            file_name: pdf_path.display().to_string(),
            content: "pdf".to_owned(),
            append: true,
            trailing_newline: true,
            leading_newline: false,
        }))
        .await;
    let docx_result = executor
        .execute(&BrowserAction::WriteFile(WriteFileAction {
            file_name: docx_path.display().to_string(),
            content: "docx".to_owned(),
            append: true,
            trailing_newline: true,
            leading_newline: false,
        }))
        .await;

    let expected_text_error = format!("File '{}' not found.", text_path.display());
    let expected_csv_error = format!("File '{}' not found.", csv_path.display());
    let expected_pdf_error = format!("File '{}' not found.", pdf_path.display());
    let expected_docx_error = format!("File '{}' not found.", docx_path.display());
    assert_eq!(
        text_result.error.as_deref(),
        Some(expected_text_error.as_str())
    );
    assert_eq!(
        csv_result.error.as_deref(),
        Some(expected_csv_error.as_str())
    );
    assert_eq!(
        pdf_result.error.as_deref(),
        Some(expected_pdf_error.as_str())
    );
    assert_eq!(
        docx_result.error.as_deref(),
        Some(expected_docx_error.as_str())
    );
    assert!(!text_path.exists());
    assert!(!csv_path.exists());
    assert!(!pdf_path.exists());
    assert!(!docx_path.exists());
}

#[tokio::test]
async fn browser_executor_normalizes_csv_file_actions_like_upstream() {
    let temp_dir = tempfile::tempdir().expect("temp dir");
    let path = temp_dir.path().join("contacts.csv");
    let file_name = path.display().to_string();
    let session = MockSession::new();
    let mut executor = BrowserActionExecutor::new(session);

    let write_result = executor
        .execute(&BrowserAction::WriteFile(WriteFileAction {
            file_name: file_name.clone(),
            content: r#"name,notes\nAda,\"likes, commas\""#.to_owned(),
            append: false,
            trailing_newline: true,
            leading_newline: false,
        }))
        .await;
    let append_result = executor
        .execute(&BrowserAction::WriteFile(WriteFileAction {
            file_name: file_name.clone(),
            content: r#"Grace,"ships, tests""#.to_owned(),
            append: true,
            trailing_newline: true,
            leading_newline: false,
        }))
        .await;
    let read_result = executor
        .execute(&BrowserAction::ReadFile(ReadFileAction {
            file_name: file_name.clone(),
        }))
        .await;

    assert_eq!(write_result.error, None);
    assert_eq!(append_result.error, None);
    assert_eq!(
        std::fs::read_to_string(&path).expect("csv content"),
        "name,notes\nAda,\"likes, commas\"\nGrace,\"ships, tests\""
    );
    assert!(
        read_result
            .extracted_content
            .as_deref()
            .expect("read content")
            .contains("Ada,\"likes, commas\"\nGrace,\"ships, tests\"")
    );
}

#[tokio::test]
async fn browser_executor_reads_image_files_as_one_time_image_payloads() {
    let temp_dir = tempfile::tempdir().expect("temp dir");
    let path = temp_dir.path().join("chart.png");
    std::fs::write(&path, b"PNGDATA").expect("seed image");
    let file_name = path.display().to_string();
    let session = MockSession::new();
    let mut executor = BrowserActionExecutor::new(session);

    let result = executor
        .execute(&BrowserAction::ReadFile(ReadFileAction {
            file_name: file_name.clone(),
        }))
        .await;

    assert_eq!(result.error, None);
    assert_eq!(
        result.extracted_content.as_deref(),
        Some(format!("Read image file {file_name}.").as_str())
    );
    assert_eq!(
        result.long_term_memory.as_deref(),
        Some(format!("Read image file {file_name}").as_str())
    );
    assert!(result.include_extracted_content_only_once);
    assert_eq!(result.images.len(), 1);
    assert_eq!(result.images[0]["name"], "chart.png");
    assert_eq!(
        result.images[0]["data"],
        base64::engine::general_purpose::STANDARD.encode("PNGDATA")
    );
}

#[tokio::test]
async fn browser_executor_reads_docx_files_as_text() {
    let temp_dir = tempfile::tempdir().expect("temp dir");
    let path = temp_dir.path().join("report.docx");
    write_minimal_docx(
        &path,
        r#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body><w:p><w:r><w:t>Alpha</w:t></w:r></w:p><w:p><w:r><w:t>Beta</w:t></w:r></w:p></w:body></w:document>"#,
    );
    let file_name = path.display().to_string();
    let session = MockSession::new();
    let mut executor = BrowserActionExecutor::new(session);

    let result = executor
        .execute(&BrowserAction::ReadFile(ReadFileAction {
            file_name: file_name.clone(),
        }))
        .await;

    assert_eq!(result.error, None);
    let content = result.extracted_content.as_deref().expect("docx content");
    assert!(content.contains(&format!("Read from file {file_name}.")));
    assert!(content.contains("<content>\nAlpha\nBeta\n</content>"));
    assert_eq!(result.long_term_memory.as_deref(), Some("Alpha\nBeta"));
    assert!(result.include_extracted_content_only_once);
}

#[tokio::test]
async fn browser_executor_writes_and_appends_docx_files_as_text() {
    let temp_dir = tempfile::tempdir().expect("temp dir");
    let path = temp_dir.path().join("report.docx");
    let file_name = path.display().to_string();
    let session = MockSession::new();
    let mut executor = BrowserActionExecutor::new(session);

    let write_result = executor
        .execute(&BrowserAction::WriteFile(WriteFileAction {
            file_name: file_name.clone(),
            content: "Title <One>\nAlpha\tBeta".to_owned(),
            append: false,
            trailing_newline: false,
            leading_newline: false,
        }))
        .await;
    let append_result = executor
        .execute(&BrowserAction::WriteFile(WriteFileAction {
            file_name: file_name.clone(),
            content: "Gamma & Delta".to_owned(),
            append: true,
            trailing_newline: false,
            leading_newline: false,
        }))
        .await;
    let read_result = executor
        .execute(&BrowserAction::ReadFile(ReadFileAction {
            file_name: file_name.clone(),
        }))
        .await;

    assert_eq!(write_result.error, None);
    assert_eq!(append_result.error, None);
    assert_eq!(
        read_docx_text(&file_name).expect("docx text"),
        "Title <One>\nAlpha\tBeta\nGamma & Delta"
    );
    let bytes = std::fs::read(&path).expect("docx bytes");
    assert_eq!(&bytes[..2], b"PK");
    let content = read_result.extracted_content.as_deref().expect("docx read");
    assert!(content.contains(&format!("Read from file {file_name}.")));
    assert!(content.contains("Title <One>\nAlpha\tBeta\nGamma & Delta"));
}

#[tokio::test]
async fn browser_executor_reads_pdf_files_as_text() {
    let temp_dir = tempfile::tempdir().expect("temp dir");
    let path = temp_dir.path().join("report.pdf");
    std::fs::write(&path, minimal_pdf("Hello PDF EvalOps")).expect("seed pdf");
    let file_name = path.display().to_string();
    let session = MockSession::new();
    let mut executor = BrowserActionExecutor::new(session);

    let result = executor
        .execute(&BrowserAction::ReadFile(ReadFileAction {
            file_name: file_name.clone(),
        }))
        .await;

    assert_eq!(result.error, None);
    let content = result.extracted_content.as_deref().expect("pdf content");
    assert!(content.contains(&format!("Read from file {file_name} (1 pages,")));
    assert!(content.contains("<content>\n--- Page 1 ---"));
    assert!(content.contains("Hello PDF EvalOps"));
    assert!(
        result
            .long_term_memory
            .as_deref()
            .expect("pdf memory")
            .contains("Hello PDF EvalOps")
    );
    assert!(result.include_extracted_content_only_once);
}

#[tokio::test]
async fn browser_executor_writes_and_appends_pdf_files_as_text() {
    let temp_dir = tempfile::tempdir().expect("temp dir");
    let path = temp_dir.path().join("report.pdf");
    let file_name = path.display().to_string();
    let session = MockSession::new();
    let mut executor = BrowserActionExecutor::new(session);

    let write_result = executor
        .execute(&BrowserAction::WriteFile(WriteFileAction {
            file_name: file_name.clone(),
            content: "# Report\nAlpha\tBeta (Gamma)".to_owned(),
            append: false,
            trailing_newline: false,
            leading_newline: false,
        }))
        .await;
    let append_result = executor
        .execute(&BrowserAction::WriteFile(WriteFileAction {
            file_name: file_name.clone(),
            content: r"Second \ line".to_owned(),
            append: true,
            trailing_newline: false,
            leading_newline: false,
        }))
        .await;
    let read_result = executor
        .execute(&BrowserAction::ReadFile(ReadFileAction {
            file_name: file_name.clone(),
        }))
        .await;

    assert_eq!(write_result.error, None);
    assert_eq!(append_result.error, None);
    let bytes = std::fs::read(&path).expect("pdf bytes");
    assert!(bytes.starts_with(b"%PDF-1.4"));
    let extracted = pdf_extract::extract_text(&path).expect("pdf text");
    assert!(extracted.contains("Report"));
    assert!(!extracted.contains("# Report"));
    assert!(extracted.contains("Alpha"));
    assert!(extracted.contains("Beta (Gamma)"));
    assert!(extracted.contains(r"Second \ line"));
    let content = read_result.extracted_content.as_deref().expect("pdf read");
    assert!(content.contains(&format!("Read from file {file_name} (1 pages,")));
    assert!(content.contains("--- Page 1 ---"));
    assert!(content.contains("Report"));
    assert!(content.contains("Beta (Gamma)"));
    assert!(content.contains(r"Second \ line"));
}

#[test]
fn pdf_document_bytes_paginates_long_content() {
    let content = (0..80)
        .map(|index| format!("Line {index}: PDF pagination parity"))
        .collect::<Vec<_>>()
        .join("\n");
    let bytes = pdf_document_bytes(&content);
    let pdf = String::from_utf8_lossy(&bytes);

    assert!(bytes.starts_with(b"%PDF-1.4"));
    assert!(pdf.matches("/Type /Page /Parent").count() > 1);
}

#[test]
fn pdf_read_envelope_marks_pages_for_small_documents() {
    let pages = vec![
        "Cover text".to_owned(),
        String::new(),
        "Appendix text".to_owned(),
    ];
    let envelope = render_pdf_read_envelope("report.pdf", &pages);

    assert!(envelope.contains("Read from file report.pdf (3 pages, 23 chars)."));
    assert!(envelope.contains("--- Page 1 ---\nCover text"));
    assert!(!envelope.contains("--- Page 2 ---"));
    assert!(envelope.contains("--- Page 3 ---\nAppendix text"));
}

#[test]
fn pdf_read_envelope_prioritizes_and_truncates_large_documents() {
    let pages = (1..=20)
        .map(|page_number| {
            let distinctive = if page_number == 17 {
                "needleword uniqueword "
            } else {
                ""
            };
            format!(
                "Page {page_number} {distinctive}{}",
                "commonword ".repeat(700)
            )
        })
        .collect::<Vec<_>>();

    let envelope = render_pdf_read_envelope("large.pdf", &pages);

    assert!(envelope.contains("Read from file large.pdf (20 pages,"));
    assert!(envelope.contains("chars total)."));
    assert!(envelope.contains("--- Page 1 ---"));
    assert!(envelope.contains("--- Page 17 ---"));
    assert!(envelope.contains("[Showing "));
    assert!(envelope.contains("Skipped pages: ["));
    assert!(envelope.chars().count() < PDF_READ_MAX_CHARS + 500);
}

fn minimal_pdf(text: &str) -> Vec<u8> {
    let escaped = text
        .replace('\\', r"\\")
        .replace('(', r"\(")
        .replace(')', r"\)");
    let stream = format!("BT /F1 24 Tf 72 720 Td ({escaped}) Tj ET");
    let objects = [
            "<< /Type /Catalog /Pages 2 0 R >>".to_owned(),
            "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_owned(),
            "<< /Type /Page /Parent 2 0 R /Resources << /Font << /F1 4 0 R >> >> /MediaBox [0 0 612 792] /Contents 5 0 R >>".to_owned(),
            "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_owned(),
            format!(
                "<< /Length {} >>\nstream\n{}\nendstream",
                stream.len(),
                stream
            ),
        ];

    let mut pdf = b"%PDF-1.4\n".to_vec();
    let mut offsets = Vec::new();
    for (index, object) in objects.iter().enumerate() {
        offsets.push(pdf.len());
        pdf.extend_from_slice(format!("{} 0 obj\n{}\nendobj\n", index + 1, object).as_bytes());
    }
    let xref_offset = pdf.len();
    pdf.extend_from_slice(format!("xref\n0 {}\n", objects.len() + 1).as_bytes());
    pdf.extend_from_slice(b"0000000000 65535 f \n");
    for offset in offsets {
        pdf.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    pdf.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{}\n%%EOF\n",
            objects.len() + 1,
            xref_offset
        )
        .as_bytes(),
    );
    pdf
}

#[tokio::test]
async fn browser_executor_done_displays_requested_text_files() {
    let temp_dir = tempfile::tempdir().expect("temp dir");
    let path = temp_dir.path().join("report.md");
    std::fs::write(&path, "alpha\nbeta").expect("seed report");
    let file_name = path.display().to_string();
    let missing_file = temp_dir.path().join("missing.md").display().to_string();
    let session = MockSession::new();
    let mut executor = BrowserActionExecutor::new(session);

    let result = executor
        .execute(&BrowserAction::Done(DoneAction {
            text: "Complete".to_owned(),
            success: true,
            files_to_display: vec![file_name.clone(), missing_file],
        }))
        .await;

    assert_eq!(result.error, None);
    assert!(result.is_done);
    assert_eq!(result.success, Some(true));
    let content = result.extracted_content.as_deref().expect("done content");
    assert!(content.starts_with("Complete\n\nAttachments:"));
    assert!(content.contains(&format!("{file_name}:\nalpha\nbeta")));
    assert_eq!(
        result.attachments,
        vec![
            std::fs::canonicalize(&path)
                .expect("canonical report path")
                .display()
                .to_string()
        ]
    );
}

#[tokio::test]
async fn browser_executor_done_can_attach_without_displaying_file_text() {
    let temp_dir = tempfile::tempdir().expect("temp dir");
    let path = temp_dir.path().join("report.md");
    std::fs::write(&path, "alpha\nbeta").expect("seed report");
    let file_name = path.display().to_string();
    let session = MockSession::new();
    let mut executor = BrowserActionExecutor::new(session);
    executor.set_display_files_in_done_text(false);

    let result = executor
        .execute(&BrowserAction::Done(DoneAction {
            text: "Complete".to_owned(),
            success: true,
            files_to_display: vec![file_name],
        }))
        .await;

    assert_eq!(result.error, None);
    assert!(result.is_done);
    assert_eq!(result.extracted_content.as_deref(), Some("Complete"));
    assert_eq!(
        result.attachments,
        vec![
            std::fs::canonicalize(&path)
                .expect("canonical report path")
                .display()
                .to_string()
        ]
    );
}

fn write_minimal_docx(path: &std::path::Path, document_xml: &str) {
    let file = std::fs::File::create(path).expect("docx file");
    let mut archive = zip::ZipWriter::new(file);
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);

    archive
        .start_file("[Content_Types].xml", options)
        .expect("content types entry");
    archive
            .write_all(
                br#"<?xml version="1.0" encoding="UTF-8"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="xml" ContentType="application/xml"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/></Types>"#,
            )
            .expect("content types xml");
    archive
        .start_file("word/document.xml", options)
        .expect("document entry");
    archive
        .write_all(document_xml.as_bytes())
        .expect("document xml");
    archive.finish().expect("finish docx");
}

#[tokio::test]
async fn browser_executor_rejects_unsupported_text_file_actions() {
    let temp_dir = tempfile::tempdir().expect("temp dir");
    let session = MockSession::new();
    let mut executor = BrowserActionExecutor::new(session);

    let binary_result = executor
        .execute(&BrowserAction::WriteFile(WriteFileAction {
            file_name: temp_dir.path().join("image.png").display().to_string(),
            content: "not really an image".to_owned(),
            append: false,
            trailing_newline: true,
            leading_newline: false,
        }))
        .await;
    let svg_write_result = executor
        .execute(&BrowserAction::WriteFile(WriteFileAction {
            file_name: temp_dir.path().join("diagram.svg").display().to_string(),
            content: "<svg />".to_owned(),
            append: false,
            trailing_newline: true,
            leading_newline: false,
        }))
        .await;
    let audio_read_result = executor
        .execute(&BrowserAction::ReadFile(ReadFileAction {
            file_name: temp_dir.path().join("clip.mp3").display().to_string(),
        }))
        .await;
    let extensionless_result = executor
        .execute(&BrowserAction::ReadFile(ReadFileAction {
            file_name: temp_dir.path().join("notes").display().to_string(),
        }))
        .await;

    let editable_path = temp_dir.path().join("notes.md");
    std::fs::write(&editable_path, "hello").expect("seed editable file");
    let empty_replace_result = executor
        .execute(&BrowserAction::ReplaceFile(ReplaceFileAction {
            file_name: editable_path.display().to_string(),
            old_str: String::new(),
            new_str: "EvalOps".to_owned(),
        }))
        .await;
    let dll_replace_result = executor
        .execute(&BrowserAction::ReplaceFile(ReplaceFileAction {
            file_name: temp_dir.path().join("plugin.dll").display().to_string(),
            old_str: "old".to_owned(),
            new_str: "new".to_owned(),
        }))
        .await;

    assert!(
        binary_result
            .error
            .as_deref()
            .expect("binary error")
            .contains("binary/image file")
    );
    assert!(
        svg_write_result
            .error
            .as_deref()
            .expect("svg binary error")
            .contains("binary/image file")
    );
    assert!(
        audio_read_result
            .error
            .as_deref()
            .expect("mp3 binary error")
            .contains("binary/image file")
    );
    assert!(
        extensionless_result
            .error
            .as_deref()
            .expect("extension error")
            .contains("has no extension")
    );
    assert!(
        empty_replace_result
            .error
            .as_deref()
            .expect("empty replace error")
            .contains("Cannot replace empty string")
    );
    assert!(
        dll_replace_result
            .error
            .as_deref()
            .expect("dll binary error")
            .contains("binary/image file")
    );
    assert!(!temp_dir.path().join("diagram.svg").exists());
    assert_eq!(
        std::fs::read_to_string(editable_path).expect("editable content"),
        "hello"
    );
}

#[tokio::test]
async fn browser_executor_maps_dropdown_actions_to_session() {
    let session = MockSession::new();
    let mut executor = BrowserActionExecutor::new(session);

    let options_result = executor
        .execute(&BrowserAction::GetDropdownOptions(
            GetDropdownOptionsAction { index: 1 },
        ))
        .await;
    let select_result = executor
        .execute(&BrowserAction::SelectDropdownOption(
            SelectDropdownOptionAction {
                index: 1,
                text: "Two".to_owned(),
            },
        ))
        .await;

    assert!(
        options_result
            .extracted_content
            .expect("options content")
            .contains("One, Two")
    );
    assert!(select_result.error.is_none());
    assert_eq!(
        executor.session().events(),
        vec!["dropdown_options:1", "select_dropdown_option:1:Two"]
    );
}

#[tokio::test]
async fn browser_executor_maps_send_keys_to_session() {
    let session = MockSession::new();
    let mut executor = BrowserActionExecutor::new(session);

    let result = executor
        .execute(&BrowserAction::SendKeys(SendKeysAction {
            keys: "EvalOps".to_owned(),
        }))
        .await;

    assert_eq!(result.error, None);
    assert_eq!(executor.session().events(), vec!["send_keys:EvalOps"]);
}

#[tokio::test]
async fn browser_executor_maps_wait_without_session_event() {
    let session = MockSession::new();
    let mut executor = BrowserActionExecutor::new(session);

    let result = executor
        .execute(&BrowserAction::Wait(WaitAction { seconds: 0 }))
        .await;

    assert_eq!(result.error, None);
    assert_eq!(
        result.extracted_content.as_deref(),
        Some("Waited for 0 seconds")
    );
    assert_eq!(executor.session().events(), Vec::<String>::new());
}

#[tokio::test]
async fn browser_executor_requests_next_screenshot_observation() {
    let session = MockSession::new();
    let mut executor = BrowserActionExecutor::new(session);

    let result = executor
        .execute(&BrowserAction::Screenshot(ScreenshotAction {
            file_name: None,
        }))
        .await;

    assert_eq!(result.error, None);
    assert_eq!(
        result.extracted_content.as_deref(),
        Some("Requested screenshot for next observation")
    );
    assert_eq!(
        result
            .metadata
            .as_ref()
            .and_then(|metadata| metadata.get("include_screenshot"))
            .and_then(Value::as_bool),
        Some(true)
    );
    assert_eq!(executor.session().events(), Vec::<String>::new());
}

#[tokio::test]
async fn browser_executor_saves_screenshot_when_file_name_is_present() {
    let temp_dir = tempfile::tempdir().expect("temp dir");
    let requested_output_path = temp_dir.path().join("shot");
    let output_path = temp_dir.path().join("shot.png");
    let session = MockSession::new();
    let mut executor = BrowserActionExecutor::new(session);

    let result = executor
        .execute(&BrowserAction::Screenshot(ScreenshotAction {
            file_name: Some(requested_output_path.display().to_string()),
        }))
        .await;

    assert_eq!(result.error, None);
    assert!(
        result
            .extracted_content
            .expect("screenshot content")
            .contains(&output_path.display().to_string())
    );
    assert_eq!(result.attachments, vec![output_path.display().to_string()]);
    assert_eq!(
        std::fs::read(&output_path).expect("screenshot file"),
        b"PNGDATA"
    );
    assert_eq!(executor.session().events(), vec!["screenshot"]);
}

#[test]
fn screenshot_output_path_appends_png_extension() {
    assert_eq!(
        screenshot_output_path("/tmp/shot"),
        std::path::PathBuf::from("/tmp/shot.png")
    );
    assert_eq!(
        screenshot_output_path("/tmp/shot.PNG"),
        std::path::PathBuf::from("/tmp/shot.PNG")
    );
    assert_eq!(
        screenshot_output_path(""),
        std::path::PathBuf::from("screenshot.png")
    );
}

#[tokio::test]
async fn browser_executor_maps_upload_file_to_session() {
    let session = MockSession::new();
    let mut executor = BrowserActionExecutor::new(session);

    let result = executor
        .execute(&BrowserAction::UploadFile(UploadFileAction {
            index: 3,
            path: "/tmp/evalops-upload.txt".to_owned(),
        }))
        .await;

    assert_eq!(result.error, None);
    assert_eq!(
        result.extracted_content.as_deref(),
        Some("Uploaded /tmp/evalops-upload.txt to element 3")
    );
    assert_eq!(
        executor.session().events(),
        vec!["upload_file:3:/tmp/evalops-upload.txt"]
    );
}

#[tokio::test]
async fn browser_executor_enforces_available_upload_paths_when_enabled() {
    let temp_dir = tempfile::tempdir().expect("temp dir");
    let upload_path = temp_dir.path().join("allowed.txt");
    std::fs::write(&upload_path, "upload me").expect("upload file");
    let session = MockSession::new();
    let mut executor = BrowserActionExecutor::new(session);
    executor.set_upload_file_availability(true, vec![upload_path.display().to_string()]);

    let result = executor
        .execute(&BrowserAction::UploadFile(UploadFileAction {
            index: 3,
            path: upload_path.display().to_string(),
        }))
        .await;

    assert_eq!(result.error, None);
    assert_eq!(
        executor.session().events(),
        vec![format!("upload_file:3:{}", upload_path.display())]
    );
}

#[tokio::test]
async fn browser_executor_rejects_unavailable_upload_paths_before_session_call() {
    let session = MockSession::new();
    let mut executor = BrowserActionExecutor::new(session);
    executor.set_upload_file_availability(true, Vec::new());

    let result = executor
        .execute(&BrowserAction::UploadFile(UploadFileAction {
            index: 3,
            path: "/tmp/not-declared.txt".to_owned(),
        }))
        .await;

    assert!(
        result
            .error
            .as_deref()
            .expect("upload error")
            .contains("AgentSettings.available_file_paths")
    );
    assert!(executor.session().events().is_empty());
}

#[tokio::test]
async fn browser_executor_rejects_empty_available_upload_files() {
    let temp_dir = tempfile::tempdir().expect("temp dir");
    let upload_path = temp_dir.path().join("empty.txt");
    std::fs::write(&upload_path, "").expect("empty upload file");
    let session = MockSession::new();
    let mut executor = BrowserActionExecutor::new(session);
    executor.set_upload_file_availability(true, vec![upload_path.display().to_string()]);

    let result = executor
        .execute(&BrowserAction::UploadFile(UploadFileAction {
            index: 3,
            path: upload_path.display().to_string(),
        }))
        .await;

    assert!(
        result
            .error
            .as_deref()
            .expect("upload error")
            .contains("empty (0 bytes)")
    );
    assert!(executor.session().events().is_empty());
}

#[tokio::test]
async fn browser_executor_uploads_managed_files_by_relative_name() {
    let temp_dir = tempfile::tempdir().expect("temp dir");
    let session = MockSession::new();
    let file_system = ManagedFileSystem::new(temp_dir.path()).expect("managed file system");
    let mut executor = BrowserActionExecutor::with_file_system(session, file_system);
    executor.set_upload_file_availability(true, Vec::new());
    executor
        .execute(&BrowserAction::WriteFile(WriteFileAction {
            file_name: "report.md".to_owned(),
            content: "upload me".to_owned(),
            append: false,
            trailing_newline: false,
            leading_newline: false,
        }))
        .await;
    let managed_path = executor.file_system().data_dir().join("report.md");

    let result = executor
        .execute(&BrowserAction::UploadFile(UploadFileAction {
            index: 3,
            path: "report.md".to_owned(),
        }))
        .await;

    assert_eq!(result.error, None);
    assert_eq!(
        executor.session().events(),
        vec![format!("upload_file:3:{}", managed_path.display())]
    );
}

#[test]
fn managed_upload_file_path_contains_traversal_basename_inside_data_dir() {
    let temp_dir = tempfile::tempdir().expect("temp dir");
    let mut file_system = ManagedFileSystem::new(temp_dir.path()).expect("managed file system");
    let write_result = file_system
        .write_file(&WriteFileAction {
            file_name: "note.md".to_owned(),
            content: "safe managed content".to_owned(),
            append: false,
            trailing_newline: false,
            leading_newline: false,
        })
        .expect("write managed file");
    assert_eq!(write_result.error, None);

    let upload_path = file_system
        .upload_file_path("../note.md")
        .expect("managed traversal basename");
    let data_dir = std::fs::canonicalize(file_system.data_dir()).expect("canonical data dir");
    let upload_path = std::fs::canonicalize(upload_path).expect("canonical upload path");

    assert!(upload_path.starts_with(&data_dir));
    assert_eq!(upload_path, data_dir.join("note.md"));
}

#[test]
fn managed_upload_file_path_rejects_missing_traversal_basename() {
    let temp_dir = tempfile::tempdir().expect("temp dir");
    let file_system = ManagedFileSystem::new(temp_dir.path()).expect("managed file system");

    assert_eq!(file_system.upload_file_path("../note.md"), None);
}

#[tokio::test]
async fn browser_executor_uploads_managed_traversal_by_owned_basename() {
    let temp_dir = tempfile::tempdir().expect("temp dir");
    let session = MockSession::new();
    let file_system = ManagedFileSystem::new(temp_dir.path()).expect("managed file system");
    let mut executor = BrowserActionExecutor::with_file_system(session, file_system);
    executor.set_upload_file_availability(true, Vec::new());
    let write_result = executor
        .execute(&BrowserAction::WriteFile(WriteFileAction {
            file_name: "note.md".to_owned(),
            content: "upload me".to_owned(),
            append: false,
            trailing_newline: false,
            leading_newline: false,
        }))
        .await;
    assert_eq!(write_result.error, None);
    let managed_path = executor.file_system().data_dir().join("note.md");

    let result = executor
        .execute(&BrowserAction::UploadFile(UploadFileAction {
            index: 3,
            path: "../note.md".to_owned(),
        }))
        .await;

    assert_eq!(result.error, None);
    assert_eq!(
        executor.session().events(),
        vec![format!("upload_file:3:{}", managed_path.display())]
    );
}

#[tokio::test]
async fn browser_executor_rejects_missing_managed_upload_traversal_basename() {
    let temp_dir = tempfile::tempdir().expect("temp dir");
    let session = MockSession::new();
    let file_system = ManagedFileSystem::new(temp_dir.path()).expect("managed file system");
    let mut executor = BrowserActionExecutor::with_file_system(session, file_system);
    executor.set_upload_file_availability(true, Vec::new());

    let result = executor
        .execute(&BrowserAction::UploadFile(UploadFileAction {
            index: 3,
            path: "../note.md".to_owned(),
        }))
        .await;

    assert!(
        result
            .error
            .as_deref()
            .expect("upload error")
            .contains("AgentSettings.available_file_paths")
    );
    assert!(executor.session().events().is_empty());
}

#[tokio::test]
async fn browser_executor_saves_pdf_when_file_name_is_present() {
    let temp_dir = tempfile::tempdir().expect("temp dir");
    let requested_output_path = temp_dir.path().join("out");
    let output_path = temp_dir.path().join("out.pdf");
    let output = requested_output_path.display().to_string();
    let session = MockSession::new();
    let mut executor = BrowserActionExecutor::new(session);

    let result = executor
        .execute(&BrowserAction::SaveAsPdf(SaveAsPdfAction {
            file_name: Some(output.clone()),
            print_background: true,
            landscape: false,
            scale: 1.0,
            paper_format: "Letter".to_owned(),
        }))
        .await;

    assert_eq!(result.error, None);
    assert!(
        result
            .extracted_content
            .expect("pdf content")
            .contains(&output_path.display().to_string())
    );
    assert_eq!(result.attachments, vec![output_path.display().to_string()]);
    assert!(
        std::fs::read(&output_path)
            .expect("pdf file")
            .starts_with(b"%PDF")
    );
    assert_eq!(
        executor.session().events(),
        vec!["save_pdf:true:false:1:Letter"]
    );
}

#[test]
fn pdf_output_path_uses_sanitized_title_and_deduplicates() {
    let temp_dir = tempfile::tempdir().expect("temp dir");
    let existing = temp_dir.path().join("Quarterly Plan  2026.pdf");
    std::fs::write(&existing, b"existing").expect("existing pdf");

    let output = pdf_output_path(None, Some("Quarterly: Plan / 2026"));
    assert_eq!(output, std::path::PathBuf::from("Quarterly Plan  2026.pdf"));

    let duplicate = next_available_pdf_path(existing);
    assert_eq!(
        duplicate.file_name().and_then(std::ffi::OsStr::to_str),
        Some("Quarterly Plan  2026 (1).pdf")
    );
    assert_eq!(
        pdf_output_path(Some("/tmp/report"), None),
        std::path::PathBuf::from("/tmp/report.pdf")
    );
    assert_eq!(
        pdf_output_path(Some("/tmp/report.PDF"), None),
        std::path::PathBuf::from("/tmp/report.PDF")
    );
}

#[tokio::test]
async fn browser_executor_extracts_page_text() {
    let session = MockSession::new();
    let mut executor = BrowserActionExecutor::new(session);

    let result = executor
        .execute(&BrowserAction::Extract(ExtractAction {
            query: "company".to_owned(),
            extract_links: false,
            extract_images: false,
            start_from_char: 6,
            output_schema: None,
            already_collected: vec![],
        }))
        .await;

    let content = result.extracted_content.expect("extracted content");
    assert!(content.contains("<url>\nabout:blank\n</url>"));
    assert!(content.contains("<query>\ncompany\n</query>"));
    assert!(content.contains("<content_stats>"));
    assert!(content.contains("started from char 6"));
    assert!(content.contains("<webpage_content>"));
    assert!(content.contains("EvalOps Beta"));
    assert_eq!(executor.session().events(), vec!["page_text"]);
}

#[tokio::test]
async fn browser_executor_extracts_links_images_schema_and_dedupe_hints() {
    let session = MockSession::new();
    let mut executor = BrowserActionExecutor::new(session);

    let result = executor
        .execute(&BrowserAction::Extract(ExtractAction {
            query: "product image URLs".to_owned(),
            extract_links: true,
            extract_images: false,
            start_from_char: 0,
            output_schema: Some(serde_json::json!({
                "type": "object",
                "properties": {
                    "products": { "type": "array" }
                }
            })),
            already_collected: vec!["Existing product".to_owned()],
        }))
        .await;

    let content = result.extracted_content.expect("extracted content");
    assert!(content.contains("<output_schema>"));
    assert!(content.contains(r#""type": "object""#));
    assert!(content.contains("extract_links=true, extract_images=true"));
    assert!(content.contains("<links>\n- Run EvalOps: https://evalops.dev/run\n</links>"));
    assert!(content.contains("<images>\n- Hero shot: https://evalops.dev/hero.png\n</images>"));
    assert!(content.contains("<already_collected>"));
    assert!(content.contains("- Existing product"));
    let metadata = result.metadata.expect("extract metadata");
    assert_eq!(metadata["structured_extraction"], true);
    assert_eq!(metadata["source_url"], "about:blank");
    assert_eq!(metadata["schema_used"]["type"], "object");
    assert_eq!(metadata["is_partial"], false);
    assert_eq!(metadata["content_stats"]["method"], "page_text");
    assert_eq!(metadata["options"]["extract_links"], true);
    assert_eq!(metadata["options"]["extract_images"], true);
    assert_eq!(metadata["options"]["links_count"], 1);
    assert_eq!(metadata["options"]["images_count"], 1);
    assert_eq!(
        executor.session().events(),
        vec![
            "page_text",
            "find_elements:a[href]:href|title|aria-label|rel:200:true",
            "find_elements:img[src], img[data-src], picture source[srcset]:src|data-src|srcset|alt|title|aria-label:200:false"
        ]
    );
}

#[tokio::test]
async fn browser_executor_rejects_extract_start_beyond_content() {
    let session = MockSession::new();
    let mut executor = BrowserActionExecutor::new(session);

    let result = executor
        .execute(&BrowserAction::Extract(ExtractAction {
            query: "company".to_owned(),
            extract_links: false,
            extract_images: false,
            start_from_char: 999,
            output_schema: None,
            already_collected: vec![],
        }))
        .await;

    assert!(
        result
            .error
            .as_deref()
            .expect("extract error")
            .contains("exceeds content length")
    );
    assert_eq!(executor.session().events(), vec!["page_text"]);
}

#[tokio::test]
async fn browser_executor_searches_page_text() {
    let session = MockSession::new();
    let mut executor = BrowserActionExecutor::new(session);

    let result = executor
        .execute(&BrowserAction::SearchPage(SearchPageAction {
            pattern: "evalops".to_owned(),
            regex: false,
            case_sensitive: false,
            context_chars: 5,
            css_scope: None,
            max_results: 2,
        }))
        .await;

    let content = result.extracted_content.expect("search content");
    assert!(content.contains("Found 2 matches for \"evalops\" on page:"));
    assert!(content.contains("[1] ...lpha EvalOps Beta..."));
    assert!(content.contains("[2] ...cond EvalOps line"));
    assert!(content.contains("EvalOps"));
    assert_eq!(content.matches("EvalOps").count(), 2);
}

#[tokio::test]
async fn browser_executor_finds_css_elements() {
    let session = MockSession::new();
    let mut executor = BrowserActionExecutor::new(session);

    let result = executor
        .execute(&BrowserAction::FindElements(FindElementsAction {
            selector: "button".to_owned(),
            attributes: Some(vec!["id".to_owned()]),
            max_results: 3,
            include_text: true,
        }))
        .await;

    let content = result.extracted_content.expect("find content");
    assert!(content.contains("Found 1 element matching \"button\":"));
    assert!(content.contains("[0] <button> \"Run EvalOps\" {id=\"run\"}"));
    assert!(content.contains("Run EvalOps"));
    assert_eq!(
        executor.session().events(),
        vec!["find_elements:button:id:3:true"]
    );
}

struct QueueModel {
    model_name: String,
    outputs: Mutex<VecDeque<Result<Value, LlmError>>>,
    usages: Mutex<VecDeque<Option<ChatUsage>>>,
    requests: Mutex<Vec<ChatRequest>>,
}

impl QueueModel {
    fn new(outputs: Vec<Value>) -> Self {
        Self::with_results(outputs.into_iter().map(Ok).collect())
    }

    fn with_results(outputs: Vec<Result<Value, LlmError>>) -> Self {
        Self::with_model_and_results("static", outputs)
    }

    fn with_model(model_name: &str, outputs: Vec<Value>) -> Self {
        Self::with_model_and_results(model_name, outputs.into_iter().map(Ok).collect())
    }

    fn with_model_and_results(model_name: &str, outputs: Vec<Result<Value, LlmError>>) -> Self {
        let output_count = outputs.len();
        Self {
            model_name: model_name.to_owned(),
            outputs: Mutex::new(outputs.into()),
            usages: Mutex::new(vec![None; output_count].into()),
            requests: Mutex::new(Vec::new()),
        }
    }

    fn with_model_outputs_and_usages(
        model_name: &str,
        outputs: Vec<Value>,
        usages: Vec<Option<ChatUsage>>,
    ) -> Self {
        assert_eq!(outputs.len(), usages.len());
        Self {
            model_name: model_name.to_owned(),
            outputs: Mutex::new(outputs.into_iter().map(Ok).collect()),
            usages: Mutex::new(usages.into()),
            requests: Mutex::new(Vec::new()),
        }
    }
}

fn request_text(request: &ChatRequest) -> String {
    request
        .messages
        .iter()
        .flat_map(|message| message.content.iter())
        .filter_map(|part| match part {
            ContentPart::Text { text } => Some(text.as_str()),
            ContentPart::ImageUrl { .. } => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[async_trait]
impl ChatModel for QueueModel {
    fn provider(&self) -> &str {
        "test"
    }

    fn model(&self) -> &str {
        &self.model_name
    }

    async fn invoke_json(&self, request: ChatRequest) -> Result<ChatCompletion<Value>, LlmError> {
        assert!(
            request
                .messages
                .iter()
                .any(|message| message.role == MessageRole::User)
        );
        assert!(request.output_schema.is_some());
        self.requests.lock().expect("requests lock").push(request);
        let content = self
            .outputs
            .lock()
            .expect("outputs lock")
            .pop_front()
            .ok_or_else(|| LlmError::Provider("no queued model output".to_owned()))??;
        Ok(ChatCompletion {
            model: self.model().to_owned(),
            content,
            usage: self
                .usages
                .lock()
                .expect("usages lock")
                .pop_front()
                .flatten(),
            raw_response: None,
        })
    }
}

#[tokio::test]
async fn agent_history_records_token_usage_summary_and_costs() {
    let done_output = serde_json::json!({
        "current_state": {
            "thinking": "done"
        },
        "action": [
            {
                "done": {
                    "text": "finished",
                    "success": true
                }
            }
        ]
    });
    let usage = ChatUsage {
        prompt_tokens: 100,
        prompt_cached_tokens: Some(40),
        prompt_cache_creation_tokens: None,
        prompt_image_tokens: None,
        completion_tokens: 20,
        total_tokens: 120,
    };
    let settings = AgentSettings {
        calculate_cost: true,
        use_judge: false,
        ..AgentSettings::default()
    };
    let mut agent = Agent::with_settings(
        "finish",
        settings,
        QueueModel::with_model_outputs_and_usages("bu-1-0", vec![done_output], vec![Some(usage)]),
        MockSession::new(),
    );

    let history = agent.run(1).await.expect("run");
    let usage = history.usage.as_ref().expect("usage summary");
    let stats = usage.by_model.get("bu-1-0").expect("model stats");

    assert_eq!(usage.entry_count, 1);
    assert_eq!(usage.total_prompt_tokens, 100);
    assert_eq!(usage.total_prompt_cached_tokens, 40);
    assert_eq!(usage.total_completion_tokens, 20);
    assert_eq!(usage.total_tokens, 120);
    assert_eq!(stats.invocations, 1);
    assert_eq!(stats.average_tokens_per_invocation, 120.0);
    assert!((usage.total_prompt_cost - 0.0000128).abs() < 0.0000000001);
    assert!((usage.total_prompt_cached_cost - 0.0000008).abs() < 0.0000000001);
    assert!((usage.total_completion_cost - 0.00004).abs() < 0.0000000001);
    assert!((usage.total_cost - 0.0000536).abs() < 0.0000000001);
}

#[tokio::test]
async fn agent_extract_action_uses_llm_result_sections() {
    let agent_output = serde_json::json!({
        "current_state": {
            "thinking": "extract"
        },
        "action": [
            {
                "extract": {
                    "query": "company summary",
                    "extract_links": false,
                    "extract_images": false,
                    "start_from_char": 0,
                    "already_collected": []
                }
            }
        ]
    });
    let extract_output = serde_json::json!({
        "result": "EvalOps appears in the page content."
    });
    let mut agent = Agent::new(
        "extract company summary",
        QueueModel::new(vec![agent_output, extract_output]),
        MockSession::new(),
    );

    let result = {
        let item = agent.step().await.expect("agent step");
        item.result[0].clone()
    };

    let content = result.extracted_content.as_deref().expect("extract result");
    assert!(content.contains("<url>\nabout:blank\n</url>"));
    assert!(content.contains("<query>\ncompany summary\n</query>"));
    assert!(content.contains("<result>\nEvalOps appears in the page content.\n</result>"));
    assert_eq!(result.long_term_memory.as_deref(), Some(content));
    assert!(!result.include_extracted_content_only_once);

    let requests = agent.llm.requests.lock().expect("requests lock").clone();
    assert_eq!(requests.len(), 2);
    let extract_request_text = request_text(&requests[1]);
    assert!(extract_request_text.contains("<webpage_content>"));
    assert!(extract_request_text.contains("Alpha EvalOps Beta"));
    assert!(
        requests[1]
            .output_schema
            .as_ref()
            .is_some_and(|schema| schema["properties"]["result"]["type"] == "string")
    );
}

#[tokio::test]
async fn agent_extract_action_uses_configured_page_extraction_llm() {
    let agent_output = serde_json::json!({
        "current_state": {},
        "action": [
            {
                "extract": {
                    "query": "company summary",
                    "extract_links": false,
                    "extract_images": false,
                    "start_from_char": 0,
                    "already_collected": []
                }
            }
        ]
    });
    let extract_output = serde_json::json!({
        "result": "Extracted by the dedicated model."
    });
    let primary = QueueModel::with_model("primary", vec![agent_output]);
    let extraction = QueueModel::with_model("extractor", vec![extract_output]);
    let mut agent = Agent::new("extract with dedicated model", primary, MockSession::new())
        .with_page_extraction_llm(extraction);

    let result = {
        let item = agent.step().await.expect("agent step");
        item.result[0].clone()
    };

    let content = result.extracted_content.as_deref().expect("extract result");
    assert!(content.contains("Extracted by the dedicated model."));
    assert_eq!(
        agent.llm.requests.lock().expect("primary requests").len(),
        1
    );
    let extraction_llm = agent
        .page_extraction_llm
        .as_ref()
        .expect("page extraction llm");
    let extraction_requests = extraction_llm.requests.lock().expect("extract requests");
    assert_eq!(extraction_requests.len(), 1);
    assert!(request_text(&extraction_requests[0]).contains("<webpage_content>"));
}

#[tokio::test]
async fn agent_switches_to_fallback_llm_on_retryable_model_error() {
    let done_output = serde_json::json!({
        "current_state": {
            "next_goal": "done"
        },
        "action": [
            {
                "done": {
                    "text": "fallback finished",
                    "success": true
                }
            }
        ]
    });
    let primary = QueueModel::with_model_and_results(
        "primary",
        vec![Err(LlmError::RateLimited("primary limited".to_owned()))],
    );
    let fallback = QueueModel::with_model("fallback", vec![done_output]);
    let mut agent =
        Agent::new("finish with fallback", primary, MockSession::new()).with_fallback_llm(fallback);

    let result = {
        let item = agent.step().await.expect("agent step");
        item.result[0].clone()
    };

    assert_eq!(
        result.extracted_content.as_deref(),
        Some("fallback finished")
    );
    assert!(agent.is_using_fallback_llm());
    assert_eq!(agent.llm.model(), "fallback");
    assert_eq!(agent.llm.requests.lock().expect("requests lock").len(), 1);
}

#[tokio::test]
async fn agent_does_not_switch_to_fallback_llm_for_invalid_structured_output() {
    let primary = QueueModel::with_model_and_results(
        "primary",
        vec![Err(LlmError::InvalidStructuredOutput(
            "bad json".to_owned(),
        ))],
    );
    let fallback = QueueModel::with_model(
        "fallback",
        vec![serde_json::json!({
            "current_state": {},
            "action": []
        })],
    );
    let mut agent =
        Agent::new("do not fallback", primary, MockSession::new()).with_fallback_llm(fallback);

    let error = agent.step().await.expect_err("invalid output should fail");

    assert!(matches!(
        error,
        AgentRunError::Llm(LlmError::InvalidStructuredOutput(message)) if message == "bad json"
    ));
    assert!(!agent.is_using_fallback_llm());
    assert!(agent.fallback_llm.is_some());
}

#[tokio::test]
async fn agent_does_not_switch_fallback_llm_twice_after_fallback_failure() {
    let primary = QueueModel::with_model_and_results(
        "primary",
        vec![Err(LlmError::RateLimited("primary limited".to_owned()))],
    );
    let fallback = QueueModel::with_model_and_results(
        "fallback",
        vec![Err(LlmError::Provider("fallback failed".to_owned()))],
    );
    let mut agent =
        Agent::new("fallback fails", primary, MockSession::new()).with_fallback_llm(fallback);

    let error = agent.step().await.expect_err("fallback should fail");

    assert!(matches!(
        error,
        AgentRunError::Llm(LlmError::Provider(message)) if message == "fallback failed"
    ));
    assert!(agent.is_using_fallback_llm());
    assert!(agent.fallback_llm.is_none());
    assert_eq!(agent.llm.model(), "fallback");
}

#[test]
fn agent_with_settings_uses_configured_file_system_path() {
    let temp_dir = tempfile::tempdir().expect("temp dir");
    let custom_base_dir = temp_dir.path().join("custom-agent-files");
    let settings = AgentSettings {
        file_system_path: Some(custom_base_dir.display().to_string()),
        ..AgentSettings::default()
    };

    let agent = Agent::with_settings(
        "use custom files",
        settings,
        QueueModel::new(Vec::new()),
        MockSession::new(),
    );

    let state = agent.file_system_state();
    assert_eq!(std::path::PathBuf::from(state.base_dir), custom_base_dir);
    assert!(
        custom_base_dir
            .join(DEFAULT_FILE_SYSTEM_PATH)
            .join("todo.md")
            .exists()
    );
}

#[test]
fn agent_with_settings_defaults_file_system_to_temp_dir() {
    let agent = Agent::new(
        "use temp files",
        QueueModel::new(Vec::new()),
        MockSession::new(),
    );
    let state = agent.file_system_state();
    let base_dir = std::path::PathBuf::from(state.base_dir);

    assert!(base_dir.starts_with(std::env::temp_dir()));
    assert!(
        base_dir
            .join(DEFAULT_FILE_SYSTEM_PATH)
            .join("todo.md")
            .exists()
    );
}

#[tokio::test]
async fn agent_shortens_prompt_urls_and_restores_model_output_before_execution() {
    let long_url = "https://example.test/path?abcdefghijklmnopqrstuvwxyz0123456789";
    let shortened = "https://example.test/path?abcdefghi...0cd4b05";
    let mut state = blank_state();
    state.url = long_url.to_owned();
    let agent_output = serde_json::json!({
        "current_state": {
            "memory": format!("Use {shortened} from the page state"),
            "next_goal": format!("Open {shortened}")
        },
        "action": [
            {
                "navigate": {
                    "url": shortened,
                    "new_tab": false
                }
            }
        ]
    });
    let settings = AgentSettings {
        directly_open_url: false,
        url_shortening_limit: Some(10),
        ..AgentSettings::default()
    };
    let mut agent = Agent::with_settings(
        "continue",
        settings,
        QueueModel::new(vec![agent_output]),
        MockSession::with_states(vec![state]),
    );

    let model_output = {
        let item = agent.step().await.expect("agent step");
        item.model_output.clone().expect("model output")
    };

    let requests = agent.llm.requests.lock().expect("requests lock").clone();
    let prompt_text = request_text(&requests[0]);
    assert!(prompt_text.contains(shortened));
    assert!(!prompt_text.contains(long_url));
    assert_eq!(
        agent.executor.session().events(),
        vec![format!("navigate:{long_url}:false")]
    );
    let expected_memory = format!("Use {long_url} from the page state");
    let expected_next_goal = format!("Open {long_url}");
    assert_eq!(
        model_output.current_state.memory.as_deref(),
        Some(expected_memory.as_str())
    );
    assert_eq!(
        model_output.current_state.next_goal.as_deref(),
        Some(expected_next_goal.as_str())
    );
    assert!(matches!(
        &model_output.action[0],
        BrowserAction::Navigate(NavigateAction { url, new_tab: false }) if url == long_url
    ));
}

#[tokio::test]
async fn agent_extract_action_passes_structured_schema_to_llm() {
    let schema = serde_json::json!({
        "type": "object",
        "properties": {
            "company": { "type": "string" },
            "links": {
                "type": "array",
                "items": { "type": "string" }
            }
        },
        "required": ["company"]
    });
    let agent_output = serde_json::json!({
        "current_state": {},
        "action": [
            {
                "extract": {
                    "query": "company and links",
                    "extract_links": true,
                    "extract_images": false,
                    "start_from_char": 0,
                    "output_schema": schema,
                    "already_collected": ["Old EvalOps"]
                }
            }
        ]
    });
    let extract_output = serde_json::json!({
        "company": "EvalOps",
        "links": ["https://evalops.dev/run"]
    });
    let settings = AgentSettings {
        extraction_schema: Some(serde_json::json!({
            "type": "object",
            "properties": {
                "ignored": { "type": "string" }
            }
        })),
        ..AgentSettings::default()
    };
    let mut agent = Agent::with_settings(
        "extract structured company data",
        settings,
        QueueModel::new(vec![agent_output, extract_output]),
        MockSession::new(),
    );

    let result = {
        let item = agent.step().await.expect("agent step");
        item.result[0].clone()
    };

    let content = result.extracted_content.as_deref().expect("extract result");
    assert!(content.contains("<structured_result>"));
    assert!(content.contains(r#""company":"EvalOps""#));
    assert!(content.contains("https://evalops.dev/run"));

    let metadata = result.metadata.as_ref().expect("structured metadata");
    assert_eq!(metadata["structured_extraction"], true);
    assert_eq!(metadata["extraction_result"]["data"]["company"], "EvalOps");
    assert_eq!(
        metadata["extraction_result"]["schema_used"]["required"][0],
        "company"
    );
    assert_eq!(metadata["extraction_result"]["source_url"], "about:blank");

    let requests = agent.llm.requests.lock().expect("requests lock").clone();
    assert_eq!(requests.len(), 2);
    assert_eq!(
        requests[1].output_schema.as_ref().expect("extract schema"),
        &schema
    );
    let extract_request_text = request_text(&requests[1]);
    assert!(extract_request_text.contains("<links>"));
    assert!(extract_request_text.contains("<already_collected>"));
}

#[tokio::test]
async fn agent_extract_action_uses_agent_extraction_schema_when_action_omits_schema() {
    let schema = serde_json::json!({
        "type": "object",
        "properties": {
            "company": { "type": "string" }
        },
        "required": ["company"]
    });
    let agent_output = serde_json::json!({
        "current_state": {},
        "action": [
            {
                "extract": {
                    "query": "company",
                    "extract_links": false,
                    "extract_images": false,
                    "start_from_char": 0,
                    "already_collected": []
                }
            }
        ]
    });
    let extract_output = serde_json::json!({
        "company": "EvalOps"
    });
    let settings = AgentSettings {
        extraction_schema: Some(schema.clone()),
        ..AgentSettings::default()
    };
    let mut agent = Agent::with_settings(
        "extract company with default schema",
        settings,
        QueueModel::new(vec![agent_output, extract_output]),
        MockSession::new(),
    );

    let (result, model_output) = {
        let item = agent.step().await.expect("agent step");
        (
            item.result[0].clone(),
            item.model_output.clone().expect("model output"),
        )
    };

    let content = result.extracted_content.as_deref().expect("extract result");
    assert!(content.contains("<structured_result>"));
    assert!(content.contains(r#""company":"EvalOps""#));
    let metadata = result.metadata.as_ref().expect("structured metadata");
    assert_eq!(
        metadata["extraction_result"]["schema_used"]["required"][0],
        "company"
    );
    let BrowserAction::Extract(history_params) = &model_output.action[0] else {
        panic!("expected extract action");
    };
    assert_eq!(history_params.output_schema, None);

    let requests = agent.llm.requests.lock().expect("requests lock").clone();
    assert_eq!(
        requests[1].output_schema.as_ref().expect("extract schema"),
        &schema
    );
    let extract_request_text = request_text(&requests[1]);
    assert!(extract_request_text.contains("<output_schema>"));
    assert!(extract_request_text.contains("\"company\""));
}

#[tokio::test]
async fn agent_extract_action_records_llm_failure_as_action_error() {
    let agent_output = serde_json::json!({
        "current_state": {},
        "action": [
            {
                "extract": {
                    "query": "company summary",
                    "extract_links": false,
                    "extract_images": false,
                    "start_from_char": 0,
                    "already_collected": []
                }
            }
        ]
    });
    let mut agent = Agent::new(
        "extract with failing model",
        QueueModel::with_results(vec![
            Ok(agent_output),
            Err(LlmError::Provider("extract unavailable".to_owned())),
        ]),
        MockSession::new(),
    );

    let result = {
        let item = agent.step().await.expect("agent step");
        item.result[0].clone()
    };

    assert!(result.extracted_content.is_none());
    assert!(
        result
            .error
            .as_deref()
            .is_some_and(|error| error.contains("LLM-backed extract failed"))
    );
}

#[tokio::test]
async fn agent_step_records_done_history() {
    let output = serde_json::json!({
        "current_state": {
            "thinking": "done"
        },
        "action": [
            {
                "done": {
                    "text": "finished",
                    "success": true,
                    "files_to_display": []
                }
            }
        ]
    });
    let mut agent = Agent::new(
        "finish the task",
        QueueModel::new(vec![output]),
        MockSession::new(),
    );

    let item = agent.step().await.expect("agent step");

    assert_eq!(item.result, vec![ActionResult::done("finished", true)]);
    let metadata = item.metadata.as_ref().expect("step metadata");
    assert_eq!(metadata.step_number, 1);
    assert_eq!(metadata.step_interval, None);
    assert!(metadata.step_end_time >= metadata.step_start_time);
    assert!(metadata.duration_seconds() >= 0.0);
    assert_eq!(agent.history().final_result(), Some("finished"));
}

#[tokio::test]
async fn agent_file_system_state_survives_steps_and_prompts() {
    let write_output = serde_json::json!({
        "current_state": {
            "thinking": "write report"
        },
        "action": [
            {
                "write_file": {
                    "file_name": "report.md",
                    "content": "alpha",
                    "append": false,
                    "trailing_newline": false,
                    "leading_newline": false
                }
            }
        ]
    });
    let read_output = serde_json::json!({
        "current_state": {
            "thinking": "read report"
        },
        "action": [
            {
                "read_file": {
                    "file_name": "report.md"
                }
            }
        ]
    });
    let temp_dir = tempfile::tempdir().expect("temp dir");
    let file_system = ManagedFileSystem::new(temp_dir.path()).expect("managed file system");
    let mut agent = Agent::with_settings_and_file_system(
        "write then read a report",
        AgentSettings::default(),
        QueueModel::new(vec![write_output, read_output]),
        MockSession::new(),
        file_system,
    );

    agent.step().await.expect("write step");
    assert_eq!(
        agent.file_system().display_file("report.md").as_deref(),
        Some("alpha")
    );

    let item = agent.step().await.expect("read step");
    assert!(
        item.result
            .first()
            .and_then(|result| result.extracted_content.as_deref())
            .expect("read result")
            .contains("alpha")
    );

    let requests = agent.llm.requests.lock().expect("requests lock");
    let second_prompt = request_text(&requests[1]);
    assert!(second_prompt.contains("<file_system>\n<file>\nreport.md - 1 lines"));
    assert!(second_prompt.contains("<content>\nalpha\n</content>"));

    let state = agent.file_system_state();
    assert!(state.files.contains_key("report.md"));
    let restored = ManagedFileSystem::from_state(state).expect("restore file system");
    assert_eq!(restored.display_file("report.md").as_deref(), Some("alpha"));
}

#[tokio::test]
async fn agent_rejects_unavailable_upload_paths_before_browser_side_effects() {
    let upload_output = serde_json::json!({
        "current_state": {
            "thinking": "upload"
        },
        "action": [
            {
                "upload_file": {
                    "index": 3,
                    "path": "/tmp/not-declared.txt"
                }
            }
        ]
    });
    let mut agent = Agent::new(
        "upload a file",
        QueueModel::new(vec![upload_output]),
        MockSession::new(),
    );

    let item = agent.step().await.expect("agent step");

    assert!(
        item.result
            .first()
            .and_then(|result| result.error.as_deref())
            .expect("upload error")
            .contains("AgentSettings.available_file_paths")
    );
    assert!(agent.executor.session().events().is_empty());
}

#[tokio::test]
async fn restored_agent_continues_with_serialized_file_system_state() {
    let temp_dir = tempfile::tempdir().expect("temp dir");
    let mut file_system = ManagedFileSystem::new(temp_dir.path()).expect("managed file system");
    file_system
        .write_file(&WriteFileAction {
            file_name: "todo.md".to_owned(),
            content: "- read the restored report".to_owned(),
            append: false,
            trailing_newline: false,
            leading_newline: false,
        })
        .expect("write todo");
    file_system
        .write_file(&WriteFileAction {
            file_name: "report.md".to_owned(),
            content: "alpha\nbeta".to_owned(),
            append: false,
            trailing_newline: false,
            leading_newline: false,
        })
        .expect("write report");

    let page_text = "EvalOps restored extracted content. ".repeat(700);
    let extract_result = extract_action_result(
        &ExtractAction {
            query: "restored context".to_owned(),
            extract_links: false,
            extract_images: false,
            start_from_char: 0,
            output_schema: None,
            already_collected: Vec::new(),
        },
        &page_text,
        Some("https://example.test/restored"),
        false,
        None,
        None,
        Some(&mut file_system),
    );
    assert!(
        extract_result
            .long_term_memory
            .as_deref()
            .expect("extract memory")
            .contains("Content in extracted_content_0.md and once in <read_state>.")
    );

    let state = file_system.get_state();
    assert_eq!(state.extracted_content_count, 1);
    let restored = ManagedFileSystem::from_state(state).expect("restore file system");
    let read_output = serde_json::json!({
        "current_state": {
            "thinking": "read restored report"
        },
        "action": [
            {
                "read_file": {
                    "file_name": "report.md"
                }
            }
        ]
    });
    let done_output = serde_json::json!({
        "current_state": {
            "thinking": "done"
        },
        "action": [
            {
                "done": {
                    "text": "restored report read",
                    "success": true,
                    "files_to_display": []
                }
            }
        ]
    });
    let mut agent = Agent::with_settings_and_file_system(
        "continue after restore",
        AgentSettings {
            use_judge: false,
            ..AgentSettings::default()
        },
        QueueModel::new(vec![read_output, done_output]),
        MockSession::new(),
        restored,
    );

    let history = agent.run(2).await.expect("restored agent run");
    assert!(history.is_done());
    assert_eq!(history.final_result(), Some("restored report read"));
    assert!(
        history.items[0].result[0]
            .extracted_content
            .as_deref()
            .expect("read result")
            .contains("<content>\nalpha\nbeta\n</content>")
    );

    let requests = agent.llm.requests.lock().expect("requests lock");
    assert_eq!(requests.len(), 2);
    let first_prompt = request_text(&requests[0]);
    assert!(first_prompt.contains("<file_system>\n<file>\nextracted_content_0.md -"));
    assert!(first_prompt.contains("<file>\nreport.md - 2 lines"));
    assert!(first_prompt.contains("<content>\nalpha\nbeta\n</content>"));
    assert!(first_prompt.contains("<todo_contents>\n- read the restored report\n</todo_contents>"));
    let second_prompt = request_text(&requests[1]);
    assert!(second_prompt.contains("<read_state_0>"));
    assert!(second_prompt.contains("Read from file report.md."));
    drop(requests);

    let next_file = agent
        .file_system_mut()
        .save_extracted_content("second restored extract")
        .expect("next extracted content");
    assert_eq!(next_file, "extracted_content_1.md");
    assert_eq!(agent.file_system_state().extracted_content_count, 2);
    assert_eq!(
        agent
            .file_system()
            .display_file("extracted_content_1.md")
            .as_deref(),
        Some("second restored extract")
    );
}

#[tokio::test]
async fn agent_checkpoint_round_trips_and_resumes_state() {
    let write_output = serde_json::json!({
        "current_state": {
            "thinking": "write checkpoint files"
        },
        "action": [
            {
                "write_file": {
                    "file_name": "todo.md",
                    "content": "- resume and read report",
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
    });
    let settings = AgentSettings {
        initial_actions: vec![BrowserAction::Wait(WaitAction { seconds: 0 })],
        use_judge: false,
        ..AgentSettings::default()
    };
    let temp_dir = tempfile::tempdir().expect("temp dir");
    let file_system = ManagedFileSystem::new(temp_dir.path()).expect("managed file system");
    let mut agent = Agent::with_settings_and_file_system(
        "checkpoint the agent",
        settings,
        QueueModel::new(vec![write_output]),
        MockSession::new(),
        file_system,
    );

    agent
        .execute_initial_actions()
        .await
        .expect("initial checkpoint action");
    agent.step().await.expect("write checkpoint files");
    assert_eq!(agent.history().items.len(), 2);
    assert_eq!(
        agent
            .history()
            .items
            .first()
            .and_then(|item| item.metadata.as_ref())
            .map(|metadata| metadata.step_number),
        Some(0)
    );
    assert_eq!(
        agent
            .file_system_mut()
            .save_extracted_content("checkpoint extract")
            .expect("seed extract"),
        "extracted_content_0.md"
    );

    let checkpoint = agent.checkpoint();
    assert!(checkpoint.initial_actions_executed);
    assert_eq!(checkpoint.task, "checkpoint the agent");
    assert_eq!(checkpoint.history.items.len(), 2);
    assert_eq!(checkpoint.file_system_state.extracted_content_count, 1);
    let checkpoint_json = serde_json::to_string_pretty(&checkpoint).expect("serialize checkpoint");
    let checkpoint: AgentCheckpoint =
        serde_json::from_str(&checkpoint_json).expect("deserialize checkpoint");
    assert!(checkpoint.initial_actions_executed);

    let read_output = serde_json::json!({
        "current_state": {
            "thinking": "read restored report"
        },
        "action": [
            {
                "read_file": {
                    "file_name": "report.md"
                }
            }
        ]
    });
    let done_output = serde_json::json!({
        "current_state": {
            "thinking": "done"
        },
        "action": [
            {
                "done": {
                    "text": "checkpoint resumed",
                    "success": true,
                    "files_to_display": []
                }
            }
        ]
    });
    let mut resumed = Agent::from_checkpoint(
        checkpoint,
        QueueModel::new(vec![read_output, done_output]),
        MockSession::new(),
    )
    .expect("resume from checkpoint");

    let history = resumed.run(2).await.expect("resumed run");
    assert!(history.is_done());
    assert_eq!(history.items.len(), 4);
    assert_eq!(history.final_result(), Some("checkpoint resumed"));
    assert!(
        history.items[2].result[0]
            .extracted_content
            .as_deref()
            .expect("read result")
            .contains("<content>\nalpha\nbeta\n</content>")
    );

    let requests = resumed.llm.requests.lock().expect("requests lock");
    assert_eq!(requests.len(), 2);
    let first_prompt = request_text(&requests[0]);
    assert!(first_prompt.contains("Wrote file report.md"));
    assert!(first_prompt.contains("<file_system>\n<file>\nextracted_content_0.md -"));
    assert!(first_prompt.contains("<todo_contents>\n- resume and read report\n</todo_contents>"));
    let second_prompt = request_text(&requests[1]);
    assert!(second_prompt.contains("<read_state_0>"));
    assert!(second_prompt.contains("Read from file report.md."));
    drop(requests);

    let next_file = resumed
        .file_system_mut()
        .save_extracted_content("after checkpoint")
        .expect("next extracted content");
    assert_eq!(next_file, "extracted_content_1.md");
    assert_eq!(resumed.file_system_state().extracted_content_count, 2);
}

#[tokio::test]
async fn agent_replaces_sensitive_input_tags_for_execution_without_history_leak() {
    let output = serde_json::json!({
        "current_state": {},
        "action": [
            {
                "input": {
                    "index": 1,
                    "text": "<secret>password</secret>",
                    "clear": true
                }
            }
        ]
    });
    let settings = AgentSettings {
        sensitive_data: BTreeMap::from([(
            "password".to_owned(),
            SensitiveDataValue::Value("correct horse battery staple".to_owned()),
        )]),
        ..AgentSettings::default()
    };
    let mut agent = Agent::with_settings(
        "enter password",
        settings,
        QueueModel::new(vec![output]),
        MockSession::new(),
    );

    let recorded_action_text = {
        let item = agent.step().await.expect("agent step");
        let model_output = item.model_output.as_ref().expect("model output");
        let BrowserAction::Input(params) = &model_output.action[0] else {
            panic!("expected input action");
        };
        params.text.clone()
    };

    assert_eq!(
        agent.executor.session().events(),
        vec!["input:1:correct horse battery staple:true"]
    );
    assert_eq!(recorded_action_text, "<secret>password</secret>");

    let request = agent.llm.requests.lock().expect("requests lock");
    let prompt_text = request_text(&request[0]);
    assert!(prompt_text.contains("<sensitive_data>SENSITIVE DATA"));
    assert!(prompt_text.contains("<secret>password</secret>"));
    assert!(!prompt_text.contains("correct horse battery staple"));
}

#[test]
fn actions_for_execution_replaces_sensitive_tags_and_literals_across_params() {
    let settings = AgentSettings {
        sensitive_data: BTreeMap::from([
            (
                "api_key".to_owned(),
                SensitiveDataValue::Value("sk-live-123".to_owned()),
            ),
            (
                "username".to_owned(),
                SensitiveDataValue::Value("evalops-user".to_owned()),
            ),
        ]),
        ..AgentSettings::default()
    };
    let actions = vec![
        BrowserAction::WriteFile(WriteFileAction {
            file_name: "request.txt".to_owned(),
            content: "Authorization: Bearer <secret>api_key</secret>".to_owned(),
            append: false,
            trailing_newline: true,
            leading_newline: false,
        }),
        BrowserAction::Input(InputTextAction {
            index: 1,
            text: "username".to_owned(),
            clear: true,
        }),
    ];

    let replaced = actions_for_execution(&actions, &settings, "https://example.test");

    assert_eq!(
        replaced[0],
        BrowserAction::WriteFile(WriteFileAction {
            file_name: "request.txt".to_owned(),
            content: "Authorization: Bearer sk-live-123".to_owned(),
            append: false,
            trailing_newline: true,
            leading_newline: false,
        })
    );
    assert_eq!(
        replaced[1],
        BrowserAction::Input(InputTextAction {
            index: 1,
            text: "evalops-user".to_owned(),
            clear: true,
        })
    );
}

#[test]
fn actions_for_execution_injects_default_extraction_schema_without_mutating_history_action() {
    let default_schema = serde_json::json!({
        "type": "object",
        "properties": {
            "company": { "type": "string" }
        }
    });
    let action_schema = serde_json::json!({
        "type": "object",
        "properties": {
            "title": { "type": "string" }
        }
    });
    let settings = AgentSettings {
        extraction_schema: Some(default_schema.clone()),
        ..AgentSettings::default()
    };
    let actions = vec![
        BrowserAction::Extract(ExtractAction {
            query: "company".to_owned(),
            extract_links: false,
            extract_images: false,
            start_from_char: 0,
            output_schema: None,
            already_collected: vec![],
        }),
        BrowserAction::Extract(ExtractAction {
            query: "title".to_owned(),
            extract_links: false,
            extract_images: false,
            start_from_char: 0,
            output_schema: Some(action_schema.clone()),
            already_collected: vec![],
        }),
    ];

    let execution_actions = actions_for_execution(&actions, &settings, "https://example.test");

    let BrowserAction::Extract(defaulted_params) = &execution_actions[0] else {
        panic!("expected extract action");
    };
    assert_eq!(
        defaulted_params.output_schema.as_ref(),
        Some(&default_schema)
    );
    let BrowserAction::Extract(action_params) = &execution_actions[1] else {
        panic!("expected extract action");
    };
    assert_eq!(action_params.output_schema.as_ref(), Some(&action_schema));
    let BrowserAction::Extract(original_params) = &actions[0] else {
        panic!("expected extract action");
    };
    assert_eq!(original_params.output_schema, None);
}

#[test]
fn sensitive_bu_2fa_code_placeholders_generate_totp_values() {
    let settings = AgentSettings {
        sensitive_data: BTreeMap::from([(
            "login_bu_2fa_code".to_owned(),
            SensitiveDataValue::Value("GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ".to_owned()),
        )]),
        ..AgentSettings::default()
    };
    let actions = vec![BrowserAction::Input(InputTextAction {
        index: 1,
        text: "<secret>login_bu_2fa_code</secret>".to_owned(),
        clear: true,
    })];

    assert_eq!(
        totp_code_at("GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ", 59, 30, 8),
        Some("94287082".to_owned())
    );

    let replaced = actions_for_execution(&actions, &settings, "https://example.test");
    let BrowserAction::Input(params) = &replaced[0] else {
        panic!("expected input action");
    };

    assert_eq!(params.text.len(), 6);
    assert!(
        params
            .text
            .chars()
            .all(|character| character.is_ascii_digit())
    );
    assert_ne!(params.text, "GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ");
}

struct SlowModel;

struct QueueThenPendingModel {
    first_output: Mutex<Option<Value>>,
}

impl QueueThenPendingModel {
    fn new(first_output: Value) -> Self {
        Self {
            first_output: Mutex::new(Some(first_output)),
        }
    }
}

#[async_trait]
impl ChatModel for SlowModel {
    fn provider(&self) -> &str {
        "test"
    }

    fn model(&self) -> &str {
        "slow"
    }

    async fn invoke_json(&self, _request: ChatRequest) -> Result<ChatCompletion<Value>, LlmError> {
        std::future::pending::<()>().await;
        unreachable!("pending model should be cancelled by timeout")
    }
}

#[async_trait]
impl ChatModel for QueueThenPendingModel {
    fn provider(&self) -> &str {
        "test"
    }

    fn model(&self) -> &str {
        "queue-then-pending"
    }

    async fn invoke_json(&self, _request: ChatRequest) -> Result<ChatCompletion<Value>, LlmError> {
        let first_output = self.first_output.lock().expect("first output lock").take();
        if let Some(content) = first_output {
            return Ok(ChatCompletion {
                model: self.model().to_owned(),
                content,
                usage: None,
                raw_response: None,
            });
        }
        std::future::pending::<()>().await;
        unreachable!("pending model should be cancelled by timeout")
    }
}

#[tokio::test]
async fn agent_step_enforces_llm_timeout() {
    let settings = AgentSettings {
        llm_timeout_seconds: 0,
        ..AgentSettings::default()
    };
    let mut agent = Agent::with_settings("timeout", settings, SlowModel, MockSession::new());

    let error = agent.step().await.expect_err("LLM timeout");

    assert!(matches!(error, AgentRunError::LlmTimedOut { seconds: 0 }));
}

#[tokio::test]
async fn agent_step_enforces_step_timeout() {
    let settings = AgentSettings {
        step_timeout_seconds: 0,
        ..AgentSettings::default()
    };
    let mut agent = Agent::with_settings("step timeout", settings, SlowModel, MockSession::new());

    let error = agent.step().await.expect_err("step timeout");

    assert!(matches!(error, AgentRunError::StepTimedOut { seconds: 0 }));
    assert!(agent.history().items.is_empty());
}

#[tokio::test]
async fn agent_action_timeout_bounds_llm_backed_extract_resolution() {
    let output = serde_json::json!({
        "current_state": {},
        "action": [
            {
                "extract": {
                    "query": "company summary",
                    "extract_links": false,
                    "extract_images": false,
                    "include_source": false
                }
            }
        ]
    });
    let settings = AgentSettings {
        action_timeout_seconds: 0.005,
        llm_timeout_seconds: 30,
        ..AgentSettings::default()
    };
    let mut agent = Agent::with_settings(
        "extract with bounded action timeout",
        settings,
        QueueThenPendingModel::new(output),
        MockSession::new(),
    );

    let item = agent.step().await.expect("step records action timeout");

    let error = item.result[0].error.as_deref().expect("timeout error");
    assert!(error.contains("Action extract timed out after"));
    assert!(error.contains("dead CDP WebSocket"));
}

#[tokio::test]
async fn agent_step_respects_use_vision_setting() {
    let output = serde_json::json!({
        "current_state": {},
        "action": [
            {
                "done": {
                    "text": "complete",
                    "success": true,
                    "files_to_display": []
                }
            }
        ]
    });
    let settings = AgentSettings {
        use_vision: VisionMode::Never,
        ..AgentSettings::default()
    };
    let mut agent = Agent::with_settings(
        "finish without screenshot",
        settings,
        QueueModel::new(vec![output]),
        MockSession::new(),
    );

    agent.step().await.expect("agent step");

    let state_requests = agent.executor.session().state_screenshot_requests();
    assert_eq!(state_requests.first(), Some(&false));
    assert!(state_requests.iter().all(|include| !include));
}

#[tokio::test]
async fn agent_step_saves_conversation_transcript() {
    let temp_dir = tempfile::tempdir().expect("temp dir");
    let transcript_dir = temp_dir.path().join("conversations");
    let output = serde_json::json!({
        "current_state": {},
        "action": [
            {
                "done": {
                    "text": "complete",
                    "success": true,
                    "files_to_display": []
                }
            }
        ]
    });
    let mut state = blank_state();
    state.screenshot = Some("abc123".to_owned());
    let settings = AgentSettings {
        save_conversation_path: Some(transcript_dir.display().to_string()),
        ..AgentSettings::default()
    };
    let mut agent = Agent::with_settings(
        "save transcript",
        settings,
        QueueModel::new(vec![output]),
        MockSession::with_states(vec![state]),
    );
    let agent_id = agent.id();

    agent.step().await.expect("agent step");

    let transcript_path = transcript_dir.join(format!("conversation_{agent_id}_1.txt"));
    let transcript = std::fs::read_to_string(&transcript_path).expect("transcript file");
    assert!(transcript.contains(" system "));
    assert!(transcript.contains(" user "));
    assert!(transcript.contains("save transcript"));
    assert!(transcript.contains("<image_url detail=\"auto\">"));
    assert!(transcript.contains("data:image/png;base64,abc123"));
    assert!(transcript.contains("\"done\""));
    assert!(transcript.contains("\"text\": \"complete\""));
}

#[test]
fn conversation_snapshot_encoding_honors_labels() {
    assert_eq!(
        encode_conversation_snapshot("hello", Some("utf8")).expect("utf8 bytes"),
        b"hello"
    );
    assert_eq!(
        encode_conversation_snapshot("£", Some("windows-1252")).expect("windows bytes"),
        vec![0xa3]
    );
}

#[test]
fn conversation_snapshot_encoding_rejects_invalid_or_lossy_labels() {
    let invalid = encode_conversation_snapshot("hello", Some("not-real")).expect_err("invalid");
    assert!(matches!(
        invalid,
        AgentRunError::ConversationEncoding { encoding } if encoding == "not-real"
    ));

    let lossy = encode_conversation_snapshot("😀", Some("windows-1252")).expect_err("lossy");
    assert!(matches!(
        lossy,
        AgentRunError::ConversationEncodingLossy { encoding } if encoding == "windows-1252"
    ));
}

#[tokio::test]
async fn agent_run_honors_screenshot_action_next_observation() {
    let screenshot_output = serde_json::json!({
        "current_state": {},
        "action": [
            {
                "screenshot": {}
            }
        ]
    });
    let done_output = serde_json::json!({
        "current_state": {},
        "action": [
            {
                "done": {
                    "text": "saw screenshot",
                    "success": true,
                    "files_to_display": []
                }
            }
        ]
    });
    let settings = AgentSettings {
        use_vision: VisionMode::Auto,
        ..AgentSettings::default()
    };
    let mut agent = Agent::with_settings(
        "request screenshot then finish",
        settings,
        QueueModel::new(vec![screenshot_output, done_output]),
        MockSession::new(),
    );

    agent.run(2).await.expect("agent run");

    let state_requests = agent.executor.session().state_screenshot_requests();
    assert_eq!(state_requests.first(), Some(&false));
    assert_eq!(state_requests.last(), Some(&true));
    assert!(
        state_requests.iter().any(|include| *include),
        "expected screenshot request override: {state_requests:?}"
    );
}

#[tokio::test]
async fn agent_step_defaults_to_screenshot_observation() {
    let output = serde_json::json!({
        "current_state": {},
        "action": [
            {
                "done": {
                    "text": "complete",
                    "success": true,
                    "files_to_display": []
                }
            }
        ]
    });
    let mut agent = Agent::with_settings(
        "finish with default vision",
        AgentSettings::default(),
        QueueModel::new(vec![output]),
        MockSession::new(),
    );

    agent.step().await.expect("agent step");

    let state_requests = agent.executor.session().state_screenshot_requests();
    assert_eq!(state_requests.first(), Some(&true));
}

#[tokio::test]
async fn agent_step_rejects_screenshot_action_outside_auto_vision() {
    let output = serde_json::json!({
        "current_state": {},
        "action": [
            {
                "screenshot": {}
            }
        ]
    });
    let mut agent = Agent::with_settings(
        "do not allow screenshot tool by default",
        AgentSettings::default(),
        QueueModel::new(vec![output]),
        MockSession::new(),
    );

    let error = {
        let item = agent.step().await.expect("agent step");
        assert_eq!(item.result.len(), 1);
        item.result[0]
            .error
            .as_deref()
            .expect("screenshot gating error")
            .to_owned()
    };

    assert!(agent.executor.session().events().is_empty());
    assert_eq!(
        agent.executor.session().state_screenshot_requests(),
        vec![true]
    );
    assert!(
        error.contains("use_vision") && error.contains("auto"),
        "unexpected error: {error}"
    );
}

#[tokio::test]
async fn agent_step_never_requests_screenshot_when_vision_disabled() {
    let output = serde_json::json!({
        "current_state": {},
        "action": [
            {
                "screenshot": {}
            }
        ]
    });
    let settings = AgentSettings {
        use_vision: VisionMode::Never,
        ..AgentSettings::default()
    };
    let mut agent = Agent::with_settings(
        "vision disabled blocks screenshot tool",
        settings,
        QueueModel::new(vec![output]),
        MockSession::new(),
    );

    let item = agent.step().await.expect("agent step");

    assert!(
        item.result[0]
            .error
            .as_deref()
            .is_some_and(|error| { error.contains("use_vision") && error.contains("auto") })
    );
    assert!(agent.executor.session().events().is_empty());
    assert_eq!(
        agent.executor.session().state_screenshot_requests(),
        vec![false]
    );
}

#[tokio::test]
async fn agent_step_truncates_too_many_actions_like_upstream() {
    let output = serde_json::json!({
        "current_state": {},
        "action": [
            {
                "click": {
                    "index": 1
                }
            },
            {
                "click": {
                    "index": 2
                }
            }
        ]
    });
    let settings = AgentSettings {
        max_actions_per_step: 1,
        ..AgentSettings::default()
    };
    let mut agent = Agent::with_settings(
        "click only once",
        settings,
        QueueModel::new(vec![output]),
        MockSession::new(),
    );

    let item = agent.step().await.expect("agent step");

    assert_eq!(item.result.len(), 1);
    assert_eq!(item.result[0].error, None);
    assert_eq!(
        item.model_output
            .as_ref()
            .expect("model output")
            .action
            .len(),
        1
    );
    assert_eq!(agent.executor.session().events(), vec!["click:1"]);
}

#[tokio::test]
async fn agent_step_scales_llm_screenshot_coordinates_to_viewport() {
    let output = serde_json::json!({
        "current_state": {},
        "action": [
            {
                "click": {
                    "coordinate_x": 700,
                    "coordinate_y": 425
                }
            }
        ]
    });
    let mut state = blank_state();
    state.page_info = Some(browser_use_dom::PageInfo {
        viewport_width: 2800,
        viewport_height: 1700,
        page_width: 2800,
        page_height: 1700,
        scroll_x: 0,
        scroll_y: 0,
        pixels_above: 0,
        pixels_below: 0,
        pixels_left: 0,
        pixels_right: 0,
    });
    let settings = AgentSettings {
        llm_screenshot_size: Some(LlmScreenshotSize::new(1400, 850).expect("valid size")),
        ..AgentSettings::default()
    };
    let mut agent = Agent::with_settings(
        "click coordinate from resized screenshot",
        settings,
        QueueModel::new(vec![output]),
        MockSession::with_states(vec![state]),
    );

    let (coordinate_x, coordinate_y) = {
        let item = agent.step().await.expect("agent step");
        assert_eq!(item.result[0].error, None);
        let BrowserAction::Click(params) =
            &item.model_output.as_ref().expect("model output").action[0]
        else {
            panic!("expected click action");
        };
        (params.coordinate_x, params.coordinate_y)
    };

    assert_eq!(
        agent.executor.session().events(),
        vec!["click_coordinates:1400:850"]
    );
    assert_eq!(coordinate_x, Some(700));
    assert_eq!(coordinate_y, Some(425));
}

#[tokio::test]
async fn agent_step_rejects_excluded_actions_before_side_effects() {
    let output = serde_json::json!({
        "current_state": {},
        "action": [
            {
                "click": {
                    "index": 1
                }
            }
        ]
    });
    let settings = AgentSettings {
        excluded_actions: vec!["click".to_owned()],
        ..AgentSettings::default()
    };
    let mut agent = Agent::with_settings(
        "do not click",
        settings,
        QueueModel::new(vec![output]),
        MockSession::new(),
    );

    let error = {
        let item = agent.step().await.expect("agent step");
        assert_eq!(item.result.len(), 1);
        item.result[0]
            .error
            .as_deref()
            .expect("excluded error")
            .to_owned()
    };

    assert!(agent.executor.session().events().is_empty());
    assert!(
        error.contains("excluded action `click`"),
        "unexpected error: {error}"
    );
}

#[tokio::test]
async fn agent_step_allows_non_excluded_actions_to_execute() {
    let output = serde_json::json!({
        "current_state": {},
        "action": [
            {
                "click": {
                    "index": 1
                }
            }
        ]
    });
    let settings = AgentSettings {
        excluded_actions: vec!["search".to_owned(), "switch-tab".to_owned()],
        ..AgentSettings::default()
    };
    let mut agent = Agent::with_settings(
        "click still allowed",
        settings,
        QueueModel::new(vec![output]),
        MockSession::new(),
    );

    {
        let item = agent.step().await.expect("agent step");
        assert_eq!(item.result.len(), 1);
        assert_eq!(item.result[0].error, None);
    }

    assert_eq!(agent.executor.session().events(), vec!["click:1"]);
}

#[tokio::test]
async fn excluded_actions_never_block_done_outputs() {
    let output = serde_json::json!({
        "current_state": {},
        "action": [
            {
                "done": {
                    "text": "complete",
                    "success": true,
                    "files_to_display": []
                }
            }
        ]
    });
    let settings = AgentSettings {
        excluded_actions: vec!["done".to_owned()],
        ..AgentSettings::default()
    };
    let mut agent = Agent::with_settings(
        "finish",
        settings,
        QueueModel::new(vec![output]),
        MockSession::new(),
    );

    let history = agent.run(1).await.expect("agent run");

    assert_eq!(history.final_result(), Some("complete"));
    assert_eq!(history.is_successful(), Some(true));
}

#[tokio::test]
async fn agent_run_executes_initial_actions_as_step_zero() {
    let done_output = serde_json::json!({
        "current_state": {},
        "action": [
            {
                "done": {
                    "text": "ready",
                    "success": true,
                    "files_to_display": []
                }
            }
        ]
    });
    let settings = AgentSettings {
        initial_actions: vec![BrowserAction::Navigate(NavigateAction {
            url: "https://example.test/start".to_owned(),
            new_tab: false,
        })],
        ..AgentSettings::default()
    };
    let mut agent = Agent::with_settings(
        "continue after preload",
        settings,
        QueueModel::new(vec![done_output]),
        MockSession::new(),
    );

    let history = agent.run(1).await.expect("agent run").clone();

    assert_eq!(
        agent.executor.session().events(),
        vec!["navigate:https://example.test/start:false"]
    );
    assert_eq!(history.items.len(), 2);
    assert_eq!(
        history.items[0]
            .metadata
            .as_ref()
            .expect("initial metadata")
            .step_number,
        0
    );
    assert_eq!(history.items[0].state.url, "https://example.test/start");
    assert_eq!(history.items[0].state.title, "Initial Actions");
    assert_eq!(
        history.items[0]
            .model_output
            .as_ref()
            .expect("initial output")
            .next_goal
            .as_deref(),
        Some("Initial navigation")
    );
    assert_eq!(
        history.items[1]
            .metadata
            .as_ref()
            .expect("step metadata")
            .step_number,
        1
    );

    let request = agent.llm.requests.lock().expect("requests lock");
    let prompt_text = request_text(&request[0]);
    assert!(prompt_text.contains("Initial navigation"));
    assert!(prompt_text.contains("Navigated to https://example.test/start"));
}

#[test]
fn extract_start_url_from_task_matches_upstream_filters() {
    assert_eq!(
        extract_start_url_from_task("Open example.com and summarize it").as_deref(),
        Some("https://example.com")
    );
    assert_eq!(
        extract_start_url_from_task("Open https://example.com/path?q=1.").as_deref(),
        Some("https://example.com/path?q=1")
    );
    assert_eq!(
        extract_start_url_from_task("Email support@example.com for the status"),
        None
    );
    assert_eq!(
        extract_start_url_from_task("Read https://example.com/report.pdf"),
        None
    );
    assert_eq!(
        extract_start_url_from_task("Do not open example.com during this task"),
        None
    );
    assert_eq!(
        extract_start_url_from_task("Compare example.com and example.org"),
        None
    );
}

#[tokio::test]
async fn agent_run_auto_navigates_single_task_url_as_initial_action() {
    let done_output = serde_json::json!({
        "current_state": {},
        "action": [
            {
                "done": {
                    "text": "ready",
                    "success": true,
                    "files_to_display": []
                }
            }
        ]
    });
    let mut agent = Agent::with_settings(
        "Summarize example.test/start before answering",
        AgentSettings {
            use_judge: false,
            ..AgentSettings::default()
        },
        QueueModel::new(vec![done_output]),
        MockSession::new(),
    );

    let history = agent.run(1).await.expect("agent run").clone();

    assert_eq!(
        agent.settings.initial_actions,
        vec![BrowserAction::Navigate(NavigateAction {
            url: "https://example.test/start".to_owned(),
            new_tab: false,
        })]
    );
    assert_eq!(
        agent.executor.session().events(),
        vec!["navigate:https://example.test/start:false"]
    );
    assert_eq!(history.items[0].state.url, "https://example.test/start");
    assert_eq!(history.items[0].state.title, "Initial Actions");
}

#[tokio::test]
async fn agent_run_respects_directly_open_url_opt_out_and_explicit_initial_actions() {
    let done_output = serde_json::json!({
        "current_state": {},
        "action": [
            {
                "done": {
                    "text": "ready",
                    "success": true,
                    "files_to_display": []
                }
            }
        ]
    });
    let mut disabled_agent = Agent::with_settings(
        "Summarize example.test/start before answering",
        AgentSettings {
            directly_open_url: false,
            use_judge: false,
            ..AgentSettings::default()
        },
        QueueModel::new(vec![done_output.clone()]),
        MockSession::new(),
    );
    disabled_agent
        .run(1)
        .await
        .expect("disabled directly_open_url run");
    assert!(disabled_agent.settings.initial_actions.is_empty());
    assert!(disabled_agent.executor.session().events().is_empty());

    let explicit_action = BrowserAction::Navigate(NavigateAction {
        url: "https://explicit.example/start".to_owned(),
        new_tab: false,
    });
    let mut explicit_agent = Agent::with_settings(
        "Summarize example.test/start before answering",
        AgentSettings {
            initial_actions: vec![explicit_action.clone()],
            use_judge: false,
            ..AgentSettings::default()
        },
        QueueModel::new(vec![done_output]),
        MockSession::new(),
    );
    explicit_agent
        .run(1)
        .await
        .expect("explicit initial action run");
    assert_eq!(
        explicit_agent.settings.initial_actions,
        vec![explicit_action]
    );
    assert_eq!(
        explicit_agent.executor.session().events(),
        vec!["navigate:https://explicit.example/start:false"]
    );
}

#[test]
fn agent_auto_configures_claude_sonnet_llm_screenshot_size() {
    let agent = Agent::new(
        "use Claude screenshot defaults",
        QueueModel::with_model("claude-sonnet-4-20250514", vec![]),
        MockSession::new(),
    );

    assert_eq!(
        agent.settings.llm_screenshot_size,
        Some(LlmScreenshotSize::new(1400, 850).expect("valid Claude default"))
    );
}

#[tokio::test]
async fn agent_run_invokes_step_and_done_callbacks() {
    let output = serde_json::json!({
        "current_state": {},
        "action": [
            {
                "done": {
                    "text": "complete",
                    "success": true,
                    "files_to_display": []
                }
            }
        ]
    });
    let events = Arc::new(Mutex::new(Vec::new()));
    let mut agent = Agent::with_settings(
        "complete",
        AgentSettings {
            use_judge: false,
            ..AgentSettings::default()
        },
        QueueModel::new(vec![output]),
        MockSession::new(),
    );

    let step_events = Arc::clone(&events);
    agent.register_new_step_callback(move |_state, output, step_number| {
        let action_name = output
            .action
            .first()
            .map(BrowserAction::name)
            .unwrap_or("none");
        step_events
            .lock()
            .expect("step events lock")
            .push(format!("step:{step_number}:{action_name}"));
        Ok::<(), String>(())
    });

    let done_events = Arc::clone(&events);
    agent.register_done_callback(move |history| {
        done_events
            .lock()
            .expect("done events lock")
            .push(format!("done:{}", history.items.len()));
        Ok::<(), String>(())
    });

    let history = agent.run(1).await.expect("agent run");

    assert_eq!(history.final_result(), Some("complete"));
    assert_eq!(
        events.lock().expect("events lock").as_slice(),
        ["step:1:done", "done:1"]
    );
}

#[tokio::test]
async fn agent_run_invokes_async_step_callback() {
    let output = serde_json::json!({
        "current_state": {},
        "action": [
            {
                "done": {
                    "text": "async complete",
                    "success": true,
                    "files_to_display": []
                }
            }
        ]
    });
    let events = Arc::new(Mutex::new(Vec::new()));
    let mut agent = Agent::with_settings(
        "complete async",
        AgentSettings {
            use_judge: false,
            ..AgentSettings::default()
        },
        QueueModel::new(vec![output]),
        MockSession::new(),
    );

    let step_events = Arc::clone(&events);
    agent.register_new_step_callback_async(move |_state, output, step_number| {
        let step_events = Arc::clone(&step_events);
        let action_name = output
            .action
            .first()
            .map(BrowserAction::name)
            .unwrap_or("none")
            .to_owned();
        Box::pin(async move {
            step_events
                .lock()
                .expect("async step events lock")
                .push(format!("async-step:{step_number}:{action_name}"));
            Ok(())
        })
    });

    let history = agent.run(1).await.expect("agent run");

    assert_eq!(history.final_result(), Some("async complete"));
    assert_eq!(
        events.lock().expect("events lock").as_slice(),
        ["async-step:1:done"]
    );
}

#[tokio::test]
async fn agent_done_callback_sees_judged_history() {
    let done_output = serde_json::json!({
        "current_state": {},
        "action": [
            {
                "done": {
                    "text": "judge before callback",
                    "success": true,
                    "files_to_display": []
                }
            }
        ]
    });
    let judge_output = serde_json::json!({
        "reasoning": "Checked the terminal result before invoking callbacks.",
        "verdict": true,
        "failure_reason": null,
        "impossible_task": false,
        "reached_captcha": false
    });
    let seen = Arc::new(Mutex::new(Vec::new()));
    let callback_seen = Arc::clone(&seen);
    let mut agent = Agent::new(
        "finish with judged history",
        QueueModel::new(vec![done_output, judge_output]),
        MockSession::new(),
    );
    agent.register_done_callback(move |history| {
        let verdict = history
            .items
            .last()
            .and_then(|item| item.result.last())
            .and_then(|result| result.judgement.as_ref())
            .map(|judgement| judgement.verdict);
        callback_seen
            .lock()
            .expect("done callback log")
            .push(format!("items:{}:verdict:{verdict:?}", history.items.len()));
        Ok::<(), String>(())
    });

    agent.run(1).await.expect("agent run");

    assert_eq!(
        seen.lock().expect("seen log").as_slice(),
        ["items:1:verdict:Some(true)"]
    );
}

#[tokio::test]
async fn agent_run_stops_before_model_when_stop_requested() {
    let mut agent = Agent::new("complete", QueueModel::new(vec![]), MockSession::new());

    agent.stop();
    let error = agent.run(1).await.expect_err("run should stop");

    assert!(matches!(
        error,
        AgentRunError::Stopped { reason } if reason == "stop requested"
    ));
    assert!(agent.is_stopped());
    assert!(agent.llm.requests.lock().expect("requests lock").is_empty());
    assert!(agent.executor.session().events().is_empty());
}

#[tokio::test]
async fn agent_step_pauses_before_model_or_browser_work() {
    let mut agent = Agent::new("pause", QueueModel::new(vec![]), MockSession::new());

    agent.pause();
    let error = agent.step().await.expect_err("step should pause");

    assert!(matches!(error, AgentRunError::Paused));
    assert!(agent.is_paused());
    assert!(agent.llm.requests.lock().expect("requests lock").is_empty());
    assert!(agent.executor.session().events().is_empty());
}

#[tokio::test]
async fn agent_checkpoint_preserves_paused_state() {
    let mut agent = Agent::new(
        "pause checkpoint",
        QueueModel::new(vec![]),
        MockSession::new(),
    );
    agent.pause();

    let checkpoint = agent.checkpoint();
    assert!(checkpoint.paused);
    assert!(!checkpoint.stopped);
    let checkpoint_json = serde_json::to_string_pretty(&checkpoint).expect("serialize checkpoint");
    assert!(checkpoint_json.contains(r#""paused": true"#));
    let checkpoint: AgentCheckpoint =
        serde_json::from_str(&checkpoint_json).expect("deserialize checkpoint");
    let mut resumed =
        Agent::from_checkpoint(checkpoint, QueueModel::new(vec![]), MockSession::new())
            .expect("resume paused checkpoint");

    assert!(resumed.is_paused());
    let error = resumed
        .run(1)
        .await
        .expect_err("paused checkpoint should pause");
    assert!(matches!(error, AgentRunError::Paused));
}

#[tokio::test]
async fn agent_resume_continues_paused_run_without_clearing_history() {
    let done_output = serde_json::json!({
        "current_state": {},
        "action": [
            {
                "done": {
                    "text": "resumed",
                    "success": true,
                    "files_to_display": []
                }
            }
        ]
    });
    let settings = AgentSettings {
        use_judge: false,
        ..AgentSettings::default()
    };
    let mut agent = Agent::with_settings(
        "resume",
        settings,
        QueueModel::new(vec![done_output]),
        MockSession::new(),
    );

    agent.pause();
    let error = agent.run(1).await.expect_err("run should pause");
    assert!(matches!(error, AgentRunError::Paused));
    assert!(agent.history().items.is_empty());

    agent.resume();
    let (final_result, item_count) = {
        let history = agent.run(1).await.expect("resumed run");
        (
            history.final_result().map(str::to_owned),
            history.items.len(),
        )
    };

    assert!(!agent.is_paused());
    assert_eq!(final_result.as_deref(), Some("resumed"));
    assert_eq!(item_count, 1);
}

#[tokio::test]
async fn agent_stop_remains_stronger_than_resume_after_pause() {
    let mut agent = Agent::new("stop wins", QueueModel::new(vec![]), MockSession::new());

    agent.pause();
    agent.stop();
    agent.resume();
    let error = agent
        .run(1)
        .await
        .expect_err("stopped agent should stay stopped");

    assert!(matches!(
        error,
        AgentRunError::Stopped { reason } if reason == "stop requested"
    ));
    assert!(agent.is_stopped());
    assert!(!agent.is_paused());
    assert!(agent.llm.requests.lock().expect("requests lock").is_empty());
}

#[tokio::test]
async fn agent_add_new_task_clears_controls_and_preserves_state() {
    let first_output = serde_json::json!({
        "current_state": {},
        "action": [
            {
                "wait": {
                    "seconds": 0
                }
            }
        ]
    });
    let follow_up_output = serde_json::json!({
        "current_state": {},
        "action": [
            {
                "done": {
                    "text": "follow-up complete",
                    "success": true,
                    "files_to_display": []
                }
            }
        ]
    });
    let settings = AgentSettings {
        initial_actions: vec![BrowserAction::Wait(WaitAction { seconds: 0 })],
        use_judge: false,
        ..AgentSettings::default()
    };
    let mut agent = Agent::with_settings(
        "open the dashboard",
        settings,
        QueueModel::new(vec![first_output, follow_up_output]),
        MockSession::new(),
    );

    agent
        .execute_initial_actions()
        .await
        .expect("initial actions");
    agent.step().await.expect("first step");
    agent.pause();
    agent.stop();

    agent.add_new_task("summarize the new panel");

    assert!(!agent.is_paused());
    assert!(!agent.is_stopped());
    assert!(agent.initial_actions_executed);
    assert_eq!(agent.history().items.len(), 2);

    let result = {
        let item = agent.step().await.expect("follow-up step");
        item.result[0].clone()
    };

    assert_eq!(
        result.extracted_content.as_deref(),
        Some("follow-up complete")
    );
    assert_eq!(agent.history().items.len(), 3);
    let requests = agent.llm.requests.lock().expect("requests lock");
    let follow_up_prompt = request_text(&requests[1]);
    assert!(
        follow_up_prompt
            .contains("<initial_user_request>open the dashboard</initial_user_request>")
    );
    assert!(
        follow_up_prompt
            .contains("<follow_up_user_request> summarize the new panel </follow_up_user_request>")
    );
}

#[test]
fn agent_add_new_task_appends_without_double_wrapping_initial_request() {
    let mut agent = Agent::new(
        "collect receipts",
        QueueModel::new(vec![]),
        MockSession::new(),
    );

    agent.add_new_task("find invoice totals");
    agent.add_new_task("compare vendors");

    assert_eq!(agent.task.matches("<initial_user_request>").count(), 1);
    assert_eq!(agent.task.matches("<follow_up_user_request>").count(), 2);
    assert!(
        agent
            .task
            .contains("<initial_user_request>collect receipts</initial_user_request>")
    );
    assert!(
        agent
            .task
            .contains("<follow_up_user_request> find invoice totals </follow_up_user_request>")
    );
    assert!(
        agent
            .task
            .contains("<follow_up_user_request> compare vendors </follow_up_user_request>")
    );
}

#[test]
fn agent_uses_supplied_task_id_and_keeps_it_for_follow_up_tasks() {
    let task_id = Uuid::parse_str("018f82d0-1234-7abc-9234-56789abcdef0").expect("valid task id");
    let mut agent = Agent::new(
        "continuous task",
        QueueModel::new(vec![]),
        MockSession::new(),
    )
    .with_task_id(task_id);

    assert_eq!(agent.id(), task_id);
    assert_eq!(agent.task_id(), task_id);

    agent.add_new_task("continue under the same identity");

    assert_eq!(agent.id(), task_id);
    assert_eq!(agent.task_id(), task_id);
}

#[test]
fn agent_checkpoint_preserves_task_id_on_restore() {
    let task_id = Uuid::parse_str("018f82d0-4321-7abc-9234-56789abcdef0").expect("valid task id");
    let agent = Agent::new(
        "checkpoint identity",
        QueueModel::new(vec![]),
        MockSession::new(),
    )
    .with_task_id(task_id);

    let checkpoint = agent.checkpoint();
    assert_eq!(checkpoint.id, task_id);
    let checkpoint_json = serde_json::to_string_pretty(&checkpoint).expect("serialize checkpoint");
    assert!(checkpoint_json.contains(&format!(r#""id": "{task_id}""#)));
    let checkpoint: AgentCheckpoint =
        serde_json::from_str(&checkpoint_json).expect("deserialize checkpoint");
    let restored = Agent::from_checkpoint(checkpoint, QueueModel::new(vec![]), MockSession::new())
        .expect("restore checkpoint");

    assert_eq!(restored.id(), task_id);
    assert_eq!(restored.task_id(), task_id);
}

#[test]
fn agent_checkpoint_without_task_id_deserializes_with_generated_identity() {
    let agent = Agent::new(
        "legacy checkpoint",
        QueueModel::new(vec![]),
        MockSession::new(),
    );
    let mut checkpoint_value =
        serde_json::to_value(agent.checkpoint()).expect("serialize checkpoint value");
    checkpoint_value
        .as_object_mut()
        .expect("checkpoint object")
        .remove("id")
        .expect("checkpoint id present");

    let checkpoint: AgentCheckpoint =
        serde_json::from_value(checkpoint_value).expect("legacy checkpoint");
    assert_ne!(checkpoint.id, Uuid::nil());
    let generated_id = checkpoint.id;
    let restored = Agent::from_checkpoint(checkpoint, QueueModel::new(vec![]), MockSession::new())
        .expect("restore legacy checkpoint");

    assert_eq!(restored.id(), generated_id);
    assert_eq!(restored.task_id(), generated_id);
}

#[tokio::test]
async fn agent_run_stops_before_model_when_should_stop_callback_requests_it() {
    let calls = Arc::new(Mutex::new(0usize));
    let mut agent = Agent::new("complete", QueueModel::new(vec![]), MockSession::new());
    let callback_calls = Arc::clone(&calls);
    agent.register_should_stop_callback(move || {
        let mut calls = callback_calls.lock().expect("callback calls lock");
        *calls += 1;
        Ok::<_, String>(*calls >= 1)
    });

    let error = agent.run(1).await.expect_err("run should stop");

    assert!(matches!(
        error,
        AgentRunError::Stopped { reason } if reason == "should_stop callback requested stop"
    ));
    assert!(agent.is_stopped());
    assert_eq!(*calls.lock().expect("calls lock"), 1);
    assert!(agent.llm.requests.lock().expect("requests lock").is_empty());
}

#[tokio::test]
async fn agent_run_interrupts_for_external_status_without_stopping() {
    let calls = Arc::new(Mutex::new(0usize));
    let mut agent = Agent::new("complete", QueueModel::new(vec![]), MockSession::new());
    let callback_calls = Arc::clone(&calls);
    agent.register_external_agent_status_callback(move || {
        let mut calls = callback_calls.lock().expect("external status calls lock");
        *calls += 1;
        Ok::<_, String>(true)
    });

    let error = agent.run(1).await.expect_err("run should interrupt");

    assert!(matches!(error, AgentRunError::ExternalStatusInterrupted));
    assert!(!agent.is_stopped());
    assert!(!agent.is_paused());
    assert_eq!(*calls.lock().expect("calls lock"), 1);
    assert!(agent.llm.requests.lock().expect("requests lock").is_empty());
    assert!(agent.executor.session().events().is_empty());
}

#[tokio::test]
async fn agent_run_reports_external_status_callback_errors_without_stop_state() {
    let mut agent = Agent::new("complete", QueueModel::new(vec![]), MockSession::new());
    agent.register_external_agent_status_raise_error_callback_async(|| {
        Box::pin(async { Err::<bool, String>("status probe failed".to_owned()) })
    });

    let error = agent.run(1).await.expect_err("callback should fail");

    assert!(matches!(
        error,
        AgentRunError::Callback {
            callback: "external_agent_status",
            message
        } if message == "status probe failed"
    ));
    assert!(!agent.is_stopped());
    assert!(!agent.is_paused());
    assert!(agent.llm.requests.lock().expect("requests lock").is_empty());
    assert!(agent.executor.session().events().is_empty());
}

#[tokio::test]
async fn agent_run_reports_callback_failures_without_side_effects() {
    let output = serde_json::json!({
        "current_state": {},
        "action": [
            {
                "click": {
                    "index": 1
                }
            }
        ]
    });
    let mut agent = Agent::with_settings(
        "click after callback",
        AgentSettings {
            use_judge: false,
            ..AgentSettings::default()
        },
        QueueModel::new(vec![output]),
        MockSession::new(),
    );
    agent.register_new_step_callback(|_state, _output, _step_number| {
        Err::<(), _>("callback refused step")
    });

    let error = agent.run(2).await.expect_err("callback should fail");

    assert!(matches!(
        error,
        AgentRunError::Callback {
            callback: "new_step",
            message
        } if message == "callback refused step"
    ));
    assert!(agent.history.items.is_empty());
    assert!(agent.executor.session().events().is_empty());
}

#[tokio::test]
async fn agent_run_stops_on_done() {
    let output = serde_json::json!({
        "current_state": {},
        "action": [
            {
                "done": {
                    "text": "complete",
                    "success": true,
                    "files_to_display": []
                }
            }
        ]
    });
    let mut agent = Agent::new(
        "complete",
        QueueModel::new(vec![output]),
        MockSession::new(),
    );

    let history = agent.run(3).await.expect("agent run");

    assert_eq!(history.items.len(), 1);
    assert_eq!(history.final_result(), Some("complete"));
}

#[tokio::test]
async fn agent_run_attaches_judge_result_to_done() {
    let done_output = serde_json::json!({
        "current_state": {},
        "action": [
            {
                "done": {
                    "text": "complete",
                    "success": true,
                    "files_to_display": []
                }
            }
        ]
    });
    let judge_output = serde_json::json!({
        "reasoning": "The task asked for stricter evidence.",
        "verdict": false,
        "failure_reason": "Missing required citation.",
        "impossible_task": false,
        "reached_captcha": false
    });
    let mut state = blank_state();
    state.screenshot = Some("judge-shot".to_owned());
    let settings = AgentSettings {
        ground_truth: Some("Must include a source citation.".to_owned()),
        ..AgentSettings::default()
    };
    let mut agent = Agent::with_settings(
        "complete with proof",
        settings,
        QueueModel::new(vec![done_output, judge_output]),
        MockSession::with_states(vec![state]),
    );

    let history = agent.run(1).await.expect("agent run").clone();

    assert_eq!(history.final_result(), Some("complete"));
    assert_eq!(history.is_successful(), Some(true));
    assert_eq!(history.is_validated(), Some(false));
    let judgement = history.judgement().expect("judgement");
    assert_eq!(
        judgement.failure_reason.as_deref(),
        Some("Missing required citation.")
    );

    let requests = agent.llm.requests.lock().expect("requests lock");
    assert_eq!(requests.len(), 2);
    let judge_text = request_text(&requests[1]);
    assert!(judge_text.contains("<ground_truth>"));
    assert!(judge_text.contains("Must include a source citation."));
    assert!(judge_text.contains("<final_result>\ncomplete\n</final_result>"));
    assert!(judge_text.contains("complete with proof"));
    assert!(
        requests[1].messages.iter().any(|message| {
            message.content.iter().any(|part| {
                matches!(
                    part,
                    ContentPart::ImageUrl { image_url, .. }
                        if image_url == "data:image/png;base64,judge-shot"
                )
            })
        }),
        "judge request should include recent screenshots"
    );
}

#[tokio::test]
async fn agent_run_uses_configured_judge_llm() {
    let done_output = serde_json::json!({
        "current_state": {},
        "action": [
            {
                "done": {
                    "text": "complete",
                    "success": true,
                    "files_to_display": []
                }
            }
        ]
    });
    let judge_output = serde_json::json!({
        "reasoning": "Dedicated judge reviewed the final answer.",
        "verdict": true,
        "failure_reason": null,
        "impossible_task": false,
        "reached_captcha": false
    });
    let primary = QueueModel::with_model("primary", vec![done_output]);
    let judge = QueueModel::with_model("judge", vec![judge_output]);
    let mut agent =
        Agent::new("complete with judge", primary, MockSession::new()).with_judge_llm(judge);

    let history = agent.run(1).await.expect("agent run").clone();

    assert_eq!(history.final_result(), Some("complete"));
    assert_eq!(history.is_validated(), Some(true));
    assert_eq!(
        history
            .judgement()
            .and_then(|judgement| judgement.reasoning.as_deref()),
        Some("Dedicated judge reviewed the final answer.")
    );
    assert_eq!(
        agent.llm.requests.lock().expect("primary requests").len(),
        1
    );
    let judge_llm = agent.judge_llm.as_ref().expect("judge llm");
    let judge_requests = judge_llm.requests.lock().expect("judge requests");
    assert_eq!(judge_requests.len(), 1);
    assert!(request_text(&judge_requests[0]).contains("<final_result>\ncomplete\n</final_result>"));
}

#[tokio::test]
async fn agent_run_skips_judge_when_disabled() {
    let output = serde_json::json!({
        "current_state": {},
        "action": [
            {
                "done": {
                    "text": "complete",
                    "success": true,
                    "files_to_display": []
                }
            }
        ]
    });
    let settings = AgentSettings {
        use_judge: false,
        ..AgentSettings::default()
    };
    let mut agent = Agent::with_settings(
        "skip judge",
        settings,
        QueueModel::new(vec![output]),
        MockSession::new(),
    );

    let history = agent.run(1).await.expect("agent run").clone();

    assert_eq!(history.final_result(), Some("complete"));
    assert_eq!(history.judgement(), None);
    let requests = agent.llm.requests.lock().expect("requests lock");
    assert_eq!(requests.len(), 1);
}

#[tokio::test]
async fn agent_run_compacts_history_between_steps() {
    let first_output = serde_json::json!({
        "current_state": {
            "memory": "Opened the checkout and started collecting receipt data",
            "next_goal": "Confirm the receipt details"
        },
        "action": [
            {
                "wait": {
                    "seconds": 0
                }
            }
        ]
    });
    let compaction_output = serde_json::json!({
        "summary": "Task: collect receipt data. IN-PROGRESS: checkout was opened but receipt details still need confirmation."
    });
    let done_output = serde_json::json!({
        "current_state": {
            "thinking": "done"
        },
        "action": [
            {
                "done": {
                    "text": "receipt confirmed",
                    "success": true,
                    "files_to_display": []
                }
            }
        ]
    });
    let settings = AgentSettings {
        use_judge: false,
        message_compaction: MessageCompaction::Settings(MessageCompactionSettings {
            compact_every_n_steps: 1,
            trigger_char_count: Some(1),
            keep_last_items: 1,
            summary_max_chars: 200,
            ..MessageCompactionSettings::default()
        }),
        ..AgentSettings::default()
    };
    let mut agent = Agent::with_settings(
        "collect receipt data",
        settings,
        QueueModel::new(vec![first_output, compaction_output, done_output]),
        MockSession::new(),
    );

    let history = agent.run(2).await.expect("agent run").clone();

    assert!(history.is_done());
    assert_eq!(
        history.compacted_memory.as_deref(),
        Some(
            "Task: collect receipt data. IN-PROGRESS: checkout was opened but receipt details still need confirmation."
        )
    );
    assert_eq!(history.compaction_count, 1);
    assert_eq!(history.last_compaction_step, Some(1));

    let checkpoint_json =
        serde_json::to_string_pretty(&agent.checkpoint()).expect("serialize checkpoint");
    let checkpoint: AgentCheckpoint =
        serde_json::from_str(&checkpoint_json).expect("deserialize checkpoint");
    assert_eq!(
        checkpoint.history.compacted_memory,
        history.compacted_memory
    );
    assert_eq!(checkpoint.history.compaction_count, 1);
    assert_eq!(checkpoint.history.last_compaction_step, Some(1));

    let requests = agent.llm.requests.lock().expect("requests lock");
    assert_eq!(requests.len(), 3);
    let compaction_text = request_text(&requests[1]);
    assert!(compaction_text.contains("Only mark a step as completed"));
    assert!(compaction_text.contains("<agent_history>"));
    assert!(compaction_text.contains("Opened the checkout"));
    assert!(compaction_text.contains("Waited for 0 seconds"));
    let second_step_prompt = request_text(&requests[2]);
    assert!(second_step_prompt.contains("<compacted_memory>"));
    assert!(second_step_prompt.contains("Treat as unverified context"));
    assert!(second_step_prompt.contains("receipt details still need confirmation"));
}

#[tokio::test]
async fn agent_run_skips_message_compaction_when_disabled() {
    let wait_output = serde_json::json!({
        "current_state": {
            "memory": "Still working"
        },
        "action": [
            {
                "wait": {
                    "seconds": 0
                }
            }
        ]
    });
    let done_output = serde_json::json!({
        "current_state": {
            "thinking": "done"
        },
        "action": [
            {
                "done": {
                    "text": "finished without compaction",
                    "success": true,
                    "files_to_display": []
                }
            }
        ]
    });
    let settings = AgentSettings {
        use_judge: false,
        message_compaction: MessageCompaction::Disabled,
        ..AgentSettings::default()
    };
    let mut agent = Agent::with_settings(
        "skip compaction",
        settings,
        QueueModel::new(vec![wait_output, done_output]),
        MockSession::new(),
    );

    let history = agent.run(2).await.expect("agent run");

    assert!(history.is_done());
    assert_eq!(history.compacted_memory, None);
    assert_eq!(history.compaction_count, 0);
    assert_eq!(agent.llm.requests.lock().expect("requests lock").len(), 2);
}

#[test]
fn generate_gif_output_path_preserves_upstream_default_and_custom_shapes() {
    assert_eq!(
        generate_gif_output_path(&GenerateGif::Enabled),
        Some(std::path::PathBuf::from("agent_history.gif"))
    );
    assert_eq!(
        generate_gif_output_path(&GenerateGif::Path("~/trace.gif".to_owned())),
        Some(expand_user_path("~/trace.gif"))
    );
    assert_eq!(generate_gif_output_path(&GenerateGif::Disabled), None);
}

#[tokio::test]
async fn agent_run_writes_generate_gif_custom_path_from_screenshots() {
    let mut state = blank_state();
    state.screenshot = Some(test_png_base64(8, 8));
    let temp_dir = tempfile::tempdir().expect("temp dir");
    let gif_path = temp_dir.path().join("history.gif");
    let output = serde_json::json!({
        "current_state": {},
        "action": [
            {
                "done": {
                    "text": "with gif",
                    "success": true,
                    "files_to_display": []
                }
            }
        ]
    });
    let settings = AgentSettings {
        use_judge: false,
        generate_gif: GenerateGif::Path(gif_path.display().to_string()),
        ..AgentSettings::default()
    };
    let mut agent = Agent::with_settings(
        "write gif",
        settings,
        QueueModel::new(vec![output]),
        MockSession::with_states(vec![state]),
    );

    agent.run(1).await.expect("agent run");

    let gif_bytes = std::fs::read(&gif_path).expect("gif bytes");
    assert!(gif_bytes.starts_with(b"GIF8"));
    assert!(gif_bytes.len() > 20);
}

#[tokio::test]
async fn agent_run_does_not_write_generate_gif_without_screenshots() {
    let temp_dir = tempfile::tempdir().expect("temp dir");
    let gif_path = temp_dir.path().join("empty.gif");
    let output = serde_json::json!({
        "current_state": {},
        "action": [
            {
                "done": {
                    "text": "no gif",
                    "success": true,
                    "files_to_display": []
                }
            }
        ]
    });
    let settings = AgentSettings {
        use_judge: false,
        generate_gif: GenerateGif::Path(gif_path.display().to_string()),
        ..AgentSettings::default()
    };
    let mut agent = Agent::with_settings(
        "skip empty gif",
        settings,
        QueueModel::new(vec![output]),
        MockSession::new(),
    );

    agent.run(1).await.expect("agent run");

    assert!(!gif_path.exists());
}

#[tokio::test]
async fn agent_run_ignores_message_compaction_failures() {
    let wait_output = serde_json::json!({
        "current_state": {
            "memory": "Collected partial evidence"
        },
        "action": [
            {
                "wait": {
                    "seconds": 0
                }
            }
        ]
    });
    let done_output = serde_json::json!({
        "current_state": {
            "thinking": "done"
        },
        "action": [
            {
                "done": {
                    "text": "finished after failed compaction",
                    "success": true,
                    "files_to_display": []
                }
            }
        ]
    });
    let settings = AgentSettings {
        use_judge: false,
        message_compaction: MessageCompaction::Settings(MessageCompactionSettings {
            compact_every_n_steps: 1,
            trigger_char_count: Some(1),
            ..MessageCompactionSettings::default()
        }),
        ..AgentSettings::default()
    };
    let mut agent = Agent::with_settings(
        "ignore compaction failure",
        settings,
        QueueModel::with_results(vec![
            Ok(wait_output),
            Err(LlmError::Provider("summary model unavailable".to_owned())),
            Ok(done_output),
        ]),
        MockSession::new(),
    );

    let history = agent.run(2).await.expect("agent run");

    assert!(history.is_done());
    assert_eq!(history.compacted_memory, None);
    assert_eq!(history.compaction_count, 0);
    let requests = agent.llm.requests.lock().expect("requests lock");
    assert_eq!(requests.len(), 3);
    assert!(request_text(&requests[1]).contains("<agent_history>"));
    assert!(!request_text(&requests[2]).contains("<compacted_memory>"));
}

#[tokio::test]
async fn agent_setting_can_attach_done_files_without_displaying_text() {
    let output = serde_json::json!({
        "current_state": {},
        "action": [
            {
                "done": {
                    "text": "Complete",
                    "success": true,
                    "files_to_display": ["report.md"]
                }
            }
        ]
    });
    let settings = AgentSettings {
        display_files_in_done_text: false,
        ..AgentSettings::default()
    };
    let mut agent = Agent::with_settings(
        "finish with report attachment",
        settings,
        QueueModel::new(vec![output]),
        MockSession::new(),
    );
    agent
        .file_system_mut()
        .write_file(&WriteFileAction {
            file_name: "report.md".to_owned(),
            content: "alpha\nbeta".to_owned(),
            append: false,
            trailing_newline: true,
            leading_newline: false,
        })
        .expect("write report");

    let history = agent.run(1).await.expect("agent run");

    assert_eq!(history.final_result(), Some("Complete"));
    let result = &history.items[0].result[0];
    assert_eq!(result.extracted_content.as_deref(), Some("Complete"));
    assert_eq!(result.attachments.len(), 1);
    assert!(result.attachments[0].ends_with("browseruse_agent_data/report.md"));
}

#[tokio::test]
async fn agent_run_recovers_from_invalid_model_output() {
    let invalid_output = serde_json::json!({
        "not_agent_output": true
    });
    let done_output = serde_json::json!({
        "current_state": {},
        "action": [
            {
                "done": {
                    "text": "recovered",
                    "success": true,
                    "files_to_display": []
                }
            }
        ]
    });
    let settings = AgentSettings {
        max_failures: 2,
        final_response_after_failure: false,
        ..AgentSettings::default()
    };
    let mut agent = Agent::with_settings(
        "recover",
        settings,
        QueueModel::new(vec![invalid_output, done_output]),
        MockSession::new(),
    );

    let history = agent.run(3).await.expect("agent run");

    assert_eq!(history.items.len(), 2);
    let first_metadata = history.items[0].metadata.as_ref().expect("first metadata");
    assert_eq!(first_metadata.step_number, 1);
    assert_eq!(first_metadata.step_interval, None);
    let second_metadata = history.items[1].metadata.as_ref().expect("second metadata");
    assert_eq!(second_metadata.step_number, 2);
    assert!(second_metadata.step_interval.unwrap_or(-1.0) >= 0.0);
    assert_eq!(history.final_result(), Some("recovered"));
    assert_eq!(history.is_successful(), Some(true));
    let errors = history.errors();
    assert_eq!(errors.len(), 2);
    let first_error = errors[0].expect("first step error");
    assert!(
        first_error.contains("invalid agent output"),
        "unexpected error: {}",
        first_error
    );
    assert_eq!(errors[1], None);
    assert!(history.has_errors());
}

#[tokio::test]
async fn agent_run_recovers_from_provider_structured_output_errors() {
    let done_output = serde_json::json!({
        "current_state": {},
        "action": [
            {
                "done": {
                    "text": "recovered from provider errors",
                    "success": true,
                    "files_to_display": []
                }
            }
        ]
    });
    let settings = AgentSettings {
        max_failures: 4,
        final_response_after_failure: false,
        ..AgentSettings::default()
    };
    let mut agent = Agent::with_settings(
        "recover provider failures",
        settings,
        QueueModel::with_results(vec![
            Err(LlmError::InvalidStructuredOutput(
                "chat completion tool call arguments: expected value".to_owned(),
            )),
            Err(LlmError::Provider(
                "model refused request: safety policy".to_owned(),
            )),
            Err(LlmError::Provider(
                "chat completion stopped with length before completing structured output"
                    .to_owned(),
            )),
            Ok(done_output),
        ]),
        MockSession::new(),
    );

    let history = agent.run(5).await.expect("agent run");

    assert_eq!(history.items.len(), 4);
    assert_eq!(
        history.final_result(),
        Some("recovered from provider errors")
    );
    let errors = history.errors();
    assert!(
        errors[0]
            .expect("malformed error")
            .contains("chat completion tool call arguments"),
        "unexpected error: {:?}",
        errors[0]
    );
    assert!(
        errors[1]
            .expect("refusal error")
            .contains("model refused request"),
        "unexpected error: {:?}",
        errors[1]
    );
    assert!(
        errors[2]
            .expect("truncation error")
            .contains("stopped with length"),
        "unexpected error: {:?}",
        errors[2]
    );
    assert_eq!(errors[3], None);
}

#[tokio::test]
async fn agent_run_enforces_max_failures() {
    let output = serde_json::json!({
        "current_state": {},
        "action": [
            {
                "click": {
                    "coordinate_x": 10
                }
            }
        ]
    });
    let settings = AgentSettings {
        max_failures: 2,
        final_response_after_failure: false,
        ..AgentSettings::default()
    };
    let mut agent = Agent::with_settings(
        "find buttons",
        settings,
        QueueModel::new(vec![output.clone(), output]),
        MockSession::new(),
    );

    let error = agent.run(5).await.expect_err("max failures");

    assert!(matches!(
        error,
        AgentRunError::MaxFailuresExceeded { failures: 2 }
    ));
    assert_eq!(agent.history().items.len(), 2);
}

#[tokio::test]
async fn agent_run_requests_final_response_after_max_failures() {
    let invalid_output = serde_json::json!({
        "not_agent_output": true
    });
    let final_done = serde_json::json!({
        "current_state": {},
        "action": [
            {
                "done": {
                    "text": "could not finish, but here is what I found",
                    "success": false,
                    "files_to_display": []
                }
            }
        ]
    });
    let settings = AgentSettings {
        max_failures: 2,
        use_judge: false,
        ..AgentSettings::default()
    };
    let mut agent = Agent::with_settings(
        "recover with final answer",
        settings,
        QueueModel::new(vec![invalid_output.clone(), invalid_output, final_done]),
        MockSession::new(),
    );

    let history = agent.run(5).await.expect("final response");

    assert_eq!(history.items.len(), 3);
    assert_eq!(
        history.final_result(),
        Some("could not finish, but here is what I found")
    );
    assert_eq!(history.is_successful(), Some(false));
    assert_eq!(history.errors().len(), 3);
    assert!(history.errors()[0].is_some());
    assert!(history.errors()[1].is_some());
    assert_eq!(history.errors()[2], None);

    let requests = agent.llm.requests.lock().expect("requests lock");
    assert_eq!(requests.len(), 3);
    let final_request_text = request_text(&requests[2]);
    assert!(final_request_text.contains("You failed 2 times"));
    assert!(final_request_text.contains("Your only available action is done"));
    assert!(final_request_text.contains("set success to false"));
}

#[tokio::test]
async fn final_response_after_failure_rejects_non_done_actions_before_side_effects() {
    let invalid_output = serde_json::json!({
        "not_agent_output": true
    });
    let final_click = serde_json::json!({
        "current_state": {},
        "action": [
            {
                "click": {
                    "index": 1
                }
            }
        ]
    });
    let settings = AgentSettings {
        max_failures: 1,
        ..AgentSettings::default()
    };
    let mut agent = Agent::with_settings(
        "avoid side effects after failure",
        settings,
        QueueModel::new(vec![invalid_output, final_click]),
        MockSession::new(),
    );

    let error = agent.run(5).await.expect_err("max failures");

    assert!(matches!(
        error,
        AgentRunError::MaxFailuresExceeded { failures: 1 }
    ));
    assert!(agent.executor.session().events().is_empty());
    assert_eq!(agent.history().items.len(), 2);
    let errors = agent.history().errors();
    assert!(
        errors[1]
            .expect("final response error")
            .contains("exactly one done action")
    );
}

#[tokio::test]
async fn agent_run_reports_step_limit() {
    let output = serde_json::json!({
        "current_state": {},
        "action": [
            {
                "click": {
                    "index": 1
                }
            }
        ]
    });
    let mut agent = Agent::new(
        "click once",
        QueueModel::new(vec![output]),
        MockSession::new(),
    );

    let error = agent.run(1).await.expect_err("step limit");

    assert!(matches!(
        error,
        AgentRunError::StepLimitReached { max_steps: 1 }
    ));
    assert_eq!(agent.history().items.len(), 1);
}

#[tokio::test]
async fn agent_run_uses_final_step_done_response_to_finish() {
    let output = serde_json::json!({
        "current_state": {},
        "action": [
            {
                "done": {
                    "text": "finished with final step",
                    "success": true,
                    "files_to_display": []
                }
            }
        ]
    });
    let mut agent = Agent::new(
        "finish on final step",
        QueueModel::new(vec![output]),
        MockSession::new(),
    );

    let history = agent.run(1).await.expect("final step done");

    assert_eq!(history.final_result(), Some("finished with final step"));
    assert_eq!(history.items.len(), 1);
}

#[tokio::test]
async fn agent_run_rejects_non_done_final_step_without_side_effects() {
    let output = serde_json::json!({
        "current_state": {},
        "action": [
            {
                "click": {
                    "index": 1
                }
            }
        ]
    });
    let mut agent = Agent::new(
        "do not click on final step",
        QueueModel::new(vec![output]),
        MockSession::new(),
    );

    let error = agent.run(1).await.expect_err("step limit");

    assert!(matches!(
        error,
        AgentRunError::StepLimitReached { max_steps: 1 }
    ));
    assert!(agent.executor.session().events().is_empty());
    assert_eq!(
        agent.history().errors()[0],
        Some("final response at step limit must return exactly one done action")
    );
}

#[tokio::test]
async fn agent_run_injects_budget_warning_before_final_step_only() {
    let click_output = serde_json::json!({
        "current_state": {},
        "action": [
            {
                "click": {
                    "index": 1
                }
            }
        ]
    });
    let done_output = serde_json::json!({
        "current_state": {},
        "action": [
            {
                "done": {
                    "text": "finished after warning",
                    "success": true,
                    "files_to_display": []
                }
            }
        ]
    });
    let settings = AgentSettings {
        use_judge: false,
        ..AgentSettings::default()
    };
    let mut agent = Agent::with_settings(
        "warn before final step",
        settings,
        QueueModel::new(vec![
            click_output.clone(),
            click_output.clone(),
            click_output,
            done_output,
        ]),
        MockSession::new(),
    );

    agent.run(4).await.expect("final step done");

    let requests = agent.llm.requests.lock().expect("requests lock");
    assert_eq!(requests.len(), 4);
    assert!(!request_text(&requests[0]).contains("BUDGET WARNING"));
    assert!(!request_text(&requests[1]).contains("BUDGET WARNING"));
    assert!(
        request_text(&requests[2])
            .contains("BUDGET WARNING: You have used 3/4 steps (75%). 1 steps remaining.")
    );
    let final_request = request_text(&requests[3]);
    assert!(final_request.contains("You reached max_steps (4)"));
    assert!(!final_request.contains("BUDGET WARNING"));
}

#[tokio::test]
async fn agent_run_detects_repeated_action_loop() {
    let output = serde_json::json!({
        "current_state": {},
        "action": [
            {
                "click": {
                    "index": 1
                }
            }
        ]
    });
    let settings = AgentSettings {
        loop_detection_window: 2,
        ..AgentSettings::default()
    };
    let mut agent = Agent::with_settings(
        "do not loop",
        settings,
        QueueModel::new(vec![output.clone(), output]),
        MockSession::new(),
    );

    let error = agent.run(5).await.expect_err("loop detected");

    assert!(matches!(error, AgentRunError::LoopDetected { window: 2 }));
    assert_eq!(agent.history().items.len(), 2);
}

#[tokio::test]
async fn agent_loop_detection_can_be_disabled() {
    let output = serde_json::json!({
        "current_state": {},
        "action": [
            {
                "click": {
                    "index": 1
                }
            }
        ]
    });
    let settings = AgentSettings {
        loop_detection_window: 2,
        loop_detection_enabled: false,
        ..AgentSettings::default()
    };
    let mut agent = Agent::with_settings(
        "loop if allowed",
        settings,
        QueueModel::new(vec![output.clone(), output]),
        MockSession::new(),
    );

    let error = agent.run(2).await.expect_err("step limit");

    assert!(matches!(
        error,
        AgentRunError::StepLimitReached { max_steps: 2 }
    ));
    assert_eq!(agent.history().items.len(), 2);
}

#[test]
fn repeated_action_loop_uses_normalized_action_signatures() {
    let history = AgentHistory {
        items: vec![
            history_item_with_actions(vec![BrowserAction::Search(
                browser_use_tools::SearchAction {
                    query: "EvalOps browser-use".to_owned(),
                    engine: SearchEngine::Google,
                },
            )]),
            history_item_with_actions(vec![BrowserAction::Search(
                browser_use_tools::SearchAction {
                    query: "browser use EvalOps!!!".to_owned(),
                    engine: SearchEngine::Google,
                },
            )]),
        ],
        ..AgentHistory::default()
    };

    assert!(repeated_action_loop(&history, 2));
}

#[test]
fn repeated_action_loop_ignores_wait_only_steps() {
    let history = AgentHistory {
        items: vec![
            history_item_with_actions(vec![BrowserAction::Wait(browser_use_tools::WaitAction {
                seconds: 3,
            })]),
            history_item_with_actions(vec![BrowserAction::Wait(browser_use_tools::WaitAction {
                seconds: 3,
            })]),
        ],
        ..AgentHistory::default()
    };

    assert!(!repeated_action_loop(&history, 2));
}

#[test]
fn step_request_includes_previous_results() {
    let mut state = blank_state();
    state.dom_state = SerializedDomState::from_elements(vec![
        browser_use_dom::DomElementRef {
            index: 1,
            target_id: "target".to_owned(),
            backend_node_id: 1,
            node_id: None,
            tag_name: "a".to_owned(),
            role: Some("link".to_owned()),
            name: Some("Docs".to_owned()),
            text: Some("Docs".to_owned()),
            attributes: BTreeMap::from([("href".to_owned(), "/docs".to_owned())]),
            bounds: None,
            is_visible: true,
            is_interactive: true,
            is_scrollable: false,
        },
        browser_use_dom::DomElementRef {
            index: 2,
            target_id: "target".to_owned(),
            backend_node_id: 2,
            node_id: None,
            tag_name: "div".to_owned(),
            role: None,
            name: Some("Results".to_owned()),
            text: None,
            attributes: BTreeMap::new(),
            bounds: None,
            is_visible: true,
            is_interactive: true,
            is_scrollable: true,
        },
    ])
    .with_page_stats(browser_use_dom::DomPageStats {
        links: 1,
        iframes: 1,
        shadow_open: 1,
        shadow_closed: 0,
        scroll_containers: 1,
        images: 3,
        interactive_elements: 2,
        total_elements: 30,
        text_chars: 120,
    });
    state.tabs = vec![browser_use_dom::TabInfo {
        url: "https://example.com/docs".to_owned(),
        title: "Docs".to_owned(),
        tab_id: browser_use_dom::TabInfo::tab_id_for_target("target-1234abcd"),
        target_id: "target-1234abcd".to_owned(),
        parent_target_id: None,
    }];
    let history = AgentHistory {
        items: vec![AgentHistoryItem {
            model_output: None,
            result: vec![ActionResult::extracted("Clicked element 1")],
            state: blank_state(),
            metadata: None,
        }],
        ..AgentHistory::default()
    };

    let request = build_step_request("keep going", &state, &history, &AgentSettings::default())
        .expect("step request");
    let request_text = serde_json::to_string(&request.messages).expect("messages json");
    let user_message = request
        .messages
        .iter()
        .find(|message| message.role == MessageRole::User)
        .expect("user message");
    let user_text = match &user_message.content[0] {
        ContentPart::Text { text } => text,
        other => panic!("unexpected first content part: {other:?}"),
    };

    assert!(user_text.contains("<agent_history>"));
    assert!(user_text.contains("<step>\nResult\nClicked element 1"));
    assert!(user_text.contains("</agent_history>"));
    assert!(user_text.contains("<agent_state>"));
    assert!(user_text.contains("Page stats"));
    assert!(user_text.contains("1 links, 2 interactive, 1 iframes"));
    assert!(user_text.contains("1 shadow(open), 0 shadow(closed)"));
    assert!(user_text.contains("3 images"));
    assert!(user_text.contains("1 scroll containers"));
    assert!(user_text.contains("30 total elements, 120 text chars"));
    assert!(user_text.contains("</agent_state>"));
    assert!(user_text.contains("<browser_state>"));
    assert!(user_text.contains("</browser_state>"));
    assert!(user_text.contains(r#""tab_id": "abcd""#));
    assert!(request_text.contains("Avoid repeating the same action sequence"));
}

#[test]
fn step_request_includes_loading_page_stats_hint_like_upstream() {
    let mut state = blank_state();
    state.dom_state =
        SerializedDomState::default().with_page_stats(browser_use_dom::DomPageStats {
            links: 0,
            iframes: 0,
            shadow_open: 0,
            shadow_closed: 0,
            scroll_containers: 0,
            images: 0,
            interactive_elements: 0,
            total_elements: 25,
            text_chars: 10,
        });
    let request = build_step_request(
        "wait for app",
        &state,
        &AgentHistory::default(),
        &AgentSettings::default(),
    )
    .expect("step request");
    let user_message = request
        .messages
        .iter()
        .find(|message| message.role == MessageRole::User)
        .expect("user message");
    let user_text = match &user_message.content[0] {
        ContentPart::Text { text } => text,
        other => panic!("unexpected first content part: {other:?}"),
    };

    assert!(
        user_text.contains("Page appears to show skeleton/placeholder content (still loading?)")
    );
    assert!(user_text.contains("25 total elements"));
}

#[test]
fn step_request_omits_recent_events_by_default() {
    let mut state = blank_state();
    state.recent_events = Some("Blocked popup https://tracker.example.test".to_owned());

    let request = build_step_request(
        "inspect page",
        &state,
        &AgentHistory::default(),
        &AgentSettings::default(),
    )
    .expect("step request");
    let user_text = request_text(&request);

    assert!(!user_text.contains("recent_events"));
    assert!(!user_text.contains("tracker.example.test"));
}

#[test]
fn step_request_includes_recent_events_when_enabled() {
    let mut state = blank_state();
    state.recent_events = Some("Blocked popup https://tracker.example.test".to_owned());
    let settings = AgentSettings {
        include_recent_events: true,
        ..AgentSettings::default()
    };

    let request = build_step_request("inspect page", &state, &AgentHistory::default(), &settings)
        .expect("step request");
    let user_text = request_text(&request);

    assert!(user_text.contains(r#""recent_events": "Blocked popup https://tracker.example.test""#));
}

#[test]
fn step_request_includes_loop_awareness_nudge() {
    let mut state = blank_state();
    state.dom_state.text = "[1] <button> Refresh".to_owned();
    let history = AgentHistory {
        items: (0..5)
            .map(|_| AgentHistoryItem {
                model_output: Some(AgentOutput {
                    current_state: AgentCurrentState {
                        thinking: None,
                        evaluation_previous_goal: None,
                        memory: None,
                        next_goal: None,
                    },
                    thinking: None,
                    evaluation_previous_goal: None,
                    memory: None,
                    next_goal: None,
                    current_plan_item: None,
                    plan_update: None,
                    action: vec![BrowserAction::Click(ClickElementAction {
                        index: Some(1),
                        coordinate_x: None,
                        coordinate_y: None,
                    })],
                }),
                result: vec![ActionResult::extracted("Clicked element 1")],
                state: state.clone(),
                metadata: None,
            })
            .collect(),
        ..AgentHistory::default()
    };

    let request = build_step_request("unstick", &state, &history, &AgentSettings::default())
        .expect("step request");
    let request_text = serde_json::to_string(&request.messages).expect("messages json");

    assert!(request_text.contains("Loop awareness"));
    assert!(request_text.contains("repeated a similar action 5 times"));
    assert!(request_text.contains("page content has not changed across 5"));

    let disabled_settings = AgentSettings {
        loop_detection_enabled: false,
        ..AgentSettings::default()
    };
    let request =
        build_step_request("unstick", &state, &history, &disabled_settings).expect("step request");
    let request_text = serde_json::to_string(&request.messages).expect("messages json");
    assert!(!request_text.contains("Loop awareness"));
}

#[test]
fn step_request_includes_planning_context_and_replan_nudge() {
    let state = blank_state();
    let history = AgentHistory {
        items: (0..3)
            .map(|_| AgentHistoryItem {
                model_output: None,
                result: vec![ActionResult::error("stalled")],
                state: blank_state(),
                metadata: None,
            })
            .collect(),
        ..AgentHistory::default()
    };

    let request = build_step_request("recover", &state, &history, &AgentSettings::default())
        .expect("step request");
    let request_text = serde_json::to_string(&request.messages).expect("messages json");

    assert!(request_text.contains("Planning"));
    assert!(request_text.contains("current_plan_item"));
    assert!(request_text.contains("plan_update"));
    assert!(request_text.contains("revise the plan"));

    let disabled_settings = AgentSettings {
        enable_planning: false,
        ..AgentSettings::default()
    };
    let request =
        build_step_request("recover", &state, &history, &disabled_settings).expect("step request");
    let request_text = serde_json::to_string(&request.messages).expect("messages json");
    assert!(!request_text.contains("Planning"));

    let flash_settings = AgentSettings {
        flash_mode: true,
        ..AgentSettings::default()
    };
    let request =
        build_step_request("recover", &state, &history, &flash_settings).expect("step request");
    let request_text = serde_json::to_string(&request.messages).expect("messages json");
    assert!(!request_text.contains("Planning"));
}

#[test]
fn step_request_includes_available_file_paths_like_upstream() {
    let settings = AgentSettings {
        available_file_paths: vec!["/tmp/report.pdf".to_owned(), "/tmp/chart.png".to_owned()],
        ..AgentSettings::default()
    };

    let request = build_step_request(
        "inspect files",
        &blank_state(),
        &AgentHistory::default(),
        &settings,
    )
    .expect("step request");
    let user_message = request
        .messages
        .iter()
        .find(|message| message.role == MessageRole::User)
        .expect("user message");
    let user_text = match &user_message.content[0] {
        ContentPart::Text { text } => text,
        other => panic!("unexpected first content part: {other:?}"),
    };

    assert!(user_text.contains(
            "<available_file_paths>/tmp/report.pdf\n/tmp/chart.png\nUse with absolute paths</available_file_paths>"
        ));
    assert!(user_text.contains("<agent_state>"));
    assert!(user_text.contains("</agent_state>"));
}

#[test]
fn step_request_includes_managed_file_system_context() {
    let temp_dir = tempfile::tempdir().expect("temp dir");
    let mut file_system = ManagedFileSystem::new(temp_dir.path()).expect("managed file system");
    let empty_request = build_step_request_with_file_system(
        "use the files",
        &blank_state(),
        &AgentHistory::default(),
        &AgentSettings::default(),
        Some(&file_system),
    )
    .expect("empty file system request");
    let empty_text = request_text(&empty_request);

    assert!(empty_text.contains("<file_system>\n\n</file_system>"));
    assert!(
        empty_text.contains(
            "<todo_contents>\n[empty todo.md, fill it when applicable]\n</todo_contents>"
        )
    );

    file_system
        .write_file(&WriteFileAction {
            file_name: "todo.md".to_owned(),
            content: "- inspect report".to_owned(),
            append: false,
            trailing_newline: false,
            leading_newline: false,
        })
        .expect("write todo");
    file_system
        .write_file(&WriteFileAction {
            file_name: "report.md".to_owned(),
            content: "alpha\nbeta".to_owned(),
            append: false,
            trailing_newline: false,
            leading_newline: false,
        })
        .expect("write report");

    let request = build_step_request_with_file_system(
        "use the files",
        &blank_state(),
        &AgentHistory::default(),
        &AgentSettings::default(),
        Some(&file_system),
    )
    .expect("file system request");
    let text = request_text(&request);

    assert!(text.contains("<file_system>\n<file>\nreport.md - 2 lines"));
    assert!(text.contains("<content>\nalpha\nbeta\n</content>"));
    assert!(text.contains("<todo_contents>\n- inspect report\n</todo_contents>"));
    assert!(!text.contains("todo.md -"));
}

#[test]
fn large_extract_results_save_to_managed_file_system() {
    let temp_dir = tempfile::tempdir().expect("temp dir");
    let mut file_system = ManagedFileSystem::new(temp_dir.path()).expect("managed file system");
    let page_text = "EvalOps extracted content. ".repeat(700);

    let result = extract_action_result(
        &ExtractAction {
            query: "summarize".to_owned(),
            extract_links: false,
            extract_images: false,
            start_from_char: 0,
            output_schema: None,
            already_collected: Vec::new(),
        },
        &page_text,
        Some("https://example.test/report"),
        false,
        None,
        None,
        Some(&mut file_system),
    );

    assert!(
        result
            .long_term_memory
            .as_deref()
            .expect("memory")
            .contains("Content in extracted_content_0.md and once in <read_state>.")
    );
    let saved = file_system
        .display_file("extracted_content_0.md")
        .expect("saved extracted content");
    assert!(saved.contains("<query>\nsummarize\n</query>"));
    assert!(saved.contains("EvalOps extracted content."));
    assert_eq!(file_system.get_state().extracted_content_count, 1);
}

#[test]
fn step_request_honors_system_message_override_and_extension() {
    let extended_settings = AgentSettings {
        max_actions_per_step: 2,
        extend_system_message: Some("Prefer stable selectors when possible.".to_owned()),
        ..AgentSettings::default()
    };
    let request = build_step_request(
        "inspect",
        &blank_state(),
        &AgentHistory::default(),
        &extended_settings,
    )
    .expect("step request");
    let system_text = request
        .messages
        .iter()
        .find(|message| message.role == MessageRole::System)
        .and_then(|message| match &message.content[0] {
            ContentPart::Text { text } => Some(text.as_str()),
            ContentPart::ImageUrl { .. } => None,
        })
        .expect("system message");

    assert!(system_text.contains("Use at most 2 actions"));
    assert!(system_text.contains("Prefer stable selectors when possible."));

    let override_settings = AgentSettings {
        override_system_message: Some("Return only the agreed JSON contract.".to_owned()),
        extend_system_message: Some("Prefer compact actions.".to_owned()),
        ..AgentSettings::default()
    };
    let request = build_step_request(
        "inspect",
        &blank_state(),
        &AgentHistory::default(),
        &override_settings,
    )
    .expect("step request");
    let system_text = request
        .messages
        .iter()
        .find(|message| message.role == MessageRole::System)
        .and_then(|message| match &message.content[0] {
            ContentPart::Text { text } => Some(text.as_str()),
            ContentPart::ImageUrl { .. } => None,
        })
        .expect("system message");

    assert_eq!(
        system_text,
        "Return only the agreed JSON contract.\nPrefer compact actions."
    );
}

#[test]
fn step_request_includes_sensitive_data_placeholders_like_upstream() {
    let mut state = blank_state();
    state.url = "https://secure.example.test/login".to_owned();
    let settings = AgentSettings {
        sensitive_data: BTreeMap::from([
            (
                "*.example.test".to_owned(),
                SensitiveDataValue::Domain(BTreeMap::from([(
                    "password".to_owned(),
                    "super-secret".to_owned(),
                )])),
            ),
            (
                "username".to_owned(),
                SensitiveDataValue::Value("evalops@example.test".to_owned()),
            ),
        ]),
        ..AgentSettings::default()
    };

    let request = build_step_request("log in", &state, &AgentHistory::default(), &settings)
        .expect("step request");
    let user_message = request
        .messages
        .iter()
        .find(|message| message.role == MessageRole::User)
        .expect("user message");
    let user_text = match &user_message.content[0] {
        ContentPart::Text { text } => text,
        other => panic!("unexpected first content part: {other:?}"),
    };

    assert!(user_text.contains("<sensitive_data>SENSITIVE DATA"));
    assert!(user_text.contains("  - password"));
    assert!(user_text.contains("  - username"));
    assert!(user_text.contains("use: <secret>password</secret>"));
    assert!(!user_text.contains("super-secret"));
    assert!(!user_text.contains("evalops@example.test"));
}

#[test]
fn step_request_filters_domain_scoped_sensitive_data_by_url() {
    let mut state = blank_state();
    state.url = "https://other.example.test/login".to_owned();
    let settings = AgentSettings {
        sensitive_data: BTreeMap::from([
            (
                "secure.example.test".to_owned(),
                SensitiveDataValue::Domain(BTreeMap::from([(
                    "password".to_owned(),
                    "super-secret".to_owned(),
                )])),
            ),
            (
                "username".to_owned(),
                SensitiveDataValue::Value("evalops@example.test".to_owned()),
            ),
        ]),
        ..AgentSettings::default()
    };

    let request = build_step_request("log in", &state, &AgentHistory::default(), &settings)
        .expect("step request");
    let user_text = request_text(&request);

    assert!(user_text.contains("<sensitive_data>SENSITIVE DATA"));
    assert!(user_text.contains("  - username"));
    assert!(!user_text.contains("  - password"));
    assert!(!user_text.contains("super-secret"));
    assert!(!user_text.contains("evalops@example.test"));
}

#[test]
fn sensitive_data_domain_pattern_matching_follows_upstream_security_defaults() {
    assert!(match_url_with_domain_pattern(
        "https://secure.example.test/login",
        "secure.example.test"
    ));
    assert!(!match_url_with_domain_pattern(
        "http://secure.example.test/login",
        "secure.example.test"
    ));
    assert!(match_url_with_domain_pattern(
        "http://secure.example.test/login",
        "http*://secure.example.test"
    ));
    assert!(match_url_with_domain_pattern(
        "https://child.example.test",
        "*.example.test"
    ));
    assert!(match_url_with_domain_pattern(
        "https://example.test",
        "*.example.test"
    ));
    assert!(match_url_with_domain_pattern(
        "chrome-extension://aaaaaaaaaaaa/options",
        "chrome-extension://*"
    ));
    assert!(!match_url_with_domain_pattern("about:blank", "*"));
    assert!(!match_url_with_domain_pattern(
        "https://deep.example.test",
        "*.*.example.test"
    ));
}

#[test]
fn step_request_redacts_sensitive_values_from_state_and_history() {
    let mut state = blank_state();
    state.url = "https://secure.example.test/login".to_owned();
    state.dom_state.text = "Password field currently contains super-secret".to_owned();
    let history = AgentHistory {
        items: vec![AgentHistoryItem {
            model_output: None,
            result: vec![ActionResult::extracted(
                "Previous action saw token sk-live-123",
            )],
            state: blank_state(),
            metadata: None,
        }],
        ..AgentHistory::default()
    };
    let settings = AgentSettings {
        sensitive_data: BTreeMap::from([
            (
                "password".to_owned(),
                SensitiveDataValue::Value("super-secret".to_owned()),
            ),
            (
                "api_key".to_owned(),
                SensitiveDataValue::Value("sk-live-123".to_owned()),
            ),
        ]),
        ..AgentSettings::default()
    };

    let request =
        build_step_request("continue", &state, &history, &settings).expect("step request");
    let user_text = request_text(&request);

    assert!(user_text.contains("<secret>password</secret>"));
    assert!(user_text.contains("<secret>api_key</secret>"));
    assert!(!user_text.contains("super-secret"));
    assert!(!user_text.contains("sk-live-123"));
}

#[test]
fn step_request_attaches_screenshot_as_image_part() {
    let mut state = blank_state();
    state.screenshot = Some("abc123".to_owned());

    let request = build_step_request(
        "inspect screenshot",
        &state,
        &AgentHistory::default(),
        &AgentSettings::default(),
    )
    .expect("step request");
    let user_message = request
        .messages
        .iter()
        .find(|message| message.role == MessageRole::User)
        .expect("user message");

    assert!(matches!(
        &user_message.content[1],
        ContentPart::ImageUrl { image_url, detail } if image_url == "data:image/png;base64,abc123"
            && *detail == Some(ImageDetailLevel::Auto)
    ));
    let text = match &user_message.content[0] {
        ContentPart::Text { text } => text,
        other => panic!("unexpected first content part: {other:?}"),
    };
    assert!(text.contains("<browser_state>"));
    assert!(!text.contains("abc123"));
}

#[test]
fn step_request_resizes_screenshot_for_llm_prompt_only() {
    let original_screenshot = test_png_base64(240, 160);
    let mut state = blank_state();
    state.screenshot = Some(original_screenshot.clone());
    let settings = AgentSettings {
        llm_screenshot_size: Some(LlmScreenshotSize::new(120, 100).expect("valid size")),
        ..AgentSettings::default()
    };

    let request = build_step_request(
        "inspect resized screenshot",
        &state,
        &AgentHistory::default(),
        &settings,
    )
    .expect("step request");
    let user_message = request
        .messages
        .iter()
        .find(|message| message.role == MessageRole::User)
        .expect("user message");
    let ContentPart::ImageUrl { image_url, detail } = &user_message.content[1] else {
        panic!("expected prompt screenshot image");
    };

    assert_eq!(*detail, Some(ImageDetailLevel::Auto));
    assert_eq!(png_dimensions_from_data_url(image_url), (120, 100));
    assert_eq!(
        state.screenshot.as_deref(),
        Some(original_screenshot.as_str())
    );
}

#[test]
fn step_request_attaches_latest_action_result_images_as_image_parts() {
    let history = AgentHistory {
        items: vec![
            AgentHistoryItem {
                model_output: None,
                result: vec![ActionResult {
                    extracted_content: Some("old image".to_owned()),
                    error: None,
                    judgement: None,
                    long_term_memory: Some("old image".to_owned()),
                    include_extracted_content_only_once: true,
                    include_in_memory: true,
                    is_done: false,
                    success: None,
                    attachments: Vec::new(),
                    images: vec![serde_json::json!({
                        "name": "old.jpg",
                        "data": "old-data",
                    })],
                    metadata: None,
                }],
                state: blank_state(),
                metadata: None,
            },
            AgentHistoryItem {
                model_output: None,
                result: vec![ActionResult {
                    extracted_content: Some("Read image file chart.png.".to_owned()),
                    error: None,
                    judgement: None,
                    long_term_memory: Some("Read image file chart.png".to_owned()),
                    include_extracted_content_only_once: true,
                    include_in_memory: true,
                    is_done: false,
                    success: None,
                    attachments: Vec::new(),
                    images: vec![serde_json::json!({
                        "name": "chart.png",
                        "data": "abc123",
                    })],
                    metadata: None,
                }],
                state: blank_state(),
                metadata: None,
            },
        ],
        ..AgentHistory::default()
    };
    let settings = AgentSettings {
        use_vision: VisionMode::Never,
        vision_detail_level: ImageDetailLevel::High,
        ..AgentSettings::default()
    };

    let request = build_step_request("inspect file image", &blank_state(), &history, &settings)
        .expect("step request");
    let user_message = request
        .messages
        .iter()
        .find(|message| message.role == MessageRole::User)
        .expect("user message");

    assert!(matches!(
        &user_message.content[1],
        ContentPart::Text { text } if text == "Image from file: chart.png"
    ));
    assert!(matches!(
        &user_message.content[2],
        ContentPart::ImageUrl { image_url, detail } if image_url == "data:image/png;base64,abc123"
            && *detail == Some(ImageDetailLevel::High)
    ));
    let request_text = serde_json::to_string(user_message).expect("message json");
    assert!(!request_text.contains("old-data"));
}

#[test]
fn step_request_inserts_sample_images_before_runtime_images() {
    let mut state = blank_state();
    state.screenshot = Some("screen-data".to_owned());
    let history = AgentHistory {
        items: vec![AgentHistoryItem {
            model_output: None,
            result: vec![ActionResult {
                extracted_content: Some("Read image file chart.png.".to_owned()),
                error: None,
                judgement: None,
                long_term_memory: Some("Read image file chart.png".to_owned()),
                include_extracted_content_only_once: true,
                include_in_memory: true,
                is_done: false,
                success: None,
                attachments: Vec::new(),
                images: vec![serde_json::json!({
                    "name": "chart.png",
                    "data": "chart-data",
                })],
                metadata: None,
            }],
            state: blank_state(),
            metadata: None,
        }],
        ..AgentHistory::default()
    };
    let settings = AgentSettings {
        vision_detail_level: ImageDetailLevel::High,
        sample_images: vec![
            ContentPart::Text {
                text: "Sample reference:".to_owned(),
            },
            ContentPart::ImageUrl {
                image_url: "data:image/png;base64,sample-data".to_owned(),
                detail: Some(ImageDetailLevel::Low),
            },
        ],
        ..AgentSettings::default()
    };

    let request =
        build_step_request("inspect samples", &state, &history, &settings).expect("step request");
    let user_message = request
        .messages
        .iter()
        .find(|message| message.role == MessageRole::User)
        .expect("user message");

    assert!(matches!(
        &user_message.content[0],
        ContentPart::Text { text } if text.contains("<browser_state>")
    ));
    assert!(matches!(
        &user_message.content[1],
        ContentPart::Text { text } if text == "Sample reference:"
    ));
    assert!(matches!(
        &user_message.content[2],
        ContentPart::ImageUrl { image_url, detail }
            if image_url == "data:image/png;base64,sample-data"
                && *detail == Some(ImageDetailLevel::Low)
    ));
    assert!(matches!(
        &user_message.content[3],
        ContentPart::ImageUrl { image_url, detail }
            if image_url == "data:image/png;base64,screen-data"
                && *detail == Some(ImageDetailLevel::High)
    ));
    assert!(matches!(
        &user_message.content[4],
        ContentPart::Text { text } if text == "Image from file: chart.png"
    ));
    assert!(matches!(
        &user_message.content[5],
        ContentPart::ImageUrl { image_url, detail }
            if image_url == "data:image/png;base64,chart-data"
                && *detail == Some(ImageDetailLevel::High)
    ));
}

#[test]
fn step_request_uses_custom_dom_include_attributes() {
    let mut state = blank_state();
    state.dom_state = SerializedDomState::from_elements(vec![browser_use_dom::DomElementRef {
        index: 1,
        target_id: "target".to_owned(),
        backend_node_id: 1,
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
    }]);
    let settings = AgentSettings {
        include_attributes: vec!["data-testid".to_owned()],
        ..AgentSettings::default()
    };

    let request = build_step_request("click run", &state, &AgentHistory::default(), &settings)
        .expect("step request");
    let user_message = request
        .messages
        .iter()
        .find(|message| message.role == MessageRole::User)
        .expect("user message");
    let text = match &user_message.content[0] {
        ContentPart::Text { text } => text,
        other => panic!("unexpected first content part: {other:?}"),
    };

    assert!(text.contains(r#""text": "[1] <button data-testid=run-action> Run""#));
    assert!(!text.contains(r#""text": "[1] <button id=run> Run""#));
}

#[test]
fn step_request_truncates_large_clickable_element_text() {
    let mut state = blank_state();
    state.dom_state.text = "abcdef".repeat(20);
    let settings = AgentSettings {
        max_clickable_elements_length: 12,
        ..AgentSettings::default()
    };

    let request = build_step_request("summarize", &state, &AgentHistory::default(), &settings)
        .expect("step request");
    let user_message = request
        .messages
        .iter()
        .find(|message| message.role == MessageRole::User)
        .expect("user message");
    let text = match &user_message.content[0] {
        ContentPart::Text { text } => text,
        other => panic!("unexpected first content part: {other:?}"),
    };

    assert!(text.contains("abcdefabcdef"));
    assert!(text.contains("[clickable elements truncated to 12 chars]"));
    assert!(!text.contains("abcdefabcdefabcdef"));
}

#[test]
fn step_request_omits_screenshot_when_vision_disabled() {
    let mut state = blank_state();
    state.screenshot = Some("abc123".to_owned());
    let settings = AgentSettings {
        use_vision: VisionMode::Never,
        ..AgentSettings::default()
    };

    let request = build_step_request("no screenshot", &state, &AgentHistory::default(), &settings)
        .expect("step request");
    let user_message = request
        .messages
        .iter()
        .find(|message| message.role == MessageRole::User)
        .expect("user message");

    assert_eq!(user_message.content.len(), 1);
    let request_text = serde_json::to_string(user_message).expect("message json");
    assert!(!request_text.contains("abc123"));
}

#[test]
fn step_request_includes_screenshot_when_auto_vision_state_has_one() {
    let mut state = blank_state();
    state.screenshot = Some("abc123".to_owned());
    let settings = AgentSettings {
        use_vision: VisionMode::Auto,
        ..AgentSettings::default()
    };

    let request = build_step_request(
        "include requested screenshot",
        &state,
        &AgentHistory::default(),
        &settings,
    )
    .expect("step request");
    let user_message = request
        .messages
        .iter()
        .find(|message| message.role == MessageRole::User)
        .expect("user message");

    assert!(matches!(
        &user_message.content[1],
        ContentPart::ImageUrl { image_url, detail }
            if image_url == "data:image/png;base64,abc123"
                && *detail == Some(ImageDetailLevel::Auto)
    ));
}

#[test]
fn previous_results_only_include_one_time_extractions_once() {
    let history = AgentHistory {
        items: vec![
            AgentHistoryItem {
                model_output: None,
                result: vec![ActionResult {
                    extracted_content: Some("very large extracted payload".to_owned()),
                    error: None,
                    judgement: None,
                    long_term_memory: Some("Large extraction saved to file".to_owned()),
                    include_extracted_content_only_once: true,
                    include_in_memory: true,
                    is_done: false,
                    success: None,
                    attachments: Vec::new(),
                    images: Vec::new(),
                    metadata: None,
                }],
                state: blank_state(),
                metadata: None,
            },
            AgentHistoryItem {
                model_output: None,
                result: vec![ActionResult::extracted("Clicked element 1")],
                state: blank_state(),
                metadata: None,
            },
        ],
        ..AgentHistory::default()
    };

    let rendered = render_previous_results(&history, None);

    assert!(!rendered.contains("very large extracted payload"));
    assert!(rendered.contains("Large extraction saved to file"));
    assert!(rendered.contains("Clicked element 1"));
}

#[test]
fn latest_one_time_extraction_moves_to_read_state_section() {
    let history = AgentHistory {
        items: vec![AgentHistoryItem {
            model_output: None,
            result: vec![ActionResult {
                extracted_content: Some("fresh extracted payload".to_owned()),
                error: None,
                judgement: None,
                long_term_memory: Some("Extraction saved to file".to_owned()),
                include_extracted_content_only_once: true,
                include_in_memory: true,
                is_done: false,
                success: None,
                attachments: Vec::new(),
                images: Vec::new(),
                metadata: None,
            }],
            state: blank_state(),
            metadata: None,
        }],
        ..AgentHistory::default()
    };

    let rendered = render_previous_results(&history, None);
    let read_state = render_read_state_description(&history).expect("read state");

    assert!(!rendered.contains("fresh extracted payload"));
    assert!(rendered.contains("Result\nExtraction saved to file"));
    assert_eq!(
        read_state,
        "<read_state_0>\nfresh extracted payload\n</read_state_0>"
    );

    let request = build_step_request(
        "inspect read state",
        &blank_state(),
        &history,
        &AgentSettings::default(),
    )
    .expect("step request");
    let user_message = request
        .messages
        .iter()
        .find(|message| message.role == MessageRole::User)
        .expect("user message");
    let user_text = match &user_message.content[0] {
        ContentPart::Text { text } => text,
        other => panic!("unexpected first content part: {other:?}"),
    };
    assert!(user_text.contains(
        "<read_state>\n<read_state_0>\nfresh extracted payload\n</read_state_0>\n</read_state>"
    ));
    assert!(
        user_text.contains(
            "<agent_history>\n<step>\nResult\nExtraction saved to file\n</agent_history>"
        )
    );
}

#[test]
fn latest_read_state_blocks_are_numbered_like_upstream() {
    let history = AgentHistory {
        items: vec![AgentHistoryItem {
            model_output: None,
            result: vec![
                ActionResult {
                    extracted_content: Some("first payload".to_owned()),
                    error: None,
                    judgement: None,
                    long_term_memory: Some("first summary".to_owned()),
                    include_extracted_content_only_once: true,
                    include_in_memory: true,
                    is_done: false,
                    success: None,
                    attachments: Vec::new(),
                    images: Vec::new(),
                    metadata: None,
                },
                ActionResult {
                    extracted_content: Some("second payload".to_owned()),
                    error: None,
                    judgement: None,
                    long_term_memory: Some("second summary".to_owned()),
                    include_extracted_content_only_once: true,
                    include_in_memory: true,
                    is_done: false,
                    success: None,
                    attachments: Vec::new(),
                    images: Vec::new(),
                    metadata: None,
                },
            ],
            state: blank_state(),
            metadata: None,
        }],
        ..AgentHistory::default()
    };

    let rendered = render_previous_results(&history, None);
    let read_state = render_read_state_description(&history).expect("read state");

    assert!(!rendered.contains("first payload"));
    assert!(!rendered.contains("second payload"));
    assert!(rendered.contains("first summary"));
    assert!(rendered.contains("second summary"));
    assert!(read_state.contains("<read_state_0>\nfirst payload\n</read_state_0>"));
    assert!(read_state.contains("<read_state_1>\nsecond payload\n</read_state_1>"));
}

#[test]
fn previous_results_include_all_history_when_limit_is_absent_like_upstream() {
    let history = AgentHistory {
        items: ["first", "second", "third", "fourth", "fifth", "sixth"]
            .into_iter()
            .map(|text| AgentHistoryItem {
                model_output: None,
                result: vec![ActionResult::extracted(text)],
                state: blank_state(),
                metadata: None,
            })
            .collect(),
        ..AgentHistory::default()
    };

    let rendered = render_previous_results(&history, None);

    assert!(rendered.contains("Result\nfirst"));
    assert!(rendered.contains("Result\nsixth"));
    assert!(!rendered.contains("previous steps omitted"));
}

#[test]
fn previous_results_prefix_compacted_memory_like_upstream() {
    let history = AgentHistory {
        compacted_memory: Some(
            "Earlier summary; checkout was started but not confirmed.".to_owned(),
        ),
        compaction_count: 1,
        last_compaction_step: Some(25),
        items: vec![AgentHistoryItem {
            model_output: None,
            result: vec![ActionResult::extracted("Latest verified step")],
            state: blank_state(),
            metadata: None,
        }],
        ..AgentHistory::default()
    };

    let rendered = render_previous_results(&history, Some(1));

    assert!(rendered.starts_with("<compacted_memory>"));
    assert!(rendered.contains("Treat as unverified context"));
    assert!(rendered.contains("Earlier summary"));
    assert!(rendered.contains("</compacted_memory>\n<step>\nResult\nLatest verified step"));
}

#[test]
fn previous_results_include_model_brain_fields_like_upstream() {
    let history = AgentHistory {
        items: vec![AgentHistoryItem {
            model_output: Some(AgentOutput {
                current_state: AgentCurrentState::default(),
                thinking: None,
                evaluation_previous_goal: Some("Previous goal succeeded".to_owned()),
                memory: Some("Remembered page context".to_owned()),
                next_goal: Some("Click next result".to_owned()),
                current_plan_item: None,
                plan_update: None,
                action: vec![BrowserAction::Wait(browser_use_tools::WaitAction {
                    seconds: 1,
                })],
            }),
            result: vec![ActionResult::extracted("Waited for 1 seconds")],
            state: blank_state(),
            metadata: None,
        }],
        ..AgentHistory::default()
    };

    let rendered = render_previous_results(&history, None);

    assert!(rendered.starts_with("<step>\nPrevious goal succeeded"));
    assert!(rendered.contains("Remembered page context"));
    assert!(rendered.contains("Click next result"));
    assert!(rendered.contains("Result\nWaited for 1 seconds"));
}

#[test]
fn previous_results_limit_preserves_initial_and_recent_tail_like_upstream() {
    let history = AgentHistory {
        items: ["first", "second", "third"]
            .into_iter()
            .map(|text| AgentHistoryItem {
                model_output: None,
                result: vec![ActionResult::extracted(text)],
                state: blank_state(),
                metadata: None,
            })
            .collect(),
        ..AgentHistory::default()
    };

    let rendered = render_previous_results(&history, Some(2));

    assert!(rendered.contains("Result\nfirst"));
    assert!(rendered.contains("<sys>[... 1 previous steps omitted...]</sys>"));
    assert!(!rendered.contains("Result\nsecond"));
    assert!(rendered.contains("Result\nthird"));
}

#[test]
fn previous_results_truncate_long_errors_like_upstream() {
    let error = format!("{}{}", "a".repeat(120), "b".repeat(120));
    let history = AgentHistory {
        items: vec![AgentHistoryItem {
            model_output: None,
            result: vec![ActionResult::error(error.clone())],
            state: blank_state(),
            metadata: None,
        }],
        ..AgentHistory::default()
    };

    let rendered = render_previous_results(&history, None);
    let expected = format!("{}......{}", "a".repeat(100), "b".repeat(100));

    assert!(rendered.contains(&expected));
    assert!(!rendered.contains(&error));
}

#[test]
fn previous_results_truncate_large_prompt_context_like_upstream() {
    let huge_result = "x".repeat(MAX_PROMPT_CONTENT_CHARS + 100);
    let history = AgentHistory {
        items: vec![AgentHistoryItem {
            model_output: None,
            result: vec![ActionResult::extracted(huge_result)],
            state: blank_state(),
            metadata: None,
        }],
        ..AgentHistory::default()
    };

    let rendered = render_previous_results(&history, None);

    assert_eq!(
        rendered.chars().count(),
        MAX_PROMPT_CONTENT_CHARS + "\n... [Content truncated at 60k characters]".len()
    );
    assert!(rendered.ends_with("\n... [Content truncated at 60k characters]"));
}

#[test]
fn search_url_matches_browser_use_engines() {
    assert_eq!(
        search_url(&SearchEngine::Google, "browser use rust"),
        "https://www.google.com/search?q=browser+use+rust&udm=14"
    );
    assert_eq!(
        search_url(&SearchEngine::DuckDuckGo, "browser use rust"),
        "https://duckduckgo.com/?q=browser+use+rust"
    );
}
