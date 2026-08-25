//! Portable, governed multi-agent teams backed by ADK-Rust 2.1.
//!
//! Zavora owns discovery and product UX. ADK owns validation, deterministic
//! registry resolution, relationship governance, execution, and receipts.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use adk_rust::agent::{
    BlackboardHistoryPolicy, BlackboardPolicy, BlackboardSchedule, BlackboardSpec, CompiledTeam,
    StaticTeamAgentRegistry, TeamAgentDescriptor, TeamAgentHealth, TeamArchitectureTemplate,
    TeamBudget, TeamExecutionSnapshot, TeamPolicy, TeamSpec, TeamTerminationPolicy,
    WorkflowArchitectureTemplate,
};
use adk_rust::prelude::*;
use anyhow::{Context, Result, bail};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::config::{AgentSource, ResolvedAgent, RuntimeConfig};
use crate::runner::{ResolvedRuntimeTools, build_single_agent_with_tools};

pub const TEAM_API_VERSION: &str = "zavora.ai/v1alpha1";

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum TeamDefinition {
    /// Exact ADK team relationships with per-edge policy and aggregate budgets.
    Team {
        #[serde(rename = "apiVersion", default = "default_api_version")]
        api_version: String,
        spec: TeamSpec,
    },
    /// A bounded shared-transcript council using round-robin or selector scheduling.
    Blackboard {
        #[serde(rename = "apiVersion", default = "default_api_version")]
        api_version: String,
        spec: BlackboardSpec,
    },
    /// Supervisor, router, or hierarchical architecture lowered to an exact TeamSpec.
    Architecture {
        #[serde(rename = "apiVersion", default = "default_api_version")]
        api_version: String,
        spec: TeamArchitectureTemplate,
    },
    /// Sequential, parallel, fan-out/fan-in, or review-loop deterministic workflow.
    Workflow {
        #[serde(rename = "apiVersion", default = "default_api_version")]
        api_version: String,
        spec: WorkflowArchitectureTemplate,
    },
}

fn default_api_version() -> String {
    TEAM_API_VERSION.to_string()
}

impl TeamDefinition {
    pub fn name(&self) -> &str {
        match self {
            Self::Team { spec, .. } => &spec.name,
            Self::Blackboard { spec, .. } => &spec.name,
            Self::Architecture { spec, .. } => architecture_name(spec),
            Self::Workflow { spec, .. } => workflow_name(spec),
        }
    }

    pub fn api_version(&self) -> &str {
        match self {
            Self::Team { api_version, .. }
            | Self::Blackboard { api_version, .. }
            | Self::Architecture { api_version, .. }
            | Self::Workflow { api_version, .. } => api_version,
        }
    }

    pub fn architecture(&self) -> &'static str {
        match self {
            Self::Team { .. } => "governed-team",
            Self::Blackboard { .. } => "blackboard",
            Self::Architecture { spec, .. } => match spec {
                TeamArchitectureTemplate::Supervisor { .. } => "supervisor",
                TeamArchitectureTemplate::Router { .. } => "router",
                TeamArchitectureTemplate::Hierarchical { .. } => "hierarchical",
            },
            Self::Workflow { spec, .. } => match spec {
                WorkflowArchitectureTemplate::Sequential { .. } => "sequential",
                WorkflowArchitectureTemplate::Parallel { .. } => "parallel",
                WorkflowArchitectureTemplate::FanOutFanIn { .. } => "fan-out-fan-in",
                WorkflowArchitectureTemplate::ReviewLoop { .. } => "review-loop",
            },
        }
    }

    pub fn member_names(&self) -> Vec<String> {
        match self {
            Self::Team { spec, .. } => spec
                .members
                .iter()
                .map(|member| member.name.clone())
                .collect(),
            Self::Blackboard { spec, .. } => spec.members.clone(),
            Self::Architecture { spec, .. } => spec
                .lower()
                .map(|spec| spec.members.into_iter().map(|member| member.name).collect())
                .unwrap_or_default(),
            Self::Workflow { spec, .. } => workflow_members(spec),
        }
    }

    pub fn validate(&self) -> Result<()> {
        if self.api_version() != TEAM_API_VERSION {
            bail!(
                "team '{}' uses unsupported apiVersion '{}' (expected '{}')",
                self.name(),
                self.api_version(),
                TEAM_API_VERSION
            );
        }
        match self {
            Self::Team { spec, .. } => spec.validate().map_err(Into::into),
            Self::Blackboard { spec, .. } => spec.validate().map_err(Into::into),
            Self::Architecture { spec, .. } => spec.lower().map(|_| ()).map_err(Into::into),
            Self::Workflow { spec, .. } => validate_workflow(spec),
        }
    }

    fn governed_spec(&self) -> Result<Option<TeamSpec>> {
        match self {
            Self::Team { spec, .. } => Ok(Some(spec.clone())),
            Self::Architecture { spec, .. } => Ok(Some(spec.lower()?)),
            _ => Ok(None),
        }
    }
}

fn architecture_name(spec: &TeamArchitectureTemplate) -> &str {
    match spec {
        TeamArchitectureTemplate::Supervisor { name, .. }
        | TeamArchitectureTemplate::Router { name, .. }
        | TeamArchitectureTemplate::Hierarchical { name, .. } => name,
    }
}

fn workflow_name(spec: &WorkflowArchitectureTemplate) -> &str {
    match spec {
        WorkflowArchitectureTemplate::Sequential { name, .. }
        | WorkflowArchitectureTemplate::Parallel { name, .. }
        | WorkflowArchitectureTemplate::FanOutFanIn { name, .. }
        | WorkflowArchitectureTemplate::ReviewLoop { name, .. } => name,
    }
}

fn workflow_members(spec: &WorkflowArchitectureTemplate) -> Vec<String> {
    match spec {
        WorkflowArchitectureTemplate::Sequential { steps, .. } => steps.clone(),
        WorkflowArchitectureTemplate::Parallel { members, .. } => members.clone(),
        WorkflowArchitectureTemplate::FanOutFanIn {
            workers,
            aggregator,
            ..
        } => workers
            .iter()
            .cloned()
            .chain(std::iter::once(aggregator.clone()))
            .collect(),
        WorkflowArchitectureTemplate::ReviewLoop {
            producer, reviewer, ..
        } => {
            vec![producer.clone(), reviewer.clone()]
        }
    }
}

fn validate_workflow(spec: &WorkflowArchitectureTemplate) -> Result<()> {
    let name = workflow_name(spec);
    if name.trim().is_empty() {
        bail!("workflow name must not be empty");
    }
    let members = workflow_members(spec);
    if members.is_empty() {
        bail!("workflow '{name}' must declare at least one member");
    }
    let mut unique = BTreeSet::new();
    for member in &members {
        if member.trim().is_empty() {
            bail!("workflow '{name}' contains an empty member name");
        }
        if !unique.insert(member) {
            bail!("workflow '{name}' contains duplicate member '{member}'");
        }
    }
    if let WorkflowArchitectureTemplate::ReviewLoop {
        max_iterations: 0, ..
    } = spec
    {
        bail!("workflow '{name}' maxIterations must be greater than zero");
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum TeamSource {
    Builtin,
    Global,
    Local,
}

impl TeamSource {
    pub fn label(self) -> &'static str {
        match self {
            Self::Builtin => "builtin",
            Self::Global => "global",
            Self::Local => "local",
        }
    }
}

#[derive(Debug, Clone)]
pub struct ResolvedTeam {
    pub definition: TeamDefinition,
    pub source: TeamSource,
    pub path: Option<PathBuf>,
}

impl ResolvedTeam {
    pub fn name(&self) -> &str {
        self.definition.name()
    }
}

fn builtin_teams() -> Vec<ResolvedTeam> {
    let policy = TeamPolicy {
        max_delegation_depth: 3,
        max_concurrent_delegations: 3,
        budget: TeamBudget {
            max_model_requests: Some(32),
            max_tool_calls: Some(96),
            max_delegations: Some(12),
            max_wall_time_ms: Some(15 * 60 * 1_000),
            ..TeamBudget::default()
        },
        ..TeamPolicy::default()
    };
    vec![
        ResolvedTeam {
            definition: TeamDefinition::Architecture {
                api_version: default_api_version(),
                spec: TeamArchitectureTemplate::Supervisor {
                    name: "frontier-delivery".to_string(),
                    coordinator: "developer_agent".to_string(),
                    specialists: vec![
                        "research_agent".to_string(),
                        "artifact_agent".to_string(),
                        "operations_agent".to_string(),
                        "reviewer_agent".to_string(),
                    ],
                    relationship: adk_rust::agent::RelationshipKind::Delegate,
                    policy,
                },
            },
            source: TeamSource::Builtin,
            path: None,
        },
        ResolvedTeam {
            definition: TeamDefinition::Workflow {
                api_version: default_api_version(),
                spec: WorkflowArchitectureTemplate::FanOutFanIn {
                    name: "evidence-review".to_string(),
                    workers: vec!["research_agent".to_string(), "developer_agent".to_string()],
                    aggregator: "reviewer_agent".to_string(),
                    shared_state: true,
                },
            },
            source: TeamSource::Builtin,
            path: None,
        },
        ResolvedTeam {
            definition: TeamDefinition::Blackboard {
                api_version: default_api_version(),
                spec: BlackboardSpec {
                    name: "work-council".to_string(),
                    description:
                        "Bounded productivity and operations council with independent review"
                            .to_string(),
                    members: vec![
                        "artifact_agent".to_string(),
                        "operations_agent".to_string(),
                        "reviewer_agent".to_string(),
                    ],
                    schedule: BlackboardSchedule::RoundRobin,
                    transitions: Vec::new(),
                    policy: BlackboardPolicy {
                        max_rounds: 2,
                        history: BlackboardHistoryPolicy::Last { max_messages: 24 },
                        budget: TeamBudget {
                            max_events: Some(96),
                            max_model_requests: Some(12),
                            max_tool_calls: Some(32),
                            max_wall_time_ms: Some(10 * 60 * 1_000),
                            ..TeamBudget::default()
                        },
                        termination: TeamTerminationPolicy::default(),
                    },
                },
            },
            source: TeamSource::Builtin,
            path: None,
        },
    ]
}

pub fn team_roots(workspace: &Path) -> Vec<(PathBuf, TeamSource)> {
    let mut roots = Vec::new();
    if let Some(config_home) = std::env::var_os("XDG_CONFIG_HOME") {
        roots.push((
            PathBuf::from(config_home).join("zavora/teams"),
            TeamSource::Global,
        ));
    } else if let Some(home) = std::env::var_os("HOME") {
        roots.push((
            PathBuf::from(home).join(".config/zavora/teams"),
            TeamSource::Global,
        ));
    }
    roots.push((workspace.join(".zavora/teams"), TeamSource::Local));
    roots.push((workspace.join(".agents/teams"), TeamSource::Local));
    roots
}

pub fn discover_teams(workspace: &Path) -> Result<BTreeMap<String, ResolvedTeam>> {
    let mut teams = builtin_teams()
        .into_iter()
        .map(|team| (team.name().to_string(), team))
        .collect::<BTreeMap<_, _>>();
    for (root, source) in team_roots(workspace) {
        if !root.is_dir() {
            continue;
        }
        let mut files = std::fs::read_dir(&root)
            .with_context(|| format!("failed to read team directory '{}'", root.display()))?
            .filter_map(std::result::Result::ok)
            .map(|entry| entry.path())
            .filter(|path| is_team_file(path))
            .collect::<Vec<_>>();
        files.sort();
        for path in files {
            let definition = load_team_file(&path)?;
            definition
                .validate()
                .with_context(|| format!("invalid team definition in '{}'", path.display()))?;
            teams.insert(
                definition.name().to_string(),
                ResolvedTeam {
                    definition,
                    source,
                    path: Some(path),
                },
            );
        }
    }
    Ok(teams)
}

fn is_team_file(path: &Path) -> bool {
    path.is_file()
        && path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| {
                matches!(
                    extension.to_ascii_lowercase().as_str(),
                    "yaml" | "yml" | "json" | "toml"
                )
            })
}

pub fn load_team_file(path: &Path) -> Result<TeamDefinition> {
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read team definition '{}'", path.display()))?;
    match path
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or_default()
    {
        extension if extension.eq_ignore_ascii_case("json") => serde_json::from_str(&content)
            .with_context(|| format!("invalid JSON team definition '{}'", path.display())),
        extension if extension.eq_ignore_ascii_case("toml") => toml::from_str(&content)
            .with_context(|| format!("invalid TOML team definition '{}'", path.display())),
        _ => serde_yaml::from_str(&content)
            .with_context(|| format!("invalid YAML team definition '{}'", path.display())),
    }
}

pub fn validate_bindings(team: &ResolvedTeam, agents: &[ResolvedAgent]) -> Result<()> {
    team.definition.validate()?;
    let names = agents
        .iter()
        .map(|agent| agent.name.as_str())
        .collect::<BTreeSet<_>>();
    let governed = team.definition.governed_spec()?;
    let missing = team
        .definition
        .member_names()
        .into_iter()
        .filter(|member| !names.contains(member.as_str()))
        .filter(|member| {
            governed.as_ref().is_none_or(|spec| {
                spec.members
                    .iter()
                    .find(|candidate| candidate.name == *member)
                    .is_none_or(|candidate| candidate.required_capabilities.is_empty())
            })
        })
        .collect::<Vec<_>>();
    if !missing.is_empty() {
        bail!(
            "team '{}' has unresolved local members: {}",
            team.name(),
            missing.join(", ")
        );
    }
    Ok(())
}

#[derive(Debug, Clone, Serialize)]
pub struct TeamTopology {
    pub name: String,
    pub architecture: String,
    pub nodes: Vec<String>,
    pub edges: Vec<TopologyEdge>,
}

#[derive(Debug, Clone, Serialize)]
pub struct TopologyEdge {
    pub from: String,
    pub to: String,
    pub relationship: String,
}

pub fn topology(definition: &TeamDefinition) -> Result<TeamTopology> {
    let mut edges = Vec::new();
    match definition {
        TeamDefinition::Team { spec, .. } => add_team_edges(spec, &mut edges),
        TeamDefinition::Architecture { spec, .. } => add_team_edges(&spec.lower()?, &mut edges),
        TeamDefinition::Blackboard { spec, .. } => {
            if spec.transitions.is_empty() {
                for pair in spec.members.windows(2) {
                    edges.push(TopologyEdge {
                        from: pair[0].clone(),
                        to: pair[1].clone(),
                        relationship: "blackboard-turn".to_string(),
                    });
                }
            } else {
                edges.extend(spec.transitions.iter().map(|edge| TopologyEdge {
                    from: edge.from.clone(),
                    to: edge.to.clone(),
                    relationship: "blackboard-transition".to_string(),
                }));
            }
        }
        TeamDefinition::Workflow { spec, .. } => match spec {
            WorkflowArchitectureTemplate::Sequential { steps, .. } => {
                for pair in steps.windows(2) {
                    edges.push(TopologyEdge {
                        from: pair[0].clone(),
                        to: pair[1].clone(),
                        relationship: "then".to_string(),
                    });
                }
            }
            WorkflowArchitectureTemplate::Parallel { .. } => {}
            WorkflowArchitectureTemplate::FanOutFanIn {
                workers,
                aggregator,
                ..
            } => {
                edges.extend(workers.iter().map(|worker| TopologyEdge {
                    from: worker.clone(),
                    to: aggregator.clone(),
                    relationship: "fan-in".to_string(),
                }));
            }
            WorkflowArchitectureTemplate::ReviewLoop {
                producer, reviewer, ..
            } => {
                edges.push(TopologyEdge {
                    from: producer.clone(),
                    to: reviewer.clone(),
                    relationship: "review".to_string(),
                });
                edges.push(TopologyEdge {
                    from: reviewer.clone(),
                    to: producer.clone(),
                    relationship: "revise".to_string(),
                });
            }
        },
    }
    Ok(TeamTopology {
        name: definition.name().to_string(),
        architecture: definition.architecture().to_string(),
        nodes: definition.member_names(),
        edges,
    })
}

fn add_team_edges(spec: &TeamSpec, edges: &mut Vec<TopologyEdge>) {
    edges.extend(spec.relationships.iter().map(|edge| TopologyEdge {
        from: edge.from.clone(),
        to: edge.to.clone(),
        relationship: format!("{:?}", edge.kind).to_ascii_lowercase(),
    }));
}

pub struct BuiltTeam {
    pub root: Arc<dyn Agent>,
    pub name: String,
    pub architecture: String,
    pub roster: Vec<String>,
    pub routes: Vec<String>,
    governed: Option<Arc<CompiledTeam>>,
}

impl BuiltTeam {
    /// Completed and in-flight semantic receipts retained by an ADK governed team.
    pub fn execution_receipts(&self) -> Vec<TeamExecutionSnapshot> {
        self.governed
            .as_ref()
            .map(|team| team.execution_snapshots())
            .unwrap_or_default()
    }
}

#[derive(Debug)]
struct TeamConfirmationHandler;

#[async_trait::async_trait]
impl adk_rust::ToolConfirmationHandler for TeamConfirmationHandler {
    async fn decide(
        &self,
        request: &adk_rust::ToolConfirmationRequest,
    ) -> adk_rust::Result<adk_rust::ToolConfirmationDecision> {
        let detail = format!(
            "Governed team relationship `{}` with arguments {}",
            request.tool_name, request.args
        );
        let decision =
            crate::tools::confirming::confirm_runtime_action("team", &request.tool_name, &detail)
                .await;
        Ok(match decision {
            crate::tools::confirming::ApprovalDecision::AllowOnce
            | crate::tools::confirming::ApprovalDecision::TrustSession => {
                adk_rust::ToolConfirmationDecision::Approve
            }
            crate::tools::confirming::ApprovalDecision::Deny => {
                adk_rust::ToolConfirmationDecision::Deny
            }
        })
    }
}

pub fn with_team_confirmation_handler(mut run_config: RunConfig) -> RunConfig {
    run_config.tool_confirmation_handler = Some(Arc::new(TeamConfirmationHandler));
    run_config
}

pub fn trust_team_relationships(team: &BuiltTeam) {
    for member in &team.roster {
        crate::tools::confirming::trust_tool(member);
    }
}

pub async fn build_team(
    team: &ResolvedTeam,
    cfg: &RuntimeConfig,
    runtime_tools: &ResolvedRuntimeTools,
) -> Result<BuiltTeam> {
    validate_bindings(team, &cfg.available_agents)?;
    let requested = team
        .definition
        .member_names()
        .into_iter()
        .collect::<BTreeSet<_>>();
    let use_discovery = team.definition.governed_spec()?.is_some_and(|spec| {
        spec.members
            .iter()
            .any(|member| !member.required_capabilities.is_empty())
    });
    let candidates = cfg
        .available_agents
        .iter()
        .filter(|agent| agent.name != "ralph")
        .filter(|agent| use_discovery || requested.contains(&agent.name))
        .collect::<Vec<_>>();

    let mut bindings = Vec::new();
    let mut routes = Vec::new();
    for resolved in candidates {
        let mut member_cfg = cfg.clone();
        crate::config::apply_agent_overrides(&mut member_cfg, resolved);
        let (model, provider, model_name) = crate::provider::resolve_model(&member_cfg)
            .with_context(|| {
                format!("failed to resolve model for team agent '{}'", resolved.name)
            })?;
        let scoped_tools = crate::tool_policy::filter_tools_by_policy(
            runtime_tools.tools().to_vec(),
            &resolved.config.allow_tools,
            &resolved.config.deny_tools,
        );
        let confirmation =
            crate::runner::resolve_tool_confirmation_settings(&member_cfg, runtime_tools);
        let agent = build_single_agent_with_tools(
            model,
            &scoped_tools,
            confirmation.policy,
            Duration::from_secs(member_cfg.tool_timeout_secs),
            Some(&member_cfg),
        )?;
        routes.push(format!(
            "{}={}/{}",
            resolved.name,
            format!("{provider:?}").to_ascii_lowercase(),
            model_name
        ));
        bindings.push((resolved, agent, scoped_tools));
    }

    let mut governed = None;
    let root: Arc<dyn Agent> = match &team.definition {
        TeamDefinition::Team { spec, .. } => {
            let mut registry = StaticTeamAgentRegistry::new();
            for (resolved, agent, tools) in &bindings {
                registry = registry.register(agent_descriptor(resolved, tools), agent.clone())?;
            }
            let compiled = Arc::new(spec.compile_with_registry(Arc::new(registry)).await?);
            governed = Some(compiled.clone());
            compiled
        }
        TeamDefinition::Architecture { spec, .. } => {
            let lowered = spec.lower()?;
            let mut registry = StaticTeamAgentRegistry::new();
            for (resolved, agent, tools) in &bindings {
                registry = registry.register(agent_descriptor(resolved, tools), agent.clone())?;
            }
            let compiled = Arc::new(lowered.compile_with_registry(Arc::new(registry)).await?);
            governed = Some(compiled.clone());
            compiled
        }
        TeamDefinition::Blackboard { spec, .. } => {
            Arc::new(spec.compile(bindings.iter().map(|(_, agent, _)| agent.clone()))?)
        }
        TeamDefinition::Workflow { spec, .. } => {
            spec.compile(bindings.iter().map(|(_, agent, _)| agent.clone()))?
        }
    };

    Ok(BuiltTeam {
        root,
        name: team.name().to_string(),
        architecture: team.definition.architecture().to_string(),
        roster: team.definition.member_names(),
        routes,
        governed,
    })
}

fn agent_descriptor(agent: &ResolvedAgent, tools: &[Arc<dyn Tool>]) -> TeamAgentDescriptor {
    let mut capabilities = tools
        .iter()
        .map(|tool| tool.name().to_string())
        .collect::<Vec<_>>();
    capabilities.extend(agent.config.skills.iter().cloned());
    capabilities.push(format!("agent:{}", agent.name));
    if let Some(category) = crate::agents::capability::specialist_category(&agent.name) {
        capabilities.push(category.label().to_ascii_lowercase());
    }
    capabilities.sort();
    capabilities.dedup();
    TeamAgentDescriptor {
        binding: format!("zavora-local:{}", agent.name),
        name: agent.name.clone(),
        description: agent.config.description.clone().unwrap_or_default(),
        capabilities,
        priority: match agent.source {
            AgentSource::Local => 30,
            AgentSource::Plugin => 25,
            AgentSource::Global => 20,
            AgentSource::Implicit => 10,
        },
        version: Some(env!("CARGO_PKG_VERSION").to_string()),
        digest: None,
        trust_labels: vec![
            "local-runtime".to_string(),
            agent.source.label().to_string(),
        ],
        health: TeamAgentHealth::Healthy,
        expires_at_ms: None,
    }
}

pub fn render_catalog(teams: &BTreeMap<String, ResolvedTeam>, json: bool) -> Result<String> {
    if json {
        let values = teams
            .values()
            .map(|team| {
                serde_json::json!({
                    "name": team.name(),
                    "architecture": team.definition.architecture(),
                    "api_version": team.definition.api_version(),
                    "members": team.definition.member_names(),
                    "source": team.source,
                    "path": team.path,
                })
            })
            .collect::<Vec<_>>();
        return Ok(serde_json::to_string_pretty(&values)?);
    }
    let mut output = String::from("## Agent teams\n\n");
    for team in teams.values() {
        output.push_str(&format!(
            "- **{}** — {} · {} members · {}\n",
            team.name(),
            team.definition.architecture(),
            team.definition.member_names().len(),
            team.source.label()
        ));
    }
    output.push_str("\nDefinitions: `.agents/teams/*.{yaml,yml,json,toml}` (workspace) or `~/.config/zavora/teams/` (user).\n");
    Ok(output)
}

pub fn render_team(team: &ResolvedTeam, json: bool) -> Result<String> {
    if json {
        return Ok(serde_json::to_string_pretty(&team.definition)?);
    }
    let topology = topology(&team.definition)?;
    let mut output = format!(
        "## {}\n\n- Architecture: `{}`\n- API: `{}`\n- Source: `{}`\n- Members: {}\n",
        team.name(),
        topology.architecture,
        team.definition.api_version(),
        team.source.label(),
        topology.nodes.join(", ")
    );
    if let Some(path) = &team.path {
        output.push_str(&format!("- Definition: `{}`\n", path.display()));
    }
    if !topology.edges.is_empty() {
        output.push_str("\n### Topology\n\n");
        for edge in topology.edges {
            output.push_str(&format!(
                "- `{}` → `{}` ({})\n",
                edge.from, edge.to, edge.relationship
            ));
        }
    }
    Ok(output)
}

pub fn schema_json() -> Result<String> {
    Ok(serde_json::to_string_pretty(&schemars::schema_for!(
        TeamDefinition
    ))?)
}

pub enum InteractiveTeamAction {
    Display(String),
    Run { name: String, task: String },
}

pub fn resolve_interactive_command(
    input: &str,
    cfg: &RuntimeConfig,
) -> Result<InteractiveTeamAction> {
    let workspace = std::env::current_dir()?;
    let teams = discover_teams(&workspace)?;
    let mut parts = input.split_whitespace();
    let command = parts.next().unwrap_or("list");
    match command {
        "list" => Ok(InteractiveTeamAction::Display(render_catalog(
            &teams, false,
        )?)),
        "show" => {
            let name = parts.next().context("usage: /teams show NAME")?;
            let team = required_team(&teams, name)?;
            Ok(InteractiveTeamAction::Display(render_team(team, false)?))
        }
        "validate" => {
            let selected = if let Some(name) = parts.next() {
                vec![required_team(&teams, name)?]
            } else {
                teams.values().collect::<Vec<_>>()
            };
            let mut output = String::from("## Team validation\n\n");
            for team in selected {
                match validate_bindings(team, &cfg.available_agents) {
                    Ok(()) => output.push_str(&format!("- ✓ `{}`\n", team.name())),
                    Err(error) => output.push_str(&format!("- ✗ `{}` — {}\n", team.name(), error)),
                }
            }
            Ok(InteractiveTeamAction::Display(output))
        }
        "topology" => {
            let name = parts.next().context("usage: /teams topology NAME")?;
            let team = required_team(&teams, name)?;
            let graph = topology(&team.definition)?;
            let mut output = format!("## {} topology\n\n", graph.name);
            for edge in graph.edges {
                output.push_str(&format!(
                    "- `{}` → `{}` ({})\n",
                    edge.from, edge.to, edge.relationship
                ));
            }
            if output.lines().count() <= 2 {
                output.push_str(&format!("Members: {}\n", graph.nodes.join(", ")));
            }
            Ok(InteractiveTeamAction::Display(output))
        }
        "schema" => Ok(InteractiveTeamAction::Display(format!(
            "```json\n{}\n```",
            schema_json()?
        ))),
        "run" => {
            let name = parts.next().context("usage: /teams run NAME TASK")?;
            required_team(&teams, name)?;
            let task = parts.collect::<Vec<_>>().join(" ");
            if task.is_empty() {
                bail!("usage: /teams run NAME TASK");
            }
            Ok(InteractiveTeamAction::Run {
                name: name.to_string(),
                task,
            })
        }
        name if !name.is_empty() && parts.clone().next().is_some() => {
            required_team(&teams, name)?;
            Ok(InteractiveTeamAction::Run {
                name: name.to_string(),
                task: parts.collect::<Vec<_>>().join(" "),
            })
        }
        name => {
            let team = required_team(&teams, name)?;
            Ok(InteractiveTeamAction::Display(render_team(team, false)?))
        }
    }
}

fn required_team<'a>(
    teams: &'a BTreeMap<String, ResolvedTeam>,
    name: &str,
) -> Result<&'a ResolvedTeam> {
    teams.get(name).ok_or_else(|| {
        anyhow::anyhow!("team '{name}' not found; use `/teams list` to inspect available teams")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    struct StubAgent(String);

    #[async_trait::async_trait]
    impl Agent for StubAgent {
        fn name(&self) -> &str {
            &self.0
        }

        fn description(&self) -> &str {
            "portable team compilation fixture"
        }

        fn sub_agents(&self) -> &[Arc<dyn Agent>] {
            &[]
        }

        fn capabilities(&self) -> adk_rust::AgentCapabilities {
            adk_rust::AgentCapabilities {
                runtime_tools: true,
                handoff: true,
                relationship_confirmation: true,
                shared_state: true,
                invocation_metadata: true,
                ..Default::default()
            }
        }

        async fn run(&self, _ctx: Arc<dyn InvocationContext>) -> adk_rust::Result<EventStream> {
            unreachable!("compilation tests never execute a member")
        }
    }

    fn stub_agents(names: Vec<String>) -> Vec<Arc<dyn Agent>> {
        names
            .into_iter()
            .map(|name| Arc::new(StubAgent(name)) as Arc<dyn Agent>)
            .collect()
    }

    #[test]
    fn builtins_are_valid_and_have_unique_names() {
        let teams = builtin_teams();
        let names = teams
            .iter()
            .map(|team| team.name())
            .collect::<BTreeSet<_>>();
        assert_eq!(names.len(), teams.len());
        for team in teams {
            team.definition.validate().unwrap();
        }
    }

    #[test]
    fn every_builtin_compiles_through_the_adk_21_runtime_type() {
        for team in builtin_teams() {
            let agents = stub_agents(team.definition.member_names());
            let root: Arc<dyn Agent> = match &team.definition {
                TeamDefinition::Team { spec, .. } => Arc::new(spec.compile(agents).unwrap()),
                TeamDefinition::Architecture { spec, .. } => {
                    Arc::new(spec.lower().unwrap().compile(agents).unwrap())
                }
                TeamDefinition::Blackboard { spec, .. } => Arc::new(spec.compile(agents).unwrap()),
                TeamDefinition::Workflow { spec, .. } => spec.compile(agents).unwrap(),
            };
            assert_eq!(root.name(), team.name());
        }
    }

    #[test]
    fn yaml_uses_portable_adk_team_spec_directly() {
        let definition: TeamDefinition = serde_yaml::from_str(
            r#"
apiVersion: zavora.ai/v1alpha1
kind: team
spec:
  name: delivery
  coordinator: lead
  members:
    - name: lead
    - name: reviewer
  relationships:
    - from: lead
      to: reviewer
      kind: delegate
      policy:
        timeoutMs: 30000
  policy:
    maxTransferDepth: 8
    maxDelegationDepth: 2
    maxConcurrentDelegations: 1
    context: shared
    failure: propagate
    budget:
      maxDelegations: 4
"#,
        )
        .unwrap();
        definition.validate().unwrap();
        let graph = topology(&definition).unwrap();
        assert_eq!(graph.edges[0].relationship, "delegate");
    }

    #[test]
    fn local_definition_overrides_builtin() {
        let workspace = tempfile::tempdir().unwrap();
        let root = workspace.path().join(".agents/teams");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(
            root.join("override.yaml"),
            r#"
apiVersion: zavora.ai/v1alpha1
kind: workflow
spec:
  architecture: sequential
  name: evidence-review
  steps: [developer_agent, reviewer_agent]
"#,
        )
        .unwrap();
        let teams = discover_teams(workspace.path()).unwrap();
        assert_eq!(teams["evidence-review"].source, TeamSource::Local);
        assert_eq!(teams["evidence-review"].definition.member_names().len(), 2);
    }
}
