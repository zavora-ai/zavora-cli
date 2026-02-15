use std::collections::{BTreeSet, HashMap};
use std::fs::OpenOptions;
use std::io::{self, BufRead, Write};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use serde_json::{Value, json};

use crate::config::RuntimeConfig;

pub fn unix_ms_now() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

#[derive(Debug, Clone)]
pub struct TelemetrySink {
    pub enabled: bool,
    pub path: PathBuf,
    pub run_id: String,
    pub command: String,
    pub session_id: String,
    pub file_lock: Arc<std::sync::Mutex<()>>,
}

impl TelemetrySink {
    pub fn new(cfg: &RuntimeConfig, command: String) -> Self {
        let run_id = format!("run-{}-{}", unix_ms_now(), std::process::id());
        Self {
            enabled: cfg.telemetry_enabled,
            path: PathBuf::from(&cfg.telemetry_path),
            run_id,
            command,
            session_id: cfg.session_id.clone(),
            file_lock: Arc::new(std::sync::Mutex::new(())),
        }
    }

    pub fn emit(&self, event: &str, payload: Value) {
        if !self.enabled {
            return;
        }

        let mut record = serde_json::Map::new();
        record.insert("ts_unix_ms".to_string(), json!(unix_ms_now()));
        record.insert("event".to_string(), json!(event));
        record.insert("run_id".to_string(), json!(self.run_id));
        record.insert("command".to_string(), json!(self.command));
        record.insert("session_id".to_string(), json!(self.session_id));

        if let Some(map) = payload.as_object() {
            for (key, value) in map {
                record.insert(key.clone(), value.clone());
            }
        }

        let value = Value::Object(record);
        if let Err(err) = self.append_event_line(&value) {
            tracing::warn!(
                event = event,
                path = %self.path.display(),
                error = %err,
                "telemetry write failed"
            );
        }
    }

    fn append_event_line(&self, value: &Value) -> Result<()> {
        if let Some(parent) = self.path.parent()
            && !parent.as_os_str().is_empty()
        {
            std::fs::create_dir_all(parent).with_context(|| {
                format!(
                    "failed to create telemetry directory '{}'",
                    parent.display()
                )
            })?;
        }

        let _guard = self.file_lock.lock().unwrap_or_else(|e| e.into_inner());

        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .with_context(|| format!("failed to open telemetry path '{}'", self.path.display()))?;

        serde_json::to_writer(&mut file, value).with_context(|| {
            format!("failed to serialize telemetry event for '{}'", self.command)
        })?;
        writeln!(file).context("failed to write telemetry newline")
    }
}

#[derive(Debug, Default)]
pub struct TelemetrySummary {
    pub total_lines: usize,
    pub parsed_events: usize,
    pub parse_errors: usize,
    pub unique_runs: BTreeSet<String>,
    pub command_counts: HashMap<String, usize>,
    pub command_completed: usize,
    pub command_failed: usize,
    pub tool_requested: usize,
    pub tool_succeeded: usize,
    pub tool_failed: usize,
    pub last_event_ts_unix_ms: Option<u128>,
}

pub fn summarize_telemetry_lines(lines: Vec<String>, limit: usize) -> TelemetrySummary {
    let mut summary = TelemetrySummary::default();
    let max_events = limit.max(1);
    summary.total_lines = lines.len();

    for line in lines.into_iter().rev().take(max_events) {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        let parsed = match serde_json::from_str::<Value>(line) {
            Ok(value) => value,
            Err(_) => {
                summary.parse_errors += 1;
                continue;
            }
        };

        summary.parsed_events += 1;

        if let Some(run_id) = parsed.get("run_id").and_then(Value::as_str)
            && !run_id.is_empty()
        {
            summary.unique_runs.insert(run_id.to_string());
        }

        if let Some(command) = parsed.get("command").and_then(Value::as_str)
            && !command.is_empty()
        {
            *summary
                .command_counts
                .entry(command.to_string())
                .or_insert(0) += 1;
        }

        if let Some(ts) = parsed.get("ts_unix_ms").and_then(Value::as_u64) {
            let ts_u128 = ts as u128;
            summary.last_event_ts_unix_ms = Some(
                summary
                    .last_event_ts_unix_ms
                    .map(|existing| existing.max(ts_u128))
                    .unwrap_or(ts_u128),
            );
        }

        match parsed
            .get("event")
            .and_then(Value::as_str)
            .unwrap_or_default()
        {
            "command.completed" => summary.command_completed += 1,
            "command.failed" => summary.command_failed += 1,
            "tool.requested" => summary.tool_requested += 1,
            "tool.succeeded" => summary.tool_succeeded += 1,
            "tool.failed" => summary.tool_failed += 1,
            _ => {}
        }
    }

    summary
}

pub fn run_telemetry_report(
    cfg: &RuntimeConfig,
    path_override: Option<String>,
    limit: usize,
) -> Result<()> {
    let path = PathBuf::from(path_override.unwrap_or_else(|| cfg.telemetry_path.clone()));
    if !path.exists() {
        println!("No telemetry file found at '{}'.", path.display());
        return Ok(());
    }

    let file = std::fs::File::open(&path)
        .with_context(|| format!("failed to open telemetry file '{}'", path.display()))?;
    let reader = io::BufReader::new(file);
    let lines = reader
        .lines()
        .collect::<std::result::Result<Vec<String>, std::io::Error>>()
        .with_context(|| format!("failed to read telemetry file '{}'", path.display()))?;

    let summary = summarize_telemetry_lines(lines, limit);
    let mut commands = summary.command_counts.iter().collect::<Vec<_>>();
    commands.sort_by_key(|(name, count)| (std::cmp::Reverse(**count), (*name).clone()));

    println!("Telemetry report");
    println!("Path: {}", path.display());
    println!("Lines in file: {}", summary.total_lines);
    println!(
        "Events analyzed: {} (parse_errors={})",
        summary.parsed_events, summary.parse_errors
    );
    println!("Unique runs: {}", summary.unique_runs.len());
    println!(
        "Command outcomes: completed={} failed={}",
        summary.command_completed, summary.command_failed
    );
    println!(
        "Tool lifecycle: requested={} succeeded={} failed={}",
        summary.tool_requested, summary.tool_succeeded, summary.tool_failed
    );

    if !commands.is_empty() {
        println!("Top commands:");
        for (name, count) in commands.into_iter().take(5) {
            println!("- {}: {}", name, count);
        }
    }

    if let Some(last_ts) = summary.last_event_ts_unix_ms {
        println!("Last event ts_unix_ms: {last_ts}");
    }

    Ok(())
}
