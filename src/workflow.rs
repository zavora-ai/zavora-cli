use std::sync::Arc;
use std::time::Duration;

use adk_rust::ToolConfirmationPolicy;
use adk_rust::futures::StreamExt;
use adk_rust::prelude::*;
use anyhow::{Context, Result};
use serde_json::{Value, json};

use crate::cli::WorkflowMode;
use crate::config::RuntimeConfig;
use crate::runner::build_single_agent_with_tools;

pub fn build_workflow_agent(
    mode: WorkflowMode,
    model: Arc<dyn Llm>,
    max_iterations: u32,
    tools: &[Arc<dyn Tool>],
    tool_confirmation_policy: ToolConfirmationPolicy,
    tool_timeout: Duration,
    runtime_cfg: Option<&RuntimeConfig>,
) -> Result<Arc<dyn Agent>> {
    match mode {
        WorkflowMode::Single => build_single_agent_with_tools(
            model,
            tools,
            tool_confirmation_policy,
            tool_timeout,
            runtime_cfg,
        ),
        WorkflowMode::Sequential => build_sequential_agent(model),
        WorkflowMode::Parallel => build_parallel_agent(model),
        WorkflowMode::Loop => build_loop_agent(model, max_iterations),
        WorkflowMode::Graph => build_graph_workflow_agent(model),
    }
}

pub fn classify_workflow_route(input: &str) -> &'static str {
    let lower = input.to_ascii_lowercase();
    if lower.contains("risk")
        || lower.contains("rollback")
        || lower.contains("mitigation")
        || lower.contains("incident")
    {
        return "risk";
    }
    if lower.contains("architecture")
        || lower.contains("design")
        || lower.contains("system")
        || lower.contains("scal")
    {
        return "architecture";
    }
    if lower.contains("release")
        || lower.contains("sprint")
        || lower.contains("milestone")
        || lower.contains("roadmap")
    {
        return "release";
    }
    "delivery"
}

pub fn workflow_template(route: &str) -> &'static str {
    match route {
        "release" => {
            "Template: Release Planning\n\
             Return concise markdown with sections: Objectives, Release Slices, Acceptance \
             Criteria, Rollout Steps."
        }
        "architecture" => {
            "Template: Architecture Design\n\
             Return concise markdown with sections: Constraints, Proposed Components, \
             Data/Control Flow, Risks."
        }
        "risk" => {
            "Template: Risk and Reliability\n\
             Return concise markdown with sections: Top Risks, Impact, Mitigation, \
             Detection, Fallback."
        }
        _ => {
            "Template: Execution Delivery\n\
             Return concise markdown with sections: Scope, Implementation Steps, \
             Validation, Next Actions."
        }
    }
}

async fn generate_model_text(model: Arc<dyn Llm>, prompt: &str) -> Result<String> {
    let req = LlmRequest::new(
        model.name().to_string(),
        vec![Content::new("user").with_text(prompt)],
    );
    let mut stream = model
        .generate_content(req, false)
        .await
        .context("failed to invoke model inside graph workflow")?;

    let mut out = String::new();
    while let Some(chunk_result) = stream.next().await {
        let chunk = chunk_result.context("graph workflow model stream error")?;
        if let Some(content) = chunk.content {
            for part in content.parts {
                if let Part::Text { text } = part {
                    out.push_str(&text);
                }
            }
        }
    }

    let trimmed = out.trim();
    if trimmed.is_empty() {
        return Err(anyhow::anyhow!(
            "graph workflow did not produce textual model output"
        ));
    }
    Ok(trimmed.to_string())
}

fn build_graph_workflow_agent(model: Arc<dyn Llm>) -> Result<Arc<dyn Agent>> {
    let route_classifier = |ctx: adk_rust::graph::NodeContext| async move {
        let input = ctx
            .get("input")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let route = classify_workflow_route(&input);
        Ok(NodeOutput::new().with_update("route", json!(route)))
    };

    let release_prep = |ctx: adk_rust::graph::NodeContext| async move {
        let input = ctx
            .get("input")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let prompt = format!(
            "{}\n\nUser request:\n{}",
            workflow_template("release"),
            input
        );
        Ok(NodeOutput::new().with_update("branch_prompt", json!(prompt)))
    };

    let architecture_prep = |ctx: adk_rust::graph::NodeContext| async move {
        let input = ctx
            .get("input")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let prompt = format!(
            "{}\n\nUser request:\n{}",
            workflow_template("architecture"),
            input
        );
        Ok(NodeOutput::new().with_update("branch_prompt", json!(prompt)))
    };

    let risk_prep = |ctx: adk_rust::graph::NodeContext| async move {
        let input = ctx
            .get("input")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let prompt = format!("{}\n\nUser request:\n{}", workflow_template("risk"), input);
        Ok(NodeOutput::new().with_update("branch_prompt", json!(prompt)))
    };

    let delivery_prep = |ctx: adk_rust::graph::NodeContext| async move {
        let input = ctx
            .get("input")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let prompt = format!(
            "{}\n\nUser request:\n{}",
            workflow_template("delivery"),
            input
        );
        Ok(NodeOutput::new().with_update("branch_prompt", json!(prompt)))
    };

    let model_for_draft = model.clone();
    let draft = move |ctx: adk_rust::graph::NodeContext| {
        let model_for_draft = model_for_draft.clone();
        async move {
            let prompt = ctx
                .get("branch_prompt")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            let route_selected = ctx
                .get("route")
                .and_then(Value::as_str)
                .unwrap_or("delivery")
                .to_string();

            let output = generate_model_text(model_for_draft, &prompt)
                .await
                .map_err(|err| adk_rust::graph::GraphError::NodeExecutionFailed {
                    node: "draft_response".to_string(),
                    message: err.to_string(),
                })?;

            Ok(NodeOutput::new()
                .with_update("output", json!(output))
                .with_update("route_selected", json!(route_selected)))
        }
    };

    let agent = GraphAgent::builder("graph_delivery")
        .description("Graph-routed orchestration workflow")
        .channels(&[
            "input",
            "route",
            "branch_prompt",
            "output",
            "route_selected",
        ])
        .node_fn("classify", route_classifier)
        .node_fn("prepare_release", release_prep)
        .node_fn("prepare_architecture", architecture_prep)
        .node_fn("prepare_risk", risk_prep)
        .node_fn("prepare_delivery", delivery_prep)
        .node_fn("draft_response", draft)
        .edge(START, "classify")
        .conditional_edge(
            "classify",
            Router::by_field("route"),
            [
                ("release", "prepare_release"),
                ("architecture", "prepare_architecture"),
                ("risk", "prepare_risk"),
                ("delivery", "prepare_delivery"),
            ],
        )
        .edge("prepare_release", "draft_response")
        .edge("prepare_architecture", "draft_response")
        .edge("prepare_risk", "draft_response")
        .edge("prepare_delivery", "draft_response")
        .edge("draft_response", END)
        .build()?;

    Ok(Arc::new(agent))
}

fn build_sequential_agent(model: Arc<dyn Llm>) -> Result<Arc<dyn Agent>> {
    let scope = Arc::new(
        LlmAgentBuilder::new("scope_analyst")
            .description("Defines a concise project scope.")
            .instruction(
                "Analyze the user's request and produce a compact scope. Include assumptions, \
                 constraints, and high-risk areas.",
            )
            .model(model.clone())
            .output_key("scope_summary")
            .build()?,
    );

    let release_planner = Arc::new(
        LlmAgentBuilder::new("release_planner")
            .description("Breaks scope into release increments.")
            .instruction(
                "Using {scope_summary}, produce release-by-release slices with explicit acceptance \
                 criteria.",
            )
            .model(model.clone())
            .output_key("release_breakdown")
            .build()?,
    );

    let execution_writer = Arc::new(
        LlmAgentBuilder::new("execution_writer")
            .description("Produces the final actionable response.")
            .instruction(
                "Using {release_breakdown}, write the final answer as a practical execution guide \
                 with milestones, quality gates, and risks.",
            )
            .model(model)
            .build()?,
    );

    let agent = SequentialAgent::new(
        "sequential_delivery",
        vec![
            scope as Arc<dyn Agent>,
            release_planner as Arc<dyn Agent>,
            execution_writer as Arc<dyn Agent>,
        ],
    );

    Ok(Arc::new(agent))
}

fn build_parallel_agent(model: Arc<dyn Llm>) -> Result<Arc<dyn Agent>> {
    let architecture = Arc::new(
        LlmAgentBuilder::new("architecture_analyst")
            .description("Focuses architecture and decomposition.")
            .instruction(
                "Analyze architecture decisions and implementation decomposition for the user \
                 request.",
            )
            .model(model.clone())
            .output_key("architecture_notes")
            .build()?,
    );
    let risk = Arc::new(
        LlmAgentBuilder::new("risk_analyst")
            .description("Focuses delivery and operational risk.")
            .instruction(
                "Analyze delivery, security, and rollout risks for the user request. Keep it \
                 concrete.",
            )
            .model(model.clone())
            .output_key("risk_notes")
            .build()?,
    );
    let quality = Arc::new(
        LlmAgentBuilder::new("quality_analyst")
            .description("Focuses test and quality gates.")
            .instruction(
                "Analyze quality strategy, testing layers, and release criteria for the user \
                 request.",
            )
            .model(model.clone())
            .output_key("quality_notes")
            .build()?,
    );

    let parallel = Arc::new(ParallelAgent::new(
        "analysis_swarm",
        vec![
            architecture as Arc<dyn Agent>,
            risk as Arc<dyn Agent>,
            quality as Arc<dyn Agent>,
        ],
    ));

    let synthesizer = Arc::new(
        LlmAgentBuilder::new("synthesizer")
            .description("Merges parallel analysis into one plan.")
            .instruction(
                "Synthesize the results into one coherent plan.\n\
                 Architecture: {architecture_notes?}\n\
                 Risks: {risk_notes?}\n\
                 Quality: {quality_notes?}\n\
                 Return a single clear execution plan.",
            )
            .model(model)
            .build()?,
    );

    let root = SequentialAgent::new(
        "parallel_delivery",
        vec![parallel as Arc<dyn Agent>, synthesizer as Arc<dyn Agent>],
    );
    Ok(Arc::new(root))
}

fn build_loop_agent(model: Arc<dyn Llm>, max_iterations: u32) -> Result<Arc<dyn Agent>> {
    let iterative = Arc::new(
        LlmAgentBuilder::new("iterative_refiner")
            .description("Refines the answer until quality is acceptable.")
            .instruction(
                "Maintain and improve a draft in {draft?}. Initialize from user request if empty. \
                 Improve one step per turn. Call exit_loop when the draft is release-ready.",
            )
            .model(model.clone())
            .tool(Arc::new(ExitLoopTool::new()))
            .output_key("draft")
            .max_iterations(24)
            .build()?,
    );

    let loop_agent = Arc::new(
        LoopAgent::new("loop_refinement", vec![iterative as Arc<dyn Agent>])
            .with_max_iterations(max_iterations.max(1)),
    );

    let finalizer = Arc::new(
        LlmAgentBuilder::new("loop_finalizer")
            .description("Formats the final loop result.")
            .instruction(
                "Return the final polished response from {draft?}. If draft is empty, provide the \
                 best concise answer directly.",
            )
            .model(model)
            .build()?,
    );

    let root = SequentialAgent::new(
        "loop_delivery",
        vec![loop_agent as Arc<dyn Agent>, finalizer as Arc<dyn Agent>],
    );
    Ok(Arc::new(root))
}

pub fn build_release_planning_agent(model: Arc<dyn Llm>, releases: u32) -> Result<Arc<dyn Agent>> {
    let scoper = Arc::new(
        LlmAgentBuilder::new("product_scoper")
            .instruction(
                "Turn the user goal into a product scope with assumptions, constraints, and \
                 measurable outcomes.",
            )
            .model(model.clone())
            .output_key("product_scope")
            .build()?,
    );

    let release_architect = Arc::new(
        LlmAgentBuilder::new("release_architect")
            .instruction(format!(
                "Create an agile release plan across {} releases from {{product_scope}}. \
                 For each release include objective, scope, validation, and demo output.",
                releases
            ))
            .model(model.clone())
            .output_key("release_plan")
            .build()?,
    );

    let final_writer = Arc::new(
        LlmAgentBuilder::new("release_writer")
            .instruction(
                "Return the final answer in markdown with sections:\n\
                 - Vision\n\
                 - Release Breakdown\n\
                 - Definition of Done per release\n\
                 - Risks and mitigations\n\
                 - Next sprint start tasks\n\
                 Use {release_plan}.",
            )
            .model(model)
            .build()?,
    );

    let root = SequentialAgent::new(
        "release_planning_pipeline",
        vec![
            scoper as Arc<dyn Agent>,
            release_architect as Arc<dyn Agent>,
            final_writer as Arc<dyn Agent>,
        ],
    );
    Ok(Arc::new(root))
}
