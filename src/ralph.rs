//! ADK-Rust v2-native Ralph development loop.
//!
//! The previous integration pulled an ADK 0.5 runtime into the v2 process and
//! used the same model for planning and implementation. This loop reuses
//! Zavora's bounded planner, worker, session, safety, and streaming surfaces.

use anyhow::Result;

use crate::cli::{OutputFormat, RalphPhase};
use crate::config::RuntimeConfig;
use crate::retrieval::RetrievalService;

pub struct RalphRunOptions {
    pub phase: Option<RalphPhase>,
    pub resume: bool,
    pub output_dir: Option<String>,
    pub output_format: OutputFormat,
    pub always_approve: bool,
}

pub async fn run_ralph(
    cfg: &RuntimeConfig,
    prompt: String,
    options: RalphRunOptions,
    telemetry: &crate::telemetry::TelemetrySink,
    retrieval: &dyn RetrievalService,
) -> Result<()> {
    let RalphRunOptions {
        phase,
        resume,
        output_dir,
        output_format,
        always_approve,
    } = options;
    let runtime_tools = crate::runner::resolve_runtime_tools(cfg).await;
    // Requirement 13.5: a Ralph loop that begins without configured tools must
    // say so up front, not report success on a degraded run.
    let degraded = runtime_tools.connect_failure_report();
    if !degraded.is_empty() {
        eprintln!(
            "warning: ralph is starting with {} unreachable MCP server(s); their tools are unavailable for this run:",
            degraded.len()
        );
        for failure in &degraded {
            eprintln!("  - {failure}");
        }
    }
    if always_approve {
        for tool in runtime_tools.tools() {
            crate::tools::confirming::trust_tool(tool.name());
        }
    }
    let confirmation = crate::runner::resolve_tool_confirmation_settings(cfg, &runtime_tools);
    let session_service = crate::session::build_session_service(cfg).await?;
    let (runner, worker_provider, worker_model) = crate::runner::build_single_runner_for_chat(
        cfg,
        session_service,
        &runtime_tools,
        &confirmation,
        telemetry,
    )
    .await?;

    let phase_instruction = phase_instruction_for(phase);
    let resume_instruction = if resume {
        "Resume from the current session state and do not repeat completed work."
    } else {
        "Begin a new development loop for this request."
    };
    let output_instruction = output_dir
        .as_deref()
        .map(|path| format!("Keep generated work inside: {path}."))
        .unwrap_or_else(|| "Work in the current project directory.".to_string());
    let routed_prompt = build_ralph_prompt(
        &prompt,
        resume_instruction,
        phase_instruction,
        &output_instruction,
    );

    telemetry.emit(
        "ralph.started",
        serde_json::json!({
            "worker_provider": format!("{worker_provider:?}").to_ascii_lowercase(),
            "worker_model": worker_model,
            "planner_provider": format!("{:?}", cfg.planner_provider).to_ascii_lowercase(),
            "planner_model": cfg.planner_model,
            "phase": phase.map(|value| format!("{value:?}").to_ascii_lowercase()),
            "resume": resume,
        }),
    );

    let execution = if output_format == OutputFormat::Text {
        crate::streaming::run_prompt_streaming(&runner, cfg, &routed_prompt, telemetry)
            .await
            .map(|_| ())
    } else {
        crate::headless::run_headless(
            &runner,
            cfg,
            &routed_prompt,
            retrieval,
            telemetry,
            &crate::headless::RunMetadata {
                command: "ralph".to_string(),
                session_id: cfg.session_id.clone(),
                provider: format!("{worker_provider:?}").to_ascii_lowercase(),
                model: worker_model.clone(),
            },
            output_format,
        )
        .await
        .map(|_| ())
    };

    match execution {
        Ok(_) => {
            telemetry.emit("ralph.completed", serde_json::json!({"status": "ok"}));
            Ok(())
        }
        Err(error) => {
            telemetry.emit(
                "ralph.failed",
                serde_json::json!({"error": error.to_string()}),
            );
            Err(error)
        }
    }
}

/// Compose the routed Ralph prompt.
///
/// Extracted so the phase and resume routing can be tested without a provider.
/// Requirement 13.7.
fn build_ralph_prompt(
    prompt: &str,
    resume_instruction: &str,
    phase_instruction: &str,
    output_instruction: &str,
) -> String {
    format!(
        "Ralph development loop\n\n{resume_instruction}\n{phase_instruction}\n{output_instruction}\n\nRequest:\n{}",
        prompt.trim()
    )
}

/// The phase instruction for a Ralph run. `None` means the full loop.
fn phase_instruction_for(phase: Option<RalphPhase>) -> &'static str {
    match phase {
        Some(RalphPhase::Prd) => "Focus this turn on requirements and acceptance criteria.",
        Some(RalphPhase::Architect) => "Focus this turn on architecture and an executable plan.",
        Some(RalphPhase::Loop) => "Use the approved plan and perform the implementation loop.",
        None => "Call plan_work once, then execute and verify the plan to completion.",
    }
}

#[cfg(test)]
mod ralph_tests {
    use super::*;

    #[test]
    fn every_phase_has_a_distinct_instruction() {
        let instructions = [
            phase_instruction_for(Some(RalphPhase::Prd)),
            phase_instruction_for(Some(RalphPhase::Architect)),
            phase_instruction_for(Some(RalphPhase::Loop)),
            phase_instruction_for(None),
        ];
        let unique = instructions
            .iter()
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(
            unique.len(),
            instructions.len(),
            "two phases share an instruction: {instructions:?}"
        );
        assert!(instructions.iter().all(|text| !text.is_empty()));
    }

    /// Without an explicit phase, Ralph must bound itself by planning once
    /// rather than replanning every turn. Requirement 13.6.
    #[test]
    fn the_default_phase_bounds_planning_to_one_call() {
        let instruction = phase_instruction_for(None);
        assert!(
            instruction.contains("plan_work once"),
            "default phase must bound planning: {instruction}"
        );
    }

    #[test]
    fn the_routed_prompt_carries_every_instruction_and_the_request() {
        let routed = build_ralph_prompt(
            "  implement the accepted issue  ",
            "Begin a new development loop for this request.",
            phase_instruction_for(Some(RalphPhase::Loop)),
            "Work in the current project directory.",
        );

        assert!(routed.starts_with("Ralph development loop"));
        assert!(routed.contains("Begin a new development loop"));
        assert!(routed.contains("implementation loop"));
        assert!(routed.contains("Work in the current project directory."));
        // The request is trimmed and kept last so it is not lost in the preamble.
        assert!(routed.ends_with("Request:\nimplement the accepted issue"));
    }

    #[test]
    fn resume_and_fresh_runs_are_distinguishable() {
        let resumed = build_ralph_prompt(
            "x",
            "Resume from the current session state and do not repeat completed work.",
            phase_instruction_for(None),
            "Work in the current project directory.",
        );
        let fresh = build_ralph_prompt(
            "x",
            "Begin a new development loop for this request.",
            phase_instruction_for(None),
            "Work in the current project directory.",
        );
        assert_ne!(resumed, fresh);
        assert!(resumed.contains("do not repeat completed work"));
    }
}
