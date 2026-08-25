//! Governed direct and parallel execution for catalog agents.
//!
//! Every child receives an isolated retained session, a tool surface that can
//! only be narrower than the sealed parent surface, its own model/skill policy,
//! bounded execution, and correlated telemetry.

use std::sync::Arc;
use std::time::{Duration, Instant};

use adk_rust::futures::{StreamExt, stream};
use adk_session::SessionService;
use serde::Serialize;
use serde_json::json;

use crate::config::{ResolvedAgent, RuntimeConfig, apply_agent_overrides};
use crate::guardrail::apply_guardrail;
use crate::provider::resolve_model;
use crate::retrieval::{RetrievalPolicy, RetrievalService, augment_prompt_with_retrieval};
use crate::runner::{
    ResolvedRuntimeTools, build_single_runner_for_chat, resolve_tool_confirmation_settings,
};
use crate::session::build_session_service;
use crate::streaming::run_prompt;
use crate::telemetry::{TelemetrySink, unix_ms_now};

pub const SUBAGENT_SCHEMA_VERSION: &str = "zavora.subagents.v1";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParallelRequest {
    pub agents: Vec<String>,
    pub max_concurrency: usize,
    pub task: String,
}

#[derive(Debug, Clone)]
pub enum ParallelProgressEvent {
    Started {
        index: usize,
        agent: String,
        session_id: String,
    },
    Finished(SubagentRunResult),
}

pub type ParallelProgress = Arc<dyn Fn(ParallelProgressEvent) + Send + Sync>;

pub struct ParallelRunContext<'a> {
    pub base_cfg: &'a RuntimeConfig,
    pub parent_tools: &'a ResolvedRuntimeTools,
    pub retrieval: Arc<dyn RetrievalService>,
    pub telemetry: &'a TelemetrySink,
    pub max_concurrency: usize,
    pub progress_format: crate::cli::OutputFormat,
    pub progress: Option<ParallelProgress>,
    /// Reuse an interactive session service so memory-backed child sessions
    /// remain resumable for the lifetime of the parent process. Headless
    /// callers may omit it; durable cross-process resume then requires SQLite.
    pub session_service: Option<Arc<dyn SessionService>>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SubagentRunResult {
    pub index: usize,
    pub agent: String,
    pub session_id: String,
    pub success: bool,
    pub response: String,
    pub error: Option<String>,
    pub duration_ms: u128,
    pub provider: String,
    pub model: String,
}

struct RunOneContext {
    base_cfg: RuntimeConfig,
    parent_tools: ResolvedRuntimeTools,
    retrieval: Arc<dyn RetrievalService>,
    telemetry: TelemetrySink,
    progress: Option<ParallelProgress>,
    session_service: Option<Arc<dyn SessionService>>,
}

/// Parse interactive parallel syntax such as
/// `@developer @reviewer --max-concurrency 2 inspect this change`.
pub fn parse_parallel_request(input: &str) -> anyhow::Result<ParallelRequest> {
    let parts = shlex::split(input).ok_or_else(|| anyhow::anyhow!("invalid quoting"))?;
    let mut agents = Vec::new();
    let mut max_concurrency = 4usize;
    let mut cursor = 0usize;
    while cursor < parts.len() {
        let part = &parts[cursor];
        if part == "--" {
            cursor += 1;
            break;
        }
        if let Some(agent) = part.strip_prefix('@') {
            if agent.is_empty() {
                return Err(anyhow::anyhow!("agent name after '@' cannot be empty"));
            }
            agents.push(agent.to_string());
            cursor += 1;
            continue;
        }
        if part == "--agent" {
            let Some(agent) = parts.get(cursor + 1) else {
                return Err(anyhow::anyhow!("--agent requires a name"));
            };
            agents.push(agent.clone());
            cursor += 2;
            continue;
        }
        if let Some(agent) = part.strip_prefix("--agent=") {
            if agent.is_empty() {
                return Err(anyhow::anyhow!("--agent requires a name"));
            }
            agents.push(agent.to_string());
            cursor += 1;
            continue;
        }
        if part == "--max-concurrency" {
            let Some(value) = parts.get(cursor + 1) else {
                return Err(anyhow::anyhow!("--max-concurrency requires a number"));
            };
            max_concurrency = value
                .parse::<usize>()
                .map_err(|_| anyhow::anyhow!("invalid concurrency '{value}'"))?;
            cursor += 2;
            continue;
        }
        if let Some(value) = part.strip_prefix("--max-concurrency=") {
            max_concurrency = value
                .parse::<usize>()
                .map_err(|_| anyhow::anyhow!("invalid concurrency '{value}'"))?;
            cursor += 1;
            continue;
        }
        break;
    }
    if agents.is_empty() {
        return Err(anyhow::anyhow!(
            "select at least one agent with @NAME or --agent NAME"
        ));
    }
    if max_concurrency == 0 {
        return Err(anyhow::anyhow!("max concurrency must be at least 1"));
    }
    let task = parts[cursor..].join(" ");
    if task.trim().is_empty() {
        return Err(anyhow::anyhow!("parallel task cannot be empty"));
    }
    Ok(ParallelRequest {
        agents,
        max_concurrency,
        task,
    })
}

/// Resolve names against the live catalog while preserving request order and
/// removing duplicates.
pub fn resolve_parallel_agents(
    cfg: &RuntimeConfig,
    names: &[String],
) -> anyhow::Result<Vec<ResolvedAgent>> {
    let mut seen = std::collections::BTreeSet::new();
    names
        .iter()
        .filter(|name| seen.insert((*name).clone()))
        .map(|name| {
            if name == "ralph" {
                return Err(anyhow::anyhow!(
                    "'ralph' is a pipeline, not a subagent; use /ralph"
                ));
            }
            if !crate::config::child_agent_allowed(cfg, name) {
                return Err(anyhow::anyhow!(
                    "agent '{name}' is denied by parent agent '{}' child policy",
                    cfg.agent_name
                ));
            }
            cfg.available_agents
                .iter()
                .find(|agent| agent.name == *name)
                .cloned()
                .ok_or_else(|| {
                    anyhow::anyhow!("agent '{name}' not found; use /agents to inspect the registry")
                })
        })
        .collect()
}

fn child_session_id(parent: &str, agent: &str, index: usize) -> String {
    let safe_agent = agent
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '-' | '_') {
                character
            } else {
                '-'
            }
        })
        .collect::<String>();
    format!("subagent-{parent}-{safe_agent}-{}-{index}", unix_ms_now())
}

async fn run_one(
    index: usize,
    agent: ResolvedAgent,
    prompt: String,
    context: RunOneContext,
) -> SubagentRunResult {
    let RunOneContext {
        base_cfg,
        parent_tools,
        retrieval,
        telemetry,
        progress,
        session_service,
    } = context;
    let started = Instant::now();
    let mut cfg = base_cfg;
    apply_agent_overrides(&mut cfg, &agent);
    cfg.session_id = child_session_id(&cfg.session_id, &agent.name, index);
    let session_id = cfg.session_id.clone();
    if let Some(progress) = &progress {
        progress(ParallelProgressEvent::Started {
            index,
            agent: agent.name.clone(),
            session_id: session_id.clone(),
        });
    }
    let fallback_provider = format!("{:?}", cfg.provider).to_ascii_lowercase();
    let fallback_model = cfg.worker_model.clone();
    telemetry.emit(
        "subagent.started",
        json!({
            "agent": agent.name,
            "child_session_id": session_id,
            "dispatch_index": index,
        }),
    );

    let execution = async {
        let input = apply_guardrail(&cfg, &telemetry, "input", cfg.guardrail_input_mode, &prompt)?;
        let policy = RetrievalPolicy {
            max_chunks: cfg.retrieval_max_chunks,
            max_chars: cfg.retrieval_max_chars,
            min_score: cfg.retrieval_min_score,
        };
        let input = augment_prompt_with_retrieval(retrieval.as_ref(), &input, policy)?;
        let (_, provider, model_name) = resolve_model(&cfg)?;
        let scoped_tools = parent_tools.restricted(&cfg.agent_allow_tools, &cfg.agent_deny_tools);
        let confirmation = resolve_tool_confirmation_settings(&cfg, &scoped_tools);
        let sessions = match session_service {
            Some(sessions) => sessions,
            None => build_session_service(&cfg).await?,
        };
        let (runner, _, _) =
            build_single_runner_for_chat(&cfg, sessions, &scoped_tools, &confirmation, &telemetry)
                .await?;
        let response = run_prompt(&runner, &cfg, &input, &telemetry).await?;
        let response = apply_guardrail(
            &cfg,
            &telemetry,
            "output",
            cfg.guardrail_output_mode,
            &response,
        )?;
        Ok::<_, anyhow::Error>((
            response,
            format!("{provider:?}").to_ascii_lowercase(),
            model_name,
        ))
    };
    let timeout = Duration::from_secs(cfg.agent_timeout_secs.unwrap_or(300).max(1));
    let result = tokio::time::timeout(timeout, execution).await;
    let (success, response, error, provider, model) = match result {
        Ok(Ok((response, provider, model))) => (true, response, None, provider, model),
        Ok(Err(error)) => (
            false,
            String::new(),
            Some(error.to_string()),
            fallback_provider,
            fallback_model,
        ),
        Err(_) => (
            false,
            String::new(),
            Some(format!("timed out after {} seconds", timeout.as_secs())),
            fallback_provider,
            fallback_model,
        ),
    };
    let result = SubagentRunResult {
        index,
        agent: agent.name,
        session_id,
        success,
        response,
        error,
        duration_ms: started.elapsed().as_millis(),
        provider,
        model,
    };
    telemetry.emit(
        if result.success {
            "subagent.completed"
        } else {
            "subagent.failed"
        },
        json!({
            "agent": result.agent,
            "child_session_id": result.session_id,
            "dispatch_index": index,
            "duration_ms": result.duration_ms,
            "error": result.error,
        }),
    );
    if let Some(progress) = progress {
        progress(ParallelProgressEvent::Finished(result.clone()));
    }
    result
}

pub async fn run_parallel(
    agents: Vec<ResolvedAgent>,
    prompt: &str,
    context: ParallelRunContext<'_>,
) -> Vec<SubagentRunResult> {
    let ParallelRunContext {
        base_cfg,
        parent_tools,
        retrieval,
        telemetry,
        max_concurrency,
        progress_format,
        progress,
        session_service,
    } = context;
    let total_agents = agents.len();
    let tasks = agents.into_iter().enumerate().map(|(index, agent)| {
        let cfg = base_cfg.clone();
        let prompt = prompt.to_string();
        let tools = parent_tools.clone();
        let retrieval = retrieval.clone();
        let telemetry = telemetry.clone();
        let progress = progress.clone();
        let session_service = session_service.clone();
        async move {
            run_one(
                index,
                agent,
                prompt,
                RunOneContext {
                    base_cfg: cfg,
                    parent_tools: tools,
                    retrieval,
                    telemetry,
                    progress,
                    session_service,
                },
            )
            .await
        }
    });
    if progress_format == crate::cli::OutputFormat::StreamJson {
        println!(
            "{}",
            json!({
                "schema_version": SUBAGENT_SCHEMA_VERSION,
                "type": "init",
                "agent_count": tasks.len(),
                "max_concurrency": max_concurrency.max(1),
            })
        );
    }
    let mut running = stream::iter(tasks).buffer_unordered(max_concurrency.max(1));
    let mut results = Vec::new();
    let mut completion_sequence = 0usize;
    while let Some(result) = running.next().await {
        completion_sequence += 1;
        match progress_format {
            crate::cli::OutputFormat::Text => eprintln!(
                "[subagent {}/{}] {} {} ({} ms)",
                completion_sequence,
                total_agents,
                result.agent,
                if result.success {
                    "completed"
                } else {
                    "failed"
                },
                result.duration_ms,
            ),
            crate::cli::OutputFormat::StreamJson => println!(
                "{}",
                json!({
                    "schema_version": SUBAGENT_SCHEMA_VERSION,
                    "type": "agent_result",
                    "sequence": completion_sequence,
                    "result": result,
                })
            ),
            crate::cli::OutputFormat::Json => {}
        }
        results.push(result);
    }
    results.sort_by_key(|result| result.index);
    results
}

pub fn format_result_markdown(result: &SubagentRunResult) -> String {
    let status = if result.success {
        "completed"
    } else {
        "failed"
    };
    format!(
        "## Agent `{}` {status}\n\n- Session: `{}`\n- Route: `{}/{}`\n- Duration: {} ms\n\n{}",
        result.agent,
        result.session_id,
        result.provider,
        result.model,
        result.duration_ms,
        if result.success {
            result.response.as_str()
        } else {
            result.error.as_deref().unwrap_or("Subagent failed")
        }
    )
}

pub fn render_results(results: &[SubagentRunResult], format: crate::cli::OutputFormat) {
    match format {
        crate::cli::OutputFormat::Text => {
            for result in results {
                println!("{}", format_result_markdown(result));
            }
        }
        crate::cli::OutputFormat::Json => println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "schema_version": SUBAGENT_SCHEMA_VERSION,
                "type": "parallel_result",
                "success": results.iter().all(|result| result.success),
                "results": results,
            }))
            .unwrap_or_else(|_| "{}".to_string())
        ),
        crate::cli::OutputFormat::StreamJson => {
            println!(
                "{}",
                json!({
                    "schema_version": SUBAGENT_SCHEMA_VERSION,
                    "type": "result",
                    "sequence": results.len() + 1,
                    "success": results.iter().all(|result| result.success),
                    "agent_count": results.len(),
                })
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::parse_parallel_request;

    #[test]
    fn interactive_parser_accepts_frontier_and_cli_style_agent_selectors() {
        let request = parse_parallel_request(
            "@developer_agent --agent reviewer_agent --max-concurrency 2 'review this change'",
        )
        .unwrap();
        assert_eq!(request.agents, vec!["developer_agent", "reviewer_agent"]);
        assert_eq!(request.max_concurrency, 2);
        assert_eq!(request.task, "review this change");
    }

    #[test]
    fn interactive_parser_requires_agents_task_and_positive_concurrency() {
        assert!(parse_parallel_request("review this").is_err());
        assert!(parse_parallel_request("@reviewer_agent").is_err());
        assert!(parse_parallel_request("@reviewer_agent --max-concurrency 0 review this").is_err());
    }
}
