use std::collections::HashMap;

use anyhow::Result;

use crate::config::{AgentPaths, ResolvedAgent, persist_agent_selection};

pub fn run_agents_list(
    agents: &HashMap<String, ResolvedAgent>,
    active_agent: &str,
    paths: &AgentPaths,
    json_output: bool,
) -> Result<()> {
    let mut names = agents.keys().cloned().collect::<Vec<String>>();
    names.sort();

    if json_output {
        let records = names
            .iter()
            .filter_map(|name| agents.get(name))
            .map(agent_json)
            .collect::<Vec<_>>();
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "active_agent": active_agent,
                "agents": records,
                "catalogs": {
                    "local": paths.local_catalog,
                    "global": paths.global_catalog,
                    "selection": paths.selection_file,
                    "local_markdown_roots": paths.local_markdown_roots,
                    "global_markdown_roots": paths.global_markdown_roots,
                }
            }))?
        );
        return Ok(());
    }

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
    json_output: bool,
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

    if json_output {
        println!("{}", serde_json::to_string_pretty(&agent_json(agent))?);
        return Ok(());
    }

    println!("Agent: {} (source={})", agent.name, agent.source.label());
    println!(
        "Definition: {}",
        agent
            .definition_path
            .as_ref()
            .map(|path| path.display().to_string())
            .unwrap_or_else(|| "<built-in>".to_string())
    );
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
        if let Some(category) = crate::agents::capability::specialist_category(&agent.name) {
            format!("<category-routed {}>", category.label())
        } else if agent.config.allow_tools.is_empty() {
            "<all parent-permitted>".to_string()
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
        "Allow skills: {}",
        display_list(&agent.config.skills, "<all enabled>")
    );
    println!(
        "Deny skills: {}",
        display_list(&agent.config.deny_skills, "<none>")
    );
    println!(
        "Allow child agents: {}",
        display_list(&agent.config.agents, "<all configured specialists>")
    );
    println!(
        "Deny child agents: {}",
        display_list(&agent.config.deny_agents, "<none>")
    );
    println!(
        "Max turns: {}",
        agent
            .config
            .max_turns
            .map(|value| value.to_string())
            .unwrap_or_else(|| "<runtime default>".to_string())
    );
    println!(
        "Timeout: {}",
        agent
            .config
            .timeout_secs
            .map(|value| format!("{value}s"))
            .unwrap_or_else(|| "<runtime default>".to_string())
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

fn display_list(values: &[String], empty: &str) -> String {
    if values.is_empty() {
        empty.to_string()
    } else {
        values.join(", ")
    }
}

fn agent_json(agent: &ResolvedAgent) -> serde_json::Value {
    let tool_scope = crate::agents::capability::specialist_category(&agent.name)
        .map(|category| format!("category-routed:{}", category.label()))
        .unwrap_or_else(|| {
            if agent.config.allow_tools.is_empty() {
                "all-parent-permitted".to_string()
            } else {
                "allowlist".to_string()
            }
        });
    serde_json::json!({
        "name": agent.name,
        "description": agent.config.description,
        "source": agent.source.label(),
        "definition_path": agent.definition_path,
        "provider": agent.config.provider.map(|provider| format!("{provider:?}").to_ascii_lowercase()),
        "model": agent.config.model,
        "instruction_configured": agent.config.instruction.as_ref().is_some_and(|value| !value.trim().is_empty()),
        "tool_confirmation_mode": agent.config.tool_confirmation_mode.map(|mode| format!("{mode:?}").to_ascii_lowercase()),
        "allow_tools": agent.config.allow_tools,
        "deny_tools": agent.config.deny_tools,
        "tool_scope": tool_scope,
        "allow_skills": agent.config.skills,
        "deny_skills": agent.config.deny_skills,
        "allow_agents": agent.config.agents,
        "deny_agents": agent.config.deny_agents,
        "resource_paths": agent.config.resource_paths,
        "max_turns": agent.config.max_turns,
        "timeout_secs": agent.config.timeout_secs,
        "coordinator_callable": agent.name != "default" && agent.name != "ralph",
    })
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
