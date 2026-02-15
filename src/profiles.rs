use anyhow::Result;

use crate::config::{ProfilesFile, RuntimeConfig, display_session_db_url};

pub fn run_profiles_list(profiles: &ProfilesFile, cfg: &RuntimeConfig) -> Result<()> {
    let mut names = profiles.profiles.keys().cloned().collect::<Vec<String>>();
    if !names.iter().any(|name| name == "default") {
        names.push("default".to_string());
    }
    names.sort();

    println!("Configured profiles (active='{}'):", cfg.profile);
    for name in names {
        let marker = if name == cfg.profile { "*" } else { " " };
        let source = if profiles.profiles.contains_key(&name) {
            "configured"
        } else {
            "implicit"
        };
        println!("{marker} {name} ({source})");
    }

    Ok(())
}

pub fn run_profiles_show(cfg: &RuntimeConfig) -> Result<()> {
    println!("Active profile: {}", cfg.profile);
    println!("Config path: {}", cfg.config_path);
    println!("Provider: {:?}", cfg.provider);
    println!(
        "Model: {}",
        cfg.model.as_deref().unwrap_or("<provider-default>")
    );
    println!("App: {}", cfg.app_name);
    println!("User: {}", cfg.user_id);
    println!(
        "Agent: {} (source={})",
        cfg.agent_name,
        cfg.agent_source.label()
    );
    println!(
        "Agent description: {}",
        cfg.agent_description.as_deref().unwrap_or("<none>")
    );
    println!(
        "Agent resources: {}",
        if cfg.agent_resource_paths.is_empty() {
            "<none>".to_string()
        } else {
            cfg.agent_resource_paths.join(", ")
        }
    );
    println!("Session ID: {}", cfg.session_id);
    println!("Session backend: {:?}", cfg.session_backend);
    println!("Session DB URL: {}", display_session_db_url(cfg));
    println!("Retrieval backend: {:?}", cfg.retrieval_backend);
    println!(
        "Retrieval doc path: {}",
        cfg.retrieval_doc_path
            .as_deref()
            .unwrap_or("<not configured>")
    );
    println!("Retrieval max chunks: {}", cfg.retrieval_max_chunks);
    println!("Retrieval max chars: {}", cfg.retrieval_max_chars);
    println!("Retrieval min score: {}", cfg.retrieval_min_score);
    println!("Tool confirmation mode: {:?}", cfg.tool_confirmation_mode);
    println!(
        "Tool confirmation required list: {}",
        if cfg.require_confirm_tool.is_empty() {
            "<none>".to_string()
        } else {
            cfg.require_confirm_tool.join(", ")
        }
    );
    println!(
        "Tool approval list: {}",
        if cfg.approve_tool.is_empty() {
            "<none>".to_string()
        } else {
            cfg.approve_tool.join(", ")
        }
    );
    println!("Tool timeout (secs): {}", cfg.tool_timeout_secs);
    println!("Tool retry attempts: {}", cfg.tool_retry_attempts);
    println!("Tool retry delay (ms): {}", cfg.tool_retry_delay_ms);
    println!("Telemetry enabled: {}", cfg.telemetry_enabled);
    println!("Telemetry path: {}", cfg.telemetry_path);
    println!(
        "Guardrails: input_mode={:?} output_mode={:?} terms={} redact_replacement={}",
        cfg.guardrail_input_mode,
        cfg.guardrail_output_mode,
        cfg.guardrail_terms.len(),
        cfg.guardrail_redact_replacement
    );
    println!("MCP servers: {}", cfg.mcp_servers.len());
    Ok(())
}
