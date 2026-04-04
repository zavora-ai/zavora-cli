use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Duration;

use adk_rust::prelude::InMemoryArtifactService;
use adk_rust::prelude::*;
use adk_rust::{ToolConfirmationDecision, ToolConfirmationPolicy};
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
use crate::tools::{

    build_builtin_tools,
};

const ORCHESTRATOR_INSTRUCTION: &str = "\
You are the orchestrator. You coordinate specialist agents to accomplish complex tasks.

CAPABILITY AGENTS (call as tools when you need their unique skills):
- time_agent: Get current time, parse relative dates (\"next Friday\", \"in 2 days\")
- memory_agent: Recall/store USER preferences, decisions, and learnings (NOT for general knowledge)

SUBAGENTS (automatically available when conditions met):
- search_agent: For news, current events, and web searches (enabled only with --provider gemini)
- ralph_agent: For greenfield projects and multi-phase development (enabled only in agent mode)

WORKFLOW AGENTS (use for complex multi-step work):
- file_search_agent: Comprehensive file discovery with saturation detection
- sequential_agent: Create plans and execute steps with progress tracking
- quality_agent: Verify work against acceptance criteria

RULES:
- For news/web searches: delegate to search_agent
- For greenfield projects, multi-file scaffolding, or multi-phase development: delegate to ralph_agent (agent mode only)
- memory_agent is ONLY for user preferences/decisions, NOT for facts or general knowledge
- For simple tasks, use your built-in tools directly
- For complex multi-step tasks, use sequential_agent
- Store only high-signal learnings: user preferences, decisions, patterns (not facts)
";

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
    build_single_agent_with_tools_and_telemetry(
        model,
        tools,
        tool_confirmation_policy,
        tool_timeout,
        runtime_cfg,
        None,
    )
}

pub fn build_single_agent_with_tools_and_telemetry(
    model: Arc<dyn Llm>,
    tools: &[Arc<dyn Tool>],
    tool_confirmation_policy: ToolConfirmationPolicy,
    tool_timeout: Duration,
    runtime_cfg: Option<&RuntimeConfig>,
    telemetry: Option<&TelemetrySink>,
) -> Result<Arc<dyn Agent>> {
    let instruction = if let Some(cfg) = runtime_cfg {
        let os_name = std::env::consts::OS;
        let cwd = std::env::current_dir()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|_| ".".to_string());
        let shell = std::env::var("SHELL").unwrap_or_else(|_| "sh".to_string());

        let mut sections = vec![
            format!(
                "You are Zavora, an AI assistant in the user's terminal. You help with coding, \
                 debugging, system administration, writing, analysis, and any professional task.\n\
                 \n\
                 <system_context>\n\
                 - Operating System: {os_name}\n\
                 - Current Directory: {cwd}\n\
                 - Shell: {shell}\n\
                 </system_context>\n\
                 \n\
                 <operational_directives>\n\
                 EXECUTE IMMEDIATELY. When the user asks you to do something, do it. Don't narrate \
                 what you would do — use your tools and produce the result.\n\
                 OUTPUT FIRST. Lead with code, results, or actions. Explanations come after, and only \
                 if needed.\n\
                 ZERO FLUFF. No philosophical preambles, no unsolicited advice, no filler. Every \
             sentence must earn its place.\n\
             STAY FOCUSED. Answer what was asked. Don't wander into tangents or related topics \
             unless directly relevant.\n\
             </operational_directives>\n\
             "
            ),
            ORCHESTRATOR_INSTRUCTION.to_string(),
            format!(
                "\n\
             <tone>\n\
             You talk like a human, not like a bot. You are conversational and natural.\n\
             - Mirror the user's style: short question gets a short answer, detailed question \
             gets a detailed answer\n\
             - NEVER present menus, numbered option lists, or \"quick options\" unless the user \
             asks for choices\n\
             - NEVER start responses with flattery (\"Great question!\", \"That's a good idea!\")\n\
             - For greetings like \"hello\" or \"hi\", respond briefly and naturally — don't list \
             capabilities or suggest actions\n\
             - When you don't know something, say so directly\n\
             - Use neutral acknowledgments: \"Let me look at that\" not \"Absolutely! I'd love to \
             help!\"\n\
             </tone>\n\
             \n\
             <coding_standards>\n\
             PROJECT AWARENESS: Before writing code, understand the project's existing patterns, \
             dependencies, and conventions. Use what's already there.\n\
             - If the project uses a library or framework, USE IT. Don't build custom solutions \
             when the existing stack provides them.\n\
             - Match the project's code style, naming conventions, and file organization.\n\
             - Every line of code must have a purpose. If it doesn't contribute to the solution, \
             remove it.\n\
             MINIMAL CHANGES: Write the absolute minimum code needed. Don't refactor surrounding \
             code unless asked. Don't add features that weren't requested.\n\
             VERIFY: Read files before modifying them. Check that builds pass after changes. \
             Don't assume — verify.\n\
             </coding_standards>\n\
             \n\
             <tool_guidelines>\n\
             - Use fs_read to examine files before modifying them\n\
             - Use file_edit for surgical text replacements in existing files (preferred over fs_write for edits)\n\
             - Use fs_write only for creating new files or full rewrites\n\
             - Use glob to find files by name pattern (e.g. '**/*.rs') — faster and safer than shell find\n\
             - Use grep to search file contents by regex — faster and safer than shell grep\n\
             - Use web_fetch to read web pages or API docs (requires confirmation since it makes network requests)\n\
             - When editing files, show only the minimal diff needed\n\
             - For shell commands, prefer simple composable commands over complex one-liners\n\
             - Consider the operating system when providing paths and commands\n\
             - Be aware of the current working directory for relative paths\n\
             - After making code changes, compile/build to verify they work\n\
             </tool_guidelines>\n\
             \n\
             <git_guidelines>\n\
             COMMIT DISCIPLINE:\n\
             - Make atomic commits: one logical change per commit. Don't bundle unrelated changes.\n\
             - Always verify the build passes (compile, tests) BEFORE committing.\n\
             - Use conventional commit prefixes: feat:, fix:, refactor:, docs:, test:, chore:\n\
             - Write a concise summary line (<72 chars). For complex changes, add a blank line \
             then a body explaining what and why.\n\
             - Stage with `git add -A` unless selectively staging specific files.\n\
             - Push after committing unless the user says otherwise.\n\
             \n\
             WORKFLOW:\n\
             - Check `git status` before starting work to understand the current state.\n\
             - Don't amend or force-push unless explicitly asked.\n\
             - When making multiple related changes, commit after each logical step — not all \
             at the end.\n\
             - If a build or test fails after changes, fix it before committing.\n\
             </git_guidelines>\n\
             \n\
             <response_format>\n\
             FOR QUICK TASKS: Just do it. Minimal or no explanation.\n\
             FOR CODE CHANGES: Brief rationale (1-2 sentences), then the code.\n\
             FOR COMPLEX TASKS: Break into steps, execute each one, report results.\n\
             FOR ANALYSIS/REVIEW: Be thorough — examine deeply, consider edge cases, provide \
             actionable recommendations.\n\
             AFTER TOOL USE: When you've already written files or executed commands via tools, \
             do NOT repeat the file contents or command output in your response. The user already \
             saw the diffs and results. Just summarize what was done in 1-2 sentences.\n\
             ALWAYS: Use markdown code blocks with language tags. Don't use headers unless \
             multi-step. Don't bold excessively. Bullet points only for genuinely parallel items.\n\
             </response_format>\n\
             \n\
             <rules>\n\
             - Never include secrets or API keys in code unless explicitly asked\n\
             - Substitute PII with generic placeholders\n\
             - Do not modify or remove tests unless explicitly requested\n\
             - Do not add tests unless explicitly requested\n\
             - Decline requests for malicious code\n\
             - When uncertain, ask for clarification rather than guessing\n\
             </rules>"
            ),
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
        "You are Zavora, an AI assistant in the user's terminal. Be concise and direct. \
         Prioritize actionable output. When planning work, prefer release-oriented increments."
            .to_string()
    };

    // Intentional: search sub-agent is only enabled when the invocation explicitly
    // runs with --provider gemini. Auto-detected provider mode does not attach it.
    let search_subagent = build_search_subagent_for_provider(runtime_cfg, model.clone());

    // Ralph sub-agent is only attached when agent mode is active.
    let ralph_subagent =
        build_ralph_subagent_if_agent_mode(runtime_cfg, model.clone(), telemetry);

    let mut builder = LlmAgentBuilder::new("assistant")
        .description("General purpose engineering assistant")
        .instruction(instruction)
        .model(model)
        .tool_confirmation_policy(tool_confirmation_policy)
        .tool_timeout(tool_timeout)
        .tool_execution_strategy(adk_rust::ToolExecutionStrategy::Auto)
        .before_model_callback(Box::new(|_ctx, mut request| {
            Box::pin(async move {
                // Fix tool response roles: conversation_history() maps all non-user
                // events to "model", but tool responses must be "function" for OpenAI.
                for content in &mut request.contents {
                    if content.role == "model"
                        && content
                            .parts
                            .iter()
                            .any(|p| matches!(p, adk_rust::prelude::Part::FunctionResponse { .. }))
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

    // Add search subagent if available
    if let Some(search_agent) = search_subagent {
        builder = builder.sub_agent(search_agent);
    }

    // Add ralph subagent if available (agent mode only)
    if let Some(ralph_agent) = ralph_subagent {
        builder = builder.sub_agent(ralph_agent);
    }

    Ok(Arc::new(builder.build()?))
}

fn build_search_subagent_for_provider(
    runtime_cfg: Option<&RuntimeConfig>,
    model: Arc<dyn Llm>,
) -> Option<Arc<dyn Agent>> {
    let cfg = runtime_cfg?;
    if cfg.provider != crate::cli::Provider::Gemini {
        return None;
    }

    match crate::agents::search::build_search_agent(model) {
        Ok(agent) => Some(agent),
        Err(err) => {
            tracing::warn!("failed to build search sub-agent: {}", err);
            None
        }
    }
}

fn build_ralph_subagent_if_agent_mode(
    runtime_cfg: Option<&RuntimeConfig>,
    model: Arc<dyn Llm>,
    telemetry: Option<&TelemetrySink>,
) -> Option<Arc<dyn Agent>> {
    use crate::tools::confirming::is_agent_mode;

    if !is_agent_mode() {
        return None;
    }

    let cfg = runtime_cfg?;
    let telemetry = telemetry?;

    match crate::agents::ralph_agent::build_ralph_agent(
        model,
        Arc::new(cfg.clone()),
        Arc::new(telemetry.clone()),
    ) {
        Ok(agent) => Some(agent),
        Err(err) => {
            tracing::warn!("failed to build ralph sub-agent: {}", err);
            None
        }
    }
}

// ---------------------------------------------------------------------------
// Old sub-agent code removed - replaced with new capability + workflow agents
// See src/agents/ for new architecture
// ---------------------------------------------------------------------------

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
    use crate::tool_policy::is_read_only_tool;

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

    // Build effective permission rules: profile rules + backward-compat mapping
    let rules = &cfg.permission_rules;

    // Legacy approve_tool → always_allow, require_confirm_tool → always_ask
    let mut effective_allow: Vec<crate::tool_policy::ToolPattern> = rules.always_allow.clone();
    let effective_deny: Vec<crate::tool_policy::ToolPattern> = rules.always_deny.clone();
    let mut effective_ask: Vec<crate::tool_policy::ToolPattern> = rules.always_ask.clone();

    for name in &cfg.approve_tool {
        let trimmed = name.trim();
        if !trimmed.is_empty() {
            effective_allow.push(crate::tool_policy::ToolPattern(trimmed.to_string()));
        }
    }
    for name in &cfg.require_confirm_tool {
        let trimmed = name.trim();
        if !trimmed.is_empty() {
            effective_ask.push(crate::tool_policy::ToolPattern(trimmed.to_string()));
        }
    }

    let effective_rules = crate::tool_policy::PermissionRules {
        always_allow: effective_allow,
        always_deny: effective_deny,
        always_ask: effective_ask,
    };

    // Determine wrapping per tool using layered rules
    tools = tools
        .into_iter()
        .map(|tool| {
            let name = tool.name();
            let decision = effective_rules.evaluate(name, None);

            match decision {
                crate::tool_policy::PermissionDecision::Allow => {
                    // Explicitly allowed — no confirmation, but show display for reads
                    if is_read_only_tool(name) {
                        ConfirmingTool::wrap_display_only(tool)
                    } else {
                        tool
                    }
                }
                crate::tool_policy::PermissionDecision::Deny => {
                    // Denied tools are already filtered by filter_tools_by_policy,
                    // but if a deny rule targets content patterns, the tool stays
                    // and ConfirmingTool handles per-call denial at runtime.
                    ConfirmingTool::wrap(tool)
                }
                crate::tool_policy::PermissionDecision::Ask => {
                    ConfirmingTool::wrap(tool)
                }
                crate::tool_policy::PermissionDecision::NoMatch => {
                    // Default behavior: read-only tools auto-approve (display-only),
                    // guarded built-ins and MCP tools require confirmation
                    if is_read_only_tool(name) {
                        ConfirmingTool::wrap_display_only(tool)
                    } else {
                        match cfg.tool_confirmation_mode {
                            ToolConfirmationMode::Always => ConfirmingTool::wrap(tool),
                            ToolConfirmationMode::McpOnly => {
                                if discovered_mcp_tool_names.contains(name)
                                    || matches!(name, "fs_write" | "file_edit" | "execute_bash" | "github_ops")
                                {
                                    ConfirmingTool::wrap(tool)
                                } else {
                                    tool
                                }
                            }
                            ToolConfirmationMode::Never => {
                                // Still wrap guarded built-ins
                                if matches!(name, "fs_write" | "file_edit" | "execute_bash" | "github_ops") {
                                    ConfirmingTool::wrap(tool)
                                } else {
                                    tool
                                }
                            }
                        }
                    }
                }
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

    // Add tool_search if total tools exceed threshold (gives LLM discovery for large tool sets)
    if tools.len() > 15 {
        let all_tools_for_search = tools.clone();
        let search_tool = FunctionTool::new(
            "tool_search",
            "Search available tools by keyword. Use when you need a tool that isn't in your current set. \
             Args: query (required, space-separated keywords to match against tool names and descriptions). \
             Returns matching tool names, descriptions, and parameter schemas.",
            move |_ctx, args| {
                let tools_ref = all_tools_for_search.clone();
                async move {
                    let query = args.get("query").and_then(serde_json::Value::as_str).unwrap_or("");
                    Ok(crate::tools::tool_search::tool_search_response(query, &tools_ref))
                }
            },
        )
        .with_read_only(true)
        .with_concurrency_safe(true);
        tools.push(Arc::new(search_tool));
    }

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

    let mut builder = Runner::builder()
        .app_name(cfg.app_name.clone())
        .agent(agent)
        .session_service(session_service)
        .artifact_service(artifact_service)
        .run_config(run_config.unwrap_or_default());

    if let Some(cc) = compaction_config {
        builder = builder.compaction_config(cc);
    }

    let runner = builder.build()
        .context("failed to build ADK runner")?;

    // Auto-discover and inject skills from .skills/ directory
    let runner = if std::path::Path::new(".skills").is_dir() {
        match runner.with_auto_skills(".", adk_skill::SkillInjectorConfig::default()) {
            Ok(r) => { tracing::info!("Skills loaded from .skills/"); r }
            Err(e) => { tracing::warn!("Skills parse error: {e}"); return Err(anyhow::anyhow!("skill error: {e}")); }
        }
    } else {
        runner
    };

    Ok(runner)
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
    let agent = build_single_agent_with_tools_and_telemetry(
        model,
        &runtime_tools.tools,
        tool_confirmation.policy.clone(),
        Duration::from_secs(cfg.tool_timeout_secs),
        Some(cfg),
        Some(telemetry),
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
