use clap::ValueEnum;
use std::sync::Arc;

use adk_rust::prelude::*;
use anyhow::{Context, Result};

use crate::cli::Provider;
use crate::config::RuntimeConfig;

pub fn validate_model_for_provider(provider: Provider, model_name: &str) -> Result<()> {
    let is_valid = match provider {
        Provider::Gemini => model_name.starts_with("gemini"),
        Provider::Openai => {
            model_name.starts_with("gpt-")
                || model_name.starts_with("o1")
                || model_name.starts_with("o3")
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
    let provider = match cfg.provider {
        Provider::Auto => detect_provider().context(
            "no provider could be auto-detected. Set one of GOOGLE_API_KEY, OPENAI_API_KEY, \
             ANTHROPIC_API_KEY, DEEPSEEK_API_KEY, GROQ_API_KEY, or use --provider ollama",
        )?,
        p => p,
    };

    match provider {
        Provider::Gemini => {
            let api_key = std::env::var("GOOGLE_API_KEY")
                .context("GOOGLE_API_KEY is required for Gemini provider")?;
            let model_name = cfg
                .model
                .clone()
                .unwrap_or_else(|| "gemini-2.5-flash".to_string());
            validate_model_for_provider(provider, &model_name)?;
            let model = GeminiModel::new(api_key, model_name.clone())?;
            Ok((Arc::new(model), provider, model_name))
        }
        Provider::Openai => {
            let api_key = std::env::var("OPENAI_API_KEY")
                .context("OPENAI_API_KEY is required for OpenAI provider")?;
            let model_name = cfg
                .model
                .clone()
                .unwrap_or_else(|| "gpt-5-mini".to_string());
            validate_model_for_provider(provider, &model_name)?;
            let model = OpenAIClient::new(OpenAIConfig::new(api_key, model_name.clone()))?;
            Ok((Arc::new(model), provider, model_name))
        }
        Provider::Anthropic => {
            let api_key = std::env::var("ANTHROPIC_API_KEY")
                .context("ANTHROPIC_API_KEY is required for Anthropic provider")?;
            let model_name = cfg
                .model
                .clone()
                .unwrap_or_else(|| "claude-sonnet-4-20250514".to_string());
            validate_model_for_provider(provider, &model_name)?;
            let model = AnthropicClient::new(AnthropicConfig::new(api_key, model_name.clone()))?;
            Ok((Arc::new(model), provider, model_name))
        }
        Provider::Deepseek => {
            let api_key = std::env::var("DEEPSEEK_API_KEY")
                .context("DEEPSEEK_API_KEY is required for DeepSeek provider")?;
            let model_name = cfg
                .model
                .clone()
                .unwrap_or_else(|| "deepseek-chat".to_string());
            validate_model_for_provider(provider, &model_name)?;
            let model = DeepSeekClient::new(DeepSeekConfig::new(api_key, model_name.clone()))?;
            Ok((Arc::new(model), provider, model_name))
        }
        Provider::Groq => {
            let api_key = std::env::var("GROQ_API_KEY")
                .context("GROQ_API_KEY is required for Groq provider")?;
            let model_name = cfg
                .model
                .clone()
                .unwrap_or_else(|| "llama-3.3-70b-versatile".to_string());
            validate_model_for_provider(provider, &model_name)?;
            let model = GroqClient::new(GroqConfig::new(api_key, model_name.clone()))?;
            Ok((Arc::new(model), provider, model_name))
        }
        Provider::Ollama => {
            let host = std::env::var("OLLAMA_HOST")
                .unwrap_or_else(|_| "http://localhost:11434".to_string());
            let model_name = cfg.model.clone().unwrap_or_else(|| "llama4".to_string());
            validate_model_for_provider(provider, &model_name)?;
            let model = OllamaModel::new(OllamaConfig::with_host(host, model_name.clone()))?;
            Ok((Arc::new(model), provider, model_name))
        }
        Provider::Auto => unreachable!("auto provider must be resolved before matching"),
    }
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
