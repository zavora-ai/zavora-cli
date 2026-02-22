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

const SEARCH_AGENT_INSTRUCTION: &str = r#"You are a web research specialist.

Use the google_search tool for current information and return a single JSON object with:
- query: the final query executed
- results: top relevant results [{title, url, snippet}]
- extracted_facts: concise factual bullets backed by the results (or empty)
- confidence: float in [0.0, 1.0]

Rules:
- Prefer recency and source quality.
- Include direct URLs for all reported facts.
- If evidence is weak or conflicting, lower confidence and say so in extracted_facts."#;

/// Check if search capability is available (Gemini model).
pub fn is_search_available(provider: &str) -> bool {
    provider.eq_ignore_ascii_case("gemini")
}

/// Build search agent with Google Search tool.
pub fn build_search_agent(model: Arc<dyn Llm>) -> Result<Arc<dyn Agent>> {
    let search_tool = adk_tool::builtin::GoogleSearchTool::new();

    let agent = LlmAgentBuilder::new("search_agent")
        .description("Web research specialist using Google Search")
        .instruction(SEARCH_AGENT_INSTRUCTION)
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
        "suggestion": "Use --provider gemini --model gemini-2.5-flash"
    })
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    #[test]
    fn is_search_available_is_case_insensitive_for_gemini() {
        assert!(is_search_available("gemini"));
        assert!(is_search_available("GEMINI"));
        assert!(is_search_available("Gemini"));
        assert!(!is_search_available("openai"));
    }

    #[test]
    fn capability_missing_response_includes_provider_hint() {
        let response = capability_missing_response();
        let payload: Value =
            serde_json::from_str(&response).expect("response should be valid json");
        assert_eq!(payload["error"], "CapabilityMissing");
        assert!(
            payload["suggestion"]
                .as_str()
                .unwrap_or_default()
                .contains("--provider gemini")
        );
    }
}
