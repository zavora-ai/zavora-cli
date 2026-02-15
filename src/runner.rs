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
use crate::tools::{build_builtin_tools, FS_READ_TOOL_NAME, FS_WRITE_TOOL_NAME, EXECUTE_BASH_TOOL_NAME, GITHUB_OPS_TOOL_NAME};

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
        let os_name = std::env::consts::OS;
        let cwd = std::env::current_dir()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|_| ".".to_string());
        let shell = std::env::var("SHELL").unwrap_or_else(|_| "sh".to_string());

        let mut sections = vec![format!(
            "You are Zavora, an AI assistant in the user's terminal. You help with coding, \
             debugging, system administration, writing, analysis, and any professional task.\n\
             \n\
             <system_context>\n\
             - Operating System: {os_name}\n\
             - Current Directory: {cwd}\n\
             - Shell: {shell}\n\
             </system_context>\n\
             \n\
             <capabilities>\n\
             - Read files and directories to understand codebases\n\
             - Write and edit files (create, overwrite, append, patch)\n\
             - Execute shell commands on the user's system\n\
             - Manage GitHub issues and PRs via gh CLI\n\
             - Create and track task lists\n\
             </capabilities>\n\
             \n\
             <tool_guidelines>\n\
             - Use fs_read to examine files before modifying them\n\
             - When editing files, show only the minimal change needed\n\
             - For shell commands, prefer simple composable commands over complex one-liners\n\
             - Consider the operating system when providing paths and commands\n\
             - Be aware of the current working directory for relative paths\n\
             </tool_guidelines>\n\
             \n\
             <response_style>\n\
             - Be concise and direct. Skip flattery and filler.\n\
             - Prioritize actionable output over explanations\n\
             - Use code blocks with language tags for code snippets\n\
             - Use bullet points for lists\n\
             - When planning work, prefer release-oriented increments\n\
             - Provide complete, working solutions when possible\n\
             </response_style>\n\
             \n\
             <rules>\n\
             - Never include secrets or API keys in code unless explicitly asked\n\
             - Substitute PII with generic placeholders\n\
             - Do not modify or remove tests unless explicitly requested\n\
             - Do not add tests unless explicitly requested\n\
             - Decline requests for malicious code\n\
             - When uncertain, ask for clarification rather than guessing\n\
             </rules>"
        )];
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
        "You are Zavora, an AI assistant in the user's terminal. Be concise and direct. \
         Prioritize actionable output. When planning work, prefer release-oriented increments."
            .to_string()
    };

    let mut builder = LlmAgentBuilder::new("assistant")
        .description("General purpose engineering assistant")
        .instruction(instruction)
        .model(model)
        .tool_confirmation_policy(tool_confirmation_policy)
        .tool_timeout(tool_timeout)
        .before_model_callback(Box::new(|_ctx, mut request| {
            Box::pin(async move {
                // Fix tool response roles: conversation_history() maps all non-user
                // events to "model", but tool responses must be "function" for OpenAI.
                for content in &mut request.contents {
                    if content.role == "model"
                        && content.parts.iter().any(|p| {
                            matches!(p, adk_rust::prelude::Part::FunctionResponse { .. })
                        })
                    {
                        content.role = "function".to_string();
                    }
                }
                Ok(adk_rust::prelude::BeforeModelResult::Continue(request))
            })
        }));

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
    // Confirmation is now handled by ConfirmingTool wrappers applied in
    // resolve_runtime_tools(). The ADK-level policy is always Never.
    let available_tool_names = runtime_tools
        .tools
        .iter()
        .map(|tool| tool.name().to_string())
        .collect::<BTreeSet<String>>();

    let mut approved_tools = BTreeSet::<String>::new();
    for tool_name in &cfg.approve_tool {
        let trimmed = tool_name.trim();
        if !trimmed.is_empty() && available_tool_names.contains(trimmed) {
            approved_tools.insert(trimmed.to_string());
        }
    }

    let mut run_config = RunConfig::default();
    for tool_name in &approved_tools {
        run_config
            .tool_confirmation_decisions
            .insert(tool_name.clone(), ToolConfirmationDecision::Approve);
    }

    ToolConfirmationSettings {
        policy: ToolConfirmationPolicy::Never,
        run_config,
    }
}

pub async fn resolve_runtime_tools(cfg: &RuntimeConfig) -> ResolvedRuntimeTools {
    use crate::tools::confirming::ConfirmingTool;

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

    // Determine which tools need interactive confirmation
    let approved: BTreeSet<String> = cfg.approve_tool.iter().map(|s| s.trim().to_string()).collect();
    let mut confirm_names = BTreeSet::<String>::new();

    // Guarded built-in tools always require confirmation (unless pre-approved)
    for name in [FS_WRITE_TOOL_NAME, EXECUTE_BASH_TOOL_NAME, GITHUB_OPS_TOOL_NAME] {
        if !approved.contains(name) {
            confirm_names.insert(name.to_string());
        }
    }

    // fs_read is display-only (shows path, auto-approves) unless explicitly pre-approved
    let mut display_only_names = BTreeSet::<String>::new();
    if !approved.contains(FS_READ_TOOL_NAME) {
        display_only_names.insert(FS_READ_TOOL_NAME.to_string());
    }

    match cfg.tool_confirmation_mode {
        ToolConfirmationMode::Always => {
            for tool in &tools {
                if !approved.contains(tool.name()) {
                    confirm_names.insert(tool.name().to_string());
                }
            }
        }
        ToolConfirmationMode::McpOnly => {
            for name in &discovered_mcp_tool_names {
                if !approved.contains(name.as_str()) {
                    confirm_names.insert(name.clone());
                }
            }
        }
        ToolConfirmationMode::Never => {
            // Only guarded built-ins (already added above)
        }
    }

    for name in &cfg.require_confirm_tool {
        let trimmed = name.trim();
        if !trimmed.is_empty() && !approved.contains(trimmed) {
            confirm_names.insert(trimmed.to_string());
        }
    }

    // Wrap tools that need confirmation or display-only
    tools = tools
        .into_iter()
        .map(|tool| {
            if confirm_names.contains(tool.name()) {
                ConfirmingTool::wrap(tool)
            } else if display_only_names.contains(tool.name()) {
                ConfirmingTool::wrap_display_only(tool)
            } else {
                tool
            }
        })
        .collect();

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

    let compaction_config = if cfg.auto_compact_enabled {
        Some(crate::compact::build_compaction_config(
            cfg.compact_interval,
            cfg.compact_overlap,
        ))
    } else {
        None
    };

    Runner::new(RunnerConfig {
        app_name: cfg.app_name.clone(),
        agent,
        session_service,
        artifact_service: Some(artifact_service),
        memory_service: None,
        plugin_manager: None,
        run_config,
        compaction_config,
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
