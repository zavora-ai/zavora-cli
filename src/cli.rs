use clap::{Parser, Subcommand, ValueEnum};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use crate::capabilities::CapabilityCategory;

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum, Deserialize, Serialize)]
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

impl std::fmt::Display for Provider {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Auto => "Auto",
            Self::Gemini => "Gemini",
            Self::Openai => "OpenAI",
            Self::Anthropic => "Anthropic",
            Self::Deepseek => "DeepSeek",
            Self::Groq => "Groq",
            Self::Ollama => "Ollama",
        })
    }
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum WorkflowMode {
    Single,
    Sequential,
    Parallel,
    Loop,
    Graph,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum SessionBackend {
    Memory,
    Sqlite,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum RetrievalBackend {
    Disabled,
    Local,
    Semantic,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ToolConfirmationMode {
    Never,
    McpOnly,
    Always,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum GuardrailMode {
    Disabled,
    Observe,
    Block,
    Redact,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum OutputFormat {
    Text,
    Json,
    #[value(alias = "jsonl", alias = "streaming-json")]
    StreamJson,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum RalphPhase {
    Prd,
    Architect,
    Loop,
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
    #[command(about = "Run one task with a named specialist agent")]
    Run {
        #[arg(long)]
        name: String,
        task: Vec<String>,
    },
}

#[derive(Debug, Subcommand)]
pub enum CapabilityCommands {
    #[command(about = "List live skills alongside bundled capability recipes")]
    List {
        #[arg(long, value_enum)]
        category: Option<CapabilityCategory>,
        #[arg(long, default_value_t = false)]
        enabled: bool,
        #[arg(long, default_value_t = false)]
        json: bool,
    },
    #[command(about = "Search capability packs, bundled servers, and skills")]
    Search {
        query: Vec<String>,
        #[arg(long, default_value_t = false)]
        json: bool,
    },
    #[command(about = "Show one capability pack")]
    Info {
        id: String,
        #[arg(long, default_value_t = false)]
        json: bool,
    },
    #[command(about = "Enable a capability pack for this workspace")]
    Enable { id: String },
    #[command(about = "Disable a capability pack for this workspace")]
    Disable { id: String },
}

#[derive(Debug, Subcommand)]
pub enum McpCommands {
    #[command(about = "Browse Zavora's curated MCP server catalog")]
    Catalog {
        query: Vec<String>,
        #[arg(long, default_value_t = false)]
        json: bool,
    },
    #[command(about = "Add a curated MCP server to the active profile")]
    Add { server: String },
    #[command(about = "Remove an MCP server from the active profile")]
    Remove { server: String },
    #[command(about = "Enable an MCP server in the active profile")]
    Enable { server: String },
    #[command(about = "Disable an MCP server in the active profile")]
    Disable { server: String },
    #[command(about = "Authenticate an OAuth-enabled MCP server")]
    Auth { server: String },
    #[command(about = "List MCP servers configured for the active profile")]
    List,
    #[command(about = "Discover MCP tools from configured servers (or a specific server)")]
    Discover {
        #[arg(long)]
        server: Option<String>,
    },
    #[command(about = "Show resolved configuration for one MCP server")]
    Info { server: String },
    #[command(about = "Diagnose MCP server configuration and connectivity")]
    Doctor {
        #[arg(long)]
        server: Option<String>,
        #[arg(long, default_value_t = false)]
        json: bool,
    },
    #[command(about = "List or read resources published by an MCP server")]
    Resources {
        server: String,
        #[arg(long)]
        uri: Option<String>,
        #[arg(long, default_value_t = false)]
        json: bool,
    },
    #[command(about = "List or resolve prompts published by an MCP server")]
    Prompts {
        server: String,
        #[arg(long)]
        name: Option<String>,
        #[arg(long)]
        arguments: Option<String>,
        #[arg(long, default_value_t = false)]
        json: bool,
    },
    #[command(about = "Show MCP 2026 protocol and client support status")]
    Protocol {
        #[arg(long, default_value_t = false)]
        json: bool,
    },
    #[command(about = "Run as an MCP server over stdio, exposing built-in tools")]
    Serve,
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
pub enum SkillCommands {
    #[command(about = "Search the Zavora skill registry")]
    Search {
        query: Vec<String>,
        #[arg(long, default_value_t = false)]
        json: bool,
    },
    #[command(about = "List discovered standard and compatible workspace skills")]
    List {
        #[arg(long, default_value_t = false)]
        json: bool,
    },
    #[command(about = "Show one resolved skill and its source")]
    Info {
        name: String,
        #[arg(long, default_value_t = false)]
        json: bool,
    },
    #[command(about = "Validate a SKILL.md file or skill directory")]
    Validate {
        path: PathBuf,
        #[arg(long, default_value_t = false)]
        json: bool,
    },
    #[command(about = "Install a skill from the Zavora registry, a directory, or Git")]
    Install {
        source: String,
        #[arg(long, value_enum, default_value_t = ExtensionScope::Workspace)]
        scope: ExtensionScope,
        #[arg(long, default_value_t = false)]
        link: bool,
    },
    #[command(about = "Link a local skill directory for live development")]
    Link {
        source: String,
        #[arg(long, value_enum, default_value_t = ExtensionScope::Workspace)]
        scope: ExtensionScope,
    },
    #[command(about = "Update one installed skill, or every managed skill")]
    Update {
        name: Option<String>,
        #[arg(long, value_enum, default_value_t = ExtensionScope::Workspace)]
        scope: ExtensionScope,
    },
    #[command(about = "Enable an installed skill")]
    Enable {
        name: String,
        #[arg(long, value_enum, default_value_t = ExtensionScope::Workspace)]
        scope: ExtensionScope,
    },
    #[command(about = "Disable an installed skill")]
    Disable {
        name: String,
        #[arg(long, value_enum, default_value_t = ExtensionScope::Workspace)]
        scope: ExtensionScope,
    },
    #[command(about = "Uninstall a managed skill or unlink a linked skill")]
    Uninstall {
        name: String,
        #[arg(long, value_enum, default_value_t = ExtensionScope::Workspace)]
        scope: ExtensionScope,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ExtensionScope {
    Workspace,
    User,
}

impl std::fmt::Display for ExtensionScope {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Workspace => "workspace",
            Self::User => "user",
        })
    }
}

#[derive(Debug, Subcommand)]
pub enum PluginCommands {
    #[command(about = "List normalized plugins and extensions from every supported ecosystem")]
    List {
        #[arg(long, default_value_t = false)]
        json: bool,
    },
    #[command(about = "Show a plugin's normalized manifest and contributed components")]
    Info {
        name: String,
        #[arg(long, default_value_t = false)]
        json: bool,
    },
    #[command(about = "Validate a Zavora, Codex, Claude, Gemini, Grok, or OpenCode package")]
    Validate {
        path: PathBuf,
        #[arg(long, default_value_t = false)]
        json: bool,
    },
    #[command(about = "Install a plugin from a directory or Git repository")]
    Install {
        source: String,
        #[arg(long, value_enum, default_value_t = ExtensionScope::Workspace)]
        scope: ExtensionScope,
        #[arg(long, default_value_t = false)]
        link: bool,
    },
    #[command(about = "Link a local plugin directory for live development")]
    Link {
        source: String,
        #[arg(long, value_enum, default_value_t = ExtensionScope::Workspace)]
        scope: ExtensionScope,
    },
    #[command(about = "Update one installed plugin, or every managed plugin")]
    Update {
        name: Option<String>,
        #[arg(long, value_enum, default_value_t = ExtensionScope::Workspace)]
        scope: ExtensionScope,
    },
    #[command(about = "Enable an installed plugin")]
    Enable {
        name: String,
        #[arg(long, value_enum, default_value_t = ExtensionScope::Workspace)]
        scope: ExtensionScope,
    },
    #[command(about = "Disable an installed plugin")]
    Disable {
        name: String,
        #[arg(long, value_enum, default_value_t = ExtensionScope::Workspace)]
        scope: ExtensionScope,
    },
    #[command(about = "Uninstall a managed plugin or unlink a linked plugin")]
    Uninstall {
        name: String,
        #[arg(long, value_enum, default_value_t = ExtensionScope::Workspace)]
        scope: ExtensionScope,
    },
    #[command(about = "Diagnose manifests, paths, executable entrypoints, and runtime components")]
    Doctor {
        #[arg(long, default_value_t = false)]
        json: bool,
    },
}

#[derive(Debug, Subcommand)]
pub enum InstructionCommands {
    #[command(about = "List active and deferred project instruction sources")]
    List {
        #[arg(long, default_value_t = false)]
        json: bool,
    },
    #[command(about = "Show the exact resolved project instruction context")]
    Show {
        #[arg(long, default_value_t = false)]
        json: bool,
    },
}

#[cfg(feature = "rag")]
#[derive(Debug, Subcommand)]
pub enum RagCommands {
    #[command(about = "Ingest documents into the RAG vector store")]
    Ingest {
        #[arg(help = "Path to file or directory to ingest")]
        path: String,
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
  zavora-cli ask --output-format json \"Summarize this repository\"\n\
  git diff | zavora-cli ask --output-format stream-json \"Review this patch\"\n\
  zavora-cli ask --file README.md --file Cargo.toml \"Compare the project metadata\"\n\
  zavora-cli --provider openai --model gpt-4.1 chat\n\
  zavora-cli workflow sequential \"Plan a v0.2.0 rollout\"\n\
  zavora-cli --session-backend sqlite --session-db-url sqlite://.zavora/sessions.db sessions list\n\
  zavora-cli --session-backend sqlite --session-db-url sqlite://.zavora/sessions.db sessions prune --keep 20 --dry-run\n\
  zavora-cli agents list\n\
  zavora-cli agents run --name research_agent \"Compare the primary sources\"\n\
  zavora-cli instructions show --json\n\
  zavora-cli skills search device\n\
  zavora-cli skills install device-fleet-management\n\
  zavora-cli plugins validate ./my-plugin\n\
  zavora-cli capabilities list --category productivity\n\
  zavora-cli capabilities enable productivity.office\n\
  zavora-cli mcp catalog office\n\
  zavora-cli mcp add docx-mcp\n\
  zavora-cli mcp list\n\
  zavora-cli mcp discover --server ops-tools\n\
  zavora-cli mcp resources ops-tools\n\
  zavora-cli mcp protocol --json\n\
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
  - In chat, use /help for command discovery and /capabilities, /mcps, /skills, /plugins, /agents, /inspect, /doctor.";

#[derive(Debug, Parser)]
#[command(name = "zavora-cli")]
// Taken from `CARGO_PKG_VERSION`, so `--version` cannot drift from the package.
// Without it clap declines the flag entirely, which is the first thing anyone runs
// against a released binary and the first thing packaging scripts read.
#[command(version)]
#[command(about = "Rust CLI agent shell built on ADK-Rust")]
#[command(after_long_help = CLI_EXAMPLES)]
pub struct Cli {
    /// Output contract for model-running commands.
    #[arg(
        short = 'o',
        long,
        global = true,
        env = "ZAVORA_OUTPUT_FORMAT",
        value_enum,
        default_value_t = OutputFormat::Text
    )]
    pub output_format: OutputFormat,

    /// Append a UTF-8 file to the prompt. May be repeated.
    #[arg(short = 'f', long = "file", global = true)]
    pub input_files: Vec<PathBuf>,

    /// Read prompt context from stdin, even when stdin is a terminal.
    #[arg(long, global = true, conflicts_with = "no_stdin")]
    pub stdin: bool,

    /// Never consume piped stdin automatically.
    #[arg(long, global = true, conflicts_with = "stdin")]
    pub no_stdin: bool,

    /// Explicitly approve every available tool for this process.
    #[arg(long, global = true, alias = "yolo")]
    pub always_approve: bool,

    #[arg(long, env = "ZAVORA_PROVIDER", value_enum, default_value_t = Provider::Auto)]
    pub provider: Provider,

    #[arg(long, env = "ZAVORA_MODEL")]
    pub model: Option<String>,

    /// Explicit worker provider. Overrides the legacy --provider flag.
    #[arg(long, env = "ZAVORA_WORKER_PROVIDER", value_enum)]
    pub worker_provider: Option<Provider>,

    /// Model used for routine conversation, tools, and implementation work.
    #[arg(long, env = "ZAVORA_WORKER_MODEL")]
    pub worker_model: Option<String>,

    /// Provider used by the bounded planning specialist.
    #[arg(long, env = "ZAVORA_PLANNER_PROVIDER", value_enum)]
    pub planner_provider: Option<Provider>,

    /// Strong model used only when a complex task needs a plan or material replan.
    #[arg(long, env = "ZAVORA_PLANNER_MODEL")]
    pub planner_model: Option<String>,

    /// Maximum strong-planner calls allowed during one CLI process (default: 4).
    #[arg(long, env = "ZAVORA_PLANNER_CALL_BUDGET")]
    pub planner_call_budget: Option<u32>,

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
    Ask { prompt: Vec<String> },
    #[command(about = "Run interactive chat mode")]
    Chat,
    #[command(about = "List model roles, quota pools, and selectable models")]
    Models,
    #[command(about = "Run a workflow mode (single, sequential, parallel, loop) for a prompt")]
    Workflow {
        #[arg(value_enum)]
        mode: WorkflowMode,
        prompt: Vec<String>,
        #[arg(long, default_value_t = 4)]
        max_iterations: u32,
    },
    #[command(about = "Generate a release-oriented plan from a product goal")]
    ReleasePlan {
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
    #[command(about = "Inspect live capabilities and bundled MCP recipes")]
    Capabilities {
        #[command(subcommand)]
        command: CapabilityCommands,
    },
    #[command(about = "Discover, configure, diagnose, and run MCP servers")]
    Mcp {
        #[command(subcommand)]
        command: McpCommands,
    },
    #[command(about = "Manage session lifecycle (list/show/delete/prune)")]
    Sessions {
        #[command(subcommand)]
        command: SessionCommands,
    },
    #[command(about = "Discover standard and compatible agent skills")]
    Skills {
        #[command(subcommand)]
        command: SkillCommands,
    },
    #[command(
        about = "Install and manage cross-CLI plugins and extensions",
        visible_alias = "extensions"
    )]
    Plugins {
        #[command(subcommand)]
        command: PluginCommands,
    },
    #[command(about = "Inspect AGENTS.md, GEMINI.md, and CLAUDE.md context")]
    Instructions {
        #[command(subcommand)]
        command: InstructionCommands,
    },
    #[cfg(feature = "rag")]
    #[command(about = "RAG document ingestion and retrieval")]
    Rag {
        #[command(subcommand)]
        command: RagCommands,
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
    #[command(about = "Run the Ralph autonomous development pipeline")]
    Ralph {
        #[arg(required_unless_present = "resume")]
        prompt: Vec<String>,

        #[arg(long, value_enum)]
        phase: Option<RalphPhase>,

        #[arg(long, default_value_t = false)]
        resume: bool,

        #[arg(long)]
        output_dir: Option<String>,
    },
    #[command(about = "Run the interactive provider setup wizard")]
    Setup,
    #[command(about = "Initialize LSP configuration for code intelligence")]
    LspInit,
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
        Commands::Models => "models".to_string(),
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
            AgentCommands::Run { .. } => "agents.run".to_string(),
        },
        Commands::Capabilities { command } => match command {
            CapabilityCommands::List { .. } => "capabilities.list".to_string(),
            CapabilityCommands::Search { .. } => "capabilities.search".to_string(),
            CapabilityCommands::Info { .. } => "capabilities.info".to_string(),
            CapabilityCommands::Enable { .. } => "capabilities.enable".to_string(),
            CapabilityCommands::Disable { .. } => "capabilities.disable".to_string(),
        },
        Commands::Mcp { command } => match command {
            McpCommands::Catalog { .. } => "mcp.catalog".to_string(),
            McpCommands::Add { .. } => "mcp.add".to_string(),
            McpCommands::Remove { .. } => "mcp.remove".to_string(),
            McpCommands::Enable { .. } => "mcp.enable".to_string(),
            McpCommands::Disable { .. } => "mcp.disable".to_string(),
            McpCommands::Auth { .. } => "mcp.auth".to_string(),
            McpCommands::List => "mcp.list".to_string(),
            McpCommands::Discover { .. } => "mcp.discover".to_string(),
            McpCommands::Info { .. } => "mcp.info".to_string(),
            McpCommands::Doctor { .. } => "mcp.doctor".to_string(),
            McpCommands::Resources { .. } => "mcp.resources".to_string(),
            McpCommands::Prompts { .. } => "mcp.prompts".to_string(),
            McpCommands::Protocol { .. } => "mcp.protocol".to_string(),
            McpCommands::Serve => "mcp.serve".to_string(),
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
        Commands::Skills { command } => match command {
            SkillCommands::List { .. } => "skills.list".to_string(),
            SkillCommands::Search { .. } => "skills.search".to_string(),
            SkillCommands::Info { .. } => "skills.info".to_string(),
            SkillCommands::Validate { .. } => "skills.validate".to_string(),
            SkillCommands::Install { .. } => "skills.install".to_string(),
            SkillCommands::Link { .. } => "skills.link".to_string(),
            SkillCommands::Update { .. } => "skills.update".to_string(),
            SkillCommands::Enable { .. } => "skills.enable".to_string(),
            SkillCommands::Disable { .. } => "skills.disable".to_string(),
            SkillCommands::Uninstall { .. } => "skills.uninstall".to_string(),
        },
        Commands::Plugins { command } => match command {
            PluginCommands::List { .. } => "plugins.list".to_string(),
            PluginCommands::Info { .. } => "plugins.info".to_string(),
            PluginCommands::Validate { .. } => "plugins.validate".to_string(),
            PluginCommands::Install { .. } => "plugins.install".to_string(),
            PluginCommands::Link { .. } => "plugins.link".to_string(),
            PluginCommands::Update { .. } => "plugins.update".to_string(),
            PluginCommands::Enable { .. } => "plugins.enable".to_string(),
            PluginCommands::Disable { .. } => "plugins.disable".to_string(),
            PluginCommands::Uninstall { .. } => "plugins.uninstall".to_string(),
            PluginCommands::Doctor { .. } => "plugins.doctor".to_string(),
        },
        Commands::Instructions { command } => match command {
            InstructionCommands::List { .. } => "instructions.list".to_string(),
            InstructionCommands::Show { .. } => "instructions.show".to_string(),
        },
        #[cfg(feature = "rag")]
        Commands::Rag { command } => match command {
            RagCommands::Ingest { .. } => "rag.ingest".to_string(),
        },
        Commands::Eval { command } => match command {
            EvalCommands::Run { .. } => "eval.run".to_string(),
        },
        Commands::Server { command } => match command {
            ServerCommands::Serve { .. } => "server.serve".to_string(),
            ServerCommands::A2aSmoke => "server.a2a-smoke".to_string(),
        },
        Commands::Setup => "setup".to_string(),
        Commands::LspInit => "lsp.init".to_string(),
        Commands::Ralph { .. } => "ralph".to_string(),
    }
}
