//! Planner/worker role routing and the bounded planner agent tool.

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use adk_rust::prelude::*;
use adk_tool::AgentTool;
use async_trait::async_trait;
use serde_json::{Value, json};

pub struct BudgetedPlannerTool {
    inner: AgentTool,
    calls: AtomicU32,
    max_calls: u32,
}

impl BudgetedPlannerTool {
    pub fn new(agent: Arc<dyn Agent>, max_calls: u32) -> Self {
        Self {
            inner: AgentTool::new(agent).timeout(std::time::Duration::from_secs(180)),
            calls: AtomicU32::new(0),
            max_calls: max_calls.max(1),
        }
    }

    pub fn calls(&self) -> u32 {
        self.calls.load(Ordering::Relaxed)
    }

    fn reserve_call(&self) -> bool {
        self.calls
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |calls| {
                (calls < self.max_calls).then_some(calls + 1)
            })
            .is_ok()
    }
}

#[async_trait]
impl Tool for BudgetedPlannerTool {
    fn name(&self) -> &str {
        "plan_work"
    }

    fn description(&self) -> &str {
        "Ask the strong planning model for a concise implementation plan when a task spans multiple files, has unclear requirements, changes architecture, or needs a material replan. Do not use for greetings, simple questions, formatting, or routine tool calls."
    }

    fn parameters_schema(&self) -> Option<Value> {
        Some(json!({
            "type": "object",
            "properties": {
                "request": {
                    "type": "string",
                    "description": "The goal, known constraints, relevant evidence, and decisions the planner must make."
                }
            },
            "required": ["request"]
        }))
    }

    async fn execute(&self, ctx: Arc<dyn ToolContext>, args: Value) -> adk_rust::Result<Value> {
        if !self.reserve_call() {
            return Ok(json!({
                "status": "budget_exhausted",
                "message": format!(
                    "Planner call budget exhausted ({}/{}). Continue with the approved plan or ask the developer before increasing the budget.",
                    self.max_calls,
                    self.max_calls
                )
            }));
        }
        self.inner.execute(ctx, args).await
    }
}

pub fn build_planner_agent(model: Arc<dyn Llm>) -> adk_rust::Result<Arc<dyn Agent>> {
    let agent = LlmAgentBuilder::new("planner")
        .description("Designs implementation plans for complex engineering work")
        .instruction(
            "You are Zavora's planning specialist. Turn the request and supplied evidence into a concise, executable plan for another agent. State the outcome, assumptions, files or systems involved, ordered implementation steps, verification, and material risks. Resolve architectural questions, but do not perform the work, call tools, or repeat the request. Keep the plan under 700 words.",
        )
        .model(model)
        .max_iterations(1)
        .build()?;
    Ok(Arc::new(agent))
}

#[cfg(test)]
mod tests {
    use super::{BudgetedPlannerTool, build_planner_agent};
    use adk_rust::model::MockLlm;
    use adk_rust::prelude::*;
    use adk_tool::SimpleToolContext;
    use serde_json::json;
    use std::sync::Arc;

    #[test]
    fn planner_budget_never_exceeds_the_configured_limit() {
        let model: Arc<dyn Llm> = Arc::new(MockLlm::new("planner-mock"));
        let agent = LlmAgentBuilder::new("planner-mock")
            .model(model)
            .build()
            .expect("mock planner should build");
        let tool = BudgetedPlannerTool::new(Arc::new(agent), 2);

        assert!(tool.reserve_call());
        assert!(tool.reserve_call());
        assert!(!tool.reserve_call());
        assert_eq!(tool.calls(), 2);
    }

    #[tokio::test]
    async fn worker_can_call_the_planner_agent_once_then_hits_the_budget() {
        let response = LlmResponse::new(Content::new("model").with_text("Inspect, edit, test."));
        let model: Arc<dyn Llm> = Arc::new(MockLlm::new("planner-mock").with_response(response));
        let planner = build_planner_agent(model).expect("planner should build");
        let tool = BudgetedPlannerTool::new(planner, 1);
        let context: Arc<dyn ToolContext> = Arc::new(SimpleToolContext::new("worker"));

        let plan = tool
            .execute(context.clone(), json!({"request": "Plan this change"}))
            .await
            .expect("planner call should execute");
        assert_eq!(plan["response"], "Inspect, edit, test.");

        let exhausted = tool
            .execute(context, json!({"request": "Plan it again"}))
            .await
            .expect("budget response should be structured");
        assert_eq!(exhausted["status"], "budget_exhausted");
        assert_eq!(tool.calls(), 1);
    }
}
