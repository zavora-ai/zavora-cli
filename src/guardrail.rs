use std::collections::BTreeSet;

use anyhow::Result;
use serde_json::json;

use crate::cli::GuardrailMode;
use crate::config::RuntimeConfig;
use crate::telemetry::TelemetrySink;

pub const DEFAULT_GUARDRAIL_TERMS: &[&str] = &[
    "password",
    "secret",
    "api key",
    "api_key",
    "private key",
    "access token",
    "ssn",
    "social security",
];

pub fn default_guardrail_terms() -> Vec<String> {
    DEFAULT_GUARDRAIL_TERMS
        .iter()
        .map(|term| term.to_string())
        .collect::<Vec<String>>()
}

pub fn guardrail_mode_label(mode: GuardrailMode) -> &'static str {
    match mode {
        GuardrailMode::Disabled => "disabled",
        GuardrailMode::Observe => "observe",
        GuardrailMode::Block => "block",
        GuardrailMode::Redact => "redact",
    }
}

pub fn contains_guardrail_terms(text: &str, terms: &[String]) -> Vec<String> {
    let mut hits = BTreeSet::<String>::new();
    let lower = text.to_ascii_lowercase();
    for term in terms {
        let normalized = term.trim().to_ascii_lowercase();
        if normalized.is_empty() {
            continue;
        }
        if lower.contains(&normalized) {
            hits.insert(normalized);
        }
    }
    hits.into_iter().collect::<Vec<String>>()
}

pub fn replace_case_insensitive(input: &str, needle: &str, replacement: &str) -> String {
    if needle.is_empty() {
        return input.to_string();
    }

    let input_lower = input.to_ascii_lowercase();
    let needle_lower = needle.to_ascii_lowercase();
    let mut out = String::new();
    let mut last_idx = 0usize;
    let mut search_idx = 0usize;

    while let Some(relative) = input_lower[search_idx..].find(&needle_lower) {
        let start = search_idx + relative;
        let end = start + needle.len();
        out.push_str(&input[last_idx..start]);
        out.push_str(replacement);
        last_idx = end;
        search_idx = end;
    }

    out.push_str(&input[last_idx..]);
    out
}

pub fn redact_guardrail_terms(text: &str, hits: &[String], replacement: &str) -> String {
    let mut redacted = text.to_string();
    for hit in hits {
        redacted = replace_case_insensitive(&redacted, hit, replacement);
    }
    redacted
}

pub fn apply_guardrail(
    cfg: &RuntimeConfig,
    telemetry: &TelemetrySink,
    direction: &str,
    mode: GuardrailMode,
    text: &str,
) -> Result<String> {
    if matches!(mode, GuardrailMode::Disabled) {
        return Ok(text.to_string());
    }

    let hits = contains_guardrail_terms(text, &cfg.guardrail_terms);
    if hits.is_empty() {
        return Ok(text.to_string());
    }

    let mode_label = guardrail_mode_label(mode);
    let hit_count = hits.len();
    let telemetry_payload = json!({
        "direction": direction,
        "mode": mode_label,
        "hits": hits.clone(),
        "hit_count": hit_count
    });

    match mode {
        GuardrailMode::Observe => {
            tracing::warn!(
                direction = direction,
                mode = mode_label,
                hit_count = hit_count,
                "Guardrail observed content matches"
            );
            telemetry.emit(
                &format!("guardrail.{direction}.observed"),
                telemetry_payload,
            );
            Ok(text.to_string())
        }
        GuardrailMode::Block => {
            tracing::warn!(
                direction = direction,
                mode = mode_label,
                hit_count = hit_count,
                "Guardrail blocked content"
            );
            telemetry.emit(&format!("guardrail.{direction}.blocked"), telemetry_payload);
            Err(anyhow::anyhow!(
                "guardrail blocked {} content due to matched terms",
                direction
            ))
        }
        GuardrailMode::Redact => {
            let redacted = redact_guardrail_terms(text, &hits, &cfg.guardrail_redact_replacement);
            tracing::warn!(
                direction = direction,
                mode = mode_label,
                hit_count = hit_count,
                "Guardrail redacted content"
            );
            telemetry.emit(
                &format!("guardrail.{direction}.redacted"),
                telemetry_payload,
            );
            Ok(redacted)
        }
        GuardrailMode::Disabled => Ok(text.to_string()),
    }
}

pub fn buffered_output_required(mode: GuardrailMode) -> bool {
    matches!(mode, GuardrailMode::Block | GuardrailMode::Redact)
}

pub fn enforce_prompt_limit(prompt: &str, max_chars: usize) -> Result<()> {
    if max_chars > 0 && prompt.len() > max_chars {
        return Err(anyhow::anyhow!(
            "prompt exceeds maximum length ({} chars > {} limit). Shorten the prompt or increase max_prompt_chars.",
            prompt.len(),
            max_chars
        ));
    }
    Ok(())
}
