use adk_rust::ToolConfirmationDecision;
use std::io::{self, Write};
use std::sync::Arc;

use adk_rust::prelude::*;

use adk_session::SessionService;
use anyhow::{Context, Result};
use serde_json::json;

use crate::cli::{GuardrailMode, Provider};
use crate::config::RuntimeConfig;
use crate::error::format_cli_error;
use crate::guardrail::{apply_guardrail, buffered_output_required};
use crate::provider::parse_provider_name;
use crate::retrieval::RetrievalService;
use crate::runner::{
    ResolvedRuntimeTools, ToolConfirmationSettings, build_single_runner_for_chat,
};
use crate::session::build_session_service;
use crate::streaming::{run_prompt_with_retrieval, run_prompt_streaming_with_retrieval};
use crate::telemetry::TelemetrySink;
use crate::checkpoint::{
    CheckpointStore, format_checkpoint_list, restore_session_events, snapshot_session_events,
};
use crate::compact::{CompactStrategy, compact_session};
use crate::context::ContextUsage;
use crate::tool_policy::matches_wildcard;
use crate::todos;
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChatCommand {
    Exit,
    Status,
    Help,
    Tools,
    Mcp,
    Usage,
    Compact,
    Checkpoint(String),
    Tangent(String),
    Todos(String),
    Delegate(String),
    Provider(String),
    Model(Option<String>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParsedChatCommand {
    NotACommand,
    Command(ChatCommand),
    MissingArgument { usage: &'static str },
    UnknownCommand(String),
}

pub fn parse_chat_command(input: &str) -> ParsedChatCommand {
    let trimmed = input.trim();

    if trimmed.eq_ignore_ascii_case("exit") || trimmed.eq_ignore_ascii_case("/exit") {
        return ParsedChatCommand::Command(ChatCommand::Exit);
    }

    if !trimmed.starts_with('/') {
        return ParsedChatCommand::NotACommand;
    }

    let slashless = trimmed.trim_start_matches('/');
    if slashless.is_empty() {
        return ParsedChatCommand::UnknownCommand("/".to_string());
    }

    let mut parts = slashless.splitn(2, char::is_whitespace);
    let command = parts
        .next()
        .map(|value| value.to_ascii_lowercase())
        .unwrap_or_default();
    let arg = parts.next().map(str::trim).unwrap_or_default();

    match command.as_str() {
        "exit" => ParsedChatCommand::Command(ChatCommand::Exit),
        "status" => ParsedChatCommand::Command(ChatCommand::Status),
        "help" => ParsedChatCommand::Command(ChatCommand::Help),
        "tools" => ParsedChatCommand::Command(ChatCommand::Tools),
        "mcp" => ParsedChatCommand::Command(ChatCommand::Mcp),
        "usage" => ParsedChatCommand::Command(ChatCommand::Usage),
        "compact" => ParsedChatCommand::Command(ChatCommand::Compact),
        "checkpoint" => ParsedChatCommand::Command(ChatCommand::Checkpoint(arg.to_string())),
        "tangent" => ParsedChatCommand::Command(ChatCommand::Tangent(arg.to_string())),
        "todos" => ParsedChatCommand::Command(ChatCommand::Todos(arg.to_string())),
        "delegate" => ParsedChatCommand::Command(ChatCommand::Delegate(arg.to_string())),
        "provider" => {
            if arg.is_empty() {
                ParsedChatCommand::MissingArgument {
                    usage: "/provider <auto|gemini|openai|anthropic|deepseek|groq|ollama>",
                }
            } else {
                ParsedChatCommand::Command(ChatCommand::Provider(arg.to_string()))
            }
        }
        "model" => {
            if arg.is_empty() {
                ParsedChatCommand::Command(ChatCommand::Model(None))
            } else {
                ParsedChatCommand::Command(ChatCommand::Model(Some(arg.to_string())))
            }
        }
        other => ParsedChatCommand::UnknownCommand(format!("/{other}")),
    }
}

pub fn print_chat_help() {
    println!("Chat commands:");
    println!("- /help: show command quick reference");
    println!("- /status: show active profile/provider/model/session");
    println!("- /provider <name>: switch provider and rebuild runtime");
    println!("- /model [id]: pick a model interactively or switch directly by id");
    println!("- /tools: show active built-in/MCP tools and confirmation policy");
    println!("- /mcp: show MCP server and tool summary");
    println!("- /usage: show context usage and token breakdown");
    println!("- /compact: summarize conversation to free context space");
    println!("- /checkpoint save|list|restore: manage conversation snapshots");
    println!("- /tangent: enter/exit exploratory branch (tail: keep last exchange)");
    println!("- /todos: view/delete/clear-finished task lists");
    println!("- /delegate <task>: (experimental) run isolated sub-agent task");
    println!("- /exit: end interactive chat");
}

pub fn print_chat_usage() {
    println!("Usage examples:");
    println!("- Type plain text to send a prompt to the agent.");
    println!("- /provider openai");
    println!("- /model");
    println!("- /model gpt-4o-mini");
    println!("- /tools");
    println!("- /mcp");
    println!("- /status");
    println!("- /exit");
}

#[derive(Debug, Clone, Copy)]
pub struct ModelPickerOption {
    id: &'static str,
    context_window: &'static str,
    description: &'static str,
}

pub fn model_picker_options(provider: Provider) -> Vec<ModelPickerOption> {
    match provider {
        Provider::Gemini => vec![
            ModelPickerOption {
                id: "gemini-2.5-flash",
                context_window: "1M",
                description: "fast balanced default",
            },
            ModelPickerOption {
                id: "gemini-2.5-pro",
                context_window: "1M",
                description: "higher reasoning depth",
            },
        ],
        Provider::Openai => vec![
            ModelPickerOption {
                id: "gpt-4o-mini",
                context_window: "128k",
                description: "low-latency default",
            },
            ModelPickerOption {
                id: "gpt-4.1",
                context_window: "1M",
                description: "higher quality general reasoning",
            },
            ModelPickerOption {
                id: "o3-mini",
                context_window: "200k",
                description: "reasoning-focused",
            },
        ],
        Provider::Anthropic => vec![
            ModelPickerOption {
                id: "claude-sonnet-4-20250514",
                context_window: "200k",
                description: "balanced default",
            },
            ModelPickerOption {
                id: "claude-3-5-haiku-latest",
                context_window: "200k",
                description: "fast lower-latency option",
            },
        ],
        Provider::Deepseek => vec![
            ModelPickerOption {
                id: "deepseek-chat",
                context_window: "64k",
                description: "general conversation default",
            },
            ModelPickerOption {
                id: "deepseek-reasoner",
                context_window: "64k",
                description: "reasoning-focused",
            },
        ],
        Provider::Groq => vec![
            ModelPickerOption {
                id: "llama-3.3-70b-versatile",
                context_window: "128k",
                description: "balanced default",
            },
            ModelPickerOption {
                id: "mixtral-8x7b-32768",
                context_window: "32k",
                description: "fast throughput option",
            },
        ],
        Provider::Ollama => vec![
            ModelPickerOption {
                id: "llama3.2",
                context_window: "local-configured",
                description: "default local model",
            },
            ModelPickerOption {
                id: "qwen2.5-coder",
                context_window: "local-configured",
                description: "coding-optimized local model",
            },
        ],
        Provider::Auto => Vec::new(),
    }
}

pub fn resolve_model_picker_selection(
    options: &[ModelPickerOption],
    selection: &str,
) -> Result<Option<String>> {
    if options.is_empty() {
        return Ok(None);
    }

    let trimmed = selection.trim();
    if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("cancel") {
        return Ok(None);
    }

    if let Ok(index) = trimmed.parse::<usize>() {
        if index == 0 || index > options.len() {
            return Err(anyhow::anyhow!(
                "invalid selection '{}'; expected 1-{}",
                trimmed,
                options.len()
            ));
        }
        return Ok(Some(options[index - 1].id.to_string()));
    }

    if let Some(option) = options
        .iter()
        .find(|option| option.id.eq_ignore_ascii_case(trimmed))
    {
        return Ok(Some(option.id.to_string()));
    }

    Ok(Some(trimmed.to_string()))
}

pub fn prompt_model_picker(provider: Provider, current_model: &str) -> Result<Option<String>> {
    let options = model_picker_options(provider);
    if options.is_empty() {
        println!(
            "Model picker catalog unavailable for provider {:?}. Use /model <model-id>.",
            provider
        );
        return Ok(None);
    }

    println!(
        "Model picker: provider={:?} active_model={}",
        provider, current_model
    );
    for (idx, option) in options.iter().enumerate() {
        println!(
            "{}. {} (ctx={}, {})",
            idx + 1,
            option.id,
            option.context_window,
            option.description
        );
    }
    print!("Select model number or id (Enter to cancel): ");
    io::stdout().flush().context("failed to flush stdout")?;

    let mut selection = String::new();
    io::stdin()
        .read_line(&mut selection)
        .context("failed to read model picker input")?;
    resolve_model_picker_selection(&options, &selection)
}

pub fn print_chat_tools(
    cfg: &RuntimeConfig,
    runtime_tools: &ResolvedRuntimeTools,
    tool_confirmation: &ToolConfirmationSettings,
) {
    let mut built_in_tools = Vec::<String>::new();
    let mut mcp_tools = Vec::<String>::new();

    for tool in &runtime_tools.tools {
        let name = tool.name().to_string();
        if runtime_tools.mcp_tool_names.contains(&name) {
            mcp_tools.push(name);
        } else {
            built_in_tools.push(name);
        }
    }

    built_in_tools.sort();
    built_in_tools.dedup();
    mcp_tools.sort();
    mcp_tools.dedup();

    let required_count = tool_confirmation
        .run_config
        .tool_confirmation_decisions
        .len();
    let approved_count = tool_confirmation
        .run_config
        .tool_confirmation_decisions
        .values()
        .filter(|decision| matches!(decision, ToolConfirmationDecision::Approve))
        .count();

    println!(
        "Tools: total={} built_in={} mcp={}",
        runtime_tools.tools.len(),
        built_in_tools.len(),
        mcp_tools.len()
    );
    println!("Tool confirmation mode: {:?}", cfg.tool_confirmation_mode);
    println!(
        "Confirmation decisions: required={} approved={} denied={}",
        required_count,
        approved_count,
        required_count.saturating_sub(approved_count)
    );

    let allow = &cfg.agent_allow_tools;
    let deny = &cfg.agent_deny_tools;
    let has_policy = !allow.is_empty() || !deny.is_empty();

    println!("Built-in tools:");
    if built_in_tools.is_empty() {
        println!("  <none>");
    } else {
        for name in &built_in_tools {
            let suffix = tool_permission_label(name, allow, deny, has_policy, &tool_confirmation.run_config);
            println!("  - {name}{suffix}");
        }
    }
    println!("MCP tools:");
    if mcp_tools.is_empty() {
        println!("  <none>");
    } else {
        for name in &mcp_tools {
            let suffix = tool_permission_label(name, allow, deny, has_policy, &tool_confirmation.run_config);
            println!("  - {name}{suffix}");
        }
    }
}

fn tool_permission_label(
    name: &str,
    allow: &[String],
    deny: &[String],
    has_policy: bool,
    run_config: &RunConfig,
) -> String {
    let mut tags = Vec::new();
    if has_policy {
        if !allow.is_empty() && allow.iter().any(|p| matches_wildcard(p.trim(), name)) {
            tags.push("allowed");
        }
        if deny.iter().any(|p| matches_wildcard(p.trim(), name)) {
            tags.push("denied");
        }
    }
    if let Some(decision) = run_config.tool_confirmation_decisions.get(name) {
        match decision {
            ToolConfirmationDecision::Approve => tags.push("approved"),
            ToolConfirmationDecision::Deny => tags.push("requires-approval"),
        }
    }
    if tags.is_empty() {
        String::new()
    } else {
        format!(" ({})", tags.join(", "))
    }
}

pub fn print_chat_mcp(cfg: &RuntimeConfig, runtime_tools: &ResolvedRuntimeTools) {
    let enabled_servers = cfg
        .mcp_servers
        .iter()
        .filter(|server| server.enabled.unwrap_or(true))
        .count();

    println!(
        "MCP: configured_servers={} enabled={} discovered_tools={}",
        cfg.mcp_servers.len(),
        enabled_servers,
        runtime_tools.mcp_tool_names.len()
    );

    for server in cfg.mcp_servers.iter().filter(|s| s.enabled.unwrap_or(true)) {
        let auth_hint = crate::mcp::check_auth_hint(server);
        let auth_status = match &auth_hint {
            Some(hint) => format!("⚠ {}", hint),
            None if server.auth_bearer_env.is_some() => "✓ configured".to_string(),
            None => "none".to_string(),
        };
        let server_tools: Vec<&String> = runtime_tools
            .mcp_tool_names
            .iter()
            .collect();
        let tool_count = server_tools.len();
        println!(
            "  {} endpoint={} auth={} tools={}",
            server.name, server.endpoint, auth_status, tool_count
        );
    }

    if runtime_tools.mcp_tool_names.is_empty() {
        println!("Discovered MCP tools: <none>");
    } else {
        println!("Discovered MCP tools:");
        for name in &runtime_tools.mcp_tool_names {
            println!("  - {name}");
        }
    }
}

pub enum ChatCommandAction {
    Continue,
    Exit,
}

pub async fn dispatch_chat_command(
    command: ChatCommand,
    cfg: &mut RuntimeConfig,
    runner: &mut Runner,
    resolved_provider: &mut Provider,
    model_name: &mut String,
    session_service: &Arc<dyn SessionService>,
    runtime_tools: &ResolvedRuntimeTools,
    tool_confirmation: &ToolConfirmationSettings,
    telemetry: &TelemetrySink,
    context_usage: Option<&ContextUsage>,
    checkpoint_store: &mut CheckpointStore,
) -> Result<ChatCommandAction> {
    match command {
        ChatCommand::Exit => Ok(ChatCommandAction::Exit),
        ChatCommand::Status => {
            println!(
                "profile={} provider={:?} model={} session_id={}",
                cfg.profile, resolved_provider, model_name, cfg.session_id
            );
            Ok(ChatCommandAction::Continue)
        }
        ChatCommand::Help => {
            print_chat_help();
            Ok(ChatCommandAction::Continue)
        }
        ChatCommand::Usage => {
            if let Some(usage) = context_usage {
                print!("{}", usage.format_usage());
            } else {
                print_chat_usage();
            }
            Ok(ChatCommandAction::Continue)
        }
        ChatCommand::Compact => {
            println!("Compacting conversation...");
            match compact_session(session_service, cfg, &CompactStrategy::default()).await {
                Ok(Some(msg)) => println!("{msg}"),
                Ok(None) => println!("Conversation too short to compact."),
                Err(e) => eprintln!("Compaction failed: {e}"),
            }
            Ok(ChatCommandAction::Continue)
        }
        ChatCommand::Checkpoint(sub) => {
            let parts: Vec<&str> = sub.split_whitespace().collect();
            match parts.first().map(|s| *s) {
                Some("save") => {
                    let label = parts.get(1..).map(|p| p.join(" ")).unwrap_or_default();
                    match snapshot_session_events(session_service, cfg).await {
                        Ok(events) => {
                            let cp = checkpoint_store.save(&label, events);
                            println!("Checkpoint [{}] '{}' saved.", cp.tag, cp.label);
                        }
                        Err(e) => eprintln!("Failed to save checkpoint: {e}"),
                    }
                }
                Some("list") => {
                    print!("{}", format_checkpoint_list(checkpoint_store));
                }
                Some("restore") => {
                    if let Some(tag_str) = parts.get(1) {
                        if let Ok(tag) = tag_str.parse::<usize>() {
                            if let Some(cp) = checkpoint_store.get(tag) {
                                let events = cp.events.clone();
                                match restore_session_events(session_service, cfg, &events).await {
                                    Ok(()) => println!("Restored to checkpoint [{}] '{}'.", cp.tag, cp.label),
                                    Err(e) => eprintln!("Restore failed: {e}"),
                                }
                            } else {
                                println!("No checkpoint with tag {tag}. Use /checkpoint list.");
                            }
                        } else {
                            println!("Invalid tag. Usage: /checkpoint restore <number>");
                        }
                    } else {
                        println!("Usage: /checkpoint restore <tag>");
                    }
                }
                _ => {
                    println!("Usage: /checkpoint save [label] | list | restore <tag>");
                }
            }
            Ok(ChatCommandAction::Continue)
        }
        ChatCommand::Tangent(sub) => {
            match sub.trim() {
                "tail" => {
                    if !checkpoint_store.in_tangent() {
                        println!("Not in tangent mode. Use /tangent to enter.");
                    } else {
                        match snapshot_session_events(session_service, cfg).await {
                            Ok(current) => {
                                if let Some(events) = checkpoint_store.exit_tangent_tail(&current) {
                                    match restore_session_events(session_service, cfg, &events).await {
                                        Ok(()) => println!("Exited tangent mode (kept last exchange)."),
                                        Err(e) => eprintln!("Tangent tail restore failed: {e}"),
                                    }
                                }
                            }
                            Err(e) => eprintln!("Failed to read session: {e}"),
                        }
                    }
                }
                _ => {
                    if checkpoint_store.in_tangent() {
                        // Exit tangent
                        if let Some(events) = checkpoint_store.exit_tangent() {
                            match restore_session_events(session_service, cfg, &events).await {
                                Ok(()) => println!("Exited tangent mode. Conversation restored."),
                                Err(e) => eprintln!("Tangent restore failed: {e}"),
                            }
                        }
                    } else {
                        // Enter tangent
                        match snapshot_session_events(session_service, cfg).await {
                            Ok(events) => {
                                let tag = checkpoint_store.enter_tangent(events);
                                println!("Entered tangent mode (baseline checkpoint [{tag}]). Use /tangent to exit or /tangent tail to keep last exchange.");
                            }
                            Err(e) => eprintln!("Failed to enter tangent: {e}"),
                        }
                    }
                }
            }
            Ok(ChatCommandAction::Continue)
        }
        ChatCommand::Todos(sub) => {
            let workspace = std::env::current_dir().unwrap_or_default();
            let parts: Vec<&str> = sub.split_whitespace().collect();
            match parts.first().map(|s| *s) {
                Some("view") => {
                    if let Some(id) = parts.get(1) {
                        match todos::load_todo(&workspace, id) {
                            Ok(todo) => print!("{}", todo.format_display()),
                            Err(e) => eprintln!("Failed to load todo: {e}"),
                        }
                    } else {
                        println!("Usage: /todos view <id>");
                    }
                }
                Some("delete") => {
                    if let Some(id) = parts.get(1) {
                        match todos::delete_todo(&workspace, id) {
                            Ok(()) => println!("Deleted todo '{id}'."),
                            Err(e) => eprintln!("Failed to delete: {e}"),
                        }
                    } else {
                        println!("Usage: /todos delete <id>");
                    }
                }
                Some("clear-finished") => {
                    match todos::clear_finished_todos(&workspace) {
                        Ok(n) => println!("Cleared {n} finished todo list(s)."),
                        Err(e) => eprintln!("Failed to clear: {e}"),
                    }
                }
                _ => {
                    match todos::format_todos_summary(&workspace) {
                        Ok(summary) => print!("{summary}"),
                        Err(e) => eprintln!("Failed to list todos: {e}"),
                    }
                }
            }
            Ok(ChatCommandAction::Continue)
        }
        ChatCommand::Delegate(task) => {
            if task.trim().is_empty() {
                println!("Usage: /delegate <task description>");
                println!("(experimental) Runs an isolated sub-agent prompt.");
            } else {
                println!("[experimental] Delegate is not yet wired to a sub-agent runner.");
                println!("Task queued: {}", task.trim());
            }
            Ok(ChatCommandAction::Continue)
        }
        ChatCommand::Tools => {
            print_chat_tools(cfg, runtime_tools, tool_confirmation);
            Ok(ChatCommandAction::Continue)
        }
        ChatCommand::Mcp => {
            print_chat_mcp(cfg, runtime_tools);
            Ok(ChatCommandAction::Continue)
        }
        ChatCommand::Provider(provider_name) => {
            let new_provider = parse_provider_name(&provider_name)?;
            let mut switched_cfg = cfg.clone();
            switched_cfg.provider = new_provider;
            switched_cfg.model = None;

            match build_single_runner_for_chat(
                &switched_cfg,
                session_service.clone(),
                runtime_tools,
                tool_confirmation,
                telemetry,
            )
            .await
            {
                Ok((new_runner, new_resolved_provider, new_model_name)) => {
                    *runner = new_runner;
                    *resolved_provider = new_resolved_provider;
                    *model_name = new_model_name;
                    telemetry.emit(
                        "chat.provider_switched",
                        json!({
                            "provider": format!("{:?}", resolved_provider).to_ascii_lowercase(),
                            "model": model_name.clone()
                        }),
                    );
                    switched_cfg.provider = *resolved_provider;
                    switched_cfg.model = Some(model_name.clone());
                    *cfg = switched_cfg;
                    tracing::info!(
                        provider = ?resolved_provider,
                        model = %model_name,
                        "Switched model provider"
                    );
                    println!(
                        "Switched provider to {:?} (model={}). Session continuity preserved.",
                        resolved_provider, model_name
                    );
                }
                Err(err) => {
                    eprintln!("{}", format_cli_error(&err, cfg.show_sensitive_config));
                    println!(
                        "Provider remains {:?} (model={}).",
                        resolved_provider, model_name
                    );
                }
            }

            Ok(ChatCommandAction::Continue)
        }
        ChatCommand::Model(next_model) => {
            let chosen_model = match next_model {
                Some(value) => Some(value),
                None => prompt_model_picker(*resolved_provider, model_name)?,
            };
            let Some(chosen_model) = chosen_model else {
                println!(
                    "Model unchanged ('{}' on provider {:?}).",
                    model_name, resolved_provider
                );
                return Ok(ChatCommandAction::Continue);
            };

            let mut switched_cfg = cfg.clone();
            switched_cfg.model = Some(chosen_model);

            match build_single_runner_for_chat(
                &switched_cfg,
                session_service.clone(),
                runtime_tools,
                tool_confirmation,
                telemetry,
            )
            .await
            {
                Ok((new_runner, new_resolved_provider, new_model_name)) => {
                    *runner = new_runner;
                    *resolved_provider = new_resolved_provider;
                    *model_name = new_model_name;
                    telemetry.emit(
                        "chat.model_switched",
                        json!({
                            "provider": format!("{:?}", resolved_provider).to_ascii_lowercase(),
                            "model": model_name.clone()
                        }),
                    );
                    switched_cfg.provider = *resolved_provider;
                    switched_cfg.model = Some(model_name.clone());
                    *cfg = switched_cfg;
                    tracing::info!(
                        provider = ?resolved_provider,
                        model = %model_name,
                        "Switched model"
                    );
                    println!(
                        "Switched model to '{}' on provider {:?}. Session continuity preserved.",
                        model_name, resolved_provider
                    );
                }
                Err(err) => {
                    eprintln!("{}", format_cli_error(&err, cfg.show_sensitive_config));
                    println!(
                        "Model remains '{}' on provider {:?}.",
                        model_name, resolved_provider
                    );
                }
            }

            Ok(ChatCommandAction::Continue)
        }
    }
}

pub async fn run_chat(
    mut cfg: RuntimeConfig,
    retrieval_service: Arc<dyn RetrievalService>,
    runtime_tools: ResolvedRuntimeTools,
    tool_confirmation: ToolConfirmationSettings,
    telemetry: &TelemetrySink,
) -> Result<()> {
    let session_service = build_session_service(&cfg).await?;
    let (mut runner, mut resolved_provider, mut model_name) = build_single_runner_for_chat(
        &cfg,
        session_service.clone(),
        &runtime_tools,
        &tool_confirmation,
        telemetry,
    )
    .await?;

    cfg.provider = resolved_provider;
    cfg.model = Some(model_name.clone());

    telemetry.emit(
        "chat.started",
        json!({
            "provider": format!("{:?}", resolved_provider).to_ascii_lowercase(),
            "model": model_name.clone(),
            "profile": cfg.profile.clone()
        }),
    );

    tracing::info!(provider = ?resolved_provider, model = %model_name, "Using model");
    println!("Interactive mode started. Type /help for commands or /exit to quit.");
    println!("Quick start: /provider <name>, /model (picker), /tools, /mcp, /usage.");
    if buffered_output_required(cfg.guardrail_output_mode) {
        println!(
            "Guardrail output mode {:?} active: chat will buffer model responses before printing.",
            cfg.guardrail_output_mode
        );
    }
    let stdin = io::stdin();
    let mut line = String::new();
    let mut checkpoint_store = CheckpointStore::new();

    loop {
        print!("zavora> ");
        io::stdout().flush().context("failed to flush stdout")?;
        line.clear();
        stdin
            .read_line(&mut line)
            .context("failed to read input from stdin")?;
        let input = line.trim();
        if input.eq_ignore_ascii_case("/exit") || input.eq_ignore_ascii_case("exit") {
            break;
        }
        if input.is_empty() {
            continue;
        }

        match parse_chat_command(input) {
            ParsedChatCommand::NotACommand => {}
            ParsedChatCommand::MissingArgument { usage } => {
                println!("Usage: {usage}");
                continue;
            }
            ParsedChatCommand::UnknownCommand(command) => {
                println!("Unknown command '{command}'. Use /help.");
                continue;
            }
            ParsedChatCommand::Command(command) => {
                let action = dispatch_chat_command(
                    command,
                    &mut cfg,
                    &mut runner,
                    &mut resolved_provider,
                    &mut model_name,
                    &session_service,
                    &runtime_tools,
                    &tool_confirmation,
                    telemetry,
                    None, // context usage computed on demand in future
                    &mut checkpoint_store,
                )
                .await?;
                if matches!(action, ChatCommandAction::Exit) {
                    break;
                }
                continue;
            }
        }

        let guarded_input =
            match apply_guardrail(&cfg, telemetry, "input", cfg.guardrail_input_mode, input) {
                Ok(text) => text,
                Err(err) => {
                    eprintln!("{}", format_cli_error(&err, cfg.show_sensitive_config));
                    continue;
                }
            };

        if buffered_output_required(cfg.guardrail_output_mode) {
            let answer = run_prompt_with_retrieval(
                &runner,
                &cfg,
                &guarded_input,
                retrieval_service.as_ref(),
                telemetry,
            )
            .await?;
            let answer = match apply_guardrail(
                &cfg,
                telemetry,
                "output",
                cfg.guardrail_output_mode,
                &answer,
            ) {
                Ok(text) => text,
                Err(err) => {
                    eprintln!("{}", format_cli_error(&err, cfg.show_sensitive_config));
                    continue;
                }
            };
            println!("{answer}");
        } else {
            let answer = run_prompt_streaming_with_retrieval(
                &runner,
                &cfg,
                &guarded_input,
                retrieval_service.as_ref(),
                telemetry,
            )
            .await?;
            if matches!(cfg.guardrail_output_mode, GuardrailMode::Observe)
                && let Err(err) = apply_guardrail(
                    &cfg,
                    telemetry,
                    "output",
                    cfg.guardrail_output_mode,
                    &answer,
                )
            {
                eprintln!("{}", format_cli_error(&err, cfg.show_sensitive_config));
            }
        }
    }

    Ok(())
}

