use anyhow::{Context, Result, anyhow};

use crate::cli::{Provider, RalphPhase};
use crate::config::RuntimeConfig;

use adk_ralph::{
    AgentModelConfig, ModelConfig, PipelinePhase, RalphConfig, TelemetryConfig,
};

/// Maps a zavora Provider enum to a Ralph provider string.
pub fn map_provider(provider: Provider) -> Result<String> {
    match provider {
        Provider::Openai => Ok("openai".to_string()),
        Provider::Anthropic => Ok("anthropic".to_string()),
        Provider::Gemini => Ok("gemini".to_string()),
        Provider::Ollama => Ok("ollama".to_string()),
        Provider::Deepseek => Err(anyhow!(
            "provider 'deepseek' is not supported by Ralph. Supported: openai, anthropic, gemini, ollama"
        )),
        Provider::Groq => Err(anyhow!(
            "provider 'groq' is not supported by Ralph. Supported: openai, anthropic, gemini, ollama"
        )),
        Provider::Auto => Err(anyhow!(
            "auto provider must be resolved before ralph invocation"
        )),
    }
}

/// Resolves the API key from the runtime config, falling back to environment variables.
fn resolve_api_key(cfg: &RuntimeConfig) -> Result<String> {
    if let Some(ref key) = cfg.api_key {
        return Ok(key.clone());
    }
    let env_key = match cfg.provider {
        Provider::Openai => std::env::var("OPENAI_API_KEY").ok(),
        Provider::Anthropic => std::env::var("ANTHROPIC_API_KEY").ok(),
        Provider::Gemini => std::env::var("GEMINI_API_KEY")
            .or_else(|_| std::env::var("GOOGLE_API_KEY"))
            .ok(),
        Provider::Ollama => Some("ollama".to_string()), // Ollama doesn't need a key
        _ => None,
    };
    env_key.context("API key required for Ralph pipeline. Set it in your profile or via environment variable.")
}

/// Maps a zavora RalphPhase to Ralph's internal PipelinePhase.
pub fn map_ralph_phase(phase: RalphPhase) -> PipelinePhase {
    match phase {
        RalphPhase::Prd => PipelinePhase::Requirements,
        RalphPhase::Architect => PipelinePhase::Design,
        RalphPhase::Loop => PipelinePhase::Implementation,
    }
}
/// Returns a sensible default model name for the given provider, matching
/// the defaults used by `resolve_model` in `src/provider.rs`.
fn default_model_for_provider(provider: Provider) -> String {
    match provider {
        Provider::Openai => "gpt-5-mini".to_string(),
        Provider::Anthropic => "claude-sonnet-4-20250514".to_string(),
        Provider::Gemini => "gemini-2.5-flash".to_string(),
        Provider::Ollama => "llama4".to_string(),
        _ => "default".to_string(),
    }
}

/// Configuration bridge that translates zavora's RuntimeConfig into Ralph's RalphConfig.
pub struct RalphConfigBridge;

impl RalphConfigBridge {
    pub fn from_runtime_config(
        cfg: &RuntimeConfig,
        output_dir: Option<&str>,
    ) -> Result<RalphConfig> {
        let provider = map_provider(cfg.provider)?;
        let api_key = resolve_api_key(cfg)?;
        let model_name = cfg.model.clone().unwrap_or_else(|| default_model_for_provider(cfg.provider));

        // Set env vars so Ralph's internal model creation picks them up
        // SAFETY: This is called before spawning any threads for the Ralph pipeline
        unsafe {
            match cfg.provider {
                Provider::Openai => std::env::set_var("OPENAI_API_KEY", &api_key),
                Provider::Anthropic => std::env::set_var("ANTHROPIC_API_KEY", &api_key),
                Provider::Gemini => std::env::set_var("GEMINI_API_KEY", &api_key),
                _ => {}
            }
        }

        let model_config = ModelConfig::new(&provider, &model_name);
        let agents = AgentModelConfig {
            prd_model: model_config.clone(),
            architect_model: model_config.clone(),
            ralph_model: model_config,
        };

        let project_path = output_dir.unwrap_or(".").to_string();

        Ok(RalphConfig {
            agents,
            telemetry: TelemetryConfig::default(),
            debug_level: adk_ralph::DebugLevel::Normal,
            max_iterations: 50,
            prd_path: "prd.md".to_string(),
            design_path: "design.md".to_string(),
            tasks_path: "tasks.json".to_string(),
            progress_path: "progress.json".to_string(),
            project_path,
            completion_promise: "All tasks completed successfully!".to_string(),
            max_task_retries: 3,
        })
    }
}

/// Run the Ralph autonomous development pipeline.
pub async fn run_ralph(
    cfg: &RuntimeConfig,
    prompt: String,
    phase: Option<RalphPhase>,
    resume: bool,
    output_dir: Option<String>,
    telemetry: &crate::telemetry::TelemetrySink,
) -> Result<()> {
    let ralph_config = RalphConfigBridge::from_runtime_config(cfg, output_dir.as_deref())?;

    let mut orchestrator = adk_ralph::RalphOrchestrator::new(ralph_config)
        .map_err(|e| anyhow!("Failed to create Ralph orchestrator: {}", e))?;

    telemetry.emit(
        "ralph.started",
        serde_json::json!({
            "provider": format!("{:?}", cfg.provider).to_ascii_lowercase(),
            "model": cfg.model.clone().unwrap_or_default(),
            "phase": phase.map(|p| format!("{:?}", p).to_ascii_lowercase()),
            "resume": resume,
        }),
    );

    let result = if resume {
        println!("Resuming Ralph pipeline...");
        orchestrator
            .resume(&prompt)
            .await
            .map_err(|e| anyhow!("{}", e))
    } else if let Some(phase) = phase {
        let ralph_phase = map_ralph_phase(phase);
        println!("Skipping to {:?} phase...", ralph_phase);
        orchestrator
            .skip_to_phase(ralph_phase)
            .map_err(|e| anyhow!("{}", e))?;
        orchestrator
            .run(&prompt)
            .await
            .map_err(|e| anyhow!("{}", e))
    } else {
        println!("Starting Ralph pipeline...");
        orchestrator
            .run(&prompt)
            .await
            .map_err(|e| anyhow!("{}", e))
    };

    match &result {
        Ok(status) => {
            telemetry.emit("ralph.completed", serde_json::json!({"status": "ok"}));
            println!("Ralph pipeline completed: {:?}", status);
        }
        Err(e) => {
            telemetry.emit(
                "ralph.failed",
                serde_json::json!({"error": e.to_string()}),
            );
        }
    }

    result.map(|_| ())
}
