use std::sync::Arc;

use adk_rust::Result as AdkResult;
use adk_rust::prelude::*;
use async_trait::async_trait;
use serde_json::Value;

// ---------------------------------------------------------------------------
// Wildcard pattern matching (simple glob: `*` matches any char sequence)
// ---------------------------------------------------------------------------

/// Match a tool name against a pattern that may contain `*` wildcards.
/// Examples: `github_ops.*` matches `github_ops.issue_create`,
///           `execute_bash.rm_*` matches `execute_bash.rm_rf`.
pub fn matches_wildcard(pattern: &str, name: &str) -> bool {
    let parts: Vec<&str> = pattern.split('*').collect();
    if parts.len() == 1 {
        return pattern == name;
    }
    let mut remaining = name;
    for (i, part) in parts.iter().enumerate() {
        if part.is_empty() {
            continue;
        }
        if i == 0 {
            if !remaining.starts_with(part) {
                return false;
            }
            remaining = &remaining[part.len()..];
        } else if i == parts.len() - 1 {
            if !remaining.ends_with(part) {
                return false;
            }
            return true;
        } else {
            match remaining.find(part) {
                Some(pos) => remaining = &remaining[pos + part.len()..],
                None => return false,
            }
        }
    }
    true
}

/// Check if any pattern in the list matches the given tool name.
pub fn any_pattern_matches(patterns: &[&str], name: &str) -> bool {
    patterns.iter().any(|p| matches_wildcard(p, name))
}

// ---------------------------------------------------------------------------
// Tool alias wrapper
// ---------------------------------------------------------------------------

/// Wraps an existing `Tool` and presents it under a different name.
pub struct AliasedTool {
    alias: String,
    inner: Arc<dyn Tool>,
}

impl AliasedTool {
    pub fn new(alias: String, inner: Arc<dyn Tool>) -> Self {
        Self { alias, inner }
    }
}

#[async_trait]
impl Tool for AliasedTool {
    fn name(&self) -> &str {
        &self.alias
    }

    fn description(&self) -> &str {
        self.inner.description()
    }

    fn enhanced_description(&self) -> String {
        self.inner.enhanced_description()
    }

    fn is_long_running(&self) -> bool {
        self.inner.is_long_running()
    }

    /// Forwarded for the same reason as on `ConfirmingTool`: an alias must not
    /// change what a tool is allowed to do. Requirement 7.3.
    fn is_read_only(&self) -> bool {
        self.inner.is_read_only()
    }

    fn is_concurrency_safe(&self) -> bool {
        self.inner.is_concurrency_safe()
    }

    fn parameters_schema(&self) -> Option<Value> {
        self.inner.parameters_schema()
    }

    fn response_schema(&self) -> Option<Value> {
        self.inner.response_schema()
    }

    async fn execute(&self, ctx: Arc<dyn ToolContext>, args: Value) -> AdkResult<Value> {
        self.inner.execute(ctx, args).await
    }
}

// ---------------------------------------------------------------------------
// Apply aliases to a set of discovered tools
// ---------------------------------------------------------------------------

use std::collections::HashMap;

/// Rename tools according to alias mappings. Keys are original names, values
/// are the desired alias. Tools not in the map pass through unchanged.
pub fn apply_tool_aliases(
    tools: Vec<Arc<dyn Tool>>,
    aliases: &HashMap<String, String>,
) -> Vec<Arc<dyn Tool>> {
    if aliases.is_empty() {
        return tools;
    }
    tools
        .into_iter()
        .map(|tool| {
            if let Some(alias) = aliases.get(tool.name()) {
                tracing::info!(
                    original = tool.name(),
                    alias = alias.as_str(),
                    "Aliased MCP tool"
                );
                Arc::new(AliasedTool::new(alias.clone(), tool)) as Arc<dyn Tool>
            } else {
                tool
            }
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Filter tools by allow/deny with wildcard support
// ---------------------------------------------------------------------------

/// Retain only tools matching at least one allow pattern. If allow list is
/// empty, all tools pass through. Deny patterns are applied after allow and
/// take precedence — a tool matching both allow and deny is denied.
pub fn filter_tools_by_policy(
    tools: Vec<Arc<dyn Tool>>,
    allow_patterns: &[String],
    deny_patterns: &[String],
) -> Vec<Arc<dyn Tool>> {
    let allow: Vec<&str> = allow_patterns
        .iter()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .collect();
    let deny: Vec<&str> = deny_patterns
        .iter()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .collect();

    tools
        .into_iter()
        .filter(|tool| {
            let name = tool.name();
            let allowed = allow.is_empty() || any_pattern_matches(&allow, name);
            let denied = !deny.is_empty() && any_pattern_matches(&deny, name);
            if denied {
                tracing::debug!(tool = name, "Tool denied by deny_tools policy");
            }
            allowed && !denied
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Permission rules (layered permission system)
// ---------------------------------------------------------------------------

use serde::{Deserialize, Serialize};

/// A permission rule pattern in the form "tool_name:content_pattern" or just "tool_name".
/// Examples: "fs_read:*", "execute_bash:git status*", "fs_write:/etc/*"
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(transparent)]
pub struct ToolPattern(pub String);

impl ToolPattern {
    /// Check if this pattern matches a tool call.
    /// Pattern format: "tool_glob" or "tool_glob:content_glob"
    pub fn matches(&self, tool_name: &str, content: Option<&str>) -> bool {
        if let Some((tool_pat, content_pat)) = self.0.split_once(':') {
            matches_wildcard(tool_pat, tool_name)
                && content
                    .map(|c| matches_wildcard(content_pat, c))
                    .unwrap_or(false)
        } else {
            matches_wildcard(&self.0, tool_name)
        }
    }
}

/// Permission decision from rule matching.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PermissionDecision {
    Allow,
    Deny,
    Ask,
    /// No rule matched — fall through to default behavior.
    NoMatch,
}

/// Layered permission rules. First match wins across always_deny → always_allow → always_ask.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct PermissionRules {
    #[serde(default)]
    pub always_allow: Vec<ToolPattern>,
    #[serde(default)]
    pub always_deny: Vec<ToolPattern>,
    #[serde(default)]
    pub always_ask: Vec<ToolPattern>,
}

impl PermissionRules {
    pub fn is_empty(&self) -> bool {
        self.always_allow.is_empty() && self.always_deny.is_empty() && self.always_ask.is_empty()
    }

    /// Evaluate rules for a tool call. Returns the first matching decision.
    /// Check order: deny → allow → ask (deny takes precedence).
    pub fn evaluate(&self, tool_name: &str, content: Option<&str>) -> PermissionDecision {
        // Deny first (highest priority)
        if self
            .always_deny
            .iter()
            .any(|p| p.matches(tool_name, content))
        {
            return PermissionDecision::Deny;
        }
        // Allow
        if self
            .always_allow
            .iter()
            .any(|p| p.matches(tool_name, content))
        {
            return PermissionDecision::Allow;
        }
        // Ask
        if self
            .always_ask
            .iter()
            .any(|p| p.matches(tool_name, content))
        {
            return PermissionDecision::Ask;
        }
        PermissionDecision::NoMatch
    }

    /// Merge another set of rules (overlay takes precedence by being checked first).
    pub fn merge_overlay(&self, overlay: &PermissionRules) -> PermissionRules {
        let mut merged = overlay.clone();
        merged
            .always_allow
            .extend(self.always_allow.iter().cloned());
        merged.always_deny.extend(self.always_deny.iter().cloned());
        merged.always_ask.extend(self.always_ask.iter().cloned());
        merged
    }
}

/// Where a tool came from. Provenance cannot be read off the `Tool` trait, so
/// the surface builder records it when the tool is added.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolProvenance {
    /// Compiled into the binary.
    BuiltIn,
    /// Compiled in behind an optional feature that drives a remote surface.
    FeatureGated,
    /// Contributed by an installed plugin package.
    Plugin,
    /// Discovered from an MCP server at runtime.
    Mcp,
}

/// What a tool can do to the world. Derived from the tool itself rather than
/// from a hand-maintained name list, so registering a new tool cannot
/// accidentally create an unguarded path.
///
/// Requirement 7.3; Correctness Property 5.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolClass {
    /// Observes state without changing it. Auto-approvable.
    ReadOnly,
    /// Mutates the filesystem, a repository, or a process.
    Mutating,
    /// Sends data off the machine. Never auto-approved, in any mode.
    NetworkEgress,
}

impl ToolClass {
    /// True when this class may run without an explicit approval.
    ///
    /// `NetworkEgress` answers false even though a fetch mutates nothing
    /// locally: the developer's data leaving the machine is the consequential
    /// act, and it cannot be undone once it has happened.
    pub fn is_auto_approvable(self) -> bool {
        matches!(self, ToolClass::ReadOnly)
    }
}

/// Built-in tools that send data off the machine.
///
/// This list names egress, not read/write, so it does not duplicate the
/// classification that `Tool::is_read_only()` already carries. Adding a tool
/// here is the only way to declare local-egress intent for a built-in.
pub const NETWORK_EGRESS_TOOLS: &[&str] = &["web_fetch", "github_ops"];

/// Classify a tool from its own declared capability plus its provenance.
///
/// Order matters. Egress is checked first because a tool can be read-only in
/// the local sense and still exfiltrate: `web_fetch` mutates nothing on disk.
/// Anything arriving from outside the binary — an MCP server, a plugin, a
/// feature-gated remote surface such as the browser — is treated as egress
/// because its blast radius is not knowable from here.
pub fn classify(tool: &Arc<dyn Tool>, provenance: ToolProvenance) -> ToolClass {
    let name = tool.name();

    if NETWORK_EGRESS_TOOLS.contains(&name) {
        return ToolClass::NetworkEgress;
    }

    match provenance {
        ToolProvenance::Mcp | ToolProvenance::Plugin | ToolProvenance::FeatureGated => {
            return ToolClass::NetworkEgress;
        }
        ToolProvenance::BuiltIn => {}
    }

    if tool.is_read_only() {
        ToolClass::ReadOnly
    } else {
        ToolClass::Mutating
    }
}

// StubTool moved to tests.rs — not needed in production code.
