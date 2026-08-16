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
        WorkflowMode::Sequential => build_sequential_agent(model, runtime_cfg),
        WorkflowMode::Parallel => build_parallel_agent(model, runtime_cfg),
        WorkflowMode::Loop => build_loop_agent(model, max_iterations, runtime_cfg),
        WorkflowMode::Graph => build_graph_workflow_agent(model, runtime_cfg),
    }
}

fn workspace_instruction_block(runtime_cfg: Option<&RuntimeConfig>) -> String {
    if runtime_cfg.is_none() {
        return String::new();
    }
    match crate::skills::resolve_workspace_instructions() {
        Ok(instructions) if !instructions.content.is_empty() => format!(
            "\n\n<workspace_instructions>\n{}\n</workspace_instructions>",
            instructions.content
        ),
        Ok(_) => String::new(),
        Err(error) => {
            tracing::warn!(%error, "workflow project instruction loading unavailable");
            String::new()
        }
    }
}

fn with_workspace_instructions(base: &str, workspace: &str) -> String {
    format!("{base}{workspace}")
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

fn build_graph_workflow_agent(
    model: Arc<dyn Llm>,
    runtime_cfg: Option<&RuntimeConfig>,
) -> Result<Arc<dyn Agent>> {
    let workspace = workspace_instruction_block(runtime_cfg);
    let route_classifier = |ctx: adk_rust::graph::NodeContext| async move {
        let input = ctx
            .get("input")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let route = classify_workflow_route(&input);
        Ok(NodeOutput::new().with_update("route", json!(route)))
    };

    let release_workspace = workspace.clone();
    let release_prep = move |ctx: adk_rust::graph::NodeContext| {
        let workspace = release_workspace.clone();
        async move {
            let input = ctx
                .get("input")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            let prompt = format!(
                "{}{}\n\nUser request:\n{}",
                workflow_template("release"),
                workspace,
                input
            );
            Ok(NodeOutput::new().with_update("branch_prompt", json!(prompt)))
        }
    };

    let architecture_workspace = workspace.clone();
    let architecture_prep = move |ctx: adk_rust::graph::NodeContext| {
        let workspace = architecture_workspace.clone();
        async move {
            let input = ctx
                .get("input")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            let prompt = format!(
                "{}{}\n\nUser request:\n{}",
                workflow_template("architecture"),
                workspace,
                input
            );
            Ok(NodeOutput::new().with_update("branch_prompt", json!(prompt)))
        }
    };

    let risk_workspace = workspace.clone();
    let risk_prep = move |ctx: adk_rust::graph::NodeContext| {
        let workspace = risk_workspace.clone();
        async move {
            let input = ctx
                .get("input")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            let prompt = format!(
                "{}{}\n\nUser request:\n{}",
                workflow_template("risk"),
                workspace,
                input
            );
            Ok(NodeOutput::new().with_update("branch_prompt", json!(prompt)))
        }
    };

    let delivery_prep = move |ctx: adk_rust::graph::NodeContext| {
        let workspace = workspace.clone();
        async move {
            let input = ctx
                .get("input")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            let prompt = format!(
                "{}{}\n\nUser request:\n{}",
                workflow_template("delivery"),
                workspace,
                input
            );
            Ok(NodeOutput::new().with_update("branch_prompt", json!(prompt)))
        }
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

fn build_sequential_agent(
    model: Arc<dyn Llm>,
    runtime_cfg: Option<&RuntimeConfig>,
) -> Result<Arc<dyn Agent>> {
    let workspace = workspace_instruction_block(runtime_cfg);
    let scope = Arc::new(
        LlmAgentBuilder::new("scope_analyst")
            .description("Defines a concise project scope.")
            .instruction(with_workspace_instructions(
                "Analyze the user's request and produce a compact scope. Include assumptions, \
                 constraints, and high-risk areas.",
                &workspace,
            ))
            .model(model.clone())
            .output_key("scope_summary")
            .build()?,
    );

    let release_planner = Arc::new(
        LlmAgentBuilder::new("release_planner")
            .description("Breaks scope into release increments.")
            .instruction(with_workspace_instructions(
                "Using {scope_summary}, produce release-by-release slices with explicit acceptance \
                 criteria.",
                &workspace,
            ))
            .model(model.clone())
            .output_key("release_breakdown")
            .build()?,
    );

    let execution_writer = Arc::new(
        LlmAgentBuilder::new("execution_writer")
            .description("Produces the final actionable response.")
            .instruction(with_workspace_instructions(
                "Using {release_breakdown}, write the final answer as a practical execution guide \
                 with milestones, quality gates, and risks.",
                &workspace,
            ))
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

fn build_parallel_agent(
    model: Arc<dyn Llm>,
    runtime_cfg: Option<&RuntimeConfig>,
) -> Result<Arc<dyn Agent>> {
    let workspace = workspace_instruction_block(runtime_cfg);
    let architecture = Arc::new(
        LlmAgentBuilder::new("architecture_analyst")
            .description("Focuses architecture and decomposition.")
            .instruction(with_workspace_instructions(
                "Analyze architecture decisions and implementation decomposition for the user \
                 request.",
                &workspace,
            ))
            .model(model.clone())
            .output_key("architecture_notes")
            .build()?,
    );
    let risk = Arc::new(
        LlmAgentBuilder::new("risk_analyst")
            .description("Focuses delivery and operational risk.")
            .instruction(with_workspace_instructions(
                "Analyze delivery, security, and rollout risks for the user request. Keep it \
                 concrete.",
                &workspace,
            ))
            .model(model.clone())
            .output_key("risk_notes")
            .build()?,
    );
    let quality = Arc::new(
        LlmAgentBuilder::new("quality_analyst")
            .description("Focuses test and quality gates.")
            .instruction(with_workspace_instructions(
                "Analyze quality strategy, testing layers, and release criteria for the user \
                 request.",
                &workspace,
            ))
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
            .instruction(with_workspace_instructions(
                "Synthesize the results into one coherent plan.\n\
                 Architecture: {architecture_notes?}\n\
                 Risks: {risk_notes?}\n\
                 Quality: {quality_notes?}\n\
                 Return a single clear execution plan.",
                &workspace,
            ))
            .model(model)
            .build()?,
    );

    let root = SequentialAgent::new(
        "parallel_delivery",
        vec![parallel as Arc<dyn Agent>, synthesizer as Arc<dyn Agent>],
    );
    Ok(Arc::new(root))
}

fn build_loop_agent(
    model: Arc<dyn Llm>,
    max_iterations: u32,
    runtime_cfg: Option<&RuntimeConfig>,
) -> Result<Arc<dyn Agent>> {
    let workspace = workspace_instruction_block(runtime_cfg);
    let iterative = Arc::new(
        LlmAgentBuilder::new("iterative_refiner")
            .description("Refines the answer until quality is acceptable.")
            .instruction(with_workspace_instructions(
                "Maintain and improve a draft in {draft?}. Initialize from user request if empty. \
                 Improve one step per turn. Call exit_loop when the draft is release-ready.",
                &workspace,
            ))
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
            .instruction(with_workspace_instructions(
                "Return the final polished response from {draft?}. If draft is empty, provide the \
                 best concise answer directly.",
                &workspace,
            ))
            .model(model)
            .build()?,
    );

    let root = SequentialAgent::new(
        "loop_delivery",
        vec![loop_agent as Arc<dyn Agent>, finalizer as Arc<dyn Agent>],
    );
    Ok(Arc::new(root))
}

pub fn build_release_planning_agent(
    model: Arc<dyn Llm>,
    releases: u32,
    runtime_cfg: Option<&RuntimeConfig>,
) -> Result<Arc<dyn Agent>> {
    let workspace = workspace_instruction_block(runtime_cfg);
    let scoper = Arc::new(
        LlmAgentBuilder::new("product_scoper")
            .instruction(with_workspace_instructions(
                "Turn the user goal into a product scope with assumptions, constraints, and \
                 measurable outcomes.",
                &workspace,
            ))
            .model(model.clone())
            .output_key("product_scope")
            .build()?,
    );

    let release_architect = Arc::new(
        LlmAgentBuilder::new("release_architect")
            .instruction(with_workspace_instructions(
                &format!(
                    "Create an agile release plan across {} releases from {{product_scope}}. \
                 For each release include objective, scope, validation, and demo output.",
                    releases
                ),
                &workspace,
            ))
            .model(model.clone())
            .output_key("release_plan")
            .build()?,
    );

    let final_writer = Arc::new(
        LlmAgentBuilder::new("release_writer")
            .instruction(with_workspace_instructions(
                "Return the final answer in markdown with sections:\n\
                 - Vision\n\
                 - Release Breakdown\n\
                 - Definition of Done per release\n\
                 - Risks and mitigations\n\
                 - Next sprint start tasks\n\
                 Use {release_plan}.",
                &workspace,
            ))
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

/// Coverage for workflow route classification and templates, which had none.
///
/// Requirement 13.7. Route classification decides which template a prompt gets,
/// so a silent misclassification changes every downstream section heading.
#[cfg(test)]
mod workflow_tests {
    use super::*;

    #[test]
    fn risk_language_routes_to_risk() {
        for prompt in [
            "assess the rollback risk",
            "incident review for the outage",
            "what mitigation do we need",
            "RISK register",
        ] {
            assert_eq!(classify_workflow_route(prompt), "risk", "{prompt}");
        }
    }

    #[test]
    fn architecture_language_routes_to_architecture() {
        for prompt in [
            "design the ingestion system",
            "how should this scale",
            "review the architecture",
        ] {
            assert_eq!(classify_workflow_route(prompt), "architecture", "{prompt}");
        }
    }

    #[test]
    fn release_language_routes_to_release() {
        for prompt in ["plan the next sprint", "milestone roadmap", "cut a release"] {
            assert_eq!(classify_workflow_route(prompt), "release", "{prompt}");
        }
    }

    #[test]
    fn unclassified_input_falls_back_to_delivery() {
        for prompt in ["add a button", "", "fix the typo in the header"] {
            assert_eq!(classify_workflow_route(prompt), "delivery", "{prompt}");
        }
    }

    /// Risk is checked before architecture, so a prompt naming both is routed to
    /// risk. Pinning the precedence prevents an accidental reorder.
    #[test]
    fn risk_takes_precedence_over_architecture() {
        assert_eq!(
            classify_workflow_route("architecture risk review"),
            "risk",
            "risk must be checked before architecture"
        );
    }

    #[test]
    fn classification_is_case_insensitive() {
        assert_eq!(classify_workflow_route("ROLLBACK plan"), "risk");
        assert_eq!(classify_workflow_route("DESIGN doc"), "architecture");
    }

    /// Every route must have a distinct, non-empty template; a missing one would
    /// silently degrade to the fallback and lose the section contract.
    #[test]
    fn every_route_has_a_distinct_template() {
        let routes = ["risk", "architecture", "release", "delivery"];
        let mut templates = Vec::new();
        for route in routes {
            let template = workflow_template(route);
            assert!(!template.is_empty(), "{route} has an empty template");
            assert!(
                template.contains("Template:"),
                "{route} template lacks a header: {template}"
            );
            templates.push(template);
        }
        let unique = templates.iter().collect::<std::collections::BTreeSet<_>>();
        assert_eq!(unique.len(), routes.len(), "templates are not distinct");
    }

    /// An unknown route must still produce a usable template rather than panic.
    #[test]
    fn an_unknown_route_still_yields_a_template() {
        let template = workflow_template("not-a-route");
        assert!(!template.is_empty());
    }
}
