use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use clap::Parser;
use serde_json::json;
use tracing::level_filters::LevelFilter;

use zavora_cli::agent_catalog::*;
use zavora_cli::chat::*;
use zavora_cli::cli::*;
use zavora_cli::config::*;
use zavora_cli::doctor::*;
use zavora_cli::error::*;
use zavora_cli::eval::*;
use zavora_cli::guardrail::*;
use zavora_cli::mcp::*;
use zavora_cli::profiles::*;
use zavora_cli::provider::*;
use zavora_cli::retrieval::*;
use zavora_cli::runner::*;
use zavora_cli::server::*;
use zavora_cli::session::*;
use zavora_cli::streaming::*;
use zavora_cli::telemetry::*;
use zavora_cli::workflow::*;

fn init_tracing(log_filter: &str) -> Result<()> {
    let level = log_filter
        .parse::<LevelFilter>()
        .unwrap_or(LevelFilter::INFO);
    tracing_subscriber::fmt()
        .with_max_level(level)
        .with_env_filter(log_filter)
        .with_target(false)
        .try_init()
        .map_err(|e| anyhow::anyhow!("failed to initialize tracing subscriber: {e}"))
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let show_sensitive_config = cli.show_sensitive_config;
    if let Err(err) = run_cli(cli).await {
        eprintln!("{}", format_cli_error(&err, show_sensitive_config));
        tracing::error!(
            category = %categorize_error(&err).code(),
            error = %render_error_message(&err, show_sensitive_config),
            "command failed"
        );
        std::process::exit(1);
    }

    Ok(())
}

async fn run_cli(cli: Cli) -> Result<()> {
    init_tracing(&cli.log_filter)?;
    let profiles = load_profiles(&cli.config_path)?;
    let agent_paths = default_agent_paths();
    let resolved_agents = load_resolved_agents(&agent_paths)?;
    let selected_agent_name = load_agent_selection(&agent_paths.selection_file)?;
    let cfg = resolve_runtime_config_with_agents(
        &cli,
        &profiles,
        &resolved_agents,
        selected_agent_name.as_deref(),
    )?;
    let command = command_label(cli.command.as_ref().unwrap_or(&Commands::Chat));
    let telemetry = TelemetrySink::new(&cfg, command.clone());
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
            let tool_confirmation = resolve_tool_confirmation_settings(&cfg, &runtime_tools);
            let agent = build_single_agent_with_tools(
                model,
                &runtime_tools.tools,
                tool_confirmation.policy,
                Duration::from_secs(cfg.tool_timeout_secs),
                Some(&cfg),
            )?;
            let runner =
                build_runner_with_run_config(agent, &cfg, Some(tool_confirmation.run_config))
                    .await?;
            let prompt = prompt.join(" ");
            enforce_prompt_limit(&prompt, cfg.max_prompt_chars)?;
            let prompt =
                apply_guardrail(&cfg, &telemetry, "input", cfg.guardrail_input_mode, &prompt)?;
            let retrieval = retrieval_service
                .as_deref()
                .context("retrieval service should be initialized for ask command")?;
            let answer =
                run_prompt_with_retrieval(&runner, &cfg, &prompt, retrieval, &telemetry).await?;
            let answer = apply_guardrail(
                &cfg,
                &telemetry,
                "output",
                cfg.guardrail_output_mode,
                &answer,
            )?;
            println!("{answer}");
            Ok(())
        }
        Commands::Chat => {
            let runtime_tools = resolve_runtime_tools(&cfg).await;
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
        Commands::Workflow {
            mode,
            prompt,
            max_iterations,
        } => {
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
            let tool_confirmation = resolve_tool_confirmation_settings(&cfg, &runtime_tools);
            let agent = build_workflow_agent(
                mode,
                model,
                max_iterations,
                &runtime_tools.tools,
                tool_confirmation.policy,
                Duration::from_secs(cfg.tool_timeout_secs),
                Some(&cfg),
            )?;
            let runner =
                build_runner_with_run_config(agent, &cfg, Some(tool_confirmation.run_config))
                    .await?;
            let prompt = prompt.join(" ");
            enforce_prompt_limit(&prompt, cfg.max_prompt_chars)?;
            let prompt =
                apply_guardrail(&cfg, &telemetry, "input", cfg.guardrail_input_mode, &prompt)?;
            let retrieval = retrieval_service
                .as_deref()
                .context("retrieval service should be initialized for workflow command")?;
            let answer =
                run_prompt_with_retrieval(&runner, &cfg, &prompt, retrieval, &telemetry).await?;
            let answer = apply_guardrail(
                &cfg,
                &telemetry,
                "output",
                cfg.guardrail_output_mode,
                &answer,
            )?;
            println!("{answer}");
            Ok(())
        }
        Commands::ReleasePlan { goal, releases } => {
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
            let agent = build_release_planning_agent(model, releases)?;
            let runner = build_runner(agent, &cfg).await?;
            let prompt = goal.join(" ");
            let prompt =
                apply_guardrail(&cfg, &telemetry, "input", cfg.guardrail_input_mode, &prompt)?;
            let retrieval = retrieval_service
                .as_deref()
                .context("retrieval service should be initialized for release-plan command")?;
            let answer =
                run_prompt_with_retrieval(&runner, &cfg, &prompt, retrieval, &telemetry).await?;
            let answer = apply_guardrail(
                &cfg,
                &telemetry,
                "output",
                cfg.guardrail_output_mode,
                &answer,
            )?;
            println!("{answer}");
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
            AgentCommands::List => {
                run_agents_list(&resolved_agents, &cfg.agent_name, &agent_paths)?;
                Ok(())
            }
            AgentCommands::Show { name } => {
                run_agents_show(&resolved_agents, &cfg.agent_name, name)?;
                Ok(())
            }
            AgentCommands::Select { name } => {
                run_agents_select(&resolved_agents, &agent_paths, name)?;
                Ok(())
            }
        },
        Commands::Mcp { command } => match command {
            McpCommands::List => {
                run_mcp_list(&cfg).await?;
                Ok(())
            }
            McpCommands::Discover { server } => {
                run_mcp_discover(&cfg, server).await?;
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

    execution
}
