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
use adk_session::{CreateRequest, DatabaseSessionService, GetRequest, ListRequest, SessionService};
use anyhow::{Context, Result};
use clap::{Parser, Subcommand, ValueEnum};
use serde_json::{Value, json};
use tracing::level_filters::LevelFilter;

#[derive(Debug, Clone, Copy, ValueEnum)]
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

#[derive(Debug, Clone, Copy, ValueEnum)]
enum SessionBackend {
    Memory,
    Sqlite,
}

#[derive(Debug, Subcommand)]
enum SessionCommands {
    List,
    Show {
        #[arg(long)]
        session_id: Option<String>,
        #[arg(long, default_value_t = 20)]
        recent: usize,
    },
}

#[derive(Debug, Parser)]
#[command(name = "zavora-cli")]
#[command(about = "Rust CLI agent shell built on ADK-Rust")]
struct Cli {
    #[arg(long, env = "ZAVORA_PROVIDER", value_enum, default_value_t = Provider::Auto)]
    provider: Provider,

    #[arg(long, env = "ZAVORA_MODEL")]
    model: Option<String>,

    #[arg(long, env = "ZAVORA_APP_NAME", default_value = "zavora-cli")]
    app_name: String,

    #[arg(long, env = "ZAVORA_USER_ID", default_value = "local-user")]
    user_id: String,

    #[arg(long, env = "ZAVORA_SESSION_ID", default_value = "default-session")]
    session_id: String,

    #[arg(long, env = "ZAVORA_SESSION_BACKEND", value_enum, default_value_t = SessionBackend::Memory)]
    session_backend: SessionBackend,

    #[arg(
        long,
        env = "ZAVORA_SESSION_DB_URL",
        default_value = "sqlite://.zavora/sessions.db"
    )]
    session_db_url: String,

    #[arg(long, env = "RUST_LOG", default_value = "warn")]
    log_filter: String,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Debug, Subcommand)]
enum Commands {
    Ask {
        #[arg(required = true)]
        prompt: Vec<String>,
    },
    Chat,
    Workflow {
        #[arg(value_enum)]
        mode: WorkflowMode,
        #[arg(required = true)]
        prompt: Vec<String>,
        #[arg(long, default_value_t = 4)]
        max_iterations: u32,
    },
    ReleasePlan {
        #[arg(required = true)]
        goal: Vec<String>,
        #[arg(long, default_value_t = 3)]
        releases: u32,
    },
    Doctor,
    Migrate,
    Sessions {
        #[command(subcommand)]
        command: SessionCommands,
    },
}

#[derive(Debug, Clone)]
struct RuntimeConfig {
    provider: Provider,
    model: Option<String>,
    app_name: String,
    user_id: String,
    session_id: String,
    session_backend: SessionBackend,
    session_db_url: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    init_tracing(&cli.log_filter)?;

    let cfg = RuntimeConfig {
        provider: cli.provider,
        model: cli.model,
        app_name: cli.app_name,
        user_id: cli.user_id,
        session_id: cli.session_id,
        session_backend: cli.session_backend,
        session_db_url: cli.session_db_url,
    };

    match cli.command {
        Commands::Ask { prompt } => {
            let (model, resolved_provider, model_name) = resolve_model(&cfg)?;
            tracing::info!(provider = ?resolved_provider, model = %model_name, "Using model");
            let agent = build_single_agent(model)?;
            let runner = build_runner(agent, &cfg).await?;
            let prompt = prompt.join(" ");
            let answer = run_prompt(&runner, &cfg, &prompt).await?;
            println!("{answer}");
        }
        Commands::Chat => {
            let (model, resolved_provider, model_name) = resolve_model(&cfg)?;
            tracing::info!(provider = ?resolved_provider, model = %model_name, "Using model");
            let agent = build_single_agent(model)?;
            let runner = build_runner(agent, &cfg).await?;
            run_chat(&runner, &cfg).await?;
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
            let answer = run_prompt(&runner, &cfg, &prompt).await?;
            println!("{answer}");
        }
        Commands::ReleasePlan { goal, releases } => {
            let (model, resolved_provider, model_name) = resolve_model(&cfg)?;
            tracing::info!(provider = ?resolved_provider, model = %model_name, releases, "Generating release plan");
            let agent = build_release_planning_agent(model, releases)?;
            let runner = build_runner(agent, &cfg).await?;
            let prompt = goal.join(" ");
            let answer = run_prompt(&runner, &cfg, &prompt).await?;
            println!("{answer}");
        }
        Commands::Doctor => {
            run_doctor(&cfg).await?;
        }
        Commands::Migrate => {
            run_migrate(&cfg).await?;
        }
        Commands::Sessions { command } => match command {
            SessionCommands::List => run_sessions_list(&cfg).await?,
            SessionCommands::Show { session_id, recent } => {
                run_sessions_show(&cfg, session_id, recent).await?
            }
        },
    }

    Ok(())
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

async fn run_chat(runner: &Runner, cfg: &RuntimeConfig) -> Result<()> {
    println!("Interactive mode started. Type /exit to quit.");
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

        run_prompt_streaming(runner, cfg, input).await?;
    }

    Ok(())
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
            let model = GroqClient::new(GroqConfig::new(api_key, model_name.clone()))?;
            Ok((Arc::new(model), provider, model_name))
        }
        Provider::Ollama => {
            let host = std::env::var("OLLAMA_HOST")
                .unwrap_or_else(|_| "http://localhost:11434".to_string());
            let model_name = cfg.model.clone().unwrap_or_else(|| "llama3.2".to_string());
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
            provider: Provider::Auto,
            model: None,
            app_name: "test-app".to_string(),
            user_id: "test-user".to_string(),
            session_id: "test-session".to_string(),
            session_backend: SessionBackend::Memory,
            session_db_url: "sqlite://.zavora/test.db".to_string(),
        }
    }

    fn mock_model(text: &str) -> Arc<dyn Llm> {
        Arc::new(
            MockLlm::new("mock")
                .with_response(LlmResponse::new(Content::new("model").with_text(text))),
        )
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
}
