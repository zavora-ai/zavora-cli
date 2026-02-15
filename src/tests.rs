use adk_rust::{ToolConfirmationDecision, ToolConfirmationPolicy};
use std::collections::{BTreeSet, HashMap};
use std::time::Duration;
use std::sync::Arc;

use adk_rust::prelude::*;
use adk_session::*;
use serde_json::{Value, json};


use crate::cli::*;
use crate::config::*;
use crate::error::*;
use crate::telemetry::*;
use crate::guardrail::*;
use crate::eval::*;
use crate::retrieval::*;
use crate::provider::*;
use crate::runner::*;
use crate::workflow::*;
use crate::streaming::*;
use crate::server::*;
use crate::chat::*;
use crate::session::*;
use crate::mcp::*;
use crate::tools::*;
use crate::tools::fs_read::*;
use crate::tools::fs_write::*;
use crate::tools::execute_bash::*;
use crate::tools::github_ops::*;
use crate::tool_policy::*;

    
    use adk_rust::LlmResponse;
    use adk_rust::model::MockLlm;
    use tempfile::tempdir;

    fn base_cfg() -> RuntimeConfig {
        RuntimeConfig {
            profile: "default".to_string(),
            config_path: ".zavora/config.toml".to_string(),
            agent_name: "default".to_string(),
            agent_source: AgentSource::Implicit,
            agent_description: Some("Built-in default assistant".to_string()),
            agent_instruction: None,
            agent_resource_paths: Vec::new(),
            agent_allow_tools: Vec::new(),
            agent_deny_tools: Vec::new(),
            provider: Provider::Auto,
            model: None,
            app_name: "test-app".to_string(),
            user_id: "test-user".to_string(),
            session_id: "test-session".to_string(),
            session_backend: SessionBackend::Memory,
            session_db_url: "sqlite://.zavora/test.db".to_string(),
            show_sensitive_config: false,
            retrieval_backend: RetrievalBackend::Disabled,
            retrieval_doc_path: None,
            retrieval_max_chunks: 3,
            retrieval_max_chars: 4000,
            retrieval_min_score: 1,
            tool_confirmation_mode: ToolConfirmationMode::McpOnly,
            require_confirm_tool: Vec::new(),
            approve_tool: Vec::new(),
            tool_timeout_secs: 45,
            tool_retry_attempts: 2,
            tool_retry_delay_ms: 500,
            telemetry_enabled: false,
            telemetry_path: ".zavora/test-telemetry.jsonl".to_string(),
            guardrail_input_mode: GuardrailMode::Disabled,
            guardrail_output_mode: GuardrailMode::Disabled,
            guardrail_terms: vec!["secret".to_string(), "password".to_string()],
            guardrail_redact_replacement: "[REDACTED]".to_string(),
            mcp_servers: Vec::new(),
            max_prompt_chars: 32_000,
            server_runner_cache_max: 64,
        }
    }

    fn test_telemetry(cfg: &RuntimeConfig) -> TelemetrySink {
        TelemetrySink::new(cfg, "test".to_string())
    }

    fn mock_model(text: &str) -> Arc<dyn Llm> {
        Arc::new(
            MockLlm::new("mock")
                .with_response(LlmResponse::new(Content::new("model").with_text(text))),
        )
    }

    fn noop_tool(name: &str) -> Arc<dyn Tool> {
        Arc::new(FunctionTool::new(
            name,
            "noop tool",
            |_ctx, _args| async move { Ok(json!({"ok": true})) },
        ))
    }

    fn make_runtime_tools(tool_names: &[&str], mcp_tool_names: &[&str]) -> ResolvedRuntimeTools {
        ResolvedRuntimeTools {
            tools: tool_names
                .iter()
                .map(|name| noop_tool(name))
                .collect::<Vec<_>>(),
            mcp_tool_names: mcp_tool_names
                .iter()
                .map(|name| name.to_string())
                .collect::<BTreeSet<String>>(),
        }
    }

    fn sqlite_cfg(session_id: &str) -> (tempfile::TempDir, RuntimeConfig) {
        let dir = tempdir().expect("temp directory should create");
        let db_path = dir.path().join("sessions.db");
        let db_url = format!("sqlite://{}", db_path.to_string_lossy());

        let mut cfg = base_cfg();
        cfg.session_backend = SessionBackend::Sqlite;
        cfg.session_db_url = db_url;
        cfg.session_id = session_id.to_string();

        (dir, cfg)
    }

    fn test_cli(config_path: &str, profile: &str) -> Cli {
        Cli {
            provider: Provider::Auto,
            model: None,
            agent: None,
            profile: profile.to_string(),
            config_path: config_path.to_string(),
            app_name: None,
            user_id: None,
            session_id: None,
            session_backend: None,
            session_db_url: None,
            show_sensitive_config: false,
            retrieval_backend: None,
            retrieval_doc_path: None,
            retrieval_max_chunks: None,
            retrieval_max_chars: None,
            retrieval_min_score: None,
            tool_confirmation_mode: None,
            require_confirm_tool: Vec::new(),
            approve_tool: Vec::new(),
            tool_timeout_secs: None,
            tool_retry_attempts: None,
            tool_retry_delay_ms: None,
            telemetry_enabled: None,
            telemetry_path: None,
            guardrail_input_mode: None,
            guardrail_output_mode: None,
            guardrail_term: Vec::new(),
            guardrail_redact_replacement: None,
            log_filter: "warn".to_string(),
            command: Commands::Doctor,
        }
    }

    fn test_execute_bash_request(command: &str) -> ExecuteBashRequest {
        ExecuteBashRequest {
            command: command.to_string(),
            approved: false,
            allow_dangerous: false,
            timeout_secs: EXECUTE_BASH_DEFAULT_TIMEOUT_SECS,
            retry_attempts: EXECUTE_BASH_DEFAULT_RETRY_ATTEMPTS,
            retry_delay_ms: 0,
            max_output_chars: EXECUTE_BASH_DEFAULT_MAX_OUTPUT_CHARS,
        }
    }

    async fn create_session(cfg: &RuntimeConfig, session_id: &str) {
        let service = build_session_service(cfg)
            .await
            .expect("service should build");
        service
            .create(CreateRequest {
                app_name: cfg.app_name.clone(),
                user_id: cfg.user_id.clone(),
                session_id: Some(session_id.to_string()),
                state: HashMap::new(),
            })
            .await
            .expect("session should create");
    }

    async fn list_session_ids(cfg: &RuntimeConfig) -> Vec<String> {
        let service = build_session_service(cfg)
            .await
            .expect("service should build");
        let mut sessions = service
            .list(ListRequest {
                app_name: cfg.app_name.clone(),
                user_id: cfg.user_id.clone(),
            })
            .await
            .expect("sessions should list")
            .into_iter()
            .map(|s| s.id().to_string())
            .collect::<Vec<String>>();
        sessions.sort();
        sessions
    }

    #[tokio::test]
    async fn single_workflow_returns_deterministic_mock_output() {
        let cfg = base_cfg();
        let telemetry = test_telemetry(&cfg);
        let runner = build_runner(
            build_single_agent(mock_model("single response")).expect("agent should build"),
            &cfg,
        )
        .await
        .expect("runner should build");

        let out = run_prompt(&runner, &cfg, "hello", &telemetry)
            .await
            .expect("prompt should run");
        assert_eq!(out, "single response");
    }

    #[tokio::test]
    async fn workflow_modes_return_deterministic_mock_output() {
        let modes = [
            WorkflowMode::Single,
            WorkflowMode::Sequential,
            WorkflowMode::Parallel,
            WorkflowMode::Loop,
            WorkflowMode::Graph,
        ];

        for mode in modes {
            let mut cfg = base_cfg();
            cfg.session_id = format!("session-{mode:?}");
            let telemetry = test_telemetry(&cfg);
            let runner = build_runner(
                build_workflow_agent(
                    mode,
                    mock_model("workflow response"),
                    1,
                    &build_builtin_tools(),
                    ToolConfirmationPolicy::Never,
                    Duration::from_secs(45),
                    None,
                )
                .expect("workflow should build"),
                &cfg,
            )
            .await
            .expect("runner should build");

            let out = run_prompt(&runner, &cfg, "build a plan", &telemetry)
                .await
                .expect("prompt should run");
            assert_eq!(out, "workflow response");
        }
    }

    #[tokio::test]
    async fn sqlite_session_backend_persists_history_between_runners() {
        let dir = tempdir().expect("temp directory should create");
        let db_path = dir.path().join("sessions.db");
        let db_url = format!("sqlite://{}", db_path.to_string_lossy());

        let mut cfg = base_cfg();
        cfg.session_backend = SessionBackend::Sqlite;
        cfg.session_db_url = db_url.clone();
        cfg.session_id = "persisted-session".to_string();
        let telemetry = test_telemetry(&cfg);

        let runner_one = build_runner(
            build_single_agent(mock_model("first answer")).expect("agent should build"),
            &cfg,
        )
        .await
        .expect("runner should build");

        let _ = run_prompt(&runner_one, &cfg, "first prompt", &telemetry)
            .await
            .expect("first prompt should run");

        let runner_two = build_runner(
            build_single_agent(mock_model("second answer")).expect("agent should build"),
            &cfg,
        )
        .await
        .expect("second runner should build");

        let _ = run_prompt(&runner_two, &cfg, "second prompt", &telemetry)
            .await
            .expect("second prompt should run");

        let service = DatabaseSessionService::new(&db_url)
            .await
            .expect("db should open");
        service.migrate().await.expect("migration should run");

        let session = service
            .get(GetRequest {
                app_name: cfg.app_name.clone(),
                user_id: cfg.user_id.clone(),
                session_id: cfg.session_id.clone(),
                num_recent_events: None,
                after: None,
            })
            .await
            .expect("session should exist");

        assert!(
            session.events().len() >= 4,
            "expected persisted event history across runs"
        );
    }

    #[test]
    fn ingest_author_text_handles_partial_then_final_snapshot() {
        let mut buffer = String::new();

        let d1 = ingest_author_text(&mut buffer, "Hello", true, false);
        let d2 = ingest_author_text(&mut buffer, " world", true, false);
        let d3 = ingest_author_text(&mut buffer, "Hello world", false, true);

        assert_eq!(d1, "Hello");
        assert_eq!(d2, " world");
        assert!(d3.is_empty(), "final snapshot should not duplicate output");
        assert_eq!(buffer, "Hello world");
    }

    #[test]
    fn ingest_author_text_handles_non_partial_incremental_chunks() {
        let mut buffer = String::new();

        let d1 = ingest_author_text(&mut buffer, "Hello", false, false);
        let d2 = ingest_author_text(&mut buffer, " world", false, false);
        let d3 = ingest_author_text(&mut buffer, "Hello world", false, true);

        assert_eq!(d1, "Hello");
        assert_eq!(d2, " world");
        assert!(d3.is_empty(), "final snapshot should be deduplicated");
        assert_eq!(buffer, "Hello world");
    }

    #[test]
    fn tracker_falls_back_to_last_textful_author() {
        let mut tracker = AuthorTextTracker::default();

        let _ = tracker.ingest_parts("assistant", "hello", false, false);
        let _ = tracker.ingest_parts("tool", "", false, true);

        assert_eq!(tracker.resolve_text().as_deref(), Some("hello"));
    }

    #[test]
    fn final_stream_suffix_emits_only_missing_tail() {
        assert_eq!(
            final_stream_suffix("Hello", "Hello world").as_deref(),
            Some(" world")
        );
        assert_eq!(final_stream_suffix("Hello world", "Hello world"), None);
        assert_eq!(
            final_stream_suffix("", "Hello world").as_deref(),
            Some("Hello world")
        );
    }

    #[test]
    fn workflow_route_classifier_is_deterministic_for_key_intents() {
        assert_eq!(
            classify_workflow_route("Plan release milestones"),
            "release"
        );
        assert_eq!(
            classify_workflow_route("Evaluate architecture tradeoffs"),
            "architecture"
        );
        assert_eq!(
            classify_workflow_route("List risk mitigations and rollback"),
            "risk"
        );
        assert_eq!(
            classify_workflow_route("Implement feature work"),
            "delivery"
        );
    }

    #[test]
    fn workflow_templates_exist_for_all_graph_routes() {
        for route in ["release", "architecture", "risk", "delivery"] {
            let template = workflow_template(route);
            assert!(
                !template.trim().is_empty(),
                "template should be non-empty for route {route}"
            );
        }
    }

    #[test]
    fn tool_failure_extractor_handles_common_error_shapes() {
        assert_eq!(
            extract_tool_failure_message(&json!({"error": "denied by policy"})).as_deref(),
            Some("denied by policy")
        );
        assert_eq!(
            extract_tool_failure_message(&json!({"status": "error", "message": "timeout"}))
                .as_deref(),
            Some("timeout")
        );
        assert_eq!(extract_tool_failure_message(&json!({"ok": true})), None);
    }

    #[test]
    fn fs_read_reads_allowed_file_content() {
        let dir = tempdir().expect("temp directory should create");
        std::fs::write(dir.path().join("notes.txt"), "alpha\nbeta\ngamma\n")
            .expect("fixture file should write");
        let workspace_root = dir
            .path()
            .canonicalize()
            .expect("workspace root should resolve");

        let payload = fs_read_tool_response_with_root(
            &json!({
                "path": "notes.txt",
                "start_line": 2,
                "max_lines": 1
            }),
            &workspace_root,
        );

        assert_eq!(payload["status"], "ok");
        assert_eq!(payload["kind"], "file");
        assert_eq!(payload["content"], "beta");
        assert_eq!(payload["line_count"], 1);
    }

    #[test]
    fn fs_read_lists_directory_entries() {
        let dir = tempdir().expect("temp directory should create");
        std::fs::create_dir_all(dir.path().join("docs")).expect("fixture dir should create");
        std::fs::write(dir.path().join("README.md"), "hello").expect("fixture file should write");
        let workspace_root = dir
            .path()
            .canonicalize()
            .expect("workspace root should resolve");

        let payload = fs_read_tool_response_with_root(
            &json!({
                "path": ".",
                "max_entries": 10
            }),
            &workspace_root,
        );

        assert_eq!(payload["status"], "ok");
        assert_eq!(payload["kind"], "directory");
        assert_eq!(payload["entry_count"], 2);
        let entries = payload["entries"]
            .as_array()
            .expect("entries should be an array");
        assert!(
            entries
                .iter()
                .any(|entry| entry.get("name") == Some(&Value::String("README.md".to_string())))
        );
        assert!(
            entries
                .iter()
                .any(|entry| entry.get("name") == Some(&Value::String("docs".to_string())))
        );
    }

    #[test]
    fn fs_read_denies_blocked_paths() {
        let dir = tempdir().expect("temp directory should create");
        std::fs::write(dir.path().join(".env"), "OPENAI_API_KEY=test").expect("fixture file");
        let workspace_root = dir
            .path()
            .canonicalize()
            .expect("workspace root should resolve");

        let payload = fs_read_tool_response_with_root(&json!({ "path": ".env" }), &workspace_root);
        assert_eq!(payload["status"], "error");
        assert_eq!(payload["code"], "denied_path");
        assert!(
            extract_tool_failure_message(&payload)
                .as_deref()
                .unwrap_or_default()
                .contains("denied path")
        );
    }

    #[test]
    fn fs_read_reports_invalid_paths() {
        let dir = tempdir().expect("temp directory should create");
        let workspace_root = dir
            .path()
            .canonicalize()
            .expect("workspace root should resolve");
        let payload =
            fs_read_tool_response_with_root(&json!({ "path": "missing.txt" }), &workspace_root);
        assert_eq!(payload["status"], "error");
        assert_eq!(payload["code"], "invalid_path");
    }

    #[test]
    fn fs_write_creates_file_when_mode_is_create() {
        let dir = tempdir().expect("temp directory should create");
        let workspace_root = dir
            .path()
            .canonicalize()
            .expect("workspace root should resolve");

        let payload = fs_write_tool_response_with_root(
            &json!({
                "path": "docs/new.txt",
                "mode": "create",
                "content": "release-ready"
            }),
            &workspace_root,
        );

        assert_eq!(payload["status"], "ok");
        assert_eq!(payload["mode"], "create");
        let content = std::fs::read_to_string(dir.path().join("docs/new.txt"))
            .expect("created file should be readable");
        assert_eq!(content, "release-ready");
    }

    #[test]
    fn fs_write_patch_updates_existing_content() {
        let dir = tempdir().expect("temp directory should create");
        let workspace_root = dir
            .path()
            .canonicalize()
            .expect("workspace root should resolve");
        std::fs::write(dir.path().join("plan.md"), "Ship alpha then beta")
            .expect("fixture file should write");

        let payload = fs_write_tool_response_with_root(
            &json!({
                "path": "plan.md",
                "mode": "patch",
                "patch": {
                    "find": "beta",
                    "replace": "rc"
                }
            }),
            &workspace_root,
        );

        assert_eq!(payload["status"], "ok");
        assert_eq!(payload["mode"], "patch");
        assert_eq!(payload["replaced_count"], 1);
        let content =
            std::fs::read_to_string(dir.path().join("plan.md")).expect("patched file should read");
        assert_eq!(content, "Ship alpha then rc");
    }

    #[test]
    fn fs_write_denies_blocked_paths() {
        let dir = tempdir().expect("temp directory should create");
        let workspace_root = dir
            .path()
            .canonicalize()
            .expect("workspace root should resolve");

        let payload = fs_write_tool_response_with_root(
            &json!({
                "path": ".env",
                "mode": "overwrite",
                "content": "should-not-write"
            }),
            &workspace_root,
        );

        assert_eq!(payload["status"], "error");
        assert_eq!(payload["code"], "denied_path");
    }

    #[test]
    fn fs_write_rejects_malformed_patch_requests() {
        let dir = tempdir().expect("temp directory should create");
        let workspace_root = dir
            .path()
            .canonicalize()
            .expect("workspace root should resolve");
        std::fs::write(dir.path().join("plan.md"), "alpha").expect("fixture file should write");

        let payload = fs_write_tool_response_with_root(
            &json!({
                "path": "plan.md",
                "mode": "patch",
                "patch": {
                    "find": "",
                    "replace": "beta"
                }
            }),
            &workspace_root,
        );

        assert_eq!(payload["status"], "error");
        assert_eq!(payload["code"], "malformed_edit");
    }

    #[test]
    fn execute_bash_policy_denies_blocked_patterns_without_override() {
        let request = test_execute_bash_request("rm -rf .");
        let err =
            evaluate_execute_bash_policy(&request).expect_err("dangerous pattern should fail");
        assert_eq!(err.code, "denied_command");
    }

    #[test]
    fn execute_bash_policy_allows_dangerous_override_when_approved() {
        let mut request = test_execute_bash_request("rm -rf ./tmp");
        request.allow_dangerous = true;
        request.approved = true;

        let decision =
            evaluate_execute_bash_policy(&request).expect("approved override should pass");
        assert!(!decision.read_only_auto_allow);
    }

    #[test]
    fn execute_bash_policy_auto_allows_read_only_commands() {
        let request = test_execute_bash_request("git status");
        let decision = evaluate_execute_bash_policy(&request).expect("read-only should pass");
        assert!(decision.read_only_auto_allow);
    }

    #[tokio::test]
    async fn execute_bash_retries_failed_commands_when_configured() {
        let payload = execute_bash_tool_response(&json!({
            "command": "false",
            "approved": true,
            "retry_attempts": 2,
            "retry_delay_ms": 0
        }))
        .await;

        assert_eq!(payload["status"], "error");
        assert_eq!(payload["code"], "command_failed");
        assert_eq!(payload["attempts"], 2);
    }

    #[test]
    fn github_ops_issue_create_runs_expected_mocked_command() {
        let calls = std::cell::RefCell::new(Vec::<Vec<String>>::new());
        let payload = github_ops_tool_response_with_runner(
            &json!({
                "action": "issue_create",
                "repo": "zavora-ai/zavora-cli",
                "title": "Test issue",
                "body": "Issue body",
                "labels": ["bug", "sprint:8"]
            }),
            true,
            |args| {
                calls.borrow_mut().push(args.to_vec());
                Ok(GitHubCliOutput {
                    success: true,
                    exit_code: 0,
                    stdout: "https://github.com/zavora-ai/zavora-cli/issues/999".to_string(),
                    stderr: String::new(),
                })
            },
        );

        assert_eq!(payload["status"], "ok");
        assert_eq!(payload["action"], "issue_create");
        let calls = calls.borrow();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0][0], "issue");
        assert_eq!(calls[0][1], "create");
    }

    #[test]
    fn github_ops_preflight_requires_auth_without_token() {
        let payload = github_ops_tool_response_with_runner(
            &json!({
                "action": "issue_create",
                "repo": "zavora-ai/zavora-cli",
                "title": "Needs auth",
                "body": "body"
            }),
            false,
            |_args| {
                Ok(GitHubCliOutput {
                    success: false,
                    exit_code: 1,
                    stdout: String::new(),
                    stderr: "not logged in".to_string(),
                })
            },
        );

        assert_eq!(payload["status"], "error");
        assert_eq!(payload["code"], "auth_required");
    }

    #[test]
    fn github_ops_project_item_update_runs_expected_mocked_command() {
        let calls = std::cell::RefCell::new(Vec::<Vec<String>>::new());
        let payload = github_ops_tool_response_with_runner(
            &json!({
                "action": "project_item_update",
                "project_id": "PVT_kwDOBVKgdc4BPPxU",
                "item_id": "PVTI_lADOBVKgdc4BPPxUzglepjM",
                "field_id": "PVTSSF_lADOBVKgdc4BPPxUzg9te4w",
                "status_option_id": "98236657"
            }),
            true,
            |args| {
                calls.borrow_mut().push(args.to_vec());
                Ok(GitHubCliOutput {
                    success: true,
                    exit_code: 0,
                    stdout: String::new(),
                    stderr: String::new(),
                })
            },
        );

        assert_eq!(payload["status"], "ok");
        assert_eq!(payload["action"], "project_item_update");
        let calls = calls.borrow();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0][0], "project");
        assert_eq!(calls[0][1], "item-edit");
    }

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
model = "gpt-4o-mini"
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
        assert_eq!(cfg.model.as_deref(), Some("gpt-4o-mini"));
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
model = "gpt-4o-mini"
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
            Some("gpt-4o-mini")
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
    fn runtime_config_applies_agent_overrides_for_model_prompt_and_tools() {
        let cli = test_cli(".zavora/config.toml", "default");
        let profiles = ProfilesFile::default();
        let mut agents = implicit_agent_map();
        agents.insert(
            "coder".to_string(),
            ResolvedAgent {
                name: "coder".to_string(),
                source: AgentSource::Local,
                config: AgentFileConfig {
                    description: Some("Coding optimized agent".to_string()),
                    instruction: Some("Always propose minimal diffs.".to_string()),
                    provider: Some(Provider::Openai),
                    model: Some("gpt-4.1".to_string()),
                    tool_confirmation_mode: Some(ToolConfirmationMode::Always),
                    resource_paths: vec!["docs/CONTRIBUTING.md".to_string()],
                    allow_tools: vec!["fs_read".to_string(), "fs_write".to_string()],
                    deny_tools: vec!["execute_bash".to_string()],
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
model = "gpt-4o-mini"
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

        let err =
            load_eval_dataset(path.to_string_lossy().as_ref()).expect_err("empty dataset fails");
        assert!(err.to_string().contains("has no cases"));
    }

    #[test]
    fn tool_confirmation_defaults_deny_unapproved_mcp_tools() {
        let cfg = base_cfg();
        let runtime_tools = make_runtime_tools(
            &["current_unix_time", "search_incidents"],
            &["search_incidents"],
        );

        let settings = resolve_tool_confirmation_settings(&cfg, &runtime_tools);
        assert!(settings.policy.requires_confirmation("search_incidents"));
        assert!(!settings.policy.requires_confirmation("current_unix_time"));
        assert_eq!(
            settings
                .run_config
                .tool_confirmation_decisions
                .get("search_incidents"),
            Some(&ToolConfirmationDecision::Deny)
        );
    }

    #[test]
    fn tool_confirmation_requires_fs_write_by_default() {
        let cfg = base_cfg();
        let runtime_tools = make_runtime_tools(&["current_unix_time", "fs_write"], &[]);

        let settings = resolve_tool_confirmation_settings(&cfg, &runtime_tools);
        assert!(settings.policy.requires_confirmation("fs_write"));
        assert_eq!(
            settings
                .run_config
                .tool_confirmation_decisions
                .get("fs_write"),
            Some(&ToolConfirmationDecision::Deny)
        );
    }

    #[test]
    fn tool_confirmation_requires_execute_bash_by_default() {
        let cfg = base_cfg();
        let runtime_tools = make_runtime_tools(&["current_unix_time", "execute_bash"], &[]);

        let settings = resolve_tool_confirmation_settings(&cfg, &runtime_tools);
        assert!(settings.policy.requires_confirmation("execute_bash"));
        assert_eq!(
            settings
                .run_config
                .tool_confirmation_decisions
                .get("execute_bash"),
            Some(&ToolConfirmationDecision::Deny)
        );
    }

    #[test]
    fn tool_confirmation_requires_github_ops_by_default() {
        let cfg = base_cfg();
        let runtime_tools = make_runtime_tools(&["current_unix_time", "github_ops"], &[]);

        let settings = resolve_tool_confirmation_settings(&cfg, &runtime_tools);
        assert!(settings.policy.requires_confirmation("github_ops"));
        assert_eq!(
            settings
                .run_config
                .tool_confirmation_decisions
                .get("github_ops"),
            Some(&ToolConfirmationDecision::Deny)
        );
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

        let settings = resolve_tool_confirmation_settings(&cfg, &runtime_tools);
        assert!(settings.policy.requires_confirmation("release_template"));
        assert!(!settings.policy.requires_confirmation("current_unix_time"));
        assert_eq!(
            settings
                .run_config
                .tool_confirmation_decisions
                .get("release_template"),
            Some(&ToolConfirmationDecision::Deny)
        );
    }

    #[test]
    fn tool_confirmation_can_require_fs_read() {
        let mut cfg = base_cfg();
        cfg.tool_confirmation_mode = ToolConfirmationMode::Never;
        cfg.require_confirm_tool = vec!["fs_read".to_string()];
        let runtime_tools = make_runtime_tools(&["fs_read", "current_unix_time"], &[]);

        let settings = resolve_tool_confirmation_settings(&cfg, &runtime_tools);
        assert!(settings.policy.requires_confirmation("fs_read"));
        assert_eq!(
            settings
                .run_config
                .tool_confirmation_decisions
                .get("fs_read"),
            Some(&ToolConfirmationDecision::Deny)
        );
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
            },
            McpServerConfig {
                name: "ops".to_string(),
                endpoint: "https://ops.example.com/mcp".to_string(),
                enabled: Some(false),
                timeout_secs: Some(10),
                auth_bearer_env: None,
                tool_allowlist: Vec::new(),
                tool_aliases: HashMap::new(),
            },
            McpServerConfig {
                name: "analytics".to_string(),
                endpoint: "https://analytics.example.com/mcp".to_string(),
                enabled: None,
                timeout_secs: None,
                auth_bearer_env: None,
                tool_allowlist: Vec::new(),
                tool_aliases: HashMap::new(),
            },
        ];

        let active = select_mcp_servers(&cfg, None).expect("active servers should resolve");
        assert_eq!(active.len(), 2);
        assert_eq!(active[0].name, "atlas");
        assert_eq!(active[1].name, "analytics");

        let single = select_mcp_servers(&cfg, Some("analytics"))
            .expect("named enabled server should resolve");
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
            err.to_string().contains(
                "Supported values: auto, gemini, openai, anthropic, deepseek, groq, ollama"
            )
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
            parse_chat_command("/model gpt-4o-mini"),
            ParsedChatCommand::Command(ChatCommand::Model(Some("gpt-4o-mini".to_string())))
        );
        assert_eq!(
            parse_chat_command("/model"),
            ParsedChatCommand::Command(ChatCommand::Model(None))
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
        assert_eq!(picked, "gpt-4.1");
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
        assert!(validate_model_for_provider(Provider::Openai, "gpt-4o-mini").is_ok());
        assert!(
            validate_model_for_provider(Provider::Anthropic, "claude-sonnet-4-20250514").is_ok()
        );
        assert!(validate_model_for_provider(Provider::Openai, "claude-sonnet-4-20250514").is_err());
    }

    #[test]
    fn augment_prompt_with_retrieval_leaves_prompt_unchanged_when_disabled() {
        let retrieval = DisabledRetrievalService;
        let prompt = "Plan release milestones";
        let out = augment_prompt_with_retrieval(
            &retrieval,
            prompt,
            RetrievalPolicy {
                max_chunks: 3,
                max_chars: 4000,
                min_score: 1,
            },
        )
        .expect("prompt augmentation should pass");
        assert_eq!(out, prompt);
    }

    #[test]
    fn local_file_retrieval_returns_relevant_chunks() {
        let dir = tempdir().expect("temp directory should create");
        let path = dir.path().join("knowledge.txt");
        std::fs::write(
            &path,
            "Rust CLI release planning\n\nADK retrieval abstraction and context injection",
        )
        .expect("doc file should write");

        let retrieval = LocalFileRetrievalService::load(path.to_string_lossy().as_ref())
            .expect("local retrieval should load");
        let chunks = retrieval
            .retrieve("retrieval abstraction", 3)
            .expect("retrieval should run");
        assert!(!chunks.is_empty(), "expected at least one relevant chunk");
    }

    #[test]
    fn local_file_retrieval_ranks_chunks_deterministically_by_term_hits() {
        let retrieval = LocalFileRetrievalService {
            chunks: vec![
                RetrievedChunk {
                    source: "rank:1".to_string(),
                    text: "release quality gates".to_string(),
                    score: 0,
                },
                RetrievedChunk {
                    source: "rank:2".to_string(),
                    text: "release quality gates release quality".to_string(),
                    score: 0,
                },
            ],
        };

        let chunks = retrieval
            .retrieve("release quality", 2)
            .expect("retrieval should run");
        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0].source, "rank:2");
        assert_eq!(chunks[1].source, "rank:1");
    }

    #[test]
    fn local_retrieval_backend_requires_doc_path() {
        let mut cfg = base_cfg();
        cfg.retrieval_backend = RetrievalBackend::Local;
        cfg.retrieval_doc_path = None;

        let err = match build_retrieval_service(&cfg) {
            Ok(_) => panic!("missing doc path should fail"),
            Err(err) => err,
        };
        assert!(
            err.to_string()
                .contains("retrieval backend 'local' requires")
        );
    }

    #[test]
    fn local_retrieval_backend_missing_file_is_reported() {
        let mut cfg = base_cfg();
        cfg.retrieval_backend = RetrievalBackend::Local;
        cfg.retrieval_doc_path = Some("does-not-exist.md".to_string());

        let err = match build_retrieval_service(&cfg) {
            Ok(_) => panic!("missing retrieval file should fail"),
            Err(err) => err,
        };
        assert!(
            err.to_string().contains("failed to read retrieval doc"),
            "expected backend unavailability error path"
        );
    }

    #[test]
    fn retrieval_policy_enforces_context_budget_and_score_threshold() {
        let retrieval = LocalFileRetrievalService {
            chunks: vec![
                RetrievedChunk {
                    source: "test:1".to_string(),
                    text: "alpha beta gamma delta".to_string(),
                    score: 10,
                },
                RetrievedChunk {
                    source: "test:2".to_string(),
                    text: "small".to_string(),
                    score: 1,
                },
            ],
        };

        let out = augment_prompt_with_retrieval(
            &retrieval,
            "alpha",
            RetrievalPolicy {
                max_chunks: 3,
                max_chars: 5,
                min_score: 2,
            },
        )
        .expect("augmentation should pass");

        assert!(
            out.contains("alpha"),
            "expected retained high-score context"
        );
        assert!(
            !out.contains("small"),
            "expected low-score chunk to be filtered"
        );
    }

    #[test]
    fn retrieval_augmentation_falls_back_when_no_matches() {
        let retrieval = LocalFileRetrievalService {
            chunks: vec![RetrievedChunk {
                source: "fallback:1".to_string(),
                text: "unrelated content".to_string(),
                score: 0,
            }],
        };

        let prompt = "release rollout";
        let out = augment_prompt_with_retrieval(
            &retrieval,
            prompt,
            RetrievalPolicy {
                max_chunks: 3,
                max_chars: 4000,
                min_score: 1,
            },
        )
        .expect("augmentation should pass");
        assert_eq!(
            out, prompt,
            "no-result path should preserve original prompt"
        );
    }

    #[cfg(not(feature = "semantic-search"))]
    #[test]
    fn semantic_retrieval_backend_requires_feature_flag() {
        let mut cfg = base_cfg();
        cfg.retrieval_backend = RetrievalBackend::Semantic;
        cfg.retrieval_doc_path = Some("README.md".to_string());
        let err = match build_retrieval_service(&cfg) {
            Ok(_) => panic!("semantic retrieval should require feature flag"),
            Err(err) => err,
        };
        assert!(
            err.to_string()
                .contains("requires feature 'semantic-search'")
        );
    }

    #[cfg(feature = "semantic-search")]
    #[test]
    fn semantic_retrieval_backend_returns_ranked_chunks() {
        let dir = tempdir().expect("temp directory should create");
        let path = dir.path().join("knowledge.txt");
        std::fs::write(
            &path,
            "Agile release planning and rollout gates\n\nSemantic retrieval context ranking",
        )
        .expect("doc file should write");

        let retrieval = SemanticLocalRetrievalService::load(path.to_string_lossy().as_ref())
            .expect("semantic retrieval should load");
        let chunks = retrieval
            .retrieve("rollout gates", 2)
            .expect("semantic retrieval should run");
        assert!(!chunks.is_empty(), "expected semantic retrieval matches");
    }

    #[tokio::test]
    async fn sessions_show_missing_session_returns_session_category_error() {
        let cfg = base_cfg();
        let err = run_sessions_show(&cfg, Some("missing-session".to_string()), 10)
            .await
            .expect_err("missing session should error");

        assert_eq!(categorize_error(&err), ErrorCategory::Session);
        let rendered = format_cli_error(&err, cfg.show_sensitive_config);
        assert!(
            rendered.contains("[SESSION]"),
            "expected session category marker in error output"
        );
    }

    #[test]
    fn redact_sensitive_text_masks_sqlite_urls() {
        let raw = "open failed at sqlite://.zavora/sessions.db; retry sqlite://tmp/test.db";
        let rendered = redact_sensitive_text(raw);

        assert!(!rendered.contains(".zavora/sessions.db"));
        assert!(!rendered.contains("tmp/test.db"));
        assert_eq!(
            rendered,
            "open failed at sqlite://[REDACTED]; retry sqlite://[REDACTED]"
        );
    }

    #[test]
    fn format_cli_error_redacts_sqlite_urls_by_default() {
        let err = anyhow::anyhow!("failed to open sqlite://.zavora/sessions.db");
        let rendered = format_cli_error(&err, false);

        assert!(rendered.contains("sqlite://[REDACTED]"));
        assert!(!rendered.contains(".zavora/sessions.db"));
    }

    #[tokio::test]
    async fn sessions_delete_requires_force_flag() {
        let (_dir, cfg) = sqlite_cfg("default-session");
        create_session(&cfg, "delete-me").await;

        let err = run_sessions_delete(&cfg, Some("delete-me".to_string()), false)
            .await
            .expect_err("delete without --force should fail");
        assert_eq!(categorize_error(&err), ErrorCategory::Input);

        let sessions = list_session_ids(&cfg).await;
        assert!(sessions.contains(&"delete-me".to_string()));
    }

    #[tokio::test]
    async fn sessions_delete_force_removes_target_session() {
        let (_dir, cfg) = sqlite_cfg("default-session");
        create_session(&cfg, "delete-me").await;

        run_sessions_delete(&cfg, Some("delete-me".to_string()), true)
            .await
            .expect("forced delete should pass");

        let sessions = list_session_ids(&cfg).await;
        assert!(!sessions.contains(&"delete-me".to_string()));
    }

    #[tokio::test]
    async fn sessions_prune_enforces_safety_and_deletes_when_forced() {
        let (_dir, cfg) = sqlite_cfg("default-session");
        create_session(&cfg, "s1").await;
        create_session(&cfg, "s2").await;
        create_session(&cfg, "s3").await;

        let err = run_sessions_prune(&cfg, 1, false, false)
            .await
            .expect_err("prune without --force should fail");
        assert_eq!(categorize_error(&err), ErrorCategory::Input);

        run_sessions_prune(&cfg, 1, true, false)
            .await
            .expect("dry run should pass");
        let sessions_after_dry_run = list_session_ids(&cfg).await;
        assert_eq!(sessions_after_dry_run.len(), 3);

        run_sessions_prune(&cfg, 1, false, true)
            .await
            .expect("forced prune should pass");
        let sessions_after_force = list_session_ids(&cfg).await;
        assert_eq!(sessions_after_force.len(), 1);
    }

    #[tokio::test]
    async fn shared_memory_session_service_preserves_history_across_runner_rebuilds() {
        let cfg = base_cfg();
        let telemetry = test_telemetry(&cfg);
        let session_service: Arc<dyn SessionService> = Arc::new(InMemorySessionService::new());

        let runner_one = build_runner_with_session_service(
            build_single_agent(mock_model("first answer")).expect("agent should build"),
            &cfg,
            session_service.clone(),
            None,
        )
        .await
        .expect("runner should build");
        run_prompt(&runner_one, &cfg, "first prompt", &telemetry)
            .await
            .expect("first prompt should run");

        let runner_two = build_runner_with_session_service(
            build_single_agent(mock_model("second answer")).expect("agent should build"),
            &cfg,
            session_service.clone(),
            None,
        )
        .await
        .expect("second runner should build");
        run_prompt(&runner_two, &cfg, "second prompt", &telemetry)
            .await
            .expect("second prompt should run");

        let session = session_service
            .get(GetRequest {
                app_name: cfg.app_name.clone(),
                user_id: cfg.user_id.clone(),
                session_id: cfg.session_id.clone(),
                num_recent_events: None,
                after: None,
            })
            .await
            .expect("session should exist");

        assert!(
            session.events().len() >= 4,
            "expected in-memory session history to persist across runner rebuilds"
        );
    }

    #[tokio::test]
    async fn chat_switch_path_builds_runner_for_ollama_without_losing_session_service() {
        let mut cfg = base_cfg();
        cfg.provider = Provider::Ollama;
        cfg.model = Some("llama3.2".to_string());
        let session_service: Arc<dyn SessionService> = Arc::new(InMemorySessionService::new());
        let runtime_tools = ResolvedRuntimeTools {
            tools: build_builtin_tools(),
            mcp_tool_names: BTreeSet::new(),
        };
        let tool_confirmation = ToolConfirmationSettings::default();
        let telemetry = test_telemetry(&cfg);

        let (_runner, provider, model_name) = build_single_runner_for_chat(
            &cfg,
            session_service.clone(),
            &runtime_tools,
            &tool_confirmation,
            &telemetry,
        )
        .await
        .expect("chat runner should build for ollama");

        assert_eq!(provider, Provider::Ollama);
        assert_eq!(model_name, "llama3.2");
    }

// ---------------------------------------------------------------------------
// Tool policy: wildcard matching
// ---------------------------------------------------------------------------

#[test]
fn test_wildcard_exact_match() {
    assert!(matches_wildcard("fs_read", "fs_read"));
    assert!(!matches_wildcard("fs_read", "fs_write"));
}

#[test]
fn test_wildcard_star_suffix() {
    assert!(matches_wildcard("github_ops.*", "github_ops.issue_create"));
    assert!(matches_wildcard("github_ops.*", "github_ops.pr_create"));
    assert!(!matches_wildcard("github_ops.*", "fs_read"));
}

#[test]
fn test_wildcard_star_prefix() {
    assert!(matches_wildcard("*_create", "github_ops.issue_create"));
    assert!(!matches_wildcard("*_create", "github_ops.issue_update"));
}

#[test]
fn test_wildcard_star_middle() {
    assert!(matches_wildcard("execute_bash.*_rf", "execute_bash.rm_rf"));
    assert!(!matches_wildcard("execute_bash.*_rf", "execute_bash.rm_r"));
}

#[test]
fn test_wildcard_star_only() {
    assert!(matches_wildcard("*", "anything"));
    assert!(matches_wildcard("*", ""));
}

// ---------------------------------------------------------------------------
// Tool policy: filter_tools_by_policy
// ---------------------------------------------------------------------------

fn make_mock_tool(name: &str) -> Arc<dyn Tool> {
    Arc::new(crate::tool_policy::StubTool {
        tool_name: name.to_string(),
    })
}

#[test]
fn test_filter_allow_wildcard() {
    let tools = vec![
        make_mock_tool("fs_read"),
        make_mock_tool("fs_write"),
        make_mock_tool("github_ops.issue_create"),
    ];
    let allow = vec!["fs_*".to_string()];
    let deny = vec![];
    let filtered = filter_tools_by_policy(tools, &allow, &deny);
    let names: Vec<&str> = filtered.iter().map(|t| t.name()).collect();
    assert_eq!(names, vec!["fs_read", "fs_write"]);
}

#[test]
fn test_filter_deny_wildcard() {
    let tools = vec![
        make_mock_tool("fs_read"),
        make_mock_tool("fs_write"),
        make_mock_tool("execute_bash"),
    ];
    let allow = vec![];
    let deny = vec!["fs_*".to_string()];
    let filtered = filter_tools_by_policy(tools, &allow, &deny);
    let names: Vec<&str> = filtered.iter().map(|t| t.name()).collect();
    assert_eq!(names, vec!["execute_bash"]);
}

#[test]
fn test_filter_deny_overrides_allow() {
    let tools = vec![
        make_mock_tool("fs_read"),
        make_mock_tool("fs_write"),
        make_mock_tool("execute_bash"),
    ];
    let allow = vec!["*".to_string()];
    let deny = vec!["fs_write".to_string()];
    let filtered = filter_tools_by_policy(tools, &allow, &deny);
    let names: Vec<&str> = filtered.iter().map(|t| t.name()).collect();
    assert_eq!(names, vec!["fs_read", "execute_bash"]);
}

// ---------------------------------------------------------------------------
// Tool policy: alias resolution
// ---------------------------------------------------------------------------

#[test]
fn test_alias_renames_tool() {
    let tools = vec![make_mock_tool("search_docs")];
    let mut aliases = HashMap::new();
    aliases.insert("search_docs".to_string(), "doc_search".to_string());
    let aliased = apply_tool_aliases(tools, &aliases);
    assert_eq!(aliased[0].name(), "doc_search");
}

#[test]
fn test_alias_no_match_passes_through() {
    let tools = vec![make_mock_tool("fs_read")];
    let mut aliases = HashMap::new();
    aliases.insert("other_tool".to_string(), "renamed".to_string());
    let result = apply_tool_aliases(tools, &aliases);
    assert_eq!(result[0].name(), "fs_read");
}

#[test]
fn test_alias_plus_wildcard_deny() {
    let tools = vec![make_mock_tool("search_docs"), make_mock_tool("run_query")];
    let mut aliases = HashMap::new();
    aliases.insert("search_docs".to_string(), "safe_search".to_string());
    let aliased = apply_tool_aliases(tools, &aliases);
    // deny the aliased name
    let deny = vec!["safe_*".to_string()];
    let filtered = filter_tools_by_policy(aliased, &[], &deny);
    let names: Vec<&str> = filtered.iter().map(|t| t.name()).collect();
    assert_eq!(names, vec!["run_query"]);
}
