use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Duration;

use adk_rust::prelude::*;
use adk_rust::{ToolConfirmationDecision, ToolConfirmationPolicy};
use adk_rust::prelude::InMemoryArtifactService;
use adk_session::SessionService;
use anyhow::{Context, Result};
use serde_json::json;

use crate::cli::ToolConfirmationMode;
use crate::config::RuntimeConfig;
use crate::mcp::discover_mcp_tools;
use crate::provider::resolve_model;
use crate::session::{build_session_service, ensure_session_exists};
use crate::telemetry::TelemetrySink;
use crate::tool_policy::filter_tools_by_policy;
use crate::tools::{build_builtin_tools, FS_WRITE_TOOL_NAME, EXECUTE_BASH_TOOL_NAME, GITHUB_OPS_TOOL_NAME};

#[cfg(test)]
pub fn build_single_agent(model: Arc<dyn Llm>) -> Result<Arc<dyn Agent>> {
    let tools = build_builtin_tools();
    build_single_agent_with_tools(
        model,
        &tools,
        ToolConfirmationPolicy::Never,
        Duration::from_secs(45),
        None,
    )
}

pub fn build_single_agent_with_tools(
    model: Arc<dyn Llm>,
    tools: &[Arc<dyn Tool>],
    tool_confirmation_policy: ToolConfirmationPolicy,
    tool_timeout: Duration,
    runtime_cfg: Option<&RuntimeConfig>,
) -> Result<Arc<dyn Agent>> {
    let instruction = if let Some(cfg) = runtime_cfg {
        let mut sections = vec![
            "You are a pragmatic AI engineer. Prioritize direct, actionable output, and when \
             planning work always prefer release-oriented increments."
                .to_string(),
        ];
        if let Some(agent_instruction) = cfg
            .agent_instruction
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            sections.push(format!("Agent-specific instruction:\n{agent_instruction}"));
        }
        if !cfg.agent_resource_paths.is_empty() {
            sections.push(format!(
                "Agent resource hints:\n{}",
                cfg.agent_resource_paths
                    .iter()
                    .map(|path| format!("- {}", path))
                    .collect::<Vec<String>>()
                    .join("\n")
            ));
        }
        sections.join("\n\n")
    } else {
        "You are a pragmatic AI engineer. Prioritize direct, actionable output, and when \
         planning work always prefer release-oriented increments."
            .to_string()
    };

    let mut builder = LlmAgentBuilder::new("assistant")
        .description("General purpose engineering assistant")
        .instruction(instruction)
        .model(model)
        .tool_confirmation_policy(tool_confirmation_policy)
        .tool_timeout(tool_timeout);

    for tool in tools {
        builder = builder.tool(tool.clone());
    }

    Ok(Arc::new(builder.build()?))
}

#[derive(Clone)]
pub struct ResolvedRuntimeTools {
    pub tools: Vec<Arc<dyn Tool>>,
    pub mcp_tool_names: BTreeSet<String>,
}

#[derive(Debug, Clone)]
pub struct ToolConfirmationSettings {
    pub policy: ToolConfirmationPolicy,
    pub run_config: RunConfig,
}

impl Default for ToolConfirmationSettings {
    fn default() -> Self {
        Self {
            policy: ToolConfirmationPolicy::Never,
            run_config: RunConfig::default(),
        }
    }
}

pub fn resolve_tool_confirmation_settings(
    cfg: &RuntimeConfig,
    runtime_tools: &ResolvedRuntimeTools,
) -> ToolConfirmationSettings {
    let available_tool_names = runtime_tools
        .tools
        .iter()
        .map(|tool| tool.name().to_string())
        .collect::<BTreeSet<String>>();

    let mut required_tools = BTreeSet::<String>::new();
    match cfg.tool_confirmation_mode {
        ToolConfirmationMode::Never => {}
        ToolConfirmationMode::McpOnly => {
            required_tools.extend(runtime_tools.mcp_tool_names.iter().cloned());
        }
        ToolConfirmationMode::Always => {
            required_tools.extend(available_tool_names.iter().cloned());
        }
    }

    for guarded_tool in [
        FS_WRITE_TOOL_NAME,
        EXECUTE_BASH_TOOL_NAME,
        GITHUB_OPS_TOOL_NAME,
    ] {
        if available_tool_names.contains(guarded_tool) {
            required_tools.insert(guarded_tool.to_string());
        }
    }

    for tool_name in &cfg.require_confirm_tool {
        let trimmed = tool_name.trim();
        if trimmed.is_empty() {
            continue;
        }
        if available_tool_names.contains(trimmed) {
            required_tools.insert(trimmed.to_string());
        } else {
            tracing::warn!(
                tool = trimmed,
                "tool in require_confirm_tool is not present in runtime toolset; ignoring"
            );
        }
    }

    let mut approved_tools = BTreeSet::<String>::new();
    for tool_name in &cfg.approve_tool {
        let trimmed = tool_name.trim();
        if trimmed.is_empty() {
            continue;
        }
        if available_tool_names.contains(trimmed) {
            approved_tools.insert(trimmed.to_string());
        } else {
            tracing::warn!(
                tool = trimmed,
                "tool in approve_tool is not present in runtime toolset; ignoring"
            );
        }
    }

    let policy = if required_tools.is_empty() {
        ToolConfirmationPolicy::Never
    } else {
        ToolConfirmationPolicy::PerTool(required_tools.clone())
    };

    let mut run_config = RunConfig::default();
    for tool_name in &required_tools {
        let decision = if approved_tools.contains(tool_name) {
            ToolConfirmationDecision::Approve
        } else {
            ToolConfirmationDecision::Deny
        };
        run_config
            .tool_confirmation_decisions
            .insert(tool_name.clone(), decision);
    }

    tracing::info!(
        mode = ?cfg.tool_confirmation_mode,
        available = available_tool_names.len(),
        required = required_tools.len(),
        approved = approved_tools.len(),
        denied = required_tools.len().saturating_sub(approved_tools.len()),
        "Resolved tool confirmation settings"
    );

    ToolConfirmationSettings { policy, run_config }
}

pub async fn resolve_runtime_tools(cfg: &RuntimeConfig) -> ResolvedRuntimeTools {
    let mut tools = build_builtin_tools();
    let built_in_count = tools.len();
    let mut mcp_tools = discover_mcp_tools(cfg).await;
    let mcp_count = mcp_tools.len();
    let discovered_mcp_tool_names = mcp_tools
        .iter()
        .map(|tool| tool.name().to_string())
        .collect::<BTreeSet<String>>();
    tools.append(&mut mcp_tools);

    tools = filter_tools_by_policy(tools, &cfg.agent_allow_tools, &cfg.agent_deny_tools);

    let mcp_tool_names = tools
        .iter()
        .map(|tool| tool.name().to_string())
        .filter(|name| discovered_mcp_tool_names.contains(name))
        .collect::<BTreeSet<String>>();

    tracing::info!(
        built_in_tools = built_in_count,
        mcp_tools = mcp_count,
        total_tools = tools.len(),
        agent_allow_tools = cfg.agent_allow_tools.len(),
        agent_deny_tools = cfg.agent_deny_tools.len(),
        "Resolved runtime toolset"
    );

    ResolvedRuntimeTools {
        tools,
        mcp_tool_names,
    }
}

pub async fn build_runner(agent: Arc<dyn Agent>, cfg: &RuntimeConfig) -> Result<Runner> {
    build_runner_with_run_config(agent, cfg, None).await
}

pub async fn build_runner_with_run_config(
    agent: Arc<dyn Agent>,
    cfg: &RuntimeConfig,
    run_config: Option<RunConfig>,
) -> Result<Runner> {
    let session_service = build_session_service(cfg).await?;
    build_runner_with_session_service(agent, cfg, session_service, run_config).await
}

pub async fn build_runner_with_session_service(
    agent: Arc<dyn Agent>,
    cfg: &RuntimeConfig,
    session_service: Arc<dyn SessionService>,
    run_config: Option<RunConfig>,
) -> Result<Runner> {
    ensure_session_exists(&session_service, cfg).await?;
    let artifact_service = Arc::new(InMemoryArtifactService::new());

    Runner::new(RunnerConfig {
        app_name: cfg.app_name.clone(),
        agent,
        session_service,
        artifact_service: Some(artifact_service),
        memory_service: None,
        plugin_manager: None,
        run_config,
        compaction_config: None,
    })
    .context("failed to build ADK runner")
}

pub async fn build_single_runner_for_chat(
    cfg: &RuntimeConfig,
    session_service: Arc<dyn SessionService>,
    runtime_tools: &ResolvedRuntimeTools,
    tool_confirmation: &ToolConfirmationSettings,
    telemetry: &TelemetrySink,
) -> Result<(Runner, crate::cli::Provider, String)> {
    let (model, resolved_provider, model_name) = resolve_model(cfg)?;
    telemetry.emit(
        "model.resolved",
        json!({
            "provider": format!("{:?}", resolved_provider).to_ascii_lowercase(),
            "model": model_name.clone(),
            "path": "chat"
        }),
    );
    let agent = build_single_agent_with_tools(
        model,
        &runtime_tools.tools,
        tool_confirmation.policy.clone(),
        Duration::from_secs(cfg.tool_timeout_secs),
        Some(cfg),
    )?;
    let runner = build_runner_with_session_service(
        agent,
        cfg,
        session_service,
        Some(tool_confirmation.run_config.clone()),
    )
    .await?;
    Ok((runner, resolved_provider, model_name))
}
