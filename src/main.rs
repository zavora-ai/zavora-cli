use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use clap::Parser;
use serde_json::json;

use zavora_cli::agent_catalog::*;
use zavora_cli::agent_supervisor::*;
use zavora_cli::capabilities::*;
use zavora_cli::chat::*;
use zavora_cli::cli::*;
use zavora_cli::config::*;
use zavora_cli::doctor::*;
use zavora_cli::error::*;
use zavora_cli::essentials;
use zavora_cli::eval::*;
use zavora_cli::guardrail::*;
use zavora_cli::headless::*;
use zavora_cli::mcp::*;
use zavora_cli::onboarding::{persist_onboarding_config, run_onboarding_wizard};
use zavora_cli::profiles::*;
use zavora_cli::provider::*;
use zavora_cli::ralph::{RalphRunOptions, run_ralph};
use zavora_cli::retrieval::*;
use zavora_cli::runner::*;
use zavora_cli::server::*;
use zavora_cli::session::*;
use zavora_cli::teams::*;
use zavora_cli::telemetry::*;
use zavora_cli::workflow::*;

fn init_tracing(log_filter: &str, use_stderr: bool, terminal_ui: bool) -> Result<()> {
    use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

    let filter = tracing_subscriber::EnvFilter::try_new(log_filter)
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));

    // OTLP layer added first (on bare Registry) so the type parameter is Registry
    let otlp_layer: Option<
        Box<dyn tracing_subscriber::Layer<tracing_subscriber::Registry> + Send + Sync>,
    > = std::env::var("OTEL_EXPORTER_OTLP_ENDPOINT")
        .ok()
        .and_then(|ep| adk_telemetry::build_otlp_layer("zavora-cli", &ep).ok());

    if terminal_ui {
        // A formatted stdout/stderr layer corrupts an alternate-screen renderer.
        // Structured telemetry remains active while the retained TUI owns the terminal.
        tracing_subscriber::registry()
            .with(otlp_layer)
            .with(filter)
            .init();
    } else if use_stderr {
        let fmt = tracing_subscriber::fmt::layer()
            .with_target(false)
            .with_writer(std::io::stderr);
        tracing_subscriber::registry()
            .with(otlp_layer)
            .with(filter)
            .with(fmt)
            .init();
    } else {
        let fmt = tracing_subscriber::fmt::layer().with_target(false);
        tracing_subscriber::registry()
            .with(otlp_layer)
            .with(filter)
            .with(fmt)
            .init();
    }
    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let show_sensitive_config = cli.show_sensitive_config;
    let output_format = cli.output_format;
    if let Err(err) = run_cli(cli).await {
        if output_format == OutputFormat::Text {
            eprintln!("{}", format_cli_error(&err, show_sensitive_config));
        } else if let Err(render_error) =
            write_structured_error(output_format, &err, show_sensitive_config)
        {
            eprintln!("failed to render structured error: {render_error}");
        }
        tracing::error!(
            category = %categorize_error(&err).code(),
            error = %render_error_message(&err, show_sensitive_config),
            "command failed"
        );
        adk_telemetry::shutdown_telemetry();
        std::process::exit(categorize_error(&err).exit_code());
    }

    adk_telemetry::shutdown_telemetry();
    Ok(())
}

async fn run_cli(mut cli: Cli) -> Result<()> {
    use std::io::IsTerminal;

    let stdin_is_terminal = std::io::stdin().is_terminal();
    if cli.command.is_none() && (cli.output_format != OutputFormat::Text || !stdin_is_terminal) {
        cli.command = Some(Commands::Ask { prompt: Vec::new() });
    }
    let terminal_ui = cli.output_format == OutputFormat::Text
        && matches!(cli.command, None | Some(Commands::Chat))
        && std::io::stdin().is_terminal()
        && std::io::stdout().is_terminal()
        && std::env::var_os("ZAVORA_CLASSIC").is_none()
        && std::env::var("TERM").map_or(true, |term| term != "dumb");
    init_tracing(
        &cli.log_filter,
        cli.output_format != OutputFormat::Text
            || matches!(
                cli.command,
                Some(Commands::Mcp {
                    command: McpCommands::Serve
                }) | Some(Commands::Essentials {
                    command: EssentialCommands::Serve { .. }
                })
            ),
        terminal_ui,
    )?;
    if let Some(Commands::Essentials {
        command: EssentialCommands::Serve { server },
    }) = cli.command.as_ref()
    {
        return essentials::serve(server).await;
    }
    let mut profiles = load_profiles(&cli.config_path)?;

    // Initialize SQLite memory (eager, before any tool use)
    // Use ADK-Rust v2 project-scoped memory to isolate workspaces.
    let memory_result = if let Some(project_id) = zavora_cli::agents::memory::detect_project_id() {
        zavora_cli::agents::memory::init_with_project(&project_id).await
    } else {
        zavora_cli::agents::memory::init().await
    };
    if let Err(e) = memory_result {
        tracing::warn!("Memory init failed: {e}");
    }

    // Auto-setup: trigger onboarding wizard for commands that need a provider
    let needs_provider = matches!(
        cli.command,
        None | Some(Commands::Ask { .. })
            | Some(Commands::Chat)
            | Some(Commands::Workflow { .. })
            | Some(Commands::ReleasePlan { .. })
            | Some(Commands::Agents {
                command: AgentCommands::Run { .. }
            })
            | Some(Commands::Agents {
                command: AgentCommands::Parallel { .. }
            })
            | Some(Commands::Agents {
                command: AgentCommands::Worker { .. }
            })
            | Some(Commands::Teams {
                command: TeamCommands::Run { .. }
            })
            | Some(Commands::Ralph { .. })
    );
    if needs_provider && cli.output_format == OutputFormat::Text && terminal_ui {
        let workspace = std::env::current_dir().unwrap_or_default();
        if zavora_cli::theme::is_first_run(&workspace) && !profiles.profiles.contains_key("default")
        {
            let result = run_onboarding_wizard(None)?;
            persist_onboarding_config(&result, &cli.config_path)?;
            profiles = load_profiles(&cli.config_path)?;
        }
    }

    let agent_paths = default_agent_paths();
    let mut resolved_agents = load_resolved_agents(&agent_paths)?;
    resolved_agents.extend(zavora_cli::plugins::enabled_plugin_agents()?);
    let selected_agent_name = load_agent_selection(&agent_paths.selection_file)?;
    let mut cfg = resolve_runtime_config_with_agents(
        &cli,
        &profiles,
        &resolved_agents,
        selected_agent_name.as_deref(),
    )?;
    let configured_mcp_names = cfg
        .mcp_servers
        .iter()
        .map(|server| server.name.clone())
        .collect::<std::collections::BTreeSet<_>>();
    for server in zavora_cli::plugins::enabled_plugin_mcp_servers()? {
        if !configured_mcp_names.contains(&server.name) {
            cfg.mcp_servers.push(server);
        }
    }
    let command = command_label(cli.command.as_ref().unwrap_or(&Commands::Chat));
    let telemetry = TelemetrySink::new(&cfg, command.clone());
    let headless_options = HeadlessOptions {
        output_format: cli.output_format,
        input_files: cli.input_files.clone(),
        read_stdin: cli.stdin,
        no_stdin: cli.no_stdin,
        always_approve: cli.always_approve,
    };
    let automation_command = matches!(
        cli.command,
        Some(Commands::Ask { .. })
            | Some(Commands::Workflow { .. })
            | Some(Commands::ReleasePlan { .. })
            | Some(Commands::Agents {
                command: AgentCommands::Run { .. }
            })
            | Some(Commands::Agents {
                command: AgentCommands::Parallel { .. }
            })
            | Some(Commands::Agents {
                command: AgentCommands::Worker { .. }
            })
            | Some(Commands::Teams {
                command: TeamCommands::Run { .. }
            })
            | Some(Commands::Ralph { .. })
    );
    zavora_cli::tools::confirming::set_headless_mode(automation_command);
    let started_at = Instant::now();
    telemetry.emit(
        "command.started",
        json!({
            "profile": cfg.profile,
            "session_backend": format!("{:?}", cfg.session_backend),
            "retrieval_backend": format!("{:?}", cfg.retrieval_backend),
            "telemetry_enabled": cfg.telemetry_enabled,
            "guardrail_input_mode": guardrail_mode_label(cfg.guardrail_input_mode),
            "guardrail_output_mode": guardrail_mode_label(cfg.guardrail_output_mode)
        }),
    );

    let retrieval_service = if matches!(
        cli.command,
        Some(Commands::Ask { .. })
            | Some(Commands::Chat)
            | Some(Commands::Workflow { .. })
            | Some(Commands::ReleasePlan { .. })
            | Some(Commands::Agents {
                command: AgentCommands::Run { .. }
            })
            | Some(Commands::Agents {
                command: AgentCommands::Parallel { .. }
            })
            | Some(Commands::Agents {
                command: AgentCommands::Worker { .. }
            })
            | Some(Commands::Teams {
                command: TeamCommands::Run { .. }
            })
            | Some(Commands::Ralph { .. })
            | None
    ) {
        let service = build_retrieval_service(&cfg)?;
        tracing::info!(
            backend = service.backend_name(),
            max_chunks = cfg.retrieval_max_chunks,
            max_chars = cfg.retrieval_max_chars,
            min_score = cfg.retrieval_min_score,
            "Using retrieval backend"
        );
        Some(service)
    } else {
        None
    };

    let execution: Result<()> = match cli.command.unwrap_or(Commands::Chat) {
        Commands::Ask { prompt } => {
            let prompt = load_prompt(&prompt, &headless_options)?;
            enforce_prompt_limit(&prompt, cfg.max_prompt_chars)?;
            let prompt =
                apply_guardrail(&cfg, &telemetry, "input", cfg.guardrail_input_mode, &prompt)?;
            let (model, resolved_provider, model_name) = resolve_model(&cfg)?;
            tracing::info!(provider = ?resolved_provider, model = %model_name, "Using model");
            telemetry.emit(
                "model.resolved",
                json!({
                    "provider": format!("{:?}", resolved_provider).to_ascii_lowercase(),
                    "model": model_name,
                    "path": "ask"
                }),
            );
            let runtime_tools = resolve_runtime_tools(&cfg).await;
            approve_runtime_tools(&runtime_tools, headless_options.always_approve);
            let tool_confirmation = resolve_tool_confirmation_settings(&cfg, &runtime_tools);
            let agent = build_single_agent_with_tools(
                model,
                runtime_tools.tools(),
                tool_confirmation.policy,
                Duration::from_secs(cfg.tool_timeout_secs),
                Some(&cfg),
            )?;
            let runner =
                build_runner_with_run_config(agent, &cfg, Some(tool_confirmation.run_config))
                    .await?;
            let retrieval = retrieval_service
                .as_deref()
                .context("retrieval service should be initialized for ask command")?;
            run_headless(
                &runner,
                &cfg,
                &prompt,
                retrieval,
                &telemetry,
                &RunMetadata {
                    command: "ask".to_string(),
                    session_id: cfg.session_id.clone(),
                    provider: format!("{resolved_provider:?}").to_ascii_lowercase(),
                    model: model_name,
                },
                headless_options.output_format,
            )
            .await?;
            Ok(())
        }
        Commands::Chat => {
            let runtime_tools = resolve_runtime_tools(&cfg).await;
            approve_runtime_tools(&runtime_tools, headless_options.always_approve);
            let tool_confirmation = resolve_tool_confirmation_settings(&cfg, &runtime_tools);
            let retrieval = retrieval_service
                .as_ref()
                .context("retrieval service should be initialized for chat command")?
                .clone();
            run_chat(
                cfg.clone(),
                retrieval,
                runtime_tools,
                tool_confirmation,
                &telemetry,
            )
            .await?;
            Ok(())
        }
        Commands::Models => {
            print_model_catalog(&cfg);
            Ok(())
        }
        Commands::Workflow {
            mode,
            prompt,
            max_iterations,
        } => {
            let prompt = load_prompt(&prompt, &headless_options)?;
            enforce_prompt_limit(&prompt, cfg.max_prompt_chars)?;
            let prompt =
                apply_guardrail(&cfg, &telemetry, "input", cfg.guardrail_input_mode, &prompt)?;
            let (model, resolved_provider, model_name) = resolve_model(&cfg)?;
            tracing::info!(provider = ?resolved_provider, model = %model_name, workflow = ?mode, "Using workflow");
            telemetry.emit(
                "model.resolved",
                json!({
                    "provider": format!("{:?}", resolved_provider).to_ascii_lowercase(),
                    "model": model_name,
                    "path": "workflow",
                    "workflow_mode": workflow_mode_label(mode)
                }),
            );
            let runtime_tools = resolve_runtime_tools(&cfg).await;
            report_degraded_surface(
                &runtime_tools,
                &format!("workflow.{}", workflow_mode_label(mode)),
            );
            approve_runtime_tools(&runtime_tools, headless_options.always_approve);
            let tool_confirmation = resolve_tool_confirmation_settings(&cfg, &runtime_tools);
            let agent = build_workflow_agent(
                mode,
                model,
                max_iterations,
                runtime_tools.tools(),
                tool_confirmation.policy,
                Duration::from_secs(cfg.tool_timeout_secs),
                Some(&cfg),
            )?;
            let runner =
                build_runner_with_run_config(agent, &cfg, Some(tool_confirmation.run_config))
                    .await?;
            let retrieval = retrieval_service
                .as_deref()
                .context("retrieval service should be initialized for workflow command")?;
            run_headless(
                &runner,
                &cfg,
                &prompt,
                retrieval,
                &telemetry,
                &RunMetadata {
                    command: format!("workflow.{}", workflow_mode_label(mode)),
                    session_id: cfg.session_id.clone(),
                    provider: format!("{resolved_provider:?}").to_ascii_lowercase(),
                    model: model_name,
                },
                headless_options.output_format,
            )
            .await?;
            Ok(())
        }
        Commands::ReleasePlan { goal, releases } => {
            let prompt = load_prompt(&goal, &headless_options)?;
            enforce_prompt_limit(&prompt, cfg.max_prompt_chars)?;
            let prompt =
                apply_guardrail(&cfg, &telemetry, "input", cfg.guardrail_input_mode, &prompt)?;
            let (model, resolved_provider, model_name) = resolve_model(&cfg)?;
            tracing::info!(provider = ?resolved_provider, model = %model_name, releases, "Generating release plan");
            telemetry.emit(
                "model.resolved",
                json!({
                    "provider": format!("{:?}", resolved_provider).to_ascii_lowercase(),
                    "model": model_name,
                    "path": "release-plan"
                }),
            );
            let agent = build_release_planning_agent(model, releases, Some(&cfg))?;
            let runner = build_runner(agent, &cfg).await?;
            let retrieval = retrieval_service
                .as_deref()
                .context("retrieval service should be initialized for release-plan command")?;
            run_headless(
                &runner,
                &cfg,
                &prompt,
                retrieval,
                &telemetry,
                &RunMetadata {
                    command: "release-plan".to_string(),
                    session_id: cfg.session_id.clone(),
                    provider: format!("{resolved_provider:?}").to_ascii_lowercase(),
                    model: model_name,
                },
                headless_options.output_format,
            )
            .await?;
            Ok(())
        }
        Commands::Doctor => {
            run_doctor(&cfg).await?;
            Ok(())
        }
        Commands::Migrate => {
            run_migrate(&cfg).await?;
            Ok(())
        }
        Commands::Profiles { command } => match command {
            ProfileCommands::List => {
                run_profiles_list(&profiles, &cfg)?;
                Ok(())
            }
            ProfileCommands::Show => {
                run_profiles_show(&cfg)?;
                Ok(())
            }
        },
        Commands::Agents { command } => match command {
            AgentCommands::List { json } => {
                run_agents_list(&resolved_agents, &cfg.agent_name, &agent_paths, json)?;
                Ok(())
            }
            AgentCommands::Show { name, json } => {
                run_agents_show(&resolved_agents, &cfg.agent_name, name, json)?;
                Ok(())
            }
            AgentCommands::Select { name } => {
                run_agents_select(&resolved_agents, &agent_paths, name)?;
                Ok(())
            }
            AgentCommands::Run { name, task } => {
                let selected = resolved_agents.get(&name).ok_or_else(|| {
                    anyhow::anyhow!("agent '{}' not found. Run 'zavora-cli agents list'.", name)
                })?;
                let mut agent_cfg = cfg.clone();
                zavora_cli::config::apply_agent_overrides(&mut agent_cfg, selected);

                let prompt = load_prompt(&task, &headless_options)?;
                enforce_prompt_limit(&prompt, agent_cfg.max_prompt_chars)?;
                let prompt = apply_guardrail(
                    &agent_cfg,
                    &telemetry,
                    "input",
                    agent_cfg.guardrail_input_mode,
                    &prompt,
                )?;
                let (model, resolved_provider, model_name) = resolve_model(&agent_cfg)?;
                let runtime_tools = resolve_runtime_tools(&agent_cfg).await;
                approve_runtime_tools(&runtime_tools, headless_options.always_approve);
                let confirmation = resolve_tool_confirmation_settings(&agent_cfg, &runtime_tools);
                let agent = build_single_agent_with_tools(
                    model,
                    runtime_tools.tools(),
                    confirmation.policy,
                    Duration::from_secs(agent_cfg.tool_timeout_secs),
                    Some(&agent_cfg),
                )?;
                let runner =
                    build_runner_with_run_config(agent, &agent_cfg, Some(confirmation.run_config))
                        .await?;
                let retrieval = retrieval_service
                    .as_deref()
                    .context("retrieval service should be initialized for agents run")?;
                run_headless(
                    &runner,
                    &agent_cfg,
                    &prompt,
                    retrieval,
                    &telemetry,
                    &RunMetadata {
                        command: format!("agents.run.{name}"),
                        session_id: agent_cfg.session_id.clone(),
                        provider: format!("{resolved_provider:?}").to_ascii_lowercase(),
                        model: model_name,
                    },
                    headless_options.output_format,
                )
                .await?;
                Ok(())
            }
            AgentCommands::Parallel {
                agents,
                max_concurrency,
                task,
            } => {
                let mut selected_agents = Vec::new();
                let mut seen = std::collections::BTreeSet::new();
                for name in agents {
                    if !seen.insert(name.clone()) {
                        continue;
                    }
                    if name == "ralph" {
                        return Err(anyhow::anyhow!(
                            "'ralph' is a pipeline, not a subagent; use `zavora-cli ralph`"
                        ));
                    }
                    let selected = resolved_agents.get(&name).cloned().ok_or_else(|| {
                        anyhow::anyhow!("agent '{}' not found. Run 'zavora-cli agents list'.", name)
                    })?;
                    selected_agents.push(selected);
                }
                let prompt = load_prompt(&task, &headless_options)?;
                enforce_prompt_limit(&prompt, cfg.max_prompt_chars)?;
                let runtime_tools = resolve_runtime_tools(&cfg).await;
                approve_runtime_tools(&runtime_tools, headless_options.always_approve);
                let retrieval = retrieval_service
                    .clone()
                    .context("retrieval service should be initialized for agents parallel")?;
                let results = zavora_cli::subagents::run_parallel(
                    selected_agents,
                    &prompt,
                    zavora_cli::subagents::ParallelRunContext {
                        base_cfg: &cfg,
                        parent_tools: &runtime_tools,
                        retrieval,
                        telemetry: &telemetry,
                        max_concurrency,
                        progress_format: headless_options.output_format,
                        progress: None,
                        session_service: None,
                    },
                )
                .await;
                let success = results.iter().all(|result| result.success);
                zavora_cli::subagents::render_results(&results, headless_options.output_format);
                if !success {
                    return Err(anyhow::anyhow!("one or more parallel subagents failed"));
                }
                Ok(())
            }
            AgentCommands::Spawn {
                name,
                parent_run_id,
                worktree,
                task,
            } => {
                ensure_spawnable_agent(&resolved_agents, &name)?;
                let task = load_prompt(&task, &headless_options)?;
                enforce_prompt_limit(&task, cfg.max_prompt_chars)?;
                let store = AgentRunStore::open_default().await?;
                let parent_run_id =
                    parent_run_id.or_else(|| std::env::var("ZAVORA_SUPERVISOR_RUN_ID").ok());
                let run = store
                    .create_run(
                        NewAgentRun {
                            parent_run_id,
                            parent_session_id: cfg.session_id.clone(),
                            agent: name,
                            task,
                            workspace: std::env::current_dir()?,
                            worktree,
                            retry_of: None,
                        },
                        &SupervisorPolicy::default(),
                    )
                    .await?;
                launch_worker(
                    &store,
                    &run,
                    LaunchOptions {
                        cfg: &cfg,
                        always_approve: headless_options.always_approve,
                    },
                )
                .await?;
                render_agent_run(
                    &store.get(&run.id).await?.unwrap_or(run),
                    headless_options.output_format != OutputFormat::Text,
                )?;
                Ok(())
            }
            AgentCommands::Runs { limit, json } => {
                let store = AgentRunStore::open_default().await?;
                render_agent_runs(
                    &store.list(limit).await?,
                    json || headless_options.output_format != OutputFormat::Text,
                )?;
                Ok(())
            }
            AgentCommands::Status { run_id, json } => {
                let store = AgentRunStore::open_default().await?;
                let run = required_agent_run(&store, &run_id).await?;
                render_agent_run_status(
                    &store,
                    &run,
                    json || headless_options.output_format != OutputFormat::Text,
                )
                .await?;
                Ok(())
            }
            AgentCommands::Send { run_id, message } => {
                let message = load_prompt(&message, &headless_options)?;
                let store = AgentRunStore::open_default().await?;
                let continuation = store.prepare_continuation(&run_id, &message).await?;
                if continuation.launch_required {
                    launch_worker(
                        &store,
                        &continuation.run,
                        LaunchOptions {
                            cfg: &cfg,
                            always_approve: headless_options.always_approve,
                        },
                    )
                    .await?;
                }
                render_agent_run(
                    &required_agent_run(&store, &run_id).await?,
                    headless_options.output_format != OutputFormat::Text,
                )?;
                Ok(())
            }
            AgentCommands::Wait {
                run_id,
                timeout_secs,
                json,
            } => {
                let store = AgentRunStore::open_default().await?;
                let run = store
                    .wait_terminal(&run_id, Duration::from_secs(timeout_secs))
                    .await?;
                render_agent_run(
                    &run,
                    json || headless_options.output_format != OutputFormat::Text,
                )?;
                Ok(())
            }
            AgentCommands::Cancel { run_id } => {
                let store = AgentRunStore::open_default().await?;
                let run = cancel_run(&store, &run_id).await?;
                render_agent_run(&run, headless_options.output_format != OutputFormat::Text)?;
                Ok(())
            }
            AgentCommands::Retry { run_id, worktree } => {
                let store = AgentRunStore::open_default().await?;
                let previous = required_agent_run(&store, &run_id).await?;
                ensure_spawnable_agent(&resolved_agents, &previous.agent)?;
                let run = store
                    .create_run(
                        NewAgentRun {
                            parent_run_id: previous.parent_run_id.clone(),
                            parent_session_id: previous.parent_session_id.clone(),
                            agent: previous.agent.clone(),
                            task: previous.task.clone(),
                            workspace: previous.workspace.clone(),
                            worktree,
                            retry_of: Some(previous.id.clone()),
                        },
                        &SupervisorPolicy::default(),
                    )
                    .await?;
                launch_worker(
                    &store,
                    &run,
                    LaunchOptions {
                        cfg: &cfg,
                        always_approve: headless_options.always_approve,
                    },
                )
                .await?;
                render_agent_run(
                    &required_agent_run(&store, &run.id).await?,
                    headless_options.output_format != OutputFormat::Text,
                )?;
                Ok(())
            }
            AgentCommands::Events {
                run_id,
                after,
                follow,
            } => run_agent_events(&run_id, after, follow).await,
            AgentCommands::WorktreeRemove { run_id, force } => {
                let store = AgentRunStore::open_default().await?;
                let run = required_agent_run(&store, &run_id).await?;
                if !run.status.is_terminal() {
                    anyhow::bail!("agent run '{run_id}' is still {}", run.status.label());
                }
                remove_worktree(&run, force)?;
                println!("removed worktree for agent run {run_id}");
                Ok(())
            }
            AgentCommands::Worker { run_id } => {
                let retrieval = retrieval_service
                    .as_deref()
                    .context("retrieval service should be initialized for agents worker")?;
                run_supervised_worker(
                    &run_id,
                    &mut cfg,
                    &resolved_agents,
                    retrieval,
                    &telemetry,
                    headless_options.always_approve,
                )
                .await
            }
        },
        Commands::Teams { command } => {
            let workspace = std::env::current_dir()?;
            let teams = discover_teams(&workspace)?;
            match command {
                TeamCommands::List { json } => {
                    println!(
                        "{}",
                        render_catalog(
                            &teams,
                            json || headless_options.output_format != OutputFormat::Text,
                        )?
                    );
                    Ok(())
                }
                TeamCommands::Show { name, json } => {
                    let team = teams.get(&name).ok_or_else(|| {
                        anyhow::anyhow!("team '{}' not found. Run 'zavora-cli teams list'.", name)
                    })?;
                    println!(
                        "{}",
                        render_team(
                            team,
                            json || headless_options.output_format != OutputFormat::Text,
                        )?
                    );
                    Ok(())
                }
                TeamCommands::Validate { name, json } => {
                    let selected = if let Some(name) = name.as_deref() {
                        vec![teams.get(name).ok_or_else(|| {
                            anyhow::anyhow!(
                                "team '{}' not found. Run 'zavora-cli teams list'.",
                                name
                            )
                        })?]
                    } else {
                        teams.values().collect::<Vec<_>>()
                    };
                    let mut records = Vec::new();
                    let mut failures = Vec::new();
                    for team in selected {
                        match validate_bindings(team, &cfg.available_agents) {
                            Ok(()) => records.push(serde_json::json!({
                                "name": team.name(),
                                "valid": true,
                                "architecture": team.definition.architecture(),
                                "members": team.definition.member_names(),
                            })),
                            Err(error) => {
                                records.push(serde_json::json!({
                                    "name": team.name(),
                                    "valid": false,
                                    "error": error.to_string(),
                                }));
                                failures.push(format!("{}: {error}", team.name()));
                            }
                        }
                    }
                    if json || headless_options.output_format != OutputFormat::Text {
                        println!("{}", serde_json::to_string_pretty(&records)?);
                    } else {
                        for record in &records {
                            println!(
                                "{} {}",
                                if record["valid"].as_bool().unwrap_or(false) {
                                    "✓"
                                } else {
                                    "✗"
                                },
                                record["name"].as_str().unwrap_or("<unknown>")
                            );
                        }
                    }
                    if !failures.is_empty() {
                        anyhow::bail!("team validation failed: {}", failures.join("; "));
                    }
                    Ok(())
                }
                TeamCommands::Topology { name, json } => {
                    let team = teams.get(&name).ok_or_else(|| {
                        anyhow::anyhow!("team '{}' not found. Run 'zavora-cli teams list'.", name)
                    })?;
                    let graph = topology(&team.definition)?;
                    if json || headless_options.output_format != OutputFormat::Text {
                        println!("{}", serde_json::to_string_pretty(&graph)?);
                    } else {
                        println!("{} ({})", graph.name, graph.architecture);
                        for node in &graph.nodes {
                            println!("  • {node}");
                        }
                        for edge in &graph.edges {
                            println!("  {} -> {} ({})", edge.from, edge.to, edge.relationship);
                        }
                    }
                    Ok(())
                }
                TeamCommands::Run { name, task } => {
                    let team = teams.get(&name).ok_or_else(|| {
                        anyhow::anyhow!("team '{}' not found. Run 'zavora-cli teams list'.", name)
                    })?;
                    let prompt = load_prompt(&task, &headless_options)?;
                    enforce_prompt_limit(&prompt, cfg.max_prompt_chars)?;
                    let prompt = apply_guardrail(
                        &cfg,
                        &telemetry,
                        "input",
                        cfg.guardrail_input_mode,
                        &prompt,
                    )?;
                    let runtime_tools = resolve_runtime_tools(&cfg).await;
                    report_degraded_surface(&runtime_tools, &format!("teams.run.{name}"));
                    approve_runtime_tools(&runtime_tools, headless_options.always_approve);
                    let confirmation = resolve_tool_confirmation_settings(&cfg, &runtime_tools);
                    let built = build_team(team, &cfg, &runtime_tools).await?;
                    if headless_options.always_approve {
                        trust_team_relationships(&built);
                    } else {
                        for member in &built.roster {
                            if cfg.approve_tool.iter().any(|pattern| {
                                zavora_cli::tool_policy::matches_wildcard(pattern, member)
                            }) {
                                zavora_cli::tools::confirming::trust_tool(member);
                            }
                        }
                    }
                    telemetry.emit(
                        "team.compiled",
                        json!({
                            "team": built.name,
                            "architecture": built.architecture,
                            "roster": built.roster,
                            "routes": built.routes,
                            "source": team.source.label(),
                            "api_version": team.definition.api_version(),
                            "adk_version": "2.1.0",
                        }),
                    );
                    let runner = build_runner_with_run_config(
                        built.root.clone(),
                        &cfg,
                        Some(with_team_confirmation_handler(confirmation.run_config)),
                    )
                    .await?;
                    let retrieval = retrieval_service
                        .as_deref()
                        .context("retrieval service should be initialized for teams run")?;
                    run_headless(
                        &runner,
                        &cfg,
                        &prompt,
                        retrieval,
                        &telemetry,
                        &RunMetadata {
                            command: format!("teams.run.{name}"),
                            session_id: cfg.session_id.clone(),
                            provider: "team".to_string(),
                            model: built.routes.join(","),
                        },
                        headless_options.output_format,
                    )
                    .await?;
                    for receipt in built.execution_receipts() {
                        telemetry.emit("team.execution_receipt", serde_json::to_value(receipt)?);
                    }
                    Ok(())
                }
                TeamCommands::Schema => {
                    println!("{}", schema_json()?);
                    Ok(())
                }
            }
        }
        Commands::Capabilities { command } => {
            let configured_servers = cfg
                .mcp_servers
                .iter()
                .map(|server| server.name.clone())
                .collect::<Vec<_>>();
            match command {
                CapabilityCommands::List {
                    category,
                    enabled,
                    json,
                } => run_capabilities_list(category, enabled, json, &configured_servers),
                CapabilityCommands::Search { query, json } => {
                    run_capabilities_search(&query.join(" "), json, &configured_servers)
                }
                CapabilityCommands::Info { id, json } => {
                    run_capabilities_info(&id, json, &configured_servers)
                }
                CapabilityCommands::Enable { id } => run_capabilities_set_enabled(&id, true),
                CapabilityCommands::Disable { id } => run_capabilities_set_enabled(&id, false),
            }
        }
        Commands::Essentials { command } => match command {
            EssentialCommands::Status { json } => {
                let configured_servers = cfg
                    .mcp_servers
                    .iter()
                    .map(|server| server.name.clone())
                    .collect::<Vec<_>>();
                essentials::run_status(&configured_servers, json)
            }
            EssentialCommands::Serve { .. } => unreachable!("handled before runtime setup"),
        },
        Commands::Mcp { command } => match command {
            McpCommands::Catalog { query, json } => {
                zavora_cli::mcp_catalog::run_catalog(&query.join(" "), json)
            }
            McpCommands::Add { server } => zavora_cli::mcp_catalog::run_add(
                std::path::Path::new(&cfg.config_path),
                &cfg.profile,
                &server,
            ),
            McpCommands::Remove { server } => zavora_cli::mcp_catalog::run_remove(
                std::path::Path::new(&cfg.config_path),
                &cfg.profile,
                &server,
            ),
            McpCommands::Enable { server } => zavora_cli::mcp_catalog::run_set_enabled(
                std::path::Path::new(&cfg.config_path),
                &cfg.profile,
                &server,
                true,
            ),
            McpCommands::Disable { server } => zavora_cli::mcp_catalog::run_set_enabled(
                std::path::Path::new(&cfg.config_path),
                &cfg.profile,
                &server,
                false,
            ),
            McpCommands::Auth { server } => zavora_cli::mcp::run_mcp_auth(&cfg, &server).await,
            McpCommands::List => {
                run_mcp_list(&cfg).await?;
                Ok(())
            }
            McpCommands::Discover { server } => {
                run_mcp_discover(&cfg, server).await?;
                Ok(())
            }
            McpCommands::Info { server } => run_mcp_info(&cfg, &server),
            McpCommands::Doctor { server, json } => run_mcp_doctor(&cfg, server, json).await,
            McpCommands::Resources { server, uri, json } => {
                run_mcp_resources(&cfg, &server, uri.as_deref(), json).await
            }
            McpCommands::Prompts {
                server,
                name,
                arguments,
                json,
            } => run_mcp_prompts(&cfg, &server, name.as_deref(), arguments.as_deref(), json).await,
            McpCommands::Protocol { json } => run_mcp_protocol(json),
            McpCommands::Serve => {
                zavora_cli::mcp_server::run_mcp_server().await?;
                Ok(())
            }
        },
        Commands::Sessions { command } => match command {
            SessionCommands::List => {
                run_sessions_list(&cfg).await?;
                Ok(())
            }
            SessionCommands::Show { session_id, recent } => {
                run_sessions_show(&cfg, session_id, recent).await?;
                Ok(())
            }
            SessionCommands::Delete { session_id, force } => {
                run_sessions_delete(&cfg, session_id, force).await?;
                Ok(())
            }
            SessionCommands::Prune {
                keep,
                dry_run,
                force,
            } => {
                run_sessions_prune(&cfg, keep, dry_run, force).await?;
                Ok(())
            }
        },
        Commands::Telemetry { command } => match command {
            TelemetryCommands::Report { path, limit } => {
                run_telemetry_report(&cfg, path, limit)?;
                Ok(())
            }
        },
        Commands::Skills { command } => match command {
            SkillCommands::Search { query, json } => run_skills_search(&query.join(" "), json),
            SkillCommands::List { json } => run_skills_list(json),
            SkillCommands::Info { name, json } => run_skill_info(&name, json),
            SkillCommands::Validate { path, json } => run_skill_validate(&path, json),
            SkillCommands::Install {
                source,
                scope,
                link,
            } => {
                let skill = zavora_cli::skills::install_skill(&source, scope, link)?;
                println!(
                    "{} skill '{}' from {} scope: {}",
                    if link { "Linked" } else { "Installed" },
                    skill.name,
                    scope,
                    skill.path.display()
                );
                Ok(())
            }
            SkillCommands::Link { source, scope } => {
                let skill = zavora_cli::skills::install_skill(&source, scope, true)?;
                println!(
                    "Linked skill '{}' in {} scope: {}",
                    skill.name,
                    scope,
                    skill.path.display()
                );
                Ok(())
            }
            SkillCommands::Update { name, scope } => {
                for updated in zavora_cli::skills::update_skills(name.as_deref(), scope)? {
                    println!("Updated {updated}");
                }
                Ok(())
            }
            SkillCommands::Enable { name, scope } => {
                zavora_cli::skills::set_skill_enabled(&name, scope, true)?;
                println!("Enabled skill '{name}' in {scope} scope");
                Ok(())
            }
            SkillCommands::Disable { name, scope } => {
                zavora_cli::skills::set_skill_enabled(&name, scope, false)?;
                println!("Disabled skill '{name}' in {scope} scope");
                Ok(())
            }
            SkillCommands::Uninstall { name, scope } => {
                let removed = zavora_cli::skills::uninstall_skill(&name, scope)?;
                println!(
                    "{} skill '{name}' from {scope} scope",
                    if removed { "Uninstalled" } else { "Unlinked" }
                );
                Ok(())
            }
        },
        Commands::Plugins { command } => match command {
            PluginCommands::List { json } => run_plugins_list(json),
            PluginCommands::Info { name, json } => run_plugin_info(&name, json),
            PluginCommands::Validate { path, json } => run_plugin_validate(&path, json),
            PluginCommands::Install {
                source,
                scope,
                link,
            } => {
                let plugin = zavora_cli::plugins::install_plugin(&source, scope, link)?;
                println!(
                    "{} {} plugin '{}' in {} scope: {}",
                    if link { "Linked" } else { "Installed" },
                    plugin.ecosystem,
                    plugin.name,
                    scope,
                    plugin.root.display()
                );
                for warning in plugin.warnings {
                    println!("warning: {warning}");
                }
                Ok(())
            }
            PluginCommands::Link { source, scope } => {
                let plugin = zavora_cli::plugins::install_plugin(&source, scope, true)?;
                println!(
                    "Linked {} plugin '{}' in {} scope: {}",
                    plugin.ecosystem,
                    plugin.name,
                    scope,
                    plugin.root.display()
                );
                for warning in plugin.warnings {
                    println!("warning: {warning}");
                }
                Ok(())
            }
            PluginCommands::Update { name, scope } => {
                for updated in zavora_cli::plugins::update_plugins(name.as_deref(), scope)? {
                    println!("Updated {updated}");
                }
                Ok(())
            }
            PluginCommands::Enable { name, scope } => {
                zavora_cli::plugins::set_plugin_enabled(&name, scope, true)?;
                println!("Enabled plugin '{name}' in {scope} scope");
                Ok(())
            }
            PluginCommands::Disable { name, scope } => {
                zavora_cli::plugins::set_plugin_enabled(&name, scope, false)?;
                println!("Disabled plugin '{name}' in {scope} scope");
                Ok(())
            }
            PluginCommands::Uninstall { name, scope } => {
                let removed = zavora_cli::plugins::uninstall_plugin(&name, scope)?;
                println!(
                    "{} plugin '{name}' from {scope} scope",
                    if removed { "Uninstalled" } else { "Unlinked" }
                );
                Ok(())
            }
            PluginCommands::Doctor { json } => run_plugin_doctor(json),
        },
        Commands::Instructions { command } => match command {
            InstructionCommands::List { json } => run_instructions(false, json),
            InstructionCommands::Show { json } => run_instructions(true, json),
        },
        #[cfg(feature = "rag")]
        Commands::Rag { command } => match command {
            RagCommands::Ingest { path } => {
                run_rag_ingest(&path).await?;
                Ok(())
            }
        },
        Commands::Eval { command } => match command {
            EvalCommands::Run {
                dataset,
                output,
                benchmark_iterations,
                fail_under,
            } => {
                run_eval(
                    dataset,
                    output,
                    benchmark_iterations,
                    fail_under,
                    &telemetry,
                )?;
                Ok(())
            }
        },
        Commands::Server { command } => match command {
            ServerCommands::Serve { host, port } => {
                run_server(cfg.clone(), host, port, &telemetry).await?;
                Ok(())
            }
            ServerCommands::A2aSmoke => {
                run_a2a_smoke(&telemetry)?;
                Ok(())
            }
        },
        Commands::Ralph {
            prompt,
            phase,
            resume,
            output_dir,
        } => {
            let prompt = match load_prompt(&prompt, &headless_options) {
                Ok(prompt) => prompt,
                Err(error) if resume && error.to_string().contains("no prompt input") => {
                    String::new()
                }
                Err(error) => return Err(error),
            };
            if !prompt.is_empty() {
                enforce_prompt_limit(&prompt, cfg.max_prompt_chars)?;
            }
            let prompt =
                apply_guardrail(&cfg, &telemetry, "input", cfg.guardrail_input_mode, &prompt)?;
            telemetry.emit(
                "model.resolved",
                json!({
                    "provider": format!("{:?}", cfg.provider).to_ascii_lowercase(),
                    "model": cfg.model.clone().unwrap_or_default(),
                    "path": "ralph"
                }),
            );
            let retrieval = retrieval_service
                .as_deref()
                .context("retrieval service should be initialized for ralph")?;
            run_ralph(
                &cfg,
                prompt,
                RalphRunOptions {
                    phase,
                    resume,
                    output_dir,
                    output_format: headless_options.output_format,
                    always_approve: headless_options.always_approve,
                },
                &telemetry,
                retrieval,
            )
            .await?;
            Ok(())
        }
        Commands::Setup => {
            let existing_profile = profiles.profiles.get("default");
            let result = run_onboarding_wizard(existing_profile)?;
            persist_onboarding_config(&result, &cli.config_path)?;
            if result.skipped {
                println!(
                    "Minimal configuration saved. Set your provider via environment variables or edit the config file."
                );
            } else {
                println!("Configuration saved! You can start chatting with `zavora`.");
            }
            Ok(())
        }
        Commands::LspInit => {
            #[cfg(feature = "lsp")]
            {
                let config = zavora_cli::lsp::manager::generate_default_config();
                if config.servers.is_empty() {
                    println!("No language servers found in PATH.");
                    println!(
                        "Install one: rust-analyzer, typescript-language-server, pylsp, gopls, clangd"
                    );
                } else {
                    let path = ".zavora/lsp.json";
                    std::fs::create_dir_all(".zavora")?;
                    let json = serde_json::to_string_pretty(&config)?;
                    std::fs::write(path, &json)?;
                    println!("LSP config written to {path}:");
                    for (lang, srv) in &config.servers {
                        println!("  {lang}: {} {}", srv.command, srv.args.join(" "));
                    }
                    println!("\nLSP code intelligence is now enabled.");
                }
                Ok(())
            }
            #[cfg(not(feature = "lsp"))]
            {
                println!("LSP support not compiled. Rebuild with: cargo build --features lsp");
                Ok(())
            }
        }
    };

    let duration_ms = started_at.elapsed().as_millis();
    match &execution {
        Ok(_) => telemetry.emit(
            "command.completed",
            json!({"duration_ms": duration_ms, "status": "ok"}),
        ),
        Err(err) => telemetry.emit(
            "command.failed",
            json!({
                "duration_ms": duration_ms,
                "status": "error",
                "error": render_error_message(err, cfg.show_sensitive_config)
            }),
        ),
    }

    zavora_cli::tools::confirming::set_headless_mode(false);

    execution
}

fn ensure_spawnable_agent(
    agents: &std::collections::HashMap<String, ResolvedAgent>,
    name: &str,
) -> Result<()> {
    if name == "ralph" {
        anyhow::bail!("'ralph' is a pipeline, not a subagent; use `zavora-cli ralph`");
    }
    if !agents.contains_key(name) {
        anyhow::bail!("agent '{name}' not found. Run 'zavora-cli agents list'.");
    }
    Ok(())
}

async fn required_agent_run(store: &AgentRunStore, id: &str) -> Result<AgentRun> {
    store
        .get(id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("agent run '{id}' not found"))
}

async fn render_agent_run_status(
    store: &AgentRunStore,
    run: &AgentRun,
    json_output: bool,
) -> Result<()> {
    let children = store.children(&run.id).await?;
    let events = store.events(&run.id, 0).await?;
    if json_output {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "schema_version": AGENT_RUN_SCHEMA_VERSION,
                "run": run,
                "children": children,
                "events": events,
            }))?
        );
        return Ok(());
    }
    render_agent_run(run, false)?;
    if !children.is_empty() {
        println!("\nChildren:");
        for child in children {
            println!(
                "  {}  {:<16} {}",
                child.id,
                child.status.label(),
                child.agent
            );
        }
    }
    println!("\nEvents: {}", events.len());
    Ok(())
}

fn render_agent_run(run: &AgentRun, json_output: bool) -> Result<()> {
    if json_output {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "schema_version": AGENT_RUN_SCHEMA_VERSION,
                "run": run,
            }))?
        );
    } else {
        println!("{}  {}  {}", run.id, run.status.label(), run.agent);
        println!("  session: {}", run.session_id);
        if let Some(parent) = &run.parent_run_id {
            println!("  parent: {parent}");
        }
        println!("  workspace: {}", run.workspace.display());
        if let Some(path) = &run.worktree_path {
            println!("  worktree: {}", path.display());
        }
        println!("  log: {}", run.log_path.display());
        if let Some(response) = &run.response {
            println!("\n{response}");
        }
        if let Some(error) = &run.error {
            println!("\nerror: {error}");
        }
    }
    Ok(())
}

fn render_agent_runs(runs: &[AgentRun], json_output: bool) -> Result<()> {
    if json_output {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "schema_version": AGENT_RUN_SCHEMA_VERSION,
                "runs": runs,
            }))?
        );
        return Ok(());
    }
    if runs.is_empty() {
        println!("No durable agent runs found.");
        return Ok(());
    }
    println!("RUN ID                         STATUS            AGENT                 DEPTH");
    for run in runs {
        println!(
            "{:<30} {:<17} {:<21} {}",
            run.id,
            run.status.label(),
            run.agent,
            run.depth
        );
    }
    Ok(())
}

async fn run_agent_events(run_id: &str, mut after: i64, follow: bool) -> Result<()> {
    let store = AgentRunStore::open_default().await?;
    required_agent_run(&store, run_id).await?;
    loop {
        let events = store.events(run_id, after).await?;
        for event in events {
            after = event.sequence;
            println!(
                "{}",
                serde_json::to_string(&json!({
                    "schema_version": AGENT_RUN_SCHEMA_VERSION,
                    "type": "agent_run_event",
                    "event": event,
                }))?
            );
        }
        let run = required_agent_run(&store, run_id).await?;
        if !follow || run.status.is_terminal() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    Ok(())
}

async fn run_supervised_worker(
    run_id: &str,
    cfg: &mut RuntimeConfig,
    agents: &std::collections::HashMap<String, ResolvedAgent>,
    retrieval: &dyn RetrievalService,
    telemetry: &TelemetrySink,
    always_approve: bool,
) -> Result<()> {
    let store = AgentRunStore::open_default().await?;
    let launch_wait = Instant::now();
    let run = loop {
        let run = required_agent_run(&store, run_id).await?;
        if run.status != AgentRunStatus::Queued {
            break run;
        }
        if launch_wait.elapsed() >= Duration::from_secs(5) {
            store
                .fail(run_id, "worker was not activated by its supervisor")
                .await?;
            anyhow::bail!("agent run '{run_id}' was not activated by its supervisor");
        }
        // The detached process can be scheduled before its parent persists the
        // PID. Do not race an early worker failure against that state update.
        tokio::time::sleep(Duration::from_millis(20)).await;
    };
    if matches!(
        run.status,
        AgentRunStatus::Cancelled | AgentRunStatus::CancelRequested
    ) {
        return Ok(());
    }
    let selected = agents.get(&run.agent).ok_or_else(|| {
        anyhow::anyhow!(
            "agent '{}' is no longer available for supervised run '{}'",
            run.agent,
            run.id
        )
    })?;
    cfg.session_backend = SessionBackend::Sqlite;
    cfg.session_db_url = default_agent_session_url();
    cfg.session_id = run.session_id.clone();
    apply_agent_overrides(cfg, selected);

    let result: Result<()> = async {
        let runtime_tools = resolve_runtime_tools(cfg).await;
        approve_runtime_tools(&runtime_tools, always_approve);
        let confirmation = resolve_tool_confirmation_settings(cfg, &runtime_tools);
        let session_service = build_session_service(cfg).await?;
        ensure_session_exists(&session_service, cfg).await?;
        let (runner, provider, model) = build_single_runner_for_chat(
            cfg,
            session_service,
            &runtime_tools,
            &confirmation,
            telemetry,
        )
        .await?;
        let provider = format!("{provider:?}").to_ascii_lowercase();
        let mut last_response = String::new();
        loop {
            let current = required_agent_run(&store, run_id).await?;
            if matches!(
                current.status,
                AgentRunStatus::Cancelled | AgentRunStatus::CancelRequested
            ) {
                store.mark_cancelled(run_id).await?;
                return Ok(());
            }
            let messages = store.drain_messages(run_id).await?;
            if messages.is_empty() {
                store
                    .complete(run_id, &last_response, &provider, &model)
                    .await?;
                // Close the small enqueue/complete race: a sender that observed
                // the worker as running may have committed just after completion.
                let late_messages = store.drain_messages(run_id).await?;
                if late_messages.is_empty() {
                    return Ok(());
                }
                store.set_process(run_id, std::process::id()).await?;
                let prompt = late_messages.join("\n\n--- follow-up ---\n\n");
                last_response = run_supervised_turn(
                    &runner, cfg, &prompt, retrieval, telemetry, run_id, &provider, &model,
                )
                .await?;
                continue;
            }
            let prompt = messages.join("\n\n--- follow-up ---\n\n");
            last_response = run_supervised_turn(
                &runner, cfg, &prompt, retrieval, telemetry, run_id, &provider, &model,
            )
            .await?;
        }
    }
    .await;

    if let Err(error) = &result {
        store.fail(run_id, &error.to_string()).await?;
    }
    result
}

#[allow(clippy::too_many_arguments)]
async fn run_supervised_turn(
    runner: &adk_rust::prelude::Runner,
    cfg: &RuntimeConfig,
    prompt: &str,
    retrieval: &dyn RetrievalService,
    telemetry: &TelemetrySink,
    run_id: &str,
    provider: &str,
    model: &str,
) -> Result<String> {
    enforce_prompt_limit(prompt, cfg.max_prompt_chars)?;
    let prompt = apply_guardrail(cfg, telemetry, "input", cfg.guardrail_input_mode, prompt)?;
    run_headless(
        runner,
        cfg,
        &prompt,
        retrieval,
        telemetry,
        &RunMetadata {
            command: format!("agents.worker.{run_id}"),
            session_id: cfg.session_id.clone(),
            provider: provider.to_string(),
            model: model.to_string(),
        },
        OutputFormat::StreamJson,
    )
    .await
}

/// Announce a degraded tool surface before a long run begins.
///
/// Requirement 13.5: a workflow or Ralph run that starts without tools it was
/// configured to have must say so. Reporting after the fact is too late — the
/// model has already planned around the tools it could see.
fn report_degraded_surface(runtime_tools: &ResolvedRuntimeTools, command: &str) {
    let failures = runtime_tools.connect_failure_report();
    if failures.is_empty() {
        return;
    }
    eprintln!(
        "warning: {command} is starting with {} unreachable MCP server(s); its tools are unavailable for this run:",
        failures.len()
    );
    for failure in failures {
        eprintln!("  - {failure}");
    }
}

fn approve_runtime_tools(runtime_tools: &ResolvedRuntimeTools, always_approve: bool) {
    if always_approve {
        for tool in runtime_tools.tools() {
            zavora_cli::tools::confirming::trust_tool(tool.name());
        }
    }
}

fn run_skills_list(json_output: bool) -> Result<()> {
    let index = zavora_cli::skills::load_workspace_skills()?;
    let skills = index.skills();
    if json_output {
        println!("{}", serde_json::to_string_pretty(&index.summaries())?);
        return Ok(());
    }
    if skills.is_empty() {
        println!(
            "No skills found. Add <name>/SKILL.md under .agents, .zavora, .claude, .gemini, .grok, or .opencode skill roots."
        );
        return Ok(());
    }
    println!("{} skill(s) discovered:\n", skills.len());
    for s in skills {
        println!("  {} — {}\n    {}", s.name, s.description, s.path.display());
    }
    Ok(())
}

fn run_skills_search(query: &str, json_output: bool) -> Result<()> {
    let entries = zavora_cli::skills::search_registry(query)?;
    if json_output {
        println!("{}", serde_json::to_string_pretty(&entries)?);
    } else if entries.is_empty() {
        println!("No registry skills matched '{query}'.");
    } else {
        println!("{} registry skill(s):\n", entries.len());
        for entry in entries {
            println!(
                "  {} — {}\n    {}",
                entry.name, entry.category, entry.repository
            );
        }
    }
    Ok(())
}

fn run_skill_info(name: &str, json_output: bool) -> Result<()> {
    let index = zavora_cli::skills::load_workspace_skills()?;
    let skill = index
        .find_by_name(name)
        .with_context(|| format!("skill '{name}' was not found"))?;
    if json_output {
        println!("{}", serde_json::to_string_pretty(skill)?);
    } else {
        println!(
            "{}\n  {}\n  source: {}\n  version: {}\n  explicit trigger: {}",
            skill.name,
            skill.description,
            skill.path.display(),
            skill.version.as_deref().unwrap_or("unspecified"),
            skill.trigger
        );
    }
    Ok(())
}

fn run_skill_validate(path: &std::path::Path, json_output: bool) -> Result<()> {
    let skill = zavora_cli::skills::validate_skill_path(path)?;
    if json_output {
        println!("{}", serde_json::to_string_pretty(&skill)?);
    } else {
        println!("Valid skill '{}' at {}", skill.name, skill.path.display());
    }
    Ok(())
}

fn find_plugin(name: &str) -> Result<zavora_cli::plugins::PluginDescriptor> {
    let matches = zavora_cli::plugins::discover_plugins()?
        .into_iter()
        .filter(|plugin| plugin.name == name)
        .collect::<Vec<_>>();
    match matches.as_slice() {
        [] => Err(anyhow::anyhow!("plugin '{name}' was not found")),
        [plugin] => Ok(plugin.clone()),
        _ => Err(anyhow::anyhow!(
            "plugin name '{name}' is ambiguous; inspect `plugins list --json` and use a unique installation"
        )),
    }
}

fn run_plugins_list(json_output: bool) -> Result<()> {
    if json_output {
        println!(
            "{}",
            serde_json::to_string_pretty(&zavora_cli::plugins::discover_plugins()?)?
        );
    } else {
        println!("{}", zavora_cli::plugins::format_plugins_markdown()?);
    }
    Ok(())
}

fn run_plugin_info(name: &str, json_output: bool) -> Result<()> {
    let plugin = find_plugin(name)?;
    if json_output {
        println!("{}", serde_json::to_string_pretty(&plugin)?);
    } else {
        println!(
            "{} ({})\n  ecosystem: {}\n  version: {}\n  state: {}{}\n  root: {}\n  skills: {}\n  MCP sources: {}\n  agents: {}\n  hooks: {}",
            plugin.display_name,
            plugin.name,
            plugin.ecosystem,
            plugin.version.as_deref().unwrap_or("unspecified"),
            if plugin.enabled {
                "enabled"
            } else {
                "disabled"
            },
            if plugin.linked { ", linked" } else { "" },
            plugin.root.display(),
            plugin.components.skill_roots.len(),
            plugin.components.mcp_files.len() + usize::from(plugin.components.inline_mcp.is_some()),
            plugin.components.agent_roots.len(),
            plugin.components.hook_files.len(),
        );
        for warning in plugin.warnings {
            println!("  warning: {warning}");
        }
    }
    Ok(())
}

fn run_plugin_validate(path: &std::path::Path, json_output: bool) -> Result<()> {
    let plugin = zavora_cli::plugins::inspect_plugin_root(path)?;
    if json_output {
        println!("{}", serde_json::to_string_pretty(&plugin)?);
    } else {
        println!(
            "Valid {} plugin '{}' at {}",
            plugin.ecosystem,
            plugin.name,
            plugin.root.display()
        );
        for warning in plugin.warnings {
            println!("warning: {warning}");
        }
    }
    Ok(())
}

fn run_plugin_doctor(json_output: bool) -> Result<()> {
    let report = zavora_cli::plugins::doctor_report()?;
    if json_output {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        println!(
            "Plugin runtime: {}\nDiscovered: {} ({} enabled)",
            report["status"].as_str().unwrap_or("unknown"),
            report["plugin_count"],
            report["enabled_count"]
        );
        if let Some(warnings) = report["warnings"].as_array() {
            for warning in warnings {
                println!(
                    "warning [{}]: {}",
                    warning["plugin"].as_str().unwrap_or("unknown"),
                    warning["warning"].as_str().unwrap_or("unknown warning")
                );
            }
        }
    }
    Ok(())
}

fn run_instructions(show_content: bool, json_output: bool) -> Result<()> {
    let resolved = zavora_cli::skills::resolve_workspace_instructions()?;
    if json_output {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "active": resolved.sources,
                "deferred": resolved.deferred_sources,
                "warnings": resolved.warnings,
                "content": show_content.then_some(resolved.content),
            }))?
        );
    } else {
        println!(
            "{}",
            zavora_cli::skills::format_instructions_markdown(show_content)
        );
    }
    Ok(())
}

#[cfg(feature = "rag")]
async fn run_rag_ingest(path: &str) -> Result<()> {
    let pipeline = zavora_cli::tools::rag::build_rag_pipeline()?;
    let p = std::path::Path::new(path);
    if p.is_dir() {
        let mut count = 0;
        for entry in ignore::WalkBuilder::new(p).build().filter_map(|e| e.ok()) {
            if entry.file_type().is_some_and(|ft| ft.is_file())
                && let Ok(text) = std::fs::read_to_string(entry.path())
            {
                let doc = adk_rag::Document {
                    id: entry.path().to_string_lossy().to_string(),
                    text,
                    metadata: Default::default(),
                    source_uri: Some(entry.path().to_string_lossy().to_string()),
                };
                pipeline
                    .ingest("default", &doc)
                    .await
                    .map_err(|e| anyhow::anyhow!("{e}"))?;
                count += 1;
            }
        }
        println!("Ingested {} files from {}", count, path);
    } else {
        let text = std::fs::read_to_string(p).context("failed to read file")?;
        let doc = adk_rag::Document {
            id: path.to_string(),
            text,
            metadata: Default::default(),
            source_uri: Some(path.to_string()),
        };
        pipeline
            .ingest("default", &doc)
            .await
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        println!("Ingested {}", path);
    }
    Ok(())
}
