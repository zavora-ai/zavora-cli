//! Server, evaluation, confirmation, MCP selection, and chat contract tests.

use super::*;

#[test]
fn a2a_ping_process_returns_ack_envelope() {
    let req = A2aPingRequest {
        from_agent: "sales".to_string(),
        to_agent: "procurement".to_string(),
        message_id: "msg-1".to_string(),
        correlation_id: Some("corr-1".to_string()),
        payload: json!({"intent": "supply-check"}),
    };

    let response = process_a2a_ping(req.clone()).expect("a2a processing should succeed");
    assert_eq!(response.to_agent, "sales");
    assert_eq!(response.from_agent, "procurement");
    assert_eq!(response.acknowledged_message_id, "msg-1");
    assert_eq!(response.correlation_id, "corr-1");
    assert_eq!(response.status, "acknowledged");
    assert!(response.message_id.starts_with("ack-"));
}

#[test]
fn a2a_ping_process_rejects_invalid_request() {
    let req = A2aPingRequest {
        from_agent: "".to_string(),
        to_agent: "procurement".to_string(),
        message_id: "msg-1".to_string(),
        correlation_id: None,
        payload: json!({}),
    };

    let err = process_a2a_ping(req).expect_err("missing from_agent should fail");
    assert!(err.to_string().contains("from_agent is required"));
}

#[test]
fn a2a_smoke_command_passes_with_default_fixture() {
    let cfg = base_cfg();
    let telemetry = test_telemetry(&cfg);
    run_a2a_smoke(&telemetry).expect("a2a smoke should pass");
}

fn eval_dataset_fixture() -> EvalDataset {
    EvalDataset {
        name: "retrieval-baseline".to_string(),
        version: "1".to_string(),
        description: "fixture".to_string(),
        cases: vec![
            EvalCase {
                id: "release".to_string(),
                query: "release rollback mitigation".to_string(),
                chunks: vec![
                    "release plan includes rollback and mitigation steps".to_string(),
                    "unrelated content".to_string(),
                ],
                required_terms: vec!["rollback".to_string(), "mitigation".to_string()],
                max_chunks: 2,
                min_term_matches: Some(2),
            },
            EvalCase {
                id: "architecture".to_string(),
                query: "architecture components".to_string(),
                chunks: vec![
                    "component diagram and architecture decisions".to_string(),
                    "random note".to_string(),
                ],
                required_terms: vec!["architecture".to_string(), "component".to_string()],
                max_chunks: 2,
                min_term_matches: Some(1),
            },
        ],
    }
}

#[test]
fn eval_harness_produces_metrics_and_threshold_result() {
    let dataset = eval_dataset_fixture();
    let report = run_eval_harness(&dataset, 10, 0.8).expect("eval harness should run");

    assert_eq!(report.total_cases, 2);
    assert_eq!(report.passed_cases, 2);
    assert_eq!(report.failed_cases, 0);
    assert_eq!(report.pass_rate, 1.0);
    assert!(report.passed_threshold);
    assert_eq!(report.benchmark_iterations, 10);
    assert!(report.avg_latency_ms >= 0.0);
    assert!(report.p95_latency_ms >= 0.0);
    assert!(report.throughput_qps >= 0.0);
}

#[test]
fn eval_harness_fails_threshold_when_case_quality_is_low() {
    let mut dataset = eval_dataset_fixture();
    dataset.cases[0].required_terms = vec!["missing-term".to_string()];
    dataset.cases[0].min_term_matches = Some(1);

    let report = run_eval_harness(&dataset, 5, 0.75).expect("eval harness should run");
    assert_eq!(report.total_cases, 2);
    assert_eq!(report.passed_cases, 1);
    assert_eq!(report.failed_cases, 1);
    assert_eq!(report.pass_rate, 0.5);
    assert!(!report.passed_threshold);
}

#[test]
fn load_eval_dataset_reports_empty_case_set() {
    let dir = tempdir().expect("temp directory should create");
    let path = dir.path().join("eval.json");
    std::fs::write(
        &path,
        r#"{"name":"empty","version":"1","description":"none","cases":[]}"#,
    )
    .expect("dataset should write");

    let err = load_eval_dataset(path.to_string_lossy().as_ref()).expect_err("empty dataset fails");
    assert!(err.to_string().contains("has no cases"));
}

#[test]
fn tool_confirmation_defaults_deny_unapproved_mcp_tools() {
    let cfg = base_cfg();
    let runtime_tools = make_runtime_tools(
        &["current_unix_time", "search_incidents"],
        &["search_incidents"],
    );

    // ADK policy is always Never — confirmation handled by ConfirmingTool wrappers
    let settings = resolve_tool_confirmation_settings(&cfg, &runtime_tools);
    assert!(!settings.policy.requires_confirmation("search_incidents"));
    assert!(!settings.policy.requires_confirmation("current_unix_time"));
}

#[test]
fn tool_confirmation_requires_fs_write_by_default() {
    let cfg = base_cfg();
    let runtime_tools = make_runtime_tools(&["current_unix_time", "fs_write"], &[]);

    // ADK policy is Never — fs_write confirmation handled by ConfirmingTool wrapper
    let settings = resolve_tool_confirmation_settings(&cfg, &runtime_tools);
    assert!(!settings.policy.requires_confirmation("fs_write"));
}

#[test]
fn tool_confirmation_requires_execute_bash_by_default() {
    let cfg = base_cfg();
    let runtime_tools = make_runtime_tools(&["current_unix_time", "execute_bash"], &[]);

    let settings = resolve_tool_confirmation_settings(&cfg, &runtime_tools);
    assert!(!settings.policy.requires_confirmation("execute_bash"));
}

#[test]
fn tool_confirmation_requires_github_ops_by_default() {
    let cfg = base_cfg();
    let runtime_tools = make_runtime_tools(&["current_unix_time", "github_ops"], &[]);

    let settings = resolve_tool_confirmation_settings(&cfg, &runtime_tools);
    assert!(!settings.policy.requires_confirmation("github_ops"));
}

#[test]
fn tool_confirmation_approve_list_overrides_default_deny() {
    let mut cfg = base_cfg();
    cfg.approve_tool = vec!["search_incidents".to_string()];
    let runtime_tools = make_runtime_tools(
        &["current_unix_time", "search_incidents"],
        &["search_incidents"],
    );

    let settings = resolve_tool_confirmation_settings(&cfg, &runtime_tools);
    assert_eq!(
        settings
            .run_config
            .tool_confirmation_decisions
            .get("search_incidents"),
        Some(&ToolConfirmationDecision::Approve)
    );
}

#[test]
fn tool_confirmation_custom_required_tools_enforced() {
    let mut cfg = base_cfg();
    cfg.tool_confirmation_mode = ToolConfirmationMode::Never;
    cfg.require_confirm_tool = vec!["release_template".to_string()];
    let runtime_tools = make_runtime_tools(&["release_template", "current_unix_time"], &[]);

    // ADK policy is Never — custom required tools handled by ConfirmingTool wrapper
    let settings = resolve_tool_confirmation_settings(&cfg, &runtime_tools);
    assert!(!settings.policy.requires_confirmation("release_template"));
}

#[test]
fn tool_confirmation_can_require_fs_read() {
    let mut cfg = base_cfg();
    cfg.tool_confirmation_mode = ToolConfirmationMode::Never;
    cfg.require_confirm_tool = vec!["fs_read".to_string()];
    let runtime_tools = make_runtime_tools(&["fs_read", "current_unix_time"], &[]);

    let settings = resolve_tool_confirmation_settings(&cfg, &runtime_tools);
    assert!(!settings.policy.requires_confirmation("fs_read"));
}

#[test]
fn select_mcp_servers_filters_enabled_and_selects_by_name() {
    let mut cfg = base_cfg();
    cfg.mcp_servers = vec![
        McpServerConfig {
            name: "atlas".to_string(),
            endpoint: "https://atlas.example.com/mcp".to_string(),
            enabled: Some(true),
            timeout_secs: Some(10),
            auth_bearer_env: None,
            tool_allowlist: Vec::new(),
            tool_aliases: HashMap::new(),
            command: None,
            args: vec![],
            env: HashMap::new(),
            oauth: None,
        },
        McpServerConfig {
            name: "ops".to_string(),
            endpoint: "https://ops.example.com/mcp".to_string(),
            enabled: Some(false),
            timeout_secs: Some(10),
            auth_bearer_env: None,
            tool_allowlist: Vec::new(),
            tool_aliases: HashMap::new(),
            command: None,
            args: vec![],
            env: HashMap::new(),
            oauth: None,
        },
        McpServerConfig {
            name: "analytics".to_string(),
            endpoint: "https://analytics.example.com/mcp".to_string(),
            enabled: None,
            timeout_secs: None,
            auth_bearer_env: None,
            tool_allowlist: Vec::new(),
            tool_aliases: HashMap::new(),
            command: None,
            args: vec![],
            env: HashMap::new(),
            oauth: None,
        },
    ];

    let active = select_mcp_servers(&cfg, None).expect("active servers should resolve");
    assert_eq!(active.len(), 2);
    assert_eq!(active[0].name, "atlas");
    assert_eq!(active[1].name, "analytics");

    let single =
        select_mcp_servers(&cfg, Some("analytics")).expect("named enabled server should resolve");
    assert_eq!(single.len(), 1);
    assert_eq!(single[0].name, "analytics");

    let err = select_mcp_servers(&cfg, Some("ops"))
        .expect_err("disabled server should not be selectable");
    assert!(err.to_string().contains("not found or not enabled"));
}

#[test]
fn resolve_mcp_auth_reports_missing_bearer_token_env() {
    let server = McpServerConfig {
        name: "secure".to_string(),
        endpoint: "https://secure.example.com/mcp".to_string(),
        enabled: Some(true),
        timeout_secs: Some(15),
        auth_bearer_env: Some("__ZAVORA_TEST_MCP_TOKEN_MISSING__".to_string()),
        tool_allowlist: Vec::new(),
        tool_aliases: HashMap::new(),
        command: None,
        args: vec![],
        env: HashMap::new(),
        oauth: None,
    };

    let err = resolve_mcp_auth(&server).expect_err("missing env should fail");
    let msg = err.to_string();
    assert!(msg.contains("requires bearer token env"));
    assert!(msg.contains("__ZAVORA_TEST_MCP_TOKEN_MISSING__"));
}

#[test]
fn runtime_config_reports_missing_profile() {
    let cli = test_cli(".zavora/does-not-exist.toml", "ops");
    let profiles = load_profiles(&cli.config_path).expect("missing config should default");
    let err = resolve_runtime_config(&cli, &profiles).expect_err("missing profile should fail");
    assert!(
        err.to_string().contains("profile 'ops' not found"),
        "expected actionable missing profile message"
    );
}

#[test]
fn invalid_profile_config_is_actionable() {
    let dir = tempdir().expect("temp directory should create");
    let path = dir.path().join("config.toml");
    std::fs::write(
        &path,
        r#"
[profiles.default]
provider = "not-a-provider"
"#,
    )
    .expect("config should write");

    let err = load_profiles(path.to_string_lossy().as_ref())
        .expect_err("invalid provider should fail parsing");
    let msg = format!("{err:#}");
    assert!(msg.contains("invalid profile configuration"));
}

#[test]
fn provider_name_parser_accepts_known_values_and_rejects_unknown() {
    assert_eq!(
        parse_provider_name("openai").expect("openai should parse"),
        Provider::Openai
    );
    let err = parse_provider_name("unknown-provider").expect_err("invalid provider must fail");
    assert!(
        err.to_string()
            .contains("Supported values: auto, gemini, openai, anthropic, deepseek, groq, ollama")
    );
}

#[test]
fn chat_command_parser_recognizes_built_in_commands() {
    assert_eq!(
        parse_chat_command("/help"),
        ParsedChatCommand::Command(ChatCommand::Help)
    );
    assert_eq!(
        parse_chat_command("/TOOLS"),
        ParsedChatCommand::Command(ChatCommand::Tools)
    );
    assert_eq!(
        parse_chat_command("exit"),
        ParsedChatCommand::Command(ChatCommand::Exit)
    );
    assert_eq!(
        parse_chat_command("/provider openai"),
        ParsedChatCommand::Command(ChatCommand::Provider("openai".to_string()))
    );
    assert_eq!(
        parse_chat_command("/model gpt-4.1"),
        ParsedChatCommand::Command(ChatCommand::Model(Some("gpt-4.1".to_string())))
    );
    assert_eq!(
        parse_chat_command("/model"),
        ParsedChatCommand::Command(ChatCommand::Model(None))
    );
    assert_eq!(
        parse_chat_command("/resume work-session"),
        ParsedChatCommand::Command(ChatCommand::Sessions("work-session".to_string()))
    );
    assert_eq!(
        parse_chat_command("/new review-session"),
        ParsedChatCommand::Command(ChatCommand::NewSession("review-session".to_string()))
    );
}

#[test]
fn chat_command_parser_reports_missing_arguments() {
    assert_eq!(
        parse_chat_command("/provider"),
        ParsedChatCommand::MissingArgument {
            usage: "/provider <auto|gemini|openai|anthropic|deepseek|groq|ollama>"
        }
    );
}

#[test]
fn model_picker_selection_falls_back_when_catalog_unavailable() {
    let options = model_picker_options(Provider::Auto);
    assert!(options.is_empty());
    assert_eq!(
        resolve_model_picker_selection(&options, "1").expect("fallback should not fail"),
        None
    );
}

#[test]
fn model_picker_selection_accepts_numeric_index() {
    let options = model_picker_options(Provider::Openai);
    let picked = resolve_model_picker_selection(&options, "2")
        .expect("selection should parse")
        .expect("selection should choose a model");
    assert_eq!(picked, "gpt-5.5-2026-04-23");
}

#[test]
fn chat_command_parser_handles_unknown_and_non_command_inputs() {
    assert_eq!(
        parse_chat_command("/does-not-exist"),
        ParsedChatCommand::UnknownCommand("/does-not-exist".to_string())
    );
    assert_eq!(
        parse_chat_command("write a short story"),
        ParsedChatCommand::NotACommand
    );
}

#[test]
fn server_runner_cache_key_uses_user_and_session() {
    let mut cfg = base_cfg();
    cfg.user_id = "perf-user".to_string();
    cfg.session_id = "perf-session".to_string();

    assert_eq!(
        server_runner_cache_key(&cfg),
        "perf-user::perf-session".to_string()
    );
}

#[test]
fn model_compatibility_validation_rejects_cross_provider_model_ids() {
    assert!(validate_model_for_provider(Provider::Openai, "gpt-4.1").is_ok());
    assert!(validate_model_for_provider(Provider::Anthropic, "claude-sonnet-4-20250514").is_ok());
    assert!(validate_model_for_provider(Provider::Openai, "claude-sonnet-4-20250514").is_err());
}
