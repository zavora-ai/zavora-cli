/// Ralph sub-agent — wraps the Ralph autonomous development pipeline as an
/// adk-rust sub-agent so the LLM can route greenfield/multi-phase work via
/// `transfer_to_agent("ralph_agent")`.
///
/// Follows the same pattern as `search_agent` in `src/agents/search.rs`.
use adk_rust::prelude::*;
use anyhow::Result;
use async_trait::async_trait;
use serde_json::{Value, json};
use std::sync::Arc;

use crate::config::RuntimeConfig;
use crate::telemetry::TelemetrySink;

const RALPH_AGENT_INSTRUCTION: &str = r#"You are the Ralph autonomous development agent.

You handle greenfield project creation, multi-phase development, and large-scale scaffolding tasks.
When given a development task, use the run_ralph_pipeline tool to execute the full Ralph pipeline
(PRD → Architect → Implementation).

You should be used for:
- Building new projects or applications from scratch
- Multi-file scaffolding and project setup
- Multi-phase development work (requirements → design → implementation)
- Large-scale feature development that needs structured planning

You should NOT be used for:
- Quick bug fixes or targeted edits
- Simple questions or explanations
- Single-file changes
- Debugging existing code
"#;

/// Tool that invokes `run_ralph()` from within the ralph sub-agent.
pub struct RalphPipelineTool {
    runtime_config: Arc<RuntimeConfig>,
    telemetry: Arc<TelemetrySink>,
}

impl RalphPipelineTool {
    pub fn new(runtime_config: Arc<RuntimeConfig>, telemetry: Arc<TelemetrySink>) -> Self {
        Self {
            runtime_config,
            telemetry,
        }
    }
}

#[async_trait]
impl Tool for RalphPipelineTool {
    fn name(&self) -> &str {
        "run_ralph_pipeline"
    }

    fn description(&self) -> &str {
        "Execute the Ralph autonomous development pipeline (PRD → Architect → Implementation) \
         for a given development prompt."
    }

    fn parameters_schema(&self) -> Option<Value> {
        Some(json!({
            "type": "object",
            "properties": {
                "prompt": {
                    "type": "string",
                    "description": "The development task description"
                }
            },
            "required": ["prompt"]
        }))
    }

    async fn execute(&self, _ctx: Arc<dyn ToolContext>, args: Value) -> adk_rust::Result<Value> {
        let prompt = args["prompt"]
            .as_str()
            .unwrap_or_default()
            .to_string();

        self.telemetry.emit(
            "ralph_agent.tool.invoked",
            json!({ "prompt_length": prompt.len() }),
        );

        match crate::ralph::run_ralph(
            &self.runtime_config,
            prompt,
            None,
            false,
            None,
            &self.telemetry,
        )
        .await
        {
            Ok(()) => Ok(json!({ "status": "completed" })),
            Err(e) => Err(adk_rust::AdkError::tool(e.to_string())),
        }
    }
}

/// Build the Ralph sub-agent with the `run_ralph_pipeline` tool.
pub fn build_ralph_agent(
    model: Arc<dyn Llm>,
    runtime_config: Arc<RuntimeConfig>,
    telemetry: Arc<TelemetrySink>,
) -> Result<Arc<dyn Agent>> {
    let ralph_tool = RalphPipelineTool::new(runtime_config, telemetry);

    let agent = LlmAgentBuilder::new("ralph_agent")
        .description(
            "Ralph autonomous development pipeline for greenfield projects \
             and multi-phase development work",
        )
        .instruction(RALPH_AGENT_INSTRUCTION)
        .model(model)
        .tool(Arc::new(ralph_tool))
        .build()?;

    Ok(Arc::new(agent))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_name_is_run_ralph_pipeline() {
        let cfg = Arc::new(test_runtime_config());
        let telemetry = Arc::new(test_telemetry_sink(&cfg));
        let tool = RalphPipelineTool::new(cfg, telemetry);
        assert_eq!(tool.name(), "run_ralph_pipeline");
    }

    #[test]
    fn tool_description_is_non_empty() {
        let cfg = Arc::new(test_runtime_config());
        let telemetry = Arc::new(test_telemetry_sink(&cfg));
        let tool = RalphPipelineTool::new(cfg, telemetry);
        assert!(!tool.description().is_empty());
    }

    #[test]
    fn parameters_schema_contains_prompt_field() {
        let cfg = Arc::new(test_runtime_config());
        let telemetry = Arc::new(test_telemetry_sink(&cfg));
        let tool = RalphPipelineTool::new(cfg, telemetry);
        let schema = tool.parameters_schema().expect("schema should be Some");
        let props = &schema["properties"];
        assert!(props.get("prompt").is_some(), "schema must have prompt field");
        assert_eq!(props["prompt"]["type"], "string");
        let required = schema["required"].as_array().expect("required should be array");
        assert!(required.contains(&json!("prompt")));
    }

    // --- test helpers ---

    fn test_runtime_config() -> RuntimeConfig {
        RuntimeConfig {
            profile: "default".to_string(),
            config_path: String::new(),
            agent_name: "default".to_string(),
            agent_source: crate::config::AgentSource::Implicit,
            agent_description: None,
            agent_instruction: None,
            agent_resource_paths: Vec::new(),
            agent_allow_tools: Vec::new(),
            agent_deny_tools: Vec::new(),
            provider: crate::cli::Provider::Openai,
            model: Some("gpt-4".to_string()),
            api_key: Some("test-key".to_string()),
            ollama_host: None,
            app_name: "test".to_string(),
            user_id: "test-user".to_string(),
            session_id: "test-session".to_string(),
            session_backend: crate::cli::SessionBackend::Memory,
            session_db_url: String::new(),
            show_sensitive_config: false,
            retrieval_backend: crate::cli::RetrievalBackend::Disabled,
            retrieval_doc_path: None,
            retrieval_max_chunks: 3,
            retrieval_max_chars: 4000,
            retrieval_min_score: 1,
            tool_confirmation_mode: crate::cli::ToolConfirmationMode::McpOnly,
            require_confirm_tool: Vec::new(),
            approve_tool: Vec::new(),
            tool_timeout_secs: 45,
            tool_retry_attempts: 2,
            tool_retry_delay_ms: 500,
            telemetry_enabled: false,
            telemetry_path: "/tmp/test-telemetry.jsonl".to_string(),
            guardrail_input_mode: crate::cli::GuardrailMode::Disabled,
            guardrail_output_mode: crate::cli::GuardrailMode::Disabled,
            guardrail_terms: Vec::new(),
            guardrail_redact_replacement: "[REDACTED]".to_string(),
            mcp_servers: Vec::new(),
            permission_rules: Default::default(),
            max_prompt_chars: 32_000,
            server_runner_cache_max: 64,
            auto_compact_enabled: true,
            compact_interval: 10,
            compact_overlap: 2,
            compaction_threshold: 0.75,
            compaction_target: 0.10,
        }
    }

    fn test_telemetry_sink(cfg: &RuntimeConfig) -> TelemetrySink {
        TelemetrySink::new(cfg, "test".to_string())
    }
}
