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
    DEFAULT_GUARDRAIL_TERMS.iter().map(|t| t.to_string()).collect()
}

pub fn guardrail_mode_label(mode: GuardrailMode) -> &'static str {
    match mode {
        GuardrailMode::Disabled => "disabled",
        GuardrailMode::Observe => "observe",
        GuardrailMode::Block => "block",
        GuardrailMode::Redact => "redact",
    }
}

/// Check text for guardrail term matches using adk-guardrail ContentFilter.
pub fn contains_guardrail_terms(text: &str, terms: &[String]) -> Vec<String> {
    let filter = adk_guardrail::ContentFilter::blocked_keywords(terms.to_vec());
    let content = adk_rust::Content::new("user").with_text(text);
    // Run synchronously — ContentFilter::validate is CPU-only (regex)
    let rt = tokio::runtime::Handle::try_current();
    let result = if let Ok(handle) = rt {
        handle.block_on(adk_guardrail::Guardrail::validate(&filter, &content))
    } else {
        // Fallback for non-async context
        let rt = tokio::runtime::Builder::new_current_thread().build().unwrap();
        rt.block_on(adk_guardrail::Guardrail::validate(&filter, &content))
    };
    match result {
        adk_guardrail::GuardrailResult::Fail { .. } => {
            // Extract matched terms from the reason
            terms
                .iter()
                .filter(|t| text.to_ascii_lowercase().contains(&t.to_ascii_lowercase()))
                .cloned()
                .collect()
        }
        _ => Vec::new(),
    }
}

/// Redact PII using adk-guardrail PiiRedactor + custom terms.
pub fn redact_text(text: &str, terms: &[String], replacement: &str) -> String {
    // First: PII redaction (emails, phones, SSNs, credit cards)
    let pii = adk_guardrail::PiiRedactor::new();
    let (redacted, _) = pii.redact(text);

    // Then: custom term redaction
    let mut result = redacted;
    for term in terms {
        if term.is_empty() { continue; }
        result = replace_case_insensitive(&result, term, replacement);
    }
    result
}

fn replace_case_insensitive(input: &str, needle: &str, replacement: &str) -> String {
    if needle.is_empty() { return input.to_string(); }
    let input_lower = input.to_ascii_lowercase();
    let needle_lower = needle.to_ascii_lowercase();
    let mut out = String::new();
    let mut last = 0;
    let mut search = 0;
    while let Some(rel) = input_lower[search..].find(&needle_lower) {
        let start = search + rel;
        let end = start + needle.len();
        out.push_str(&input[last..start]);
        out.push_str(replacement);
        last = end;
        search = end;
    }
    out.push_str(&input[last..]);
    out
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
    let payload = json!({"direction": direction, "mode": mode_label, "hits": &hits, "hit_count": hits.len()});

    match mode {
        GuardrailMode::Observe => {
            tracing::warn!(direction, mode = mode_label, hit_count = hits.len(), "Guardrail observed");
            telemetry.emit(&format!("guardrail.{direction}.observed"), payload);
            Ok(text.to_string())
        }
        GuardrailMode::Block => {
            tracing::warn!(direction, mode = mode_label, hit_count = hits.len(), "Guardrail blocked");
            telemetry.emit(&format!("guardrail.{direction}.blocked"), payload);
            Err(anyhow::anyhow!("guardrail blocked {direction} content due to matched terms"))
        }
        GuardrailMode::Redact => {
            let redacted = redact_text(text, &hits, &cfg.guardrail_redact_replacement);
            tracing::warn!(direction, mode = mode_label, hit_count = hits.len(), "Guardrail redacted");
            telemetry.emit(&format!("guardrail.{direction}.redacted"), payload);
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
            "prompt exceeds maximum length ({} chars > {} limit)",
            prompt.len(), max_chars
        ));
    }
    Ok(())
}
