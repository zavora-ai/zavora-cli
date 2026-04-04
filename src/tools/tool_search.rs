//! Tool search — keyword-based tool discovery for large tool sets.
//!
//! When enabled and tool count exceeds a threshold, only core tools are
//! included in the system prompt. Other tools are deferred and discoverable
//! via this search tool.

use std::sync::Arc;

use adk_rust::prelude::*;
use serde_json::{Value, json};

/// Search available tools by keyword, returning matching tool schemas.
pub fn tool_search_response(query: &str, all_tools: &[Arc<dyn Tool>]) -> Value {
    if query.trim().is_empty() {
        return json!({
            "status": "error",
            "error": "query is required"
        });
    }

    let terms: Vec<String> = query
        .split_whitespace()
        .map(|t| t.to_ascii_lowercase())
        .collect();

    let matches: Vec<Value> = all_tools
        .iter()
        .filter(|t| {
            let name = t.name().to_ascii_lowercase();
            let desc = t.description().to_ascii_lowercase();
            terms.iter().any(|term| name.contains(term) || desc.contains(term))
        })
        .map(|t| {
            json!({
                "name": t.name(),
                "description": t.description(),
                "parameters": t.parameters_schema(),
            })
        })
        .collect();

    json!({
        "query": query,
        "matches": matches.len(),
        "tools": matches,
    })
}

/// Core tools always included in the system prompt (even when tool search is active).
pub const CORE_TOOLS: &[&str] = &[
    "fs_read", "fs_write", "file_edit", "execute_bash", "glob", "grep", "tool_search",
];

/// Check if a tool is a core tool.
pub fn is_core_tool(name: &str) -> bool {
    CORE_TOOLS.contains(&name)
}
