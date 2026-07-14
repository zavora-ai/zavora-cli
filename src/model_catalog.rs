//! Versioned model metadata used by routing and the terminal model picker.

use crate::cli::Provider;

pub const DEFAULT_WORKER_MODEL: &str = "gpt-5.4-mini-2026-03-17";
pub const DEFAULT_PLANNER_MODEL: &str = "gpt-5.6-sol";
pub const DEFAULT_UTILITY_MODEL: &str = "gpt-5.4-nano-2026-03-17";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelRole {
    Worker,
    Planner,
    Utility,
}

impl ModelRole {
    pub fn label(self) -> &'static str {
        match self {
            Self::Worker => "worker",
            Self::Planner => "planner",
            Self::Utility => "utility",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuotaPool {
    Premium1M,
    Throughput10M,
}

impl QuotaPool {
    pub fn label(self) -> &'static str {
        match self {
            Self::Premium1M => "1M shared daily pool",
            Self::Throughput10M => "10M shared daily pool",
        }
    }

    pub fn tier_one_two_allowance(self) -> &'static str {
        match self {
            Self::Premium1M => "250K on usage tiers 1–2",
            Self::Throughput10M => "2.5M on usage tiers 1–2",
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct ModelRecord {
    pub id: &'static str,
    pub pool: QuotaPool,
    pub recommended_role: ModelRole,
    pub description: &'static str,
    pub active: bool,
}

const PREMIUM_MODELS: &[(&str, &str)] = &[
    ("gpt-5.6-sol", "strongest planning default"),
    ("gpt-5.5-2026-04-23", "high-capability general reasoning"),
    ("gpt-5.4-2026-03-05", "high-capability planning and review"),
    ("gpt-5.2-2025-12-11", "strong general reasoning"),
    ("gpt-5.1-2025-11-13", "strong general reasoning"),
    ("gpt-5.1-codex", "high-capability coding"),
    ("gpt-5-codex", "agentic coding"),
    ("gpt-5-2025-08-07", "general GPT-5 snapshot"),
    ("gpt-5-chat-latest", "conversational GPT-5"),
    ("gpt-4.1-2025-04-14", "reliable long-context work"),
    ("gpt-4o-2024-05-13", "multimodal GPT-4o snapshot"),
    ("gpt-4o-2024-08-06", "multimodal GPT-4o snapshot"),
    ("gpt-4o-2024-11-20", "multimodal GPT-4o snapshot"),
    ("o3-2025-04-16", "deep reasoning"),
    ("o1-preview-2024-09-12", "legacy reasoning preview"),
    ("o1-2024-12-17", "reasoning snapshot"),
];

const THROUGHPUT_MODELS: &[(&str, &str, ModelRole)] = &[
    (
        "gpt-5.6-terra",
        "high-throughput capable model",
        ModelRole::Worker,
    ),
    (
        "gpt-5.6-luna",
        "high-throughput balanced model",
        ModelRole::Worker,
    ),
    (
        "gpt-5.4-mini-2026-03-17",
        "recommended coding worker",
        ModelRole::Worker,
    ),
    (
        "gpt-5.4-nano-2026-03-17",
        "fast scans and summaries",
        ModelRole::Utility,
    ),
    (
        "gpt-5.1-codex-mini",
        "efficient coding worker",
        ModelRole::Worker,
    ),
    (
        "gpt-5-mini-2025-08-07",
        "efficient general worker",
        ModelRole::Worker,
    ),
    (
        "gpt-5-nano-2025-08-07",
        "low-cost utility work",
        ModelRole::Utility,
    ),
    (
        "gpt-4.1-mini-2025-04-14",
        "efficient long-context work",
        ModelRole::Worker,
    ),
    (
        "gpt-4.1-nano-2025-04-14",
        "fast classification and extraction",
        ModelRole::Utility,
    ),
    (
        "gpt-4o-mini-2024-07-18",
        "fast multimodal work",
        ModelRole::Worker,
    ),
    (
        "o4-mini-2025-04-16",
        "efficient reasoning",
        ModelRole::Worker,
    ),
    (
        "o1-mini-2024-09-12",
        "legacy efficient reasoning",
        ModelRole::Worker,
    ),
    (
        "codex-mini-latest",
        "efficient agentic coding",
        ModelRole::Worker,
    ),
];

pub fn openai_models() -> Vec<ModelRecord> {
    let mut models = PREMIUM_MODELS
        .iter()
        .map(|(id, description)| ModelRecord {
            id,
            pool: QuotaPool::Premium1M,
            recommended_role: ModelRole::Planner,
            description,
            active: true,
        })
        .collect::<Vec<_>>();
    models.push(ModelRecord {
        id: "gpt-4.5-preview-2025-02-27",
        pool: QuotaPool::Premium1M,
        recommended_role: ModelRole::Planner,
        description: "retired on 2025-07-14",
        active: false,
    });
    models.extend(
        THROUGHPUT_MODELS
            .iter()
            .map(|(id, description, role)| ModelRecord {
                id,
                pool: QuotaPool::Throughput10M,
                recommended_role: *role,
                description,
                active: true,
            }),
    );
    models
}

pub fn models_for_provider(provider: Provider) -> Vec<ModelRecord> {
    match provider {
        Provider::Openai => openai_models()
            .into_iter()
            .filter(|model| model.active)
            .collect(),
        _ => Vec::new(),
    }
}

pub fn model_record(id: &str) -> Option<ModelRecord> {
    openai_models().into_iter().find(|model| model.id == id)
}

pub fn default_model(provider: Provider, role: ModelRole) -> &'static str {
    match (provider, role) {
        (Provider::Openai, ModelRole::Planner) => DEFAULT_PLANNER_MODEL,
        (Provider::Openai, ModelRole::Utility) => DEFAULT_UTILITY_MODEL,
        (Provider::Openai, ModelRole::Worker) => DEFAULT_WORKER_MODEL,
        (Provider::Gemini, ModelRole::Planner) => "gemini-2.5-pro",
        (Provider::Gemini, _) => "gemini-2.5-flash",
        (Provider::Anthropic, ModelRole::Planner) => "claude-opus-4-6",
        (Provider::Anthropic, _) => "claude-sonnet-4-20250514",
        (Provider::Deepseek, ModelRole::Planner) => "deepseek-reasoner",
        (Provider::Deepseek, _) => "deepseek-chat",
        (Provider::Groq, _) => "llama-3.3-70b-versatile",
        (Provider::Ollama, ModelRole::Planner) => "qwen2.5-coder",
        (Provider::Ollama, _) => "llama4",
        (Provider::Auto, role) => default_model(Provider::Openai, role),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn openai_defaults_keep_routine_work_in_the_larger_pool() {
        assert_eq!(
            model_record(DEFAULT_WORKER_MODEL).map(|model| model.pool),
            Some(QuotaPool::Throughput10M)
        );
        assert_eq!(
            model_record(DEFAULT_PLANNER_MODEL).map(|model| model.pool),
            Some(QuotaPool::Premium1M)
        );
    }

    #[test]
    fn retired_models_are_documented_but_not_selectable() {
        let retired = "gpt-4.5-preview-2025-02-27";
        assert_eq!(model_record(retired).map(|model| model.active), Some(false));
        assert!(
            models_for_provider(Provider::Openai)
                .iter()
                .all(|model| model.id != retired)
        );
    }
}
