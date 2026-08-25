use adk_rust::{ToolConfirmationDecision, ToolConfirmationPolicy};
use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;
use std::time::Duration;

use adk_rust::prelude::*;
use adk_session::*;
use async_trait::async_trait;
use serde_json::{Value, json};

/// A trivial `Tool` implementation used only in tests.
struct StubTool {
    tool_name: String,
}

#[async_trait]
impl Tool for StubTool {
    fn name(&self) -> &str {
        &self.tool_name
    }
    fn description(&self) -> &str {
        "stub tool for testing"
    }
    async fn execute(&self, _ctx: Arc<dyn ToolContext>, _args: Value) -> adk_rust::Result<Value> {
        Ok(Value::Null)
    }
}

use crate::chat::*;
use crate::cli::*;
use crate::config::*;
use crate::error::*;
use crate::eval::*;
use crate::guardrail::*;
use crate::mcp::*;
use crate::provider::*;
use crate::retrieval::*;
use crate::runner::*;
use crate::server::*;
use crate::session::*;
use crate::streaming::*;
use crate::telemetry::*;
use crate::test_support::base_cfg;
use crate::tool_policy::*;
use crate::tools::execute_bash::*;
use crate::tools::fs_read::*;
use crate::tools::fs_write::*;
use crate::tools::github_ops::*;
use crate::tools::*;
use crate::workflow::*;

use adk_rust::LlmResponse;
use adk_rust::model::MockLlm;
use tempfile::tempdir;

fn test_telemetry(cfg: &RuntimeConfig) -> TelemetrySink {
    TelemetrySink::new(cfg, "test".to_string())
}

fn mock_model(text: &str) -> Arc<dyn Llm> {
    Arc::new(
        MockLlm::new("mock").with_response(LlmResponse::new(Content::new("model").with_text(text))),
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
    ResolvedRuntimeTools::for_test(
        tool_names
            .iter()
            .map(|name| noop_tool(name))
            .collect::<Vec<_>>(),
        mcp_tool_names
            .iter()
            .map(|name| name.to_string())
            .collect::<BTreeSet<String>>(),
    )
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
        output_format: OutputFormat::Text,
        input_files: Vec::new(),
        stdin: false,
        no_stdin: false,
        always_approve: false,
        provider: Provider::Auto,
        model: None,
        worker_provider: None,
        worker_model: None,
        planner_provider: None,
        planner_model: None,
        planner_call_budget: None,
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
        command: Some(Commands::Doctor),
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
            limit: None,
            offset: None,
        })
        .await
        .expect("sessions should list")
        .into_iter()
        .map(|s| s.id().to_string())
        .collect::<Vec<String>>();
    sessions.sort();
    sessions
}

mod checkpoints;
mod compaction;
mod configuration_quality;
mod context_usage;
mod filesystem_egress;
mod hooks;
mod mcp_diagnostics;
mod retrieval_sessions;
mod runtime;
mod runtime_contracts;
mod theme;
mod todos;
mod tool_policy;
mod tools_integration;
