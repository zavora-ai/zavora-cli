//! Model-facing controls for the durable subagent supervisor.
//!
//! These tools use the same SQLite run graph and detached worker entry point as
//! the CLI. Mutating operations remain subject to the sealed tool policy and
//! confirmation wrapper; inspection and waiting are marked read-only.

use std::sync::Arc;
use std::time::Duration;

use adk_tool::{FunctionTool, Tool};
use anyhow::{Result, bail};
use serde_json::{Value, json};

use crate::agent_supervisor::{
    AGENT_RUN_SCHEMA_VERSION, AgentRunStore, LaunchOptions, NewAgentRun, SupervisorPolicy,
    cancel_run, launch_worker,
};
use crate::config::RuntimeConfig;

fn string_arg<'a>(args: &'a Value, key: &str) -> Result<&'a str> {
    args.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| anyhow::anyhow!("'{key}' is required"))
}

fn run_json(run: &crate::agent_supervisor::AgentRun) -> Value {
    json!({
        "schema_version": AGENT_RUN_SCHEMA_VERSION,
        "run": run,
    })
}

fn tool_result(result: Result<Value>) -> std::result::Result<Value, adk_rust::AdkError> {
    result.map_err(|error| adk_rust::AdkError::tool(error.to_string()))
}

pub fn build_tools(cfg: &RuntimeConfig) -> Vec<Arc<dyn Tool>> {
    let spawn_cfg = cfg.clone();
    let spawn = FunctionTool::new(
        "spawn_agent",
        "Start a named specialist as a durable background child run. The run persists across CLI sessions and can optionally use an isolated git worktree. Args: agent (required), task (required), worktree (optional boolean). Returns the run id and lifecycle state.",
        move |_ctx, args| {
            let cfg = spawn_cfg.clone();
            async move {
                tool_result(async {
                    let agent = string_arg(&args, "agent")?.to_string();
                    let task = string_arg(&args, "task")?.to_string();
                    if agent == "ralph"
                        || !cfg.available_agents.iter().any(|item| item.name == agent)
                    {
                        bail!("agent '{agent}' is not an available specialist");
                    }
                    if !crate::config::child_agent_allowed(&cfg, &agent) {
                        bail!(
                            "agent '{}' is denied by parent agent '{}' child policy",
                            agent,
                            cfg.agent_name
                        );
                    }
                    let parent_run_id = std::env::var("ZAVORA_SUPERVISOR_RUN_ID").ok();
                    let store = AgentRunStore::open_default().await?;
                    let run = store
                        .create_run(
                            NewAgentRun {
                                parent_run_id,
                                parent_session_id: cfg.session_id.clone(),
                                agent,
                                task,
                                workspace: std::env::current_dir()?,
                                worktree: args
                                    .get("worktree")
                                    .and_then(Value::as_bool)
                                    .unwrap_or(false),
                                retry_of: None,
                            },
                            &SupervisorPolicy::default(),
                        )
                        .await?;
                    launch_worker(
                        &store,
                        &run,
                        LaunchOptions {
                            cfg: &cfg,
                            always_approve: false,
                        },
                    )
                    .await?;
                    Ok(run_json(&store.get(&run.id).await?.unwrap_or(run)))
                }
                .await)
            }
        },
    )
    .with_concurrency_safe(true);

    let list = FunctionTool::new(
        "list_agent_runs",
        "List durable local subagent runs and their current lifecycle states. Args: limit (optional, 1-200).",
        |_ctx, args| async move {
            tool_result(async {
                let limit = args
                    .get("limit")
                    .and_then(Value::as_u64)
                    .unwrap_or(50)
                    .clamp(1, 200) as usize;
                let store = AgentRunStore::open_default().await?;
                Ok(json!({
                    "schema_version": AGENT_RUN_SCHEMA_VERSION,
                    "runs": store.list(limit).await?,
                }))
            }
            .await)
        },
    )
    .with_read_only(true)
    .with_concurrency_safe(true);

    let status = FunctionTool::new(
        "agent_run_status",
        "Inspect one durable subagent run, including its response, error, parent, session, workspace, and process metadata. Args: run_id (required).",
        |_ctx, args| async move {
            tool_result(async {
                let run_id = string_arg(&args, "run_id")?;
                let store = AgentRunStore::open_default().await?;
                let run = store
                    .get(run_id)
                    .await?
                    .ok_or_else(|| anyhow::anyhow!("agent run '{run_id}' not found"))?;
                Ok(json!({
                    "schema_version": AGENT_RUN_SCHEMA_VERSION,
                    "run": run,
                    "children": store.children(run_id).await?,
                    "events": store.events(run_id, 0).await?,
                }))
            }
            .await)
        },
    )
    .with_read_only(true)
    .with_concurrency_safe(true);

    let send_cfg = cfg.clone();
    let send = FunctionTool::new(
        "send_agent_message",
        "Queue a follow-up for a durable subagent. Active agents consume it on their next turn; completed or input-blocked agents resume with the same session. Args: run_id and message (required).",
        move |_ctx, args| {
            let cfg = send_cfg.clone();
            async move {
                tool_result(async {
                    let run_id = string_arg(&args, "run_id")?;
                    let message = string_arg(&args, "message")?;
                    let store = AgentRunStore::open_default().await?;
                    let continuation = store.prepare_continuation(run_id, message).await?;
                    if continuation.launch_required {
                        launch_worker(
                            &store,
                            &continuation.run,
                            LaunchOptions {
                                cfg: &cfg,
                                always_approve: false,
                            },
                        )
                        .await?;
                    }
                    let run = store.get(run_id).await?.unwrap_or(continuation.run);
                    Ok(run_json(&run))
                }
                .await)
            }
        },
    )
    .with_concurrency_safe(true);

    let wait = FunctionTool::new(
        "wait_agent",
        "Wait for a durable subagent to reach a terminal state and return its response or error. Args: run_id (required), timeout_secs (optional, 1-3600).",
        |_ctx, args| async move {
            tool_result(async {
                let run_id = string_arg(&args, "run_id")?;
                let timeout = args
                    .get("timeout_secs")
                    .and_then(Value::as_u64)
                    .unwrap_or(300)
                    .clamp(1, 3600);
                let store = AgentRunStore::open_default().await?;
                let run = store
                    .wait_terminal(run_id, Duration::from_secs(timeout))
                    .await?;
                Ok(run_json(&run))
            }
            .await)
        },
    )
    .with_read_only(true)
    .with_concurrency_safe(true);

    let cancel = FunctionTool::new(
        "cancel_agent",
        "Cancel a queued or running durable subagent by exact run id. Args: run_id (required).",
        |_ctx, args| async move {
            tool_result(
                async {
                    let run_id = string_arg(&args, "run_id")?;
                    let store = AgentRunStore::open_default().await?;
                    Ok(run_json(&cancel_run(&store, run_id).await?))
                }
                .await,
            )
        },
    )
    .with_concurrency_safe(true);

    let events = FunctionTool::new(
        "agent_run_events",
        "Read ordered lifecycle events for one durable subagent. Args: run_id (required), after (optional sequence cursor).",
        |_ctx, args| async move {
            tool_result(async {
                let run_id = string_arg(&args, "run_id")?;
                let after = args.get("after").and_then(Value::as_i64).unwrap_or(0);
                let store = AgentRunStore::open_default().await?;
                if store.get(run_id).await?.is_none() {
                    bail!("agent run '{run_id}' not found");
                }
                Ok(json!({
                    "schema_version": AGENT_RUN_SCHEMA_VERSION,
                    "events": store.events(run_id, after).await?,
                }))
            }
            .await)
        },
    )
    .with_read_only(true)
    .with_concurrency_safe(true);

    vec![
        Arc::new(spawn),
        Arc::new(list),
        Arc::new(status),
        Arc::new(send),
        Arc::new(wait),
        Arc::new(cancel),
        Arc::new(events),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn supervisor_surface_has_frontier_lifecycle_controls() {
        let cfg = crate::test_support::base_cfg();
        let names = build_tools(&cfg)
            .into_iter()
            .map(|tool| tool.name().to_string())
            .collect::<Vec<_>>();
        assert!(names.contains(&"spawn_agent".to_string()));
        assert!(names.contains(&"send_agent_message".to_string()));
        assert!(names.contains(&"wait_agent".to_string()));
        assert!(names.contains(&"cancel_agent".to_string()));
    }
}
