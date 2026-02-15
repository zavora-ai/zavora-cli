use std::collections::{BTreeSet, HashMap};
use std::fs::OpenOptions;
use std::io::{self, BufRead, Write};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use adk_rust::futures::StreamExt;
use adk_rust::prelude::{
    Agent, AnthropicClient, AnthropicConfig, Content, DeepSeekClient, DeepSeekConfig, END, Event,
    ExitLoopTool, FunctionTool, GeminiModel, GraphAgent, GroqClient, GroqConfig,
    InMemoryArtifactService, InMemorySessionService, Llm, LlmAgentBuilder, LlmRequest, LoopAgent,
    NodeOutput, OllamaConfig, OllamaModel, OpenAIClient, OpenAIConfig, ParallelAgent, Part, Router,
    RunConfig, Runner, RunnerConfig, START, SequentialAgent, Tool, Toolset,
};
use adk_rust::{ReadonlyContext, ToolConfirmationDecision, ToolConfirmationPolicy};
use adk_session::{
    CreateRequest, DatabaseSessionService, DeleteRequest, GetRequest, ListRequest, SessionService,
};
use adk_tool::mcp::RefreshConfig;
use adk_tool::{McpAuth, McpHttpClientBuilder};
use anyhow::{Context, Result};
use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router as AxumRouter};
use clap::{Parser, Subcommand, ValueEnum};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tracing::level_filters::LevelFilter;

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum, Deserialize)]
#[serde(rename_all = "lowercase")]
enum Provider {
    Auto,
    Gemini,
    Openai,
    Anthropic,
    Deepseek,
    Groq,
    Ollama,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum WorkflowMode {
    Single,
    Sequential,
    Parallel,
    Loop,
    Graph,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum, Deserialize)]
#[serde(rename_all = "lowercase")]
enum SessionBackend {
    Memory,
    Sqlite,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum, Deserialize)]
#[serde(rename_all = "lowercase")]
enum RetrievalBackend {
    Disabled,
    Local,
    Semantic,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum, Deserialize)]
#[serde(rename_all = "kebab-case")]
enum ToolConfirmationMode {
    Never,
    McpOnly,
    Always,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum, Deserialize)]
#[serde(rename_all = "kebab-case")]
enum GuardrailMode {
    Disabled,
    Observe,
    Block,
    Redact,
}

#[derive(Debug, Subcommand)]
enum ProfileCommands {
    #[command(about = "List configured profiles and highlight the active profile")]
    List,
    #[command(about = "Show the active profile's resolved runtime settings")]
    Show,
}

#[derive(Debug, Subcommand)]
enum AgentCommands {
    #[command(about = "List available agents from local/global catalogs")]
    List,
    #[command(about = "Show resolved agent configuration")]
    Show {
        #[arg(long)]
        name: Option<String>,
    },
    #[command(about = "Select active agent for this workspace")]
    Select {
        #[arg(long)]
        name: String,
    },
}

#[derive(Debug, Subcommand)]
enum McpCommands {
    #[command(about = "List MCP servers configured for the active profile")]
    List,
    #[command(about = "Discover MCP tools from configured servers (or a specific server)")]
    Discover {
        #[arg(long)]
        server: Option<String>,
    },
}

#[derive(Debug, Subcommand)]
enum SessionCommands {
    #[command(about = "List all sessions for the current app/user")]
    List,
    #[command(about = "Show events for a specific session")]
    Show {
        #[arg(long)]
        session_id: Option<String>,
        #[arg(long, default_value_t = 20)]
        recent: usize,
    },
    #[command(about = "Delete a session (requires --force)")]
    Delete {
        #[arg(long)]
        session_id: Option<String>,
        #[arg(long, default_value_t = false)]
        force: bool,
    },
    #[command(
        about = "Prune old sessions, keeping N most recent (requires --force unless --dry-run)"
    )]
    Prune {
        #[arg(long, default_value_t = 20)]
        keep: usize,
        #[arg(long, default_value_t = false)]
        dry_run: bool,
        #[arg(long, default_value_t = false)]
        force: bool,
    },
}

#[derive(Debug, Subcommand)]
enum TelemetryCommands {
    #[command(about = "Summarize telemetry events from a JSONL stream")]
    Report {
        #[arg(long)]
        path: Option<String>,
        #[arg(long, default_value_t = 5000)]
        limit: usize,
    },
}

#[derive(Debug, Subcommand)]
enum EvalCommands {
    #[command(about = "Run eval dataset and emit quality/benchmark report")]
    Run {
        #[arg(long)]
        dataset: Option<String>,
        #[arg(long)]
        output: Option<String>,
        #[arg(long, default_value_t = 100)]
        benchmark_iterations: usize,
        #[arg(long, default_value_t = 0.80)]
        fail_under: f64,
    },
}

#[derive(Debug, Subcommand)]
enum ServerCommands {
    #[command(about = "Run HTTP server mode for health, ask, and A2A endpoints")]
    Serve {
        #[arg(long, default_value = "127.0.0.1")]
        host: String,
        #[arg(long, default_value_t = 8787)]
        port: u16,
    },
    #[command(about = "Run local A2A contract smoke check")]
    A2aSmoke,
}

const CLI_EXAMPLES: &str = "Examples:\n\
  zavora-cli ask \"Design a Rust CLI with release-based milestones\"\n\
  zavora-cli --provider openai --model gpt-4o-mini chat\n\
  zavora-cli workflow sequential \"Plan a v0.2.0 rollout\"\n\
  zavora-cli --session-backend sqlite --session-db-url sqlite://.zavora/sessions.db sessions list\n\
  zavora-cli --session-backend sqlite --session-db-url sqlite://.zavora/sessions.db sessions prune --keep 20 --dry-run\n\
  zavora-cli agents list\n\
  zavora-cli agents show --name coder\n\
  zavora-cli agents select --name reviewer\n\
  zavora-cli mcp list\n\
  zavora-cli mcp discover --server ops-tools\n\
  zavora-cli --tool-confirmation-mode always --approve-tool release_template ask \"Draft release checklist\"\n\
  zavora-cli --guardrail-input-mode observe --guardrail-output-mode redact ask \"Review this draft\"\n\
  zavora-cli server serve --host 127.0.0.1 --port 8787\n\
  zavora-cli server a2a-smoke\n\
  zavora-cli telemetry report --limit 2000\n\
  zavora-cli eval run --benchmark-iterations 200 --fail-under 0.90\n\
\n\
Switching behavior:\n\
  - Use --agent <name> to select a named agent profile for this invocation.\n\
  - Use --provider/--model to switch runtime model selection per invocation.\n\
  - In chat, use /help for command discovery and /provider, /model, /tools, /mcp, /usage, /status.";

#[derive(Debug, Parser)]
#[command(name = "zavora-cli")]
#[command(about = "Rust CLI agent shell built on ADK-Rust")]
#[command(after_long_help = CLI_EXAMPLES)]
struct Cli {
    #[arg(long, env = "ZAVORA_PROVIDER", value_enum, default_value_t = Provider::Auto)]
    provider: Provider,

    #[arg(long, env = "ZAVORA_MODEL")]
    model: Option<String>,

    #[arg(long, env = "ZAVORA_AGENT")]
    agent: Option<String>,

    #[arg(long, env = "ZAVORA_PROFILE", default_value = "default")]
    profile: String,

    #[arg(long, env = "ZAVORA_CONFIG", default_value = ".zavora/config.toml")]
    config_path: String,

    #[arg(long, env = "ZAVORA_APP_NAME")]
    app_name: Option<String>,

    #[arg(long, env = "ZAVORA_USER_ID")]
    user_id: Option<String>,

    #[arg(long, env = "ZAVORA_SESSION_ID")]
    session_id: Option<String>,

    #[arg(long, env = "ZAVORA_SESSION_BACKEND", value_enum)]
    session_backend: Option<SessionBackend>,

    #[arg(long, env = "ZAVORA_SESSION_DB_URL")]
    session_db_url: Option<String>,

    #[arg(long, env = "ZAVORA_SHOW_SENSITIVE_CONFIG", default_value_t = false)]
    show_sensitive_config: bool,

    #[arg(long, env = "ZAVORA_RETRIEVAL_BACKEND", value_enum)]
    retrieval_backend: Option<RetrievalBackend>,

    #[arg(long, env = "ZAVORA_RETRIEVAL_DOC_PATH")]
    retrieval_doc_path: Option<String>,

    #[arg(long, env = "ZAVORA_RETRIEVAL_MAX_CHUNKS")]
    retrieval_max_chunks: Option<usize>,

    #[arg(long, env = "ZAVORA_RETRIEVAL_MAX_CHARS")]
    retrieval_max_chars: Option<usize>,

    #[arg(long, env = "ZAVORA_RETRIEVAL_MIN_SCORE")]
    retrieval_min_score: Option<usize>,

    #[arg(long, env = "ZAVORA_TOOL_CONFIRMATION_MODE", value_enum)]
    tool_confirmation_mode: Option<ToolConfirmationMode>,

    #[arg(long, env = "ZAVORA_REQUIRE_CONFIRM_TOOL")]
    require_confirm_tool: Vec<String>,

    #[arg(long, env = "ZAVORA_APPROVE_TOOL")]
    approve_tool: Vec<String>,

    #[arg(long, env = "ZAVORA_TOOL_TIMEOUT_SECS")]
    tool_timeout_secs: Option<u64>,

    #[arg(long, env = "ZAVORA_TOOL_RETRY_ATTEMPTS")]
    tool_retry_attempts: Option<u32>,

    #[arg(long, env = "ZAVORA_TOOL_RETRY_DELAY_MS")]
    tool_retry_delay_ms: Option<u64>,

    #[arg(long, env = "ZAVORA_TELEMETRY_ENABLED", action = clap::ArgAction::Set)]
    telemetry_enabled: Option<bool>,

    #[arg(long, env = "ZAVORA_TELEMETRY_PATH")]
    telemetry_path: Option<String>,

    #[arg(long, env = "ZAVORA_GUARDRAIL_INPUT_MODE", value_enum)]
    guardrail_input_mode: Option<GuardrailMode>,

    #[arg(long, env = "ZAVORA_GUARDRAIL_OUTPUT_MODE", value_enum)]
    guardrail_output_mode: Option<GuardrailMode>,

    #[arg(long, env = "ZAVORA_GUARDRAIL_TERM")]
    guardrail_term: Vec<String>,

    #[arg(long, env = "ZAVORA_GUARDRAIL_REDACT_REPLACEMENT")]
    guardrail_redact_replacement: Option<String>,

    #[arg(long, env = "RUST_LOG", default_value = "warn")]
    log_filter: String,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Debug, Subcommand)]
enum Commands {
    #[command(about = "Run a one-shot prompt and print the final response")]
    Ask {
        #[arg(required = true)]
        prompt: Vec<String>,
    },
    #[command(about = "Run interactive chat mode")]
    Chat,
    #[command(about = "Run a workflow mode (single, sequential, parallel, loop) for a prompt")]
    Workflow {
        #[arg(value_enum)]
        mode: WorkflowMode,
        #[arg(required = true)]
        prompt: Vec<String>,
        #[arg(long, default_value_t = 4)]
        max_iterations: u32,
    },
    #[command(about = "Generate a release-oriented plan from a product goal")]
    ReleasePlan {
        #[arg(required = true)]
        goal: Vec<String>,
        #[arg(long, default_value_t = 3)]
        releases: u32,
    },
    #[command(about = "Validate provider environment and session backend configuration")]
    Doctor,
    #[command(about = "Run session backend migrations (sqlite only)")]
    Migrate,
    #[command(about = "Inspect profile configuration and active resolved profile state")]
    Profiles {
        #[command(subcommand)]
        command: ProfileCommands,
    },
    #[command(about = "Manage agent catalogs and active agent selection")]
    Agents {
        #[command(subcommand)]
        command: AgentCommands,
    },
    #[command(about = "Manage MCP toolset registration and discovery")]
    Mcp {
        #[command(subcommand)]
        command: McpCommands,
    },
    #[command(about = "Manage session lifecycle (list/show/delete/prune)")]
    Sessions {
        #[command(subcommand)]
        command: SessionCommands,
    },
    #[command(about = "Telemetry utilities and reporting")]
    Telemetry {
        #[command(subcommand)]
        command: TelemetryCommands,
    },
    #[command(about = "Evaluation harness and benchmark suite")]
    Eval {
        #[command(subcommand)]
        command: EvalCommands,
    },
    #[command(about = "Server mode and A2A smoke checks")]
    Server {
        #[command(subcommand)]
        command: ServerCommands,
    },
}

#[derive(Debug, Clone)]
struct RuntimeConfig {
    profile: String,
    config_path: String,
    agent_name: String,
    agent_source: AgentSource,
    agent_description: Option<String>,
    agent_instruction: Option<String>,
    agent_resource_paths: Vec<String>,
    agent_allow_tools: Vec<String>,
    agent_deny_tools: Vec<String>,
    provider: Provider,
    model: Option<String>,
    app_name: String,
    user_id: String,
    session_id: String,
    session_backend: SessionBackend,
    session_db_url: String,
    show_sensitive_config: bool,
    retrieval_backend: RetrievalBackend,
    retrieval_doc_path: Option<String>,
    retrieval_max_chunks: usize,
    retrieval_max_chars: usize,
    retrieval_min_score: usize,
    tool_confirmation_mode: ToolConfirmationMode,
    require_confirm_tool: Vec<String>,
    approve_tool: Vec<String>,
    tool_timeout_secs: u64,
    tool_retry_attempts: u32,
    tool_retry_delay_ms: u64,
    telemetry_enabled: bool,
    telemetry_path: String,
    guardrail_input_mode: GuardrailMode,
    guardrail_output_mode: GuardrailMode,
    guardrail_terms: Vec<String>,
    guardrail_redact_replacement: String,
    mcp_servers: Vec<McpServerConfig>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProfilesFile {
    #[serde(default)]
    profiles: HashMap<String, ProfileConfig>,
}

#[derive(Debug, Default, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProfileConfig {
    provider: Option<Provider>,
    model: Option<String>,
    app_name: Option<String>,
    user_id: Option<String>,
    session_id: Option<String>,
    session_backend: Option<SessionBackend>,
    session_db_url: Option<String>,
    retrieval_backend: Option<RetrievalBackend>,
    retrieval_doc_path: Option<String>,
    retrieval_max_chunks: Option<usize>,
    retrieval_max_chars: Option<usize>,
    retrieval_min_score: Option<usize>,
    tool_confirmation_mode: Option<ToolConfirmationMode>,
    #[serde(default)]
    require_confirm_tool: Vec<String>,
    #[serde(default)]
    approve_tool: Vec<String>,
    tool_timeout_secs: Option<u64>,
    tool_retry_attempts: Option<u32>,
    tool_retry_delay_ms: Option<u64>,
    telemetry_enabled: Option<bool>,
    telemetry_path: Option<String>,
    guardrail_input_mode: Option<GuardrailMode>,
    guardrail_output_mode: Option<GuardrailMode>,
    #[serde(default)]
    guardrail_terms: Vec<String>,
    guardrail_redact_replacement: Option<String>,
    #[serde(default)]
    mcp_servers: Vec<McpServerConfig>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AgentSource {
    Implicit,
    Global,
    Local,
}

impl AgentSource {
    fn label(self) -> &'static str {
        match self {
            AgentSource::Implicit => "implicit",
            AgentSource::Global => "global",
            AgentSource::Local => "local",
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct AgentFileConfig {
    description: Option<String>,
    instruction: Option<String>,
    provider: Option<Provider>,
    model: Option<String>,
    tool_confirmation_mode: Option<ToolConfirmationMode>,
    #[serde(default)]
    resource_paths: Vec<String>,
    #[serde(default)]
    allow_tools: Vec<String>,
    #[serde(default)]
    deny_tools: Vec<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct AgentCatalogFile {
    #[serde(default)]
    agents: HashMap<String, AgentFileConfig>,
}

#[derive(Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct AgentSelectionFile {
    agent: Option<String>,
}

#[derive(Debug, Clone)]
struct ResolvedAgent {
    name: String,
    source: AgentSource,
    config: AgentFileConfig,
}

#[derive(Debug, Clone)]
struct AgentPaths {
    local_catalog: PathBuf,
    global_catalog: Option<PathBuf>,
    selection_file: PathBuf,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct McpServerConfig {
    name: String,
    endpoint: String,
    enabled: Option<bool>,
    timeout_secs: Option<u64>,
    auth_bearer_env: Option<String>,
    #[serde(default)]
    tool_allowlist: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ErrorCategory {
    Provider,
    Session,
    Tooling,
    Input,
    Internal,
}

impl ErrorCategory {
    fn code(self) -> &'static str {
        match self {
            ErrorCategory::Provider => "PROVIDER",
            ErrorCategory::Session => "SESSION",
            ErrorCategory::Tooling => "TOOLING",
            ErrorCategory::Input => "INPUT",
            ErrorCategory::Internal => "INTERNAL",
        }
    }

    fn hint(self) -> &'static str {
        match self {
            ErrorCategory::Provider => {
                "Set provider credentials (for example OPENAI_API_KEY) or run with --provider ollama."
            }
            ErrorCategory::Session => {
                "Check --session-backend/--session-db-url and run migrate for sqlite sessions."
            }
            ErrorCategory::Tooling => {
                "Review tool configuration and retry with RUST_LOG=info for detailed tool/runtime logs."
            }
            ErrorCategory::Input => "Run zavora-cli --help and correct command arguments.",
            ErrorCategory::Internal => {
                "Retry with RUST_LOG=debug. If it persists, capture logs and open an issue."
            }
        }
    }
}

fn categorize_error(err: &anyhow::Error) -> ErrorCategory {
    let msg = format!("{err:#}").to_ascii_lowercase();

    if msg.contains("api_key")
        || msg.contains("no provider could be auto-detected")
        || msg.contains("provider")
    {
        return ErrorCategory::Provider;
    }

    if msg.contains("--force")
        || msg.contains("destructive")
        || msg.contains("invalid value")
        || msg.contains("unknown argument")
        || msg.contains("failed to read input")
        || msg.contains("profile")
    {
        return ErrorCategory::Input;
    }

    if msg.contains("session") || msg.contains("sqlite") || msg.contains("migrate") {
        return ErrorCategory::Session;
    }

    if msg.contains("tool") || msg.contains("mcp") || msg.contains("retrieval") {
        return ErrorCategory::Tooling;
    }

    ErrorCategory::Internal
}

fn format_cli_error(err: &anyhow::Error, show_sensitive_config: bool) -> String {
    let category = categorize_error(err);
    let rendered_error = render_error_message(err, show_sensitive_config);
    format!(
        "[{}] {}\nHint: {}",
        category.code(),
        rendered_error,
        category.hint()
    )
}

fn render_error_message(err: &anyhow::Error, show_sensitive_config: bool) -> String {
    if show_sensitive_config {
        err.to_string()
    } else {
        redact_sensitive_text(&err.to_string())
    }
}

fn redact_sensitive_text(text: &str) -> String {
    redact_sqlite_urls(text)
}

fn redact_sqlite_urls(text: &str) -> String {
    const SQLITE_PREFIX: &str = "sqlite:";
    let mut out = String::with_capacity(text.len());
    let mut cursor = 0usize;

    while let Some(offset) = text[cursor..].find(SQLITE_PREFIX) {
        let start = cursor + offset;
        out.push_str(&text[cursor..start]);

        let remainder = &text[start..];
        let end = remainder
            .find(|ch: char| {
                ch.is_whitespace()
                    || matches!(
                        ch,
                        '"' | '\'' | '(' | ')' | '[' | ']' | '{' | '}' | ',' | ';'
                    )
            })
            .unwrap_or(remainder.len());
        let token = &remainder[..end];
        out.push_str(&redact_sqlite_url_value(token));
        cursor = start + end;
    }

    out.push_str(&text[cursor..]);
    out
}

fn redact_sqlite_url_value(value: &str) -> String {
    if value.starts_with("sqlite://") {
        "sqlite://[REDACTED]".to_string()
    } else if value.starts_with("sqlite:") {
        "sqlite:[REDACTED]".to_string()
    } else {
        value.to_string()
    }
}

fn display_session_db_url(cfg: &RuntimeConfig) -> String {
    if cfg.show_sensitive_config {
        cfg.session_db_url.clone()
    } else {
        format!(
            "{} (set --show-sensitive-config to reveal)",
            redact_sqlite_url_value(&cfg.session_db_url)
        )
    }
}

fn command_label(command: &Commands) -> String {
    match command {
        Commands::Ask { .. } => "ask".to_string(),
        Commands::Chat => "chat".to_string(),
        Commands::Workflow { mode, .. } => format!("workflow.{}", workflow_mode_label(*mode)),
        Commands::ReleasePlan { .. } => "release-plan".to_string(),
        Commands::Doctor => "doctor".to_string(),
        Commands::Migrate => "migrate".to_string(),
        Commands::Profiles { command } => match command {
            ProfileCommands::List => "profiles.list".to_string(),
            ProfileCommands::Show => "profiles.show".to_string(),
        },
        Commands::Agents { command } => match command {
            AgentCommands::List => "agents.list".to_string(),
            AgentCommands::Show { .. } => "agents.show".to_string(),
            AgentCommands::Select { .. } => "agents.select".to_string(),
        },
        Commands::Mcp { command } => match command {
            McpCommands::List => "mcp.list".to_string(),
            McpCommands::Discover { .. } => "mcp.discover".to_string(),
        },
        Commands::Sessions { command } => match command {
            SessionCommands::List => "sessions.list".to_string(),
            SessionCommands::Show { .. } => "sessions.show".to_string(),
            SessionCommands::Delete { .. } => "sessions.delete".to_string(),
            SessionCommands::Prune { .. } => "sessions.prune".to_string(),
        },
        Commands::Telemetry { command } => match command {
            TelemetryCommands::Report { .. } => "telemetry.report".to_string(),
        },
        Commands::Eval { command } => match command {
            EvalCommands::Run { .. } => "eval.run".to_string(),
        },
        Commands::Server { command } => match command {
            ServerCommands::Serve { .. } => "server.serve".to_string(),
            ServerCommands::A2aSmoke => "server.a2a-smoke".to_string(),
        },
    }
}

fn workflow_mode_label(mode: WorkflowMode) -> &'static str {
    match mode {
        WorkflowMode::Single => "single",
        WorkflowMode::Sequential => "sequential",
        WorkflowMode::Parallel => "parallel",
        WorkflowMode::Loop => "loop",
        WorkflowMode::Graph => "graph",
    }
}

#[derive(Debug, Clone)]
struct TelemetrySink {
    enabled: bool,
    path: PathBuf,
    run_id: String,
    command: String,
    session_id: String,
}

impl TelemetrySink {
    fn new(cfg: &RuntimeConfig, command: String) -> Self {
        let run_id = format!("run-{}-{}", unix_ms_now(), std::process::id());
        Self {
            enabled: cfg.telemetry_enabled,
            path: PathBuf::from(&cfg.telemetry_path),
            run_id,
            command,
            session_id: cfg.session_id.clone(),
        }
    }

    fn emit(&self, event: &str, payload: Value) {
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
struct TelemetrySummary {
    total_lines: usize,
    parsed_events: usize,
    parse_errors: usize,
    unique_runs: BTreeSet<String>,
    command_counts: HashMap<String, usize>,
    command_completed: usize,
    command_failed: usize,
    tool_requested: usize,
    tool_succeeded: usize,
    tool_failed: usize,
    last_event_ts_unix_ms: Option<u128>,
}

fn summarize_telemetry_lines(lines: Vec<String>, limit: usize) -> TelemetrySummary {
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

fn run_telemetry_report(
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

fn unix_ms_now() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

const DEFAULT_EVAL_DATASET_PATH: &str = "evals/datasets/retrieval-baseline.v1.json";
const DEFAULT_EVAL_OUTPUT_PATH: &str = ".zavora/evals/latest.json";
const DEFAULT_GUARDRAIL_TERMS: &[&str] = &[
    "password",
    "secret",
    "api key",
    "api_key",
    "private key",
    "access token",
    "ssn",
    "social security",
];

#[derive(Debug, Deserialize)]
struct EvalDataset {
    name: String,
    version: String,
    #[serde(default)]
    description: String,
    cases: Vec<EvalCase>,
}

#[derive(Debug, Deserialize)]
struct EvalCase {
    id: String,
    query: String,
    chunks: Vec<String>,
    #[serde(default)]
    required_terms: Vec<String>,
    #[serde(default = "default_eval_max_chunks")]
    max_chunks: usize,
    min_term_matches: Option<usize>,
}

fn default_eval_max_chunks() -> usize {
    3
}

#[derive(Debug, Serialize)]
struct EvalCaseReport {
    id: String,
    passed: bool,
    required_terms: usize,
    matched_terms: usize,
    retrieved_chunks: usize,
    top_score: usize,
    avg_latency_ms: f64,
}

#[derive(Debug, Serialize)]
struct EvalRunReport {
    generated_at_unix_ms: u128,
    dataset_name: String,
    dataset_version: String,
    dataset_description: String,
    benchmark_iterations: usize,
    total_cases: usize,
    passed_cases: usize,
    failed_cases: usize,
    pass_rate: f64,
    fail_under: f64,
    passed_threshold: bool,
    avg_latency_ms: f64,
    p95_latency_ms: f64,
    throughput_qps: f64,
    case_reports: Vec<EvalCaseReport>,
}

fn load_eval_dataset(path: &str) -> Result<EvalDataset> {
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read eval dataset at '{}'", path))?;
    let dataset = serde_json::from_str::<EvalDataset>(&content)
        .with_context(|| format!("invalid eval dataset json at '{}'", path))?;
    if dataset.cases.is_empty() {
        return Err(anyhow::anyhow!(
            "eval dataset '{}' has no cases; add at least one case",
            path
        ));
    }
    Ok(dataset)
}

fn normalize_eval_terms(raw_terms: &[String], query: &str) -> Vec<String> {
    let mut terms = if raw_terms.is_empty() {
        query_terms(query)
    } else {
        raw_terms
            .iter()
            .map(|t| t.trim().to_ascii_lowercase())
            .filter(|t| !t.is_empty())
            .collect::<Vec<String>>()
    };

    terms.sort();
    terms.dedup();
    terms
}

fn percentile(values: &[f64], pct: f64) -> f64 {
    if values.is_empty() {
        return 0.0;
    }

    let pct = pct.clamp(0.0, 100.0);
    let rank = ((pct / 100.0) * ((values.len() - 1) as f64)).round() as usize;
    values[rank.min(values.len() - 1)]
}

fn round_metric(value: f64) -> f64 {
    (value * 1000.0).round() / 1000.0
}

fn run_eval_harness(
    dataset: &EvalDataset,
    benchmark_iterations: usize,
    fail_under: f64,
) -> Result<EvalRunReport> {
    let iterations = benchmark_iterations.max(1);
    let suite_start = Instant::now();
    let mut passed_cases = 0usize;
    let mut latency_ms = Vec::<f64>::new();
    let mut case_reports = Vec::<EvalCaseReport>::new();

    for case in &dataset.cases {
        if case.id.trim().is_empty() {
            return Err(anyhow::anyhow!("eval dataset contains case with empty id"));
        }
        if case.query.trim().is_empty() {
            return Err(anyhow::anyhow!(
                "eval case '{}' has empty query; each case must include query",
                case.id
            ));
        }
        if case.chunks.is_empty() {
            return Err(anyhow::anyhow!(
                "eval case '{}' has no chunks; each case must include retrieval corpus chunks",
                case.id
            ));
        }

        let retrieval = LocalFileRetrievalService {
            chunks: case
                .chunks
                .iter()
                .enumerate()
                .map(|(idx, chunk)| RetrievedChunk {
                    source: format!("eval:{}#{}", case.id, idx + 1),
                    text: chunk.clone(),
                    score: 0,
                })
                .collect::<Vec<RetrievedChunk>>(),
        };

        let case_start = Instant::now();
        let mut retrieved = Vec::<RetrievedChunk>::new();
        for _ in 0..iterations {
            retrieved = retrieval.retrieve(&case.query, case.max_chunks.max(1))?;
        }
        let case_elapsed = case_start.elapsed();
        let case_avg_latency_ms = (case_elapsed.as_secs_f64() * 1000.0) / (iterations as f64);
        latency_ms.push(case_avg_latency_ms);

        let terms = normalize_eval_terms(&case.required_terms, &case.query);
        if terms.is_empty() {
            return Err(anyhow::anyhow!(
                "eval case '{}' produced no required terms; add required_terms or a richer query",
                case.id
            ));
        }

        let joined = retrieved
            .iter()
            .map(|chunk| chunk.text.to_ascii_lowercase())
            .collect::<Vec<String>>()
            .join("\n");

        let matched_terms = terms
            .iter()
            .filter(|term| joined.contains(term.as_str()))
            .count();
        let required_terms = terms.len();
        let min_term_matches = case
            .min_term_matches
            .unwrap_or(required_terms)
            .clamp(1, required_terms);
        let passed = matched_terms >= min_term_matches;
        if passed {
            passed_cases += 1;
        }

        case_reports.push(EvalCaseReport {
            id: case.id.clone(),
            passed,
            required_terms,
            matched_terms,
            retrieved_chunks: retrieved.len(),
            top_score: retrieved
                .first()
                .map(|chunk| chunk.score)
                .unwrap_or_default(),
            avg_latency_ms: round_metric(case_avg_latency_ms),
        });
    }

    let total_cases = dataset.cases.len();
    let failed_cases = total_cases.saturating_sub(passed_cases);
    let pass_rate = if total_cases == 0 {
        0.0
    } else {
        passed_cases as f64 / total_cases as f64
    };

    let mut sorted_latencies = latency_ms.clone();
    sorted_latencies.sort_by(|a, b| a.total_cmp(b));
    let avg_latency_ms = if latency_ms.is_empty() {
        0.0
    } else {
        latency_ms.iter().sum::<f64>() / latency_ms.len() as f64
    };
    let p95_latency_ms = percentile(&sorted_latencies, 95.0);

    let suite_elapsed_secs = suite_start.elapsed().as_secs_f64();
    let throughput_qps = if suite_elapsed_secs <= 0.0 {
        0.0
    } else {
        (total_cases as f64 * iterations as f64) / suite_elapsed_secs
    };

    let passed_threshold = pass_rate >= fail_under.clamp(0.0, 1.0);
    Ok(EvalRunReport {
        generated_at_unix_ms: unix_ms_now(),
        dataset_name: dataset.name.clone(),
        dataset_version: dataset.version.clone(),
        dataset_description: dataset.description.clone(),
        benchmark_iterations: iterations,
        total_cases,
        passed_cases,
        failed_cases,
        pass_rate: round_metric(pass_rate),
        fail_under: round_metric(fail_under.clamp(0.0, 1.0)),
        passed_threshold,
        avg_latency_ms: round_metric(avg_latency_ms),
        p95_latency_ms: round_metric(p95_latency_ms),
        throughput_qps: round_metric(throughput_qps),
        case_reports,
    })
}

fn write_eval_report(path: &str, report: &EvalRunReport) -> Result<()> {
    let path_buf = PathBuf::from(path);
    if let Some(parent) = path_buf.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent).with_context(|| {
            format!(
                "failed to create eval report directory '{}'",
                parent.display()
            )
        })?;
    }

    let payload =
        serde_json::to_string_pretty(report).context("failed to serialize eval report to json")?;
    std::fs::write(&path_buf, payload)
        .with_context(|| format!("failed to write eval report to '{}'", path_buf.display()))
}

fn run_eval(
    dataset_path: Option<String>,
    output_path: Option<String>,
    benchmark_iterations: usize,
    fail_under: f64,
    telemetry: &TelemetrySink,
) -> Result<()> {
    let dataset_path = dataset_path.unwrap_or_else(|| DEFAULT_EVAL_DATASET_PATH.to_string());
    let output_path = output_path.unwrap_or_else(|| DEFAULT_EVAL_OUTPUT_PATH.to_string());
    let dataset = load_eval_dataset(&dataset_path)?;
    let report = run_eval_harness(&dataset, benchmark_iterations, fail_under)?;

    write_eval_report(&output_path, &report)?;
    telemetry.emit(
        "eval.completed",
        json!({
            "dataset": report.dataset_name,
            "dataset_version": report.dataset_version,
            "total_cases": report.total_cases,
            "pass_rate": report.pass_rate,
            "passed_threshold": report.passed_threshold,
            "output_path": output_path
        }),
    );

    println!(
        "Eval completed: dataset={} version={} cases={} pass_rate={:.3} threshold={:.3}",
        report.dataset_name,
        report.dataset_version,
        report.total_cases,
        report.pass_rate,
        report.fail_under
    );
    println!(
        "Benchmark: avg_latency_ms={:.3} p95_latency_ms={:.3} throughput_qps={:.3}",
        report.avg_latency_ms, report.p95_latency_ms, report.throughput_qps
    );
    println!("Report written to {}", output_path);

    if !report.passed_threshold {
        return Err(anyhow::anyhow!(
            "eval pass rate {:.3} is below threshold {:.3}",
            report.pass_rate,
            report.fail_under
        ));
    }

    Ok(())
}

fn default_guardrail_terms() -> Vec<String> {
    DEFAULT_GUARDRAIL_TERMS
        .iter()
        .map(|term| term.to_string())
        .collect::<Vec<String>>()
}

fn guardrail_mode_label(mode: GuardrailMode) -> &'static str {
    match mode {
        GuardrailMode::Disabled => "disabled",
        GuardrailMode::Observe => "observe",
        GuardrailMode::Block => "block",
        GuardrailMode::Redact => "redact",
    }
}

fn contains_guardrail_terms(text: &str, terms: &[String]) -> Vec<String> {
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

fn replace_case_insensitive(input: &str, needle: &str, replacement: &str) -> String {
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

fn redact_guardrail_terms(text: &str, hits: &[String], replacement: &str) -> String {
    let mut redacted = text.to_string();
    for hit in hits {
        redacted = replace_case_insensitive(&redacted, hit, replacement);
    }
    redacted
}

fn apply_guardrail(
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

fn buffered_output_required(mode: GuardrailMode) -> bool {
    matches!(mode, GuardrailMode::Block | GuardrailMode::Redact)
}

#[derive(Clone)]
struct ServerState {
    cfg: RuntimeConfig,
    retrieval: Arc<dyn RetrievalService>,
    telemetry: TelemetrySink,
    server_agent: Arc<dyn Agent>,
    session_service: Arc<dyn SessionService>,
    run_config: RunConfig,
    provider_label: String,
    model_name: String,
    runner_cache: Arc<tokio::sync::RwLock<HashMap<String, Arc<Runner>>>>,
}

#[derive(Debug, Serialize)]
struct ServerHealthResponse {
    status: &'static str,
    app_name: String,
    profile: String,
}

#[derive(Debug, Deserialize)]
struct ServerAskRequest {
    prompt: String,
    session_id: Option<String>,
    user_id: Option<String>,
}

#[derive(Debug, Serialize)]
struct ServerAskResponse {
    answer: String,
    provider: String,
    model: String,
    session_id: String,
    user_id: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct A2aPingRequest {
    from_agent: String,
    to_agent: String,
    message_id: String,
    correlation_id: Option<String>,
    #[serde(default)]
    payload: Value,
}

#[derive(Debug, Serialize)]
struct A2aPingResponse {
    from_agent: String,
    to_agent: String,
    message_id: String,
    correlation_id: String,
    acknowledged_message_id: String,
    status: String,
    payload: Value,
}

type ApiError = (StatusCode, Json<Value>);
type ApiResult<T> = std::result::Result<Json<T>, ApiError>;

fn api_error(status: StatusCode, message: impl Into<String>) -> ApiError {
    (status, Json(json!({ "error": message.into() })))
}

fn server_runner_cache_key(cfg: &RuntimeConfig) -> String {
    format!("{}::{}", cfg.user_id, cfg.session_id)
}

async fn get_or_build_server_runner(
    state: &ServerState,
    cfg: &RuntimeConfig,
) -> Result<(Arc<Runner>, &'static str)> {
    let key = server_runner_cache_key(cfg);
    if let Some(runner) = state.runner_cache.read().await.get(&key).cloned() {
        return Ok((runner, "hit"));
    }

    let runner = Arc::new(
        build_runner_with_session_service(
            state.server_agent.clone(),
            cfg,
            state.session_service.clone(),
            Some(state.run_config.clone()),
        )
        .await?,
    );

    let mut cache = state.runner_cache.write().await;
    if let Some(existing) = cache.get(&key).cloned() {
        return Ok((existing, "hit-race"));
    }
    cache.insert(key, runner.clone());
    Ok((runner, "miss"))
}

async fn handle_server_health(State(state): State<Arc<ServerState>>) -> Json<ServerHealthResponse> {
    Json(ServerHealthResponse {
        status: "ok",
        app_name: state.cfg.app_name.clone(),
        profile: state.cfg.profile.clone(),
    })
}

async fn handle_server_ask(
    State(state): State<Arc<ServerState>>,
    Json(request): Json<ServerAskRequest>,
) -> ApiResult<ServerAskResponse> {
    let started_at = Instant::now();
    let mut cfg = state.cfg.clone();
    if let Some(session_id) = request.session_id {
        cfg.session_id = session_id;
    }
    if let Some(user_id) = request.user_id {
        cfg.user_id = user_id;
    }

    let prompt = request.prompt.trim().to_string();
    if prompt.is_empty() {
        return Err(api_error(
            StatusCode::BAD_REQUEST,
            "prompt cannot be empty for /v1/ask",
        ));
    }

    let guarded_prompt = apply_guardrail(
        &cfg,
        &state.telemetry,
        "input",
        cfg.guardrail_input_mode,
        &prompt,
    )
    .map_err(|err| api_error(StatusCode::BAD_REQUEST, err.to_string()))?;

    let (runner, cache_status) = get_or_build_server_runner(&state, &cfg)
        .await
        .map_err(|err| api_error(StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))?;

    let answer = run_prompt_with_retrieval(
        runner.as_ref(),
        &cfg,
        &guarded_prompt,
        state.retrieval.as_ref(),
        &state.telemetry,
    )
    .await
    .map_err(|err| api_error(StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))?;
    let answer = apply_guardrail(
        &cfg,
        &state.telemetry,
        "output",
        cfg.guardrail_output_mode,
        &answer,
    )
    .map_err(|err| api_error(StatusCode::FORBIDDEN, err.to_string()))?;

    state.telemetry.emit(
        "server.ask.completed",
        json!({
            "provider": state.provider_label.clone(),
            "model": state.model_name.clone(),
            "session_id": cfg.session_id.clone(),
            "user_id": cfg.user_id.clone(),
            "runner_cache": cache_status,
            "latency_ms": round_metric(started_at.elapsed().as_secs_f64() * 1000.0)
        }),
    );

    Ok(Json(ServerAskResponse {
        answer,
        provider: state.provider_label.clone(),
        model: state.model_name.clone(),
        session_id: cfg.session_id,
        user_id: cfg.user_id,
    }))
}

fn process_a2a_ping(request: A2aPingRequest) -> Result<A2aPingResponse> {
    if request.from_agent.trim().is_empty() {
        return Err(anyhow::anyhow!("from_agent is required for A2A ping"));
    }
    if request.to_agent.trim().is_empty() {
        return Err(anyhow::anyhow!("to_agent is required for A2A ping"));
    }
    if request.message_id.trim().is_empty() {
        return Err(anyhow::anyhow!("message_id is required for A2A ping"));
    }

    let correlation_id = request
        .correlation_id
        .clone()
        .unwrap_or_else(|| request.message_id.clone());

    Ok(A2aPingResponse {
        from_agent: request.to_agent.clone(),
        to_agent: request.from_agent.clone(),
        message_id: format!("ack-{}", request.message_id),
        correlation_id,
        acknowledged_message_id: request.message_id,
        status: "acknowledged".to_string(),
        payload: json!({
            "accepted": true,
            "protocol": "zavora-a2a-v1"
        }),
    })
}

async fn handle_a2a_ping(
    State(state): State<Arc<ServerState>>,
    Json(request): Json<A2aPingRequest>,
) -> ApiResult<A2aPingResponse> {
    state.telemetry.emit(
        "a2a.ping.received",
        json!({
            "from_agent": request.from_agent.clone(),
            "to_agent": request.to_agent.clone(),
            "message_id": request.message_id.clone()
        }),
    );
    let response = process_a2a_ping(request)
        .map_err(|err| api_error(StatusCode::BAD_REQUEST, err.to_string()))?;
    state.telemetry.emit(
        "a2a.ping.responded",
        json!({
            "message_id": response.message_id,
            "status": response.status
        }),
    );
    Ok(Json(response))
}

fn build_server_router(state: Arc<ServerState>) -> AxumRouter {
    AxumRouter::new()
        .route("/healthz", get(handle_server_health))
        .route("/v1/ask", post(handle_server_ask))
        .route("/v1/a2a/ping", post(handle_a2a_ping))
        .with_state(state)
}

async fn run_server(
    cfg: RuntimeConfig,
    host: String,
    port: u16,
    telemetry: &TelemetrySink,
) -> Result<()> {
    let addr = format!("{host}:{port}")
        .parse::<SocketAddr>()
        .with_context(|| format!("invalid server bind address '{}:{}'", host, port))?;
    let retrieval = build_retrieval_service(&cfg)?;
    let runtime_tools = resolve_runtime_tools(&cfg).await;
    let tool_confirmation = resolve_tool_confirmation_settings(&cfg, &runtime_tools);
    let (model, resolved_provider, model_name) = resolve_model(&cfg)?;
    let provider_label = format!("{:?}", resolved_provider).to_ascii_lowercase();
    telemetry.emit(
        "model.resolved",
        json!({
            "provider": provider_label.clone(),
            "model": model_name.clone(),
            "path": "server"
        }),
    );
    let server_agent = build_single_agent_with_tools(
        model,
        &runtime_tools.tools,
        tool_confirmation.policy.clone(),
        Duration::from_secs(cfg.tool_timeout_secs),
        Some(&cfg),
    )?;
    let session_service = build_session_service(&cfg).await?;
    let warm_runner = Arc::new(
        build_runner_with_session_service(
            server_agent.clone(),
            &cfg,
            session_service.clone(),
            Some(tool_confirmation.run_config.clone()),
        )
        .await?,
    );
    let mut runner_cache = HashMap::new();
    runner_cache.insert(server_runner_cache_key(&cfg), warm_runner);
    let state = Arc::new(ServerState {
        cfg: cfg.clone(),
        retrieval,
        telemetry: telemetry.clone(),
        server_agent,
        session_service,
        run_config: tool_confirmation.run_config.clone(),
        provider_label: provider_label.clone(),
        model_name: model_name.clone(),
        runner_cache: Arc::new(tokio::sync::RwLock::new(runner_cache)),
    });

    telemetry.emit(
        "server.started",
        json!({
            "host": host,
            "port": port,
            "profile": cfg.profile,
            "session_backend": format!("{:?}", cfg.session_backend),
            "provider": provider_label,
            "model": model_name
        }),
    );

    println!(
        "Server mode listening on http://{} (health: /healthz, ask: /v1/ask, a2a: /v1/a2a/ping)",
        addr
    );

    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .context("failed to bind server listener")?;
    axum::serve(listener, build_server_router(state))
        .await
        .context("server runtime failed")
}

fn run_a2a_smoke(telemetry: &TelemetrySink) -> Result<()> {
    let request = A2aPingRequest {
        from_agent: "sales-agent".to_string(),
        to_agent: "procurement-agent".to_string(),
        message_id: "msg-001".to_string(),
        correlation_id: Some("corr-001".to_string()),
        payload: json!({ "intent": "supply-check" }),
    };
    let response = process_a2a_ping(request.clone())?;

    if response.acknowledged_message_id != request.message_id {
        return Err(anyhow::anyhow!(
            "a2a smoke failed: ack id '{}' does not match request id '{}'",
            response.acknowledged_message_id,
            request.message_id
        ));
    }
    if response.correlation_id != "corr-001" {
        return Err(anyhow::anyhow!(
            "a2a smoke failed: expected correlation_id corr-001 but got '{}'",
            response.correlation_id
        ));
    }

    telemetry.emit(
        "a2a.smoke.passed",
        json!({
            "from_agent": request.from_agent,
            "to_agent": request.to_agent,
            "message_id": request.message_id
        }),
    );
    println!("A2A smoke passed: basic request/ack contract is valid.");
    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let show_sensitive_config = cli.show_sensitive_config;
    if let Err(err) = run_cli(cli).await {
        eprintln!("{}", format_cli_error(&err, show_sensitive_config));
        tracing::error!(
            category = %categorize_error(&err).code(),
            error = %render_error_message(&err, show_sensitive_config),
            "command failed"
        );
        std::process::exit(1);
    }

    Ok(())
}

async fn run_cli(cli: Cli) -> Result<()> {
    init_tracing(&cli.log_filter)?;
    let profiles = load_profiles(&cli.config_path)?;
    let agent_paths = default_agent_paths();
    let resolved_agents = load_resolved_agents(&agent_paths)?;
    let selected_agent_name = load_agent_selection(&agent_paths.selection_file)?;
    let cfg = resolve_runtime_config_with_agents(
        &cli,
        &profiles,
        &resolved_agents,
        selected_agent_name.as_deref(),
    )?;
    let command = command_label(&cli.command);
    let telemetry = TelemetrySink::new(&cfg, command.clone());
    let started_at = Instant::now();
    telemetry.emit(
        "command.started",
        json!({
            "profile": cfg.profile,
            "session_backend": format!("{:?}", cfg.session_backend),
            "retrieval_backend": format!("{:?}", cfg.retrieval_backend),
            "telemetry_enabled": cfg.telemetry_enabled,
            "guardrail_input_mode": guardrail_mode_label(cfg.guardrail_input_mode),
            "guardrail_output_mode": guardrail_mode_label(cfg.guardrail_output_mode)
        }),
    );

    let retrieval_service = if matches!(
        cli.command,
        Commands::Ask { .. }
            | Commands::Chat
            | Commands::Workflow { .. }
            | Commands::ReleasePlan { .. }
    ) {
        let service = build_retrieval_service(&cfg)?;
        tracing::info!(
            backend = service.backend_name(),
            max_chunks = cfg.retrieval_max_chunks,
            max_chars = cfg.retrieval_max_chars,
            min_score = cfg.retrieval_min_score,
            "Using retrieval backend"
        );
        Some(service)
    } else {
        None
    };

    let execution: Result<()> = match cli.command {
        Commands::Ask { prompt } => {
            let (model, resolved_provider, model_name) = resolve_model(&cfg)?;
            tracing::info!(provider = ?resolved_provider, model = %model_name, "Using model");
            telemetry.emit(
                "model.resolved",
                json!({
                    "provider": format!("{:?}", resolved_provider).to_ascii_lowercase(),
                    "model": model_name,
                    "path": "ask"
                }),
            );
            let runtime_tools = resolve_runtime_tools(&cfg).await;
            let tool_confirmation = resolve_tool_confirmation_settings(&cfg, &runtime_tools);
            let agent = build_single_agent_with_tools(
                model,
                &runtime_tools.tools,
                tool_confirmation.policy,
                Duration::from_secs(cfg.tool_timeout_secs),
                Some(&cfg),
            )?;
            let runner =
                build_runner_with_run_config(agent, &cfg, Some(tool_confirmation.run_config))
                    .await?;
            let prompt = prompt.join(" ");
            let prompt =
                apply_guardrail(&cfg, &telemetry, "input", cfg.guardrail_input_mode, &prompt)?;
            let retrieval = retrieval_service
                .as_deref()
                .context("retrieval service should be initialized for ask command")?;
            let answer =
                run_prompt_with_retrieval(&runner, &cfg, &prompt, retrieval, &telemetry).await?;
            let answer = apply_guardrail(
                &cfg,
                &telemetry,
                "output",
                cfg.guardrail_output_mode,
                &answer,
            )?;
            println!("{answer}");
            Ok(())
        }
        Commands::Chat => {
            let runtime_tools = resolve_runtime_tools(&cfg).await;
            let tool_confirmation = resolve_tool_confirmation_settings(&cfg, &runtime_tools);
            let retrieval = retrieval_service
                .as_ref()
                .context("retrieval service should be initialized for chat command")?
                .clone();
            run_chat(
                cfg.clone(),
                retrieval,
                runtime_tools,
                tool_confirmation,
                &telemetry,
            )
            .await?;
            Ok(())
        }
        Commands::Workflow {
            mode,
            prompt,
            max_iterations,
        } => {
            let (model, resolved_provider, model_name) = resolve_model(&cfg)?;
            tracing::info!(provider = ?resolved_provider, model = %model_name, workflow = ?mode, "Using workflow");
            telemetry.emit(
                "model.resolved",
                json!({
                    "provider": format!("{:?}", resolved_provider).to_ascii_lowercase(),
                    "model": model_name,
                    "path": "workflow",
                    "workflow_mode": workflow_mode_label(mode)
                }),
            );
            let runtime_tools = resolve_runtime_tools(&cfg).await;
            let tool_confirmation = resolve_tool_confirmation_settings(&cfg, &runtime_tools);
            let agent = build_workflow_agent(
                mode,
                model,
                max_iterations,
                &runtime_tools.tools,
                tool_confirmation.policy,
                Duration::from_secs(cfg.tool_timeout_secs),
                Some(&cfg),
            )?;
            let runner =
                build_runner_with_run_config(agent, &cfg, Some(tool_confirmation.run_config))
                    .await?;
            let prompt = prompt.join(" ");
            let prompt =
                apply_guardrail(&cfg, &telemetry, "input", cfg.guardrail_input_mode, &prompt)?;
            let retrieval = retrieval_service
                .as_deref()
                .context("retrieval service should be initialized for workflow command")?;
            let answer =
                run_prompt_with_retrieval(&runner, &cfg, &prompt, retrieval, &telemetry).await?;
            let answer = apply_guardrail(
                &cfg,
                &telemetry,
                "output",
                cfg.guardrail_output_mode,
                &answer,
            )?;
            println!("{answer}");
            Ok(())
        }
        Commands::ReleasePlan { goal, releases } => {
            let (model, resolved_provider, model_name) = resolve_model(&cfg)?;
            tracing::info!(provider = ?resolved_provider, model = %model_name, releases, "Generating release plan");
            telemetry.emit(
                "model.resolved",
                json!({
                    "provider": format!("{:?}", resolved_provider).to_ascii_lowercase(),
                    "model": model_name,
                    "path": "release-plan"
                }),
            );
            let agent = build_release_planning_agent(model, releases)?;
            let runner = build_runner(agent, &cfg).await?;
            let prompt = goal.join(" ");
            let prompt =
                apply_guardrail(&cfg, &telemetry, "input", cfg.guardrail_input_mode, &prompt)?;
            let retrieval = retrieval_service
                .as_deref()
                .context("retrieval service should be initialized for release-plan command")?;
            let answer =
                run_prompt_with_retrieval(&runner, &cfg, &prompt, retrieval, &telemetry).await?;
            let answer = apply_guardrail(
                &cfg,
                &telemetry,
                "output",
                cfg.guardrail_output_mode,
                &answer,
            )?;
            println!("{answer}");
            Ok(())
        }
        Commands::Doctor => {
            run_doctor(&cfg).await?;
            Ok(())
        }
        Commands::Migrate => {
            run_migrate(&cfg).await?;
            Ok(())
        }
        Commands::Profiles { command } => match command {
            ProfileCommands::List => {
                run_profiles_list(&profiles, &cfg)?;
                Ok(())
            }
            ProfileCommands::Show => {
                run_profiles_show(&cfg)?;
                Ok(())
            }
        },
        Commands::Agents { command } => match command {
            AgentCommands::List => {
                run_agents_list(&resolved_agents, &cfg.agent_name, &agent_paths)?;
                Ok(())
            }
            AgentCommands::Show { name } => {
                run_agents_show(&resolved_agents, &cfg.agent_name, name)?;
                Ok(())
            }
            AgentCommands::Select { name } => {
                run_agents_select(&resolved_agents, &agent_paths, name)?;
                Ok(())
            }
        },
        Commands::Mcp { command } => match command {
            McpCommands::List => {
                run_mcp_list(&cfg).await?;
                Ok(())
            }
            McpCommands::Discover { server } => {
                run_mcp_discover(&cfg, server).await?;
                Ok(())
            }
        },
        Commands::Sessions { command } => match command {
            SessionCommands::List => {
                run_sessions_list(&cfg).await?;
                Ok(())
            }
            SessionCommands::Show { session_id, recent } => {
                run_sessions_show(&cfg, session_id, recent).await?;
                Ok(())
            }
            SessionCommands::Delete { session_id, force } => {
                run_sessions_delete(&cfg, session_id, force).await?;
                Ok(())
            }
            SessionCommands::Prune {
                keep,
                dry_run,
                force,
            } => {
                run_sessions_prune(&cfg, keep, dry_run, force).await?;
                Ok(())
            }
        },
        Commands::Telemetry { command } => match command {
            TelemetryCommands::Report { path, limit } => {
                run_telemetry_report(&cfg, path, limit)?;
                Ok(())
            }
        },
        Commands::Eval { command } => match command {
            EvalCommands::Run {
                dataset,
                output,
                benchmark_iterations,
                fail_under,
            } => {
                run_eval(
                    dataset,
                    output,
                    benchmark_iterations,
                    fail_under,
                    &telemetry,
                )?;
                Ok(())
            }
        },
        Commands::Server { command } => match command {
            ServerCommands::Serve { host, port } => {
                run_server(cfg.clone(), host, port, &telemetry).await?;
                Ok(())
            }
            ServerCommands::A2aSmoke => {
                run_a2a_smoke(&telemetry)?;
                Ok(())
            }
        },
    };

    let duration_ms = started_at.elapsed().as_millis();
    match &execution {
        Ok(_) => telemetry.emit(
            "command.completed",
            json!({"duration_ms": duration_ms, "status": "ok"}),
        ),
        Err(err) => telemetry.emit(
            "command.failed",
            json!({
                "duration_ms": duration_ms,
                "status": "error",
                "error": render_error_message(err, cfg.show_sensitive_config)
            }),
        ),
    }

    execution
}

fn load_profiles(config_path: &str) -> Result<ProfilesFile> {
    let path = Path::new(config_path);
    if !path.exists() {
        return Ok(ProfilesFile::default());
    }

    let content = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read profile config file at '{}'", path.display()))?;
    toml::from_str::<ProfilesFile>(&content).with_context(|| {
        format!(
            "invalid profile configuration in '{}'. Check provider/session values and field names.",
            path.display()
        )
    })
}

fn default_agent_paths() -> AgentPaths {
    let local_catalog = PathBuf::from(".zavora/agents.toml");
    let selection_file = PathBuf::from(".zavora/agent-selection.toml");
    let global_catalog = std::env::var("HOME")
        .ok()
        .map(PathBuf::from)
        .map(|home| home.join(".zavora/agents.toml"));
    AgentPaths {
        local_catalog,
        global_catalog,
        selection_file,
    }
}

fn load_agent_catalog_file(path: &Path) -> Result<AgentCatalogFile> {
    if !path.exists() {
        return Ok(AgentCatalogFile::default());
    }

    let content = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read agent catalog file at '{}'", path.display()))?;
    toml::from_str::<AgentCatalogFile>(&content).with_context(|| {
        format!(
            "invalid agent catalog configuration in '{}'. Check field names and provider/tool settings.",
            path.display()
        )
    })
}

fn load_resolved_agents(paths: &AgentPaths) -> Result<HashMap<String, ResolvedAgent>> {
    let mut resolved = implicit_agent_map();

    if let Some(global_path) = paths.global_catalog.as_ref() {
        let global = load_agent_catalog_file(global_path)?;
        for (name, config) in global.agents {
            resolved.insert(
                name.clone(),
                ResolvedAgent {
                    name,
                    source: AgentSource::Global,
                    config,
                },
            );
        }
    }

    let local = load_agent_catalog_file(&paths.local_catalog)?;
    for (name, config) in local.agents {
        resolved.insert(
            name.clone(),
            ResolvedAgent {
                name,
                source: AgentSource::Local,
                config,
            },
        );
    }

    Ok(resolved)
}

fn implicit_agent_map() -> HashMap<String, ResolvedAgent> {
    let mut resolved = HashMap::<String, ResolvedAgent>::new();
    resolved.insert(
        "default".to_string(),
        ResolvedAgent {
            name: "default".to_string(),
            source: AgentSource::Implicit,
            config: AgentFileConfig {
                description: Some("Built-in default assistant".to_string()),
                instruction: None,
                provider: None,
                model: None,
                tool_confirmation_mode: None,
                resource_paths: Vec::new(),
                allow_tools: Vec::new(),
                deny_tools: Vec::new(),
            },
        },
    );
    resolved
}

fn load_agent_selection(path: &Path) -> Result<Option<String>> {
    if !path.exists() {
        return Ok(None);
    }
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read agent selection file '{}'", path.display()))?;
    let parsed = toml::from_str::<AgentSelectionFile>(&content)
        .with_context(|| format!("invalid agent selection config '{}'", path.display()))?;
    Ok(parsed
        .agent
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty()))
}

fn resolve_active_agent_name(
    cli: &Cli,
    agents: &HashMap<String, ResolvedAgent>,
    selected_agent: Option<&str>,
) -> Result<String> {
    if let Some(requested) = cli.agent.as_deref() {
        let trimmed = requested.trim();
        if agents.contains_key(trimmed) {
            return Ok(trimmed.to_string());
        }
        let mut names = agents.keys().cloned().collect::<Vec<String>>();
        names.sort();
        return Err(anyhow::anyhow!(
            "agent '{}' not found. Available agents: {}",
            trimmed,
            names.join(", ")
        ));
    }

    if let Some(selected) = selected_agent
        .map(str::trim)
        .filter(|value| !value.is_empty())
        && agents.contains_key(selected)
    {
        return Ok(selected.to_string());
    }

    if agents.contains_key("default") {
        return Ok("default".to_string());
    }

    let mut names = agents.keys().cloned().collect::<Vec<String>>();
    names.sort();
    names.into_iter().next().ok_or_else(|| {
        anyhow::anyhow!(
            "no agents available. Add '.zavora/agents.toml' or '~/.zavora/agents.toml'."
        )
    })
}

fn persist_agent_selection(path: &Path, agent_name: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).with_context(|| {
            format!(
                "failed to create agent selection directory '{}'",
                parent.display()
            )
        })?;
    }
    let payload = toml::to_string(&AgentSelectionFile {
        agent: Some(agent_name.to_string()),
    })
    .context("failed to serialize agent selection file")?;
    std::fs::write(path, payload)
        .with_context(|| format!("failed to write agent selection file '{}'", path.display()))
}

fn merge_unique_names(first: &[String], second: &[String]) -> Vec<String> {
    let mut seen = BTreeSet::<String>::new();
    let mut merged = Vec::<String>::new();

    for name in first.iter().chain(second.iter()) {
        let trimmed = name.trim();
        if trimmed.is_empty() {
            continue;
        }
        if seen.insert(trimmed.to_string()) {
            merged.push(trimmed.to_string());
        }
    }

    merged
}

fn resolve_runtime_config_with_agents(
    cli: &Cli,
    profiles: &ProfilesFile,
    resolved_agents: &HashMap<String, ResolvedAgent>,
    selected_agent_name: Option<&str>,
) -> Result<RuntimeConfig> {
    let selected = cli.profile.trim();
    if selected.is_empty() {
        return Err(anyhow::anyhow!(
            "profile name cannot be empty. Set --profile <name>."
        ));
    }

    let profile = if selected == "default" && !profiles.profiles.contains_key("default") {
        ProfileConfig::default()
    } else {
        profiles.profiles.get(selected).cloned().ok_or_else(|| {
            let mut names = profiles.profiles.keys().cloned().collect::<Vec<String>>();
            names.sort();
            if names.is_empty() {
                anyhow::anyhow!(
                    "profile '{}' not found in '{}'. No profiles are defined yet.",
                    selected,
                    cli.config_path
                )
            } else {
                anyhow::anyhow!(
                    "profile '{}' not found in '{}'. Available profiles: {}",
                    selected,
                    cli.config_path,
                    names.join(", ")
                )
            }
        })?
    };

    let active_agent_name = resolve_active_agent_name(cli, resolved_agents, selected_agent_name)?;
    let active_agent = resolved_agents.get(&active_agent_name).ok_or_else(|| {
        anyhow::anyhow!("resolved active agent '{}' is missing", active_agent_name)
    })?;

    let provider = if cli.provider != Provider::Auto {
        cli.provider
    } else {
        active_agent
            .config
            .provider
            .or(profile.provider)
            .unwrap_or(Provider::Auto)
    };

    let require_confirm_tool =
        merge_unique_names(&profile.require_confirm_tool, &cli.require_confirm_tool);
    let approve_tool = merge_unique_names(&profile.approve_tool, &cli.approve_tool);
    let guardrail_terms = {
        let merged = merge_unique_names(&profile.guardrail_terms, &cli.guardrail_term);
        if merged.is_empty() {
            default_guardrail_terms()
        } else {
            merged
        }
    };
    let mcp_servers = profile.mcp_servers.clone();

    Ok(RuntimeConfig {
        profile: selected.to_string(),
        config_path: cli.config_path.clone(),
        agent_name: active_agent.name.clone(),
        agent_source: active_agent.source,
        agent_description: active_agent.config.description.clone(),
        agent_instruction: active_agent.config.instruction.clone(),
        agent_resource_paths: active_agent.config.resource_paths.clone(),
        agent_allow_tools: active_agent.config.allow_tools.clone(),
        agent_deny_tools: active_agent.config.deny_tools.clone(),
        provider,
        model: cli
            .model
            .clone()
            .or(active_agent.config.model.clone())
            .or(profile.model),
        app_name: cli
            .app_name
            .clone()
            .or(profile.app_name)
            .unwrap_or_else(|| "zavora-cli".to_string()),
        user_id: cli
            .user_id
            .clone()
            .or(profile.user_id)
            .unwrap_or_else(|| "local-user".to_string()),
        session_id: cli
            .session_id
            .clone()
            .or(profile.session_id)
            .unwrap_or_else(|| "default-session".to_string()),
        session_backend: cli
            .session_backend
            .or(profile.session_backend)
            .unwrap_or(SessionBackend::Memory),
        session_db_url: cli
            .session_db_url
            .clone()
            .or(profile.session_db_url)
            .unwrap_or_else(|| "sqlite://.zavora/sessions.db".to_string()),
        show_sensitive_config: cli.show_sensitive_config,
        retrieval_backend: cli
            .retrieval_backend
            .or(profile.retrieval_backend)
            .unwrap_or(RetrievalBackend::Disabled),
        retrieval_doc_path: cli
            .retrieval_doc_path
            .clone()
            .or(profile.retrieval_doc_path),
        retrieval_max_chunks: cli
            .retrieval_max_chunks
            .or(profile.retrieval_max_chunks)
            .unwrap_or(3)
            .max(1),
        retrieval_max_chars: cli
            .retrieval_max_chars
            .or(profile.retrieval_max_chars)
            .unwrap_or(4000)
            .max(256),
        retrieval_min_score: cli
            .retrieval_min_score
            .or(profile.retrieval_min_score)
            .unwrap_or(1),
        tool_confirmation_mode: cli
            .tool_confirmation_mode
            .or(active_agent.config.tool_confirmation_mode)
            .or(profile.tool_confirmation_mode)
            .unwrap_or(ToolConfirmationMode::McpOnly),
        require_confirm_tool,
        approve_tool,
        tool_timeout_secs: cli
            .tool_timeout_secs
            .or(profile.tool_timeout_secs)
            .unwrap_or(45)
            .max(1),
        tool_retry_attempts: cli
            .tool_retry_attempts
            .or(profile.tool_retry_attempts)
            .unwrap_or(2)
            .max(1),
        tool_retry_delay_ms: cli
            .tool_retry_delay_ms
            .or(profile.tool_retry_delay_ms)
            .unwrap_or(500),
        telemetry_enabled: cli
            .telemetry_enabled
            .or(profile.telemetry_enabled)
            .unwrap_or(true),
        telemetry_path: cli
            .telemetry_path
            .clone()
            .or(profile.telemetry_path)
            .unwrap_or_else(|| ".zavora/telemetry/events.jsonl".to_string()),
        guardrail_input_mode: cli
            .guardrail_input_mode
            .or(profile.guardrail_input_mode)
            .unwrap_or(GuardrailMode::Disabled),
        guardrail_output_mode: cli
            .guardrail_output_mode
            .or(profile.guardrail_output_mode)
            .unwrap_or(GuardrailMode::Disabled),
        guardrail_terms,
        guardrail_redact_replacement: cli
            .guardrail_redact_replacement
            .clone()
            .or(profile.guardrail_redact_replacement)
            .unwrap_or_else(|| "[REDACTED]".to_string()),
        mcp_servers,
    })
}

#[cfg(test)]
fn resolve_runtime_config(cli: &Cli, profiles: &ProfilesFile) -> Result<RuntimeConfig> {
    let resolved_agents = implicit_agent_map();
    resolve_runtime_config_with_agents(cli, profiles, &resolved_agents, None)
}

fn run_profiles_list(profiles: &ProfilesFile, cfg: &RuntimeConfig) -> Result<()> {
    let mut names = profiles.profiles.keys().cloned().collect::<Vec<String>>();
    if !names.iter().any(|name| name == "default") {
        names.push("default".to_string());
    }
    names.sort();

    println!("Configured profiles (active='{}'):", cfg.profile);
    for name in names {
        let marker = if name == cfg.profile { "*" } else { " " };
        let source = if profiles.profiles.contains_key(&name) {
            "configured"
        } else {
            "implicit"
        };
        println!("{marker} {name} ({source})");
    }

    Ok(())
}

fn run_profiles_show(cfg: &RuntimeConfig) -> Result<()> {
    println!("Active profile: {}", cfg.profile);
    println!("Config path: {}", cfg.config_path);
    println!("Provider: {:?}", cfg.provider);
    println!(
        "Model: {}",
        cfg.model.as_deref().unwrap_or("<provider-default>")
    );
    println!("App: {}", cfg.app_name);
    println!("User: {}", cfg.user_id);
    println!(
        "Agent: {} (source={})",
        cfg.agent_name,
        cfg.agent_source.label()
    );
    println!(
        "Agent description: {}",
        cfg.agent_description.as_deref().unwrap_or("<none>")
    );
    println!(
        "Agent resources: {}",
        if cfg.agent_resource_paths.is_empty() {
            "<none>".to_string()
        } else {
            cfg.agent_resource_paths.join(", ")
        }
    );
    println!("Session ID: {}", cfg.session_id);
    println!("Session backend: {:?}", cfg.session_backend);
    println!("Session DB URL: {}", display_session_db_url(cfg));
    println!("Retrieval backend: {:?}", cfg.retrieval_backend);
    println!(
        "Retrieval doc path: {}",
        cfg.retrieval_doc_path
            .as_deref()
            .unwrap_or("<not configured>")
    );
    println!("Retrieval max chunks: {}", cfg.retrieval_max_chunks);
    println!("Retrieval max chars: {}", cfg.retrieval_max_chars);
    println!("Retrieval min score: {}", cfg.retrieval_min_score);
    println!("Tool confirmation mode: {:?}", cfg.tool_confirmation_mode);
    println!(
        "Tool confirmation required list: {}",
        if cfg.require_confirm_tool.is_empty() {
            "<none>".to_string()
        } else {
            cfg.require_confirm_tool.join(", ")
        }
    );
    println!(
        "Tool approval list: {}",
        if cfg.approve_tool.is_empty() {
            "<none>".to_string()
        } else {
            cfg.approve_tool.join(", ")
        }
    );
    println!("Tool timeout (secs): {}", cfg.tool_timeout_secs);
    println!("Tool retry attempts: {}", cfg.tool_retry_attempts);
    println!("Tool retry delay (ms): {}", cfg.tool_retry_delay_ms);
    println!("Telemetry enabled: {}", cfg.telemetry_enabled);
    println!("Telemetry path: {}", cfg.telemetry_path);
    println!(
        "Guardrails: input_mode={:?} output_mode={:?} terms={} redact_replacement={}",
        cfg.guardrail_input_mode,
        cfg.guardrail_output_mode,
        cfg.guardrail_terms.len(),
        cfg.guardrail_redact_replacement
    );
    println!("MCP servers: {}", cfg.mcp_servers.len());
    Ok(())
}

fn run_agents_list(
    agents: &HashMap<String, ResolvedAgent>,
    active_agent: &str,
    paths: &AgentPaths,
) -> Result<()> {
    let mut names = agents.keys().cloned().collect::<Vec<String>>();
    names.sort();

    println!("Available agents (active='{}'):", active_agent);
    for name in names {
        let marker = if name == active_agent { "*" } else { " " };
        let source = agents
            .get(&name)
            .map(|agent| agent.source.label())
            .unwrap_or("unknown");
        println!("{marker} {name} ({source})");
    }
    println!("Local catalog: {}", paths.local_catalog.display());
    if let Some(global) = paths.global_catalog.as_ref() {
        println!("Global catalog: {}", global.display());
    } else {
        println!("Global catalog: <HOME not set>");
    }
    println!("Selection file: {}", paths.selection_file.display());
    Ok(())
}

fn run_agents_show(
    agents: &HashMap<String, ResolvedAgent>,
    active_agent: &str,
    requested_name: Option<String>,
) -> Result<()> {
    let name = requested_name.unwrap_or_else(|| active_agent.to_string());
    let agent = agents.get(&name).ok_or_else(|| {
        let mut names = agents.keys().cloned().collect::<Vec<String>>();
        names.sort();
        anyhow::anyhow!(
            "agent '{}' not found. Available agents: {}",
            name,
            names.join(", ")
        )
    })?;

    println!("Agent: {} (source={})", agent.name, agent.source.label());
    println!(
        "Description: {}",
        agent.config.description.as_deref().unwrap_or("<none>")
    );
    println!(
        "Instruction: {}",
        agent.config.instruction.as_deref().unwrap_or("<none>")
    );
    println!(
        "Provider override: {}",
        agent
            .config
            .provider
            .map(|p| format!("{:?}", p))
            .unwrap_or_else(|| "<none>".to_string())
    );
    println!(
        "Model override: {}",
        agent.config.model.as_deref().unwrap_or("<none>")
    );
    println!(
        "Tool confirmation mode override: {}",
        agent
            .config
            .tool_confirmation_mode
            .map(|mode| format!("{:?}", mode))
            .unwrap_or_else(|| "<none>".to_string())
    );
    println!(
        "Allow tools: {}",
        if agent.config.allow_tools.is_empty() {
            "<none>".to_string()
        } else {
            agent.config.allow_tools.join(", ")
        }
    );
    println!(
        "Deny tools: {}",
        if agent.config.deny_tools.is_empty() {
            "<none>".to_string()
        } else {
            agent.config.deny_tools.join(", ")
        }
    );
    println!(
        "Resource paths: {}",
        if agent.config.resource_paths.is_empty() {
            "<none>".to_string()
        } else {
            agent.config.resource_paths.join(", ")
        }
    );
    Ok(())
}

fn run_agents_select(
    agents: &HashMap<String, ResolvedAgent>,
    paths: &AgentPaths,
    name: String,
) -> Result<()> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err(anyhow::anyhow!("agent name cannot be empty"));
    }
    if !agents.contains_key(trimmed) {
        let mut names = agents.keys().cloned().collect::<Vec<String>>();
        names.sort();
        return Err(anyhow::anyhow!(
            "agent '{}' not found. Available agents: {}",
            trimmed,
            names.join(", ")
        ));
    }
    persist_agent_selection(&paths.selection_file, trimmed)?;
    println!(
        "Selected agent '{}' (selection file: {}).",
        trimmed,
        paths.selection_file.display()
    );
    Ok(())
}

#[derive(Debug)]
struct McpDiscoveryContext {
    user_content: Content,
}

impl Default for McpDiscoveryContext {
    fn default() -> Self {
        Self {
            user_content: Content::new("user").with_text("discover mcp tools"),
        }
    }
}

impl ReadonlyContext for McpDiscoveryContext {
    fn invocation_id(&self) -> &str {
        "mcp-discovery"
    }
    fn agent_name(&self) -> &str {
        "mcp-manager"
    }
    fn user_id(&self) -> &str {
        "local-user"
    }
    fn app_name(&self) -> &str {
        "zavora-cli"
    }
    fn session_id(&self) -> &str {
        "mcp-discovery"
    }
    fn branch(&self) -> &str {
        "main"
    }
    fn user_content(&self) -> &Content {
        &self.user_content
    }
}

fn select_mcp_servers(
    cfg: &RuntimeConfig,
    server_name: Option<&str>,
) -> Result<Vec<McpServerConfig>> {
    let active = cfg
        .mcp_servers
        .iter()
        .filter(|server| server.enabled.unwrap_or(true))
        .cloned()
        .collect::<Vec<McpServerConfig>>();

    if let Some(name) = server_name {
        let server = active
            .into_iter()
            .find(|server| server.name == name)
            .ok_or_else(|| anyhow::anyhow!("MCP server '{}' not found or not enabled", name))?;
        return Ok(vec![server]);
    }

    Ok(active)
}

fn resolve_mcp_auth(server: &McpServerConfig) -> Result<Option<McpAuth>> {
    let Some(env_key) = server.auth_bearer_env.as_deref() else {
        return Ok(None);
    };

    let token = std::env::var(env_key).with_context(|| {
        format!(
            "MCP server '{}' requires bearer token env '{}' but it is missing",
            server.name, env_key
        )
    })?;

    if token.trim().is_empty() {
        return Err(anyhow::anyhow!(
            "MCP server '{}' has empty bearer token from env '{}'",
            server.name,
            env_key
        ));
    }

    Ok(Some(McpAuth::bearer(token)))
}

async fn discover_mcp_tools_for_server(
    server: &McpServerConfig,
    retry_attempts: u32,
    retry_delay_ms: u64,
) -> Result<Vec<Arc<dyn Tool>>> {
    let mut builder = McpHttpClientBuilder::new(server.endpoint.clone())
        .timeout(Duration::from_secs(server.timeout_secs.unwrap_or(15)));
    if let Some(auth) = resolve_mcp_auth(server)? {
        builder = builder.with_auth(auth);
    }

    let mut toolset = builder
        .connect()
        .await
        .with_context(|| {
            format!(
                "failed to connect to MCP server '{}' at {}",
                server.name, server.endpoint
            )
        })?
        .with_name(format!("mcp:{}", server.name));

    toolset = toolset.with_refresh_config(
        RefreshConfig::default()
            .with_max_attempts(retry_attempts.max(1))
            .with_retry_delay_ms(retry_delay_ms),
    );

    if !server.tool_allowlist.is_empty() {
        let allowed = server.tool_allowlist.clone();
        toolset = toolset.with_filter(move |tool_name| {
            allowed.iter().any(|allowed_name| allowed_name == tool_name)
        });
    }

    let ctx: Arc<dyn ReadonlyContext> = Arc::new(McpDiscoveryContext::default());
    toolset.tools(ctx).await.with_context(|| {
        format!(
            "failed to discover MCP tools from '{}' ({})",
            server.name, server.endpoint
        )
    })
}

async fn discover_mcp_tools(cfg: &RuntimeConfig) -> Vec<Arc<dyn Tool>> {
    let mut all_tools = Vec::<Arc<dyn Tool>>::new();
    let servers = match select_mcp_servers(cfg, None) {
        Ok(servers) => servers,
        Err(err) => {
            tracing::warn!(error = %err, "MCP server selection failed");
            return all_tools;
        }
    };

    for server in servers {
        match discover_mcp_tools_for_server(
            &server,
            cfg.tool_retry_attempts,
            cfg.tool_retry_delay_ms,
        )
        .await
        {
            Ok(mut tools) => {
                tracing::info!(
                    server = %server.name,
                    endpoint = %server.endpoint,
                    tools = tools.len(),
                    "MCP tools discovered"
                );
                all_tools.append(&mut tools);
            }
            Err(err) => {
                tracing::warn!(
                    server = %server.name,
                    endpoint = %server.endpoint,
                    error = %err,
                    "MCP server unavailable; continuing without its tools"
                );
            }
        }
    }

    all_tools
}

async fn run_mcp_list(cfg: &RuntimeConfig) -> Result<()> {
    let servers = select_mcp_servers(cfg, None)?;
    if servers.is_empty() {
        println!(
            "No enabled MCP servers configured for profile '{}'.",
            cfg.profile
        );
        return Ok(());
    }

    println!("Enabled MCP servers for profile '{}':", cfg.profile);
    println!(
        "Runtime MCP reliability policy: retry_attempts={} retry_delay_ms={}",
        cfg.tool_retry_attempts, cfg.tool_retry_delay_ms
    );
    for server in servers {
        let auth = server.auth_bearer_env.as_deref().unwrap_or("<none>");
        let allowlist = if server.tool_allowlist.is_empty() {
            "<all>".to_string()
        } else {
            server.tool_allowlist.join(",")
        };
        println!(
            "- {} endpoint={} timeout={}s auth_env={} allowlist={}",
            server.name,
            server.endpoint,
            server.timeout_secs.unwrap_or(15),
            auth,
            allowlist
        );
    }

    Ok(())
}

async fn run_mcp_discover(cfg: &RuntimeConfig, server_name: Option<String>) -> Result<()> {
    let servers = select_mcp_servers(cfg, server_name.as_deref())?;
    if servers.is_empty() {
        println!("No enabled MCP servers configured for discovery.");
        return Ok(());
    }

    let mut failures = 0usize;
    for server in servers {
        match discover_mcp_tools_for_server(
            &server,
            cfg.tool_retry_attempts,
            cfg.tool_retry_delay_ms,
        )
        .await
        {
            Ok(tools) => {
                println!(
                    "MCP server '{}' reachable. Discovered {} tool(s):",
                    server.name,
                    tools.len()
                );
                for tool in tools {
                    println!("- {}", tool.name());
                }
            }
            Err(err) => {
                failures += 1;
                eprintln!(
                    "[TOOLING] MCP discovery failed for '{}' ({}): {}",
                    server.name, server.endpoint, err
                );
            }
        }
    }

    if failures > 0 {
        return Err(anyhow::anyhow!(
            "MCP discovery completed with {} failure(s). Check endpoint/auth and retry.",
            failures
        ));
    }

    Ok(())
}

#[derive(Debug, Clone)]
struct RetrievedChunk {
    source: String,
    text: String,
    score: usize,
}

trait RetrievalService: Send + Sync {
    fn backend_name(&self) -> &'static str;
    fn retrieve(&self, query: &str, max_chunks: usize) -> Result<Vec<RetrievedChunk>>;
}

struct DisabledRetrievalService;

impl RetrievalService for DisabledRetrievalService {
    fn backend_name(&self) -> &'static str {
        "disabled"
    }

    fn retrieve(&self, _query: &str, _max_chunks: usize) -> Result<Vec<RetrievedChunk>> {
        Ok(Vec::new())
    }
}

struct LocalFileRetrievalService {
    chunks: Vec<RetrievedChunk>,
}

fn load_retrieval_chunks(path: &str, source_prefix: &str) -> Result<Vec<RetrievedChunk>> {
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read retrieval doc at '{}'", path))?;
    let chunks = content
        .split("\n\n")
        .map(str::trim)
        .filter(|chunk| !chunk.is_empty())
        .enumerate()
        .map(|(index, text)| RetrievedChunk {
            source: format!("{source_prefix}:{path}#{}", index + 1),
            text: text.to_string(),
            score: 0,
        })
        .collect::<Vec<RetrievedChunk>>();
    Ok(chunks)
}

fn query_terms(query: &str) -> Vec<String> {
    query
        .split_whitespace()
        .map(|token| token.trim_matches(|c: char| !c.is_ascii_alphanumeric()))
        .map(str::to_ascii_lowercase)
        .filter(|token| token.len() > 2)
        .collect::<Vec<String>>()
}

impl LocalFileRetrievalService {
    fn load(path: &str) -> Result<Self> {
        Ok(Self {
            chunks: load_retrieval_chunks(path, "local")?,
        })
    }
}

impl RetrievalService for LocalFileRetrievalService {
    fn backend_name(&self) -> &'static str {
        "local"
    }

    fn retrieve(&self, query: &str, max_chunks: usize) -> Result<Vec<RetrievedChunk>> {
        let terms = query_terms(query);

        if terms.is_empty() {
            return Ok(Vec::new());
        }

        let mut scored = self
            .chunks
            .iter()
            .filter_map(|chunk| {
                let body = chunk.text.to_ascii_lowercase();
                let score = terms
                    .iter()
                    .map(|term| body.matches(term.as_str()).count())
                    .sum::<usize>();
                (score > 0).then_some(RetrievedChunk {
                    source: chunk.source.clone(),
                    text: chunk.text.clone(),
                    score,
                })
            })
            .collect::<Vec<RetrievedChunk>>();

        scored.sort_by_key(|chunk| std::cmp::Reverse(chunk.score));
        scored.truncate(max_chunks.max(1));
        Ok(scored)
    }
}

#[cfg(feature = "semantic-search")]
struct SemanticLocalRetrievalService {
    chunks: Vec<RetrievedChunk>,
}

#[cfg(feature = "semantic-search")]
impl SemanticLocalRetrievalService {
    fn load(path: &str) -> Result<Self> {
        Ok(Self {
            chunks: load_retrieval_chunks(path, "semantic")?,
        })
    }
}

#[cfg(feature = "semantic-search")]
impl RetrievalService for SemanticLocalRetrievalService {
    fn backend_name(&self) -> &'static str {
        "semantic"
    }

    fn retrieve(&self, query: &str, max_chunks: usize) -> Result<Vec<RetrievedChunk>> {
        let query_lower = query.to_ascii_lowercase();
        let terms = query_terms(query);
        if query_lower.trim().is_empty() {
            return Ok(Vec::new());
        }

        let mut scored = self
            .chunks
            .iter()
            .filter_map(|chunk| {
                let body = chunk.text.to_ascii_lowercase();
                let similarity = strsim::jaro_winkler(&query_lower, &body);
                let lexical_hits = terms
                    .iter()
                    .map(|term| body.matches(term.as_str()).count())
                    .sum::<usize>();
                let score = ((similarity * 1000.0) as usize) + (lexical_hits * 25);
                (score > 0).then_some(RetrievedChunk {
                    source: chunk.source.clone(),
                    text: chunk.text.clone(),
                    score,
                })
            })
            .collect::<Vec<RetrievedChunk>>();

        scored.sort_by_key(|chunk| std::cmp::Reverse(chunk.score));
        scored.truncate(max_chunks.max(1));
        Ok(scored)
    }
}

fn build_retrieval_service(cfg: &RuntimeConfig) -> Result<Arc<dyn RetrievalService>> {
    match cfg.retrieval_backend {
        RetrievalBackend::Disabled => Ok(Arc::new(DisabledRetrievalService)),
        RetrievalBackend::Local => {
            let path = cfg.retrieval_doc_path.as_deref().ok_or_else(|| {
                anyhow::anyhow!(
                    "retrieval backend 'local' requires --retrieval-doc-path or profile.retrieval_doc_path"
                )
            })?;
            let service = LocalFileRetrievalService::load(path)?;
            Ok(Arc::new(service))
        }
        RetrievalBackend::Semantic => {
            let path = cfg.retrieval_doc_path.as_deref().ok_or_else(|| {
                anyhow::anyhow!(
                    "retrieval backend 'semantic' requires --retrieval-doc-path or profile.retrieval_doc_path"
                )
            })?;

            #[cfg(feature = "semantic-search")]
            {
                let service = SemanticLocalRetrievalService::load(path)?;
                return Ok(Arc::new(service));
            }

            #[cfg(not(feature = "semantic-search"))]
            {
                let _ = path;
                Err(anyhow::anyhow!(
                    "retrieval backend 'semantic' requires feature 'semantic-search'. Rebuild with: cargo run --features semantic-search -- ..."
                ))
            }
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct RetrievalPolicy {
    max_chunks: usize,
    max_chars: usize,
    min_score: usize,
}

fn augment_prompt_with_retrieval(
    retrieval: &dyn RetrievalService,
    prompt: &str,
    policy: RetrievalPolicy,
) -> Result<String> {
    let chunks = retrieval.retrieve(prompt, policy.max_chunks)?;
    let mut used_chars = 0usize;
    let mut filtered = Vec::new();

    for chunk in chunks {
        if chunk.score < policy.min_score {
            continue;
        }

        if used_chars >= policy.max_chars {
            break;
        }

        let remaining = policy.max_chars - used_chars;
        if remaining == 0 {
            break;
        }

        let mut text = chunk.text;
        if text.len() > remaining {
            text.truncate(remaining);
        }

        if text.trim().is_empty() {
            continue;
        }

        used_chars += text.len();
        filtered.push(RetrievedChunk {
            source: chunk.source,
            text,
            score: chunk.score,
        });
    }

    if filtered.is_empty() {
        return Ok(prompt.to_string());
    }

    let mut out = String::new();
    out.push_str("Retrieved context (use if relevant):\n");
    for (index, chunk) in filtered.iter().enumerate() {
        out.push_str(&format!(
            "[{}] {} (score={})\n{}\n",
            index + 1,
            chunk.source,
            chunk.score,
            chunk.text
        ));
    }
    out.push_str("\nUser request:\n");
    out.push_str(prompt);
    Ok(out)
}

fn init_tracing(log_filter: &str) -> Result<()> {
    let level = log_filter
        .parse::<LevelFilter>()
        .unwrap_or(LevelFilter::INFO);
    tracing_subscriber::fmt()
        .with_max_level(level)
        .with_env_filter(log_filter)
        .with_target(false)
        .try_init()
        .map_err(|e| anyhow::anyhow!("failed to initialize tracing subscriber: {e}"))
}

const FS_READ_DEFAULT_MAX_BYTES: usize = 8192;
const FS_READ_MAX_BYTES_LIMIT: usize = 65536;
const FS_READ_DEFAULT_MAX_LINES: usize = 200;
const FS_READ_MAX_LINES_LIMIT: usize = 2000;
const FS_READ_DEFAULT_MAX_ENTRIES: usize = 100;
const FS_READ_MAX_ENTRIES_LIMIT: usize = 500;
const FS_READ_DENIED_SEGMENTS: &[&str] = &[".git", ".zavora"];
const FS_READ_DENIED_FILE_NAMES: &[&str] =
    &[".env", ".env.local", ".env.development", ".env.production"];
const FS_WRITE_TOOL_NAME: &str = "fs_write";
const EXECUTE_BASH_TOOL_NAME: &str = "execute_bash";
const GITHUB_OPS_TOOL_NAME: &str = "github_ops";
const EXECUTE_BASH_DEFAULT_TIMEOUT_SECS: u64 = 20;
const EXECUTE_BASH_DEFAULT_RETRY_ATTEMPTS: u32 = 1;
const EXECUTE_BASH_DEFAULT_RETRY_DELAY_MS: u64 = 250;
const EXECUTE_BASH_DEFAULT_MAX_OUTPUT_CHARS: usize = 8000;
const EXECUTE_BASH_MAX_OUTPUT_CHARS_LIMIT: usize = 20000;
const EXECUTE_BASH_DENIED_PATTERNS: &[&str] = &[
    "rm -rf", "mkfs", "shutdown", "reboot", "poweroff", "halt", ":(){", "dd if=",
];
const EXECUTE_BASH_READ_ONLY_PREFIXES: &[&str] = &[
    "ls",
    "pwd",
    "cat ",
    "rg ",
    "grep ",
    "head ",
    "tail ",
    "wc ",
    "find ",
    "stat ",
    "git status",
    "git diff",
    "git log",
    "echo ",
];

#[derive(Debug, Clone, PartialEq, Eq)]
struct FsReadRequest {
    path: String,
    start_line: usize,
    max_lines: usize,
    max_bytes: usize,
    max_entries: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct FsReadToolError {
    code: &'static str,
    message: String,
}

impl FsReadToolError {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

fn fs_read_error_payload(path: &str, err: FsReadToolError) -> Value {
    json!({
        "status": "error",
        "code": err.code,
        "error": err.message,
        "path": path
    })
}

fn parse_fs_read_usize_arg(
    args: &Value,
    key: &str,
    default: usize,
    min: usize,
    max: usize,
) -> Result<usize, FsReadToolError> {
    let Some(raw_value) = args.get(key) else {
        return Ok(default);
    };

    let Some(value) = raw_value.as_u64() else {
        return Err(FsReadToolError::new(
            "invalid_args",
            format!("'{key}' must be a positive integer"),
        ));
    };

    let parsed = usize::try_from(value).map_err(|_| {
        FsReadToolError::new(
            "invalid_args",
            format!("'{key}' is too large for this platform"),
        )
    })?;
    if parsed < min || parsed > max {
        return Err(FsReadToolError::new(
            "invalid_args",
            format!("'{key}' must be between {min} and {max}"),
        ));
    }

    Ok(parsed)
}

fn parse_fs_read_request(args: &Value) -> Result<FsReadRequest, FsReadToolError> {
    let path = args
        .get("path")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default()
        .to_string();
    if path.is_empty() {
        return Err(FsReadToolError::new(
            "invalid_args",
            "'path' is required for fs_read",
        ));
    }

    Ok(FsReadRequest {
        path,
        start_line: parse_fs_read_usize_arg(args, "start_line", 1, 1, 1_000_000)?,
        max_lines: parse_fs_read_usize_arg(
            args,
            "max_lines",
            FS_READ_DEFAULT_MAX_LINES,
            1,
            FS_READ_MAX_LINES_LIMIT,
        )?,
        max_bytes: parse_fs_read_usize_arg(
            args,
            "max_bytes",
            FS_READ_DEFAULT_MAX_BYTES,
            1,
            FS_READ_MAX_BYTES_LIMIT,
        )?,
        max_entries: parse_fs_read_usize_arg(
            args,
            "max_entries",
            FS_READ_DEFAULT_MAX_ENTRIES,
            1,
            FS_READ_MAX_ENTRIES_LIMIT,
        )?,
    })
}

fn fs_read_workspace_root() -> Result<PathBuf, FsReadToolError> {
    let cwd = std::env::current_dir().map_err(|_| {
        FsReadToolError::new(
            "internal_error",
            "failed to resolve workspace root from current directory",
        )
    })?;

    cwd.canonicalize().map_err(|_| {
        FsReadToolError::new(
            "internal_error",
            "failed to canonicalize workspace root path",
        )
    })
}

fn resolve_fs_read_path(
    workspace_root: &Path,
    requested_path: &str,
) -> Result<PathBuf, FsReadToolError> {
    let requested = PathBuf::from(requested_path);
    let absolute = if requested.is_absolute() {
        requested
    } else {
        workspace_root.join(requested)
    };

    if !absolute.exists() {
        return Err(FsReadToolError::new(
            "invalid_path",
            format!("path '{}' does not exist", requested_path),
        ));
    }

    absolute.canonicalize().map_err(|_| {
        FsReadToolError::new(
            "invalid_path",
            format!("path '{}' could not be resolved", requested_path),
        )
    })
}

fn enforce_workspace_path_policy(
    requested_path: &str,
    resolved: &Path,
    workspace_root: &Path,
) -> Result<(), FsReadToolError> {
    if !resolved.starts_with(workspace_root) {
        return Err(FsReadToolError::new(
            "denied_path",
            format!(
                "fs_read denied path '{}': outside workspace root '{}'",
                requested_path,
                workspace_root.display()
            ),
        ));
    }

    for component in resolved.components() {
        let segment = component.as_os_str().to_string_lossy();
        if FS_READ_DENIED_SEGMENTS
            .iter()
            .any(|denied| segment.eq_ignore_ascii_case(denied))
        {
            return Err(FsReadToolError::new(
                "denied_path",
                format!(
                    "fs_read denied path '{}': segment '{}' is blocked by policy",
                    requested_path, segment
                ),
            ));
        }
    }

    if let Some(name) = resolved.file_name().and_then(|value| value.to_str())
        && FS_READ_DENIED_FILE_NAMES
            .iter()
            .any(|denied| name.eq_ignore_ascii_case(denied))
    {
        return Err(FsReadToolError::new(
            "denied_path",
            format!(
                "fs_read denied path '{}': filename '{}' is blocked by policy",
                requested_path, name
            ),
        ));
    }

    Ok(())
}

fn fs_read_display_path(path: &Path, workspace_root: &Path) -> String {
    path.strip_prefix(workspace_root)
        .map(|relative| {
            if relative.as_os_str().is_empty() {
                ".".to_string()
            } else {
                format!("./{}", relative.display())
            }
        })
        .unwrap_or_else(|_| path.display().to_string())
}

fn fs_read_file_payload(
    resolved: &Path,
    display_path: &str,
    request: &FsReadRequest,
) -> Result<Value, FsReadToolError> {
    let data = std::fs::read(resolved).map_err(|_| {
        FsReadToolError::new(
            "io_error",
            format!("failed to read file '{}'", display_path),
        )
    })?;

    let bytes_to_use = data.len().min(request.max_bytes);
    let truncated_by_bytes = data.len() > bytes_to_use;
    let content = String::from_utf8_lossy(&data[..bytes_to_use]).to_string();
    let lines = content.lines().collect::<Vec<&str>>();

    let start_index = request.start_line.saturating_sub(1).min(lines.len());
    let end_index = start_index
        .saturating_add(request.max_lines)
        .min(lines.len());
    let selected = lines[start_index..end_index].join("\n");
    let omitted_lines = lines.len().saturating_sub(end_index);

    Ok(json!({
        "status": "ok",
        "kind": "file",
        "path": display_path,
        "start_line": request.start_line,
        "line_count": end_index.saturating_sub(start_index),
        "omitted_lines": omitted_lines,
        "truncated": truncated_by_bytes || omitted_lines > 0,
        "content": selected
    }))
}

fn fs_read_directory_payload(
    resolved: &Path,
    display_path: &str,
    request: &FsReadRequest,
) -> Result<Value, FsReadToolError> {
    let mut entries = std::fs::read_dir(resolved)
        .map_err(|_| {
            FsReadToolError::new(
                "io_error",
                format!("failed to read directory '{}'", display_path),
            )
        })?
        .filter_map(|entry| entry.ok())
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().to_string();
            let file_type = entry.file_type().ok()?;
            let kind = if file_type.is_dir() {
                "dir"
            } else if file_type.is_file() {
                "file"
            } else if file_type.is_symlink() {
                "symlink"
            } else {
                "other"
            };
            Some((name, kind.to_string()))
        })
        .collect::<Vec<(String, String)>>();
    entries.sort_by(|a, b| a.0.cmp(&b.0));

    let total_entries = entries.len();
    let truncated = total_entries > request.max_entries;
    if truncated {
        entries.truncate(request.max_entries);
    }

    let rendered_entries = entries
        .into_iter()
        .map(|(name, kind)| json!({ "name": name, "kind": kind }))
        .collect::<Vec<Value>>();

    Ok(json!({
        "status": "ok",
        "kind": "directory",
        "path": display_path,
        "entry_count": total_entries,
        "truncated": truncated,
        "entries": rendered_entries
    }))
}

fn fs_read_tool_response_with_root(args: &Value, workspace_root: &Path) -> Value {
    let request = match parse_fs_read_request(args) {
        Ok(request) => request,
        Err(err) => return fs_read_error_payload("<missing>", err),
    };

    let resolved = match resolve_fs_read_path(workspace_root, &request.path) {
        Ok(path) => path,
        Err(err) => return fs_read_error_payload(&request.path, err),
    };
    if let Err(err) = enforce_workspace_path_policy(&request.path, &resolved, workspace_root) {
        return fs_read_error_payload(&request.path, err);
    }

    let display_path = fs_read_display_path(&resolved, workspace_root);
    if resolved.is_file() {
        return match fs_read_file_payload(&resolved, &display_path, &request) {
            Ok(value) => value,
            Err(err) => fs_read_error_payload(&request.path, err),
        };
    }

    if resolved.is_dir() {
        return match fs_read_directory_payload(&resolved, &display_path, &request) {
            Ok(value) => value,
            Err(err) => fs_read_error_payload(&request.path, err),
        };
    }

    fs_read_error_payload(
        &request.path,
        FsReadToolError::new(
            "unsupported_path",
            format!(
                "fs_read supports only files and directories (path '{}')",
                request.path
            ),
        ),
    )
}

fn fs_read_tool_response(args: &Value) -> Value {
    let workspace_root = match fs_read_workspace_root() {
        Ok(root) => root,
        Err(err) => return fs_read_error_payload("<workspace>", err),
    };
    fs_read_tool_response_with_root(args, &workspace_root)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FsWriteMode {
    Create,
    Overwrite,
    Append,
    Patch,
}

impl FsWriteMode {
    fn label(self) -> &'static str {
        match self {
            FsWriteMode::Create => "create",
            FsWriteMode::Overwrite => "overwrite",
            FsWriteMode::Append => "append",
            FsWriteMode::Patch => "patch",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct FsWritePatch {
    find: String,
    replace: String,
    replace_all: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct FsWriteRequest {
    path: String,
    mode: FsWriteMode,
    content: Option<String>,
    patch: Option<FsWritePatch>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct FsWriteToolError {
    code: &'static str,
    message: String,
}

impl FsWriteToolError {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

fn fs_write_error_payload(path: &str, err: FsWriteToolError) -> Value {
    json!({
        "status": "error",
        "code": err.code,
        "error": err.message,
        "path": path
    })
}

fn parse_fs_write_mode(args: &Value) -> Result<FsWriteMode, FsWriteToolError> {
    let mode = args
        .get("mode")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("overwrite")
        .to_ascii_lowercase();

    match mode.as_str() {
        "create" => Ok(FsWriteMode::Create),
        "overwrite" | "update" => Ok(FsWriteMode::Overwrite),
        "append" => Ok(FsWriteMode::Append),
        "patch" => Ok(FsWriteMode::Patch),
        _ => Err(FsWriteToolError::new(
            "invalid_args",
            "mode must be one of: create, overwrite, append, patch",
        )),
    }
}

fn parse_fs_write_patch(args: &Value) -> Result<Option<FsWritePatch>, FsWriteToolError> {
    let Some(raw_patch) = args.get("patch") else {
        return Ok(None);
    };

    let Some(patch_obj) = raw_patch.as_object() else {
        return Err(FsWriteToolError::new(
            "malformed_edit",
            "'patch' must be an object with 'find' and 'replace' fields",
        ));
    };

    let find = patch_obj
        .get("find")
        .and_then(Value::as_str)
        .map(str::to_string)
        .unwrap_or_default();
    let replace = patch_obj
        .get("replace")
        .and_then(Value::as_str)
        .map(str::to_string)
        .unwrap_or_default();
    let replace_all = patch_obj
        .get("replace_all")
        .and_then(Value::as_bool)
        .unwrap_or(false);

    Ok(Some(FsWritePatch {
        find,
        replace,
        replace_all,
    }))
}

fn parse_fs_write_request(args: &Value) -> Result<FsWriteRequest, FsWriteToolError> {
    let path = args
        .get("path")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default()
        .to_string();
    if path.is_empty() {
        return Err(FsWriteToolError::new(
            "invalid_args",
            "'path' is required for fs_write",
        ));
    }

    let mode = parse_fs_write_mode(args)?;
    let content = args
        .get("content")
        .and_then(Value::as_str)
        .map(str::to_string);
    let patch = parse_fs_write_patch(args)?;

    match mode {
        FsWriteMode::Create | FsWriteMode::Overwrite | FsWriteMode::Append => {
            if content.is_none() {
                return Err(FsWriteToolError::new(
                    "invalid_args",
                    "'content' is required for create/overwrite/append modes",
                ));
            }
        }
        FsWriteMode::Patch => {
            let Some(patch_ref) = patch.as_ref() else {
                return Err(FsWriteToolError::new(
                    "malformed_edit",
                    "'patch' object is required for patch mode",
                ));
            };
            if patch_ref.find.is_empty() {
                return Err(FsWriteToolError::new(
                    "malformed_edit",
                    "'patch.find' cannot be empty",
                ));
            }
        }
    }

    Ok(FsWriteRequest {
        path,
        mode,
        content,
        patch,
    })
}

fn resolve_fs_write_path(
    workspace_root: &Path,
    requested_path: &str,
) -> Result<PathBuf, FsWriteToolError> {
    let requested = PathBuf::from(requested_path);
    let absolute = if requested.is_absolute() {
        requested
    } else {
        workspace_root.join(requested)
    };

    let mut existing = absolute.as_path();
    while !existing.exists() {
        existing = existing.parent().ok_or_else(|| {
            FsWriteToolError::new(
                "invalid_path",
                format!("path '{}' has no resolvable parent", requested_path),
            )
        })?;
    }

    let canonical_existing = existing.canonicalize().map_err(|_| {
        FsWriteToolError::new(
            "invalid_path",
            format!("path '{}' could not be resolved", requested_path),
        )
    })?;
    let suffix = absolute.strip_prefix(existing).map_err(|_| {
        FsWriteToolError::new(
            "invalid_path",
            format!("path '{}' could not be normalized", requested_path),
        )
    })?;

    if suffix.as_os_str().is_empty() {
        return Ok(canonical_existing);
    }

    Ok(canonical_existing.join(suffix))
}

fn fs_write_ok_payload(
    display_path: &str,
    mode: FsWriteMode,
    changed: bool,
    bytes_written: usize,
    replaced_count: usize,
) -> Value {
    json!({
        "status": "ok",
        "kind": "fs_write",
        "path": display_path,
        "mode": mode.label(),
        "changed": changed,
        "bytes_written": bytes_written,
        "replaced_count": replaced_count
    })
}

fn fs_write_tool_response_with_root(args: &Value, workspace_root: &Path) -> Value {
    let request = match parse_fs_write_request(args) {
        Ok(request) => request,
        Err(err) => return fs_write_error_payload("<missing>", err),
    };

    let resolved = match resolve_fs_write_path(workspace_root, &request.path) {
        Ok(path) => path,
        Err(err) => return fs_write_error_payload(&request.path, err),
    };
    if let Err(err) = enforce_workspace_path_policy(&request.path, &resolved, workspace_root) {
        return fs_write_error_payload(&request.path, FsWriteToolError::new(err.code, err.message));
    }

    let display_path = fs_read_display_path(&resolved, workspace_root);
    let result = match request.mode {
        FsWriteMode::Create => {
            if resolved.exists() {
                Err(FsWriteToolError::new(
                    "invalid_path",
                    format!("file '{}' already exists", request.path),
                ))
            } else {
                if let Some(parent) = resolved.parent()
                    && std::fs::create_dir_all(parent).is_err()
                {
                    return fs_write_error_payload(
                        &request.path,
                        FsWriteToolError::new(
                            "io_error",
                            format!("failed to create parent directories for '{}'", request.path),
                        ),
                    );
                }
                let content = request.content.as_deref().unwrap_or_default();
                std::fs::write(&resolved, content.as_bytes())
                    .map(|_| {
                        fs_write_ok_payload(&display_path, request.mode, true, content.len(), 0)
                    })
                    .map_err(|_| {
                        FsWriteToolError::new(
                            "io_error",
                            format!("failed to write '{}'", request.path),
                        )
                    })
            }
        }
        FsWriteMode::Overwrite => {
            if let Some(parent) = resolved.parent()
                && std::fs::create_dir_all(parent).is_err()
            {
                return fs_write_error_payload(
                    &request.path,
                    FsWriteToolError::new(
                        "io_error",
                        format!("failed to create parent directories for '{}'", request.path),
                    ),
                );
            }
            let content = request.content.as_deref().unwrap_or_default();
            std::fs::write(&resolved, content.as_bytes())
                .map(|_| fs_write_ok_payload(&display_path, request.mode, true, content.len(), 0))
                .map_err(|_| {
                    FsWriteToolError::new("io_error", format!("failed to write '{}'", request.path))
                })
        }
        FsWriteMode::Append => {
            if let Some(parent) = resolved.parent()
                && std::fs::create_dir_all(parent).is_err()
            {
                return fs_write_error_payload(
                    &request.path,
                    FsWriteToolError::new(
                        "io_error",
                        format!("failed to create parent directories for '{}'", request.path),
                    ),
                );
            }
            let content = request.content.as_deref().unwrap_or_default();
            OpenOptions::new()
                .append(true)
                .create(true)
                .open(&resolved)
                .and_then(|mut file| std::io::Write::write_all(&mut file, content.as_bytes()))
                .map(|_| fs_write_ok_payload(&display_path, request.mode, true, content.len(), 0))
                .map_err(|_| {
                    FsWriteToolError::new(
                        "io_error",
                        format!("failed to append to '{}'", request.path),
                    )
                })
        }
        FsWriteMode::Patch => {
            if !resolved.exists() {
                Err(FsWriteToolError::new(
                    "invalid_path",
                    format!("file '{}' does not exist for patch mode", request.path),
                ))
            } else {
                let patch = request.patch.as_ref().expect("patch mode validated");
                let original = match std::fs::read_to_string(&resolved) {
                    Ok(content) => content,
                    Err(_) => {
                        return fs_write_error_payload(
                            &request.path,
                            FsWriteToolError::new(
                                "io_error",
                                format!("failed to read '{}' for patch mode", request.path),
                            ),
                        );
                    }
                };

                let replaced_count = if patch.replace_all {
                    original.matches(&patch.find).count()
                } else if original.contains(&patch.find) {
                    1
                } else {
                    0
                };
                if replaced_count == 0 {
                    Err(FsWriteToolError::new(
                        "malformed_edit",
                        format!(
                            "patch.find value not found in '{}': '{}'",
                            request.path, patch.find
                        ),
                    ))
                } else {
                    let updated = if patch.replace_all {
                        original.replace(&patch.find, &patch.replace)
                    } else {
                        original.replacen(&patch.find, &patch.replace, 1)
                    };
                    let changed = updated != original;
                    std::fs::write(&resolved, updated.as_bytes())
                        .map(|_| {
                            fs_write_ok_payload(
                                &display_path,
                                request.mode,
                                changed,
                                updated.len(),
                                replaced_count,
                            )
                        })
                        .map_err(|_| {
                            FsWriteToolError::new(
                                "io_error",
                                format!("failed to write patched content to '{}'", request.path),
                            )
                        })
                }
            }
        }
    };

    match result {
        Ok(payload) => payload,
        Err(err) => fs_write_error_payload(&request.path, err),
    }
}

fn fs_write_tool_response(args: &Value) -> Value {
    let workspace_root = match fs_read_workspace_root() {
        Ok(root) => root,
        Err(err) => {
            return fs_write_error_payload(
                "<workspace>",
                FsWriteToolError::new(err.code, err.message),
            );
        }
    };
    fs_write_tool_response_with_root(args, &workspace_root)
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ExecuteBashRequest {
    command: String,
    approved: bool,
    allow_dangerous: bool,
    timeout_secs: u64,
    retry_attempts: u32,
    retry_delay_ms: u64,
    max_output_chars: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ExecuteBashToolError {
    code: &'static str,
    message: String,
}

impl ExecuteBashToolError {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ExecuteBashPolicyDecision {
    read_only_auto_allow: bool,
}

fn execute_bash_error_payload(command: &str, err: ExecuteBashToolError, attempts: u32) -> Value {
    json!({
        "status": "error",
        "kind": "execute_bash",
        "code": err.code,
        "error": err.message,
        "command": command,
        "attempts": attempts
    })
}

fn parse_execute_bash_u64_arg(
    args: &Value,
    key: &str,
    default: u64,
    min: u64,
    max: u64,
) -> Result<u64, ExecuteBashToolError> {
    let Some(value) = args.get(key) else {
        return Ok(default);
    };
    let Some(parsed) = value.as_u64() else {
        return Err(ExecuteBashToolError::new(
            "invalid_args",
            format!("'{key}' must be a positive integer"),
        ));
    };
    if parsed < min || parsed > max {
        return Err(ExecuteBashToolError::new(
            "invalid_args",
            format!("'{key}' must be between {min} and {max}"),
        ));
    }
    Ok(parsed)
}

fn parse_execute_bash_request(args: &Value) -> Result<ExecuteBashRequest, ExecuteBashToolError> {
    let command = args
        .get("command")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default()
        .to_string();
    if command.is_empty() {
        return Err(ExecuteBashToolError::new(
            "invalid_args",
            "'command' is required for execute_bash",
        ));
    }

    let max_output_chars = parse_fs_read_usize_arg(
        args,
        "max_output_chars",
        EXECUTE_BASH_DEFAULT_MAX_OUTPUT_CHARS,
        128,
        EXECUTE_BASH_MAX_OUTPUT_CHARS_LIMIT,
    )
    .map_err(|err| ExecuteBashToolError::new(err.code, err.message))?;

    Ok(ExecuteBashRequest {
        command,
        approved: args
            .get("approved")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        allow_dangerous: args
            .get("allow_dangerous")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        timeout_secs: parse_execute_bash_u64_arg(
            args,
            "timeout_secs",
            EXECUTE_BASH_DEFAULT_TIMEOUT_SECS,
            1,
            120,
        )?,
        retry_attempts: parse_execute_bash_u64_arg(
            args,
            "retry_attempts",
            EXECUTE_BASH_DEFAULT_RETRY_ATTEMPTS as u64,
            1,
            5,
        )? as u32,
        retry_delay_ms: parse_execute_bash_u64_arg(
            args,
            "retry_delay_ms",
            EXECUTE_BASH_DEFAULT_RETRY_DELAY_MS,
            0,
            5000,
        )?,
        max_output_chars,
    })
}

fn is_read_only_command(command: &str) -> bool {
    let normalized = command.trim().to_ascii_lowercase();
    if normalized.is_empty() {
        return false;
    }

    EXECUTE_BASH_READ_ONLY_PREFIXES
        .iter()
        .any(|prefix| normalized == *prefix || normalized.starts_with(prefix))
}

fn matched_denied_pattern(command: &str) -> Option<&'static str> {
    let normalized = command.trim().to_ascii_lowercase();
    EXECUTE_BASH_DENIED_PATTERNS
        .iter()
        .copied()
        .find(|pattern| normalized.contains(pattern))
}

fn evaluate_execute_bash_policy(
    request: &ExecuteBashRequest,
) -> Result<ExecuteBashPolicyDecision, ExecuteBashToolError> {
    if let Some(pattern) = matched_denied_pattern(&request.command) {
        if !request.allow_dangerous {
            return Err(ExecuteBashToolError::new(
                "denied_command",
                format!(
                    "execute_bash denied command due to blocked pattern '{pattern}'. Set allow_dangerous=true and approved=true to override."
                ),
            ));
        }
        if !request.approved {
            return Err(ExecuteBashToolError::new(
                "approval_required",
                "execute_bash requires approved=true for dangerous command override",
            ));
        }
        return Ok(ExecuteBashPolicyDecision {
            read_only_auto_allow: false,
        });
    }

    if is_read_only_command(&request.command) {
        return Ok(ExecuteBashPolicyDecision {
            read_only_auto_allow: true,
        });
    }

    if !request.approved {
        return Err(ExecuteBashToolError::new(
            "approval_required",
            "execute_bash requires approved=true for non-read-only commands",
        ));
    }

    Ok(ExecuteBashPolicyDecision {
        read_only_auto_allow: false,
    })
}

fn truncate_text(text: &str, max_chars: usize) -> (String, bool) {
    let mut iter = text.chars();
    let truncated = iter.by_ref().take(max_chars).collect::<String>();
    if iter.next().is_some() {
        (truncated, true)
    } else {
        (text.to_string(), false)
    }
}

async fn run_execute_bash_once(
    command: &str,
    timeout_secs: u64,
) -> Result<std::process::Output, ExecuteBashToolError> {
    let child = tokio::process::Command::new("sh")
        .arg("-lc")
        .arg(command)
        .output();
    match tokio::time::timeout(Duration::from_secs(timeout_secs), child).await {
        Ok(result) => result
            .map_err(|_| ExecuteBashToolError::new("io_error", "failed to launch shell command")),
        Err(_) => Err(ExecuteBashToolError::new(
            "timeout",
            format!("command timed out after {timeout_secs}s"),
        )),
    }
}

fn execute_bash_output_payload(
    request: &ExecuteBashRequest,
    policy: &ExecuteBashPolicyDecision,
    attempts: u32,
    output: std::process::Output,
) -> Value {
    let stdout_text = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr_text = String::from_utf8_lossy(&output.stderr).to_string();
    let (stdout, stdout_truncated) = truncate_text(&stdout_text, request.max_output_chars);
    let (stderr, stderr_truncated) = truncate_text(&stderr_text, request.max_output_chars);

    if output.status.success() {
        return json!({
            "status": "ok",
            "kind": "execute_bash",
            "command": request.command,
            "attempts": attempts,
            "exit_code": output.status.code().unwrap_or(0),
            "read_only_auto_allow": policy.read_only_auto_allow,
            "stdout": stdout,
            "stderr": stderr,
            "stdout_truncated": stdout_truncated,
            "stderr_truncated": stderr_truncated
        });
    }

    json!({
        "status": "error",
        "kind": "execute_bash",
        "code": "command_failed",
        "error": format!("command exited with non-zero status: {}", output.status),
        "command": request.command,
        "attempts": attempts,
        "exit_code": output.status.code().unwrap_or(-1),
        "read_only_auto_allow": policy.read_only_auto_allow,
        "stdout": stdout,
        "stderr": stderr,
        "stdout_truncated": stdout_truncated,
        "stderr_truncated": stderr_truncated
    })
}

async fn execute_bash_tool_response(args: &Value) -> Value {
    let request = match parse_execute_bash_request(args) {
        Ok(request) => request,
        Err(err) => return execute_bash_error_payload("<missing>", err, 0),
    };
    let policy = match evaluate_execute_bash_policy(&request) {
        Ok(decision) => decision,
        Err(err) => return execute_bash_error_payload(&request.command, err, 0),
    };

    let mut attempts = 0u32;
    let mut last_error: Option<ExecuteBashToolError> = None;

    while attempts < request.retry_attempts {
        attempts += 1;
        match run_execute_bash_once(&request.command, request.timeout_secs).await {
            Ok(output) => {
                let payload = execute_bash_output_payload(&request, &policy, attempts, output);
                let failed = payload
                    .get("status")
                    .and_then(Value::as_str)
                    .map(|status| status.eq_ignore_ascii_case("error"))
                    .unwrap_or(false);
                if !failed || attempts >= request.retry_attempts {
                    return payload;
                }
                last_error = Some(ExecuteBashToolError::new(
                    "command_failed",
                    payload
                        .get("error")
                        .and_then(Value::as_str)
                        .unwrap_or("command failed"),
                ));
            }
            Err(err) => {
                last_error = Some(err);
            }
        }

        if attempts < request.retry_attempts && request.retry_delay_ms > 0 {
            tokio::time::sleep(Duration::from_millis(request.retry_delay_ms)).await;
        }
    }

    execute_bash_error_payload(
        &request.command,
        last_error.unwrap_or_else(|| {
            ExecuteBashToolError::new("internal_error", "execute_bash failed unexpectedly")
        }),
        attempts,
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct GitHubOpsError {
    code: &'static str,
    message: String,
}

impl GitHubOpsError {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct GitHubCliOutput {
    success: bool,
    exit_code: i32,
    stdout: String,
    stderr: String,
}

fn github_ops_error_payload(action: &str, err: GitHubOpsError) -> Value {
    json!({
        "status": "error",
        "kind": "github_ops",
        "action": action,
        "code": err.code,
        "error": err.message
    })
}

fn github_token_present() -> bool {
    ["GH_TOKEN", "GITHUB_TOKEN"].iter().any(|key| {
        std::env::var(key)
            .map(|value| !value.trim().is_empty())
            .unwrap_or(false)
    })
}

fn parse_required_string_arg(args: &Value, key: &str) -> Result<String, GitHubOpsError> {
    args.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .ok_or_else(|| GitHubOpsError::new("invalid_args", format!("'{key}' is required")))
}

fn parse_optional_string_arg(args: &Value, key: &str) -> Option<String> {
    args.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn parse_optional_string_list(args: &Value, key: &str) -> Result<Vec<String>, GitHubOpsError> {
    let Some(raw_value) = args.get(key) else {
        return Ok(Vec::new());
    };

    let Some(values) = raw_value.as_array() else {
        return Err(GitHubOpsError::new(
            "invalid_args",
            format!("'{key}' must be an array of strings"),
        ));
    };

    Ok(values
        .iter()
        .filter_map(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .collect::<Vec<String>>())
}

fn build_github_ops_command(args: &Value) -> Result<(String, Vec<String>), GitHubOpsError> {
    let action = parse_required_string_arg(args, "action")?.to_ascii_lowercase();
    match action.as_str() {
        "issue_create" => {
            let repo = parse_required_string_arg(args, "repo")?;
            let title = parse_required_string_arg(args, "title")?;
            let body = parse_required_string_arg(args, "body")?;
            let labels = parse_optional_string_list(args, "labels")?;

            let mut command = vec![
                "issue".to_string(),
                "create".to_string(),
                "--repo".to_string(),
                repo,
                "--title".to_string(),
                title,
                "--body".to_string(),
                body,
            ];
            for label in labels {
                command.push("--label".to_string());
                command.push(label);
            }
            Ok((action, command))
        }
        "issue_update" => {
            let repo = parse_required_string_arg(args, "repo")?;
            let issue_number = parse_required_string_arg(args, "issue_number")?;
            let state = parse_optional_string_arg(args, "state")
                .map(|value| value.to_ascii_lowercase())
                .unwrap_or_default();
            if state == "closed" {
                return Ok((
                    action,
                    vec![
                        "issue".to_string(),
                        "close".to_string(),
                        issue_number,
                        "--repo".to_string(),
                        repo,
                    ],
                ));
            }
            if state == "open" {
                return Ok((
                    action,
                    vec![
                        "issue".to_string(),
                        "reopen".to_string(),
                        issue_number,
                        "--repo".to_string(),
                        repo,
                    ],
                ));
            }

            let title = parse_optional_string_arg(args, "title");
            let body = parse_optional_string_arg(args, "body");
            let add_labels = parse_optional_string_list(args, "add_labels")?;
            let remove_labels = parse_optional_string_list(args, "remove_labels")?;
            if title.is_none()
                && body.is_none()
                && add_labels.is_empty()
                && remove_labels.is_empty()
            {
                return Err(GitHubOpsError::new(
                    "invalid_args",
                    "issue_update requires at least one of title/body/add_labels/remove_labels/state",
                ));
            }

            let mut command = vec![
                "issue".to_string(),
                "edit".to_string(),
                issue_number,
                "--repo".to_string(),
                repo,
            ];
            if let Some(title) = title {
                command.push("--title".to_string());
                command.push(title);
            }
            if let Some(body) = body {
                command.push("--body".to_string());
                command.push(body);
            }
            for label in add_labels {
                command.push("--add-label".to_string());
                command.push(label);
            }
            for label in remove_labels {
                command.push("--remove-label".to_string());
                command.push(label);
            }
            Ok((action, command))
        }
        "pr_create" => {
            let repo = parse_required_string_arg(args, "repo")?;
            let title = parse_required_string_arg(args, "title")?;
            let body = parse_required_string_arg(args, "body")?;
            let head = parse_optional_string_arg(args, "head");
            let base = parse_optional_string_arg(args, "base");
            let draft = args.get("draft").and_then(Value::as_bool).unwrap_or(true);

            let mut command = vec![
                "pr".to_string(),
                "create".to_string(),
                "--repo".to_string(),
                repo,
                "--title".to_string(),
                title,
                "--body".to_string(),
                body,
            ];
            if draft {
                command.push("--draft".to_string());
            }
            if let Some(head) = head {
                command.push("--head".to_string());
                command.push(head);
            }
            if let Some(base) = base {
                command.push("--base".to_string());
                command.push(base);
            }
            Ok((action, command))
        }
        "project_item_update" => {
            let project_id = parse_required_string_arg(args, "project_id")?;
            let item_id = parse_required_string_arg(args, "item_id")?;
            let field_id = parse_required_string_arg(args, "field_id")?;
            let status_option_id = parse_required_string_arg(args, "status_option_id")?;
            Ok((
                action,
                vec![
                    "project".to_string(),
                    "item-edit".to_string(),
                    "--project-id".to_string(),
                    project_id,
                    "--id".to_string(),
                    item_id,
                    "--field-id".to_string(),
                    field_id,
                    "--single-select-option-id".to_string(),
                    status_option_id,
                ],
            ))
        }
        _ => Err(GitHubOpsError::new(
            "invalid_args",
            "action must be one of: issue_create, issue_update, pr_create, project_item_update",
        )),
    }
}

fn run_gh_command(args: &[String]) -> Result<GitHubCliOutput, GitHubOpsError> {
    let output = std::process::Command::new("gh")
        .args(args)
        .output()
        .map_err(|err| {
            if err.kind() == io::ErrorKind::NotFound {
                GitHubOpsError::new(
                    "gh_missing",
                    "GitHub CLI 'gh' was not found. Install gh and retry.",
                )
            } else {
                GitHubOpsError::new("io_error", format!("failed to run gh command: {err}"))
            }
        })?;

    Ok(GitHubCliOutput {
        success: output.status.success(),
        exit_code: output.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&output.stdout).to_string(),
        stderr: String::from_utf8_lossy(&output.stderr).to_string(),
    })
}

fn github_ops_tool_response_with_runner<F>(
    args: &Value,
    token_present: bool,
    mut runner: F,
) -> Value
where
    F: FnMut(&[String]) -> Result<GitHubCliOutput, GitHubOpsError>,
{
    let (action, command) = match build_github_ops_command(args) {
        Ok(parsed) => parsed,
        Err(err) => return github_ops_error_payload("unknown", err),
    };

    if !token_present {
        let auth_command = vec!["auth".to_string(), "status".to_string()];
        let auth_ok = runner(&auth_command)
            .map(|out| out.success)
            .unwrap_or(false);
        if !auth_ok {
            return github_ops_error_payload(
                &action,
                GitHubOpsError::new(
                    "auth_required",
                    "GitHub auth not detected. Set GH_TOKEN/GITHUB_TOKEN or run `gh auth login`.",
                ),
            );
        }
    }

    match runner(&command) {
        Ok(output) => {
            if output.success {
                json!({
                    "status": "ok",
                    "kind": "github_ops",
                    "action": action,
                    "command": format!("gh {}", command.join(" ")),
                    "exit_code": output.exit_code,
                    "stdout": output.stdout,
                    "stderr": output.stderr
                })
            } else {
                json!({
                    "status": "error",
                    "kind": "github_ops",
                    "action": action,
                    "code": "github_command_failed",
                    "error": format!("gh command exited with non-zero status: {}", output.exit_code),
                    "command": format!("gh {}", command.join(" ")),
                    "exit_code": output.exit_code,
                    "stdout": output.stdout,
                    "stderr": output.stderr
                })
            }
        }
        Err(err) => github_ops_error_payload(&action, err),
    }
}

fn github_ops_tool_response(args: &Value) -> Value {
    github_ops_tool_response_with_runner(args, github_token_present(), run_gh_command)
}

fn build_builtin_tools() -> Vec<Arc<dyn Tool>> {
    let current_time = FunctionTool::new(
        "current_unix_time",
        "Returns the current UTC timestamp in unix seconds.",
        |_ctx, _args| async move {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            Ok(json!({ "unix_utc_seconds": now }))
        },
    );

    let release_template = FunctionTool::new(
        "release_template",
        "Returns a concise release checklist skeleton for agile delivery.",
        |_ctx, args| async move {
            let releases = args.get("releases").and_then(Value::as_u64).unwrap_or(3);
            Ok(json!({
                "releases": releases,
                "template": [
                    "Objectives",
                    "Scope / Non-scope",
                    "Implementation slices",
                    "Quality gates",
                    "Release notes + rollback plan"
                ]
            }))
        },
    );

    let fs_read = FunctionTool::new(
        "fs_read",
        "Reads file content or directory entries within the workspace using path policy checks. \
         Args: path (required), start_line, max_lines, max_bytes, max_entries.",
        |_ctx, args| async move { Ok(fs_read_tool_response(&args)) },
    );

    let fs_write = FunctionTool::new(
        "fs_write",
        "Writes files within the workspace with safe modes. \
         Args: path (required), mode=create|overwrite|append|patch, content, patch={find,replace,replace_all}.",
        |_ctx, args| async move { Ok(fs_write_tool_response(&args)) },
    );

    let execute_bash = FunctionTool::new(
        "execute_bash",
        "Executes shell commands with policy checks and approval gates. \
         Args: command (required), approved, allow_dangerous, timeout_secs, retry_attempts, retry_delay_ms, max_output_chars.",
        |_ctx, args| async move { Ok(execute_bash_tool_response(&args).await) },
    );

    let github_ops = FunctionTool::new(
        "github_ops",
        "Runs GitHub workflow operations through gh CLI. \
         Args: action=issue_create|issue_update|pr_create|project_item_update plus action-specific fields.",
        |_ctx, args| async move { Ok(github_ops_tool_response(&args)) },
    );

    vec![
        Arc::new(current_time),
        Arc::new(release_template),
        Arc::new(fs_read),
        Arc::new(fs_write),
        Arc::new(execute_bash),
        Arc::new(github_ops),
    ]
}

#[cfg(test)]
fn build_single_agent(model: Arc<dyn Llm>) -> Result<Arc<dyn Agent>> {
    let tools = build_builtin_tools();
    build_single_agent_with_tools(
        model,
        &tools,
        ToolConfirmationPolicy::Never,
        Duration::from_secs(45),
        None,
    )
}

fn build_single_agent_with_tools(
    model: Arc<dyn Llm>,
    tools: &[Arc<dyn Tool>],
    tool_confirmation_policy: ToolConfirmationPolicy,
    tool_timeout: Duration,
    runtime_cfg: Option<&RuntimeConfig>,
) -> Result<Arc<dyn Agent>> {
    let instruction = if let Some(cfg) = runtime_cfg {
        let mut sections = vec![
            "You are a pragmatic AI engineer. Prioritize direct, actionable output, and when \
             planning work always prefer release-oriented increments."
                .to_string(),
        ];
        if let Some(agent_instruction) = cfg
            .agent_instruction
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            sections.push(format!("Agent-specific instruction:\n{agent_instruction}"));
        }
        if !cfg.agent_resource_paths.is_empty() {
            sections.push(format!(
                "Agent resource hints:\n{}",
                cfg.agent_resource_paths
                    .iter()
                    .map(|path| format!("- {}", path))
                    .collect::<Vec<String>>()
                    .join("\n")
            ));
        }
        sections.join("\n\n")
    } else {
        "You are a pragmatic AI engineer. Prioritize direct, actionable output, and when \
         planning work always prefer release-oriented increments."
            .to_string()
    };

    let mut builder = LlmAgentBuilder::new("assistant")
        .description("General purpose engineering assistant")
        .instruction(instruction)
        .model(model)
        .tool_confirmation_policy(tool_confirmation_policy)
        .tool_timeout(tool_timeout);

    for tool in tools {
        builder = builder.tool(tool.clone());
    }

    Ok(Arc::new(builder.build()?))
}

#[derive(Clone)]
struct ResolvedRuntimeTools {
    tools: Vec<Arc<dyn Tool>>,
    mcp_tool_names: BTreeSet<String>,
}

#[derive(Debug, Clone)]
struct ToolConfirmationSettings {
    policy: ToolConfirmationPolicy,
    run_config: RunConfig,
}

impl Default for ToolConfirmationSettings {
    fn default() -> Self {
        Self {
            policy: ToolConfirmationPolicy::Never,
            run_config: RunConfig::default(),
        }
    }
}

fn resolve_tool_confirmation_settings(
    cfg: &RuntimeConfig,
    runtime_tools: &ResolvedRuntimeTools,
) -> ToolConfirmationSettings {
    let available_tool_names = runtime_tools
        .tools
        .iter()
        .map(|tool| tool.name().to_string())
        .collect::<BTreeSet<String>>();

    let mut required_tools = BTreeSet::<String>::new();
    match cfg.tool_confirmation_mode {
        ToolConfirmationMode::Never => {}
        ToolConfirmationMode::McpOnly => {
            required_tools.extend(runtime_tools.mcp_tool_names.iter().cloned());
        }
        ToolConfirmationMode::Always => {
            required_tools.extend(available_tool_names.iter().cloned());
        }
    }

    for guarded_tool in [
        FS_WRITE_TOOL_NAME,
        EXECUTE_BASH_TOOL_NAME,
        GITHUB_OPS_TOOL_NAME,
    ] {
        if available_tool_names.contains(guarded_tool) {
            required_tools.insert(guarded_tool.to_string());
        }
    }

    for tool_name in &cfg.require_confirm_tool {
        let trimmed = tool_name.trim();
        if trimmed.is_empty() {
            continue;
        }
        if available_tool_names.contains(trimmed) {
            required_tools.insert(trimmed.to_string());
        } else {
            tracing::warn!(
                tool = trimmed,
                "tool in require_confirm_tool is not present in runtime toolset; ignoring"
            );
        }
    }

    let mut approved_tools = BTreeSet::<String>::new();
    for tool_name in &cfg.approve_tool {
        let trimmed = tool_name.trim();
        if trimmed.is_empty() {
            continue;
        }
        if available_tool_names.contains(trimmed) {
            approved_tools.insert(trimmed.to_string());
        } else {
            tracing::warn!(
                tool = trimmed,
                "tool in approve_tool is not present in runtime toolset; ignoring"
            );
        }
    }

    let policy = if required_tools.is_empty() {
        ToolConfirmationPolicy::Never
    } else {
        ToolConfirmationPolicy::PerTool(required_tools.clone())
    };

    let mut run_config = RunConfig::default();
    for tool_name in &required_tools {
        let decision = if approved_tools.contains(tool_name) {
            ToolConfirmationDecision::Approve
        } else {
            ToolConfirmationDecision::Deny
        };
        run_config
            .tool_confirmation_decisions
            .insert(tool_name.clone(), decision);
    }

    tracing::info!(
        mode = ?cfg.tool_confirmation_mode,
        available = available_tool_names.len(),
        required = required_tools.len(),
        approved = approved_tools.len(),
        denied = required_tools.len().saturating_sub(approved_tools.len()),
        "Resolved tool confirmation settings"
    );

    ToolConfirmationSettings { policy, run_config }
}

async fn resolve_runtime_tools(cfg: &RuntimeConfig) -> ResolvedRuntimeTools {
    let mut tools = build_builtin_tools();
    let built_in_count = tools.len();
    let mut mcp_tools = discover_mcp_tools(cfg).await;
    let mcp_count = mcp_tools.len();
    let discovered_mcp_tool_names = mcp_tools
        .iter()
        .map(|tool| tool.name().to_string())
        .collect::<BTreeSet<String>>();
    tools.append(&mut mcp_tools);

    if !cfg.agent_allow_tools.is_empty() {
        let allow = cfg
            .agent_allow_tools
            .iter()
            .map(|name| name.trim())
            .filter(|name| !name.is_empty())
            .collect::<BTreeSet<&str>>();
        tools.retain(|tool| allow.contains(tool.name()));
    }
    if !cfg.agent_deny_tools.is_empty() {
        let deny = cfg
            .agent_deny_tools
            .iter()
            .map(|name| name.trim())
            .filter(|name| !name.is_empty())
            .collect::<BTreeSet<&str>>();
        tools.retain(|tool| !deny.contains(tool.name()));
    }

    let mcp_tool_names = tools
        .iter()
        .map(|tool| tool.name().to_string())
        .filter(|name| discovered_mcp_tool_names.contains(name))
        .collect::<BTreeSet<String>>();

    tracing::info!(
        built_in_tools = built_in_count,
        mcp_tools = mcp_count,
        total_tools = tools.len(),
        agent_allow_tools = cfg.agent_allow_tools.len(),
        agent_deny_tools = cfg.agent_deny_tools.len(),
        "Resolved runtime toolset"
    );

    ResolvedRuntimeTools {
        tools,
        mcp_tool_names,
    }
}

fn build_workflow_agent(
    mode: WorkflowMode,
    model: Arc<dyn Llm>,
    max_iterations: u32,
    tools: &[Arc<dyn Tool>],
    tool_confirmation_policy: ToolConfirmationPolicy,
    tool_timeout: Duration,
    runtime_cfg: Option<&RuntimeConfig>,
) -> Result<Arc<dyn Agent>> {
    match mode {
        WorkflowMode::Single => build_single_agent_with_tools(
            model,
            tools,
            tool_confirmation_policy,
            tool_timeout,
            runtime_cfg,
        ),
        WorkflowMode::Sequential => build_sequential_agent(model),
        WorkflowMode::Parallel => build_parallel_agent(model),
        WorkflowMode::Loop => build_loop_agent(model, max_iterations),
        WorkflowMode::Graph => build_graph_workflow_agent(model),
    }
}

fn classify_workflow_route(input: &str) -> &'static str {
    let lower = input.to_ascii_lowercase();
    if lower.contains("risk")
        || lower.contains("rollback")
        || lower.contains("mitigation")
        || lower.contains("incident")
    {
        return "risk";
    }
    if lower.contains("architecture")
        || lower.contains("design")
        || lower.contains("system")
        || lower.contains("scal")
    {
        return "architecture";
    }
    if lower.contains("release")
        || lower.contains("sprint")
        || lower.contains("milestone")
        || lower.contains("roadmap")
    {
        return "release";
    }
    "delivery"
}

fn workflow_template(route: &str) -> &'static str {
    match route {
        "release" => {
            "Template: Release Planning\n\
             Return concise markdown with sections: Objectives, Release Slices, Acceptance \
             Criteria, Rollout Steps."
        }
        "architecture" => {
            "Template: Architecture Design\n\
             Return concise markdown with sections: Constraints, Proposed Components, \
             Data/Control Flow, Risks."
        }
        "risk" => {
            "Template: Risk and Reliability\n\
             Return concise markdown with sections: Top Risks, Impact, Mitigation, \
             Detection, Fallback."
        }
        _ => {
            "Template: Execution Delivery\n\
             Return concise markdown with sections: Scope, Implementation Steps, \
             Validation, Next Actions."
        }
    }
}

async fn generate_model_text(model: Arc<dyn Llm>, prompt: &str) -> Result<String> {
    let req = LlmRequest::new(
        model.name().to_string(),
        vec![Content::new("user").with_text(prompt)],
    );
    let mut stream = model
        .generate_content(req, false)
        .await
        .context("failed to invoke model inside graph workflow")?;

    let mut out = String::new();
    while let Some(chunk_result) = stream.next().await {
        let chunk = chunk_result.context("graph workflow model stream error")?;
        if let Some(content) = chunk.content {
            for part in content.parts {
                if let Part::Text { text } = part {
                    out.push_str(&text);
                }
            }
        }
    }

    let trimmed = out.trim();
    if trimmed.is_empty() {
        return Err(anyhow::anyhow!(
            "graph workflow did not produce textual model output"
        ));
    }
    Ok(trimmed.to_string())
}

fn build_graph_workflow_agent(model: Arc<dyn Llm>) -> Result<Arc<dyn Agent>> {
    let route_classifier = |ctx: adk_rust::graph::NodeContext| async move {
        let input = ctx
            .get("input")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let route = classify_workflow_route(&input);
        Ok(NodeOutput::new().with_update("route", json!(route)))
    };

    let release_prep = |ctx: adk_rust::graph::NodeContext| async move {
        let input = ctx
            .get("input")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let prompt = format!(
            "{}\n\nUser request:\n{}",
            workflow_template("release"),
            input
        );
        Ok(NodeOutput::new().with_update("branch_prompt", json!(prompt)))
    };

    let architecture_prep = |ctx: adk_rust::graph::NodeContext| async move {
        let input = ctx
            .get("input")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let prompt = format!(
            "{}\n\nUser request:\n{}",
            workflow_template("architecture"),
            input
        );
        Ok(NodeOutput::new().with_update("branch_prompt", json!(prompt)))
    };

    let risk_prep = |ctx: adk_rust::graph::NodeContext| async move {
        let input = ctx
            .get("input")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let prompt = format!("{}\n\nUser request:\n{}", workflow_template("risk"), input);
        Ok(NodeOutput::new().with_update("branch_prompt", json!(prompt)))
    };

    let delivery_prep = |ctx: adk_rust::graph::NodeContext| async move {
        let input = ctx
            .get("input")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let prompt = format!(
            "{}\n\nUser request:\n{}",
            workflow_template("delivery"),
            input
        );
        Ok(NodeOutput::new().with_update("branch_prompt", json!(prompt)))
    };

    let model_for_draft = model.clone();
    let draft = move |ctx: adk_rust::graph::NodeContext| {
        let model_for_draft = model_for_draft.clone();
        async move {
            let prompt = ctx
                .get("branch_prompt")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            let route_selected = ctx
                .get("route")
                .and_then(Value::as_str)
                .unwrap_or("delivery")
                .to_string();

            let output = generate_model_text(model_for_draft, &prompt)
                .await
                .map_err(|err| adk_rust::graph::GraphError::NodeExecutionFailed {
                    node: "draft_response".to_string(),
                    message: err.to_string(),
                })?;

            Ok(NodeOutput::new()
                .with_update("output", json!(output))
                .with_update("route_selected", json!(route_selected)))
        }
    };

    let agent = GraphAgent::builder("graph_delivery")
        .description("Graph-routed orchestration workflow")
        .channels(&[
            "input",
            "route",
            "branch_prompt",
            "output",
            "route_selected",
        ])
        .node_fn("classify", route_classifier)
        .node_fn("prepare_release", release_prep)
        .node_fn("prepare_architecture", architecture_prep)
        .node_fn("prepare_risk", risk_prep)
        .node_fn("prepare_delivery", delivery_prep)
        .node_fn("draft_response", draft)
        .edge(START, "classify")
        .conditional_edge(
            "classify",
            Router::by_field("route"),
            [
                ("release", "prepare_release"),
                ("architecture", "prepare_architecture"),
                ("risk", "prepare_risk"),
                ("delivery", "prepare_delivery"),
            ],
        )
        .edge("prepare_release", "draft_response")
        .edge("prepare_architecture", "draft_response")
        .edge("prepare_risk", "draft_response")
        .edge("prepare_delivery", "draft_response")
        .edge("draft_response", END)
        .build()?;

    Ok(Arc::new(agent))
}

fn build_sequential_agent(model: Arc<dyn Llm>) -> Result<Arc<dyn Agent>> {
    let scope = Arc::new(
        LlmAgentBuilder::new("scope_analyst")
            .description("Defines a concise project scope.")
            .instruction(
                "Analyze the user's request and produce a compact scope. Include assumptions, \
                 constraints, and high-risk areas.",
            )
            .model(model.clone())
            .output_key("scope_summary")
            .build()?,
    );

    let release_planner = Arc::new(
        LlmAgentBuilder::new("release_planner")
            .description("Breaks scope into release increments.")
            .instruction(
                "Using {scope_summary}, produce release-by-release slices with explicit acceptance \
                 criteria.",
            )
            .model(model.clone())
            .output_key("release_breakdown")
            .build()?,
    );

    let execution_writer = Arc::new(
        LlmAgentBuilder::new("execution_writer")
            .description("Produces the final actionable response.")
            .instruction(
                "Using {release_breakdown}, write the final answer as a practical execution guide \
                 with milestones, quality gates, and risks.",
            )
            .model(model)
            .build()?,
    );

    let agent = SequentialAgent::new(
        "sequential_delivery",
        vec![
            scope as Arc<dyn Agent>,
            release_planner as Arc<dyn Agent>,
            execution_writer as Arc<dyn Agent>,
        ],
    );

    Ok(Arc::new(agent))
}

fn build_parallel_agent(model: Arc<dyn Llm>) -> Result<Arc<dyn Agent>> {
    let architecture = Arc::new(
        LlmAgentBuilder::new("architecture_analyst")
            .description("Focuses architecture and decomposition.")
            .instruction(
                "Analyze architecture decisions and implementation decomposition for the user \
                 request.",
            )
            .model(model.clone())
            .output_key("architecture_notes")
            .build()?,
    );
    let risk = Arc::new(
        LlmAgentBuilder::new("risk_analyst")
            .description("Focuses delivery and operational risk.")
            .instruction(
                "Analyze delivery, security, and rollout risks for the user request. Keep it \
                 concrete.",
            )
            .model(model.clone())
            .output_key("risk_notes")
            .build()?,
    );
    let quality = Arc::new(
        LlmAgentBuilder::new("quality_analyst")
            .description("Focuses test and quality gates.")
            .instruction(
                "Analyze quality strategy, testing layers, and release criteria for the user \
                 request.",
            )
            .model(model.clone())
            .output_key("quality_notes")
            .build()?,
    );

    let parallel = Arc::new(ParallelAgent::new(
        "analysis_swarm",
        vec![
            architecture as Arc<dyn Agent>,
            risk as Arc<dyn Agent>,
            quality as Arc<dyn Agent>,
        ],
    ));

    let synthesizer = Arc::new(
        LlmAgentBuilder::new("synthesizer")
            .description("Merges parallel analysis into one plan.")
            .instruction(
                "Synthesize the results into one coherent plan.\n\
                 Architecture: {architecture_notes?}\n\
                 Risks: {risk_notes?}\n\
                 Quality: {quality_notes?}\n\
                 Return a single clear execution plan.",
            )
            .model(model)
            .build()?,
    );

    let root = SequentialAgent::new(
        "parallel_delivery",
        vec![parallel as Arc<dyn Agent>, synthesizer as Arc<dyn Agent>],
    );
    Ok(Arc::new(root))
}

fn build_loop_agent(model: Arc<dyn Llm>, max_iterations: u32) -> Result<Arc<dyn Agent>> {
    let iterative = Arc::new(
        LlmAgentBuilder::new("iterative_refiner")
            .description("Refines the answer until quality is acceptable.")
            .instruction(
                "Maintain and improve a draft in {draft?}. Initialize from user request if empty. \
                 Improve one step per turn. Call exit_loop when the draft is release-ready.",
            )
            .model(model.clone())
            .tool(Arc::new(ExitLoopTool::new()))
            .output_key("draft")
            .max_iterations(24)
            .build()?,
    );

    let loop_agent = Arc::new(
        LoopAgent::new("loop_refinement", vec![iterative as Arc<dyn Agent>])
            .with_max_iterations(max_iterations.max(1)),
    );

    let finalizer = Arc::new(
        LlmAgentBuilder::new("loop_finalizer")
            .description("Formats the final loop result.")
            .instruction(
                "Return the final polished response from {draft?}. If draft is empty, provide the \
                 best concise answer directly.",
            )
            .model(model)
            .build()?,
    );

    let root = SequentialAgent::new(
        "loop_delivery",
        vec![loop_agent as Arc<dyn Agent>, finalizer as Arc<dyn Agent>],
    );
    Ok(Arc::new(root))
}

fn build_release_planning_agent(model: Arc<dyn Llm>, releases: u32) -> Result<Arc<dyn Agent>> {
    let scoper = Arc::new(
        LlmAgentBuilder::new("product_scoper")
            .instruction(
                "Turn the user goal into a product scope with assumptions, constraints, and \
                 measurable outcomes.",
            )
            .model(model.clone())
            .output_key("product_scope")
            .build()?,
    );

    let release_architect = Arc::new(
        LlmAgentBuilder::new("release_architect")
            .instruction(format!(
                "Create an agile release plan across {} releases from {{product_scope}}. \
                 For each release include objective, scope, validation, and demo output.",
                releases
            ))
            .model(model.clone())
            .output_key("release_plan")
            .build()?,
    );

    let final_writer = Arc::new(
        LlmAgentBuilder::new("release_writer")
            .instruction(
                "Return the final answer in markdown with sections:\n\
                 - Vision\n\
                 - Release Breakdown\n\
                 - Definition of Done per release\n\
                 - Risks and mitigations\n\
                 - Next sprint start tasks\n\
                 Use {release_plan}.",
            )
            .model(model)
            .build()?,
    );

    let root = SequentialAgent::new(
        "release_planning_pipeline",
        vec![
            scoper as Arc<dyn Agent>,
            release_architect as Arc<dyn Agent>,
            final_writer as Arc<dyn Agent>,
        ],
    );
    Ok(Arc::new(root))
}

async fn build_runner(agent: Arc<dyn Agent>, cfg: &RuntimeConfig) -> Result<Runner> {
    build_runner_with_run_config(agent, cfg, None).await
}

async fn build_runner_with_run_config(
    agent: Arc<dyn Agent>,
    cfg: &RuntimeConfig,
    run_config: Option<RunConfig>,
) -> Result<Runner> {
    let session_service = build_session_service(cfg).await?;
    build_runner_with_session_service(agent, cfg, session_service, run_config).await
}

async fn build_runner_with_session_service(
    agent: Arc<dyn Agent>,
    cfg: &RuntimeConfig,
    session_service: Arc<dyn SessionService>,
    run_config: Option<RunConfig>,
) -> Result<Runner> {
    ensure_session_exists(&session_service, cfg).await?;
    let artifact_service = Arc::new(InMemoryArtifactService::new());

    Runner::new(RunnerConfig {
        app_name: cfg.app_name.clone(),
        agent,
        session_service,
        artifact_service: Some(artifact_service),
        memory_service: None,
        plugin_manager: None,
        run_config,
        compaction_config: None,
    })
    .context("failed to build ADK runner")
}

async fn build_single_runner_for_chat(
    cfg: &RuntimeConfig,
    session_service: Arc<dyn SessionService>,
    runtime_tools: &ResolvedRuntimeTools,
    tool_confirmation: &ToolConfirmationSettings,
    telemetry: &TelemetrySink,
) -> Result<(Runner, Provider, String)> {
    let (model, resolved_provider, model_name) = resolve_model(cfg)?;
    telemetry.emit(
        "model.resolved",
        json!({
            "provider": format!("{:?}", resolved_provider).to_ascii_lowercase(),
            "model": model_name.clone(),
            "path": "chat"
        }),
    );
    let agent = build_single_agent_with_tools(
        model,
        &runtime_tools.tools,
        tool_confirmation.policy.clone(),
        Duration::from_secs(cfg.tool_timeout_secs),
        Some(cfg),
    )?;
    let runner = build_runner_with_session_service(
        agent,
        cfg,
        session_service,
        Some(tool_confirmation.run_config.clone()),
    )
    .await?;
    Ok((runner, resolved_provider, model_name))
}

async fn build_session_service(cfg: &RuntimeConfig) -> Result<Arc<dyn SessionService>> {
    match cfg.session_backend {
        SessionBackend::Memory => Ok(Arc::new(InMemorySessionService::new())),
        SessionBackend::Sqlite => {
            let service = open_sqlite_session_service(&cfg.session_db_url).await?;
            Ok(Arc::new(service))
        }
    }
}

async fn open_sqlite_session_service(db_url: &str) -> Result<DatabaseSessionService> {
    ensure_parent_dir_for_sqlite_url(db_url)?;
    let service = DatabaseSessionService::new(db_url)
        .await
        .context("failed to open sqlite session database")?;
    service
        .migrate()
        .await
        .context("failed to run sqlite session migrations")?;
    Ok(service)
}

async fn ensure_session_exists(
    session_service: &Arc<dyn SessionService>,
    cfg: &RuntimeConfig,
) -> Result<()> {
    let session = session_service
        .get(GetRequest {
            app_name: cfg.app_name.clone(),
            user_id: cfg.user_id.clone(),
            session_id: cfg.session_id.clone(),
            num_recent_events: None,
            after: None,
        })
        .await;

    if session.is_ok() {
        return Ok(());
    }

    session_service
        .create(CreateRequest {
            app_name: cfg.app_name.clone(),
            user_id: cfg.user_id.clone(),
            session_id: Some(cfg.session_id.clone()),
            state: HashMap::new(),
        })
        .await
        .with_context(|| {
            format!(
                "failed to create session '{}' for app '{}'",
                cfg.session_id, cfg.app_name
            )
        })?;

    Ok(())
}

fn ensure_parent_dir_for_sqlite_url(db_url: &str) -> Result<()> {
    let Some(db_path) = sqlite_path_from_url(db_url) else {
        return Ok(());
    };

    if let Some(parent) = db_path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent).with_context(|| {
            format!(
                "failed to create directory for sqlite database: {}",
                parent.display()
            )
        })?;
    }

    if !db_path.exists() {
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&db_path)
            .with_context(|| {
                format!(
                    "failed to initialize sqlite database file: {}",
                    db_path.display()
                )
            })?;
    }

    Ok(())
}

fn sqlite_path_from_url(db_url: &str) -> Option<PathBuf> {
    if !db_url.starts_with("sqlite://") {
        return None;
    }

    let path_with_params = db_url.trim_start_matches("sqlite://");
    let path_without_params = path_with_params
        .split_once('?')
        .map(|(path, _)| path)
        .unwrap_or(path_with_params);

    if path_without_params.is_empty() || path_without_params == ":memory:" {
        return None;
    }

    Some(Path::new(path_without_params).to_path_buf())
}

const NO_TEXTUAL_RESPONSE: &str = "No textual response produced by the agent.";

#[derive(Default, Debug)]
struct AuthorTextTracker {
    latest_final_text: Option<String>,
    latest_final_author: Option<String>,
    last_textful_author: Option<String>,
    by_author: HashMap<String, String>,
}

impl AuthorTextTracker {
    fn ingest_parts(&mut self, author: &str, text: &str, partial: bool, is_final: bool) -> String {
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

    fn resolve_text(&self) -> Option<String> {
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

fn ingest_author_text(buffer: &mut String, text: &str, partial: bool, is_final: bool) -> String {
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

fn suffix_prefix_overlap(existing: &str, incoming: &str) -> usize {
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

fn final_stream_suffix(emitted: &str, final_text: &str) -> Option<String> {
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

async fn run_prompt(
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
        let event = event_result.context("runner returned event error")?;
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

async fn run_prompt_with_retrieval(
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

#[derive(Debug, Clone, PartialEq, Eq)]
enum ChatCommand {
    Exit,
    Status,
    Help,
    Tools,
    Mcp,
    Usage,
    Provider(String),
    Model(Option<String>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ParsedChatCommand {
    NotACommand,
    Command(ChatCommand),
    MissingArgument { usage: &'static str },
    UnknownCommand(String),
}

fn parse_chat_command(input: &str) -> ParsedChatCommand {
    let trimmed = input.trim();

    if trimmed.eq_ignore_ascii_case("exit") || trimmed.eq_ignore_ascii_case("/exit") {
        return ParsedChatCommand::Command(ChatCommand::Exit);
    }

    if !trimmed.starts_with('/') {
        return ParsedChatCommand::NotACommand;
    }

    let slashless = trimmed.trim_start_matches('/');
    if slashless.is_empty() {
        return ParsedChatCommand::UnknownCommand("/".to_string());
    }

    let mut parts = slashless.splitn(2, char::is_whitespace);
    let command = parts
        .next()
        .map(|value| value.to_ascii_lowercase())
        .unwrap_or_default();
    let arg = parts.next().map(str::trim).unwrap_or_default();

    match command.as_str() {
        "exit" => ParsedChatCommand::Command(ChatCommand::Exit),
        "status" => ParsedChatCommand::Command(ChatCommand::Status),
        "help" => ParsedChatCommand::Command(ChatCommand::Help),
        "tools" => ParsedChatCommand::Command(ChatCommand::Tools),
        "mcp" => ParsedChatCommand::Command(ChatCommand::Mcp),
        "usage" => ParsedChatCommand::Command(ChatCommand::Usage),
        "provider" => {
            if arg.is_empty() {
                ParsedChatCommand::MissingArgument {
                    usage: "/provider <auto|gemini|openai|anthropic|deepseek|groq|ollama>",
                }
            } else {
                ParsedChatCommand::Command(ChatCommand::Provider(arg.to_string()))
            }
        }
        "model" => {
            if arg.is_empty() {
                ParsedChatCommand::Command(ChatCommand::Model(None))
            } else {
                ParsedChatCommand::Command(ChatCommand::Model(Some(arg.to_string())))
            }
        }
        other => ParsedChatCommand::UnknownCommand(format!("/{other}")),
    }
}

fn print_chat_help() {
    println!("Chat commands:");
    println!("- /help: show command quick reference");
    println!("- /status: show active profile/provider/model/session");
    println!("- /provider <name>: switch provider and rebuild runtime");
    println!("- /model [id]: pick a model interactively or switch directly by id");
    println!("- /tools: show active built-in/MCP tools and confirmation policy");
    println!("- /mcp: show MCP server and tool summary");
    println!("- /usage: show usage examples");
    println!("- /exit: end interactive chat");
}

fn print_chat_usage() {
    println!("Usage examples:");
    println!("- Type plain text to send a prompt to the agent.");
    println!("- /provider openai");
    println!("- /model");
    println!("- /model gpt-4o-mini");
    println!("- /tools");
    println!("- /mcp");
    println!("- /status");
    println!("- /exit");
}

#[derive(Debug, Clone, Copy)]
struct ModelPickerOption {
    id: &'static str,
    context_window: &'static str,
    description: &'static str,
}

fn model_picker_options(provider: Provider) -> Vec<ModelPickerOption> {
    match provider {
        Provider::Gemini => vec![
            ModelPickerOption {
                id: "gemini-2.5-flash",
                context_window: "1M",
                description: "fast balanced default",
            },
            ModelPickerOption {
                id: "gemini-2.5-pro",
                context_window: "1M",
                description: "higher reasoning depth",
            },
        ],
        Provider::Openai => vec![
            ModelPickerOption {
                id: "gpt-4o-mini",
                context_window: "128k",
                description: "low-latency default",
            },
            ModelPickerOption {
                id: "gpt-4.1",
                context_window: "1M",
                description: "higher quality general reasoning",
            },
            ModelPickerOption {
                id: "o3-mini",
                context_window: "200k",
                description: "reasoning-focused",
            },
        ],
        Provider::Anthropic => vec![
            ModelPickerOption {
                id: "claude-sonnet-4-20250514",
                context_window: "200k",
                description: "balanced default",
            },
            ModelPickerOption {
                id: "claude-3-5-haiku-latest",
                context_window: "200k",
                description: "fast lower-latency option",
            },
        ],
        Provider::Deepseek => vec![
            ModelPickerOption {
                id: "deepseek-chat",
                context_window: "64k",
                description: "general conversation default",
            },
            ModelPickerOption {
                id: "deepseek-reasoner",
                context_window: "64k",
                description: "reasoning-focused",
            },
        ],
        Provider::Groq => vec![
            ModelPickerOption {
                id: "llama-3.3-70b-versatile",
                context_window: "128k",
                description: "balanced default",
            },
            ModelPickerOption {
                id: "mixtral-8x7b-32768",
                context_window: "32k",
                description: "fast throughput option",
            },
        ],
        Provider::Ollama => vec![
            ModelPickerOption {
                id: "llama3.2",
                context_window: "local-configured",
                description: "default local model",
            },
            ModelPickerOption {
                id: "qwen2.5-coder",
                context_window: "local-configured",
                description: "coding-optimized local model",
            },
        ],
        Provider::Auto => Vec::new(),
    }
}

fn resolve_model_picker_selection(
    options: &[ModelPickerOption],
    selection: &str,
) -> Result<Option<String>> {
    if options.is_empty() {
        return Ok(None);
    }

    let trimmed = selection.trim();
    if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("cancel") {
        return Ok(None);
    }

    if let Ok(index) = trimmed.parse::<usize>() {
        if index == 0 || index > options.len() {
            return Err(anyhow::anyhow!(
                "invalid selection '{}'; expected 1-{}",
                trimmed,
                options.len()
            ));
        }
        return Ok(Some(options[index - 1].id.to_string()));
    }

    if let Some(option) = options
        .iter()
        .find(|option| option.id.eq_ignore_ascii_case(trimmed))
    {
        return Ok(Some(option.id.to_string()));
    }

    Ok(Some(trimmed.to_string()))
}

fn prompt_model_picker(provider: Provider, current_model: &str) -> Result<Option<String>> {
    let options = model_picker_options(provider);
    if options.is_empty() {
        println!(
            "Model picker catalog unavailable for provider {:?}. Use /model <model-id>.",
            provider
        );
        return Ok(None);
    }

    println!(
        "Model picker: provider={:?} active_model={}",
        provider, current_model
    );
    for (idx, option) in options.iter().enumerate() {
        println!(
            "{}. {} (ctx={}, {})",
            idx + 1,
            option.id,
            option.context_window,
            option.description
        );
    }
    print!("Select model number or id (Enter to cancel): ");
    io::stdout().flush().context("failed to flush stdout")?;

    let mut selection = String::new();
    io::stdin()
        .read_line(&mut selection)
        .context("failed to read model picker input")?;
    resolve_model_picker_selection(&options, &selection)
}

fn print_chat_tools(
    cfg: &RuntimeConfig,
    runtime_tools: &ResolvedRuntimeTools,
    tool_confirmation: &ToolConfirmationSettings,
) {
    let mut built_in_tools = Vec::<String>::new();
    let mut mcp_tools = Vec::<String>::new();

    for tool in &runtime_tools.tools {
        let name = tool.name().to_string();
        if runtime_tools.mcp_tool_names.contains(&name) {
            mcp_tools.push(name);
        } else {
            built_in_tools.push(name);
        }
    }

    built_in_tools.sort();
    built_in_tools.dedup();
    mcp_tools.sort();
    mcp_tools.dedup();

    let required_count = tool_confirmation
        .run_config
        .tool_confirmation_decisions
        .len();
    let approved_count = tool_confirmation
        .run_config
        .tool_confirmation_decisions
        .values()
        .filter(|decision| matches!(decision, ToolConfirmationDecision::Approve))
        .count();

    println!(
        "Tools: total={} built_in={} mcp={}",
        runtime_tools.tools.len(),
        built_in_tools.len(),
        mcp_tools.len()
    );
    println!("Tool confirmation mode: {:?}", cfg.tool_confirmation_mode);
    println!(
        "Confirmation decisions: required={} approved={} denied={}",
        required_count,
        approved_count,
        required_count.saturating_sub(approved_count)
    );
    println!(
        "Built-in tools: {}",
        if built_in_tools.is_empty() {
            "<none>".to_string()
        } else {
            built_in_tools.join(", ")
        }
    );
    println!(
        "MCP tools: {}",
        if mcp_tools.is_empty() {
            "<none>".to_string()
        } else {
            mcp_tools.join(", ")
        }
    );
}

fn print_chat_mcp(cfg: &RuntimeConfig, runtime_tools: &ResolvedRuntimeTools) {
    let enabled_servers = cfg
        .mcp_servers
        .iter()
        .filter(|server| server.enabled.unwrap_or(true))
        .count();
    let mut server_names = cfg
        .mcp_servers
        .iter()
        .filter(|server| server.enabled.unwrap_or(true))
        .map(|server| server.name.clone())
        .collect::<Vec<String>>();
    server_names.sort();

    println!(
        "MCP: configured_servers={} enabled_servers={} discovered_tools={}",
        cfg.mcp_servers.len(),
        enabled_servers,
        runtime_tools.mcp_tool_names.len()
    );
    println!(
        "Enabled MCP servers: {}",
        if server_names.is_empty() {
            "<none>".to_string()
        } else {
            server_names.join(", ")
        }
    );
    println!(
        "Discovered MCP tools: {}",
        if runtime_tools.mcp_tool_names.is_empty() {
            "<none>".to_string()
        } else {
            runtime_tools
                .mcp_tool_names
                .iter()
                .cloned()
                .collect::<Vec<String>>()
                .join(", ")
        }
    );
}

enum ChatCommandAction {
    Continue,
    Exit,
}

async fn dispatch_chat_command(
    command: ChatCommand,
    cfg: &mut RuntimeConfig,
    runner: &mut Runner,
    resolved_provider: &mut Provider,
    model_name: &mut String,
    session_service: &Arc<dyn SessionService>,
    runtime_tools: &ResolvedRuntimeTools,
    tool_confirmation: &ToolConfirmationSettings,
    telemetry: &TelemetrySink,
) -> Result<ChatCommandAction> {
    match command {
        ChatCommand::Exit => Ok(ChatCommandAction::Exit),
        ChatCommand::Status => {
            println!(
                "profile={} provider={:?} model={} session_id={}",
                cfg.profile, resolved_provider, model_name, cfg.session_id
            );
            Ok(ChatCommandAction::Continue)
        }
        ChatCommand::Help => {
            print_chat_help();
            Ok(ChatCommandAction::Continue)
        }
        ChatCommand::Usage => {
            print_chat_usage();
            Ok(ChatCommandAction::Continue)
        }
        ChatCommand::Tools => {
            print_chat_tools(cfg, runtime_tools, tool_confirmation);
            Ok(ChatCommandAction::Continue)
        }
        ChatCommand::Mcp => {
            print_chat_mcp(cfg, runtime_tools);
            Ok(ChatCommandAction::Continue)
        }
        ChatCommand::Provider(provider_name) => {
            let new_provider = parse_provider_name(&provider_name)?;
            let mut switched_cfg = cfg.clone();
            switched_cfg.provider = new_provider;
            switched_cfg.model = None;

            match build_single_runner_for_chat(
                &switched_cfg,
                session_service.clone(),
                runtime_tools,
                tool_confirmation,
                telemetry,
            )
            .await
            {
                Ok((new_runner, new_resolved_provider, new_model_name)) => {
                    *runner = new_runner;
                    *resolved_provider = new_resolved_provider;
                    *model_name = new_model_name;
                    telemetry.emit(
                        "chat.provider_switched",
                        json!({
                            "provider": format!("{:?}", resolved_provider).to_ascii_lowercase(),
                            "model": model_name.clone()
                        }),
                    );
                    switched_cfg.provider = *resolved_provider;
                    switched_cfg.model = Some(model_name.clone());
                    *cfg = switched_cfg;
                    tracing::info!(
                        provider = ?resolved_provider,
                        model = %model_name,
                        "Switched model provider"
                    );
                    println!(
                        "Switched provider to {:?} (model={}). Session continuity preserved.",
                        resolved_provider, model_name
                    );
                }
                Err(err) => {
                    eprintln!("{}", format_cli_error(&err, cfg.show_sensitive_config));
                    println!(
                        "Provider remains {:?} (model={}).",
                        resolved_provider, model_name
                    );
                }
            }

            Ok(ChatCommandAction::Continue)
        }
        ChatCommand::Model(next_model) => {
            let chosen_model = match next_model {
                Some(value) => Some(value),
                None => prompt_model_picker(*resolved_provider, model_name)?,
            };
            let Some(chosen_model) = chosen_model else {
                println!(
                    "Model unchanged ('{}' on provider {:?}).",
                    model_name, resolved_provider
                );
                return Ok(ChatCommandAction::Continue);
            };

            let mut switched_cfg = cfg.clone();
            switched_cfg.model = Some(chosen_model);

            match build_single_runner_for_chat(
                &switched_cfg,
                session_service.clone(),
                runtime_tools,
                tool_confirmation,
                telemetry,
            )
            .await
            {
                Ok((new_runner, new_resolved_provider, new_model_name)) => {
                    *runner = new_runner;
                    *resolved_provider = new_resolved_provider;
                    *model_name = new_model_name;
                    telemetry.emit(
                        "chat.model_switched",
                        json!({
                            "provider": format!("{:?}", resolved_provider).to_ascii_lowercase(),
                            "model": model_name.clone()
                        }),
                    );
                    switched_cfg.provider = *resolved_provider;
                    switched_cfg.model = Some(model_name.clone());
                    *cfg = switched_cfg;
                    tracing::info!(
                        provider = ?resolved_provider,
                        model = %model_name,
                        "Switched model"
                    );
                    println!(
                        "Switched model to '{}' on provider {:?}. Session continuity preserved.",
                        model_name, resolved_provider
                    );
                }
                Err(err) => {
                    eprintln!("{}", format_cli_error(&err, cfg.show_sensitive_config));
                    println!(
                        "Model remains '{}' on provider {:?}.",
                        model_name, resolved_provider
                    );
                }
            }

            Ok(ChatCommandAction::Continue)
        }
    }
}

async fn run_chat(
    mut cfg: RuntimeConfig,
    retrieval_service: Arc<dyn RetrievalService>,
    runtime_tools: ResolvedRuntimeTools,
    tool_confirmation: ToolConfirmationSettings,
    telemetry: &TelemetrySink,
) -> Result<()> {
    let session_service = build_session_service(&cfg).await?;
    let (mut runner, mut resolved_provider, mut model_name) = build_single_runner_for_chat(
        &cfg,
        session_service.clone(),
        &runtime_tools,
        &tool_confirmation,
        telemetry,
    )
    .await?;

    cfg.provider = resolved_provider;
    cfg.model = Some(model_name.clone());

    telemetry.emit(
        "chat.started",
        json!({
            "provider": format!("{:?}", resolved_provider).to_ascii_lowercase(),
            "model": model_name.clone(),
            "profile": cfg.profile.clone()
        }),
    );

    tracing::info!(provider = ?resolved_provider, model = %model_name, "Using model");
    println!("Interactive mode started. Type /help for commands or /exit to quit.");
    println!("Quick start: /provider <name>, /model (picker), /tools, /mcp, /usage.");
    if buffered_output_required(cfg.guardrail_output_mode) {
        println!(
            "Guardrail output mode {:?} active: chat will buffer model responses before printing.",
            cfg.guardrail_output_mode
        );
    }
    let stdin = io::stdin();
    let mut line = String::new();

    loop {
        print!("zavora> ");
        io::stdout().flush().context("failed to flush stdout")?;
        line.clear();
        stdin
            .read_line(&mut line)
            .context("failed to read input from stdin")?;
        let input = line.trim();
        if input.eq_ignore_ascii_case("/exit") || input.eq_ignore_ascii_case("exit") {
            break;
        }
        if input.is_empty() {
            continue;
        }

        match parse_chat_command(input) {
            ParsedChatCommand::NotACommand => {}
            ParsedChatCommand::MissingArgument { usage } => {
                println!("Usage: {usage}");
                continue;
            }
            ParsedChatCommand::UnknownCommand(command) => {
                println!("Unknown command '{command}'. Use /help.");
                continue;
            }
            ParsedChatCommand::Command(command) => {
                let action = dispatch_chat_command(
                    command,
                    &mut cfg,
                    &mut runner,
                    &mut resolved_provider,
                    &mut model_name,
                    &session_service,
                    &runtime_tools,
                    &tool_confirmation,
                    telemetry,
                )
                .await?;
                if matches!(action, ChatCommandAction::Exit) {
                    break;
                }
                continue;
            }
        }

        let guarded_input =
            match apply_guardrail(&cfg, telemetry, "input", cfg.guardrail_input_mode, input) {
                Ok(text) => text,
                Err(err) => {
                    eprintln!("{}", format_cli_error(&err, cfg.show_sensitive_config));
                    continue;
                }
            };

        if buffered_output_required(cfg.guardrail_output_mode) {
            let answer = run_prompt_with_retrieval(
                &runner,
                &cfg,
                &guarded_input,
                retrieval_service.as_ref(),
                telemetry,
            )
            .await?;
            let answer = match apply_guardrail(
                &cfg,
                telemetry,
                "output",
                cfg.guardrail_output_mode,
                &answer,
            ) {
                Ok(text) => text,
                Err(err) => {
                    eprintln!("{}", format_cli_error(&err, cfg.show_sensitive_config));
                    continue;
                }
            };
            println!("{answer}");
        } else {
            let answer = run_prompt_streaming_with_retrieval(
                &runner,
                &cfg,
                &guarded_input,
                retrieval_service.as_ref(),
                telemetry,
            )
            .await?;
            if matches!(cfg.guardrail_output_mode, GuardrailMode::Observe)
                && let Err(err) = apply_guardrail(
                    &cfg,
                    telemetry,
                    "output",
                    cfg.guardrail_output_mode,
                    &answer,
                )
            {
                eprintln!("{}", format_cli_error(&err, cfg.show_sensitive_config));
            }
        }
    }

    Ok(())
}

fn parse_provider_name(value: &str) -> Result<Provider> {
    Provider::from_str(value, true)
        .map_err(|_| {
            anyhow::anyhow!(
                "invalid provider '{}'. Supported values: auto, gemini, openai, anthropic, deepseek, groq, ollama",
                value
            )
        })
}

async fn run_prompt_streaming(
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

    while let Some(event_result) = stream.next().await {
        let event = event_result.context("runner returned event error")?;
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
            print!("{delta}");
            io::stdout().flush().context("failed to flush stdout")?;
            emitted_text_by_author
                .entry(event.author.clone())
                .or_default()
                .push_str(&delta);
            printed_any_output = true;
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

async fn run_prompt_streaming_with_retrieval(
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

fn event_text(event: &Event) -> String {
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

fn extract_tool_failure_message(response: &Value) -> Option<String> {
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

fn emit_tool_lifecycle_events(event: &Event, telemetry: &TelemetrySink) {
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
                    json!({
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
                        json!({
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
                        json!({
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

fn validate_model_for_provider(provider: Provider, model_name: &str) -> Result<()> {
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

fn resolve_model(cfg: &RuntimeConfig) -> Result<(Arc<dyn Llm>, Provider, String)> {
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
                .unwrap_or_else(|| "gpt-4o-mini".to_string());
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
            let model_name = cfg.model.clone().unwrap_or_else(|| "llama3.2".to_string());
            validate_model_for_provider(provider, &model_name)?;
            let model = OllamaModel::new(OllamaConfig::with_host(host, model_name.clone()))?;
            Ok((Arc::new(model), provider, model_name))
        }
        Provider::Auto => unreachable!("auto provider must be resolved before matching"),
    }
}

fn detect_provider() -> Option<Provider> {
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

fn env_present(key: &str) -> bool {
    std::env::var(key)
        .map(|v| !v.trim().is_empty())
        .unwrap_or(false)
}

async fn run_doctor(cfg: &RuntimeConfig) -> Result<()> {
    println!(
        "Active profile: '{}' (config: {})",
        cfg.profile, cfg.config_path
    );

    let checks = [
        ("GOOGLE_API_KEY", env_present("GOOGLE_API_KEY")),
        ("OPENAI_API_KEY", env_present("OPENAI_API_KEY")),
        ("ANTHROPIC_API_KEY", env_present("ANTHROPIC_API_KEY")),
        ("DEEPSEEK_API_KEY", env_present("DEEPSEEK_API_KEY")),
        ("GROQ_API_KEY", env_present("GROQ_API_KEY")),
        ("OLLAMA_HOST", env_present("OLLAMA_HOST")),
    ];

    println!("Provider environment check:");
    for (key, ok) in checks {
        let status = if ok { "set" } else { "missing" };
        println!("- {key}: {status}");
    }

    match detect_provider() {
        Some(provider) => println!("Auto provider resolution: {:?}", provider),
        None => {
            println!("Auto provider resolution: none");
            println!("Tip: export one provider key or run with --provider ollama");
        }
    }

    println!(
        "Session backend: {:?} (session_id: {}, app: {}, user: {})",
        cfg.session_backend, cfg.session_id, cfg.app_name, cfg.user_id
    );
    println!(
        "Agent: {} (source={}) model_override={} resources={}",
        cfg.agent_name,
        cfg.agent_source.label(),
        cfg.model.as_deref().unwrap_or("<provider-default>"),
        cfg.agent_resource_paths.len()
    );
    println!(
        "Retrieval: backend={:?}, doc_path={}, max_chunks={}, max_chars={}, min_score={}",
        cfg.retrieval_backend,
        cfg.retrieval_doc_path
            .as_deref()
            .unwrap_or("<not configured>"),
        cfg.retrieval_max_chunks,
        cfg.retrieval_max_chars,
        cfg.retrieval_min_score
    );
    println!(
        "Tool confirmation: mode={:?}, required_tools={}, approved_tools={}, timeout_secs={}, retry_attempts={}, retry_delay_ms={}",
        cfg.tool_confirmation_mode,
        cfg.require_confirm_tool.len(),
        cfg.approve_tool.len(),
        cfg.tool_timeout_secs,
        cfg.tool_retry_attempts,
        cfg.tool_retry_delay_ms
    );
    println!(
        "Telemetry: enabled={} path={}",
        cfg.telemetry_enabled, cfg.telemetry_path
    );
    println!(
        "Guardrails: input_mode={:?} output_mode={:?} terms={} redact_replacement={}",
        cfg.guardrail_input_mode,
        cfg.guardrail_output_mode,
        cfg.guardrail_terms.len(),
        cfg.guardrail_redact_replacement
    );
    println!(
        "MCP servers: configured={}, enabled={}",
        cfg.mcp_servers.len(),
        cfg.mcp_servers
            .iter()
            .filter(|server| server.enabled.unwrap_or(true))
            .count()
    );

    if matches!(cfg.session_backend, SessionBackend::Sqlite) {
        let _service = open_sqlite_session_service(&cfg.session_db_url).await?;
        println!(
            "SQLite session DB check: ok ({})",
            display_session_db_url(cfg)
        );
    }

    Ok(())
}

async fn run_migrate(cfg: &RuntimeConfig) -> Result<()> {
    match cfg.session_backend {
        SessionBackend::Memory => {
            println!("Session backend is memory; no migration required.");
        }
        SessionBackend::Sqlite => {
            let _service = open_sqlite_session_service(&cfg.session_db_url).await?;
            println!(
                "SQLite migrations applied successfully: {}",
                display_session_db_url(cfg)
            );
        }
    }
    Ok(())
}

async fn run_sessions_list(cfg: &RuntimeConfig) -> Result<()> {
    let session_service = build_session_service(cfg).await?;
    let mut sessions = session_service
        .list(ListRequest {
            app_name: cfg.app_name.clone(),
            user_id: cfg.user_id.clone(),
        })
        .await
        .with_context(|| {
            format!(
                "failed to list sessions for app '{}' and user '{}'",
                cfg.app_name, cfg.user_id
            )
        })?;

    if sessions.is_empty() {
        println!(
            "No sessions found for app '{}' and user '{}'.",
            cfg.app_name, cfg.user_id
        );
        return Ok(());
    }

    sessions.sort_by_key(|session| std::cmp::Reverse(session.last_update_time()));

    println!(
        "Sessions for app '{}' and user '{}':",
        cfg.app_name, cfg.user_id
    );
    for session in sessions {
        println!(
            "- {} (updated: {})",
            session.id(),
            session.last_update_time().to_rfc3339()
        );
    }

    Ok(())
}

async fn run_sessions_show(
    cfg: &RuntimeConfig,
    session_id_override: Option<String>,
    recent: usize,
) -> Result<()> {
    let session_id = session_id_override.unwrap_or_else(|| cfg.session_id.clone());
    let session_service = build_session_service(cfg).await?;
    let session = session_service
        .get(GetRequest {
            app_name: cfg.app_name.clone(),
            user_id: cfg.user_id.clone(),
            session_id: session_id.clone(),
            num_recent_events: (recent > 0).then_some(recent),
            after: None,
        })
        .await
        .with_context(|| {
            format!(
                "failed to load session '{}' for app '{}' and user '{}'",
                session_id, cfg.app_name, cfg.user_id
            )
        })?;

    println!(
        "Session '{}' (app='{}', user='{}', events={}):",
        session.id(),
        session.app_name(),
        session.user_id(),
        session.events().len()
    );

    let events = session.events().all();
    if events.is_empty() {
        println!("No events in this session.");
        return Ok(());
    }

    for event in events {
        print_session_event(&event);
    }

    Ok(())
}

async fn run_sessions_delete(
    cfg: &RuntimeConfig,
    session_id_override: Option<String>,
    force: bool,
) -> Result<()> {
    let session_id = session_id_override.unwrap_or_else(|| cfg.session_id.clone());
    if !force {
        return Err(anyhow::anyhow!(
            "session delete is destructive. Re-run with --force to delete session '{}'",
            session_id
        ));
    }

    let session_service = build_session_service(cfg).await?;
    session_service
        .delete(DeleteRequest {
            app_name: cfg.app_name.clone(),
            user_id: cfg.user_id.clone(),
            session_id: session_id.clone(),
        })
        .await
        .with_context(|| {
            format!(
                "failed to delete session '{}' for app '{}' and user '{}'",
                session_id, cfg.app_name, cfg.user_id
            )
        })?;

    println!(
        "Deleted session '{}' for app '{}' and user '{}'.",
        session_id, cfg.app_name, cfg.user_id
    );
    Ok(())
}

async fn run_sessions_prune(
    cfg: &RuntimeConfig,
    keep: usize,
    dry_run: bool,
    force: bool,
) -> Result<()> {
    let keep = keep.max(1);
    let session_service = build_session_service(cfg).await?;
    let mut sessions = session_service
        .list(ListRequest {
            app_name: cfg.app_name.clone(),
            user_id: cfg.user_id.clone(),
        })
        .await
        .with_context(|| {
            format!(
                "failed to list sessions for prune in app '{}' and user '{}'",
                cfg.app_name, cfg.user_id
            )
        })?;

    sessions.sort_by_key(|session| std::cmp::Reverse(session.last_update_time()));
    let prune_ids = sessions
        .into_iter()
        .skip(keep)
        .map(|session| session.id().to_string())
        .collect::<Vec<String>>();

    if prune_ids.is_empty() {
        println!(
            "Nothing to prune. Keep={} and current session count is within limit.",
            keep
        );
        return Ok(());
    }

    if dry_run {
        println!(
            "Dry-run: {} session(s) would be deleted (keeping {} most recent):",
            prune_ids.len(),
            keep
        );
        for id in prune_ids {
            println!("- {id}");
        }
        return Ok(());
    }

    if !force {
        return Err(anyhow::anyhow!(
            "session prune is destructive and would delete {} session(s). Re-run with --force or preview with --dry-run",
            prune_ids.len()
        ));
    }

    for session_id in &prune_ids {
        session_service
            .delete(DeleteRequest {
                app_name: cfg.app_name.clone(),
                user_id: cfg.user_id.clone(),
                session_id: session_id.clone(),
            })
            .await
            .with_context(|| {
                format!(
                    "failed to delete pruned session '{}' for app '{}' and user '{}'",
                    session_id, cfg.app_name, cfg.user_id
                )
            })?;
    }

    println!(
        "Pruned {} session(s). Kept {} most recent session(s).",
        prune_ids.len(),
        keep
    );
    Ok(())
}

fn print_session_event(event: &Event) {
    let mut header = format!("[{}] {}", event.timestamp.to_rfc3339(), event.author);
    if event.is_final_response() {
        header.push_str(" [final]");
    }
    println!("{header}");

    let text = event_text(event);
    if !text.is_empty() {
        println!("{text}");
    } else {
        println!("<non-text event>");
    }

    if !event.actions.state_delta.is_empty() {
        let mut keys = event
            .actions
            .state_delta
            .keys()
            .cloned()
            .collect::<Vec<String>>();
        keys.sort();
        println!("state_delta keys: {}", keys.join(", "));
    }

    println!();
}

#[cfg(test)]
mod tests {
    use super::*;
    use adk_rust::LlmResponse;
    use adk_rust::model::MockLlm;
    use tempfile::tempdir;

    fn base_cfg() -> RuntimeConfig {
        RuntimeConfig {
            profile: "default".to_string(),
            config_path: ".zavora/config.toml".to_string(),
            agent_name: "default".to_string(),
            agent_source: AgentSource::Implicit,
            agent_description: Some("Built-in default assistant".to_string()),
            agent_instruction: None,
            agent_resource_paths: Vec::new(),
            agent_allow_tools: Vec::new(),
            agent_deny_tools: Vec::new(),
            provider: Provider::Auto,
            model: None,
            app_name: "test-app".to_string(),
            user_id: "test-user".to_string(),
            session_id: "test-session".to_string(),
            session_backend: SessionBackend::Memory,
            session_db_url: "sqlite://.zavora/test.db".to_string(),
            show_sensitive_config: false,
            retrieval_backend: RetrievalBackend::Disabled,
            retrieval_doc_path: None,
            retrieval_max_chunks: 3,
            retrieval_max_chars: 4000,
            retrieval_min_score: 1,
            tool_confirmation_mode: ToolConfirmationMode::McpOnly,
            require_confirm_tool: Vec::new(),
            approve_tool: Vec::new(),
            tool_timeout_secs: 45,
            tool_retry_attempts: 2,
            tool_retry_delay_ms: 500,
            telemetry_enabled: false,
            telemetry_path: ".zavora/test-telemetry.jsonl".to_string(),
            guardrail_input_mode: GuardrailMode::Disabled,
            guardrail_output_mode: GuardrailMode::Disabled,
            guardrail_terms: vec!["secret".to_string(), "password".to_string()],
            guardrail_redact_replacement: "[REDACTED]".to_string(),
            mcp_servers: Vec::new(),
        }
    }

    fn test_telemetry(cfg: &RuntimeConfig) -> TelemetrySink {
        TelemetrySink::new(cfg, "test".to_string())
    }

    fn mock_model(text: &str) -> Arc<dyn Llm> {
        Arc::new(
            MockLlm::new("mock")
                .with_response(LlmResponse::new(Content::new("model").with_text(text))),
        )
    }

    fn noop_tool(name: &str) -> Arc<dyn Tool> {
        Arc::new(FunctionTool::new(
            name,
            "noop tool",
            |_ctx, _args| async move { Ok(json!({"ok": true})) },
        ))
    }

    fn make_runtime_tools(tool_names: &[&str], mcp_tool_names: &[&str]) -> ResolvedRuntimeTools {
        ResolvedRuntimeTools {
            tools: tool_names
                .iter()
                .map(|name| noop_tool(name))
                .collect::<Vec<_>>(),
            mcp_tool_names: mcp_tool_names
                .iter()
                .map(|name| name.to_string())
                .collect::<BTreeSet<String>>(),
        }
    }

    fn sqlite_cfg(session_id: &str) -> (tempfile::TempDir, RuntimeConfig) {
        let dir = tempdir().expect("temp directory should create");
        let db_path = dir.path().join("sessions.db");
        let db_url = format!("sqlite://{}", db_path.to_string_lossy());

        let mut cfg = base_cfg();
        cfg.session_backend = SessionBackend::Sqlite;
        cfg.session_db_url = db_url;
        cfg.session_id = session_id.to_string();

        (dir, cfg)
    }

    fn test_cli(config_path: &str, profile: &str) -> Cli {
        Cli {
            provider: Provider::Auto,
            model: None,
            agent: None,
            profile: profile.to_string(),
            config_path: config_path.to_string(),
            app_name: None,
            user_id: None,
            session_id: None,
            session_backend: None,
            session_db_url: None,
            show_sensitive_config: false,
            retrieval_backend: None,
            retrieval_doc_path: None,
            retrieval_max_chunks: None,
            retrieval_max_chars: None,
            retrieval_min_score: None,
            tool_confirmation_mode: None,
            require_confirm_tool: Vec::new(),
            approve_tool: Vec::new(),
            tool_timeout_secs: None,
            tool_retry_attempts: None,
            tool_retry_delay_ms: None,
            telemetry_enabled: None,
            telemetry_path: None,
            guardrail_input_mode: None,
            guardrail_output_mode: None,
            guardrail_term: Vec::new(),
            guardrail_redact_replacement: None,
            log_filter: "warn".to_string(),
            command: Commands::Doctor,
        }
    }

    fn test_execute_bash_request(command: &str) -> ExecuteBashRequest {
        ExecuteBashRequest {
            command: command.to_string(),
            approved: false,
            allow_dangerous: false,
            timeout_secs: EXECUTE_BASH_DEFAULT_TIMEOUT_SECS,
            retry_attempts: EXECUTE_BASH_DEFAULT_RETRY_ATTEMPTS,
            retry_delay_ms: 0,
            max_output_chars: EXECUTE_BASH_DEFAULT_MAX_OUTPUT_CHARS,
        }
    }

    async fn create_session(cfg: &RuntimeConfig, session_id: &str) {
        let service = build_session_service(cfg)
            .await
            .expect("service should build");
        service
            .create(CreateRequest {
                app_name: cfg.app_name.clone(),
                user_id: cfg.user_id.clone(),
                session_id: Some(session_id.to_string()),
                state: HashMap::new(),
            })
            .await
            .expect("session should create");
    }

    async fn list_session_ids(cfg: &RuntimeConfig) -> Vec<String> {
        let service = build_session_service(cfg)
            .await
            .expect("service should build");
        let mut sessions = service
            .list(ListRequest {
                app_name: cfg.app_name.clone(),
                user_id: cfg.user_id.clone(),
            })
            .await
            .expect("sessions should list")
            .into_iter()
            .map(|s| s.id().to_string())
            .collect::<Vec<String>>();
        sessions.sort();
        sessions
    }

    #[tokio::test]
    async fn single_workflow_returns_deterministic_mock_output() {
        let cfg = base_cfg();
        let telemetry = test_telemetry(&cfg);
        let runner = build_runner(
            build_single_agent(mock_model("single response")).expect("agent should build"),
            &cfg,
        )
        .await
        .expect("runner should build");

        let out = run_prompt(&runner, &cfg, "hello", &telemetry)
            .await
            .expect("prompt should run");
        assert_eq!(out, "single response");
    }

    #[tokio::test]
    async fn workflow_modes_return_deterministic_mock_output() {
        let modes = [
            WorkflowMode::Single,
            WorkflowMode::Sequential,
            WorkflowMode::Parallel,
            WorkflowMode::Loop,
            WorkflowMode::Graph,
        ];

        for mode in modes {
            let mut cfg = base_cfg();
            cfg.session_id = format!("session-{mode:?}");
            let telemetry = test_telemetry(&cfg);
            let runner = build_runner(
                build_workflow_agent(
                    mode,
                    mock_model("workflow response"),
                    1,
                    &build_builtin_tools(),
                    ToolConfirmationPolicy::Never,
                    Duration::from_secs(45),
                    None,
                )
                .expect("workflow should build"),
                &cfg,
            )
            .await
            .expect("runner should build");

            let out = run_prompt(&runner, &cfg, "build a plan", &telemetry)
                .await
                .expect("prompt should run");
            assert_eq!(out, "workflow response");
        }
    }

    #[tokio::test]
    async fn sqlite_session_backend_persists_history_between_runners() {
        let dir = tempdir().expect("temp directory should create");
        let db_path = dir.path().join("sessions.db");
        let db_url = format!("sqlite://{}", db_path.to_string_lossy());

        let mut cfg = base_cfg();
        cfg.session_backend = SessionBackend::Sqlite;
        cfg.session_db_url = db_url.clone();
        cfg.session_id = "persisted-session".to_string();
        let telemetry = test_telemetry(&cfg);

        let runner_one = build_runner(
            build_single_agent(mock_model("first answer")).expect("agent should build"),
            &cfg,
        )
        .await
        .expect("runner should build");

        let _ = run_prompt(&runner_one, &cfg, "first prompt", &telemetry)
            .await
            .expect("first prompt should run");

        let runner_two = build_runner(
            build_single_agent(mock_model("second answer")).expect("agent should build"),
            &cfg,
        )
        .await
        .expect("second runner should build");

        let _ = run_prompt(&runner_two, &cfg, "second prompt", &telemetry)
            .await
            .expect("second prompt should run");

        let service = DatabaseSessionService::new(&db_url)
            .await
            .expect("db should open");
        service.migrate().await.expect("migration should run");

        let session = service
            .get(GetRequest {
                app_name: cfg.app_name.clone(),
                user_id: cfg.user_id.clone(),
                session_id: cfg.session_id.clone(),
                num_recent_events: None,
                after: None,
            })
            .await
            .expect("session should exist");

        assert!(
            session.events().len() >= 4,
            "expected persisted event history across runs"
        );
    }

    #[test]
    fn ingest_author_text_handles_partial_then_final_snapshot() {
        let mut buffer = String::new();

        let d1 = ingest_author_text(&mut buffer, "Hello", true, false);
        let d2 = ingest_author_text(&mut buffer, " world", true, false);
        let d3 = ingest_author_text(&mut buffer, "Hello world", false, true);

        assert_eq!(d1, "Hello");
        assert_eq!(d2, " world");
        assert!(d3.is_empty(), "final snapshot should not duplicate output");
        assert_eq!(buffer, "Hello world");
    }

    #[test]
    fn ingest_author_text_handles_non_partial_incremental_chunks() {
        let mut buffer = String::new();

        let d1 = ingest_author_text(&mut buffer, "Hello", false, false);
        let d2 = ingest_author_text(&mut buffer, " world", false, false);
        let d3 = ingest_author_text(&mut buffer, "Hello world", false, true);

        assert_eq!(d1, "Hello");
        assert_eq!(d2, " world");
        assert!(d3.is_empty(), "final snapshot should be deduplicated");
        assert_eq!(buffer, "Hello world");
    }

    #[test]
    fn tracker_falls_back_to_last_textful_author() {
        let mut tracker = AuthorTextTracker::default();

        let _ = tracker.ingest_parts("assistant", "hello", false, false);
        let _ = tracker.ingest_parts("tool", "", false, true);

        assert_eq!(tracker.resolve_text().as_deref(), Some("hello"));
    }

    #[test]
    fn final_stream_suffix_emits_only_missing_tail() {
        assert_eq!(
            final_stream_suffix("Hello", "Hello world").as_deref(),
            Some(" world")
        );
        assert_eq!(final_stream_suffix("Hello world", "Hello world"), None);
        assert_eq!(
            final_stream_suffix("", "Hello world").as_deref(),
            Some("Hello world")
        );
    }

    #[test]
    fn workflow_route_classifier_is_deterministic_for_key_intents() {
        assert_eq!(
            classify_workflow_route("Plan release milestones"),
            "release"
        );
        assert_eq!(
            classify_workflow_route("Evaluate architecture tradeoffs"),
            "architecture"
        );
        assert_eq!(
            classify_workflow_route("List risk mitigations and rollback"),
            "risk"
        );
        assert_eq!(
            classify_workflow_route("Implement feature work"),
            "delivery"
        );
    }

    #[test]
    fn workflow_templates_exist_for_all_graph_routes() {
        for route in ["release", "architecture", "risk", "delivery"] {
            let template = workflow_template(route);
            assert!(
                !template.trim().is_empty(),
                "template should be non-empty for route {route}"
            );
        }
    }

    #[test]
    fn tool_failure_extractor_handles_common_error_shapes() {
        assert_eq!(
            extract_tool_failure_message(&json!({"error": "denied by policy"})).as_deref(),
            Some("denied by policy")
        );
        assert_eq!(
            extract_tool_failure_message(&json!({"status": "error", "message": "timeout"}))
                .as_deref(),
            Some("timeout")
        );
        assert_eq!(extract_tool_failure_message(&json!({"ok": true})), None);
    }

    #[test]
    fn fs_read_reads_allowed_file_content() {
        let dir = tempdir().expect("temp directory should create");
        std::fs::write(dir.path().join("notes.txt"), "alpha\nbeta\ngamma\n")
            .expect("fixture file should write");
        let workspace_root = dir
            .path()
            .canonicalize()
            .expect("workspace root should resolve");

        let payload = fs_read_tool_response_with_root(
            &json!({
                "path": "notes.txt",
                "start_line": 2,
                "max_lines": 1
            }),
            &workspace_root,
        );

        assert_eq!(payload["status"], "ok");
        assert_eq!(payload["kind"], "file");
        assert_eq!(payload["content"], "beta");
        assert_eq!(payload["line_count"], 1);
    }

    #[test]
    fn fs_read_lists_directory_entries() {
        let dir = tempdir().expect("temp directory should create");
        std::fs::create_dir_all(dir.path().join("docs")).expect("fixture dir should create");
        std::fs::write(dir.path().join("README.md"), "hello").expect("fixture file should write");
        let workspace_root = dir
            .path()
            .canonicalize()
            .expect("workspace root should resolve");

        let payload = fs_read_tool_response_with_root(
            &json!({
                "path": ".",
                "max_entries": 10
            }),
            &workspace_root,
        );

        assert_eq!(payload["status"], "ok");
        assert_eq!(payload["kind"], "directory");
        assert_eq!(payload["entry_count"], 2);
        let entries = payload["entries"]
            .as_array()
            .expect("entries should be an array");
        assert!(
            entries
                .iter()
                .any(|entry| entry.get("name") == Some(&Value::String("README.md".to_string())))
        );
        assert!(
            entries
                .iter()
                .any(|entry| entry.get("name") == Some(&Value::String("docs".to_string())))
        );
    }

    #[test]
    fn fs_read_denies_blocked_paths() {
        let dir = tempdir().expect("temp directory should create");
        std::fs::write(dir.path().join(".env"), "OPENAI_API_KEY=test").expect("fixture file");
        let workspace_root = dir
            .path()
            .canonicalize()
            .expect("workspace root should resolve");

        let payload = fs_read_tool_response_with_root(&json!({ "path": ".env" }), &workspace_root);
        assert_eq!(payload["status"], "error");
        assert_eq!(payload["code"], "denied_path");
        assert!(
            extract_tool_failure_message(&payload)
                .as_deref()
                .unwrap_or_default()
                .contains("denied path")
        );
    }

    #[test]
    fn fs_read_reports_invalid_paths() {
        let dir = tempdir().expect("temp directory should create");
        let workspace_root = dir
            .path()
            .canonicalize()
            .expect("workspace root should resolve");
        let payload =
            fs_read_tool_response_with_root(&json!({ "path": "missing.txt" }), &workspace_root);
        assert_eq!(payload["status"], "error");
        assert_eq!(payload["code"], "invalid_path");
    }

    #[test]
    fn fs_write_creates_file_when_mode_is_create() {
        let dir = tempdir().expect("temp directory should create");
        let workspace_root = dir
            .path()
            .canonicalize()
            .expect("workspace root should resolve");

        let payload = fs_write_tool_response_with_root(
            &json!({
                "path": "docs/new.txt",
                "mode": "create",
                "content": "release-ready"
            }),
            &workspace_root,
        );

        assert_eq!(payload["status"], "ok");
        assert_eq!(payload["mode"], "create");
        let content = std::fs::read_to_string(dir.path().join("docs/new.txt"))
            .expect("created file should be readable");
        assert_eq!(content, "release-ready");
    }

    #[test]
    fn fs_write_patch_updates_existing_content() {
        let dir = tempdir().expect("temp directory should create");
        let workspace_root = dir
            .path()
            .canonicalize()
            .expect("workspace root should resolve");
        std::fs::write(dir.path().join("plan.md"), "Ship alpha then beta")
            .expect("fixture file should write");

        let payload = fs_write_tool_response_with_root(
            &json!({
                "path": "plan.md",
                "mode": "patch",
                "patch": {
                    "find": "beta",
                    "replace": "rc"
                }
            }),
            &workspace_root,
        );

        assert_eq!(payload["status"], "ok");
        assert_eq!(payload["mode"], "patch");
        assert_eq!(payload["replaced_count"], 1);
        let content =
            std::fs::read_to_string(dir.path().join("plan.md")).expect("patched file should read");
        assert_eq!(content, "Ship alpha then rc");
    }

    #[test]
    fn fs_write_denies_blocked_paths() {
        let dir = tempdir().expect("temp directory should create");
        let workspace_root = dir
            .path()
            .canonicalize()
            .expect("workspace root should resolve");

        let payload = fs_write_tool_response_with_root(
            &json!({
                "path": ".env",
                "mode": "overwrite",
                "content": "should-not-write"
            }),
            &workspace_root,
        );

        assert_eq!(payload["status"], "error");
        assert_eq!(payload["code"], "denied_path");
    }

    #[test]
    fn fs_write_rejects_malformed_patch_requests() {
        let dir = tempdir().expect("temp directory should create");
        let workspace_root = dir
            .path()
            .canonicalize()
            .expect("workspace root should resolve");
        std::fs::write(dir.path().join("plan.md"), "alpha").expect("fixture file should write");

        let payload = fs_write_tool_response_with_root(
            &json!({
                "path": "plan.md",
                "mode": "patch",
                "patch": {
                    "find": "",
                    "replace": "beta"
                }
            }),
            &workspace_root,
        );

        assert_eq!(payload["status"], "error");
        assert_eq!(payload["code"], "malformed_edit");
    }

    #[test]
    fn execute_bash_policy_denies_blocked_patterns_without_override() {
        let request = test_execute_bash_request("rm -rf .");
        let err =
            evaluate_execute_bash_policy(&request).expect_err("dangerous pattern should fail");
        assert_eq!(err.code, "denied_command");
    }

    #[test]
    fn execute_bash_policy_allows_dangerous_override_when_approved() {
        let mut request = test_execute_bash_request("rm -rf ./tmp");
        request.allow_dangerous = true;
        request.approved = true;

        let decision =
            evaluate_execute_bash_policy(&request).expect("approved override should pass");
        assert!(!decision.read_only_auto_allow);
    }

    #[test]
    fn execute_bash_policy_auto_allows_read_only_commands() {
        let request = test_execute_bash_request("git status");
        let decision = evaluate_execute_bash_policy(&request).expect("read-only should pass");
        assert!(decision.read_only_auto_allow);
    }

    #[tokio::test]
    async fn execute_bash_retries_failed_commands_when_configured() {
        let payload = execute_bash_tool_response(&json!({
            "command": "false",
            "approved": true,
            "retry_attempts": 2,
            "retry_delay_ms": 0
        }))
        .await;

        assert_eq!(payload["status"], "error");
        assert_eq!(payload["code"], "command_failed");
        assert_eq!(payload["attempts"], 2);
    }

    #[test]
    fn github_ops_issue_create_runs_expected_mocked_command() {
        let calls = std::cell::RefCell::new(Vec::<Vec<String>>::new());
        let payload = github_ops_tool_response_with_runner(
            &json!({
                "action": "issue_create",
                "repo": "zavora-ai/zavora-cli",
                "title": "Test issue",
                "body": "Issue body",
                "labels": ["bug", "sprint:8"]
            }),
            true,
            |args| {
                calls.borrow_mut().push(args.to_vec());
                Ok(GitHubCliOutput {
                    success: true,
                    exit_code: 0,
                    stdout: "https://github.com/zavora-ai/zavora-cli/issues/999".to_string(),
                    stderr: String::new(),
                })
            },
        );

        assert_eq!(payload["status"], "ok");
        assert_eq!(payload["action"], "issue_create");
        let calls = calls.borrow();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0][0], "issue");
        assert_eq!(calls[0][1], "create");
    }

    #[test]
    fn github_ops_preflight_requires_auth_without_token() {
        let payload = github_ops_tool_response_with_runner(
            &json!({
                "action": "issue_create",
                "repo": "zavora-ai/zavora-cli",
                "title": "Needs auth",
                "body": "body"
            }),
            false,
            |_args| {
                Ok(GitHubCliOutput {
                    success: false,
                    exit_code: 1,
                    stdout: String::new(),
                    stderr: "not logged in".to_string(),
                })
            },
        );

        assert_eq!(payload["status"], "error");
        assert_eq!(payload["code"], "auth_required");
    }

    #[test]
    fn github_ops_project_item_update_runs_expected_mocked_command() {
        let calls = std::cell::RefCell::new(Vec::<Vec<String>>::new());
        let payload = github_ops_tool_response_with_runner(
            &json!({
                "action": "project_item_update",
                "project_id": "PVT_kwDOBVKgdc4BPPxU",
                "item_id": "PVTI_lADOBVKgdc4BPPxUzglepjM",
                "field_id": "PVTSSF_lADOBVKgdc4BPPxUzg9te4w",
                "status_option_id": "98236657"
            }),
            true,
            |args| {
                calls.borrow_mut().push(args.to_vec());
                Ok(GitHubCliOutput {
                    success: true,
                    exit_code: 0,
                    stdout: String::new(),
                    stderr: String::new(),
                })
            },
        );

        assert_eq!(payload["status"], "ok");
        assert_eq!(payload["action"], "project_item_update");
        let calls = calls.borrow();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0][0], "project");
        assert_eq!(calls[0][1], "item-edit");
    }

    #[test]
    fn error_taxonomy_distinguishes_provider_session_and_tooling() {
        let provider_err = anyhow::anyhow!("OPENAI_API_KEY is required for OpenAI provider");
        let session_err = anyhow::anyhow!("failed to load session 'abc'");
        let tooling_err = anyhow::anyhow!("tool invocation failed: timeout");

        assert_eq!(categorize_error(&provider_err), ErrorCategory::Provider);
        assert_eq!(categorize_error(&session_err), ErrorCategory::Session);
        assert_eq!(categorize_error(&tooling_err), ErrorCategory::Tooling);
    }

    #[test]
    fn runtime_config_uses_selected_profile_defaults() {
        let dir = tempdir().expect("temp directory should create");
        let path = dir.path().join("config.toml");
        std::fs::write(
            &path,
            r#"
[profiles.dev]
provider = "openai"
model = "gpt-4o-mini"
session_backend = "sqlite"
session_db_url = "sqlite://.zavora/dev.db"
app_name = "zavora-dev"
user_id = "dev-user"
session_id = "dev-session"
retrieval_backend = "local"
retrieval_doc_path = "docs/knowledge.md"
retrieval_max_chunks = 5
retrieval_max_chars = 2048
retrieval_min_score = 2
"#,
        )
        .expect("config should write");

        let cli = test_cli(path.to_string_lossy().as_ref(), "dev");
        let profiles = load_profiles(&cli.config_path).expect("profiles should load");
        let cfg = resolve_runtime_config(&cli, &profiles).expect("runtime config should resolve");

        assert_eq!(cfg.profile, "dev");
        assert_eq!(cfg.provider, Provider::Openai);
        assert_eq!(cfg.model.as_deref(), Some("gpt-4o-mini"));
        assert_eq!(cfg.session_backend, SessionBackend::Sqlite);
        assert!(!cfg.show_sensitive_config);
        assert_eq!(cfg.app_name, "zavora-dev");
        assert_eq!(cfg.user_id, "dev-user");
        assert_eq!(cfg.session_id, "dev-session");
        assert_eq!(cfg.retrieval_backend, RetrievalBackend::Local);
        assert_eq!(cfg.retrieval_doc_path.as_deref(), Some("docs/knowledge.md"));
        assert_eq!(cfg.retrieval_max_chunks, 5);
        assert_eq!(cfg.retrieval_max_chars, 2048);
        assert_eq!(cfg.retrieval_min_score, 2);
        assert_eq!(cfg.tool_confirmation_mode, ToolConfirmationMode::McpOnly);
        assert!(cfg.require_confirm_tool.is_empty());
        assert!(cfg.approve_tool.is_empty());
        assert_eq!(cfg.tool_timeout_secs, 45);
        assert_eq!(cfg.tool_retry_attempts, 2);
        assert_eq!(cfg.tool_retry_delay_ms, 500);
        assert!(cfg.telemetry_enabled);
        assert_eq!(cfg.telemetry_path, ".zavora/telemetry/events.jsonl");
        assert_eq!(cfg.guardrail_input_mode, GuardrailMode::Disabled);
        assert_eq!(cfg.guardrail_output_mode, GuardrailMode::Disabled);
        assert!(
            cfg.guardrail_terms.iter().any(|term| term == "password"),
            "default guardrail terms should include baseline sensitive markers"
        );
        assert_eq!(cfg.guardrail_redact_replacement, "[REDACTED]");
    }

    #[test]
    fn agent_catalog_local_overrides_global_with_deterministic_precedence() {
        let dir = tempdir().expect("temp directory should create");
        let global = dir.path().join("global-agents.toml");
        let local = dir.path().join("local-agents.toml");
        std::fs::write(
            &global,
            r#"
[agents.default]
instruction = "global-default"

[agents.coder]
model = "gpt-4o-mini"
"#,
        )
        .expect("global agent catalog should write");
        std::fs::write(
            &local,
            r#"
[agents.default]
instruction = "local-default"

[agents.reviewer]
model = "gpt-4.1"
"#,
        )
        .expect("local agent catalog should write");

        let paths = AgentPaths {
            local_catalog: local,
            global_catalog: Some(global),
            selection_file: dir.path().join("selection.toml"),
        };
        let resolved = load_resolved_agents(&paths).expect("agents should load");

        assert_eq!(
            resolved
                .get("default")
                .and_then(|agent| agent.config.instruction.as_deref()),
            Some("local-default")
        );
        assert_eq!(
            resolved.get("default").map(|agent| agent.source),
            Some(AgentSource::Local)
        );
        assert_eq!(
            resolved
                .get("coder")
                .and_then(|agent| agent.config.model.as_deref()),
            Some("gpt-4o-mini")
        );
        assert_eq!(
            resolved.get("coder").map(|agent| agent.source),
            Some(AgentSource::Global)
        );
        assert_eq!(
            resolved
                .get("reviewer")
                .and_then(|agent| agent.config.model.as_deref()),
            Some("gpt-4.1")
        );
    }

    #[test]
    fn runtime_config_applies_agent_overrides_for_model_prompt_and_tools() {
        let cli = test_cli(".zavora/config.toml", "default");
        let profiles = ProfilesFile::default();
        let mut agents = implicit_agent_map();
        agents.insert(
            "coder".to_string(),
            ResolvedAgent {
                name: "coder".to_string(),
                source: AgentSource::Local,
                config: AgentFileConfig {
                    description: Some("Coding optimized agent".to_string()),
                    instruction: Some("Always propose minimal diffs.".to_string()),
                    provider: Some(Provider::Openai),
                    model: Some("gpt-4.1".to_string()),
                    tool_confirmation_mode: Some(ToolConfirmationMode::Always),
                    resource_paths: vec!["docs/CONTRIBUTING.md".to_string()],
                    allow_tools: vec!["fs_read".to_string(), "fs_write".to_string()],
                    deny_tools: vec!["execute_bash".to_string()],
                },
            },
        );

        let cfg = resolve_runtime_config_with_agents(&cli, &profiles, &agents, Some("coder"))
            .expect("runtime config should resolve");
        assert_eq!(cfg.agent_name, "coder");
        assert_eq!(cfg.agent_source, AgentSource::Local);
        assert_eq!(cfg.provider, Provider::Openai);
        assert_eq!(cfg.model.as_deref(), Some("gpt-4.1"));
        assert_eq!(cfg.tool_confirmation_mode, ToolConfirmationMode::Always);
        assert_eq!(
            cfg.agent_instruction.as_deref(),
            Some("Always propose minimal diffs.")
        );
        assert_eq!(cfg.agent_resource_paths, vec!["docs/CONTRIBUTING.md"]);
        assert_eq!(cfg.agent_allow_tools, vec!["fs_read", "fs_write"]);
        assert_eq!(cfg.agent_deny_tools, vec!["execute_bash"]);
    }

    #[test]
    fn resolve_active_agent_falls_back_to_default_when_selection_missing() {
        let cli = test_cli(".zavora/config.toml", "default");
        let agents = implicit_agent_map();
        let selected = resolve_active_agent_name(&cli, &agents, Some("missing-agent"))
            .expect("missing persisted selection should fall back");
        assert_eq!(selected, "default");
    }

    #[test]
    fn resolve_active_agent_reports_missing_explicit_agent() {
        let mut cli = test_cli(".zavora/config.toml", "default");
        cli.agent = Some("missing-agent".to_string());
        let agents = implicit_agent_map();
        let err = resolve_active_agent_name(&cli, &agents, None)
            .expect_err("explicit missing agent should fail");
        assert!(err.to_string().contains("agent 'missing-agent' not found"));
    }

    #[test]
    fn runtime_config_parses_profile_mcp_servers() {
        let dir = tempdir().expect("temp directory should create");
        let path = dir.path().join("config.toml");
        std::fs::write(
            &path,
            r#"
[profiles.dev]
provider = "openai"
model = "gpt-4o-mini"
tool_confirmation_mode = "always"
require_confirm_tool = ["release_template"]
approve_tool = ["release_template"]
tool_timeout_secs = 90
tool_retry_attempts = 4
tool_retry_delay_ms = 750
guardrail_input_mode = "observe"
guardrail_output_mode = "redact"
guardrail_terms = ["internal-only", "private data"]
guardrail_redact_replacement = "***"

[[profiles.dev.mcp_servers]]
name = "atlas"
endpoint = "https://atlas.example.com/mcp"
enabled = true
timeout_secs = 20
auth_bearer_env = "ATLAS_MCP_TOKEN"
tool_allowlist = ["search", "lookup"]

[[profiles.dev.mcp_servers]]
name = "disabled-tooling"
endpoint = "https://disabled.example.com/mcp"
enabled = false
"#,
        )
        .expect("config should write");

        let cli = test_cli(path.to_string_lossy().as_ref(), "dev");
        let profiles = load_profiles(&cli.config_path).expect("profiles should load");
        let cfg = resolve_runtime_config(&cli, &profiles).expect("runtime config should resolve");

        assert_eq!(cfg.mcp_servers.len(), 2);
        assert_eq!(cfg.mcp_servers[0].name, "atlas");
        assert_eq!(cfg.mcp_servers[0].endpoint, "https://atlas.example.com/mcp");
        assert_eq!(cfg.mcp_servers[0].enabled, Some(true));
        assert_eq!(cfg.mcp_servers[0].timeout_secs, Some(20));
        assert_eq!(
            cfg.mcp_servers[0].auth_bearer_env.as_deref(),
            Some("ATLAS_MCP_TOKEN")
        );
        assert_eq!(cfg.mcp_servers[0].tool_allowlist, vec!["search", "lookup"]);
        assert_eq!(cfg.tool_confirmation_mode, ToolConfirmationMode::Always);
        assert_eq!(cfg.require_confirm_tool, vec!["release_template"]);
        assert_eq!(cfg.approve_tool, vec!["release_template"]);
        assert_eq!(cfg.tool_timeout_secs, 90);
        assert_eq!(cfg.tool_retry_attempts, 4);
        assert_eq!(cfg.tool_retry_delay_ms, 750);
        assert!(cfg.telemetry_enabled);
        assert_eq!(cfg.telemetry_path, ".zavora/telemetry/events.jsonl");
        assert_eq!(cfg.guardrail_input_mode, GuardrailMode::Observe);
        assert_eq!(cfg.guardrail_output_mode, GuardrailMode::Redact);
        assert_eq!(cfg.guardrail_terms, vec!["internal-only", "private data"]);
        assert_eq!(cfg.guardrail_redact_replacement, "***");
        assert_eq!(cfg.mcp_servers[1].name, "disabled-tooling");
        assert_eq!(cfg.mcp_servers[1].enabled, Some(false));
    }

    #[test]
    fn runtime_config_telemetry_cli_overrides_profile_values() {
        let dir = tempdir().expect("temp directory should create");
        let path = dir.path().join("config.toml");
        std::fs::write(
            &path,
            r#"
[profiles.dev]
telemetry_enabled = true
telemetry_path = ".zavora/telemetry/dev.jsonl"
"#,
        )
        .expect("config should write");

        let mut cli = test_cli(path.to_string_lossy().as_ref(), "dev");
        cli.telemetry_enabled = Some(false);
        cli.telemetry_path = Some(".zavora/telemetry/override.jsonl".to_string());

        let profiles = load_profiles(&cli.config_path).expect("profiles should load");
        let cfg = resolve_runtime_config(&cli, &profiles).expect("runtime config should resolve");

        assert!(!cfg.telemetry_enabled);
        assert_eq!(cfg.telemetry_path, ".zavora/telemetry/override.jsonl");
    }

    #[test]
    fn runtime_config_guardrail_cli_overrides_profile_values() {
        let dir = tempdir().expect("temp directory should create");
        let path = dir.path().join("config.toml");
        std::fs::write(
            &path,
            r#"
[profiles.dev]
guardrail_input_mode = "observe"
guardrail_output_mode = "block"
guardrail_terms = ["secret"]
guardrail_redact_replacement = "***"
"#,
        )
        .expect("config should write");

        let mut cli = test_cli(path.to_string_lossy().as_ref(), "dev");
        cli.guardrail_input_mode = Some(GuardrailMode::Block);
        cli.guardrail_output_mode = Some(GuardrailMode::Redact);
        cli.guardrail_term = vec!["token".to_string(), "password".to_string()];
        cli.guardrail_redact_replacement = Some("[MASKED]".to_string());

        let profiles = load_profiles(&cli.config_path).expect("profiles should load");
        let cfg = resolve_runtime_config(&cli, &profiles).expect("runtime config should resolve");

        assert_eq!(cfg.guardrail_input_mode, GuardrailMode::Block);
        assert_eq!(cfg.guardrail_output_mode, GuardrailMode::Redact);
        assert_eq!(cfg.guardrail_terms, vec!["secret", "token", "password"]);
        assert_eq!(cfg.guardrail_redact_replacement, "[MASKED]");
    }

    #[test]
    fn runtime_config_honors_show_sensitive_config_flag() {
        let mut cli = test_cli(".zavora/config.toml", "default");
        cli.show_sensitive_config = true;
        let profiles = ProfilesFile::default();

        let cfg = resolve_runtime_config(&cli, &profiles).expect("runtime config should resolve");
        assert!(cfg.show_sensitive_config);
    }

    #[test]
    fn telemetry_summary_counts_command_and_tool_events() {
        let lines = vec![
            json!({
                "ts_unix_ms": 1000,
                "event": "command.started",
                "run_id": "run-a",
                "command": "ask"
            })
            .to_string(),
            json!({
                "ts_unix_ms": 1100,
                "event": "tool.requested",
                "run_id": "run-a",
                "command": "ask",
                "tool": "release_template"
            })
            .to_string(),
            json!({
                "ts_unix_ms": 1200,
                "event": "tool.succeeded",
                "run_id": "run-a",
                "command": "ask",
                "tool": "release_template"
            })
            .to_string(),
            json!({
                "ts_unix_ms": 1300,
                "event": "command.completed",
                "run_id": "run-a",
                "command": "ask"
            })
            .to_string(),
            json!({
                "ts_unix_ms": 1400,
                "event": "command.failed",
                "run_id": "run-b",
                "command": "workflow.parallel"
            })
            .to_string(),
            "invalid-json-line".to_string(),
        ];

        let summary = summarize_telemetry_lines(lines, 100);
        assert_eq!(summary.total_lines, 6);
        assert_eq!(summary.parsed_events, 5);
        assert_eq!(summary.parse_errors, 1);
        assert_eq!(summary.unique_runs.len(), 2);
        assert_eq!(summary.command_completed, 1);
        assert_eq!(summary.command_failed, 1);
        assert_eq!(summary.tool_requested, 1);
        assert_eq!(summary.tool_succeeded, 1);
        assert_eq!(summary.tool_failed, 0);
        assert_eq!(summary.command_counts.get("ask"), Some(&4));
        assert_eq!(summary.command_counts.get("workflow.parallel"), Some(&1));
        assert_eq!(summary.last_event_ts_unix_ms, Some(1400));
    }

    #[test]
    fn guardrail_redact_mode_masks_detected_terms() {
        let mut cfg = base_cfg();
        cfg.guardrail_terms = vec!["api key".to_string()];
        cfg.guardrail_redact_replacement = "[MASKED]".to_string();
        let telemetry = test_telemetry(&cfg);

        let out = apply_guardrail(
            &cfg,
            &telemetry,
            "output",
            GuardrailMode::Redact,
            "Share the API KEY only with admins.",
        )
        .expect("redact mode should return transformed text");

        assert_eq!(out, "Share the [MASKED] only with admins.");
    }

    #[test]
    fn guardrail_block_mode_rejects_matching_content() {
        let mut cfg = base_cfg();
        cfg.guardrail_terms = vec!["secret".to_string()];
        let telemetry = test_telemetry(&cfg);

        let err = apply_guardrail(
            &cfg,
            &telemetry,
            "input",
            GuardrailMode::Block,
            "This contains a secret token.",
        )
        .expect_err("block mode should fail on term match");
        assert!(err.to_string().contains("guardrail blocked input content"));
    }

    #[test]
    fn guardrail_observe_mode_logs_but_does_not_modify_text() {
        let mut cfg = base_cfg();
        cfg.guardrail_terms = vec!["password".to_string()];
        let telemetry = test_telemetry(&cfg);

        let text = "password rotation should happen every 90 days";
        let out = apply_guardrail(&cfg, &telemetry, "output", GuardrailMode::Observe, text)
            .expect("observe mode should not fail");
        assert_eq!(out, text);
    }

    #[test]
    fn a2a_ping_process_returns_ack_envelope() {
        let req = A2aPingRequest {
            from_agent: "sales".to_string(),
            to_agent: "procurement".to_string(),
            message_id: "msg-1".to_string(),
            correlation_id: Some("corr-1".to_string()),
            payload: json!({"intent": "supply-check"}),
        };

        let response = process_a2a_ping(req.clone()).expect("a2a processing should succeed");
        assert_eq!(response.to_agent, "sales");
        assert_eq!(response.from_agent, "procurement");
        assert_eq!(response.acknowledged_message_id, "msg-1");
        assert_eq!(response.correlation_id, "corr-1");
        assert_eq!(response.status, "acknowledged");
        assert!(response.message_id.starts_with("ack-"));
    }

    #[test]
    fn a2a_ping_process_rejects_invalid_request() {
        let req = A2aPingRequest {
            from_agent: "".to_string(),
            to_agent: "procurement".to_string(),
            message_id: "msg-1".to_string(),
            correlation_id: None,
            payload: json!({}),
        };

        let err = process_a2a_ping(req).expect_err("missing from_agent should fail");
        assert!(err.to_string().contains("from_agent is required"));
    }

    #[test]
    fn a2a_smoke_command_passes_with_default_fixture() {
        let cfg = base_cfg();
        let telemetry = test_telemetry(&cfg);
        run_a2a_smoke(&telemetry).expect("a2a smoke should pass");
    }

    fn eval_dataset_fixture() -> EvalDataset {
        EvalDataset {
            name: "retrieval-baseline".to_string(),
            version: "1".to_string(),
            description: "fixture".to_string(),
            cases: vec![
                EvalCase {
                    id: "release".to_string(),
                    query: "release rollback mitigation".to_string(),
                    chunks: vec![
                        "release plan includes rollback and mitigation steps".to_string(),
                        "unrelated content".to_string(),
                    ],
                    required_terms: vec!["rollback".to_string(), "mitigation".to_string()],
                    max_chunks: 2,
                    min_term_matches: Some(2),
                },
                EvalCase {
                    id: "architecture".to_string(),
                    query: "architecture components".to_string(),
                    chunks: vec![
                        "component diagram and architecture decisions".to_string(),
                        "random note".to_string(),
                    ],
                    required_terms: vec!["architecture".to_string(), "component".to_string()],
                    max_chunks: 2,
                    min_term_matches: Some(1),
                },
            ],
        }
    }

    #[test]
    fn eval_harness_produces_metrics_and_threshold_result() {
        let dataset = eval_dataset_fixture();
        let report = run_eval_harness(&dataset, 10, 0.8).expect("eval harness should run");

        assert_eq!(report.total_cases, 2);
        assert_eq!(report.passed_cases, 2);
        assert_eq!(report.failed_cases, 0);
        assert_eq!(report.pass_rate, 1.0);
        assert!(report.passed_threshold);
        assert_eq!(report.benchmark_iterations, 10);
        assert!(report.avg_latency_ms >= 0.0);
        assert!(report.p95_latency_ms >= 0.0);
        assert!(report.throughput_qps >= 0.0);
    }

    #[test]
    fn eval_harness_fails_threshold_when_case_quality_is_low() {
        let mut dataset = eval_dataset_fixture();
        dataset.cases[0].required_terms = vec!["missing-term".to_string()];
        dataset.cases[0].min_term_matches = Some(1);

        let report = run_eval_harness(&dataset, 5, 0.75).expect("eval harness should run");
        assert_eq!(report.total_cases, 2);
        assert_eq!(report.passed_cases, 1);
        assert_eq!(report.failed_cases, 1);
        assert_eq!(report.pass_rate, 0.5);
        assert!(!report.passed_threshold);
    }

    #[test]
    fn load_eval_dataset_reports_empty_case_set() {
        let dir = tempdir().expect("temp directory should create");
        let path = dir.path().join("eval.json");
        std::fs::write(
            &path,
            r#"{"name":"empty","version":"1","description":"none","cases":[]}"#,
        )
        .expect("dataset should write");

        let err =
            load_eval_dataset(path.to_string_lossy().as_ref()).expect_err("empty dataset fails");
        assert!(err.to_string().contains("has no cases"));
    }

    #[test]
    fn tool_confirmation_defaults_deny_unapproved_mcp_tools() {
        let cfg = base_cfg();
        let runtime_tools = make_runtime_tools(
            &["current_unix_time", "search_incidents"],
            &["search_incidents"],
        );

        let settings = resolve_tool_confirmation_settings(&cfg, &runtime_tools);
        assert!(settings.policy.requires_confirmation("search_incidents"));
        assert!(!settings.policy.requires_confirmation("current_unix_time"));
        assert_eq!(
            settings
                .run_config
                .tool_confirmation_decisions
                .get("search_incidents"),
            Some(&ToolConfirmationDecision::Deny)
        );
    }

    #[test]
    fn tool_confirmation_requires_fs_write_by_default() {
        let cfg = base_cfg();
        let runtime_tools = make_runtime_tools(&["current_unix_time", "fs_write"], &[]);

        let settings = resolve_tool_confirmation_settings(&cfg, &runtime_tools);
        assert!(settings.policy.requires_confirmation("fs_write"));
        assert_eq!(
            settings
                .run_config
                .tool_confirmation_decisions
                .get("fs_write"),
            Some(&ToolConfirmationDecision::Deny)
        );
    }

    #[test]
    fn tool_confirmation_requires_execute_bash_by_default() {
        let cfg = base_cfg();
        let runtime_tools = make_runtime_tools(&["current_unix_time", "execute_bash"], &[]);

        let settings = resolve_tool_confirmation_settings(&cfg, &runtime_tools);
        assert!(settings.policy.requires_confirmation("execute_bash"));
        assert_eq!(
            settings
                .run_config
                .tool_confirmation_decisions
                .get("execute_bash"),
            Some(&ToolConfirmationDecision::Deny)
        );
    }

    #[test]
    fn tool_confirmation_requires_github_ops_by_default() {
        let cfg = base_cfg();
        let runtime_tools = make_runtime_tools(&["current_unix_time", "github_ops"], &[]);

        let settings = resolve_tool_confirmation_settings(&cfg, &runtime_tools);
        assert!(settings.policy.requires_confirmation("github_ops"));
        assert_eq!(
            settings
                .run_config
                .tool_confirmation_decisions
                .get("github_ops"),
            Some(&ToolConfirmationDecision::Deny)
        );
    }

    #[test]
    fn tool_confirmation_approve_list_overrides_default_deny() {
        let mut cfg = base_cfg();
        cfg.approve_tool = vec!["search_incidents".to_string()];
        let runtime_tools = make_runtime_tools(
            &["current_unix_time", "search_incidents"],
            &["search_incidents"],
        );

        let settings = resolve_tool_confirmation_settings(&cfg, &runtime_tools);
        assert_eq!(
            settings
                .run_config
                .tool_confirmation_decisions
                .get("search_incidents"),
            Some(&ToolConfirmationDecision::Approve)
        );
    }

    #[test]
    fn tool_confirmation_custom_required_tools_enforced() {
        let mut cfg = base_cfg();
        cfg.tool_confirmation_mode = ToolConfirmationMode::Never;
        cfg.require_confirm_tool = vec!["release_template".to_string()];
        let runtime_tools = make_runtime_tools(&["release_template", "current_unix_time"], &[]);

        let settings = resolve_tool_confirmation_settings(&cfg, &runtime_tools);
        assert!(settings.policy.requires_confirmation("release_template"));
        assert!(!settings.policy.requires_confirmation("current_unix_time"));
        assert_eq!(
            settings
                .run_config
                .tool_confirmation_decisions
                .get("release_template"),
            Some(&ToolConfirmationDecision::Deny)
        );
    }

    #[test]
    fn tool_confirmation_can_require_fs_read() {
        let mut cfg = base_cfg();
        cfg.tool_confirmation_mode = ToolConfirmationMode::Never;
        cfg.require_confirm_tool = vec!["fs_read".to_string()];
        let runtime_tools = make_runtime_tools(&["fs_read", "current_unix_time"], &[]);

        let settings = resolve_tool_confirmation_settings(&cfg, &runtime_tools);
        assert!(settings.policy.requires_confirmation("fs_read"));
        assert_eq!(
            settings
                .run_config
                .tool_confirmation_decisions
                .get("fs_read"),
            Some(&ToolConfirmationDecision::Deny)
        );
    }

    #[test]
    fn select_mcp_servers_filters_enabled_and_selects_by_name() {
        let mut cfg = base_cfg();
        cfg.mcp_servers = vec![
            McpServerConfig {
                name: "atlas".to_string(),
                endpoint: "https://atlas.example.com/mcp".to_string(),
                enabled: Some(true),
                timeout_secs: Some(10),
                auth_bearer_env: None,
                tool_allowlist: Vec::new(),
            },
            McpServerConfig {
                name: "ops".to_string(),
                endpoint: "https://ops.example.com/mcp".to_string(),
                enabled: Some(false),
                timeout_secs: Some(10),
                auth_bearer_env: None,
                tool_allowlist: Vec::new(),
            },
            McpServerConfig {
                name: "analytics".to_string(),
                endpoint: "https://analytics.example.com/mcp".to_string(),
                enabled: None,
                timeout_secs: None,
                auth_bearer_env: None,
                tool_allowlist: Vec::new(),
            },
        ];

        let active = select_mcp_servers(&cfg, None).expect("active servers should resolve");
        assert_eq!(active.len(), 2);
        assert_eq!(active[0].name, "atlas");
        assert_eq!(active[1].name, "analytics");

        let single = select_mcp_servers(&cfg, Some("analytics"))
            .expect("named enabled server should resolve");
        assert_eq!(single.len(), 1);
        assert_eq!(single[0].name, "analytics");

        let err = select_mcp_servers(&cfg, Some("ops"))
            .expect_err("disabled server should not be selectable");
        assert!(err.to_string().contains("not found or not enabled"));
    }

    #[test]
    fn resolve_mcp_auth_reports_missing_bearer_token_env() {
        let server = McpServerConfig {
            name: "secure".to_string(),
            endpoint: "https://secure.example.com/mcp".to_string(),
            enabled: Some(true),
            timeout_secs: Some(15),
            auth_bearer_env: Some("__ZAVORA_TEST_MCP_TOKEN_MISSING__".to_string()),
            tool_allowlist: Vec::new(),
        };

        let err = resolve_mcp_auth(&server).expect_err("missing env should fail");
        let msg = err.to_string();
        assert!(msg.contains("requires bearer token env"));
        assert!(msg.contains("__ZAVORA_TEST_MCP_TOKEN_MISSING__"));
    }

    #[test]
    fn runtime_config_reports_missing_profile() {
        let cli = test_cli(".zavora/does-not-exist.toml", "ops");
        let profiles = load_profiles(&cli.config_path).expect("missing config should default");
        let err = resolve_runtime_config(&cli, &profiles).expect_err("missing profile should fail");
        assert!(
            err.to_string().contains("profile 'ops' not found"),
            "expected actionable missing profile message"
        );
    }

    #[test]
    fn invalid_profile_config_is_actionable() {
        let dir = tempdir().expect("temp directory should create");
        let path = dir.path().join("config.toml");
        std::fs::write(
            &path,
            r#"
[profiles.default]
provider = "not-a-provider"
"#,
        )
        .expect("config should write");

        let err = load_profiles(path.to_string_lossy().as_ref())
            .expect_err("invalid provider should fail parsing");
        let msg = format!("{err:#}");
        assert!(msg.contains("invalid profile configuration"));
    }

    #[test]
    fn provider_name_parser_accepts_known_values_and_rejects_unknown() {
        assert_eq!(
            parse_provider_name("openai").expect("openai should parse"),
            Provider::Openai
        );
        let err = parse_provider_name("unknown-provider").expect_err("invalid provider must fail");
        assert!(
            err.to_string().contains(
                "Supported values: auto, gemini, openai, anthropic, deepseek, groq, ollama"
            )
        );
    }

    #[test]
    fn chat_command_parser_recognizes_built_in_commands() {
        assert_eq!(
            parse_chat_command("/help"),
            ParsedChatCommand::Command(ChatCommand::Help)
        );
        assert_eq!(
            parse_chat_command("/TOOLS"),
            ParsedChatCommand::Command(ChatCommand::Tools)
        );
        assert_eq!(
            parse_chat_command("exit"),
            ParsedChatCommand::Command(ChatCommand::Exit)
        );
        assert_eq!(
            parse_chat_command("/provider openai"),
            ParsedChatCommand::Command(ChatCommand::Provider("openai".to_string()))
        );
        assert_eq!(
            parse_chat_command("/model gpt-4o-mini"),
            ParsedChatCommand::Command(ChatCommand::Model(Some("gpt-4o-mini".to_string())))
        );
        assert_eq!(
            parse_chat_command("/model"),
            ParsedChatCommand::Command(ChatCommand::Model(None))
        );
    }

    #[test]
    fn chat_command_parser_reports_missing_arguments() {
        assert_eq!(
            parse_chat_command("/provider"),
            ParsedChatCommand::MissingArgument {
                usage: "/provider <auto|gemini|openai|anthropic|deepseek|groq|ollama>"
            }
        );
    }

    #[test]
    fn model_picker_selection_falls_back_when_catalog_unavailable() {
        let options = model_picker_options(Provider::Auto);
        assert!(options.is_empty());
        assert_eq!(
            resolve_model_picker_selection(&options, "1").expect("fallback should not fail"),
            None
        );
    }

    #[test]
    fn model_picker_selection_accepts_numeric_index() {
        let options = model_picker_options(Provider::Openai);
        let picked = resolve_model_picker_selection(&options, "2")
            .expect("selection should parse")
            .expect("selection should choose a model");
        assert_eq!(picked, "gpt-4.1");
    }

    #[test]
    fn chat_command_parser_handles_unknown_and_non_command_inputs() {
        assert_eq!(
            parse_chat_command("/does-not-exist"),
            ParsedChatCommand::UnknownCommand("/does-not-exist".to_string())
        );
        assert_eq!(
            parse_chat_command("write a short story"),
            ParsedChatCommand::NotACommand
        );
    }

    #[test]
    fn server_runner_cache_key_uses_user_and_session() {
        let mut cfg = base_cfg();
        cfg.user_id = "perf-user".to_string();
        cfg.session_id = "perf-session".to_string();

        assert_eq!(
            server_runner_cache_key(&cfg),
            "perf-user::perf-session".to_string()
        );
    }

    #[test]
    fn model_compatibility_validation_rejects_cross_provider_model_ids() {
        assert!(validate_model_for_provider(Provider::Openai, "gpt-4o-mini").is_ok());
        assert!(
            validate_model_for_provider(Provider::Anthropic, "claude-sonnet-4-20250514").is_ok()
        );
        assert!(validate_model_for_provider(Provider::Openai, "claude-sonnet-4-20250514").is_err());
    }

    #[test]
    fn augment_prompt_with_retrieval_leaves_prompt_unchanged_when_disabled() {
        let retrieval = DisabledRetrievalService;
        let prompt = "Plan release milestones";
        let out = augment_prompt_with_retrieval(
            &retrieval,
            prompt,
            RetrievalPolicy {
                max_chunks: 3,
                max_chars: 4000,
                min_score: 1,
            },
        )
        .expect("prompt augmentation should pass");
        assert_eq!(out, prompt);
    }

    #[test]
    fn local_file_retrieval_returns_relevant_chunks() {
        let dir = tempdir().expect("temp directory should create");
        let path = dir.path().join("knowledge.txt");
        std::fs::write(
            &path,
            "Rust CLI release planning\n\nADK retrieval abstraction and context injection",
        )
        .expect("doc file should write");

        let retrieval = LocalFileRetrievalService::load(path.to_string_lossy().as_ref())
            .expect("local retrieval should load");
        let chunks = retrieval
            .retrieve("retrieval abstraction", 3)
            .expect("retrieval should run");
        assert!(!chunks.is_empty(), "expected at least one relevant chunk");
    }

    #[test]
    fn local_file_retrieval_ranks_chunks_deterministically_by_term_hits() {
        let retrieval = LocalFileRetrievalService {
            chunks: vec![
                RetrievedChunk {
                    source: "rank:1".to_string(),
                    text: "release quality gates".to_string(),
                    score: 0,
                },
                RetrievedChunk {
                    source: "rank:2".to_string(),
                    text: "release quality gates release quality".to_string(),
                    score: 0,
                },
            ],
        };

        let chunks = retrieval
            .retrieve("release quality", 2)
            .expect("retrieval should run");
        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0].source, "rank:2");
        assert_eq!(chunks[1].source, "rank:1");
    }

    #[test]
    fn local_retrieval_backend_requires_doc_path() {
        let mut cfg = base_cfg();
        cfg.retrieval_backend = RetrievalBackend::Local;
        cfg.retrieval_doc_path = None;

        let err = match build_retrieval_service(&cfg) {
            Ok(_) => panic!("missing doc path should fail"),
            Err(err) => err,
        };
        assert!(
            err.to_string()
                .contains("retrieval backend 'local' requires")
        );
    }

    #[test]
    fn local_retrieval_backend_missing_file_is_reported() {
        let mut cfg = base_cfg();
        cfg.retrieval_backend = RetrievalBackend::Local;
        cfg.retrieval_doc_path = Some("does-not-exist.md".to_string());

        let err = match build_retrieval_service(&cfg) {
            Ok(_) => panic!("missing retrieval file should fail"),
            Err(err) => err,
        };
        assert!(
            err.to_string().contains("failed to read retrieval doc"),
            "expected backend unavailability error path"
        );
    }

    #[test]
    fn retrieval_policy_enforces_context_budget_and_score_threshold() {
        let retrieval = LocalFileRetrievalService {
            chunks: vec![
                RetrievedChunk {
                    source: "test:1".to_string(),
                    text: "alpha beta gamma delta".to_string(),
                    score: 10,
                },
                RetrievedChunk {
                    source: "test:2".to_string(),
                    text: "small".to_string(),
                    score: 1,
                },
            ],
        };

        let out = augment_prompt_with_retrieval(
            &retrieval,
            "alpha",
            RetrievalPolicy {
                max_chunks: 3,
                max_chars: 5,
                min_score: 2,
            },
        )
        .expect("augmentation should pass");

        assert!(
            out.contains("alpha"),
            "expected retained high-score context"
        );
        assert!(
            !out.contains("small"),
            "expected low-score chunk to be filtered"
        );
    }

    #[test]
    fn retrieval_augmentation_falls_back_when_no_matches() {
        let retrieval = LocalFileRetrievalService {
            chunks: vec![RetrievedChunk {
                source: "fallback:1".to_string(),
                text: "unrelated content".to_string(),
                score: 0,
            }],
        };

        let prompt = "release rollout";
        let out = augment_prompt_with_retrieval(
            &retrieval,
            prompt,
            RetrievalPolicy {
                max_chunks: 3,
                max_chars: 4000,
                min_score: 1,
            },
        )
        .expect("augmentation should pass");
        assert_eq!(
            out, prompt,
            "no-result path should preserve original prompt"
        );
    }

    #[cfg(not(feature = "semantic-search"))]
    #[test]
    fn semantic_retrieval_backend_requires_feature_flag() {
        let mut cfg = base_cfg();
        cfg.retrieval_backend = RetrievalBackend::Semantic;
        cfg.retrieval_doc_path = Some("README.md".to_string());
        let err = match build_retrieval_service(&cfg) {
            Ok(_) => panic!("semantic retrieval should require feature flag"),
            Err(err) => err,
        };
        assert!(
            err.to_string()
                .contains("requires feature 'semantic-search'")
        );
    }

    #[cfg(feature = "semantic-search")]
    #[test]
    fn semantic_retrieval_backend_returns_ranked_chunks() {
        let dir = tempdir().expect("temp directory should create");
        let path = dir.path().join("knowledge.txt");
        std::fs::write(
            &path,
            "Agile release planning and rollout gates\n\nSemantic retrieval context ranking",
        )
        .expect("doc file should write");

        let retrieval = SemanticLocalRetrievalService::load(path.to_string_lossy().as_ref())
            .expect("semantic retrieval should load");
        let chunks = retrieval
            .retrieve("rollout gates", 2)
            .expect("semantic retrieval should run");
        assert!(!chunks.is_empty(), "expected semantic retrieval matches");
    }

    #[tokio::test]
    async fn sessions_show_missing_session_returns_session_category_error() {
        let cfg = base_cfg();
        let err = run_sessions_show(&cfg, Some("missing-session".to_string()), 10)
            .await
            .expect_err("missing session should error");

        assert_eq!(categorize_error(&err), ErrorCategory::Session);
        let rendered = format_cli_error(&err, cfg.show_sensitive_config);
        assert!(
            rendered.contains("[SESSION]"),
            "expected session category marker in error output"
        );
    }

    #[test]
    fn redact_sensitive_text_masks_sqlite_urls() {
        let raw = "open failed at sqlite://.zavora/sessions.db; retry sqlite://tmp/test.db";
        let rendered = redact_sensitive_text(raw);

        assert!(!rendered.contains(".zavora/sessions.db"));
        assert!(!rendered.contains("tmp/test.db"));
        assert_eq!(
            rendered,
            "open failed at sqlite://[REDACTED]; retry sqlite://[REDACTED]"
        );
    }

    #[test]
    fn format_cli_error_redacts_sqlite_urls_by_default() {
        let err = anyhow::anyhow!("failed to open sqlite://.zavora/sessions.db");
        let rendered = format_cli_error(&err, false);

        assert!(rendered.contains("sqlite://[REDACTED]"));
        assert!(!rendered.contains(".zavora/sessions.db"));
    }

    #[tokio::test]
    async fn sessions_delete_requires_force_flag() {
        let (_dir, cfg) = sqlite_cfg("default-session");
        create_session(&cfg, "delete-me").await;

        let err = run_sessions_delete(&cfg, Some("delete-me".to_string()), false)
            .await
            .expect_err("delete without --force should fail");
        assert_eq!(categorize_error(&err), ErrorCategory::Input);

        let sessions = list_session_ids(&cfg).await;
        assert!(sessions.contains(&"delete-me".to_string()));
    }

    #[tokio::test]
    async fn sessions_delete_force_removes_target_session() {
        let (_dir, cfg) = sqlite_cfg("default-session");
        create_session(&cfg, "delete-me").await;

        run_sessions_delete(&cfg, Some("delete-me".to_string()), true)
            .await
            .expect("forced delete should pass");

        let sessions = list_session_ids(&cfg).await;
        assert!(!sessions.contains(&"delete-me".to_string()));
    }

    #[tokio::test]
    async fn sessions_prune_enforces_safety_and_deletes_when_forced() {
        let (_dir, cfg) = sqlite_cfg("default-session");
        create_session(&cfg, "s1").await;
        create_session(&cfg, "s2").await;
        create_session(&cfg, "s3").await;

        let err = run_sessions_prune(&cfg, 1, false, false)
            .await
            .expect_err("prune without --force should fail");
        assert_eq!(categorize_error(&err), ErrorCategory::Input);

        run_sessions_prune(&cfg, 1, true, false)
            .await
            .expect("dry run should pass");
        let sessions_after_dry_run = list_session_ids(&cfg).await;
        assert_eq!(sessions_after_dry_run.len(), 3);

        run_sessions_prune(&cfg, 1, false, true)
            .await
            .expect("forced prune should pass");
        let sessions_after_force = list_session_ids(&cfg).await;
        assert_eq!(sessions_after_force.len(), 1);
    }

    #[tokio::test]
    async fn shared_memory_session_service_preserves_history_across_runner_rebuilds() {
        let cfg = base_cfg();
        let telemetry = test_telemetry(&cfg);
        let session_service: Arc<dyn SessionService> = Arc::new(InMemorySessionService::new());

        let runner_one = build_runner_with_session_service(
            build_single_agent(mock_model("first answer")).expect("agent should build"),
            &cfg,
            session_service.clone(),
            None,
        )
        .await
        .expect("runner should build");
        run_prompt(&runner_one, &cfg, "first prompt", &telemetry)
            .await
            .expect("first prompt should run");

        let runner_two = build_runner_with_session_service(
            build_single_agent(mock_model("second answer")).expect("agent should build"),
            &cfg,
            session_service.clone(),
            None,
        )
        .await
        .expect("second runner should build");
        run_prompt(&runner_two, &cfg, "second prompt", &telemetry)
            .await
            .expect("second prompt should run");

        let session = session_service
            .get(GetRequest {
                app_name: cfg.app_name.clone(),
                user_id: cfg.user_id.clone(),
                session_id: cfg.session_id.clone(),
                num_recent_events: None,
                after: None,
            })
            .await
            .expect("session should exist");

        assert!(
            session.events().len() >= 4,
            "expected in-memory session history to persist across runner rebuilds"
        );
    }

    #[tokio::test]
    async fn chat_switch_path_builds_runner_for_ollama_without_losing_session_service() {
        let mut cfg = base_cfg();
        cfg.provider = Provider::Ollama;
        cfg.model = Some("llama3.2".to_string());
        let session_service: Arc<dyn SessionService> = Arc::new(InMemorySessionService::new());
        let runtime_tools = ResolvedRuntimeTools {
            tools: build_builtin_tools(),
            mcp_tool_names: BTreeSet::new(),
        };
        let tool_confirmation = ToolConfirmationSettings::default();
        let telemetry = test_telemetry(&cfg);

        let (_runner, provider, model_name) = build_single_runner_for_chat(
            &cfg,
            session_service.clone(),
            &runtime_tools,
            &tool_confirmation,
            &telemetry,
        )
        .await
        .expect("chat runner should build for ollama");

        assert_eq!(provider, Provider::Ollama);
        assert_eq!(model_name, "llama3.2");
    }
}
