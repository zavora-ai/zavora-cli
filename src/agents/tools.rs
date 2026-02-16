/// Agent tools - Expose capability agents as callable tools.
use adk_rust::prelude::*;
use async_trait::async_trait;
use serde_json::{Value, json};
use std::sync::Arc;

use crate::agents::{memory::MemoryAgent, time::TimeAgent};

/// Time agent tool - Provides time context and parsing.
pub struct TimeAgentTool;

impl TimeAgentTool {
    pub fn new() -> Self {
        Self
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
        let action = args.get("action").and_then(Value::as_str).unwrap_or("handshake");

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

/// Memory agent tool - Provides persistent learning storage.
pub struct MemoryAgentTool {
    workspace: std::path::PathBuf,
}

impl MemoryAgentTool {
    pub fn new(workspace: std::path::PathBuf) -> Self {
        Self { workspace }
    }
}

#[async_trait]
impl Tool for MemoryAgentTool {
    fn name(&self) -> &str {
        "memory_agent"
    }

    fn description(&self) -> &str {
        "Store and recall persistent learnings across sessions. \
         Args: {\"action\": \"recall\" | \"remember\" | \"forget\", \"query\": \"<text>\", \
         \"tags\": [\"<optional tags>\"], \"confidence\": <0.0-1.0>}"
    }

    async fn execute(&self, _ctx: Arc<dyn ToolContext>, args: Value) -> adk_rust::Result<Value> {
        let mut memory = MemoryAgent::new(&self.workspace)
            .map_err(|e| adk_rust::AdkError::Tool(e.to_string()))?;
        let action = args.get("action").and_then(Value::as_str).unwrap_or("recall");

        match action {
            "recall" => {
                let query = args.get("query").and_then(Value::as_str).unwrap_or("");
                let tags: Vec<String> = args
                    .get("tags")
                    .and_then(Value::as_array)
                    .map(|arr| {
                        arr.iter()
                            .filter_map(Value::as_str)
                            .map(String::from)
                            .collect()
                    })
                    .unwrap_or_default();
                let top_k = args.get("top_k").and_then(Value::as_u64).unwrap_or(5) as usize;

                let results = memory.recall(query, &tags, top_k);
                Ok(json!({
                    "query": query,
                    "results": results.iter().map(|e| json!({
                        "text": e.text,
                        "tags": e.tags,
                        "confidence": e.confidence,
                        "created_at": e.created_at.to_rfc3339(),
                    })).collect::<Vec<_>>(),
                }))
            }
            "remember" => {
                let text = args.get("text").and_then(Value::as_str).unwrap_or("");
                if text.is_empty() {
                    return Ok(json!({"error": "text is required"}));
                }

                let tags: Vec<String> = args
                    .get("tags")
                    .and_then(Value::as_array)
                    .map(|arr| {
                        arr.iter()
                            .filter_map(Value::as_str)
                            .map(String::from)
                            .collect()
                    })
                    .unwrap_or_else(|| vec!["auto".to_string()]);

                let confidence = args
                    .get("confidence")
                    .and_then(Value::as_f64)
                    .unwrap_or(0.8) as f32;

                memory.remember(text.to_string(), tags.clone(), confidence, None)
                    .map_err(|e| adk_rust::AdkError::Tool(e.to_string()))?;
                Ok(json!({
                    "status": "stored",
                    "text": text,
                    "tags": tags,
                    "confidence": confidence,
                }))
            }
            "forget" => {
                let selector = args.get("selector").and_then(Value::as_str).unwrap_or("");
                if selector.is_empty() {
                    return Ok(json!({"error": "selector is required"}));
                }

                let removed = memory.forget(selector)
                    .map_err(|e| adk_rust::AdkError::Tool(e.to_string()))?;
                Ok(json!({
                    "status": "removed",
                    "count": removed,
                    "selector": selector,
                }))
            }
            _ => Ok(json!({"error": "Unknown action. Use 'recall', 'remember', or 'forget'"})),
        }
    }
}
