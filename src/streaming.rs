use std::collections::HashMap;
use std::io::{self, Write};

use adk_rust::prelude::*;
use adk_rust::futures::StreamExt;
use anyhow::{Context, Result};
use serde_json::Value;

use crate::config::RuntimeConfig;
use crate::retrieval::{RetrievalPolicy, RetrievalService, augment_prompt_with_retrieval};
use crate::telemetry::TelemetrySink;
use crate::theme::Spinner;
use crate::markdown::{ParseState, parse_markdown};

pub const NO_TEXTUAL_RESPONSE: &str = "No textual response produced by the agent.";

#[derive(Default, Debug)]
pub struct AuthorTextTracker {
    pub latest_final_text: Option<String>,
    pub latest_final_author: Option<String>,
    pub last_textful_author: Option<String>,
    pub by_author: HashMap<String, String>,
}

impl AuthorTextTracker {
    pub fn ingest_parts(&mut self, author: &str, text: &str, partial: bool, is_final: bool) -> String {
        if text.is_empty() {
            return String::new();
        }

        self.last_textful_author = Some(author.to_string());
        let buffer = self.by_author.entry(author.to_string()).or_default();
        let delta = ingest_author_text(buffer, text, partial, is_final);

        if is_final && !text.trim().is_empty() {
            self.latest_final_text = Some(text.to_string());
            self.latest_final_author = Some(author.to_string());
        }

        delta
    }

    pub fn resolve_text(&self) -> Option<String> {
        if let Some(final_text) = &self.latest_final_text {
            return Some(final_text.clone());
        }

        let author = self.last_textful_author.as_ref()?;
        let text = self.by_author.get(author)?;
        let trimmed = text.trim();
        if trimmed.is_empty() {
            return None;
        }

        Some(trimmed.to_string())
    }
}

pub fn ingest_author_text(buffer: &mut String, text: &str, partial: bool, is_final: bool) -> String {
    if text.is_empty() {
        return String::new();
    }

    if partial {
        buffer.push_str(text);
        return text.to_string();
    }

    if buffer.is_empty() {
        buffer.push_str(text);
        return text.to_string();
    }

    if text == buffer.as_str() {
        return String::new();
    }

    if text.starts_with(buffer.as_str()) {
        let delta = text[buffer.len()..].to_string();
        *buffer = text.to_string();
        return delta;
    }

    // Final snapshots are authoritative. Keep them as state but do not re-print
    // to avoid duplication after partial streaming has already emitted text.
    if is_final {
        *buffer = text.to_string();
        return String::new();
    }

    let overlap = suffix_prefix_overlap(buffer, text);
    if overlap >= text.len() {
        return String::new();
    }

    let delta = text[overlap..].to_string();
    buffer.push_str(&delta);
    delta
}

pub fn suffix_prefix_overlap(existing: &str, incoming: &str) -> usize {
    let max_len = existing.len().min(incoming.len());
    let mut boundaries = incoming
        .char_indices()
        .map(|(idx, _)| idx)
        .collect::<Vec<usize>>();
    boundaries.push(incoming.len());

    for boundary in boundaries.into_iter().rev() {
        if boundary == 0 || boundary > max_len {
            continue;
        }
        if existing.ends_with(&incoming[..boundary]) {
            return boundary;
        }
    }

    0
}

pub fn final_stream_suffix(emitted: &str, final_text: &str) -> Option<String> {
    if final_text.trim().is_empty() {
        return None;
    }

    if emitted.is_empty() {
        return Some(final_text.to_string());
    }

    if final_text == emitted || final_text.trim() == emitted.trim() {
        return None;
    }

    if let Some(suffix) = final_text.strip_prefix(emitted) {
        if suffix.is_empty() {
            return None;
        }
        return Some(suffix.to_string());
    }

    Some(format!("\n{final_text}"))
}

pub fn event_text(event: &Event) -> String {
    match event.content() {
        Some(content) => content
            .parts
            .iter()
            .filter_map(|part| match part {
                Part::Text { text } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join(""),
        None => String::new(),
    }
}

pub fn extract_tool_failure_message(response: &Value) -> Option<String> {
    if let Some(message) = response.get("error").and_then(Value::as_str) {
        return Some(message.to_string());
    }
    if let Some(message) = response.get("message").and_then(Value::as_str) {
        let status = response
            .get("status")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if status.eq_ignore_ascii_case("error") || status.eq_ignore_ascii_case("failed") {
            return Some(message.to_string());
        }
    }
    None
}

pub fn emit_tool_lifecycle_events(event: &Event, telemetry: &TelemetrySink) {
    let Some(content) = event.content() else {
        return;
    };

    for part in &content.parts {
        match part {
            Part::FunctionCall { name, .. } => {
                tracing::info!(
                    tool = %name,
                    author = %event.author,
                    lifecycle = "requested",
                    "Tool call requested"
                );
                telemetry.emit(
                    "tool.requested",
                    serde_json::json!({
                        "tool": name,
                        "author": event.author
                    }),
                );
            }
            Part::FunctionResponse {
                function_response, ..
            } => {
                if let Some(error_message) =
                    extract_tool_failure_message(&function_response.response)
                {
                    tracing::warn!(
                        tool = %function_response.name,
                        author = %event.author,
                        lifecycle = "failed",
                        error = %error_message,
                        "Tool execution failed"
                    );
                    telemetry.emit(
                        "tool.failed",
                        serde_json::json!({
                            "tool": function_response.name,
                            "author": event.author,
                            "error": error_message
                        }),
                    );
                } else {
                    tracing::info!(
                        tool = %function_response.name,
                        author = %event.author,
                        lifecycle = "succeeded",
                        "Tool execution completed"
                    );
                    telemetry.emit(
                        "tool.succeeded",
                        serde_json::json!({
                            "tool": function_response.name,
                            "author": event.author
                        }),
                    );
                }
            }
            _ => {}
        }
    }
}

pub async fn run_prompt(
    runner: &Runner,
    cfg: &RuntimeConfig,
    prompt: &str,
    telemetry: &TelemetrySink,
) -> Result<String> {
    let mut stream = runner
        .run(
            cfg.user_id.clone(),
            cfg.session_id.clone(),
            Content::new("user").with_text(prompt),
        )
        .await
        .context("failed to start runner stream")?;

    let mut tracker = AuthorTextTracker::default();

    while let Some(event_result) = stream.next().await {
        let event = match event_result {
            Ok(e) => e,
            Err(e) => {
                tracing::warn!("Runner event error: {e:#}");
                continue;
            }
        };
        let text = event_text(&event);

        tracing::debug!(
            author = %event.author,
            is_final = event.is_final_response(),
            partial = event.llm_response.partial,
            text_len = text.len(),
            "received runner event"
        );

        if event.author == "user" {
            continue;
        }

        emit_tool_lifecycle_events(&event, telemetry);

        let _ = tracker.ingest_parts(
            &event.author,
            &text,
            event.llm_response.partial,
            event.is_final_response(),
        );
    }

    Ok(tracker
        .resolve_text()
        .unwrap_or_else(|| NO_TEXTUAL_RESPONSE.to_string()))
}

pub async fn run_prompt_with_retrieval(
    runner: &Runner,
    cfg: &RuntimeConfig,
    prompt: &str,
    retrieval: &dyn RetrievalService,
    telemetry: &TelemetrySink,
) -> Result<String> {
    let policy = RetrievalPolicy {
        max_chunks: cfg.retrieval_max_chunks,
        max_chars: cfg.retrieval_max_chars,
        min_score: cfg.retrieval_min_score,
    };
    let enriched = augment_prompt_with_retrieval(retrieval, prompt, policy)?;
    run_prompt(runner, cfg, &enriched, telemetry).await
}

pub async fn run_prompt_streaming(
    runner: &Runner,
    cfg: &RuntimeConfig,
    prompt: &str,
    telemetry: &TelemetrySink,
) -> Result<String> {
    let mut stream = runner
        .run(
            cfg.user_id.clone(),
            cfg.session_id.clone(),
            Content::new("user").with_text(prompt),
        )
        .await
        .context("failed to start runner stream")?;

    let mut tracker = AuthorTextTracker::default();
    let mut emitted_text_by_author: HashMap<String, String> = HashMap::new();
    let mut printed_any_output = false;
    let mut spinner = Some(Spinner::start("Thinking..."));

    // Winnow streaming markdown state
    let mut md_buf = String::new();
    let mut md_offset: usize = 0;
    let mut md_state = ParseState::new();
    let mut stdout = io::stdout();

    while let Some(event_result) = stream.next().await {
        let event = match event_result {
            Ok(e) => e,
            Err(e) => {
                eprintln!("{}", crate::theme::DIM);
                eprintln!("  Runner error: {e:#}");
                eprintln!("{}", crate::theme::RESET);
                continue;
            }
        };
        let text = event_text(&event);

        if event.author == "user" {
            continue;
        }

        emit_tool_lifecycle_events(&event, telemetry);

        let delta = tracker.ingest_parts(
            &event.author,
            &text,
            event.llm_response.partial,
            event.is_final_response(),
        );
        if !delta.is_empty() {
            if let Some(s) = spinner.take() {
                s.stop();
            }

            md_buf.push_str(&delta);

            // Parse as much as possible from the buffer
            loop {
                let input = winnow::Partial::new(&md_buf[md_offset..]);
                match parse_markdown(input, &mut stdout, &mut md_state) {
                    Ok(parsed) => {
                        md_offset += winnow::stream::Offset::offset_from(&parsed, &input);
                        stdout.flush().context("failed to flush stdout")?;
                        md_state.newline = md_state.set_newline;
                        md_state.set_newline = false;
                    }
                    Err(winnow::error::ErrMode::Incomplete(_)) => break,
                    Err(_) => break,
                }
            }

            emitted_text_by_author
                .entry(event.author.clone())
                .or_default()
                .push_str(&delta);
            printed_any_output = true;
        }
    }

    // Ensure spinner is stopped if stream ended without output
    drop(spinner);

    // Flush remaining buffer: append newline to force parser to complete (Q CLI hack)
    md_buf.push('\n');
    loop {
        let input = winnow::Partial::new(&md_buf[md_offset..]);
        match parse_markdown(input, &mut stdout, &mut md_state) {
            Ok(parsed) => {
                md_offset += winnow::stream::Offset::offset_from(&parsed, &input);
                stdout.flush().ok();
                md_state.newline = md_state.set_newline;
                md_state.set_newline = false;
            }
            _ => break,
        }
    }

    if printed_any_output {
        if let (Some(final_text), Some(final_author)) = (
            tracker.latest_final_text.as_deref(),
            tracker.latest_final_author.as_deref(),
        ) {
            let emitted = emitted_text_by_author
                .get(final_author)
                .map(String::as_str)
                .unwrap_or_default();

            if let Some(suffix) = final_stream_suffix(emitted, final_text) {
                print!("{suffix}");
                io::stdout().flush().context("failed to flush stdout")?;
            }
        }

        println!();
        return Ok(tracker
            .resolve_text()
            .unwrap_or_else(|| NO_TEXTUAL_RESPONSE.to_string()));
    }

    let fallback = tracker
        .resolve_text()
        .unwrap_or_else(|| NO_TEXTUAL_RESPONSE.to_string());

    println!("{fallback}");
    Ok(fallback)
}

pub async fn run_prompt_streaming_with_retrieval(
    runner: &Runner,
    cfg: &RuntimeConfig,
    prompt: &str,
    retrieval: &dyn RetrievalService,
    telemetry: &TelemetrySink,
) -> Result<String> {
    let policy = RetrievalPolicy {
        max_chunks: cfg.retrieval_max_chunks,
        max_chars: cfg.retrieval_max_chars,
        min_score: cfg.retrieval_min_score,
    };
    let enriched = augment_prompt_with_retrieval(retrieval, prompt, policy)?;
    run_prompt_streaming(runner, cfg, &enriched, telemetry).await
}
