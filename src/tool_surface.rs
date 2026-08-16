//! The tool surface and its single enforcement point.
//!
//! Tools used to be assembled, filtered, wrapped — and then appended to again.
//! Two additions landed after enforcement: `tool_search` and, more seriously,
//! the browser toolset, which escaped both confirmation and allow/deny policy
//! entirely. Patching those two call sites would have left the next one free to
//! reintroduce the bug.
//!
//! So the surface is a builder that [`ToolSurface::seal`] consumes. `seal`
//! returns a [`ResolvedRuntimeTools`] whose tool vector is private with no
//! mutating accessor, which makes a post-enforcement addition a compile error
//! rather than a review question.
//!
//! Requirements 6.2, 6.3, 7.1, 7.2; Correctness Properties 2 and 5.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use adk_rust::prelude::*;

use crate::cli::ToolConfirmationMode;
use crate::config::RuntimeConfig;
use crate::tool_policy::{
    PermissionDecision, PermissionRules, ToolClass, ToolPattern, ToolProvenance, classify,
    filter_tools_by_policy,
};
use crate::tools::confirming::ConfirmingTool;

/// A tool plus the provenance the builder recorded for it.
struct Candidate {
    tool: Arc<dyn Tool>,
    provenance: ToolProvenance,
}

/// Mutable accumulator for the turn's tool surface.
///
/// Every `add_*` method takes and returns `self`, so the builder reads as a
/// pipeline and cannot be reused after sealing.
pub struct ToolSurface {
    candidates: Vec<Candidate>,
    connect_failures: Vec<crate::mcp::McpConnectFailure>,
}

impl ToolSurface {
    pub fn new() -> Self {
        Self {
            candidates: Vec::new(),
            connect_failures: Vec::new(),
        }
    }

    /// Record servers that were configured but could not be reached, so the
    /// active surface can say so instead of silently offering fewer tools.
    pub fn add_connect_failures(mut self, failures: Vec<crate::mcp::McpConnectFailure>) -> Self {
        self.connect_failures.extend(failures);
        self
    }

    fn push_all(mut self, tools: Vec<Arc<dyn Tool>>, provenance: ToolProvenance) -> Self {
        self.candidates
            .extend(tools.into_iter().map(|tool| Candidate { tool, provenance }));
        self
    }

    /// Tools compiled into the binary.
    pub fn add_builtins(self) -> Self {
        self.push_all(crate::tools::build_builtin_tools(), ToolProvenance::BuiltIn)
    }

    /// Tools behind optional features that drive a remote or out-of-process
    /// surface. Classified as egress because their blast radius is not knowable
    /// from here.
    pub async fn add_feature_gated(self) -> Self {
        #[allow(unused_mut)]
        let mut surface = self;

        #[cfg(feature = "browser")]
        {
            // Initialization is deliberate and eager: a lazily initialized tool
            // that is offered to a model before its backend exists answers
            // "not initialized" forever. Requirement 6.4.
            match crate::tools::browser::get_browser().await {
                Ok(session) => {
                    let browser_tools = crate::tools::browser::build_browser_tools(session);
                    tracing::info!(count = browser_tools.len(), "Browser tools loaded");
                    surface = surface.push_all(browser_tools, ToolProvenance::FeatureGated);
                }
                Err(error) => {
                    tracing::warn!(%error, "browser tools unavailable");
                }
            }
        }

        #[cfg(feature = "lsp")]
        {
            // The `lsp` tool is registered by `build_builtin_tools`, but its
            // manager is lazy. Without this the tool always answered
            // `not_initialized`. Requirement 6.4.
            if crate::tools::lsp::init_manager() {
                tracing::info!("LSP manager initialized");
            } else {
                tracing::debug!("no .zavora/lsp.json; LSP tool will report not_initialized");
            }
        }

        surface
    }

    /// Tools contributed by installed plugin packages.
    pub fn add_plugin_contributed(self, tools: Vec<Arc<dyn Tool>>) -> Self {
        self.push_all(tools, ToolProvenance::Plugin)
    }

    /// Tools discovered from MCP servers.
    pub fn add_mcp(self, tools: Vec<Arc<dyn Tool>>) -> Self {
        self.push_all(tools, ToolProvenance::Mcp)
    }

    /// Register the keyword discovery tool when the surface is large enough for
    /// a model to lose track of it.
    ///
    /// It searches the assembled set rather than a pre-enforcement snapshot,
    /// which is why it belongs here and not after `seal`.
    pub fn add_discovery_tool(mut self, threshold: usize) -> Self {
        if self.candidates.len() <= threshold {
            return self;
        }

        let searchable = self
            .candidates
            .iter()
            .map(|candidate| Arc::clone(&candidate.tool))
            .collect::<Vec<_>>();

        let search_tool = FunctionTool::new(
            "tool_search",
            "Search available tools by keyword. Use when you need a tool that isn't in your current set. \
             Args: query (required, space-separated keywords to match against tool names and descriptions). \
             Returns matching tool names, descriptions, and parameter schemas.",
            move |_ctx, args| {
                let tools_ref = searchable.clone();
                async move {
                    let query = args
                        .get("query")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or("");
                    Ok(crate::tools::tool_search::tool_search_response(
                        query, &tools_ref,
                    ))
                }
            },
        )
        .with_read_only(true)
        .with_concurrency_safe(true);

        self.candidates.push(Candidate {
            tool: Arc::new(search_tool),
            provenance: ToolProvenance::BuiltIn,
        });
        self
    }

    /// The single enforcement point: classify, filter, decide, wrap, freeze.
    ///
    /// Nothing may be added after this returns.
    pub fn seal(mut self, cfg: &RuntimeConfig) -> ResolvedRuntimeTools {
        // Install the hook stage for this surface. Doing it here rather than at
        // each call site means a tool cannot be wrapped without hooks applying
        // to it. Requirement 7.8.
        if cfg.hooks.is_empty() {
            crate::tools::confirming::clear_hook_executor();
        } else {
            let configured = cfg.hooks.values().map(Vec::len).sum::<usize>();
            tracing::info!(hooks = configured, "installing lifecycle hook executor");
            crate::tools::confirming::install_hook_executor(Arc::new(
                crate::hooks::HookExecutor::new(cfg.hooks.clone()),
            ));
        }

        let mut classes = BTreeMap::<String, ToolClass>::new();
        let mut mcp_names = BTreeSet::<String>::new();
        let mut built_in_count = 0usize;

        for candidate in &self.candidates {
            let name = candidate.tool.name().to_string();
            classes.insert(
                name.clone(),
                classify(&candidate.tool, candidate.provenance),
            );
            match candidate.provenance {
                ToolProvenance::Mcp => {
                    mcp_names.insert(name);
                }
                ToolProvenance::BuiltIn => built_in_count += 1,
                _ => {}
            }
        }

        let connect_failures = std::mem::take(&mut self.connect_failures);
        let assembled = self
            .candidates
            .into_iter()
            .map(|candidate| candidate.tool)
            .collect::<Vec<_>>();
        let assembled_count = assembled.len();

        // Deny rules drop tools outright.
        let permitted =
            filter_tools_by_policy(assembled, &cfg.agent_allow_tools, &cfg.agent_deny_tools);

        let rules = effective_permission_rules(cfg);

        let wrapped = permitted
            .into_iter()
            .map(|tool| {
                let name = tool.name().to_string();
                let class = classes
                    .get(&name)
                    .copied()
                    // A tool that appeared without being classified is a bug in
                    // the builder. Fail closed.
                    .unwrap_or(ToolClass::Mutating);
                wrap_for_policy(tool, class, &rules, cfg)
            })
            .collect::<Vec<_>>();

        tracing::info!(
            built_in_tools = built_in_count,
            mcp_tools = mcp_names.len(),
            assembled_tools = assembled_count,
            total_tools = wrapped.len(),
            agent_allow_tools = cfg.agent_allow_tools.len(),
            agent_deny_tools = cfg.agent_deny_tools.len(),
            "Sealed runtime toolset"
        );

        // Deny filtering may have removed names; keep the reported MCP set and
        // the class map aligned with what actually survived.
        let surviving = wrapped
            .iter()
            .map(|tool| tool.name().to_string())
            .collect::<BTreeSet<_>>();
        classes.retain(|name, _| surviving.contains(name));
        mcp_names.retain(|name| surviving.contains(name));

        ResolvedRuntimeTools {
            tools: wrapped,
            classes,
            mcp_tool_names: mcp_names,
            connect_failures,
        }
    }
}

impl Default for ToolSurface {
    fn default() -> Self {
        Self::new()
    }
}

/// Profile permission rules merged with the legacy per-tool flags.
fn effective_permission_rules(cfg: &RuntimeConfig) -> PermissionRules {
    let rules = &cfg.permission_rules;

    let mut always_allow: Vec<ToolPattern> = rules.always_allow.clone();
    let always_deny: Vec<ToolPattern> = rules.always_deny.clone();
    let mut always_ask: Vec<ToolPattern> = rules.always_ask.clone();

    // Legacy approve_tool → always_allow, require_confirm_tool → always_ask.
    for name in &cfg.approve_tool {
        let trimmed = name.trim();
        if !trimmed.is_empty() {
            always_allow.push(ToolPattern(trimmed.to_string()));
        }
    }
    for name in &cfg.require_confirm_tool {
        let trimmed = name.trim();
        if !trimmed.is_empty() {
            always_ask.push(ToolPattern(trimmed.to_string()));
        }
    }

    PermissionRules {
        always_allow,
        always_deny,
        always_ask,
    }
}

/// Decide how a single tool is wrapped.
///
/// `ToolClass::NetworkEgress` is never returned unwrapped, in any confirmation
/// mode. That is what closes the `web_fetch` gap without a name list, and it is
/// why an explicit `always_allow` rule still yields a display wrap rather than
/// a bare tool.
fn wrap_for_policy(
    tool: Arc<dyn Tool>,
    class: ToolClass,
    rules: &PermissionRules,
    cfg: &RuntimeConfig,
) -> Arc<dyn Tool> {
    let name = tool.name();

    match rules.evaluate(name, None) {
        PermissionDecision::Allow => {
            if class.is_auto_approvable() {
                ConfirmingTool::wrap_display_only(tool)
            } else {
                // The developer allowed it explicitly, so do not prompt — but
                // egress and mutation stay visible.
                ConfirmingTool::wrap_display_only(tool)
            }
        }
        // Name-level denials are already filtered out. A surviving deny rule
        // targets content patterns, so per-call denial happens at execute time.
        PermissionDecision::Deny => ConfirmingTool::wrap(tool),
        PermissionDecision::Ask => ConfirmingTool::wrap(tool),
        PermissionDecision::NoMatch => {
            if class.is_auto_approvable() {
                return ConfirmingTool::wrap_display_only(tool);
            }

            match cfg.tool_confirmation_mode {
                ToolConfirmationMode::Always => ConfirmingTool::wrap(tool),
                // Egress is confirmed in every mode; local mutation of the
                // developer's own workspace is what `Never` may skip.
                ToolConfirmationMode::McpOnly | ToolConfirmationMode::Never => {
                    ConfirmingTool::wrap(tool)
                }
            }
        }
    }
}

/// The sealed tool surface for a turn.
///
/// Fields are private on purpose. Property 2 is enforced by visibility: there
/// is no `push`, `extend`, `insert`, or `&mut` accessor, so code that tries to
/// add a tool after enforcement does not compile.
///
/// `Clone` is safe here: a clone of a sealed surface is still sealed.
#[derive(Clone)]
pub struct ResolvedRuntimeTools {
    tools: Vec<Arc<dyn Tool>>,
    classes: BTreeMap<String, ToolClass>,
    mcp_tool_names: BTreeSet<String>,
    connect_failures: Vec<crate::mcp::McpConnectFailure>,
}

impl ResolvedRuntimeTools {
    pub fn tools(&self) -> &[Arc<dyn Tool>] {
        &self.tools
    }

    pub fn mcp_tool_names(&self) -> &BTreeSet<String> {
        &self.mcp_tool_names
    }

    /// Servers that were configured and enabled but could not be reached.
    pub fn connect_failures(&self) -> &[crate::mcp::McpConnectFailure] {
        &self.connect_failures
    }

    /// A one-line summary per unreachable server, for display on any surface.
    pub fn connect_failure_report(&self) -> Vec<String> {
        self.connect_failures
            .iter()
            .map(|failure| {
                if failure.target.is_empty() {
                    format!("{}: {}", failure.server, failure.error)
                } else {
                    format!("{} ({}): {}", failure.server, failure.target, failure.error)
                }
            })
            .collect()
    }

    pub fn len(&self) -> usize {
        self.tools.len()
    }

    pub fn is_empty(&self) -> bool {
        self.tools.is_empty()
    }

    pub fn names(&self) -> BTreeSet<String> {
        self.tools
            .iter()
            .map(|tool| tool.name().to_string())
            .collect()
    }

    /// Recorded class for a tool, or `None` if it is not on this surface.
    pub fn class_of(&self, name: &str) -> Option<ToolClass> {
        self.classes.get(name).copied()
    }

    /// Construct a surface directly. Test-only: production code must go through
    /// [`ToolSurface::seal`] so enforcement cannot be skipped.
    #[cfg(test)]
    pub fn for_test(tools: Vec<Arc<dyn Tool>>, mcp_tool_names: BTreeSet<String>) -> Self {
        let classes = tools
            .iter()
            .map(|tool| {
                let provenance = if mcp_tool_names.contains(tool.name()) {
                    ToolProvenance::Mcp
                } else {
                    ToolProvenance::BuiltIn
                };
                (tool.name().to_string(), classify(tool, provenance))
            })
            .collect();
        Self {
            tools,
            classes,
            mcp_tool_names,
            connect_failures: Vec::new(),
        }
    }
}

/// Assemble and seal the tool surface for the current configuration.
pub async fn resolve_runtime_tools(cfg: &RuntimeConfig) -> ResolvedRuntimeTools {
    let (mcp_tools, connect_failures) = crate::mcp::discover_mcp_tools_reporting(cfg).await;

    ToolSurface::new()
        .add_builtins()
        .add_feature_gated()
        .await
        .add_mcp(mcp_tools)
        .add_connect_failures(connect_failures)
        .add_discovery_tool(DISCOVERY_TOOL_THRESHOLD)
        .seal(cfg)
}

/// Above this many tools, a model benefits from keyword discovery.
const DISCOVERY_TOOL_THRESHOLD: usize = 15;
