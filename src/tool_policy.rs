use std::sync::Arc;

use async_trait::async_trait;
use adk_rust::prelude::*;
use adk_rust::Result as AdkResult;
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
// Minimal test-only Tool impl for policy tests
// ---------------------------------------------------------------------------

/// A trivial `Tool` implementation used only in tests.
pub struct StubTool {
    pub tool_name: String,
}

#[async_trait]
impl Tool for StubTool {
    fn name(&self) -> &str {
        &self.tool_name
    }
    fn description(&self) -> &str {
        "stub tool for testing"
    }
    async fn execute(&self, _ctx: Arc<dyn ToolContext>, _args: Value) -> AdkResult<Value> {
        Ok(Value::Null)
    }
}
