use std::collections::HashMap;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use adk_rust::futures::StreamExt;
use adk_rust::prelude::{
    Agent, AnthropicClient, AnthropicConfig, Content, DeepSeekClient, DeepSeekConfig, Event,
    ExitLoopTool, FunctionTool, GeminiModel, GroqClient, GroqConfig, InMemoryArtifactService,
    InMemorySessionService, Llm, LlmAgentBuilder, LoopAgent, OllamaConfig, OllamaModel,
    OpenAIClient, OpenAIConfig, ParallelAgent, Part, Runner, RunnerConfig, SequentialAgent, Tool,
};
use adk_session::{
    CreateRequest, DatabaseSessionService, DeleteRequest, GetRequest, ListRequest, SessionService,
};
use anyhow::{Context, Result};
use clap::{Parser, Subcommand, ValueEnum};
use serde::Deserialize;
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
}

#[derive(Debug, Subcommand)]
enum ProfileCommands {
    #[command(about = "List configured profiles and highlight the active profile")]
    List,
    #[command(about = "Show the active profile's resolved runtime settings")]
    Show,
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

const CLI_EXAMPLES: &str = "Examples:\n\
  zavora-cli ask \"Design a Rust CLI with release-based milestones\"\n\
  zavora-cli --provider openai --model gpt-4o-mini chat\n\
  zavora-cli workflow sequential \"Plan a v0.2.0 rollout\"\n\
  zavora-cli --session-backend sqlite --session-db-url sqlite://.zavora/sessions.db sessions list\n\
  zavora-cli --session-backend sqlite --session-db-url sqlite://.zavora/sessions.db sessions prune --keep 20 --dry-run\n\
\n\
Switching behavior:\n\
  - Use --provider/--model to switch runtime model selection per invocation.\n\
  - In chat, use /provider <name>, /model <id>, and /status for in-session switching.";

#[derive(Debug, Parser)]
#[command(name = "zavora-cli")]
#[command(about = "Rust CLI agent shell built on ADK-Rust")]
#[command(after_long_help = CLI_EXAMPLES)]
struct Cli {
    #[arg(long, env = "ZAVORA_PROVIDER", value_enum, default_value_t = Provider::Auto)]
    provider: Provider,

    #[arg(long, env = "ZAVORA_MODEL")]
    model: Option<String>,

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

    #[arg(long, env = "ZAVORA_RETRIEVAL_BACKEND", value_enum)]
    retrieval_backend: Option<RetrievalBackend>,

    #[arg(long, env = "ZAVORA_RETRIEVAL_DOC_PATH")]
    retrieval_doc_path: Option<String>,

    #[arg(long, env = "ZAVORA_RETRIEVAL_MAX_CHUNKS")]
    retrieval_max_chunks: Option<usize>,

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
    #[command(about = "Manage session lifecycle (list/show/delete/prune)")]
    Sessions {
        #[command(subcommand)]
        command: SessionCommands,
    },
}

#[derive(Debug, Clone)]
struct RuntimeConfig {
    profile: String,
    config_path: String,
    provider: Provider,
    model: Option<String>,
    app_name: String,
    user_id: String,
    session_id: String,
    session_backend: SessionBackend,
    session_db_url: String,
    retrieval_backend: RetrievalBackend,
    retrieval_doc_path: Option<String>,
    retrieval_max_chunks: usize,
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

fn format_cli_error(err: &anyhow::Error) -> String {
    let category = categorize_error(err);
    format!("[{}] {}\nHint: {}", category.code(), err, category.hint())
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    if let Err(err) = run_cli(cli).await {
        eprintln!("{}", format_cli_error(&err));
        tracing::error!(category = %categorize_error(&err).code(), error = %err, "command failed");
        std::process::exit(1);
    }

    Ok(())
}

async fn run_cli(cli: Cli) -> Result<()> {
    init_tracing(&cli.log_filter)?;
    let profiles = load_profiles(&cli.config_path)?;
    let cfg = resolve_runtime_config(&cli, &profiles)?;
    let retrieval_service = build_retrieval_service(&cfg)?;
    tracing::info!(
        backend = retrieval_service.backend_name(),
        max_chunks = cfg.retrieval_max_chunks,
        "Using retrieval backend"
    );

    match cli.command {
        Commands::Ask { prompt } => {
            let (model, resolved_provider, model_name) = resolve_model(&cfg)?;
            tracing::info!(provider = ?resolved_provider, model = %model_name, "Using model");
            let agent = build_single_agent(model)?;
            let runner = build_runner(agent, &cfg).await?;
            let prompt = prompt.join(" ");
            let answer =
                run_prompt_with_retrieval(&runner, &cfg, &prompt, retrieval_service.as_ref())
                    .await?;
            println!("{answer}");
        }
        Commands::Chat => {
            run_chat(cfg.clone(), retrieval_service.clone()).await?;
        }
        Commands::Workflow {
            mode,
            prompt,
            max_iterations,
        } => {
            let (model, resolved_provider, model_name) = resolve_model(&cfg)?;
            tracing::info!(provider = ?resolved_provider, model = %model_name, workflow = ?mode, "Using workflow");
            let agent = build_workflow_agent(mode, model, max_iterations)?;
            let runner = build_runner(agent, &cfg).await?;
            let prompt = prompt.join(" ");
            let answer =
                run_prompt_with_retrieval(&runner, &cfg, &prompt, retrieval_service.as_ref())
                    .await?;
            println!("{answer}");
        }
        Commands::ReleasePlan { goal, releases } => {
            let (model, resolved_provider, model_name) = resolve_model(&cfg)?;
            tracing::info!(provider = ?resolved_provider, model = %model_name, releases, "Generating release plan");
            let agent = build_release_planning_agent(model, releases)?;
            let runner = build_runner(agent, &cfg).await?;
            let prompt = goal.join(" ");
            let answer =
                run_prompt_with_retrieval(&runner, &cfg, &prompt, retrieval_service.as_ref())
                    .await?;
            println!("{answer}");
        }
        Commands::Doctor => {
            run_doctor(&cfg).await?;
        }
        Commands::Migrate => {
            run_migrate(&cfg).await?;
        }
        Commands::Profiles { command } => match command {
            ProfileCommands::List => run_profiles_list(&profiles, &cfg)?,
            ProfileCommands::Show => run_profiles_show(&cfg)?,
        },
        Commands::Sessions { command } => match command {
            SessionCommands::List => run_sessions_list(&cfg).await?,
            SessionCommands::Show { session_id, recent } => {
                run_sessions_show(&cfg, session_id, recent).await?
            }
            SessionCommands::Delete { session_id, force } => {
                run_sessions_delete(&cfg, session_id, force).await?
            }
            SessionCommands::Prune {
                keep,
                dry_run,
                force,
            } => run_sessions_prune(&cfg, keep, dry_run, force).await?,
        },
    }

    Ok(())
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

fn resolve_runtime_config(cli: &Cli, profiles: &ProfilesFile) -> Result<RuntimeConfig> {
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

    let provider = if cli.provider != Provider::Auto {
        cli.provider
    } else {
        profile.provider.unwrap_or(Provider::Auto)
    };

    Ok(RuntimeConfig {
        profile: selected.to_string(),
        config_path: cli.config_path.clone(),
        provider,
        model: cli.model.clone().or(profile.model),
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
    })
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
    println!("Session ID: {}", cfg.session_id);
    println!("Session backend: {:?}", cfg.session_backend);
    println!("Session DB URL: {}", cfg.session_db_url);
    println!("Retrieval backend: {:?}", cfg.retrieval_backend);
    println!(
        "Retrieval doc path: {}",
        cfg.retrieval_doc_path
            .as_deref()
            .unwrap_or("<not configured>")
    );
    println!("Retrieval max chunks: {}", cfg.retrieval_max_chunks);
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

impl LocalFileRetrievalService {
    fn load(path: &str) -> Result<Self> {
        let content = std::fs::read_to_string(path)
            .with_context(|| format!("failed to read retrieval doc at '{}'", path))?;
        let chunks = content
            .split("\n\n")
            .map(str::trim)
            .filter(|chunk| !chunk.is_empty())
            .enumerate()
            .map(|(index, text)| RetrievedChunk {
                source: format!("local:{}#{}", path, index + 1),
                text: text.to_string(),
                score: 0,
            })
            .collect::<Vec<RetrievedChunk>>();

        Ok(Self { chunks })
    }
}

impl RetrievalService for LocalFileRetrievalService {
    fn backend_name(&self) -> &'static str {
        "local"
    }

    fn retrieve(&self, query: &str, max_chunks: usize) -> Result<Vec<RetrievedChunk>> {
        let terms = query
            .split_whitespace()
            .map(|token| token.trim_matches(|c: char| !c.is_ascii_alphanumeric()))
            .map(str::to_ascii_lowercase)
            .filter(|token| token.len() > 2)
            .collect::<Vec<String>>();

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
                    .filter(|term| body.contains(term.as_str()))
                    .count();
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
    }
}

fn augment_prompt_with_retrieval(
    retrieval: &dyn RetrievalService,
    prompt: &str,
    max_chunks: usize,
) -> Result<String> {
    let chunks = retrieval.retrieve(prompt, max_chunks)?;
    if chunks.is_empty() {
        return Ok(prompt.to_string());
    }

    let mut out = String::new();
    out.push_str("Retrieved context (use if relevant):\n");
    for (index, chunk) in chunks.iter().enumerate() {
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

fn build_tools() -> Vec<Arc<dyn Tool>> {
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

    vec![Arc::new(current_time), Arc::new(release_template)]
}

fn build_single_agent(model: Arc<dyn Llm>) -> Result<Arc<dyn Agent>> {
    let mut builder = LlmAgentBuilder::new("assistant")
        .description("General purpose engineering assistant")
        .instruction(
            "You are a pragmatic AI engineer. Prioritize direct, actionable output, and when \
             planning work always prefer release-oriented increments.",
        )
        .model(model);

    for tool in build_tools() {
        builder = builder.tool(tool);
    }

    Ok(Arc::new(builder.build()?))
}

fn build_workflow_agent(
    mode: WorkflowMode,
    model: Arc<dyn Llm>,
    max_iterations: u32,
) -> Result<Arc<dyn Agent>> {
    match mode {
        WorkflowMode::Single => build_single_agent(model),
        WorkflowMode::Sequential => build_sequential_agent(model),
        WorkflowMode::Parallel => build_parallel_agent(model),
        WorkflowMode::Loop => build_loop_agent(model, max_iterations),
    }
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
    let session_service = build_session_service(cfg).await?;
    build_runner_with_session_service(agent, cfg, session_service).await
}

async fn build_runner_with_session_service(
    agent: Arc<dyn Agent>,
    cfg: &RuntimeConfig,
    session_service: Arc<dyn SessionService>,
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
        run_config: None,
        compaction_config: None,
    })
    .context("failed to build ADK runner")
}

async fn build_single_runner_for_chat(
    cfg: &RuntimeConfig,
    session_service: Arc<dyn SessionService>,
) -> Result<(Runner, Provider, String)> {
    let (model, resolved_provider, model_name) = resolve_model(cfg)?;
    let agent = build_single_agent(model)?;
    let runner = build_runner_with_session_service(agent, cfg, session_service).await?;
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
        .with_context(|| format!("failed to open sqlite session database at {db_url}"))?;
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

    if final_text.starts_with(emitted) {
        let suffix = &final_text[emitted.len()..];
        if suffix.is_empty() {
            return None;
        }
        return Some(suffix.to_string());
    }

    Some(format!("\n{final_text}"))
}

async fn run_prompt(runner: &Runner, cfg: &RuntimeConfig, prompt: &str) -> Result<String> {
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
) -> Result<String> {
    let enriched = augment_prompt_with_retrieval(retrieval, prompt, cfg.retrieval_max_chunks)?;
    run_prompt(runner, cfg, &enriched).await
}

async fn run_chat(
    mut cfg: RuntimeConfig,
    retrieval_service: Arc<dyn RetrievalService>,
) -> Result<()> {
    let session_service = build_session_service(&cfg).await?;
    let (mut runner, mut resolved_provider, mut model_name) =
        build_single_runner_for_chat(&cfg, session_service.clone()).await?;

    cfg.provider = resolved_provider;
    cfg.model = Some(model_name.clone());

    tracing::info!(provider = ?resolved_provider, model = %model_name, "Using model");
    println!(
        "Interactive mode started. Type /exit to quit. Use /provider <name> or /model <id> to switch runtime."
    );
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

        if input.eq_ignore_ascii_case("/status") {
            println!(
                "profile={} provider={:?} model={} session_id={}",
                cfg.profile, resolved_provider, model_name, cfg.session_id
            );
            continue;
        }

        if let Some(rest) = input.strip_prefix("/provider") {
            let provider_name = rest.trim();
            if provider_name.is_empty() {
                println!("Usage: /provider <auto|gemini|openai|anthropic|deepseek|groq|ollama>");
                continue;
            }

            let new_provider = parse_provider_name(provider_name)?;
            let mut switched_cfg = cfg.clone();
            switched_cfg.provider = new_provider;
            switched_cfg.model = None;

            match build_single_runner_for_chat(&switched_cfg, session_service.clone()).await {
                Ok((new_runner, new_resolved_provider, new_model_name)) => {
                    runner = new_runner;
                    resolved_provider = new_resolved_provider;
                    model_name = new_model_name;
                    switched_cfg.provider = new_resolved_provider;
                    switched_cfg.model = Some(model_name.clone());
                    cfg = switched_cfg;
                    tracing::info!(provider = ?resolved_provider, model = %model_name, "Switched model provider");
                    println!(
                        "Switched provider to {:?} (model={}). Session continuity preserved.",
                        resolved_provider, model_name
                    );
                }
                Err(err) => {
                    eprintln!("{}", format_cli_error(&err));
                    println!(
                        "Provider remains {:?} (model={}).",
                        resolved_provider, model_name
                    );
                }
            }
            continue;
        }

        if let Some(rest) = input.strip_prefix("/model") {
            let next_model = rest.trim();
            if next_model.is_empty() {
                println!("Usage: /model <model-id>");
                continue;
            }

            let mut switched_cfg = cfg.clone();
            switched_cfg.model = Some(next_model.to_string());

            match build_single_runner_for_chat(&switched_cfg, session_service.clone()).await {
                Ok((new_runner, new_resolved_provider, new_model_name)) => {
                    runner = new_runner;
                    resolved_provider = new_resolved_provider;
                    model_name = new_model_name;
                    switched_cfg.provider = new_resolved_provider;
                    switched_cfg.model = Some(model_name.clone());
                    cfg = switched_cfg;
                    tracing::info!(provider = ?resolved_provider, model = %model_name, "Switched model");
                    println!(
                        "Switched model to '{}' on provider {:?}. Session continuity preserved.",
                        model_name, resolved_provider
                    );
                }
                Err(err) => {
                    eprintln!("{}", format_cli_error(&err));
                    println!(
                        "Model remains '{}' on provider {:?}.",
                        model_name, resolved_provider
                    );
                }
            }
            continue;
        }

        run_prompt_streaming_with_retrieval(&runner, &cfg, input, retrieval_service.as_ref())
            .await?;
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

async fn run_prompt_streaming(runner: &Runner, cfg: &RuntimeConfig, prompt: &str) -> Result<()> {
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
        return Ok(());
    }

    let fallback = tracker
        .resolve_text()
        .unwrap_or_else(|| NO_TEXTUAL_RESPONSE.to_string());

    println!("{fallback}");
    Ok(())
}

async fn run_prompt_streaming_with_retrieval(
    runner: &Runner,
    cfg: &RuntimeConfig,
    prompt: &str,
    retrieval: &dyn RetrievalService,
) -> Result<()> {
    let enriched = augment_prompt_with_retrieval(retrieval, prompt, cfg.retrieval_max_chunks)?;
    run_prompt_streaming(runner, cfg, &enriched).await
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
        "Retrieval: backend={:?}, doc_path={}, max_chunks={}",
        cfg.retrieval_backend,
        cfg.retrieval_doc_path
            .as_deref()
            .unwrap_or("<not configured>"),
        cfg.retrieval_max_chunks
    );

    if matches!(cfg.session_backend, SessionBackend::Sqlite) {
        let _service = open_sqlite_session_service(&cfg.session_db_url).await?;
        println!("SQLite session DB check: ok ({})", cfg.session_db_url);
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
                cfg.session_db_url
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
            provider: Provider::Auto,
            model: None,
            app_name: "test-app".to_string(),
            user_id: "test-user".to_string(),
            session_id: "test-session".to_string(),
            session_backend: SessionBackend::Memory,
            session_db_url: "sqlite://.zavora/test.db".to_string(),
            retrieval_backend: RetrievalBackend::Disabled,
            retrieval_doc_path: None,
            retrieval_max_chunks: 3,
        }
    }

    fn mock_model(text: &str) -> Arc<dyn Llm> {
        Arc::new(
            MockLlm::new("mock")
                .with_response(LlmResponse::new(Content::new("model").with_text(text))),
        )
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
            profile: profile.to_string(),
            config_path: config_path.to_string(),
            app_name: None,
            user_id: None,
            session_id: None,
            session_backend: None,
            session_db_url: None,
            retrieval_backend: None,
            retrieval_doc_path: None,
            retrieval_max_chunks: None,
            log_filter: "warn".to_string(),
            command: Commands::Doctor,
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
        let runner = build_runner(
            build_single_agent(mock_model("single response")).expect("agent should build"),
            &cfg,
        )
        .await
        .expect("runner should build");

        let out = run_prompt(&runner, &cfg, "hello")
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
        ];

        for mode in modes {
            let mut cfg = base_cfg();
            cfg.session_id = format!("session-{mode:?}");
            let runner = build_runner(
                build_workflow_agent(mode, mock_model("workflow response"), 1)
                    .expect("workflow should build"),
                &cfg,
            )
            .await
            .expect("runner should build");

            let out = run_prompt(&runner, &cfg, "build a plan")
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

        let runner_one = build_runner(
            build_single_agent(mock_model("first answer")).expect("agent should build"),
            &cfg,
        )
        .await
        .expect("runner should build");

        let _ = run_prompt(&runner_one, &cfg, "first prompt")
            .await
            .expect("first prompt should run");

        let runner_two = build_runner(
            build_single_agent(mock_model("second answer")).expect("agent should build"),
            &cfg,
        )
        .await
        .expect("second runner should build");

        let _ = run_prompt(&runner_two, &cfg, "second prompt")
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
        assert_eq!(cfg.app_name, "zavora-dev");
        assert_eq!(cfg.user_id, "dev-user");
        assert_eq!(cfg.session_id, "dev-session");
        assert_eq!(cfg.retrieval_backend, RetrievalBackend::Local);
        assert_eq!(cfg.retrieval_doc_path.as_deref(), Some("docs/knowledge.md"));
        assert_eq!(cfg.retrieval_max_chunks, 5);
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
        let out = augment_prompt_with_retrieval(&retrieval, prompt, 3)
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

    #[tokio::test]
    async fn sessions_show_missing_session_returns_session_category_error() {
        let cfg = base_cfg();
        let err = run_sessions_show(&cfg, Some("missing-session".to_string()), 10)
            .await
            .expect_err("missing session should error");

        assert_eq!(categorize_error(&err), ErrorCategory::Session);
        let rendered = format_cli_error(&err);
        assert!(
            rendered.contains("[SESSION]"),
            "expected session category marker in error output"
        );
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
        let session_service: Arc<dyn SessionService> = Arc::new(InMemorySessionService::new());

        let runner_one = build_runner_with_session_service(
            build_single_agent(mock_model("first answer")).expect("agent should build"),
            &cfg,
            session_service.clone(),
        )
        .await
        .expect("runner should build");
        run_prompt(&runner_one, &cfg, "first prompt")
            .await
            .expect("first prompt should run");

        let runner_two = build_runner_with_session_service(
            build_single_agent(mock_model("second answer")).expect("agent should build"),
            &cfg,
            session_service.clone(),
        )
        .await
        .expect("second runner should build");
        run_prompt(&runner_two, &cfg, "second prompt")
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

        let (_runner, provider, model_name) =
            build_single_runner_for_chat(&cfg, session_service.clone())
                .await
                .expect("chat runner should build for ollama");

        assert_eq!(provider, Provider::Ollama);
        assert_eq!(model_name, "llama3.2");
    }
}
