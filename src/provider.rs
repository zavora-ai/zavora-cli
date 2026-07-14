use clap::ValueEnum;
use std::sync::Arc;

use adk_rust::prelude::*;
use anyhow::{Context, Result};

use crate::cli::Provider;
use crate::config::RuntimeConfig;
use crate::model_catalog::{ModelRole, default_model};

/// Auto-detect provider using adk-rust's built-in `provider_from_env()`.
/// Falls back to our manual detection if the ADK helper isn't available for the
/// configured provider set.
pub fn auto_detect_provider_from_env() -> Option<Arc<dyn Llm>> {
    adk_rust::provider_from_env().ok()
}

pub fn validate_model_for_provider(provider: Provider, model_name: &str) -> Result<()> {
    let is_valid = match provider {
        Provider::Gemini => model_name.starts_with("gemini"),
        Provider::Openai => {
            let known_retired =
                crate::model_catalog::model_record(model_name).is_some_and(|model| !model.active);
            !known_retired
                && (model_name.starts_with("gpt-")
                    || model_name.starts_with("o1")
                    || model_name.starts_with("o3")
                    || model_name.starts_with("o4")
                    || model_name.starts_with("codex-"))
        }
        Provider::Anthropic => model_name.starts_with("claude"),
        Provider::Deepseek => model_name.starts_with("deepseek"),
        Provider::Groq => !model_name.trim().is_empty(),
        Provider::Ollama => !model_name.trim().is_empty(),
        Provider::Auto => true,
    };

    if is_valid {
        return Ok(());
    }

    Err(anyhow::anyhow!(
        "model '{}' is not compatible with provider '{:?}'",
        model_name,
        provider
    ))
}

pub fn resolve_model(cfg: &RuntimeConfig) -> Result<(Arc<dyn Llm>, Provider, String)> {
    resolve_model_route(
        cfg,
        cfg.worker_provider,
        &cfg.worker_model,
        ModelRole::Worker,
    )
}

pub fn resolve_planner_model(cfg: &RuntimeConfig) -> Result<(Arc<dyn Llm>, Provider, String)> {
    resolve_model_route(
        cfg,
        cfg.planner_provider,
        &cfg.planner_model,
        ModelRole::Planner,
    )
}

fn resolve_model_route(
    cfg: &RuntimeConfig,
    configured_provider: Provider,
    configured_model: &str,
    role: ModelRole,
) -> Result<(Arc<dyn Llm>, Provider, String)> {
    let provider = match configured_provider {
        Provider::Auto => detect_provider().context(
            "no provider could be auto-detected. Run 'zavora-cli setup' or set one of \
             GOOGLE_API_KEY, OPENAI_API_KEY, ANTHROPIC_API_KEY, DEEPSEEK_API_KEY, GROQ_API_KEY, \
             or use --provider ollama",
        )?,
        p => p,
    };
    let model_name = if configured_model.trim().is_empty() {
        default_model(provider, role).to_string()
    } else {
        configured_model.to_string()
    };

    match provider {
        Provider::Gemini => {
            let api_key = resolve_api_key(cfg, provider, "GOOGLE_API_KEY")?;
            validate_model_for_provider(provider, &model_name)?;
            let model = GeminiModel::new(api_key, model_name.clone())?;
            Ok((Arc::new(model), provider, model_name))
        }
        Provider::Openai => {
            let api_key = resolve_api_key(cfg, provider, "OPENAI_API_KEY")?;
            validate_model_for_provider(provider, &model_name)?;
            let model = adk_rust::model::openai::OpenAIResponsesClient::new(
                adk_rust::model::openai::OpenAIResponsesConfig::new(api_key, model_name.clone()),
            )?;
            Ok((Arc::new(model), provider, model_name))
        }
        Provider::Anthropic => {
            let api_key = resolve_api_key(cfg, provider, "ANTHROPIC_API_KEY")?;
            validate_model_for_provider(provider, &model_name)?;
            let model = AnthropicClient::new(AnthropicConfig::new(api_key, model_name.clone()))?;
            Ok((Arc::new(model), provider, model_name))
        }
        Provider::Deepseek => {
            let api_key = resolve_api_key(cfg, provider, "DEEPSEEK_API_KEY")?;
            validate_model_for_provider(provider, &model_name)?;
            let model = DeepSeekClient::new(DeepSeekConfig::new(api_key, model_name.clone()))?;
            Ok((Arc::new(model), provider, model_name))
        }
        Provider::Groq => {
            let api_key = resolve_api_key(cfg, provider, "GROQ_API_KEY")?;
            validate_model_for_provider(provider, &model_name)?;
            let model = GroqClient::new(GroqConfig::new(api_key, model_name.clone()))?;
            Ok((Arc::new(model), provider, model_name))
        }
        Provider::Ollama => {
            let host = std::env::var("OLLAMA_HOST")
                .ok()
                .or_else(|| cfg.ollama_host.clone())
                .unwrap_or_else(|| "http://localhost:11434".to_string());
            validate_model_for_provider(provider, &model_name)?;
            let model = OllamaModel::new(OllamaConfig::with_host(host, model_name.clone()))?;
            Ok((Arc::new(model), provider, model_name))
        }
        Provider::Auto => unreachable!("auto provider must be resolved before matching"),
    }
}

fn resolve_api_key(cfg: &RuntimeConfig, provider: Provider, env_name: &str) -> Result<String> {
    std::env::var(env_name)
        .ok()
        .filter(|key| !key.trim().is_empty())
        .or_else(|| crate::credentials::load_api_key(provider))
        // Backward-compatible read for configurations created before v2. New
        // setup runs never persist this field.
        .or_else(|| cfg.api_key.clone().filter(|key| !key.trim().is_empty()))
        .with_context(|| {
            format!(
                "{env_name} is required for {provider:?}. Run 'zavora-cli setup' to store it securely, or set the environment variable."
            )
        })
}

pub fn detect_provider() -> Option<Provider> {
    if env_present("OPENAI_API_KEY") {
        return Some(Provider::Openai);
    }
    if env_present("ANTHROPIC_API_KEY") {
        return Some(Provider::Anthropic);
    }
    if env_present("DEEPSEEK_API_KEY") {
        return Some(Provider::Deepseek);
    }
    if env_present("GROQ_API_KEY") {
        return Some(Provider::Groq);
    }
    if env_present("GOOGLE_API_KEY") {
        return Some(Provider::Gemini);
    }
    if env_present("OLLAMA_HOST") {
        return Some(Provider::Ollama);
    }
    None
}

pub fn env_present(key: &str) -> bool {
    std::env::var(key)
        .map(|v| !v.trim().is_empty())
        .unwrap_or(false)
}

pub fn parse_provider_name(value: &str) -> Result<Provider> {
    Provider::from_str(value, true)
        .map_err(|_| {
            anyhow::anyhow!(
                "invalid provider '{}'. Supported values: auto, gemini, openai, anthropic, deepseek, groq, ollama",
                value
            )
        })
}
