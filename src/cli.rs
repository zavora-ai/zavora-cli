use clap::{Parser, Subcommand, ValueEnum};
use serde::Deserialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Provider {
    Auto,
    Gemini,
    Openai,
    Anthropic,
    Deepseek,
    Groq,
    Ollama,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum WorkflowMode {
    Single,
    Sequential,
    Parallel,
    Loop,
    Graph,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SessionBackend {
    Memory,
    Sqlite,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RetrievalBackend {
    Disabled,
    Local,
    Semantic,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ToolConfirmationMode {
    Never,
    McpOnly,
    Always,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum GuardrailMode {
    Disabled,
    Observe,
    Block,
    Redact,
}

#[derive(Debug, Subcommand)]
pub enum ProfileCommands {
    #[command(about = "List configured profiles and highlight the active profile")]
    List,
    #[command(about = "Show the active profile's resolved runtime settings")]
    Show,
}

#[derive(Debug, Subcommand)]
pub enum AgentCommands {
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
pub enum McpCommands {
    #[command(about = "List MCP servers configured for the active profile")]
    List,
    #[command(about = "Discover MCP tools from configured servers (or a specific server)")]
    Discover {
        #[arg(long)]
        server: Option<String>,
    },
}

#[derive(Debug, Subcommand)]
pub enum SessionCommands {
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
pub enum TelemetryCommands {
    #[command(about = "Summarize telemetry events from a JSONL stream")]
    Report {
        #[arg(long)]
        path: Option<String>,
        #[arg(long, default_value_t = 5000)]
        limit: usize,
    },
}

#[derive(Debug, Subcommand)]
pub enum EvalCommands {
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
pub enum ServerCommands {
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
  zavora-cli --provider openai --model gpt-4.1 chat\n\
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
pub struct Cli {
    #[arg(long, env = "ZAVORA_PROVIDER", value_enum, default_value_t = Provider::Auto)]
    pub provider: Provider,

    #[arg(long, env = "ZAVORA_MODEL")]
    pub model: Option<String>,

    #[arg(long, env = "ZAVORA_AGENT")]
    pub agent: Option<String>,

    #[arg(long, env = "ZAVORA_PROFILE", default_value = "default")]
    pub profile: String,

    #[arg(long, env = "ZAVORA_CONFIG", default_value = ".zavora/config.toml")]
    pub config_path: String,

    #[arg(long, env = "ZAVORA_APP_NAME")]
    pub app_name: Option<String>,

    #[arg(long, env = "ZAVORA_USER_ID")]
    pub user_id: Option<String>,

    #[arg(long, env = "ZAVORA_SESSION_ID")]
    pub session_id: Option<String>,

    #[arg(long, env = "ZAVORA_SESSION_BACKEND", value_enum)]
    pub session_backend: Option<SessionBackend>,

    #[arg(long, env = "ZAVORA_SESSION_DB_URL")]
    pub session_db_url: Option<String>,

    #[arg(long, env = "ZAVORA_SHOW_SENSITIVE_CONFIG", default_value_t = false)]
    pub show_sensitive_config: bool,

    #[arg(long, env = "ZAVORA_RETRIEVAL_BACKEND", value_enum)]
    pub retrieval_backend: Option<RetrievalBackend>,

    #[arg(long, env = "ZAVORA_RETRIEVAL_DOC_PATH")]
    pub retrieval_doc_path: Option<String>,

    #[arg(long, env = "ZAVORA_RETRIEVAL_MAX_CHUNKS")]
    pub retrieval_max_chunks: Option<usize>,

    #[arg(long, env = "ZAVORA_RETRIEVAL_MAX_CHARS")]
    pub retrieval_max_chars: Option<usize>,

    #[arg(long, env = "ZAVORA_RETRIEVAL_MIN_SCORE")]
    pub retrieval_min_score: Option<usize>,

    #[arg(long, env = "ZAVORA_TOOL_CONFIRMATION_MODE", value_enum)]
    pub tool_confirmation_mode: Option<ToolConfirmationMode>,

    #[arg(long, env = "ZAVORA_REQUIRE_CONFIRM_TOOL")]
    pub require_confirm_tool: Vec<String>,

    #[arg(long, env = "ZAVORA_APPROVE_TOOL")]
    pub approve_tool: Vec<String>,

    #[arg(long, env = "ZAVORA_TOOL_TIMEOUT_SECS")]
    pub tool_timeout_secs: Option<u64>,

    #[arg(long, env = "ZAVORA_TOOL_RETRY_ATTEMPTS")]
    pub tool_retry_attempts: Option<u32>,

    #[arg(long, env = "ZAVORA_TOOL_RETRY_DELAY_MS")]
    pub tool_retry_delay_ms: Option<u64>,

    #[arg(long, env = "ZAVORA_TELEMETRY_ENABLED", action = clap::ArgAction::Set)]
    pub telemetry_enabled: Option<bool>,

    #[arg(long, env = "ZAVORA_TELEMETRY_PATH")]
    pub telemetry_path: Option<String>,

    #[arg(long, env = "ZAVORA_GUARDRAIL_INPUT_MODE", value_enum)]
    pub guardrail_input_mode: Option<GuardrailMode>,

    #[arg(long, env = "ZAVORA_GUARDRAIL_OUTPUT_MODE", value_enum)]
    pub guardrail_output_mode: Option<GuardrailMode>,

    #[arg(long, env = "ZAVORA_GUARDRAIL_TERM")]
    pub guardrail_term: Vec<String>,

    #[arg(long, env = "ZAVORA_GUARDRAIL_REDACT_REPLACEMENT")]
    pub guardrail_redact_replacement: Option<String>,

    #[arg(long, env = "RUST_LOG", default_value = "error")]
    pub log_filter: String,

    #[command(subcommand)]
    pub command: Option<Commands>,
}

#[derive(Debug, Subcommand)]
pub enum Commands {
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

pub fn workflow_mode_label(mode: WorkflowMode) -> &'static str {
    match mode {
        WorkflowMode::Single => "single",
        WorkflowMode::Sequential => "sequential",
        WorkflowMode::Parallel => "parallel",
        WorkflowMode::Loop => "loop",
        WorkflowMode::Graph => "graph",
    }
}

pub fn command_label(command: &Commands) -> String {
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
