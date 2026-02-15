use std::collections::HashMap;

use anyhow::Result;

use crate::config::{AgentPaths, ResolvedAgent, persist_agent_selection};

pub fn run_agents_list(
    agents: &HashMap<String, ResolvedAgent>,
    active_agent: &str,
    paths: &AgentPaths,
) -> Result<()> {
    let mut names = agents.keys().cloned().collect::<Vec<String>>();
    names.sort();

    println!("Available agents (active='{}'):", active_agent);
    for name in names {
        let marker = if name == active_agent { "*" } else { " " };
        let source = agents
            .get(&name)
            .map(|agent| agent.source.label())
            .unwrap_or("unknown");
        println!("{marker} {name} ({source})");
    }
    println!("Local catalog: {}", paths.local_catalog.display());
    if let Some(global) = paths.global_catalog.as_ref() {
        println!("Global catalog: {}", global.display());
    } else {
        println!("Global catalog: <HOME not set>");
    }
    println!("Selection file: {}", paths.selection_file.display());
    Ok(())
}

pub fn run_agents_show(
    agents: &HashMap<String, ResolvedAgent>,
    active_agent: &str,
    requested_name: Option<String>,
) -> Result<()> {
    let name = requested_name.unwrap_or_else(|| active_agent.to_string());
    let agent = agents.get(&name).ok_or_else(|| {
        let mut names = agents.keys().cloned().collect::<Vec<String>>();
        names.sort();
        anyhow::anyhow!(
            "agent '{}' not found. Available agents: {}",
            name,
            names.join(", ")
        )
    })?;

    println!("Agent: {} (source={})", agent.name, agent.source.label());
    println!(
        "Description: {}",
        agent.config.description.as_deref().unwrap_or("<none>")
    );
    println!(
        "Instruction: {}",
        agent.config.instruction.as_deref().unwrap_or("<none>")
    );
    println!(
        "Provider override: {}",
        agent
            .config
            .provider
            .map(|p| format!("{:?}", p))
            .unwrap_or_else(|| "<none>".to_string())
    );
    println!(
        "Model override: {}",
        agent.config.model.as_deref().unwrap_or("<none>")
    );
    println!(
        "Tool confirmation mode override: {}",
        agent
            .config
            .tool_confirmation_mode
            .map(|mode| format!("{:?}", mode))
            .unwrap_or_else(|| "<none>".to_string())
    );
    println!(
        "Allow tools: {}",
        if agent.config.allow_tools.is_empty() {
            "<none>".to_string()
        } else {
            agent.config.allow_tools.join(", ")
        }
    );
    println!(
        "Deny tools: {}",
        if agent.config.deny_tools.is_empty() {
            "<none>".to_string()
        } else {
            agent.config.deny_tools.join(", ")
        }
    );
    println!(
        "Resource paths: {}",
        if agent.config.resource_paths.is_empty() {
            "<none>".to_string()
        } else {
            agent.config.resource_paths.join(", ")
        }
    );
    Ok(())
}

pub fn run_agents_select(
    agents: &HashMap<String, ResolvedAgent>,
    paths: &AgentPaths,
    name: String,
) -> Result<()> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err(anyhow::anyhow!("agent name cannot be empty"));
    }
    if !agents.contains_key(trimmed) {
        let mut names = agents.keys().cloned().collect::<Vec<String>>();
        names.sort();
        return Err(anyhow::anyhow!(
            "agent '{}' not found. Available agents: {}",
            trimmed,
            names.join(", ")
        ));
    }
    persist_agent_selection(&paths.selection_file, trimmed)?;
    println!(
        "Selected agent '{}' (selection file: {}).",
        trimmed,
        paths.selection_file.display()
    );
    Ok(())
}
