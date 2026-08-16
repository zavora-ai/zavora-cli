use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Duration;

use adk_rust::prelude::InMemoryArtifactService;
use adk_rust::prelude::*;
use adk_rust::{ToolConfirmationDecision, ToolConfirmationPolicy};
use adk_session::SessionService;
use anyhow::{Context, Result};
use serde_json::json;

use crate::config::RuntimeConfig;
use crate::provider::{resolve_model, resolve_planner_model};
use crate::session::{build_session_service, ensure_session_exists};
use crate::telemetry::TelemetrySink;

/// Descriptions for every agent the orchestrator may be told about.
///
/// The prompt section is rendered from this table filtered against what is
/// actually registered, so a conditionally attached agent — `search_agent` is
/// only present with `--provider gemini` — is simply absent from the prompt
/// rather than advertised and then stripped. Requirement 6.2, 6.5.
/// Agents exposed as real tools: the orchestrator calls them and gets a result
/// back, with control never leaving the orchestrator.
///
/// `time_agent` and `memory_agent` are `impl Tool` (`src/agents/tools.rs`) and
/// `plan_work` is a `BudgetedPlannerTool`, so "call as tools" is literally true
/// for these three and only these three.
const AGENT_TOOL_CATALOGUE: &[(&str, &str)] = &[
    (
        "time_agent",
        "Get current time, parse relative dates (for example \"next Friday\", \"in 2 days\")",
    ),
    (
        "memory_agent",
        "Recall or store USER preferences, decisions, and learnings (NOT general knowledge)",
    ),
    (
        "plan_work",
        "Produce a concise plan for complex or architectural work",
    ),
];

/// Specialists registered with `builder.sub_agent(...)`.
///
/// These are reached through ADK's `transfer_to_agent`, which **hands over
/// control** rather than returning a value. Describing them as callable tools
/// caused exactly the failure it sounds like: the orchestrator "called" one, ADK
/// turned that into a transfer, and the receiving specialist read the same
/// instruction and transferred onward — three models paid to pass a task around
/// without doing it. The framing below is deliberately different for that reason.
/// Requirement 13.2, 13.3.
const SUBAGENT_CATALOGUE: &[(&str, &str)] = &[
    ("search_agent", "News, current events, and web searches"),
    (
        "artifact_agent",
        "Documents, presentations, spreadsheets, PDFs, email, and work management",
    ),
    (
        "developer_agent",
        "Repository, implementation, testing, dependencies, CI/CD, and delivery",
    ),
    (
        "research_agent",
        "Source-grounded web and specialist research",
    ),
    (
        "operations_agent",
        "Devices, desktop automation, infrastructure, incidents, and business systems",
    ),
    (
        "reviewer_agent",
        "Independent correctness, safety, provenance, and acceptance review",
    ),
];

/// Test-only views of both catalogues.
#[cfg(test)]
pub const AGENT_CATALOGUE_FOR_TEST: &[(&str, &str)] = SUBAGENT_CATALOGUE;
#[cfg(test)]
pub const AGENT_TOOL_CATALOGUE_FOR_TEST: &[(&str, &str)] = AGENT_TOOL_CATALOGUE;
#[cfg(test)]
pub const SUBAGENT_CATALOGUE_FOR_TEST: &[(&str, &str)] = SUBAGENT_CATALOGUE;

pub const ORCHESTRATOR_INSTRUCTION: &str = "\
You are the orchestrator. You coordinate specialist agents to accomplish complex tasks.

RULES:
- For news/web searches: delegate to search_agent
- Delegate focused work to the matching specialist; do not send every task through a subagent
- For complex multi-file or architectural work: call plan_work once, then execute its plan
- memory_agent is ONLY for user preferences/decisions, NOT for facts or general knowledge
- For simple tasks, use your built-in tools directly
- Store only high-signal learnings: user preferences, decisions, patterns (not facts)
";

#[cfg(test)]
pub fn build_single_agent(model: Arc<dyn Llm>) -> Result<Arc<dyn Agent>> {
    let tools = crate::tools::build_builtin_tools();
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
             - Treat every tool result as authoritative. Never claim a file was created or changed \
             when the write or edit tool reported an error, denial, or timeout\n\
             - After a write or edit, read the affected file before reporting the change as complete\n\
             </tool_guidelines>\n\
             \n\
             <git_guidelines>\n\
             COMMIT DISCIPLINE:\n\
             - Make atomic commits: one logical change per commit. Don't bundle unrelated changes.\n\
             - Always verify the build passes (compile, tests) BEFORE committing.\n\
             - Use conventional commit prefixes: feat:, fix:, refactor:, docs:, test:, chore:\n\
             - Write a concise summary line (<72 chars). For complex changes, add a blank line \
             then a body explaining what and why.\n\
             - Stage only files that belong to the requested change. Preserve unrelated work.\n\
             - Commit or push only when the user explicitly asks for it.\n\
             \n\
             WORKFLOW:\n\
             - Check `git status` before starting work to understand the current state.\n\
             - Don't amend or force-push unless explicitly asked.\n\
             - Never amend, force-push, or change branches without explicit approval.\n\
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
             - Preserve existing tests and add focused tests when they are needed to verify the change\n\
             - Decline requests for malicious code\n\
             - When uncertain, ask for clarification rather than guessing\n\
             </rules>"
            ),
        ];
        let configured_servers = cfg
            .mcp_servers
            .iter()
            .filter(|server| server.enabled.unwrap_or(true))
            .map(|server| server.name.clone())
            .collect::<Vec<_>>();
        let connected_mcp_tools = tools
            .iter()
            .map(|tool| tool.name())
            .filter(|name| name.starts_with("mcp:"))
            .map(str::to_string)
            .collect::<Vec<_>>();
        sections.push(format!(
            "<runtime_capabilities>\n{}\n</runtime_capabilities>",
            crate::capabilities::format_prompt_capabilities(
                &configured_servers,
                &connected_mcp_tools,
            )
        ));
        match crate::skills::resolve_workspace_instructions() {
            Ok(instructions) if !instructions.content.is_empty() => sections.push(format!(
                "<workspace_instructions>\n{}\n</workspace_instructions>",
                instructions.content
            )),
            Ok(_) => {}
            Err(error) => tracing::warn!(%error, "project instruction loading unavailable"),
        }
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
    let capability_subagents = runtime_cfg
        .filter(|cfg| cfg.agent_name == "default")
        .map(|_| crate::agents::capability::build_specialist_agents(model.clone(), tools))
        .transpose()?
        .unwrap_or_default();

    // Property 3: a name the prompt advertises must be a name the runtime can
    // serve. The v2 prompt advertised three workflow agents that were never
    // registered, so every session paid tokens describing tools that could only
    // fail when called. Auditing here — after the tool surface and sub-agents
    // are both known — is the only point where both sides are visible.
    let mut advertised = tools
        .iter()
        .map(|tool| tool.name().to_string())
        .collect::<Vec<_>>();
    advertised.extend(
        [
            "time_agent",
            "memory_agent",
            "plan_work",
            "artifact_agent",
            "developer_agent",
            "research_agent",
            "operations_agent",
            "reviewer_agent",
        ]
        .into_iter()
        .map(str::to_string),
    );
    if search_subagent.is_some() {
        advertised.push("search_agent".to_string());
    }
    for specialist in &capability_subagents {
        advertised.push(specialist.name().to_string());
    }

    let surface = crate::prompt_surface::PromptSurface::from_names(advertised);

    // Two sections, because there are two mechanisms. Tools return a value and
    // keep control here; sub-agents take over the turn. Collapsing them into one
    // "call as tools" list made the orchestrator transfer instead of work.
    let tool_section = surface.render_section(AGENT_TOOL_CATALOGUE);
    let subagent_section = surface.render_section(SUBAGENT_CATALOGUE);

    let mut instruction = instruction;
    if !tool_section.is_empty() {
        instruction.push_str("\n\nAGENT TOOLS (call these; they return a result to you):\n");
        instruction.push_str(&tool_section);
    }
    if !subagent_section.is_empty() {
        instruction.push_str(
            "\n\nSPECIALISTS (transferring hands the whole turn to them; you do not get \
             control back):\n",
        );
        instruction.push_str(&subagent_section);
        instruction.push_str(
            "\nTransfer only when the specialist's domain is clearly the whole task. \
             If you transfer, you are done. If you receive a transfer, complete the work \
             yourself and answer the user — do not transfer again.\n",
        );
    }

    // Belt to that: anything still advertised and unregistered is real drift.
    let (instruction, phantoms) = surface.sanitize(&instruction);
    if !phantoms.is_empty() {
        // Loud on purpose: with the sections generated, a surviving phantom means
        // hand-written prose names a tool the runtime cannot serve.
        tracing::error!(
            count = phantoms.len(),
            phantoms = ?phantoms.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(),
            "system prompt advertised tools that are not registered; removed before the model saw them"
        );
    }

    let mut builder = LlmAgentBuilder::new("assistant")
        .description("General purpose engineering assistant")
        .instruction(instruction)
        .model(model)
        .tool_confirmation_policy(tool_confirmation_policy)
        .tool_timeout(tool_timeout)
        .tool_execution_strategy(adk_rust::ToolExecutionStrategy::Auto);

    if runtime_cfg.is_some() {
        builder = builder.toolset(Arc::new(crate::capabilities::CapabilityToolset::routed(
            "prompt-routed-capabilities",
            tools.to_vec(),
        )));
    } else {
        for tool in tools {
            builder = builder.tool(tool.clone());
        }
    }

    // Add search subagent if available
    if let Some(search_agent) = search_subagent {
        builder = builder.sub_agent(search_agent);
    }
    for specialist in capability_subagents {
        builder = builder.sub_agent(specialist);
    }

    if let Some(cfg) = runtime_cfg {
        match resolve_planner_model(cfg) {
            Ok((planner_model, provider, model_name)) => {
                let planner = crate::model_roles::build_workspace_planner_agent(planner_model)?;
                builder = builder.tool(Arc::new(crate::model_roles::BudgetedPlannerTool::new(
                    planner,
                    cfg.planner_call_budget,
                )));
                if let Some(telemetry) = telemetry {
                    telemetry.emit(
                        "planner.available",
                        json!({
                            "provider": format!("{provider:?}").to_ascii_lowercase(),
                            "model": model_name,
                            "call_budget": cfg.planner_call_budget,
                        }),
                    );
                }
            }
            Err(error) => {
                tracing::warn!(%error, "planner unavailable; worker will continue without plan_work")
            }
        }
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

// ---------------------------------------------------------------------------
// Old sub-agent code removed - replaced with new capability + workflow agents
// See src/agents/ for new architecture
// ---------------------------------------------------------------------------

// The tool surface and its single enforcement point live in `tool_surface`.
// Re-exported here so existing `crate::runner::` imports keep working.
pub use crate::tool_surface::{ResolvedRuntimeTools, ToolSurface, resolve_runtime_tools};

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
        .tools()
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

    // Wire shared memory singleton (initialized in main.rs)
    if let Some(mem) = crate::agents::memory::adapter() {
        builder = builder.memory_service(mem);
    }

    match crate::skills::load_workspace_skills() {
        Ok(index) if !index.is_empty() => {
            let injector = adk_skill::SkillInjector::from_index(
                index,
                adk_skill::SkillInjectorConfig::default(),
            );
            builder = builder.plugin_manager(Arc::new(injector.build_plugin_manager("skills")));
        }
        Ok(_) => {}
        Err(error) => tracing::warn!(%error, "skill injection unavailable"),
    }

    if let Some(cc) = compaction_config {
        builder = builder.compaction_config(cc);
    }

    let runner = builder.build().context("failed to build ADK runner")?;

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
        runtime_tools.tools(),
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
