//! CLI, daemon, and MCP adapter tests.
//!
//! The CLI has a large integration surface: argument translation, provider
//! configuration, persistent session bookkeeping, JSON-RPC framing, HTTP/TCP
//! daemon handling, and MCP tool execution. These tests pin that behavior
//! without requiring a real browser unless a test explicitly opts into it.

use super::*;

#[test]
fn parses_agent_settings_flags() {
    let cli = Cli::try_parse_from([
        "browser-use-rs",
        "agent",
        "https://example.com",
        "test task",
        "--provider",
        "openai",
        "--max-steps",
        "3",
        "--no-vision",
        "--vision-detail-level",
        "high",
        "--max-failures",
        "2",
        "--generate-gif",
        "/tmp/trace.gif",
        "--max-actions-per-step",
        "1",
        "--llm-timeout-seconds",
        "11",
        "--step-timeout-seconds",
        "22",
        "--action-timeout-seconds",
        "33.5",
        "--wait-between-actions-seconds",
        "0.25",
        "--no-directly-open-url",
        "--no-final-response-after-failure",
        "--no-display-files-in-done-text",
        "--no-loop-detection",
        "--loop-detection-window",
        "4",
        "--no-thinking",
        "--flash-mode",
        "--no-judge",
        "--ground-truth",
        "Must include a receipt.",
        "--extraction-schema",
        r#"{"type":"object","properties":{"company":{"type":"string"}}}"#,
        "--calculate-cost",
        "--include-tool-call-examples",
        "--save-conversation-path",
        "/tmp/browser-use-conversations",
        "--save-conversation-path-encoding",
        "utf-8",
        "--file-system-path",
        "/tmp/browser-use-agent-files",
        "--no-planning",
        "--planning-replan-on-stall",
        "5",
        "--planning-exploration-limit",
        "6",
        "--max-history-items",
        "7",
        "--message-compaction-compact-every-n-steps",
        "3",
        "--message-compaction-trigger-token-count",
        "1000",
        "--message-compaction-chars-per-token",
        "3.5",
        "--message-compaction-keep-last-items",
        "2",
        "--message-compaction-summary-max-chars",
        "1200",
        "--message-compaction-include-read-state",
        "--max-clickable-elements-length",
        "8000",
        "--include-recent-events",
        "--include-attribute",
        "data-testid",
        "--include-attribute",
        "aria-label",
        "--available-file-path",
        "/tmp/report.pdf",
        "--available-file-path",
        "/tmp/chart.png",
        "--exclude-action",
        "search",
        "--exclude-action",
        "scroll",
        "--sensitive-data",
        "username=evalops@example.test",
        "--sensitive-data",
        "api_key=sk=value",
        "--sensitive-data-domain",
        "*.example.test=password=super-secret",
        "--override-system-message",
        "Custom system prompt.",
        "--extend-system-message",
        "Add selector guidance.",
        "--allowed-domain",
        "*.example.test",
        "--prohibited-domain",
        "tracker.example.test",
        "--block-ip-addresses",
    ])
    .expect("agent settings flags should parse");

    match cli.command.expect("agent command") {
        Command::Agent {
            provider,
            max_steps,
            no_vision,
            vision_detail_level,
            max_failures,
            generate_gif,
            max_actions_per_step,
            llm_timeout_seconds,
            step_timeout_seconds,
            action_timeout_seconds,
            wait_between_actions_seconds,
            no_directly_open_url,
            no_final_response_after_failure,
            no_display_files_in_done_text,
            no_loop_detection,
            loop_detection_window,
            no_thinking,
            flash_mode,
            no_judge,
            ground_truth,
            extraction_schema,
            calculate_cost,
            include_tool_call_examples,
            save_conversation_path,
            save_conversation_path_encoding,
            file_system_path,
            no_planning,
            planning_replan_on_stall,
            planning_exploration_limit,
            max_history_items,
            no_message_compaction,
            message_compaction_compact_every_n_steps,
            message_compaction_trigger_char_count,
            message_compaction_trigger_token_count,
            message_compaction_chars_per_token,
            message_compaction_keep_last_items,
            message_compaction_summary_max_chars,
            message_compaction_include_read_state,
            max_clickable_elements_length,
            include_recent_events,
            include_attributes,
            available_file_paths,
            excluded_actions,
            sensitive_data,
            sensitive_data_domains,
            override_system_message,
            extend_system_message,
            allowed_domains,
            prohibited_domains,
            block_ip_addresses,
            ..
        } => {
            assert_eq!(provider, LlmProvider::OpenAiCompatible);
            assert_eq!(max_steps, 3);
            assert!(no_vision);
            assert_eq!(vision_detail_level, Some(CliVisionDetailLevel::High));
            assert_eq!(max_failures, Some(2));
            assert_eq!(generate_gif.as_deref(), Some("/tmp/trace.gif"));
            assert_eq!(max_actions_per_step, Some(1));
            assert_eq!(llm_timeout_seconds, Some(11));
            assert_eq!(step_timeout_seconds, Some(22));
            assert_eq!(action_timeout_seconds, Some(33.5));
            assert_eq!(wait_between_actions_seconds, Some(0.25));
            assert!(no_directly_open_url);
            assert!(no_final_response_after_failure);
            assert!(no_display_files_in_done_text);
            assert!(no_loop_detection);
            assert_eq!(loop_detection_window, Some(4));
            assert!(no_thinking);
            assert!(flash_mode);
            assert!(no_judge);
            assert_eq!(ground_truth.as_deref(), Some("Must include a receipt."));
            assert_eq!(
                extraction_schema
                    .as_ref()
                    .and_then(|schema| schema["properties"]["company"]["type"].as_str()),
                Some("string")
            );
            assert!(calculate_cost);
            assert!(include_tool_call_examples);
            assert_eq!(
                save_conversation_path.as_deref(),
                Some("/tmp/browser-use-conversations")
            );
            assert_eq!(save_conversation_path_encoding.as_deref(), Some("utf-8"));
            assert_eq!(
                file_system_path.as_deref(),
                Some("/tmp/browser-use-agent-files")
            );
            assert!(no_planning);
            assert_eq!(planning_replan_on_stall, Some(5));
            assert_eq!(planning_exploration_limit, Some(6));
            assert_eq!(max_history_items, Some(7));
            assert!(!no_message_compaction);
            assert_eq!(message_compaction_compact_every_n_steps, Some(3));
            assert_eq!(message_compaction_trigger_char_count, None);
            assert_eq!(message_compaction_trigger_token_count, Some(1000));
            assert_eq!(message_compaction_chars_per_token, Some(3.5));
            assert_eq!(message_compaction_keep_last_items, Some(2));
            assert_eq!(message_compaction_summary_max_chars, Some(1200));
            assert!(message_compaction_include_read_state);
            assert_eq!(max_clickable_elements_length, Some(8000));
            assert!(include_recent_events);
            assert_eq!(include_attributes, ["data-testid", "aria-label"]);
            assert_eq!(available_file_paths, ["/tmp/report.pdf", "/tmp/chart.png"]);
            assert_eq!(excluded_actions, ["search", "scroll"]);
            assert_eq!(
                sensitive_data,
                [
                    SensitiveDataEntry {
                        placeholder: "username".to_owned(),
                        value: "evalops@example.test".to_owned()
                    },
                    SensitiveDataEntry {
                        placeholder: "api_key".to_owned(),
                        value: "sk=value".to_owned()
                    }
                ]
            );
            assert_eq!(
                sensitive_data_domains,
                [DomainSensitiveDataEntry {
                    domain_pattern: "*.example.test".to_owned(),
                    placeholder: "password".to_owned(),
                    value: "super-secret".to_owned()
                }]
            );
            assert_eq!(
                override_system_message.as_deref(),
                Some("Custom system prompt.")
            );
            assert_eq!(
                extend_system_message.as_deref(),
                Some("Add selector guidance.")
            );
            assert_eq!(allowed_domains, ["*.example.test"]);
            assert_eq!(prohibited_domains, ["tracker.example.test"]);
            assert!(block_ip_addresses);
        }
        _ => panic!("expected agent command"),
    }
}

#[test]
fn parses_replay_command() {
    let cli = Cli::try_parse_from([
        "browser-use-rs",
        "replay",
        "https://example.com",
        "history.json",
    ])
    .expect("replay command should parse");

    match cli.command.expect("replay command") {
        Command::Replay { url, history } => {
            assert_eq!(url, "https://example.com");
            assert_eq!(history, PathBuf::from("history.json"));
        }
        _ => panic!("expected replay command"),
    }
}

#[test]
fn parses_replay_run_schema_contract() {
    let cli = Cli::try_parse_from(["browser-use-rs", "schema", "replay-run"])
        .expect("schema command should parse");

    match cli.command.expect("schema command") {
        Command::Schema { contract } => assert!(matches!(contract, SchemaContract::ReplayRun)),
        _ => panic!("expected schema command"),
    }
}

#[test]
fn parses_upstream_openai_wire_provider_aliases() {
    for (provider_name, expected_provider) in [
        ("deepseek", LlmProvider::DeepSeek),
        ("deep-seek", LlmProvider::DeepSeek),
        ("groq", LlmProvider::Groq),
        ("cerebras", LlmProvider::Cerebras),
        ("mistral", LlmProvider::Mistral),
        ("openrouter", LlmProvider::OpenRouter),
        ("open-router", LlmProvider::OpenRouter),
        ("vercel", LlmProvider::Vercel),
        ("ai-gateway", LlmProvider::Vercel),
    ] {
        let cli = Cli::try_parse_from([
            "browser-use-rs",
            "agent",
            "https://example.com",
            "test task",
            "--provider",
            provider_name,
        ])
        .expect("agent provider should parse");

        match cli.command.expect("agent command") {
            Command::Agent { provider, .. } => assert_eq!(provider, expected_provider),
            _ => panic!("expected agent command"),
        }
    }
}

#[test]
fn configures_openai_wire_provider_aliases_without_env() {
    for (provider, expected_name, expected_model) in [
        (LlmProvider::DeepSeek, "deepseek", "deepseek-chat"),
        (LlmProvider::Cerebras, "cerebras", "llama3.1-8b"),
        (LlmProvider::Mistral, "mistral", "mistral-medium-latest"),
    ] {
        let llm = configured_chat_model(provider, Some("test-key".to_owned()), None, None, None)
            .expect("provider should use default model");

        assert_eq!(llm.provider(), expected_name);
        assert_eq!(llm.model(), expected_model);
    }

    let openrouter = configured_chat_model(
        LlmProvider::OpenRouter,
        Some("test-key".to_owned()),
        Some("openai/gpt-4o-mini".to_owned()),
        None,
        None,
    )
    .expect("openrouter with explicit model");
    assert_eq!(openrouter.provider(), "openrouter");
    assert_eq!(openrouter.model(), "openai/gpt-4o-mini");

    assert_eq!(
        openai_wire_provider_config(LlmProvider::DeepSeek).structured_output_mode,
        OpenAiStructuredOutputMode::ToolCall
    );
    assert_eq!(
        openai_wire_provider_config(LlmProvider::Cerebras).structured_output_mode,
        OpenAiStructuredOutputMode::PromptOnly
    );
    assert_eq!(
        openai_wire_provider_config(LlmProvider::Mistral).schema_transform,
        OpenAiSchemaTransform::MistralCompatible
    );
    assert!(
        openai_wire_provider_config(LlmProvider::DeepSeek)
            .default_headers
            .is_empty()
    );
}

#[test]
fn maps_provider_specific_structured_output_fallback_modes() {
    let groq = openai_wire_provider_config(LlmProvider::Groq);
    assert_eq!(
        default_structured_output_mode(groq, "moonshotai/kimi-k2-instruct", None),
        OpenAiStructuredOutputMode::ToolCall
    );
    assert_eq!(
        default_structured_output_mode(groq, "meta-llama/llama-4-scout-17b-16e-instruct", None),
        OpenAiStructuredOutputMode::JsonSchema
    );

    let vercel = openai_wire_provider_config(LlmProvider::Vercel);
    assert_eq!(
        default_structured_output_mode(vercel, "google/gemini-2.5-flash", None),
        OpenAiStructuredOutputMode::PromptOnly
    );
    assert_eq!(
        default_structured_output_mode(vercel, "anthropic/claude-sonnet-4.5", None),
        OpenAiStructuredOutputMode::PromptOnly
    );
    assert_eq!(
        default_structured_output_mode(vercel, "openai/gpt-oss-120b", None),
        OpenAiStructuredOutputMode::PromptOnly
    );
    assert_eq!(
        default_structured_output_mode(vercel, "openai/gpt-4o-mini", None),
        OpenAiStructuredOutputMode::JsonSchema
    );
    assert_eq!(
        default_structured_output_mode(
            vercel,
            "google/gemini-2.5-flash",
            Some(OpenAiStructuredOutputMode::ToolCall)
        ),
        OpenAiStructuredOutputMode::ToolCall
    );
}

#[test]
fn openrouter_default_headers_read_app_attribution_env_names() {
    let config = openai_wire_provider_config(LlmProvider::OpenRouter);
    let headers = openai_wire_default_headers(config, |names| {
        if names == ["OPENROUTER_HTTP_REFERER", "OPENROUTER_APP_URL"] {
            Some("https://evalops.dev".to_owned())
        } else if names == ["OPENROUTER_X_TITLE", "OPENROUTER_APP_TITLE"] {
            Some("EvalOps browser-use-rs".to_owned())
        } else {
            None
        }
    });

    assert_eq!(
        headers,
        [
            ("HTTP-Referer", "https://evalops.dev".to_owned()),
            ("X-Title", "EvalOps browser-use-rs".to_owned()),
            ("X-OpenRouter-Title", "EvalOps browser-use-rs".to_owned())
        ]
    );
}

#[test]
fn parses_structured_output_mode_override() {
    let cli = Cli::try_parse_from([
        "browser-use-rs",
        "agent",
        "https://example.com",
        "test task",
        "--provider",
        "openrouter",
        "--structured-output-mode",
        "tool-call",
    ])
    .expect("structured output mode should parse");

    match cli.command.expect("agent command") {
        Command::Agent {
            structured_output_mode,
            ..
        } => assert_eq!(structured_output_mode, Some(StructuredOutputMode::ToolCall)),
        _ => panic!("expected agent command"),
    }
}

#[test]
fn maps_mcp_structured_output_mode_override() {
    let mode = StructuredOutputMode::from_mcp(browser_use_mcp::AgentStructuredOutputMode::ToolCall)
        .into_openai_mode();

    assert_eq!(mode, OpenAiStructuredOutputMode::ToolCall);
}

#[test]
fn rejects_malformed_sensitive_data_flags() {
    assert!(
        Cli::try_parse_from([
            "browser-use-rs",
            "agent",
            "https://example.com",
            "test task",
            "--sensitive-data",
            "username",
        ])
        .is_err()
    );
    assert!(
        Cli::try_parse_from([
            "browser-use-rs",
            "agent",
            "https://example.com",
            "test task",
            "--sensitive-data-domain",
            "*.example.test=password",
        ])
        .is_err()
    );
}

#[test]
fn parses_agent_auto_vision_mode_flag() {
    let cli = Cli::try_parse_from([
        "browser-use-rs",
        "agent",
        "https://example.com",
        "test task",
        "--vision-mode",
        "auto",
    ])
    .expect("auto vision mode should parse");

    match cli.command.expect("agent command") {
        Command::Agent { vision_mode, .. } => {
            assert_eq!(vision_mode, Some(CliVisionMode::Auto));
        }
        _ => panic!("expected agent command"),
    }

    assert!(
        Cli::try_parse_from([
            "browser-use-rs",
            "agent",
            "https://example.com",
            "test task",
            "--no-vision",
            "--vision-mode",
            "auto",
        ])
        .is_err()
    );
}

#[test]
fn parses_generate_gif_optional_path_flag() {
    let enabled = Cli::try_parse_from([
        "browser-use-rs",
        "agent",
        "https://example.com",
        "test task",
        "--generate-gif",
    ])
    .expect("bare generate-gif should parse");

    match enabled.command.expect("agent command") {
        Command::Agent { generate_gif, .. } => {
            assert_eq!(generate_gif.as_deref(), Some("true"));
        }
        _ => panic!("expected agent command"),
    }

    let with_path = Cli::try_parse_from([
        "browser-use-rs",
        "agent",
        "https://example.com",
        "test task",
        "--generate-gif",
        "/tmp/trace.gif",
    ])
    .expect("path generate-gif should parse");

    match with_path.command.expect("agent command") {
        Command::Agent { generate_gif, .. } => {
            assert_eq!(generate_gif.as_deref(), Some("/tmp/trace.gif"));
        }
        _ => panic!("expected agent command"),
    }
}

#[test]
fn builds_agent_settings_from_cli_flags() {
    let settings = cli_agent_settings(CliAgentSettingsArgs {
        no_vision: true,
        vision_mode: None,
        vision_detail_level: Some(CliVisionDetailLevel::High),
        max_failures: Some(2),
        generate_gif: Some("/tmp/trace.gif".to_owned()),
        max_actions_per_step: Some(1),
        llm_timeout_seconds: Some(11),
        step_timeout_seconds: Some(22),
        action_timeout_seconds: Some(33.5),
        wait_between_actions_seconds: Some(0.25),
        no_directly_open_url: true,
        no_final_response_after_failure: true,
        no_display_files_in_done_text: true,
        no_loop_detection: true,
        loop_detection_window: Some(4),
        no_thinking: true,
        flash_mode: true,
        no_judge: true,
        ground_truth: Some("Must include a receipt.".to_owned()),
        extraction_schema: Some(serde_json::json!({
            "type": "object",
            "properties": {
                "company": { "type": "string" }
            }
        })),
        calculate_cost: true,
        include_tool_call_examples: true,
        save_conversation_path: Some("/tmp/browser-use-conversations".to_owned()),
        save_conversation_path_encoding: Some("utf-8".to_owned()),
        file_system_path: Some("/tmp/browser-use-agent-files".to_owned()),
        no_planning: true,
        planning_replan_on_stall: Some(5),
        planning_exploration_limit: Some(6),
        max_history_items: Some(7),
        no_message_compaction: false,
        message_compaction_compact_every_n_steps: Some(3),
        message_compaction_trigger_char_count: None,
        message_compaction_trigger_token_count: Some(1000),
        message_compaction_chars_per_token: Some(3.5),
        message_compaction_keep_last_items: Some(2),
        message_compaction_summary_max_chars: Some(1200),
        message_compaction_include_read_state: true,
        max_clickable_elements_length: Some(8000),
        include_recent_events: true,
        include_attributes: vec!["data-testid".to_owned(), "aria-label".to_owned()],
        available_file_paths: vec!["/tmp/report.pdf".to_owned(), "/tmp/chart.png".to_owned()],
        excluded_actions: vec!["search".to_owned(), "scroll".to_owned()],
        sensitive_data: vec![SensitiveDataEntry {
            placeholder: "username".to_owned(),
            value: "evalops@example.test".to_owned(),
        }],
        sensitive_data_domains: vec![
            DomainSensitiveDataEntry {
                domain_pattern: "*.example.test".to_owned(),
                placeholder: "password".to_owned(),
                value: "super-secret".to_owned(),
            },
            DomainSensitiveDataEntry {
                domain_pattern: "*.example.test".to_owned(),
                placeholder: "otp".to_owned(),
                value: "123456".to_owned(),
            },
        ],
        override_system_message: Some("Custom system prompt.".to_owned()),
        extend_system_message: Some("Add selector guidance.".to_owned()),
    });

    assert_eq!(settings.use_vision, VisionMode::Never);
    assert_eq!(settings.vision_detail_level, ImageDetailLevel::High);
    assert_eq!(settings.max_failures, 2);
    assert_eq!(
        settings.generate_gif,
        GenerateGif::Path("/tmp/trace.gif".to_owned())
    );
    assert_eq!(settings.max_actions_per_step, 1);
    assert_eq!(settings.llm_timeout_seconds, 11);
    assert_eq!(settings.step_timeout_seconds, 22);
    assert_eq!(settings.action_timeout_seconds, 33.5);
    assert_eq!(settings.wait_between_actions_seconds, 0.25);
    assert!(!settings.directly_open_url);
    assert!(!settings.final_response_after_failure);
    assert!(!settings.display_files_in_done_text);
    assert!(!settings.loop_detection_enabled);
    assert_eq!(settings.loop_detection_window, 4);
    assert!(!settings.use_thinking);
    assert!(settings.flash_mode);
    assert!(!settings.use_judge);
    assert_eq!(
        settings.ground_truth.as_deref(),
        Some("Must include a receipt.")
    );
    assert_eq!(
        settings
            .extraction_schema
            .as_ref()
            .and_then(|schema| schema["properties"]["company"]["type"].as_str()),
        Some("string")
    );
    assert!(settings.calculate_cost);
    assert!(settings.include_tool_call_examples);
    assert_eq!(
        settings.save_conversation_path.as_deref(),
        Some("/tmp/browser-use-conversations")
    );
    assert_eq!(
        settings.save_conversation_path_encoding.as_deref(),
        Some("utf-8")
    );
    assert_eq!(
        settings.file_system_path.as_deref(),
        Some("/tmp/browser-use-agent-files")
    );
    assert!(!settings.enable_planning);
    assert_eq!(settings.planning_replan_on_stall, 5);
    assert_eq!(settings.planning_exploration_limit, 6);
    assert_eq!(settings.max_history_items, Some(7));
    let MessageCompaction::Settings(message_compaction) = &settings.message_compaction else {
        panic!("expected custom message compaction settings");
    };
    assert_eq!(message_compaction.compact_every_n_steps, 3);
    assert_eq!(message_compaction.trigger_token_count, Some(1000));
    assert_eq!(message_compaction.trigger_char_count, Some(3500));
    assert_eq!(message_compaction.chars_per_token, 3.5);
    assert_eq!(message_compaction.keep_last_items, 2);
    assert_eq!(message_compaction.summary_max_chars, 1200);
    assert!(message_compaction.include_read_state);
    assert_eq!(settings.max_clickable_elements_length, 8000);
    assert!(settings.include_recent_events);
    assert_eq!(settings.include_attributes, ["data-testid", "aria-label"]);
    assert_eq!(
        settings.available_file_paths,
        ["/tmp/report.pdf", "/tmp/chart.png"]
    );
    assert_eq!(settings.excluded_actions, ["search", "scroll"]);
    assert_eq!(
        settings.sensitive_data.get("username"),
        Some(&SensitiveDataValue::Value(
            "evalops@example.test".to_owned()
        ))
    );
    assert_eq!(
        settings.sensitive_data.get("*.example.test"),
        Some(&SensitiveDataValue::Domain(BTreeMap::from([
            ("otp".to_owned(), "123456".to_owned()),
            ("password".to_owned(), "super-secret".to_owned())
        ])))
    );
    assert_eq!(
        settings.override_system_message.as_deref(),
        Some("Custom system prompt.")
    );
    assert_eq!(
        settings.extend_system_message.as_deref(),
        Some("Add selector guidance.")
    );
}

#[test]
fn builds_agent_settings_with_auto_vision_mode() {
    let settings = cli_agent_settings(CliAgentSettingsArgs {
        vision_mode: Some(CliVisionMode::Auto),
        ..CliAgentSettingsArgs::default()
    });

    assert_eq!(settings.use_vision, VisionMode::Auto);
}

#[test]
fn parses_http_daemon_flags() {
    let cli = Cli::try_parse_from([
        "browser-use-rs",
        "daemon",
        "--addr",
        "127.0.0.1:0",
        "--transport",
        "http",
        "--auth-token",
        "secret",
        "--pid-file",
        "/tmp/browser-use-rs.pid",
        "--ready-file",
        "/tmp/browser-use-rs.ready.json",
    ])
    .expect("daemon flags should parse");

    match cli.command.expect("daemon command") {
        Command::Daemon {
            addr,
            transport,
            auth_token,
            pid_file,
            ready_file,
        } => {
            assert_eq!(addr, "127.0.0.1:0");
            assert_eq!(transport, DaemonTransport::Http);
            assert_eq!(auth_token.as_deref(), Some("secret"));
            assert_eq!(pid_file, Some(PathBuf::from("/tmp/browser-use-rs.pid")));
            assert_eq!(
                ready_file,
                Some(PathBuf::from("/tmp/browser-use-rs.ready.json"))
            );
        }
        _ => panic!("expected daemon command"),
    }
}

#[test]
fn daemon_transport_boundary_excludes_unix_sockets() {
    let error = Cli::try_parse_from(["browser-use-rs", "daemon", "--transport", "unix"])
        .expect_err("unix socket transport should not parse");
    let message = error.to_string();
    assert!(message.contains("unix"), "{message}");
    assert!(message.contains("tcp"), "{message}");
    assert!(message.contains("http"), "{message}");
}

#[test]
fn mcp_session_plan_persists_implicit_session_ids() {
    assert_eq!(
        mcp_session_plan(true, false, false, "existing").expect("in memory"),
        McpSessionPlan::ReuseInMemory
    );
    assert_eq!(
        mcp_session_plan(false, true, false, "recorded").expect("record"),
        McpSessionPlan::ReconnectPersistentRecord
    );
    assert_eq!(
        mcp_session_plan(false, false, true, "implicit").expect("create"),
        McpSessionPlan::CreatePersistentRecord
    );

    let error =
        mcp_session_plan(false, false, false, "missing-url").expect_err("missing url should fail");
    assert_eq!(
        error.to_string(),
        "url is required to create MCP session missing-url"
    );
}

#[test]
fn session_record_status_is_backward_compatible() {
    let record: StoredSession = serde_json::from_value(serde_json::json!({
        "id": "legacy",
        "endpoint": {
            "http_url": "http://127.0.0.1:9222",
            "websocket_url": "ws://127.0.0.1:9222/devtools/browser/legacy"
        },
        "user_data_dir": "/tmp/browser-use-rs-legacy",
        "process_id": 4294967295_u32
    }))
    .expect("legacy record");

    assert_eq!(record.status, None);
    assert_eq!(
        session_status_with_checker(&record, |_| false),
        browser_use_mcp::SessionStatus::Stale
    );
    assert_eq!(
        session_status_with_checker(&record, |_| true),
        browser_use_mcp::SessionStatus::Running
    );
    assert_eq!(
        annotate_session_status(StoredSession {
            process_id: None,
            ..record
        })
        .status,
        Some(browser_use_mcp::SessionStatus::Unknown)
    );
}

#[test]
fn session_cleanup_decision_is_conservative() {
    let record: StoredSession = serde_json::from_value(serde_json::json!({
        "id": "cleanup-target",
        "endpoint": {
            "http_url": "http://127.0.0.1:9222",
            "websocket_url": "ws://127.0.0.1:9222/devtools/browser/cleanup"
        },
        "user_data_dir": "/tmp/browser-use-rs-cleanup",
        "process_id": 1234_u32
    }))
    .expect("cleanup record");

    assert_eq!(
        session_cleanup_decision(&record, false, |_| true),
        SessionCleanupDecision::SkipRunning
    );
    assert_eq!(
        session_cleanup_decision(&record, true, |_| true),
        SessionCleanupDecision::StopRunning
    );
    assert_eq!(
        session_cleanup_decision(&record, false, |_| false),
        SessionCleanupDecision::RemoveRecord
    );

    let unknown = StoredSession {
        process_id: None,
        ..record
    };
    assert_eq!(
        session_cleanup_decision(&unknown, false, |_| false),
        SessionCleanupDecision::SkipUnknown
    );
    assert_eq!(
        session_cleanup_decision(&unknown, true, |_| false),
        SessionCleanupDecision::RemoveRecord
    );
}

#[test]
fn parses_session_cleanup_flags() {
    let cli = Cli::try_parse_from([
        "browser-use-rs",
        "session",
        "cleanup",
        "stale-one",
        "--force",
    ])
    .expect("cleanup flags should parse");

    match cli.command.expect("command") {
        Command::Session {
            command: SessionCommand::Cleanup { id, force },
        } => {
            assert_eq!(id.as_deref(), Some("stale-one"));
            assert!(force);
        }
        _ => panic!("expected session cleanup command"),
    }
}

#[test]
fn parses_session_replay_command() {
    let cli = Cli::try_parse_from([
        "browser-use-rs",
        "session",
        "replay",
        "existing",
        "history.json",
    ])
    .expect("session replay command should parse");

    match cli.command.expect("command") {
        Command::Session {
            command: SessionCommand::Replay { id, history },
        } => {
            assert_eq!(id, "existing");
            assert_eq!(history, PathBuf::from("history.json"));
        }
        _ => panic!("expected session replay command"),
    }
}

#[test]
fn daemon_lifecycle_files_write_supervisor_artifacts() {
    let dir = std::env::temp_dir().join(format!(
        "browser-use-rs-daemon-lifecycle-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("temp dir");
    let pid_file = dir.join("daemon.pid");
    let ready_file = dir.join("daemon.ready.json");

    {
        let files = DaemonLifecycleFiles::write(
            DaemonLifecycleOptions {
                pid_file: Some(pid_file.clone()),
                ready_file: Some(ready_file.clone()),
            },
            DaemonTransport::Http,
            "127.0.0.1:8765",
        )
        .expect("write lifecycle files");
        let pid = std::fs::read_to_string(&pid_file).expect("pid file");
        assert_eq!(pid.trim(), std::process::id().to_string());
        let ready: Value = serde_json::from_slice(&std::fs::read(&ready_file).expect("ready file"))
            .expect("ready json");
        assert_eq!(ready["ready"], true);
        assert_eq!(ready["transport"], "http");
        assert_eq!(ready["addr"], "127.0.0.1:8765");
        assert_eq!(ready["pid"], std::process::id());
        drop(files);
    }

    assert!(!pid_file.exists());
    assert!(!ready_file.exists());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn authorizes_http_daemon_requests() {
    let request = http_request(
        "POST",
        "/rpc",
        [
            ("authorization", "Bearer secret"),
            ("x-browser-use-rs-token", "wrong"),
        ],
        b"{}",
    );
    assert!(http_request_authorized(&request, Some("secret")));
    assert!(http_request_authorized(&request, None));

    let request = http_request(
        "POST",
        "/rpc",
        [("x-browser-use-rs-token", "secret")],
        b"{}",
    );
    assert!(http_request_authorized(&request, Some("secret")));

    let request = http_request("POST", "/rpc", [("authorization", "Bearer nope")], b"{}");
    assert!(!http_request_authorized(&request, Some("secret")));
}

#[tokio::test]
async fn mcp_replay_tool_dispatches_to_schema_errors() {
    let mut runtime = McpRuntime::default();
    let response = handle_mcp_message(
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"browser_use_replay","arguments":{}}}"#,
            &mut runtime,
        )
        .await
        .expect("response");

    assert_eq!(response["jsonrpc"], "2.0");
    assert_eq!(response["id"], 1);
    assert_eq!(response["result"]["isError"], true);
    let text = response["result"]["content"][0]["text"]
        .as_str()
        .expect("text content");
    assert!(text.contains("history"));
}

#[tokio::test]
async fn http_daemon_healthz_does_not_require_auth() {
    let mut runtime = McpRuntime::default();
    let response = handle_http_request(
        http_request("GET", "/healthz", [], b""),
        &mut runtime,
        Some("secret"),
    )
    .await;

    assert_eq!(response.status, 200);
    let body: Value = serde_json::from_slice(&response.body).expect("json body");
    assert_eq!(body["ok"], true);
}

#[tokio::test]
async fn http_daemon_rejects_missing_token() {
    let mut runtime = McpRuntime::default();
    let response = handle_http_request(
        http_request(
            "POST",
            "/rpc",
            [],
            br#"{"jsonrpc":"2.0","id":1,"method":"ping"}"#,
        ),
        &mut runtime,
        Some("secret"),
    )
    .await;

    assert_eq!(response.status, 401);
    let body: Value = serde_json::from_slice(&response.body).expect("json body");
    assert_eq!(body["error"], "unauthorized");
}

#[tokio::test]
async fn http_daemon_dispatches_json_rpc_with_auth() {
    let mut runtime = McpRuntime::default();
    let response = handle_http_request(
        http_request(
            "POST",
            "/rpc",
            [("authorization", "Bearer secret")],
            br#"{"jsonrpc":"2.0","id":1,"method":"tools/list","params":{}}"#,
        ),
        &mut runtime,
        Some("secret"),
    )
    .await;

    assert_eq!(response.status, 200);
    let body: Value = serde_json::from_slice(&response.body).expect("json body");
    assert_eq!(body["jsonrpc"], "2.0");
    assert_eq!(body["id"], 1);
    assert!(body["result"]["tools"].as_array().is_some_and(|tools| {
        tools
            .iter()
            .any(|tool| tool["name"] == browser_use_mcp::STATE_TOOL_NAME)
    }));
}

#[tokio::test]
async fn reads_http_request_with_split_body() {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("local addr");

    let writer = tokio::spawn(async move {
        let mut stream = TcpStream::connect(addr).await.expect("connect");
        stream
            .write_all(b"POST /rpc HTTP/1.1\r\nHost: localhost\r\nContent-Length: 11\r\n\r\nhello")
            .await
            .expect("write headers");
        stream.write_all(b" world").await.expect("write body");
    });

    let (mut stream, _) = listener.accept().await.expect("accept");
    let request = read_http_request(&mut stream).await.expect("read request");
    writer.await.expect("writer task");

    assert_eq!(request.method, "POST");
    assert_eq!(request.path, "/rpc");
    assert_eq!(request.headers["host"], "localhost");
    assert_eq!(request.body, b"hello world");
}

fn http_request<const N: usize>(
    method: &str,
    path: &str,
    headers: [(&str, &str); N],
    body: &[u8],
) -> HttpRequest {
    HttpRequest {
        method: method.to_owned(),
        path: path.to_owned(),
        headers: headers
            .into_iter()
            .map(|(name, value)| (name.to_ascii_lowercase(), value.to_owned()))
            .collect(),
        body: body.to_vec(),
    }
}
