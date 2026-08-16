use std::io::{self, IsTerminal, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use adk_rust::prelude::Runner;
use anyhow::{Context, Result, bail};
use chrono::Utc;
use serde::Serialize;
use serde_json::{Value, json};

use crate::cli::OutputFormat;
use crate::config::RuntimeConfig;
use crate::error::{categorize_error, render_error_message};
use crate::guardrail::{apply_guardrail, buffered_output_required};
use crate::retrieval::{RetrievalPolicy, RetrievalService, augment_prompt_with_retrieval};
use crate::streaming::{NO_TEXTUAL_RESPONSE, UiEvent, run_prompt_to_ui};
use crate::telemetry::TelemetrySink;

pub const HEADLESS_SCHEMA_VERSION: &str = "zavora.headless.v1";
const MAX_INPUT_BYTES: u64 = 16 * 1024 * 1024;
static NEXT_STREAM_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone)]
pub struct HeadlessOptions {
    pub output_format: OutputFormat,
    pub input_files: Vec<PathBuf>,
    pub read_stdin: bool,
    pub no_stdin: bool,
    pub always_approve: bool,
}

#[derive(Debug, Clone)]
pub struct RunMetadata {
    pub command: String,
    pub session_id: String,
    pub provider: String,
    pub model: String,
}

struct HeadlessRun<'a> {
    runner: &'a Runner,
    cfg: &'a RuntimeConfig,
    retrieval: &'a dyn RetrievalService,
    telemetry: &'a TelemetrySink,
    metadata: &'a RunMetadata,
}

#[derive(Debug, Serialize)]
struct RunStats {
    duration_ms: u128,
    response_chars: usize,
    tool_calls: usize,
}

#[derive(Debug, Serialize)]
struct ResultDocument<'a> {
    schema_version: &'static str,
    #[serde(rename = "type")]
    event_type: &'static str,
    success: bool,
    command: &'a str,
    session_id: &'a str,
    provider: &'a str,
    model: &'a str,
    response: &'a str,
    stats: RunStats,
}

pub fn should_read_stdin(force: bool, suppress: bool) -> bool {
    force || (!suppress && !io::stdin().is_terminal())
}

pub fn load_prompt(parts: &[String], options: &HeadlessOptions) -> Result<String> {
    let stdin = if should_read_stdin(options.read_stdin, options.no_stdin) {
        Some(read_limited(io::stdin(), "stdin")?)
    } else {
        None
    };
    assemble_prompt(parts, &options.input_files, stdin.as_deref())
}

pub fn assemble_prompt(
    parts: &[String],
    input_files: &[PathBuf],
    stdin: Option<&str>,
) -> Result<String> {
    let mut sections = Vec::new();
    let direct = parts.join(" ");
    if !direct.trim().is_empty() {
        sections.push(direct);
    }

    for path in input_files {
        let content = read_file_limited(path)?;
        if !content.trim().is_empty() {
            sections.push(format!(
                "--- begin file: {} ---\n{}\n--- end file: {} ---",
                path.display(),
                content,
                path.display()
            ));
        }
    }

    if let Some(content) = stdin
        && !content.trim().is_empty()
    {
        sections.push(format!("--- begin stdin ---\n{content}\n--- end stdin ---"));
    }

    if sections.is_empty() {
        bail!("no prompt input supplied; pass text, --file PATH, or pipe stdin")
    }
    Ok(sections.join("\n\n"))
}

fn read_file_limited(path: &Path) -> Result<String> {
    let file = std::fs::File::open(path)
        .with_context(|| format!("failed to read input file '{}'", path.display()))?;
    read_limited(file, &format!("input file '{}'", path.display()))
}

fn read_limited(reader: impl Read, source: &str) -> Result<String> {
    let mut bytes = Vec::new();
    reader
        .take(MAX_INPUT_BYTES + 1)
        .read_to_end(&mut bytes)
        .with_context(|| format!("failed to read {source}"))?;
    if bytes.len() as u64 > MAX_INPUT_BYTES {
        bail!("{source} exceeds the 16 MiB input limit")
    }
    String::from_utf8(bytes).with_context(|| format!("{source} is not valid UTF-8"))
}

pub async fn run_headless(
    runner: &Runner,
    cfg: &RuntimeConfig,
    prompt: &str,
    retrieval: &dyn RetrievalService,
    telemetry: &TelemetrySink,
    metadata: &RunMetadata,
    format: OutputFormat,
) -> Result<String> {
    if format == OutputFormat::StreamJson {
        NEXT_STREAM_SEQUENCE.store(0, Ordering::SeqCst);
    }
    let mut stdout = io::stdout().lock();
    run_headless_to(
        HeadlessRun {
            runner,
            cfg,
            retrieval,
            telemetry,
            metadata,
        },
        prompt,
        format,
        &mut stdout,
    )
    .await
}

async fn run_headless_to<W: Write>(
    run: HeadlessRun<'_>,
    prompt: &str,
    format: OutputFormat,
    writer: &mut W,
) -> Result<String> {
    let HeadlessRun {
        runner,
        cfg,
        retrieval,
        telemetry,
        metadata,
    } = run;
    let started = Instant::now();
    let mut tool_calls = 0usize;

    let answer = if format == OutputFormat::StreamJson
        && !buffered_output_required(cfg.guardrail_output_mode)
    {
        write_event(
            writer,
            0,
            "init",
            metadata,
            json!({
                "capabilities": ["message", "tool_use", "tool_result", "agent", "system"]
            }),
        )?;
        let policy = RetrievalPolicy {
            max_chunks: cfg.retrieval_max_chunks,
            max_chars: cfg.retrieval_max_chars,
            min_score: cfg.retrieval_min_score,
        };
        let enriched = augment_prompt_with_retrieval(retrieval, prompt, policy)?;
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let run = run_prompt_to_ui(runner, cfg, &enriched, telemetry, tx);
        tokio::pin!(run);
        let mut sequence = 1u64;
        let mut runtime_errors = Vec::new();
        let answer = loop {
            tokio::select! {
                result = &mut run => break result?,
                event = rx.recv() => {
                    let Some(event) = event else { continue };
                    if let UiEvent::Error(message) = &event {
                        runtime_errors.push(message.clone());
                    }
                    if emit_ui_event(writer, sequence, metadata, event, &mut tool_calls)? {
                        sequence += 1;
                    }
                }
            }
        };
        while let Ok(event) = rx.try_recv() {
            if let UiEvent::Error(message) = &event {
                runtime_errors.push(message.clone());
            }
            if emit_ui_event(writer, sequence, metadata, event, &mut tool_calls)? {
                sequence += 1;
            }
        }
        fail_on_error_only_response(&answer, &runtime_errors)?;
        let answer = apply_guardrail(cfg, telemetry, "output", cfg.guardrail_output_mode, &answer)?;
        write_result_event(
            writer,
            sequence,
            metadata,
            &answer,
            started.elapsed().as_millis(),
            tool_calls,
        )?;
        answer
    } else {
        let (answer, captured_tool_calls) =
            run_buffered(runner, cfg, prompt, retrieval, telemetry).await?;
        tool_calls = captured_tool_calls;
        let answer = apply_guardrail(cfg, telemetry, "output", cfg.guardrail_output_mode, &answer)?;
        match format {
            OutputFormat::Text => writeln!(writer, "{answer}")?,
            OutputFormat::Json => write_result_document(
                writer,
                metadata,
                &answer,
                started.elapsed().as_millis(),
                tool_calls,
            )?,
            OutputFormat::StreamJson => {
                write_event(writer, 0, "init", metadata, json!({"buffered": true}))?;
                write_event(
                    writer,
                    1,
                    "message",
                    metadata,
                    json!({
                        "role": "assistant", "author": "agent", "delta": answer
                    }),
                )?;
                write_result_event(
                    writer,
                    2,
                    metadata,
                    &answer,
                    started.elapsed().as_millis(),
                    tool_calls,
                )?;
            }
        }
        answer
    };

    writer.flush()?;
    Ok(answer)
}

async fn run_buffered(
    runner: &Runner,
    cfg: &RuntimeConfig,
    prompt: &str,
    retrieval: &dyn RetrievalService,
    telemetry: &TelemetrySink,
) -> Result<(String, usize)> {
    let policy = RetrievalPolicy {
        max_chunks: cfg.retrieval_max_chunks,
        max_chars: cfg.retrieval_max_chars,
        min_score: cfg.retrieval_min_score,
    };
    let enriched = augment_prompt_with_retrieval(retrieval, prompt, policy)?;
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let answer = run_prompt_to_ui(runner, cfg, &enriched, telemetry, tx).await?;
    let mut tool_calls = 0usize;
    let mut runtime_errors = Vec::new();
    while let Ok(event) = rx.try_recv() {
        if matches!(event, UiEvent::ToolStarted { .. }) {
            tool_calls += 1;
        }
        if let UiEvent::Error(message) = event {
            runtime_errors.push(message);
        }
    }
    fail_on_error_only_response(&answer, &runtime_errors)?;
    Ok((answer, tool_calls))
}

fn fail_on_error_only_response(answer: &str, runtime_errors: &[String]) -> Result<()> {
    if answer == NO_TEXTUAL_RESPONSE
        && let Some(error) = runtime_errors.last()
    {
        bail!("provider stream failed: {error}");
    }
    Ok(())
}

fn emit_ui_event<W: Write>(
    writer: &mut W,
    sequence: u64,
    metadata: &RunMetadata,
    event: UiEvent,
    tool_calls: &mut usize,
) -> Result<bool> {
    let (kind, data) = match event {
        UiEvent::AgentChanged(agent) => ("agent", json!({"name": agent})),
        UiEvent::System(message) => ("system", json!({"message": message})),
        UiEvent::TextDelta { author, text } => (
            "message",
            json!({"role": "assistant", "author": author, "delta": text}),
        ),
        UiEvent::ToolStarted {
            call_id,
            name,
            detail,
        } => {
            *tool_calls += 1;
            (
                "tool_use",
                json!({"call_id": call_id, "name": name, "detail": detail}),
            )
        }
        UiEvent::ToolFinished {
            call_id,
            name,
            success,
            detail,
        } => (
            "tool_result",
            json!({"call_id": call_id, "name": name, "success": success, "detail": detail}),
        ),
        UiEvent::Error(message) => ("error", json!({"fatal": false, "message": message})),
        UiEvent::Completed(_) => return Ok(false),
    };
    write_event(writer, sequence, kind, metadata, data)?;
    Ok(true)
}

fn write_event<W: Write>(
    writer: &mut W,
    sequence: u64,
    kind: &str,
    metadata: &RunMetadata,
    data: Value,
) -> Result<()> {
    NEXT_STREAM_SEQUENCE.store(sequence.saturating_add(1), Ordering::SeqCst);
    write_json_line(
        writer,
        &json!({
            "schema_version": HEADLESS_SCHEMA_VERSION,
            "sequence": sequence,
            "timestamp": Utc::now().to_rfc3339(),
            "type": kind,
            "command": metadata.command,
            "session_id": metadata.session_id,
            "data": data
        }),
    )
}

fn write_result_event<W: Write>(
    writer: &mut W,
    sequence: u64,
    metadata: &RunMetadata,
    answer: &str,
    duration_ms: u128,
    tool_calls: usize,
) -> Result<()> {
    write_event(
        writer,
        sequence,
        "result",
        metadata,
        json!({
            "success": true,
            "response": answer,
            "provider": metadata.provider,
            "model": metadata.model,
            "stats": {
                "duration_ms": duration_ms,
                "response_chars": answer.chars().count(),
                "tool_calls": tool_calls
            }
        }),
    )
}

fn write_result_document<W: Write>(
    writer: &mut W,
    metadata: &RunMetadata,
    answer: &str,
    duration_ms: u128,
    tool_calls: usize,
) -> Result<()> {
    let document = ResultDocument {
        schema_version: HEADLESS_SCHEMA_VERSION,
        event_type: "result",
        success: true,
        command: &metadata.command,
        session_id: &metadata.session_id,
        provider: &metadata.provider,
        model: &metadata.model,
        response: answer,
        stats: RunStats {
            duration_ms,
            response_chars: answer.chars().count(),
            tool_calls,
        },
    };
    write_json_line(writer, &document)
}

fn write_json_line<W: Write, T: Serialize>(writer: &mut W, value: &T) -> Result<()> {
    serde_json::to_writer(&mut *writer, value)?;
    writer.write_all(b"\n")?;
    writer.flush()?;
    Ok(())
}

pub fn write_structured_error(
    format: OutputFormat,
    err: &anyhow::Error,
    show_sensitive: bool,
) -> Result<()> {
    let category = categorize_error(err);
    let message = render_error_message(err, show_sensitive);
    let error = json!({
        "category": category.code(),
        "message": message,
        "hint": category.hint(),
        "exit_code": category.exit_code()
    });
    let value = json!({
        "schema_version": HEADLESS_SCHEMA_VERSION,
        "type": "error",
        "success": false,
        "error": error
    });
    let mut stdout = io::stdout().lock();
    match format {
        OutputFormat::Text => unreachable!("text errors use the human renderer"),
        OutputFormat::Json => write_json_line(&mut stdout, &value),
        OutputFormat::StreamJson => write_json_line(
            &mut stdout,
            &json!({
                "schema_version": HEADLESS_SCHEMA_VERSION,
                "sequence": NEXT_STREAM_SEQUENCE.fetch_add(1, Ordering::SeqCst),
                "timestamp": Utc::now().to_rfc3339(),
                "type": "error",
                "command": Value::Null,
                "session_id": Value::Null,
                "data": {"fatal": true, "success": false, "error": error}
            }),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::{Cli, Commands};
    use clap::Parser;

    #[test]
    fn prompt_assembly_is_ordered_and_requires_input() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("context.md");
        std::fs::write(&path, "file body").unwrap();
        let prompt = assemble_prompt(&["do the work".into()], &[path], Some("stdin body")).unwrap();
        assert!(prompt.starts_with("do the work"));
        assert!(prompt.find("file body").unwrap() < prompt.find("stdin body").unwrap());
        assert!(assemble_prompt(&[], &[], None).is_err());
    }

    #[test]
    fn result_document_has_a_versioned_stable_shape() {
        let metadata = RunMetadata {
            command: "ask".into(),
            session_id: "s1".into(),
            provider: "openai".into(),
            model: "test".into(),
        };
        let mut output = Vec::new();
        write_result_document(&mut output, &metadata, "hello", 7, 2).unwrap();
        let value: Value = serde_json::from_slice(&output).unwrap();
        assert_eq!(value["schema_version"], HEADLESS_SCHEMA_VERSION);
        assert_eq!(value["type"], "result");
        assert_eq!(value["response"], "hello");
        assert_eq!(value["stats"]["tool_calls"], 2);
    }

    #[test]
    fn jsonl_events_are_one_object_per_line() {
        let metadata = RunMetadata {
            command: "ask".into(),
            session_id: "s1".into(),
            provider: "openai".into(),
            model: "test".into(),
        };
        let mut output = Vec::new();
        write_event(&mut output, 3, "message", &metadata, json!({"delta": "hi"})).unwrap();
        let line = std::str::from_utf8(&output).unwrap();
        assert_eq!(line.lines().count(), 1);
        let value: Value = serde_json::from_str(line).unwrap();
        assert_eq!(value["sequence"], 3);
        assert_eq!(value["data"]["delta"], "hi");
    }

    #[test]
    fn cli_accepts_structured_formats_and_input_only_ask() {
        let cli = Cli::try_parse_from([
            "zavora-cli",
            "ask",
            "--output-format",
            "jsonl",
            "--file",
            "context.md",
        ])
        .unwrap();
        assert_eq!(cli.output_format, OutputFormat::StreamJson);
        assert_eq!(cli.input_files, vec![PathBuf::from("context.md")]);
        assert!(matches!(cli.command, Some(Commands::Ask { prompt }) if prompt.is_empty()));
    }

    #[test]
    fn error_only_responses_are_failures() {
        assert!(
            fail_on_error_only_response(NO_TEXTUAL_RESPONSE, &["network down".into()]).is_err()
        );
        assert!(
            fail_on_error_only_response("partial but useful", &["retry failed".into()]).is_ok()
        );
    }
}
