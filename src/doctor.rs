use anyhow::Result;

use crate::cli::SessionBackend;
use crate::config::{RuntimeConfig, display_session_db_url};
use crate::provider::{detect_provider, env_present};
use crate::session::open_sqlite_session_service;

pub async fn run_doctor(cfg: &RuntimeConfig) -> Result<()> {
    println!(
        "Active profile: '{}' (config: {})",
        cfg.profile, cfg.config_path
    );

    let checks = [
        ("GOOGLE_API_KEY", env_present("GOOGLE_API_KEY")),
        ("OPENAI_API_KEY", env_present("OPENAI_API_KEY")),
        ("ANTHROPIC_API_KEY", env_present("ANTHROPIC_API_KEY")),
        ("DEEPSEEK_API_KEY", env_present("DEEPSEEK_API_KEY")),
        ("GROQ_API_KEY", env_present("GROQ_API_KEY")),
        ("OLLAMA_HOST", env_present("OLLAMA_HOST")),
    ];

    println!("Provider environment check:");
    for (key, ok) in checks {
        let status = if ok { "set" } else { "missing" };
        println!("- {key}: {status}");
    }

    match detect_provider() {
        Some(provider) => println!("Auto provider resolution: {:?}", provider),
        None => {
            println!("Auto provider resolution: none");
            println!("Tip: export one provider key or run with --provider ollama");
        }
    }

    println!(
        "Session backend: {:?} (session_id: {}, app: {}, user: {})",
        cfg.session_backend, cfg.session_id, cfg.app_name, cfg.user_id
    );
    println!(
        "Agent: {} (source={}) model_override={} resources={}",
        cfg.agent_name,
        cfg.agent_source.label(),
        cfg.model.as_deref().unwrap_or("<provider-default>"),
        cfg.agent_resource_paths.len()
    );
    println!(
        "Retrieval: backend={:?}, doc_path={}, max_chunks={}, max_chars={}, min_score={}",
        cfg.retrieval_backend,
        cfg.retrieval_doc_path
            .as_deref()
            .unwrap_or("<not configured>"),
        cfg.retrieval_max_chunks,
        cfg.retrieval_max_chars,
        cfg.retrieval_min_score
    );
    println!(
        "Tool confirmation: mode={:?}, required_tools={}, approved_tools={}, timeout_secs={}, retry_attempts={}, retry_delay_ms={}",
        cfg.tool_confirmation_mode,
        cfg.require_confirm_tool.len(),
        cfg.approve_tool.len(),
        cfg.tool_timeout_secs,
        cfg.tool_retry_attempts,
        cfg.tool_retry_delay_ms
    );
    println!(
        "Telemetry: enabled={} path={}",
        cfg.telemetry_enabled, cfg.telemetry_path
    );
    println!(
        "Guardrails: input_mode={:?} output_mode={:?} terms={} redact_replacement={}",
        cfg.guardrail_input_mode,
        cfg.guardrail_output_mode,
        cfg.guardrail_terms.len(),
        cfg.guardrail_redact_replacement
    );
    println!(
        "MCP servers: configured={}, enabled={}",
        cfg.mcp_servers.len(),
        cfg.mcp_servers
            .iter()
            .filter(|server| server.enabled.unwrap_or(true))
            .count()
    );

    if matches!(cfg.session_backend, SessionBackend::Sqlite) {
        let _service = open_sqlite_session_service(&cfg.session_db_url).await?;
        println!(
            "SQLite session DB check: ok ({})",
            display_session_db_url(cfg)
        );
    }

    Ok(())
}

pub async fn run_migrate(cfg: &RuntimeConfig) -> Result<()> {
    match cfg.session_backend {
        SessionBackend::Memory => {
            println!("Session backend is memory; no migration required.");
        }
        SessionBackend::Sqlite => {
            let _service = open_sqlite_session_service(&cfg.session_db_url).await?;
            println!(
                "SQLite migrations applied successfully: {}",
                display_session_db_url(cfg)
            );
        }
    }
    Ok(())
}
