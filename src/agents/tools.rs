/// Agent tools - Expose capability agents as callable tools.
use adk_rust::prelude::*;
use async_trait::async_trait;
use serde_json::{Value, json};
use std::sync::Arc;

use crate::agents::time::TimeAgent;

/// Time agent tool - Provides time context and parsing.
pub struct TimeAgentTool;

impl TimeAgentTool {
    pub fn new() -> Self {
        Self
    }
}

impl Default for TimeAgentTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for TimeAgentTool {
    fn name(&self) -> &str {
        "time_agent"
    }

    fn description(&self) -> &str {
        "Get current time context or parse relative time expressions. \
         Args: {\"action\": \"handshake\" | \"parse\", \"query\": \"<optional time expression>\"}"
    }

    async fn execute(&self, _ctx: Arc<dyn ToolContext>, args: Value) -> adk_rust::Result<Value> {
        let action = args
            .get("action")
            .and_then(Value::as_str)
            .unwrap_or("handshake");

        match action {
            "handshake" => {
                let ctx = TimeAgent::handshake();
                Ok(json!({
                    "now_iso": ctx.now_iso,
                    "timezone": ctx.timezone,
                    "weekday": ctx.weekday,
                    "date": ctx.date,
                }))
            }
            "parse" => {
                let query = args.get("query").and_then(Value::as_str).unwrap_or("");
                match TimeAgent::parse_relative(query) {
                    Ok(dt) => Ok(json!({
                        "query": query,
                        "result": dt.to_rfc3339(),
                        "timestamp": dt.timestamp(),
                    })),
                    Err(e) => Ok(json!({
                        "error": e.to_string(),
                        "query": query,
                    })),
                }
            }
            _ => Ok(json!({"error": "Unknown action. Use 'handshake' or 'parse'"})),
        }
    }
}

/// Memory agent tool - Provides persistent learning storage via SQLite.
pub struct MemoryAgentTool;

impl MemoryAgentTool {
    pub fn new(_workspace: std::path::PathBuf) -> Self {
        Self
    }
}

#[async_trait]
impl Tool for MemoryAgentTool {
    fn name(&self) -> &str {
        "memory_agent"
    }

    fn description(&self) -> &str {
        "Store and recall persistent learnings across sessions. \
         Args: {\"action\": \"recall\" | \"remember\" | \"forget\", \"query\": \"<text>\"}"
    }

    async fn execute(&self, _ctx: Arc<dyn ToolContext>, args: Value) -> adk_rust::Result<Value> {
        let action = args.get("action").and_then(Value::as_str).unwrap_or("recall");

        match action {
            "recall" => {
                let query = args.get("query").and_then(Value::as_str).unwrap_or("");
                let top_k = args.get("top_k").and_then(Value::as_u64).unwrap_or(5) as usize;
                let results = crate::agents::memory::recall(query, top_k)
                    .await
                    .map_err(|e| adk_rust::AdkError::tool(e.to_string()))?;
                Ok(json!({"query": query, "results": results}))
            }
            "remember" => {
                let text = args.get("text").and_then(Value::as_str).unwrap_or("");
                if text.is_empty() {
                    return Ok(json!({"error": "text is required"}));
                }
                crate::agents::memory::remember(text)
                    .await
                    .map_err(|e| adk_rust::AdkError::tool(e.to_string()))?;
                Ok(json!({"status": "stored", "text": text}))
            }
            "forget" => {
                let selector = args.get("selector").and_then(Value::as_str).unwrap_or("");
                if selector.is_empty() {
                    return Ok(json!({"error": "selector is required"}));
                }
                let removed = crate::agents::memory::forget(selector)
                    .await
                    .map_err(|e| adk_rust::AdkError::tool(e.to_string()))?;
                Ok(json!({"status": "removed", "count": removed}))
            }
            _ => Ok(json!({"error": "Unknown action. Use 'recall', 'remember', or 'forget'"})),
        }
    }
}
