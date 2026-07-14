//! ADK-Rust v2-native Ralph development loop.
//!
//! The previous integration pulled an ADK 0.5 runtime into the v2 process and
//! used the same model for planning and implementation. This loop reuses
//! Zavora's bounded planner, worker, session, safety, and streaming surfaces.

use anyhow::Result;

use crate::cli::RalphPhase;
use crate::config::RuntimeConfig;

pub async fn run_ralph(
    cfg: &RuntimeConfig,
    prompt: String,
    phase: Option<RalphPhase>,
    resume: bool,
    output_dir: Option<String>,
    telemetry: &crate::telemetry::TelemetrySink,
) -> Result<()> {
    let runtime_tools = crate::runner::resolve_runtime_tools(cfg).await;
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

    let phase_instruction = match phase {
        Some(RalphPhase::Prd) => "Focus this turn on requirements and acceptance criteria.",
        Some(RalphPhase::Architect) => "Focus this turn on architecture and an executable plan.",
        Some(RalphPhase::Loop) => "Use the approved plan and perform the implementation loop.",
        None => "Call plan_work once, then execute and verify the plan to completion.",
    };
    let resume_instruction = if resume {
        "Resume from the current session state and do not repeat completed work."
    } else {
        "Begin a new development loop for this request."
    };
    let output_instruction = output_dir
        .as_deref()
        .map(|path| format!("Keep generated work inside: {path}."))
        .unwrap_or_else(|| "Work in the current project directory.".to_string());
    let routed_prompt = format!(
        "Ralph development loop\n\n{resume_instruction}\n{phase_instruction}\n{output_instruction}\n\nRequest:\n{}",
        prompt.trim()
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

    match crate::streaming::run_prompt_streaming(&runner, cfg, &routed_prompt, telemetry).await {
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
