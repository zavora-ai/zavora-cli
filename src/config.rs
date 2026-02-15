use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::cli::*;
use crate::guardrail::default_guardrail_terms;

#[derive(Debug, Clone)]
pub struct RuntimeConfig {
    pub profile: String,
    pub config_path: String,
    pub agent_name: String,
    pub agent_source: AgentSource,
    pub agent_description: Option<String>,
    pub agent_instruction: Option<String>,
    pub agent_resource_paths: Vec<String>,
    pub agent_allow_tools: Vec<String>,
    pub agent_deny_tools: Vec<String>,
    pub provider: Provider,
    pub model: Option<String>,
    pub app_name: String,
    pub user_id: String,
    pub session_id: String,
    pub session_backend: SessionBackend,
    pub session_db_url: String,
    pub show_sensitive_config: bool,
    pub retrieval_backend: RetrievalBackend,
    pub retrieval_doc_path: Option<String>,
    pub retrieval_max_chunks: usize,
    pub retrieval_max_chars: usize,
    pub retrieval_min_score: usize,
    pub tool_confirmation_mode: ToolConfirmationMode,
    pub require_confirm_tool: Vec<String>,
    pub approve_tool: Vec<String>,
    pub tool_timeout_secs: u64,
    pub tool_retry_attempts: u32,
    pub tool_retry_delay_ms: u64,
    pub telemetry_enabled: bool,
    pub telemetry_path: String,
    pub guardrail_input_mode: GuardrailMode,
    pub guardrail_output_mode: GuardrailMode,
    pub guardrail_terms: Vec<String>,
    pub guardrail_redact_replacement: String,
    pub mcp_servers: Vec<McpServerConfig>,
    pub max_prompt_chars: usize,
    pub server_runner_cache_max: usize,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProfilesFile {
    #[serde(default)]
    pub profiles: HashMap<String, ProfileConfig>,
}

#[derive(Debug, Default, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProfileConfig {
    pub provider: Option<Provider>,
    pub model: Option<String>,
    pub app_name: Option<String>,
    pub user_id: Option<String>,
    pub session_id: Option<String>,
    pub session_backend: Option<SessionBackend>,
    pub session_db_url: Option<String>,
    pub retrieval_backend: Option<RetrievalBackend>,
    pub retrieval_doc_path: Option<String>,
    pub retrieval_max_chunks: Option<usize>,
    pub retrieval_max_chars: Option<usize>,
    pub retrieval_min_score: Option<usize>,
    pub tool_confirmation_mode: Option<ToolConfirmationMode>,
    #[serde(default)]
    pub require_confirm_tool: Vec<String>,
    #[serde(default)]
    pub approve_tool: Vec<String>,
    pub tool_timeout_secs: Option<u64>,
    pub tool_retry_attempts: Option<u32>,
    pub tool_retry_delay_ms: Option<u64>,
    pub telemetry_enabled: Option<bool>,
    pub telemetry_path: Option<String>,
    pub guardrail_input_mode: Option<GuardrailMode>,
    pub guardrail_output_mode: Option<GuardrailMode>,
    #[serde(default)]
    pub guardrail_terms: Vec<String>,
    pub guardrail_redact_replacement: Option<String>,
    #[serde(default)]
    pub mcp_servers: Vec<McpServerConfig>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentSource {
    Implicit,
    Global,
    Local,
}

impl AgentSource {
    pub fn label(self) -> &'static str {
        match self {
            AgentSource::Implicit => "implicit",
            AgentSource::Global => "global",
            AgentSource::Local => "local",
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentFileConfig {
    pub description: Option<String>,
    pub instruction: Option<String>,
    pub provider: Option<Provider>,
    pub model: Option<String>,
    pub tool_confirmation_mode: Option<ToolConfirmationMode>,
    #[serde(default)]
    pub resource_paths: Vec<String>,
    #[serde(default)]
    pub allow_tools: Vec<String>,
    #[serde(default)]
    pub deny_tools: Vec<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentCatalogFile {
    #[serde(default)]
    pub agents: HashMap<String, AgentFileConfig>,
}

#[derive(Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AgentSelectionFile {
    pub agent: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ResolvedAgent {
    pub name: String,
    pub source: AgentSource,
    pub config: AgentFileConfig,
}

#[derive(Debug, Clone)]
pub struct AgentPaths {
    pub local_catalog: PathBuf,
    pub global_catalog: Option<PathBuf>,
    pub selection_file: PathBuf,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct McpServerConfig {
    pub name: String,
    pub endpoint: String,
    pub enabled: Option<bool>,
    pub timeout_secs: Option<u64>,
    pub auth_bearer_env: Option<String>,
    #[serde(default)]
    pub tool_allowlist: Vec<String>,
    #[serde(default)]
    pub tool_aliases: HashMap<String, String>,
}

pub fn load_profiles(config_path: &str) -> Result<ProfilesFile> {
    let path = Path::new(config_path);
    if !path.exists() {
        return Ok(ProfilesFile::default());
    }

    let content = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read profile config file at '{}'", path.display()))?;
    toml::from_str::<ProfilesFile>(&content).with_context(|| {
        format!(
            "invalid profile configuration in '{}'. Check provider/session values and field names.",
            path.display()
        )
    })
}

pub fn default_agent_paths() -> AgentPaths {
    let local_catalog = PathBuf::from(".zavora/agents.toml");
    let selection_file = PathBuf::from(".zavora/agent-selection.toml");
    let global_catalog = std::env::var("HOME")
        .ok()
        .map(PathBuf::from)
        .map(|home| home.join(".zavora/agents.toml"));
    AgentPaths {
        local_catalog,
        global_catalog,
        selection_file,
    }
}

pub fn load_agent_catalog_file(path: &Path) -> Result<AgentCatalogFile> {
    if !path.exists() {
        return Ok(AgentCatalogFile::default());
    }

    let content = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read agent catalog file at '{}'", path.display()))?;
    toml::from_str::<AgentCatalogFile>(&content).with_context(|| {
        format!(
            "invalid agent catalog configuration in '{}'. Check field names and provider/tool settings.",
            path.display()
        )
    })
}

pub fn load_resolved_agents(paths: &AgentPaths) -> Result<HashMap<String, ResolvedAgent>> {
    let mut resolved = implicit_agent_map();

    if let Some(global_path) = paths.global_catalog.as_ref() {
        let global = load_agent_catalog_file(global_path)?;
        for (name, config) in global.agents {
            resolved.insert(
                name.clone(),
                ResolvedAgent {
                    name,
                    source: AgentSource::Global,
                    config,
                },
            );
        }
    }

    let local = load_agent_catalog_file(&paths.local_catalog)?;
    for (name, config) in local.agents {
        resolved.insert(
            name.clone(),
            ResolvedAgent {
                name,
                source: AgentSource::Local,
                config,
            },
        );
    }

    Ok(resolved)
}

pub fn implicit_agent_map() -> HashMap<String, ResolvedAgent> {
    let mut resolved = HashMap::<String, ResolvedAgent>::new();
    resolved.insert(
        "default".to_string(),
        ResolvedAgent {
            name: "default".to_string(),
            source: AgentSource::Implicit,
            config: AgentFileConfig {
                description: Some("Built-in default assistant".to_string()),
                instruction: None,
                provider: None,
                model: None,
                tool_confirmation_mode: None,
                resource_paths: Vec::new(),
                allow_tools: Vec::new(),
                deny_tools: Vec::new(),
            },
        },
    );
    resolved
}

pub fn load_agent_selection(path: &Path) -> Result<Option<String>> {
    if !path.exists() {
        return Ok(None);
    }
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read agent selection file '{}'", path.display()))?;
    let parsed = toml::from_str::<AgentSelectionFile>(&content)
        .with_context(|| format!("invalid agent selection config '{}'", path.display()))?;
    Ok(parsed
        .agent
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty()))
}

pub fn resolve_active_agent_name(
    cli: &Cli,
    agents: &HashMap<String, ResolvedAgent>,
    selected_agent: Option<&str>,
) -> Result<String> {
    if let Some(requested) = cli.agent.as_deref() {
        let trimmed = requested.trim();
        if agents.contains_key(trimmed) {
            return Ok(trimmed.to_string());
        }
        let mut names = agents.keys().cloned().collect::<Vec<String>>();
        names.sort();
        return Err(anyhow::anyhow!(
            "agent '{}' not found. Available agents: {}",
            trimmed,
            names.join(", ")
        ));
    }

    if let Some(selected) = selected_agent
        .map(str::trim)
        .filter(|value| !value.is_empty())
        && agents.contains_key(selected)
    {
        return Ok(selected.to_string());
    }

    if agents.contains_key("default") {
        return Ok("default".to_string());
    }

    let mut names = agents.keys().cloned().collect::<Vec<String>>();
    names.sort();
    names.into_iter().next().ok_or_else(|| {
        anyhow::anyhow!(
            "no agents available. Add '.zavora/agents.toml' or '~/.zavora/agents.toml'."
        )
    })
}

pub fn persist_agent_selection(path: &Path, agent_name: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).with_context(|| {
            format!(
                "failed to create agent selection directory '{}'",
                parent.display()
            )
        })?;
    }
    let payload = toml::to_string(&AgentSelectionFile {
        agent: Some(agent_name.to_string()),
    })
    .context("failed to serialize agent selection file")?;
    std::fs::write(path, payload)
        .with_context(|| format!("failed to write agent selection file '{}'", path.display()))
}

fn merge_unique_names(first: &[String], second: &[String]) -> Vec<String> {
    let mut seen = BTreeSet::<String>::new();
    let mut merged = Vec::<String>::new();

    for name in first.iter().chain(second.iter()) {
        let trimmed = name.trim();
        if trimmed.is_empty() {
            continue;
        }
        if seen.insert(trimmed.to_string()) {
            merged.push(trimmed.to_string());
        }
    }

    merged
}

pub fn resolve_runtime_config_with_agents(
    cli: &Cli,
    profiles: &ProfilesFile,
    resolved_agents: &HashMap<String, ResolvedAgent>,
    selected_agent_name: Option<&str>,
) -> Result<RuntimeConfig> {
    let selected = cli.profile.trim();
    if selected.is_empty() {
        return Err(anyhow::anyhow!(
            "profile name cannot be empty. Set --profile <name>."
        ));
    }

    let profile = if selected == "default" && !profiles.profiles.contains_key("default") {
        ProfileConfig::default()
    } else {
        profiles.profiles.get(selected).cloned().ok_or_else(|| {
            let mut names = profiles.profiles.keys().cloned().collect::<Vec<String>>();
            names.sort();
            if names.is_empty() {
                anyhow::anyhow!(
                    "profile '{}' not found in '{}'. No profiles are defined yet.",
                    selected,
                    cli.config_path
                )
            } else {
                anyhow::anyhow!(
                    "profile '{}' not found in '{}'. Available profiles: {}",
                    selected,
                    cli.config_path,
                    names.join(", ")
                )
            }
        })?
    };

    let active_agent_name = resolve_active_agent_name(cli, resolved_agents, selected_agent_name)?;
    let active_agent = resolved_agents.get(&active_agent_name).ok_or_else(|| {
        anyhow::anyhow!("resolved active agent '{}' is missing", active_agent_name)
    })?;

    let provider = if cli.provider != Provider::Auto {
        cli.provider
    } else {
        active_agent
            .config
            .provider
            .or(profile.provider)
            .unwrap_or(Provider::Auto)
    };

    let require_confirm_tool =
        merge_unique_names(&profile.require_confirm_tool, &cli.require_confirm_tool);
    let approve_tool = merge_unique_names(&profile.approve_tool, &cli.approve_tool);
    let guardrail_terms = {
        let merged = merge_unique_names(&profile.guardrail_terms, &cli.guardrail_term);
        if merged.is_empty() {
            default_guardrail_terms()
        } else {
            merged
        }
    };
    let mcp_servers = profile.mcp_servers.clone();

    Ok(RuntimeConfig {
        profile: selected.to_string(),
        config_path: cli.config_path.clone(),
        agent_name: active_agent.name.clone(),
        agent_source: active_agent.source,
        agent_description: active_agent.config.description.clone(),
        agent_instruction: active_agent.config.instruction.clone(),
        agent_resource_paths: active_agent.config.resource_paths.clone(),
        agent_allow_tools: active_agent.config.allow_tools.clone(),
        agent_deny_tools: active_agent.config.deny_tools.clone(),
        provider,
        model: cli
            .model
            .clone()
            .or(active_agent.config.model.clone())
            .or(profile.model),
        app_name: cli
            .app_name
            .clone()
            .or(profile.app_name)
            .unwrap_or_else(|| "zavora-cli".to_string()),
        user_id: cli
            .user_id
            .clone()
            .or(profile.user_id)
            .unwrap_or_else(|| "local-user".to_string()),
        session_id: cli
            .session_id
            .clone()
            .or(profile.session_id)
            .unwrap_or_else(|| "default-session".to_string()),
        session_backend: cli
            .session_backend
            .or(profile.session_backend)
            .unwrap_or(SessionBackend::Memory),
        session_db_url: cli
            .session_db_url
            .clone()
            .or(profile.session_db_url)
            .unwrap_or_else(|| "sqlite://.zavora/sessions.db".to_string()),
        show_sensitive_config: cli.show_sensitive_config,
        retrieval_backend: cli
            .retrieval_backend
            .or(profile.retrieval_backend)
            .unwrap_or(RetrievalBackend::Disabled),
        retrieval_doc_path: cli
            .retrieval_doc_path
            .clone()
            .or(profile.retrieval_doc_path),
        retrieval_max_chunks: cli
            .retrieval_max_chunks
            .or(profile.retrieval_max_chunks)
            .unwrap_or(3)
            .max(1),
        retrieval_max_chars: cli
            .retrieval_max_chars
            .or(profile.retrieval_max_chars)
            .unwrap_or(4000)
            .max(256),
        retrieval_min_score: cli
            .retrieval_min_score
            .or(profile.retrieval_min_score)
            .unwrap_or(1),
        tool_confirmation_mode: cli
            .tool_confirmation_mode
            .or(active_agent.config.tool_confirmation_mode)
            .or(profile.tool_confirmation_mode)
            .unwrap_or(ToolConfirmationMode::McpOnly),
        require_confirm_tool,
        approve_tool,
        tool_timeout_secs: cli
            .tool_timeout_secs
            .or(profile.tool_timeout_secs)
            .unwrap_or(45)
            .max(1),
        tool_retry_attempts: cli
            .tool_retry_attempts
            .or(profile.tool_retry_attempts)
            .unwrap_or(2)
            .max(1),
        tool_retry_delay_ms: cli
            .tool_retry_delay_ms
            .or(profile.tool_retry_delay_ms)
            .unwrap_or(500),
        telemetry_enabled: cli
            .telemetry_enabled
            .or(profile.telemetry_enabled)
            .unwrap_or(true),
        telemetry_path: cli
            .telemetry_path
            .clone()
            .or(profile.telemetry_path)
            .unwrap_or_else(|| ".zavora/telemetry/events.jsonl".to_string()),
        guardrail_input_mode: cli
            .guardrail_input_mode
            .or(profile.guardrail_input_mode)
            .unwrap_or(GuardrailMode::Disabled),
        guardrail_output_mode: cli
            .guardrail_output_mode
            .or(profile.guardrail_output_mode)
            .unwrap_or(GuardrailMode::Disabled),
        guardrail_terms,
        guardrail_redact_replacement: cli
            .guardrail_redact_replacement
            .clone()
            .or(profile.guardrail_redact_replacement)
            .unwrap_or_else(|| "[REDACTED]".to_string()),
        mcp_servers,
        max_prompt_chars: 32_000,
        server_runner_cache_max: 64,
    })
}

#[cfg(test)]
pub fn resolve_runtime_config(cli: &Cli, profiles: &ProfilesFile) -> Result<RuntimeConfig> {
    let resolved_agents = implicit_agent_map();
    resolve_runtime_config_with_agents(cli, profiles, &resolved_agents, None)
}

pub fn display_session_db_url(cfg: &RuntimeConfig) -> String {
    if cfg.show_sensitive_config {
        cfg.session_db_url.clone()
    } else {
        format!(
            "{} (set --show-sensitive-config to reveal)",
            crate::error::redact_sqlite_url_value(&cfg.session_db_url)
        )
    }
}
