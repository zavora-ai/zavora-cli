//! Interactive command execution and command-specific domain adapters.

use super::render::{format_agents_markdown, format_mcp_doctor_markdown, format_mcp_markdown};
use super::*;

#[allow(clippy::too_many_arguments)]
pub(super) async fn dispatch_tui_command(
    command: ChatCommand,
    app: &mut App,
    runner: &mut Arc<Runner>,
    cfg: &mut RuntimeConfig,
    provider: &mut crate::cli::Provider,
    model_name: &mut String,
    session_service: &Arc<dyn adk_session::SessionService>,
    checkpoint_store: &mut CheckpointStore,
    runtime_tools: &ResolvedRuntimeTools,
    confirmation: &ToolConfirmationSettings,
    retrieval: Arc<dyn RetrievalService>,
    telemetry: &TelemetrySink,
    tx: &tokio::sync::mpsc::UnboundedSender<UiEvent>,
) -> bool {
    match command {
        ChatCommand::Exit => return true,
        ChatCommand::Help => app.palette = Some(PaletteState::default()),
        ChatCommand::Status => app.push_system(format_status_markdown(cfg, app)),
        ChatCommand::Tools => app.push_system(format_tools_markdown(cfg, runtime_tools)),
        ChatCommand::Mcp => app.push_system(format_mcp_markdown(cfg, runtime_tools)),
        ChatCommand::Capabilities => {
            let configured = cfg
                .mcp_servers
                .iter()
                .map(|server| server.name.clone())
                .collect::<Vec<_>>();
            app.push_system(crate::capabilities::format_catalog_markdown_with_runtime(
                &configured,
                &runtime_tools
                    .mcp_tool_names()
                    .iter()
                    .cloned()
                    .collect::<Vec<_>>(),
            ));
        }
        ChatCommand::Essentials => {
            let configured = cfg
                .mcp_servers
                .iter()
                .map(|server| server.name.clone())
                .collect::<Vec<_>>();
            app.push_system(crate::essentials::format_status_markdown(&configured));
        }
        ChatCommand::Skills => app.push_system(crate::skills::format_skills_markdown()),
        ChatCommand::Plugins => match crate::plugins::format_plugins_markdown() {
            Ok(markdown) => app.push_system(markdown),
            Err(error) => app.push_system(format!("Plugin inspection failed: {error}")),
        },
        ChatCommand::Instructions(subcommand) => app.push_system(
            crate::skills::format_instructions_markdown(subcommand.trim() == "show"),
        ),
        ChatCommand::Agents => {
            app.push_system(format_agents_markdown());
            match crate::agent_supervisor::AgentRunStore::open_default().await {
                Ok(store) => match store.list(10).await {
                    Ok(runs) => {
                        app.push_system(crate::agent_supervisor::format_runs_markdown(&runs))
                    }
                    Err(error) => app.push_system(format!("Agent run inspection failed: {error}")),
                },
                Err(error) => app.push_system(format!("Agent supervisor unavailable: {error}")),
            }
        }
        ChatCommand::Teams(subcommand) => {
            match crate::teams::resolve_interactive_command(&subcommand, cfg) {
                Ok(crate::teams::InteractiveTeamAction::Display(output)) => app.push_system(output),
                Ok(crate::teams::InteractiveTeamAction::Run { name, task }) => {
                    let teams = match crate::teams::discover_teams(
                        &std::env::current_dir().unwrap_or_default(),
                    ) {
                        Ok(teams) => teams,
                        Err(error) => {
                            app.push_system(format!("Team discovery failed: {error}"));
                            return false;
                        }
                    };
                    let Some(team) = teams.get(&name) else {
                        app.push_system(format!("Team `{name}` is no longer available."));
                        return false;
                    };
                    let built = match crate::teams::build_team(team, cfg, runtime_tools).await {
                        Ok(built) => built,
                        Err(error) => {
                            app.push_system(format!("Team compilation failed: {error}"));
                            return false;
                        }
                    };
                    telemetry.emit(
                        "team.compiled",
                        serde_json::json!({
                            "team": built.name,
                            "architecture": built.architecture,
                            "roster": built.roster,
                            "routes": built.routes,
                            "surface": "tui",
                        }),
                    );
                    let team_runner = match crate::runner::build_runner_with_session_service(
                        built.root.clone(),
                        cfg,
                        session_service.clone(),
                        Some(crate::teams::with_team_confirmation_handler(
                            confirmation.run_config.clone(),
                        )),
                    )
                    .await
                    {
                        Ok(runner) => Arc::new(runner),
                        Err(error) => {
                            app.push_system(format!("Team runner failed: {error}"));
                            return false;
                        }
                    };
                    app.busy = true;
                    app.active_agent = format!("team · {name}");
                    app.push_system(format!(
                        "Started governed team `{name}`. Press `Esc` to cancel this turn."
                    ));
                    let cfg = cfg.clone();
                    let telemetry = telemetry.clone();
                    let tx = tx.clone();
                    let task = tokio::spawn(async move {
                        let policy = crate::retrieval::RetrievalPolicy {
                            max_chunks: cfg.retrieval_max_chunks,
                            max_chars: cfg.retrieval_max_chars,
                            min_score: cfg.retrieval_min_score,
                        };
                        let prompt = crate::retrieval::augment_prompt_with_retrieval(
                            retrieval.as_ref(),
                            &task,
                            policy,
                        )
                        .unwrap_or(task);
                        if let Err(error) = crate::streaming::run_prompt_to_ui(
                            &team_runner,
                            &cfg,
                            &prompt,
                            &telemetry,
                            tx.clone(),
                        )
                        .await
                        {
                            let _ = tx.send(UiEvent::Error(error.to_string()));
                            let _ = tx.send(UiEvent::Completed(String::new()));
                        }
                        for receipt in built.execution_receipts() {
                            if let Ok(value) = serde_json::to_value(receipt) {
                                telemetry.emit("team.execution_receipt", value);
                            }
                        }
                    });
                    app.task_abort = Some(task.abort_handle());
                }
                Err(error) => app.push_system(format!("Team command failed: {error}")),
            }
        }
        ChatCommand::Inspect => app.push_system(format_inspect_markdown(cfg, runtime_tools)),
        ChatCommand::Doctor => app.push_system(format_mcp_doctor_markdown(cfg, runtime_tools)),
        ChatCommand::Models => app.push_system(format_models_markdown(cfg)),
        ChatCommand::Usage => match snapshot_session_events(session_service, cfg).await {
            Ok(events) => {
                let usage =
                    compute_context_usage(&events, &provider.to_string(), &cfg.worker_model);
                app.push_system(format_context_markdown(&usage));
            }
            Err(error) => app.push_system(format!("Context inspection failed: {error}")),
        },
        ChatCommand::Sessions(subcommand) => {
            handle_sessions_command(app, cfg, session_service, &subcommand).await;
        }
        ChatCommand::NewSession(session_id) => {
            let session_id = if session_id.trim().is_empty() {
                format!(
                    "session-{}",
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_millis()
                )
            } else {
                session_id.trim().to_string()
            };
            cfg.session_id = session_id.clone();
            match crate::session::ensure_session_exists(session_service, cfg).await {
                Ok(()) => {
                    app.messages.clear();
                    app.activities.clear();
                    app.context_percent = 0;
                    app.push_system(format!("Started session `{session_id}`."));
                }
                Err(error) => app.push_system(format!("Could not create session: {error}")),
            }
        }
        ChatCommand::Compact => {
            app.active_agent = "compacting".into();
            match crate::compact::compact_session(
                session_service,
                cfg,
                &crate::compact::CompactStrategy::default(),
            )
            .await
            {
                Ok(Some(summary)) => app.push_system(format!(
                    "## Conversation compacted\n\n{}",
                    crate::text::truncate(&summary, 2000, "…")
                )),
                Ok(None) => app.push_system("Conversation is too short to compact."),
                Err(error) => app.push_system(format!("Compaction failed: {error}")),
            }
            app.active_agent = "idle".into();
        }
        ChatCommand::Checkpoint(subcommand) => {
            handle_checkpoint_command(app, cfg, session_service, checkpoint_store, &subcommand)
                .await;
        }
        ChatCommand::Tangent(subcommand) => {
            handle_tangent_command(app, cfg, session_service, checkpoint_store, &subcommand).await;
        }
        ChatCommand::Todos(subcommand) => handle_todos_command(app, &subcommand),
        ChatCommand::Undo => match crate::file_history::undo_last() {
            Ok(message) => app.push_system(format!("✓ {message}")),
            Err(error) => app.push_system(format!("Cannot undo: {error}")),
        },
        ChatCommand::Allow(pattern) => {
            crate::tools::confirming::trust_tool(&pattern);
            app.push_system(format!("Session rule added: always allow `{pattern}`."));
        }
        ChatCommand::Deny(pattern) => {
            crate::tools::confirming::deny_tool(&pattern);
            app.push_system(format!(
                "Session rule added: always deny `{pattern}`. Deny rules override allow rules."
            ));
        }
        ChatCommand::Agent => {
            if crate::tools::confirming::is_agent_mode() {
                app.push_system("Agent mode is already active.");
            } else {
                app.approval = Some(PendingApproval {
                    agent: app.active_agent.clone(),
                    tool: "agent mode".into(),
                    detail:
                        "Trust fs_read, fs_write, and execute_bash for the rest of this session?"
                            .into(),
                    response: None,
                    enables_agent_mode: true,
                });
            }
        }
        ChatCommand::AutoCompact => {
            cfg.auto_compact_enabled = !cfg.auto_compact_enabled;
            app.push_system(format!(
                "Auto-compaction {} at {:.0}% utilization (target {:.0}%).",
                if cfg.auto_compact_enabled {
                    "enabled"
                } else {
                    "disabled"
                },
                cfg.compaction_threshold * 100.0,
                cfg.compaction_target * 100.0
            ));
        }
        ChatCommand::Provider(provider_name) => {
            let next_provider = match crate::provider::parse_provider_name(&provider_name) {
                Ok(provider) => provider,
                Err(error) => {
                    app.push_system(format!("Provider switch failed: {error}"));
                    return false;
                }
            };
            let mut switched = cfg.clone();
            switched.provider = next_provider;
            switched.worker_provider = next_provider;
            switched.worker_model = crate::model_catalog::default_model(
                next_provider,
                crate::model_catalog::ModelRole::Worker,
            )
            .to_string();
            switched.model = Some(switched.worker_model.clone());
            switch_runtime(
                app,
                runner,
                cfg,
                provider,
                model_name,
                session_service,
                runtime_tools,
                confirmation,
                telemetry,
                switched,
                "worker provider",
            )
            .await;
        }
        ChatCommand::Model(next_model) | ChatCommand::Worker(next_model) => {
            let Some(next_model) = next_model else {
                app.push_system(format_models_markdown(cfg));
                return false;
            };
            let mut switched = cfg.clone();
            switched.model = Some(next_model.clone());
            switched.worker_model = next_model;
            switch_runtime(
                app,
                runner,
                cfg,
                provider,
                model_name,
                session_service,
                runtime_tools,
                confirmation,
                telemetry,
                switched,
                "worker model",
            )
            .await;
        }
        ChatCommand::PlannerProvider(provider_name) => {
            let next_provider = match crate::provider::parse_provider_name(&provider_name) {
                Ok(crate::cli::Provider::Auto) => {
                    app.push_system("Planner provider must be explicit.");
                    return false;
                }
                Ok(provider) => provider,
                Err(error) => {
                    app.push_system(format!("Planner switch failed: {error}"));
                    return false;
                }
            };
            let mut switched = cfg.clone();
            switched.planner_provider = next_provider;
            switched.planner_model = crate::model_catalog::default_model(
                next_provider,
                crate::model_catalog::ModelRole::Planner,
            )
            .to_string();
            switch_runtime(
                app,
                runner,
                cfg,
                provider,
                model_name,
                session_service,
                runtime_tools,
                confirmation,
                telemetry,
                switched,
                "planner provider",
            )
            .await;
        }
        ChatCommand::Planner(next_model) => {
            let Some(next_model) = next_model else {
                app.push_system(format_models_markdown(cfg));
                return false;
            };
            let mut switched = cfg.clone();
            switched.planner_model = next_model;
            switch_runtime(
                app,
                runner,
                cfg,
                provider,
                model_name,
                session_service,
                runtime_tools,
                confirmation,
                telemetry,
                switched,
                "planner model",
            )
            .await;
        }
        ChatCommand::Time(query) => {
            if query.trim().is_empty() {
                let context = crate::agents::time::TimeAgent::handshake();
                app.push_system(format!(
                    "## Time\n\n- **Current:** `{}`\n- **Timezone:** `{}`\n- **Weekday:** {}",
                    context.now_iso, context.timezone, context.weekday
                ));
            } else {
                match crate::agents::time::TimeAgent::parse_relative(&query) {
                    Ok(time) => app.push_system(format!("`{query}` → `{}`", time.to_rfc3339())),
                    Err(error) => app.push_system(format!("Time parsing failed: {error}")),
                }
            }
        }
        ChatCommand::Memory(subcommand) => handle_memory_command(app, &subcommand).await,
        ChatCommand::Delegate(task) => {
            let parsed = crate::todos::parse_delegate_request(task.trim());
            let Ok((agent, delegate_task)) = parsed else {
                app.push_system(format!("Invalid delegate command: {}", parsed.unwrap_err()));
                return true;
            };
            if delegate_task.trim().is_empty() {
                app.push_system("Usage: `/delegate [@agent|--agent NAME] <task>`.");
            } else {
                app.busy = true;
                app.active_agent = agent.clone().unwrap_or_else(|| "delegate".into());
                let cfg = cfg.clone();
                let tools = runtime_tools.clone();
                let confirmation = confirmation.clone();
                let telemetry = telemetry.clone();
                let sessions = session_service.clone();
                let tx = tx.clone();
                let task = tokio::spawn(async move {
                    let result = crate::todos::run_named_delegate(
                        delegate_task.trim(),
                        agent.as_deref(),
                        &cfg,
                        sessions,
                        &tools,
                        &confirmation,
                        &telemetry,
                    )
                    .await;
                    let _ = tx.send(UiEvent::System(result.format_display()));
                    let _ = tx.send(UiEvent::Completed(String::new()));
                });
                app.task_abort = Some(task.abort_handle());
            }
        }
        ChatCommand::Parallel(request) => {
            let request = match crate::subagents::parse_parallel_request(&request) {
                Ok(request) => request,
                Err(error) => {
                    app.push_system(format!(
                        "Invalid parallel command: {error}\n\nUsage: `/parallel @AGENT [@AGENT ...] <task>`."
                    ));
                    return false;
                }
            };
            let agents = match crate::subagents::resolve_parallel_agents(cfg, &request.agents) {
                Ok(agents) => agents,
                Err(error) => {
                    app.push_system(format!("Cannot start parallel agents: {error}"));
                    return false;
                }
            };
            app.busy = true;
            app.active_agent = format!("parallel · {} agents", agents.len());
            app.push_system(format!(
                "Started {} governed subagents with concurrency {}. Press `Esc` to cancel; child session IDs remain available through `/sessions`.",
                agents.len(),
                request.max_concurrency.min(agents.len())
            ));
            let cfg = cfg.clone();
            let tools = runtime_tools.clone();
            let sessions = session_service.clone();
            let telemetry = telemetry.clone();
            let tx = tx.clone();
            let progress_tx = tx.clone();
            let progress: crate::subagents::ParallelProgress = Arc::new(move |event| match event {
                crate::subagents::ParallelProgressEvent::Started {
                    index,
                    agent,
                    session_id,
                } => {
                    let _ = progress_tx.send(UiEvent::ToolStarted {
                        call_id: Some(format!("subagent-{index}")),
                        name: format!("agent:{agent}"),
                        detail: format!("session {session_id}"),
                    });
                }
                crate::subagents::ParallelProgressEvent::Finished(result) => {
                    let detail = if result.success {
                        format!(
                            "completed in {} ms · {}",
                            result.duration_ms, result.session_id
                        )
                    } else {
                        result
                            .error
                            .clone()
                            .unwrap_or_else(|| "subagent failed".to_string())
                    };
                    let _ = progress_tx.send(UiEvent::ToolFinished {
                        call_id: Some(format!("subagent-{}", result.index)),
                        name: format!("agent:{}", result.agent),
                        success: result.success,
                        detail,
                    });
                    let _ = progress_tx.send(UiEvent::System(
                        crate::subagents::format_result_markdown(&result),
                    ));
                }
            });
            let task = tokio::task::spawn_local(async move {
                let results = crate::subagents::run_parallel(
                    agents,
                    &request.task,
                    crate::subagents::ParallelRunContext {
                        base_cfg: &cfg,
                        parent_tools: &tools,
                        retrieval,
                        telemetry: &telemetry,
                        max_concurrency: request.max_concurrency,
                        progress_format: crate::cli::OutputFormat::Json,
                        progress: Some(progress),
                        session_service: Some(sessions),
                    },
                )
                .await;
                let completed = results.iter().filter(|result| result.success).count();
                let _ = tx.send(UiEvent::System(format!(
                    "Parallel run finished: {completed}/{} agents completed successfully.",
                    results.len()
                )));
                let _ = tx.send(UiEvent::Completed(String::new()));
            });
            app.task_abort = Some(task.abort_handle());
        }
        ChatCommand::Ralph(prompt) => {
            if prompt.trim().is_empty() {
                app.push_system("Usage: `/ralph <goal>`.");
            } else {
                app.busy = true;
                app.active_agent = "ralph".into();
                let cfg = cfg.clone();
                let telemetry = telemetry.clone();
                let tx = tx.clone();
                let task = tokio::task::spawn_local(async move {
                    let retrieval = crate::retrieval::DisabledRetrievalService;
                    let message = match crate::ralph::run_ralph(
                        &cfg,
                        prompt,
                        crate::ralph::RalphRunOptions {
                            phase: None,
                            resume: false,
                            output_dir: None,
                            output_format: crate::cli::OutputFormat::Text,
                            always_approve: false,
                        },
                        &telemetry,
                        &retrieval,
                    )
                    .await
                    {
                        Ok(()) => "Ralph pipeline completed.".to_string(),
                        Err(error) => format!("Ralph pipeline failed: {error}"),
                    };
                    let _ = tx.send(UiEvent::System(message));
                    let _ = tx.send(UiEvent::Completed(String::new()));
                });
                app.task_abort = Some(task.abort_handle());
            }
        }
        ChatCommand::Spawn(request) => {
            let request = match crate::agent_supervisor::parse_spawn_request(&request) {
                Ok(request) => request,
                Err(error) => {
                    app.push_system(format!(
                        "Invalid spawn command: {error}\n\nUsage: `/spawn @AGENT [--worktree] <task>`."
                    ));
                    return false;
                }
            };
            if request.agent == "ralph"
                || !cfg
                    .available_agents
                    .iter()
                    .any(|agent| agent.name == request.agent)
            {
                app.push_system(format!(
                    "Agent `{}` is not an available specialist.",
                    request.agent
                ));
                return false;
            }
            match crate::agent_supervisor::AgentRunStore::open_default().await {
                Ok(store) => {
                    let created = store
                        .create_run(
                            crate::agent_supervisor::NewAgentRun {
                                parent_run_id: std::env::var("ZAVORA_SUPERVISOR_RUN_ID").ok(),
                                parent_session_id: cfg.session_id.clone(),
                                agent: request.agent,
                                task: request.task,
                                workspace: std::env::current_dir().unwrap_or_default(),
                                worktree: request.worktree,
                                retry_of: None,
                            },
                            &crate::agent_supervisor::SupervisorPolicy::default(),
                        )
                        .await;
                    match created {
                        Ok(run) => {
                            match crate::agent_supervisor::launch_worker(
                                &store,
                                &run,
                                crate::agent_supervisor::LaunchOptions {
                                    cfg,
                                    always_approve: false,
                                },
                            )
                            .await
                            {
                                Ok(_) => app.push_system(format!(
                                    "Started `{}` as durable run `{}`. It continues in the background; `/runs` shows live state.",
                                    run.agent, run.id
                                )),
                                Err(error) => app.push_system(format!(
                                    "Could not launch background agent: {error}"
                                )),
                            }
                        }
                        Err(error) => {
                            app.push_system(format!("Could not create background agent: {error}"))
                        }
                    }
                }
                Err(error) => app.push_system(format!("Agent supervisor unavailable: {error}")),
            }
        }
        ChatCommand::Runs => match crate::agent_supervisor::AgentRunStore::open_default().await {
            Ok(store) => match store.list(50).await {
                Ok(runs) => app.push_system(crate::agent_supervisor::format_runs_markdown(&runs)),
                Err(error) => app.push_system(format!("Agent run inspection failed: {error}")),
            },
            Err(error) => app.push_system(format!("Agent supervisor unavailable: {error}")),
        },
        ChatCommand::AgentSend(request) => {
            let Some((run_id, message)) = request.split_once(char::is_whitespace) else {
                app.push_system("Usage: `/send RUN_ID <message>`.");
                return false;
            };
            match crate::agent_supervisor::AgentRunStore::open_default().await {
                Ok(store) => match store.prepare_continuation(run_id, message.trim()).await {
                    Ok(continuation) => {
                        let result = if continuation.launch_required {
                            crate::agent_supervisor::launch_worker(
                                &store,
                                &continuation.run,
                                crate::agent_supervisor::LaunchOptions {
                                    cfg,
                                    always_approve: false,
                                },
                            )
                            .await
                            .map(|_| ())
                        } else {
                            Ok(())
                        };
                        match result {
                            Ok(()) => app.push_system(format!("Queued follow-up for `{run_id}`.")),
                            Err(error) => app
                                .push_system(format!("Could not resume agent `{run_id}`: {error}")),
                        }
                    }
                    Err(error) => {
                        app.push_system(format!("Could not send to agent `{run_id}`: {error}"))
                    }
                },
                Err(error) => app.push_system(format!("Agent supervisor unavailable: {error}")),
            }
        }
        ChatCommand::AgentCancel(run_id) => {
            let run_id = run_id.trim();
            match crate::agent_supervisor::AgentRunStore::open_default().await {
                Ok(store) => match crate::agent_supervisor::cancel_run(&store, run_id).await {
                    Ok(run) => app.push_system(format!(
                        "Agent run `{}` is {}.",
                        run.id,
                        run.status.label()
                    )),
                    Err(error) => {
                        app.push_system(format!("Could not cancel agent `{run_id}`: {error}"))
                    }
                },
                Err(error) => app.push_system(format!("Agent supervisor unavailable: {error}")),
            }
        }
    }
    false
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn switch_runtime(
    app: &mut App,
    runner: &mut Arc<Runner>,
    cfg: &mut RuntimeConfig,
    provider: &mut crate::cli::Provider,
    model_name: &mut String,
    session_service: &Arc<dyn adk_session::SessionService>,
    runtime_tools: &ResolvedRuntimeTools,
    confirmation: &ToolConfirmationSettings,
    telemetry: &TelemetrySink,
    switched: RuntimeConfig,
    route: &str,
) {
    app.active_agent = "switching route".into();
    match build_single_runner_for_chat(
        &switched,
        session_service.clone(),
        runtime_tools,
        confirmation,
        telemetry,
    )
    .await
    {
        Ok((next_runner, next_provider, next_model)) => {
            *runner = Arc::new(next_runner);
            *provider = next_provider;
            *model_name = next_model.clone();
            *cfg = switched;
            cfg.provider = next_provider;
            cfg.model = Some(next_model.clone());
            cfg.worker_provider = next_provider;
            cfg.worker_model = next_model;
            app.push_system(format!(
                "Switched {route}.\n\n- **Worker:** {} / `{}`\n- **Planner:** {} / `{}`\n- **Session:** preserved",
                cfg.worker_provider, cfg.worker_model, cfg.planner_provider, cfg.planner_model
            ));
        }
        Err(error) => app.push_system(format!(
            "Could not switch {route}; the previous route remains active.\n\n`{}`",
            crate::error::format_cli_error(&error, cfg.show_sensitive_config)
        )),
    }
    app.active_agent = "idle".into();
}

pub(super) async fn handle_checkpoint_command(
    app: &mut App,
    cfg: &RuntimeConfig,
    session_service: &Arc<dyn adk_session::SessionService>,
    store: &mut CheckpointStore,
    subcommand: &str,
) {
    let parts = subcommand.split_whitespace().collect::<Vec<_>>();
    match parts.first().copied() {
        Some("save") => match snapshot_session_events(session_service, cfg).await {
            Ok(events) => {
                let label = parts
                    .get(1..)
                    .map(|parts| parts.join(" "))
                    .unwrap_or_default();
                let checkpoint = store.save(&label, events).clone();
                let workspace = std::env::current_dir().unwrap_or_default();
                match store.save_to_disk(&workspace) {
                    Ok(()) => app.push_system(format!(
                        "Saved checkpoint **[{}] {}** with {} events.",
                        checkpoint.tag,
                        checkpoint.label,
                        checkpoint.events.len()
                    )),
                    Err(error) => app.push_system(format!(
                        "Checkpoint was created in memory but could not be persisted: {error}"
                    )),
                }
            }
            Err(error) => app.push_system(format!("Checkpoint failed: {error}")),
        },
        Some("list") | None => app.push_system(format_checkpoint_list(store)),
        Some("restore") => {
            let Some(tag) = parts.get(1).and_then(|tag| tag.parse::<usize>().ok()) else {
                app.push_system("Usage: `/checkpoint restore TAG`.");
                return;
            };
            let Some(events) = store.get(tag).map(|checkpoint| checkpoint.events.clone()) else {
                app.push_system(format!("No checkpoint with tag `{tag}`."));
                return;
            };
            match restore_session_events(session_service, cfg, &events).await {
                Ok(()) => {
                    app.messages.clear();
                    app.activities.clear();
                    app.push_system(format!(
                        "Restored checkpoint **[{tag}]**. The visible transcript was cleared to match the restored session."
                    ));
                }
                Err(error) => app.push_system(format!("Checkpoint restore failed: {error}")),
            }
        }
        _ => app.push_system(
            "Usage: `/checkpoint save [label]`, `/checkpoint list`, or `/checkpoint restore TAG`.",
        ),
    }
}

pub(super) async fn handle_sessions_command(
    app: &mut App,
    cfg: &mut RuntimeConfig,
    session_service: &Arc<dyn adk_session::SessionService>,
    subcommand: &str,
) {
    let parts = subcommand.split_whitespace().collect::<Vec<_>>();
    let requested = match parts.as_slice() {
        [] | ["list"] => None,
        ["switch", session_id, ..] => Some(*session_id),
        [session_id, ..] => Some(*session_id),
    };
    if let Some(session_id) = requested {
        match session_service
            .get(adk_session::GetRequest {
                app_name: cfg.app_name.clone(),
                user_id: cfg.user_id.clone(),
                session_id: session_id.to_string(),
                num_recent_events: None,
                after: None,
            })
            .await
        {
            Ok(session) => {
                cfg.session_id = session_id.to_string();
                app.messages = session
                    .events()
                    .all()
                    .into_iter()
                    .filter_map(|event| {
                        let text = crate::streaming::event_text(&event);
                        (!text.trim().is_empty()).then(|| {
                            let role = if event.author == "user" {
                                "YOU".to_string()
                            } else {
                                event.author.to_ascii_uppercase()
                            };
                            Message::new(role, text)
                        })
                    })
                    .collect();
                app.activities.clear();
                app.scroll_to_end();
                app.current_assistant = None;
                app.push_system(format!("Switched to session `{session_id}`."));
            }
            Err(error) => app.push_system(format!(
                "Could not switch to session `{session_id}`: {error}"
            )),
        }
        return;
    }

    match session_service
        .list(adk_session::ListRequest {
            app_name: cfg.app_name.clone(),
            user_id: cfg.user_id.clone(),
            limit: None,
            offset: None,
        })
        .await
    {
        Ok(mut sessions) => {
            sessions.sort_by_key(|session| std::cmp::Reverse(session.last_update_time()));
            let mut output = String::from("## Sessions\n\n");
            if sessions.is_empty() {
                output.push_str("No persisted sessions were found.");
            } else {
                for session in sessions {
                    output.push_str(&format!(
                        "- {} `{}` — {}\n",
                        if session.id() == cfg.session_id {
                            "**active**"
                        } else {
                            "available"
                        },
                        session.id(),
                        session.last_update_time().to_rfc3339()
                    ));
                }
                output.push_str("\nUse `/sessions switch ID` to load one.");
            }
            app.push_system(output);
        }
        Err(error) => app.push_system(format!("Session listing failed: {error}")),
    }
}

/// Put text on the system clipboard.
///
/// The full-screen workspace claims mouse reporting so the wheel can scroll,
/// which takes the terminal's own click-drag selection away. Rather than force a
/// choice between scrolling and copying, the app can hand text over directly.
///
/// Uses the platform tool rather than a clipboard crate: no new dependency, and
/// it works over SSH where a linked X11/Wayland clipboard would not.
pub(super) fn copy_to_clipboard(text: &str) -> anyhow::Result<&'static str> {
    use std::io::Write;
    use std::process::{Command, Stdio};

    // Ordered by platform likelihood; the first present binary wins.
    let candidates: &[(&str, &[&str])] = if cfg!(target_os = "macos") {
        &[("pbcopy", &[])]
    } else if cfg!(target_os = "windows") {
        &[("clip", &[])]
    } else {
        &[
            ("wl-copy", &[]),
            ("xclip", &["-selection", "clipboard"]),
            ("xsel", &["--clipboard", "--input"]),
        ]
    };

    for (binary, args) in candidates {
        let Ok(mut child) = Command::new(binary)
            .args(*args)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
        else {
            continue; // not installed; try the next
        };

        if let Some(stdin) = child.stdin.as_mut() {
            stdin
                .write_all(text.as_bytes())
                .with_context(|| format!("failed to write to {binary}"))?;
        }
        let status = child
            .wait()
            .with_context(|| format!("failed to run {binary}"))?;
        if status.success() {
            return Ok(binary);
        }
    }

    anyhow::bail!(
        "no clipboard tool available (looked for {}). Use `/export` to write the transcript to a file instead.",
        candidates
            .iter()
            .map(|(binary, _)| *binary)
            .collect::<Vec<_>>()
            .join(", ")
    )
}

/// The text `/copy` should place on the clipboard.
///
/// Defaults to the most recent assistant response, which is what a developer
/// almost always wants: the code or answer just produced. `all` takes the whole
/// transcript.
pub(super) fn clipboard_payload(app: &App, whole_transcript: bool) -> Option<String> {
    if whole_transcript {
        let body = app
            .messages
            .iter()
            .filter(|message| message.role != ELIDED_ROLE)
            .map(|message| format!("## {}\n\n{}", message.role, message.text))
            .collect::<Vec<_>>()
            .join("\n\n");
        return (!body.trim().is_empty()).then_some(body);
    }

    app.messages
        .iter()
        .rev()
        .find(|message| message.role != "YOU" && message.role != ELIDED_ROLE)
        .map(|message| message.text.clone())
        .filter(|text| !text.trim().is_empty())
}

pub(super) fn export_transcript(
    app: &App,
    cfg: &RuntimeConfig,
    requested_path: &str,
) -> Result<std::path::PathBuf> {
    let path = if requested_path.is_empty() {
        std::path::PathBuf::from(".zavora")
            .join("exports")
            .join(format!("{}.md", cfg.session_id))
    } else {
        std::path::PathBuf::from(requested_path)
    };
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("failed to create export directory '{}'", parent.display()))?;
    }
    let markdown = format_transcript_markdown(
        app,
        &cfg.session_id,
        &cfg.profile,
        &cfg.worker_provider.to_string(),
        &cfg.worker_model,
    );
    std::fs::write(&path, markdown)
        .with_context(|| format!("failed to write transcript '{}'", path.display()))?;
    Ok(path)
}

pub(super) fn format_transcript_markdown(
    app: &App,
    session_id: &str,
    profile: &str,
    provider: &str,
    model: &str,
) -> String {
    let mut markdown = format!(
        "# Zavora session {}\n\nProfile: `{}`  \nWorker: {} / `{}`\n\n",
        session_id, profile, provider, model
    );
    for message in &app.messages {
        markdown.push_str(&format!("## {}\n\n{}\n\n", message.role, message.text));
    }
    markdown
}

pub(super) async fn handle_tangent_command(
    app: &mut App,
    cfg: &RuntimeConfig,
    session_service: &Arc<dyn adk_session::SessionService>,
    store: &mut CheckpointStore,
    subcommand: &str,
) {
    if store.in_tangent() {
        let events = if subcommand.trim() == "tail" {
            match snapshot_session_events(session_service, cfg).await {
                Ok(current) => store.exit_tangent_tail(&current),
                Err(error) => {
                    app.push_system(format!("Could not inspect tangent session: {error}"));
                    return;
                }
            }
        } else {
            store.exit_tangent()
        };
        let Some(events) = events else {
            app.push_system("The tangent baseline is unavailable.");
            return;
        };
        match restore_session_events(session_service, cfg, &events).await {
            Ok(()) => {
                app.messages.clear();
                app.activities.clear();
                let workspace = std::env::current_dir().unwrap_or_default();
                if let Err(error) = store.save_to_disk(&workspace) {
                    app.push_system(format!(
                        "Tangent state changed but could not be persisted: {error}"
                    ));
                }
                app.push_system(if subcommand.trim() == "tail" {
                    "Exited tangent mode and retained the latest exchange."
                } else {
                    "Exited tangent mode and restored the baseline conversation."
                });
            }
            Err(error) => app.push_system(format!("Tangent restore failed: {error}")),
        }
    } else {
        match snapshot_session_events(session_service, cfg).await {
            Ok(events) => {
                let tag = store.enter_tangent(events);
                let workspace = std::env::current_dir().unwrap_or_default();
                if let Err(error) = store.save_to_disk(&workspace) {
                    app.push_system(format!(
                        "Entered tangent mode at [{tag}], but persistence failed: {error}"
                    ));
                } else {
                    app.push_system(format!(
                        "Entered tangent mode at checkpoint **[{tag}]**. Use `/tangent` to discard the branch or `/tangent tail` to retain its latest exchange."
                    ));
                }
            }
            Err(error) => app.push_system(format!("Could not enter tangent mode: {error}")),
        }
    }
}

pub(super) fn handle_todos_command(app: &mut App, subcommand: &str) {
    let workspace = std::env::current_dir().unwrap_or_default();
    let parts = subcommand.split_whitespace().collect::<Vec<_>>();
    let result = match parts.first().copied() {
        Some("view") => parts
            .get(1)
            .ok_or_else(|| anyhow::anyhow!("Usage: /todos view ID"))
            .and_then(|id| crate::todos::load_todo(&workspace, id))
            .map(|todo| todo.format_display()),
        Some("delete") => parts
            .get(1)
            .ok_or_else(|| anyhow::anyhow!("Usage: /todos delete ID"))
            .and_then(|id| {
                crate::todos::delete_todo(&workspace, id)?;
                Ok(format!("Deleted todo `{id}`."))
            }),
        Some("clear-finished") => crate::todos::clear_finished_todos(&workspace)
            .map(|count| format!("Cleared {count} finished todo list(s).")),
        _ => crate::todos::format_todos_summary(&workspace),
    };
    match result {
        Ok(output) => app.push_system(output),
        Err(error) => app.push_system(format!("Todo command failed: {error}")),
    }
}

pub(super) async fn handle_memory_command(app: &mut App, subcommand: &str) {
    let parts = subcommand.split_whitespace().collect::<Vec<_>>();
    let argument = parts
        .get(1..)
        .map(|parts| parts.join(" "))
        .unwrap_or_default();
    match parts.first().copied() {
        Some("recall") => match crate::agents::memory::recall(&argument, 10).await {
            Ok(memories) if memories.is_empty() => {
                app.push_system(format!("No memories found for `{argument}`."));
            }
            Ok(memories) => app.push_system(format!(
                "## Recalled memories\n\n{}",
                memories
                    .iter()
                    .map(|memory| format!("- {memory}"))
                    .collect::<Vec<_>>()
                    .join("\n")
            )),
            Err(error) => app.push_system(format!("Memory recall failed: {error}")),
        },
        Some("remember") if !argument.is_empty() => {
            match crate::agents::memory::remember(&argument).await {
                Ok(()) => app.push_system("Memory stored."),
                Err(error) => app.push_system(format!("Memory storage failed: {error}")),
            }
        }
        Some("forget") if !argument.is_empty() => {
            match crate::agents::memory::forget(&argument).await {
                Ok(count) => app.push_system(format!("Removed {count} memories.")),
                Err(error) => app.push_system(format!("Memory removal failed: {error}")),
            }
        }
        _ => app.push_system("Usage: `/memory recall|remember|forget <text>`."),
    }
}

pub(super) fn format_status_markdown(cfg: &RuntimeConfig, app: &App) -> String {
    format!(
        "## Workspace status\n\n- **Profile:** `{}`\n- **Agent:** `{}` ({})\n- **Mode:** {}\n- **Worker:** {} / `{}`\n- **Planner:** {} / `{}`\n- **Session:** `{}`\n- **Context:** {}%\n- **Auto-compact:** {}\n- **Run history:** {}\n- **Mouse capture:** {}",
        cfg.profile,
        cfg.agent_name,
        cfg.agent_source.label(),
        app.mode.label(),
        cfg.worker_provider,
        cfg.worker_model,
        cfg.planner_provider,
        cfg.planner_model,
        cfg.session_id,
        app.context_percent,
        if cfg.auto_compact_enabled {
            "on"
        } else {
            "off"
        },
        app.activity_visibility.label(),
        if app.mouse_capture {
            "on (wheel scrolls; `/mouse` to select text)"
        } else {
            "off (mouse selection available)"
        }
    )
}

pub(super) fn format_tools_markdown(
    cfg: &RuntimeConfig,
    runtime_tools: &ResolvedRuntimeTools,
) -> String {
    let mut names = runtime_tools
        .tools()
        .iter()
        .map(|tool| tool.name().to_string())
        .collect::<Vec<_>>();
    names.sort();
    let mut output = format!(
        "## Runtime tools\n\n- **Available:** {}\n- **MCP tools:** {}\n- **Confirmation mode:** `{:?}`\n\n",
        names.len(),
        runtime_tools.mcp_tool_names().len(),
        cfg.tool_confirmation_mode
    );
    for name in names {
        let source = if runtime_tools.mcp_tool_names().contains(&name) {
            "MCP"
        } else {
            "built-in"
        };
        output.push_str(&format!("- `{name}` — {source}\n"));
    }
    output
}

pub(super) fn format_inspect_markdown(
    cfg: &RuntimeConfig,
    runtime_tools: &ResolvedRuntimeTools,
) -> String {
    let configured = cfg
        .mcp_servers
        .iter()
        .map(|server| server.name.clone())
        .collect::<Vec<_>>();
    let instruction_status = match crate::skills::resolve_workspace_instructions() {
        Ok(instructions) => format!(
            "{} active / {} deferred",
            instructions.sources.len(),
            instructions.deferred_sources.len()
        ),
        Err(error) => format!("unavailable ({error})"),
    };
    format!(
        "## Runtime inspection\n\n- **Profile:** `{}`\n- **Agent:** `{}`\n- **Worker:** {} / `{}`\n- **Planner:** {} / `{}`\n- **Session backend:** `{:?}`\n- **MCP:** {} configured / {} connected tools\n- **Instructions:** {}\n- **Capabilities:** `{}`\n\n{}",
        cfg.profile,
        cfg.agent_name,
        cfg.worker_provider,
        cfg.worker_model,
        cfg.planner_provider,
        cfg.planner_model,
        cfg.session_backend,
        cfg.mcp_servers.len(),
        runtime_tools.mcp_tool_names().len(),
        instruction_status,
        crate::capabilities::state_path().display(),
        crate::capabilities::format_catalog_markdown_with_runtime(
            &configured,
            &runtime_tools
                .mcp_tool_names()
                .iter()
                .cloned()
                .collect::<Vec<_>>(),
        )
    )
}

pub(super) fn format_models_markdown(cfg: &RuntimeConfig) -> String {
    let mut output = format!(
        "## Model routes\n\n- **Worker:** {} / `{}`\n- **Planner:** {} / `{}`\n\n",
        cfg.worker_provider, cfg.worker_model, cfg.planner_provider, cfg.planner_model
    );
    let models = crate::model_catalog::models_for_provider(cfg.worker_provider);
    if models.is_empty() {
        output.push_str("This provider accepts model identifiers directly. Use `/model MODEL`.\n");
    } else {
        output.push_str("### Selectable worker models\n\n");
        for model in models {
            output.push_str(&format!(
                "- `{}` — {} · {}\n",
                model.id,
                model.recommended_role.label(),
                model.description
            ));
        }
    }
    output
}

pub(super) fn format_context_markdown(usage: &crate::context::ContextUsage) -> String {
    let total = usage.total_tokens();
    let remaining = usage.context_window_tokens.saturating_sub(total);
    format!(
        "## Context usage\n\n- **Used:** {total} / {} tokens ({})\n- **Remaining:** {remaining} tokens\n- **Events:** {}\n- **User:** ~{} tokens\n- **Assistant:** ~{} tokens\n- **Tools:** ~{} tokens",
        usage.context_window_tokens,
        usage.prompt_indicator(),
        usage.event_count,
        crate::context::estimate_tokens(usage.user_chars),
        crate::context::estimate_tokens(usage.assistant_chars),
        crate::context::estimate_tokens(usage.tool_chars)
    )
}

pub(super) fn run_shell_command(
    app: &mut App,
    cfg: &RuntimeConfig,
    command: String,
    session_service: Arc<dyn adk_session::SessionService>,
    telemetry: TelemetrySink,
    tx: tokio::sync::mpsc::UnboundedSender<UiEvent>,
) {
    app.push_message(Message::new("YOU", format!("$ {command}")));
    app.activities.clear();
    app.activities.push(Activity {
        call_id: Some("direct-shell".into()),
        name: "execute_bash".into(),
        detail: command.clone(),
        state: ActivityState::Running,
        started: Instant::now(),
        elapsed: None,
    });
    app.busy = true;
    app.active_agent = "shell".into();
    telemetry.emit_content(
        "tui.shell.started",
        serde_json::json!({ "command": command }),
    );
    let session_id = cfg.session_id.clone();
    let task = tokio::spawn(async move {
        let shell = std::env::var("SHELL").unwrap_or_else(|_| "sh".into());
        let output = tokio::process::Command::new(&shell)
            .arg("-lc")
            .arg(&command)
            .current_dir(std::env::current_dir().unwrap_or_default())
            .output()
            .await;
        match output {
            Ok(output) => {
                let success = output.status.success();
                let stdout = String::from_utf8_lossy(&output.stdout);
                let stderr = String::from_utf8_lossy(&output.stderr);
                let combined = match (stdout.trim().is_empty(), stderr.trim().is_empty()) {
                    (false, false) => format!("{stdout}\n{stderr}"),
                    (false, true) => stdout.into_owned(),
                    (true, false) => stderr.into_owned(),
                    (true, true) => "(command produced no output)".into(),
                };
                let detail = format!(
                    "exit {}",
                    output
                        .status
                        .code()
                        .map_or_else(|| "signal".into(), |code| code.to_string())
                );
                let _ = tx.send(UiEvent::ToolFinished {
                    call_id: Some("direct-shell".into()),
                    name: "execute_bash".into(),
                    success,
                    detail,
                });
                let _ = tx.send(UiEvent::System(format!(
                    "## Shell output\n\n```text\n{}\n```",
                    crate::text::truncate(&combined, 50_000, "\n… output truncated")
                )));
                let mut event = Event::new("tui-shell");
                event.author = "user".into();
                event.llm_response.content = Some(Content::new("user").with_text(format!(
                    "Direct shell command: {command}\nExit status: {}\nOutput:\n{}",
                    output
                        .status
                        .code()
                        .map_or_else(|| "signal".into(), |code| code.to_string()),
                    crate::text::truncate(&combined, 20_000, "\n… output truncated")
                )));
                if let Err(error) = session_service.append_event(&session_id, event).await {
                    let _ = tx.send(UiEvent::System(format!(
                        "Shell output was shown but could not be added to agent context: {error}"
                    )));
                }
                telemetry.emit(
                    "tui.shell.completed",
                    serde_json::json!({
                        "session_id": session_id,
                        "success": success,
                        "exit_code": output.status.code(),
                    }),
                );
            }
            Err(error) => {
                let _ = tx.send(UiEvent::ToolFinished {
                    call_id: Some("direct-shell".into()),
                    name: "execute_bash".into(),
                    success: false,
                    detail: error.to_string(),
                });
                let _ = tx.send(UiEvent::System(format!("Shell command failed: {error}")));
            }
        }
        let _ = tx.send(UiEvent::Completed(String::new()));
    });
    app.task_abort = Some(task.abort_handle());
}
