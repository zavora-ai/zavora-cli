//! Configuration, agents, telemetry, and guardrail tests.

use super::*;

#[test]
fn error_taxonomy_distinguishes_provider_session_and_tooling() {
    let provider_err = anyhow::anyhow!("OPENAI_API_KEY is required for OpenAI provider");
    let session_err = anyhow::anyhow!("failed to load session 'abc'");
    let tooling_err = anyhow::anyhow!("tool invocation failed: timeout");

    assert_eq!(categorize_error(&provider_err), ErrorCategory::Provider);
    assert_eq!(categorize_error(&session_err), ErrorCategory::Session);
    assert_eq!(categorize_error(&tooling_err), ErrorCategory::Tooling);
}

#[test]
fn runtime_config_uses_selected_profile_defaults() {
    let dir = tempdir().expect("temp directory should create");
    let path = dir.path().join("config.toml");
    std::fs::write(
        &path,
        r#"
[profiles.dev]
provider = "openai"
model = "gpt-4.1"
planner_provider = "openai"
planner_model = "gpt-5.6-sol"
planner_call_budget = 2
session_backend = "sqlite"
session_db_url = "sqlite://.zavora/dev.db"
app_name = "zavora-dev"
user_id = "dev-user"
session_id = "dev-session"
retrieval_backend = "local"
retrieval_doc_path = "docs/knowledge.md"
retrieval_max_chunks = 5
retrieval_max_chars = 2048
retrieval_min_score = 2
"#,
    )
    .expect("config should write");

    let cli = test_cli(path.to_string_lossy().as_ref(), "dev");
    let profiles = load_profiles(&cli.config_path).expect("profiles should load");
    let cfg = resolve_runtime_config(&cli, &profiles).expect("runtime config should resolve");

    assert_eq!(cfg.profile, "dev");
    assert_eq!(cfg.provider, Provider::Openai);
    assert_eq!(cfg.model.as_deref(), Some("gpt-4.1"));
    assert_eq!(cfg.worker_provider, Provider::Openai);
    assert_eq!(cfg.worker_model, "gpt-4.1");
    assert_eq!(cfg.planner_provider, Provider::Openai);
    assert_eq!(cfg.planner_model, "gpt-5.6-sol");
    assert_eq!(cfg.planner_call_budget, 2);
    assert_eq!(cfg.session_backend, SessionBackend::Sqlite);
    assert!(!cfg.show_sensitive_config);
    assert_eq!(cfg.app_name, "zavora-dev");
    assert_eq!(cfg.user_id, "dev-user");
    assert_eq!(cfg.session_id, "dev-session");
    assert_eq!(cfg.retrieval_backend, RetrievalBackend::Local);
    assert_eq!(cfg.retrieval_doc_path.as_deref(), Some("docs/knowledge.md"));
    assert_eq!(cfg.retrieval_max_chunks, 5);
    assert_eq!(cfg.retrieval_max_chars, 2048);
    assert_eq!(cfg.retrieval_min_score, 2);
    assert_eq!(cfg.tool_confirmation_mode, ToolConfirmationMode::McpOnly);
    assert!(cfg.require_confirm_tool.is_empty());
    assert!(cfg.approve_tool.is_empty());
    assert_eq!(cfg.tool_timeout_secs, 45);
    assert_eq!(cfg.tool_retry_attempts, 2);
    assert_eq!(cfg.tool_retry_delay_ms, 500);
    assert!(cfg.telemetry_enabled);
    assert_eq!(cfg.telemetry_path, ".zavora/telemetry/events.jsonl");
    assert_eq!(cfg.guardrail_input_mode, GuardrailMode::Disabled);
    assert_eq!(cfg.guardrail_output_mode, GuardrailMode::Disabled);
    assert!(
        cfg.guardrail_terms.iter().any(|term| term == "password"),
        "default guardrail terms should include baseline sensitive markers"
    );
    assert_eq!(cfg.guardrail_redact_replacement, "[REDACTED]");

    let mut cli_override = test_cli(path.to_string_lossy().as_ref(), "dev");
    cli_override.planner_call_budget = Some(1);
    let overridden =
        resolve_runtime_config(&cli_override, &profiles).expect("CLI override should resolve");
    assert_eq!(overridden.planner_call_budget, 1);
}

#[test]
fn agent_catalog_local_overrides_global_with_deterministic_precedence() {
    let dir = tempdir().expect("temp directory should create");
    let global = dir.path().join("global-agents.toml");
    let local = dir.path().join("local-agents.toml");
    std::fs::write(
        &global,
        r#"
[agents.default]
instruction = "global-default"

[agents.coder]
model = "gpt-4.1"
"#,
    )
    .expect("global agent catalog should write");
    std::fs::write(
        &local,
        r#"
[agents.default]
instruction = "local-default"

[agents.reviewer]
model = "gpt-4.1"
"#,
    )
    .expect("local agent catalog should write");

    let paths = AgentPaths {
        local_catalog: local,
        global_catalog: Some(global),
        selection_file: dir.path().join("selection.toml"),
        local_markdown_roots: Vec::new(),
        global_markdown_roots: Vec::new(),
    };
    let resolved = load_resolved_agents(&paths).expect("agents should load");

    assert_eq!(
        resolved
            .get("default")
            .and_then(|agent| agent.config.instruction.as_deref()),
        Some("local-default")
    );
    assert_eq!(
        resolved.get("default").map(|agent| agent.source),
        Some(AgentSource::Local)
    );
    assert_eq!(
        resolved
            .get("coder")
            .and_then(|agent| agent.config.model.as_deref()),
        Some("gpt-4.1")
    );
    assert_eq!(
        resolved.get("coder").map(|agent| agent.source),
        Some(AgentSource::Global)
    );
    assert_eq!(
        resolved
            .get("reviewer")
            .and_then(|agent| agent.config.model.as_deref()),
        Some("gpt-4.1")
    );
}

#[test]
fn portable_markdown_agents_load_with_policy_and_project_precedence() {
    let dir = tempdir().expect("temp directory should create");
    let global_root = dir.path().join("global-agents");
    let local_root = dir.path().join("local-agents");
    std::fs::create_dir_all(&global_root).unwrap();
    std::fs::create_dir_all(&local_root).unwrap();
    std::fs::write(
        global_root.join("architect.md"),
        "---\nname: architect\ndescription: Global architect\n---\nGlobal rules.",
    )
    .unwrap();
    std::fs::write(
        local_root.join("architect.md"),
        r#"---
name: Architect
description: Project architect
provider: openai
model: gpt-4.1
tools:
  Read: true
  Write: false
  Grep: true
disallowedTools: [Bash]
skills: [repository-*, source-research]
disallowedSkills: [deployment-*]
permissionMode: always
maxTurns: 7
timeoutSeconds: 75
---
Design the smallest coherent system.
"#,
    )
    .unwrap();
    let paths = AgentPaths {
        local_catalog: dir.path().join("missing-local.toml"),
        global_catalog: Some(dir.path().join("missing-global.toml")),
        selection_file: dir.path().join("selection.toml"),
        local_markdown_roots: vec![local_root],
        global_markdown_roots: vec![global_root],
    };

    let agents = load_resolved_agents(&paths).expect("portable agents should load");
    let architect = agents.get("architect").expect("architect agent");
    assert_eq!(architect.source, AgentSource::Local);
    assert_eq!(
        architect.config.description.as_deref(),
        Some("Project architect")
    );
    assert_eq!(architect.config.allow_tools, vec!["grep", "fs_read"]);
    assert_eq!(architect.config.deny_tools, vec!["execute_bash"]);
    assert_eq!(
        architect.config.skills,
        vec!["repository-*", "source-research"]
    );
    assert_eq!(architect.config.deny_skills, vec!["deployment-*"]);
    assert_eq!(architect.config.max_turns, Some(7));
    assert_eq!(architect.config.timeout_secs, Some(75));
    assert_eq!(
        architect.config.tool_confirmation_mode,
        Some(ToolConfirmationMode::Always)
    );
    assert_eq!(
        architect.config.instruction.as_deref(),
        Some("Design the smallest coherent system.")
    );
    assert!(architect.definition_path.is_some());
}

#[test]
fn runtime_config_applies_agent_overrides_for_model_prompt_and_tools() {
    let cli = test_cli(".zavora/config.toml", "default");
    let profiles = ProfilesFile::default();
    let mut agents = implicit_agent_map();
    agents.insert(
        "coder".to_string(),
        ResolvedAgent {
            name: "coder".to_string(),
            source: AgentSource::Local,
            definition_path: None,
            config: AgentFileConfig {
                description: Some("Coding optimized agent".to_string()),
                instruction: Some("Always propose minimal diffs.".to_string()),
                provider: Some(Provider::Openai),
                model: Some("gpt-4.1".to_string()),
                tool_confirmation_mode: Some(ToolConfirmationMode::Always),
                resource_paths: vec!["docs/CONTRIBUTING.md".to_string()],
                allow_tools: vec!["fs_read".to_string(), "fs_write".to_string()],
                deny_tools: vec!["execute_bash".to_string()],
                skills: vec!["repository-*".to_string()],
                deny_skills: vec!["deployment".to_string()],
                agents: vec!["reviewer*".to_string()],
                deny_agents: vec!["reviewer-dangerous".to_string()],
                max_turns: Some(8),
                timeout_secs: Some(90),
                hooks: HashMap::new(),
            },
        },
    );

    let cfg = resolve_runtime_config_with_agents(&cli, &profiles, &agents, Some("coder"))
        .expect("runtime config should resolve");
    assert_eq!(cfg.agent_name, "coder");
    assert_eq!(cfg.agent_source, AgentSource::Local);
    assert_eq!(cfg.provider, Provider::Openai);
    assert_eq!(cfg.model.as_deref(), Some("gpt-4.1"));
    assert_eq!(cfg.tool_confirmation_mode, ToolConfirmationMode::Always);
    assert_eq!(
        cfg.agent_instruction.as_deref(),
        Some("Always propose minimal diffs.")
    );
    assert_eq!(cfg.agent_resource_paths, vec!["docs/CONTRIBUTING.md"]);
    assert_eq!(cfg.agent_allow_tools, vec!["fs_read", "fs_write"]);
    assert_eq!(cfg.agent_deny_tools, vec!["execute_bash"]);
    assert_eq!(cfg.agent_allow_skills, vec!["repository-*"]);
    assert_eq!(cfg.agent_deny_skills, vec!["deployment"]);
    assert_eq!(cfg.agent_allow_agents, vec!["reviewer*"]);
    assert_eq!(cfg.agent_deny_agents, vec!["reviewer-dangerous"]);
    assert!(crate::config::child_agent_allowed(&cfg, "reviewer_agent"));
    assert!(!crate::config::child_agent_allowed(
        &cfg,
        "reviewer-dangerous"
    ));
    assert!(!crate::config::child_agent_allowed(&cfg, "research_agent"));
    assert_eq!(cfg.agent_max_turns, Some(8));
    assert_eq!(cfg.agent_timeout_secs, Some(90));
}

#[test]
fn resolve_active_agent_falls_back_to_default_when_selection_missing() {
    let cli = test_cli(".zavora/config.toml", "default");
    let agents = implicit_agent_map();
    let selected = resolve_active_agent_name(&cli, &agents, Some("missing-agent"))
        .expect("missing persisted selection should fall back");
    assert_eq!(selected, "default");
}

#[test]
fn resolve_active_agent_reports_missing_explicit_agent() {
    let mut cli = test_cli(".zavora/config.toml", "default");
    cli.agent = Some("missing-agent".to_string());
    let agents = implicit_agent_map();
    let err = resolve_active_agent_name(&cli, &agents, None)
        .expect_err("explicit missing agent should fail");
    assert!(err.to_string().contains("agent 'missing-agent' not found"));
}

#[test]
fn runtime_config_parses_profile_mcp_servers() {
    let dir = tempdir().expect("temp directory should create");
    let path = dir.path().join("config.toml");
    std::fs::write(
        &path,
        r#"
[profiles.dev]
provider = "openai"
model = "gpt-4.1"
tool_confirmation_mode = "always"
require_confirm_tool = ["release_template"]
approve_tool = ["release_template"]
tool_timeout_secs = 90
tool_retry_attempts = 4
tool_retry_delay_ms = 750
guardrail_input_mode = "observe"
guardrail_output_mode = "redact"
guardrail_terms = ["internal-only", "private data"]
guardrail_redact_replacement = "***"

[[profiles.dev.mcp_servers]]
name = "atlas"
endpoint = "https://atlas.example.com/mcp"
enabled = true
timeout_secs = 20
auth_bearer_env = "ATLAS_MCP_TOKEN"
tool_allowlist = ["search", "lookup"]

[[profiles.dev.mcp_servers]]
name = "disabled-tooling"
endpoint = "https://disabled.example.com/mcp"
enabled = false
"#,
    )
    .expect("config should write");

    let cli = test_cli(path.to_string_lossy().as_ref(), "dev");
    let profiles = load_profiles(&cli.config_path).expect("profiles should load");
    let cfg = resolve_runtime_config(&cli, &profiles).expect("runtime config should resolve");

    assert_eq!(cfg.mcp_servers.len(), 2);
    assert_eq!(cfg.mcp_servers[0].name, "atlas");
    assert_eq!(cfg.mcp_servers[0].endpoint, "https://atlas.example.com/mcp");
    assert_eq!(cfg.mcp_servers[0].enabled, Some(true));
    assert_eq!(cfg.mcp_servers[0].timeout_secs, Some(20));
    assert_eq!(
        cfg.mcp_servers[0].auth_bearer_env.as_deref(),
        Some("ATLAS_MCP_TOKEN")
    );
    assert_eq!(cfg.mcp_servers[0].tool_allowlist, vec!["search", "lookup"]);
    assert_eq!(cfg.tool_confirmation_mode, ToolConfirmationMode::Always);
    assert_eq!(cfg.require_confirm_tool, vec!["release_template"]);
    assert_eq!(cfg.approve_tool, vec!["release_template"]);
    assert_eq!(cfg.tool_timeout_secs, 90);
    assert_eq!(cfg.tool_retry_attempts, 4);
    assert_eq!(cfg.tool_retry_delay_ms, 750);
    assert!(cfg.telemetry_enabled);
    assert_eq!(cfg.telemetry_path, ".zavora/telemetry/events.jsonl");
    assert_eq!(cfg.guardrail_input_mode, GuardrailMode::Observe);
    assert_eq!(cfg.guardrail_output_mode, GuardrailMode::Redact);
    assert_eq!(cfg.guardrail_terms, vec!["internal-only", "private data"]);
    assert_eq!(cfg.guardrail_redact_replacement, "***");
    assert_eq!(cfg.mcp_servers[1].name, "disabled-tooling");
    assert_eq!(cfg.mcp_servers[1].enabled, Some(false));
}

#[test]
fn runtime_config_telemetry_cli_overrides_profile_values() {
    let dir = tempdir().expect("temp directory should create");
    let path = dir.path().join("config.toml");
    std::fs::write(
        &path,
        r#"
[profiles.dev]
telemetry_enabled = true
telemetry_path = ".zavora/telemetry/dev.jsonl"
"#,
    )
    .expect("config should write");

    let mut cli = test_cli(path.to_string_lossy().as_ref(), "dev");
    cli.telemetry_enabled = Some(false);
    cli.telemetry_path = Some(".zavora/telemetry/override.jsonl".to_string());

    let profiles = load_profiles(&cli.config_path).expect("profiles should load");
    let cfg = resolve_runtime_config(&cli, &profiles).expect("runtime config should resolve");

    assert!(!cfg.telemetry_enabled);
    assert_eq!(cfg.telemetry_path, ".zavora/telemetry/override.jsonl");
}

#[test]
fn runtime_config_guardrail_cli_overrides_profile_values() {
    let dir = tempdir().expect("temp directory should create");
    let path = dir.path().join("config.toml");
    std::fs::write(
        &path,
        r#"
[profiles.dev]
guardrail_input_mode = "observe"
guardrail_output_mode = "block"
guardrail_terms = ["secret"]
guardrail_redact_replacement = "***"
"#,
    )
    .expect("config should write");

    let mut cli = test_cli(path.to_string_lossy().as_ref(), "dev");
    cli.guardrail_input_mode = Some(GuardrailMode::Block);
    cli.guardrail_output_mode = Some(GuardrailMode::Redact);
    cli.guardrail_term = vec!["token".to_string(), "password".to_string()];
    cli.guardrail_redact_replacement = Some("[MASKED]".to_string());

    let profiles = load_profiles(&cli.config_path).expect("profiles should load");
    let cfg = resolve_runtime_config(&cli, &profiles).expect("runtime config should resolve");

    assert_eq!(cfg.guardrail_input_mode, GuardrailMode::Block);
    assert_eq!(cfg.guardrail_output_mode, GuardrailMode::Redact);
    assert_eq!(cfg.guardrail_terms, vec!["secret", "token", "password"]);
    assert_eq!(cfg.guardrail_redact_replacement, "[MASKED]");
}

#[test]
fn runtime_config_honors_show_sensitive_config_flag() {
    let mut cli = test_cli(".zavora/config.toml", "default");
    cli.show_sensitive_config = true;
    let profiles = ProfilesFile::default();

    let cfg = resolve_runtime_config(&cli, &profiles).expect("runtime config should resolve");
    assert!(cfg.show_sensitive_config);
}

#[test]
fn telemetry_summary_counts_command_and_tool_events() {
    let lines = vec![
        json!({
            "ts_unix_ms": 1000,
            "event": "command.started",
            "run_id": "run-a",
            "command": "ask"
        })
        .to_string(),
        json!({
            "ts_unix_ms": 1100,
            "event": "tool.requested",
            "run_id": "run-a",
            "command": "ask",
            "tool": "release_template"
        })
        .to_string(),
        json!({
            "ts_unix_ms": 1200,
            "event": "tool.succeeded",
            "run_id": "run-a",
            "command": "ask",
            "tool": "release_template"
        })
        .to_string(),
        json!({
            "ts_unix_ms": 1300,
            "event": "command.completed",
            "run_id": "run-a",
            "command": "ask"
        })
        .to_string(),
        json!({
            "ts_unix_ms": 1400,
            "event": "command.failed",
            "run_id": "run-b",
            "command": "workflow.parallel"
        })
        .to_string(),
        "invalid-json-line".to_string(),
    ];

    let summary = summarize_telemetry_lines(lines, 100);
    assert_eq!(summary.total_lines, 6);
    assert_eq!(summary.parsed_events, 5);
    assert_eq!(summary.parse_errors, 1);
    assert_eq!(summary.unique_runs.len(), 2);
    assert_eq!(summary.command_completed, 1);
    assert_eq!(summary.command_failed, 1);
    assert_eq!(summary.tool_requested, 1);
    assert_eq!(summary.tool_succeeded, 1);
    assert_eq!(summary.tool_failed, 0);
    assert_eq!(summary.command_counts.get("ask"), Some(&4));
    assert_eq!(summary.command_counts.get("workflow.parallel"), Some(&1));
    assert_eq!(summary.last_event_ts_unix_ms, Some(1400));
}

#[test]
fn guardrail_redact_mode_masks_detected_terms() {
    let mut cfg = base_cfg();
    cfg.guardrail_terms = vec!["api key".to_string()];
    cfg.guardrail_redact_replacement = "[MASKED]".to_string();
    let telemetry = test_telemetry(&cfg);

    let out = apply_guardrail(
        &cfg,
        &telemetry,
        "output",
        GuardrailMode::Redact,
        "Share the API KEY only with admins.",
    )
    .expect("redact mode should return transformed text");

    assert_eq!(out, "Share the [MASKED] only with admins.");
}

#[test]
fn guardrail_block_mode_rejects_matching_content() {
    let mut cfg = base_cfg();
    cfg.guardrail_terms = vec!["secret".to_string()];
    let telemetry = test_telemetry(&cfg);

    let err = apply_guardrail(
        &cfg,
        &telemetry,
        "input",
        GuardrailMode::Block,
        "This contains a secret token.",
    )
    .expect_err("block mode should fail on term match");
    assert!(err.to_string().contains("guardrail blocked input content"));
}

#[test]
fn guardrail_observe_mode_logs_but_does_not_modify_text() {
    let mut cfg = base_cfg();
    cfg.guardrail_terms = vec!["password".to_string()];
    let telemetry = test_telemetry(&cfg);

    let text = "password rotation should happen every 90 days";
    let out = apply_guardrail(&cfg, &telemetry, "output", GuardrailMode::Observe, text)
        .expect("observe mode should not fail");
    assert_eq!(out, text);
}
