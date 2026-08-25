use std::collections::{BTreeMap, BTreeSet, HashMap};
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
    pub agent_allow_skills: Vec<String>,
    pub agent_deny_skills: Vec<String>,
    /// Child-agent names or wildcard patterns this agent may spawn. Empty means
    /// all configured specialists; deny rules always win.
    pub agent_allow_agents: Vec<String>,
    pub agent_deny_agents: Vec<String>,
    pub agent_max_turns: Option<u32>,
    pub agent_timeout_secs: Option<u64>,
    /// Resolved agent definitions available to the coordinator. These are
    /// configuration records, not connected or running agents.
    pub available_agents: Vec<ResolvedAgent>,
    /// Lifecycle hooks keyed by hook point, resolved from the active agent.
    ///
    /// Previously the agent config accepted a `hooks` table that nothing ever
    /// read, so documented configuration silently did nothing. Requirement 7.8.
    pub hooks: HashMap<crate::hooks::HookPoint, Vec<HookConfig>>,
    pub provider: Provider,
    pub model: Option<String>,
    pub worker_provider: Provider,
    pub worker_model: String,
    pub planner_provider: Provider,
    pub planner_model: String,
    pub planner_call_budget: u32,
    pub api_key: Option<String>,
    pub ollama_host: Option<String>,
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
    pub permission_rules: crate::tool_policy::PermissionRules,
    pub max_prompt_chars: usize,
    pub server_runner_cache_max: usize,
    pub auto_compact_enabled: bool,
    pub compact_interval: u32,
    pub compact_overlap: u32,
    pub compaction_threshold: f64,
    pub compaction_target: f64,
}

#[derive(Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProfilesFile {
    #[serde(default)]
    pub profiles: HashMap<String, ProfileConfig>,
}

#[derive(Debug, Default, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProfileConfig {
    pub provider: Option<Provider>,
    pub model: Option<String>,
    pub worker_provider: Option<Provider>,
    pub worker_model: Option<String>,
    pub planner_provider: Option<Provider>,
    pub planner_model: Option<String>,
    pub planner_call_budget: Option<u32>,
    pub api_key: Option<String>,
    pub ollama_host: Option<String>,
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
    #[serde(default)]
    pub permission_rules: crate::tool_policy::PermissionRules,
    pub compaction_threshold: Option<f64>,
    pub compaction_target: Option<f64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentSource {
    Implicit,
    Global,
    Local,
    Plugin,
}

impl AgentSource {
    pub fn label(self) -> &'static str {
        match self {
            AgentSource::Implicit => "implicit",
            AgentSource::Global => "global",
            AgentSource::Local => "local",
            AgentSource::Plugin => "plugin",
        }
    }
}

use crate::hooks::HookConfig;

#[derive(Debug, Clone, Default, Deserialize)]
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
    /// Skill names or wildcard patterns this agent may load. Empty means all
    /// enabled workspace skills; deny rules always win.
    #[serde(default)]
    pub skills: Vec<String>,
    #[serde(default)]
    pub deny_skills: Vec<String>,
    #[serde(default, alias = "subagents", alias = "allowed_agents")]
    pub agents: Vec<String>,
    #[serde(default, alias = "disallowed_agents")]
    pub deny_agents: Vec<String>,
    /// Bound model/tool iterations for this agent.
    pub max_turns: Option<u32>,
    /// Bound a delegated invocation independently of the parent turn.
    pub timeout_secs: Option<u64>,
    #[serde(default)]
    pub hooks: HashMap<String, Vec<HookConfig>>,
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
    pub definition_path: Option<PathBuf>,
    pub config: AgentFileConfig,
}

#[derive(Debug, Clone)]
pub struct AgentPaths {
    pub local_catalog: PathBuf,
    pub global_catalog: Option<PathBuf>,
    pub selection_file: PathBuf,
    pub local_markdown_roots: Vec<PathBuf>,
    pub global_markdown_roots: Vec<PathBuf>,
}

#[derive(Debug, Default, Deserialize)]
struct MarkdownAgentFrontmatter {
    name: Option<String>,
    description: Option<String>,
    provider: Option<Provider>,
    model: Option<String>,
    #[serde(default, alias = "tools", alias = "allowedTools")]
    allow_tools: StringList,
    #[serde(
        default,
        alias = "disallowedTools",
        alias = "denyTools",
        alias = "disallowed_tools"
    )]
    deny_tools: StringList,
    #[serde(default)]
    skills: StringList,
    #[serde(default, alias = "disallowedSkills")]
    deny_skills: StringList,
    #[serde(default, alias = "subagents", alias = "allowedAgents")]
    agents: StringList,
    #[serde(default, alias = "disallowedAgents")]
    deny_agents: StringList,
    #[serde(alias = "permissionMode", alias = "permission_mode")]
    tool_confirmation_mode: Option<ToolConfirmationMode>,
    #[serde(alias = "maxTurns", alias = "max_iterations")]
    max_turns: Option<u32>,
    #[serde(alias = "timeout", alias = "timeoutSeconds")]
    timeout_secs: Option<u64>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(untagged)]
enum StringList {
    #[default]
    Empty,
    One(String),
    Many(Vec<String>),
    Flags(BTreeMap<String, bool>),
}

impl StringList {
    fn into_vec(self) -> Vec<String> {
        let values = match self {
            Self::Empty => Vec::new(),
            Self::One(value) => value
                .split(',')
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
                .collect(),
            Self::Many(values) => values,
            Self::Flags(values) => values
                .into_iter()
                .filter_map(|(name, enabled)| enabled.then_some(name))
                .collect(),
        };
        values
            .into_iter()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .collect()
    }

    fn into_tools(self) -> Vec<String> {
        self.into_vec()
            .into_iter()
            .map(|value| normalize_imported_tool_name(&value))
            .collect()
    }
}

fn normalize_imported_tool_name(value: &str) -> String {
    match value.trim().to_ascii_lowercase().as_str() {
        "read" => "fs_read".to_string(),
        "write" => "fs_write".to_string(),
        "edit" => "file_edit".to_string(),
        "bash" | "shell" => "execute_bash".to_string(),
        "glob" => "glob".to_string(),
        "grep" => "grep".to_string(),
        "webfetch" | "web_fetch" => "web_fetch".to_string(),
        _ => value.trim().to_string(),
    }
}

fn normalize_agent_name(value: &str) -> String {
    value
        .trim()
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '_' | '-' | ':') {
                character.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect::<String>()
        .trim_matches('_')
        .to_string()
}

fn parse_markdown_agent(path: &Path, source: AgentSource) -> Result<Option<ResolvedAgent>> {
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read agent definition '{}'", path.display()))?;
    let (frontmatter, body) = if let Some(rest) = content.strip_prefix("---") {
        let (frontmatter, body) = rest.split_once("\n---").ok_or_else(|| {
            anyhow::anyhow!(
                "agent definition '{}' has an unterminated YAML frontmatter block",
                path.display()
            )
        })?;
        let parsed = serde_yaml::from_str::<MarkdownAgentFrontmatter>(frontmatter)
            .with_context(|| format!("invalid agent frontmatter in '{}'", path.display()))?;
        (parsed, body.trim_start_matches(['\r', '\n']).trim())
    } else {
        (MarkdownAgentFrontmatter::default(), content.trim())
    };
    if body.is_empty() {
        return Ok(None);
    }
    let name = frontmatter
        .name
        .as_deref()
        .map(normalize_agent_name)
        .filter(|name| !name.is_empty())
        .or_else(|| {
            path.file_stem()
                .and_then(|name| name.to_str())
                .map(normalize_agent_name)
                .filter(|name| !name.is_empty())
        })
        .ok_or_else(|| {
            anyhow::anyhow!("agent definition '{}' has no usable name", path.display())
        })?;
    let resource_paths = path
        .parent()
        .map(|parent| vec![parent.display().to_string()])
        .unwrap_or_default();
    Ok(Some(ResolvedAgent {
        name,
        source,
        definition_path: Some(path.to_path_buf()),
        config: AgentFileConfig {
            description: frontmatter.description,
            instruction: Some(body.to_string()),
            provider: frontmatter.provider,
            model: frontmatter.model,
            tool_confirmation_mode: frontmatter.tool_confirmation_mode,
            resource_paths,
            allow_tools: frontmatter.allow_tools.into_tools(),
            deny_tools: frontmatter.deny_tools.into_tools(),
            skills: frontmatter.skills.into_vec(),
            deny_skills: frontmatter.deny_skills.into_vec(),
            agents: frontmatter.agents.into_vec(),
            deny_agents: frontmatter.deny_agents.into_vec(),
            max_turns: frontmatter.max_turns,
            timeout_secs: frontmatter.timeout_secs,
            hooks: HashMap::new(),
        },
    }))
}

fn load_markdown_agents(
    roots: &[PathBuf],
    source: AgentSource,
) -> Result<HashMap<String, ResolvedAgent>> {
    let mut resolved = HashMap::new();
    for root in roots {
        if !root.is_dir() {
            continue;
        }
        let mut files = ignore::WalkBuilder::new(root)
            .max_depth(Some(3))
            .hidden(false)
            .build()
            .filter_map(std::result::Result::ok)
            .filter(|entry| entry.file_type().is_some_and(|kind| kind.is_file()))
            .map(|entry| entry.into_path())
            .filter(|path| {
                path.extension()
                    .and_then(|extension| extension.to_str())
                    .is_some_and(|extension| extension.eq_ignore_ascii_case("md"))
            })
            .collect::<Vec<_>>();
        files.sort();
        for path in files {
            if let Some(agent) = parse_markdown_agent(&path, source)? {
                resolved.insert(agent.name.clone(), agent);
            }
        }
    }
    Ok(resolved)
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct McpServerConfig {
    pub name: String,
    /// HTTP endpoint URL. Required for HTTP transport, omit for stdio.
    #[serde(default)]
    pub endpoint: String,
    /// Command to spawn for stdio transport. If set, uses stdio instead of HTTP.
    pub command: Option<String>,
    /// Arguments for the stdio command.
    #[serde(default)]
    pub args: Vec<String>,
    /// Environment variables for the stdio command.
    #[serde(default)]
    pub env: HashMap<String, String>,
    pub enabled: Option<bool>,
    pub timeout_secs: Option<u64>,
    pub auth_bearer_env: Option<String>,
    #[serde(default)]
    pub tool_allowlist: Vec<String>,
    #[serde(default)]
    pub tool_aliases: HashMap<String, String>,
    /// OAuth 2.0 config for authenticated MCP servers (feature: oauth).
    pub oauth: Option<crate::mcp_auth::McpOAuthConfig>,
}

impl McpServerConfig {
    /// Returns true if this server uses stdio transport.
    pub fn is_stdio(&self) -> bool {
        self.command.is_some()
    }

    /// Display string for the server's connection target.
    pub fn display_target(&self) -> &str {
        if let Some(cmd) = &self.command {
            cmd.as_str()
        } else {
            &self.endpoint
        }
    }
}

/// Re-read this profile's MCP servers from disk, returning how many are declared.
///
/// Only that one field is refreshed. A running session may have switched worker
/// or planner route since it started, and reloading the whole configuration would
/// silently discard those choices. Used after a capability is enabled, which
/// appends servers to the profile that the running tool surface has never seen.
pub fn reload_mcp_servers(cfg: &mut RuntimeConfig) -> Result<usize> {
    let profiles = load_profiles(&cfg.config_path)?;
    let servers = profiles
        .profiles
        .get(&cfg.profile)
        .map(|profile| profile.mcp_servers.clone())
        .unwrap_or_default();
    cfg.mcp_servers = servers;
    Ok(cfg.mcp_servers.len())
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
    let local_markdown_roots = [
        ".opencode/agents",
        ".gemini/agents",
        ".grok/agents",
        ".claude/agents",
        ".zavora/agents",
        ".agents/agents",
    ]
    .into_iter()
    .map(PathBuf::from)
    .collect();
    let global_markdown_roots = std::env::var("HOME")
        .ok()
        .map(PathBuf::from)
        .map(|home| {
            [
                ".config/opencode/agents",
                ".gemini/agents",
                ".grok/agents",
                ".claude/agents",
                ".zavora/agents",
                ".agents/agents",
            ]
            .into_iter()
            .map(|relative| home.join(relative))
            .collect()
        })
        .unwrap_or_default();
    AgentPaths {
        local_catalog,
        global_catalog,
        selection_file,
        local_markdown_roots,
        global_markdown_roots,
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

    // Lowest-to-highest precedence: portable global Markdown, global TOML,
    // portable project Markdown, project TOML. Within Markdown roots the
    // canonical `.agents/agents` directory is processed last.
    resolved.extend(load_markdown_agents(
        &paths.global_markdown_roots,
        AgentSource::Global,
    )?);

    if let Some(global_path) = paths.global_catalog.as_ref() {
        let global = load_agent_catalog_file(global_path)?;
        for (name, config) in global.agents {
            resolved.insert(
                name.clone(),
                ResolvedAgent {
                    name,
                    source: AgentSource::Global,
                    definition_path: Some(global_path.clone()),
                    config,
                },
            );
        }
    }

    resolved.extend(load_markdown_agents(
        &paths.local_markdown_roots,
        AgentSource::Local,
    )?);

    let local = load_agent_catalog_file(&paths.local_catalog)?;
    for (name, config) in local.agents {
        resolved.insert(
            name.clone(),
            ResolvedAgent {
                name,
                source: AgentSource::Local,
                definition_path: Some(paths.local_catalog.clone()),
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
            definition_path: None,
            config: AgentFileConfig {
                description: Some("Built-in default assistant".to_string()),
                instruction: None,
                provider: None,
                model: None,
                tool_confirmation_mode: None,
                resource_paths: Vec::new(),
                allow_tools: Vec::new(),
                deny_tools: Vec::new(),
                skills: Vec::new(),
                deny_skills: Vec::new(),
                agents: Vec::new(),
                deny_agents: Vec::new(),
                max_turns: None,
                timeout_secs: None,
                hooks: HashMap::new(),
            },
        },
    );
    resolved.insert(
        "ralph".to_string(),
        ResolvedAgent {
            name: "ralph".to_string(),
            source: AgentSource::Implicit,
            definition_path: None,
            config: AgentFileConfig {
                description: Some(
                    "Ralph autonomous development pipeline (PRD → Architect → Loop)".to_string(),
                ),
                instruction: None,
                provider: None,
                model: None,
                tool_confirmation_mode: None,
                resource_paths: Vec::new(),
                allow_tools: Vec::new(),
                deny_tools: Vec::new(),
                skills: Vec::new(),
                deny_skills: Vec::new(),
                agents: Vec::new(),
                deny_agents: Vec::new(),
                max_turns: None,
                timeout_secs: None,
                hooks: HashMap::new(),
            },
        },
    );
    for name in crate::agents::capability::specialist_names() {
        resolved.insert(
            name.to_string(),
            ResolvedAgent {
                name: name.to_string(),
                source: AgentSource::Implicit,
                definition_path: None,
                config: AgentFileConfig {
                    description: crate::agents::capability::specialist_description(name)
                        .map(str::to_string),
                    instruction: Some(format!(
                        "Act as the built-in {name} specialist. Use only capabilities relevant to this role and verify the result."
                    )),
                    provider: None,
                    model: None,
                    tool_confirmation_mode: None,
                    resource_paths: Vec::new(),
                    allow_tools: Vec::new(),
                    deny_tools: Vec::new(),
                    skills: crate::agents::capability::specialist_skill_patterns(name)
                        .unwrap_or_default()
                        .iter()
                        .map(|pattern| pattern.to_string())
                        .collect(),
                    deny_skills: Vec::new(),
                    agents: Vec::new(),
                    deny_agents: Vec::new(),
                    max_turns: Some(12),
                    timeout_secs: None,
                    hooks: HashMap::new(),
                },
            },
        );
    }
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

    let provider = cli
        .worker_provider
        .or((cli.provider != Provider::Auto).then_some(cli.provider))
        .or(active_agent.config.provider)
        .or(profile.worker_provider)
        .or(profile.provider)
        .unwrap_or(Provider::Openai);
    let worker_model = cli
        .worker_model
        .clone()
        .or(cli.model.clone())
        .or(active_agent.config.model.clone())
        .or(profile.worker_model.clone())
        .or(profile.model.clone())
        .unwrap_or_else(|| {
            crate::model_catalog::default_model(provider, crate::model_catalog::ModelRole::Worker)
                .to_string()
        });
    let planner_provider = cli
        .planner_provider
        .or(profile.planner_provider)
        .unwrap_or(Provider::Openai);
    let planner_model = cli
        .planner_model
        .clone()
        .or(profile.planner_model.clone())
        .unwrap_or_else(|| {
            crate::model_catalog::default_model(
                planner_provider,
                crate::model_catalog::ModelRole::Planner,
            )
            .to_string()
        });

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
    let mut available_agents = resolved_agents.values().cloned().collect::<Vec<_>>();
    available_agents.sort_by(|left, right| left.name.cmp(&right.name));

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
        agent_allow_skills: active_agent.config.skills.clone(),
        agent_deny_skills: active_agent.config.deny_skills.clone(),
        agent_allow_agents: active_agent.config.agents.clone(),
        agent_deny_agents: active_agent.config.deny_agents.clone(),
        agent_max_turns: active_agent.config.max_turns,
        agent_timeout_secs: active_agent.config.timeout_secs,
        available_agents,
        hooks: resolve_agent_hooks(&active_agent.config.hooks),
        provider,
        model: Some(worker_model.clone()),
        worker_provider: provider,
        worker_model,
        planner_provider,
        planner_model,
        planner_call_budget: cli
            .planner_call_budget
            .or(profile.planner_call_budget)
            .unwrap_or(4)
            .max(1),
        api_key: {
            // Requirement 3.5: keep working, but say that the value belongs in
            // the OS credential vault. Setup has written to the vault since v2;
            // a plaintext key here is a leftover that will be committed by
            // accident sooner or later.
            if profile.api_key.is_some() {
                tracing::warn!(
                    "a plaintext api_key is set in profile configuration; \
                     run `zavora-cli setup` to move it into the OS credential vault \
                     and remove it from the file"
                );
            }
            profile.api_key
        },
        ollama_host: profile.ollama_host,
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
            .or(active_agent.config.timeout_secs)
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
        permission_rules: profile.permission_rules.clone(),
        max_prompt_chars: 32_000,
        server_runner_cache_max: 64,
        auto_compact_enabled: true,
        compact_interval: 10,
        compact_overlap: 2,
        compaction_threshold: profile.compaction_threshold.unwrap_or(0.75),
        compaction_target: profile.compaction_target.unwrap_or(0.10),
    })
}

/// Apply a resolved agent to an already-resolved profile configuration.
///
/// This is used by direct runs, delegated sessions, and coordinator-created
/// agent tools so every execution surface receives identical model, prompt,
/// tool, skill, hook, and timeout policy.
pub fn apply_agent_overrides(cfg: &mut RuntimeConfig, agent: &ResolvedAgent) {
    cfg.agent_name = agent.name.clone();
    cfg.agent_source = agent.source;
    cfg.agent_description = agent.config.description.clone();
    cfg.agent_instruction = agent.config.instruction.clone();
    cfg.agent_resource_paths = agent.config.resource_paths.clone();
    cfg.agent_allow_tools = agent.config.allow_tools.clone();
    cfg.agent_deny_tools = agent.config.deny_tools.clone();
    cfg.agent_allow_skills = agent.config.skills.clone();
    cfg.agent_deny_skills = agent.config.deny_skills.clone();
    cfg.agent_allow_agents = agent.config.agents.clone();
    cfg.agent_deny_agents = agent.config.deny_agents.clone();
    cfg.agent_max_turns = agent.config.max_turns;
    cfg.agent_timeout_secs = agent.config.timeout_secs;
    cfg.hooks = resolve_agent_hooks(&agent.config.hooks);
    if let Some(provider) = agent.config.provider {
        cfg.provider = provider;
        cfg.worker_provider = provider;
    }
    if let Some(model) = agent.config.model.clone() {
        cfg.model = Some(model.clone());
        cfg.worker_model = model;
    }
    if let Some(mode) = agent.config.tool_confirmation_mode {
        cfg.tool_confirmation_mode = mode;
    }
    if let Some(timeout_secs) = agent.config.timeout_secs {
        cfg.tool_timeout_secs = timeout_secs.max(1);
    }
}

pub fn child_agent_allowed(cfg: &RuntimeConfig, name: &str) -> bool {
    let allowed = cfg.agent_allow_agents.is_empty()
        || cfg
            .agent_allow_agents
            .iter()
            .any(|pattern| crate::tool_policy::matches_wildcard(pattern, name));
    let denied = cfg
        .agent_deny_agents
        .iter()
        .any(|pattern| crate::tool_policy::matches_wildcard(pattern, name));
    allowed && !denied
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

/// Map the string-keyed `hooks` table from agent configuration onto typed hook
/// points, discarding unknown keys with a warning rather than failing the whole
/// configuration load.
fn resolve_agent_hooks(
    raw: &HashMap<String, Vec<HookConfig>>,
) -> HashMap<crate::hooks::HookPoint, Vec<HookConfig>> {
    use crate::hooks::HookPoint;

    let mut resolved: HashMap<HookPoint, Vec<HookConfig>> = HashMap::new();
    for (key, configs) in raw {
        let point = match key.as_str() {
            "agent_spawn" => HookPoint::AgentSpawn,
            "prompt_submit" => HookPoint::PromptSubmit,
            "pre_tool" => HookPoint::PreTool,
            "post_tool" => HookPoint::PostTool,
            "stop" => HookPoint::Stop,
            other => {
                tracing::warn!(
                    hook_point = other,
                    "unknown hook point in agent configuration; ignoring"
                );
                continue;
            }
        };
        resolved.entry(point).or_default().extend(configs.clone());
    }
    resolved
}
