/// Search agent - Web search via Gemini's built-in Google Search.
///
/// This agent is capability-gated: only available when using Gemini model.
use adk_rust::prelude::*;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResult {
    pub title: String,
    pub url: String,
    pub snippet: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvidenceBundle {
    pub query: String,
    pub results: Vec<SearchResult>,
    pub extracted_facts: Option<Vec<String>>,
    pub confidence: f32,
}

/// Check if search capability is available (Gemini model).
pub fn is_search_available(provider: &str) -> bool {
    provider.to_lowercase() == "gemini"
}

/// Build search agent with Google Search tool.
pub fn build_search_agent(model: Arc<dyn Llm>) -> Result<Arc<dyn Agent>> {
    let search_tool = adk_tool::builtin::GoogleSearchTool::new();
    
    let agent = LlmAgentBuilder::new("search_agent")
        .description("Web search specialist using Google Search")
        .instruction(
            "You are a search specialist. Your job is to:\n\
             1. Formulate effective search queries\n\
             2. Execute searches using google_search tool\n\
             3. Extract key facts from results\n\
             4. Provide evidence bundle with citations\n\n\
             Always include:\n\
             - Original query\n\
             - Top results (title, URL, snippet)\n\
             - Extracted facts (if applicable)\n\
             - Confidence score (0.0-1.0)\n\n\
             Format your response as structured data, not conversational text."
        )
        .model(model)
        .tool(Arc::new(search_tool))
        .build()?;
    
    Ok(Arc::new(agent))
}

/// Capability missing response when search is not available.
pub fn capability_missing_response() -> String {
    serde_json::json!({
        "error": "CapabilityMissing",
        "message": "Search agent requires Gemini model with Google Search",
        "suggestion": "Use --provider gemini --model gemini-2.0-flash-exp"
    })
    .to_string()
}
