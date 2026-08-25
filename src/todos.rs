/// Todo list persistence and delegate sub-agent experiments.
///
/// Todos are file-backed task lists stored in `.zavora/todos/`. The delegate
/// mode runs an isolated prompt in a separate session and returns the result.
/// Delegate is experimental and gated behind a flag.
use anyhow::{Context as _, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

// ---------------------------------------------------------------------------
// Todo data model
// ---------------------------------------------------------------------------

/// A single task in a todo list.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Task {
    pub description: String,
    pub completed: bool,
}

/// A persistent todo list with tasks, context, and file tracking.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TodoList {
    pub id: String,
    pub description: String,
    pub tasks: Vec<Task>,
    #[serde(default)]
    pub context: Vec<String>,
    #[serde(default)]
    pub modified_files: Vec<String>,
}

impl TodoList {
    /// Create a new todo list with the given description and tasks.
    pub fn new(id: &str, description: &str, tasks: Vec<String>) -> Self {
        Self {
            id: id.to_string(),
            description: description.to_string(),
            tasks: tasks
                .into_iter()
                .map(|d| Task {
                    description: d,
                    completed: false,
                })
                .collect(),
            context: Vec::new(),
            modified_files: Vec::new(),
        }
    }

    /// Mark a task as completed by index.
    pub fn complete_task(&mut self, index: usize) -> bool {
        if let Some(task) = self.tasks.get_mut(index) {
            task.completed = true;
            true
        } else {
            false
        }
    }

    /// Number of completed tasks.
    pub fn completed_count(&self) -> usize {
        self.tasks.iter().filter(|t| t.completed).count()
    }

    /// Whether all tasks are done.
    pub fn is_finished(&self) -> bool {
        !self.tasks.is_empty() && self.tasks.iter().all(|t| t.completed)
    }

    /// Format the todo list for display.
    pub fn format_display(&self) -> String {
        let mut out = format!(
            "TODO: {} ({}/{})\n",
            self.description,
            self.completed_count(),
            self.tasks.len()
        );
        for (i, task) in self.tasks.iter().enumerate() {
            let mark = if task.completed { "✓" } else { " " };
            out.push_str(&format!("  [{}] {}: {}\n", mark, i, task.description));
        }
        out
    }
}

// ---------------------------------------------------------------------------
// File persistence
// ---------------------------------------------------------------------------

/// Directory for todo list storage.
pub fn todos_dir(workspace: &Path) -> PathBuf {
    workspace.join(".zavora").join("todos")
}

/// Save a todo list to disk.
pub fn save_todo(workspace: &Path, todo: &TodoList) -> Result<()> {
    let dir = todos_dir(workspace);
    std::fs::create_dir_all(&dir).context("failed to create todos directory")?;
    let path = dir.join(format!("{}.json", todo.id));
    let json = serde_json::to_string_pretty(todo).context("failed to serialize todo")?;
    std::fs::write(&path, json).context("failed to write todo file")?;
    Ok(())
}

/// Load a todo list from disk by ID.
pub fn load_todo(workspace: &Path, id: &str) -> Result<TodoList> {
    let path = todos_dir(workspace).join(format!("{id}.json"));
    let json =
        std::fs::read_to_string(&path).with_context(|| format!("failed to read todo '{id}'"))?;
    serde_json::from_str(&json).with_context(|| format!("failed to parse todo '{id}'"))
}

/// List all todo list IDs from disk.
pub fn list_todo_ids(workspace: &Path) -> Result<Vec<String>> {
    let dir = todos_dir(workspace);
    if !dir.exists() {
        return Ok(Vec::new());
    }
    let mut ids = Vec::new();
    for entry in std::fs::read_dir(&dir).context("failed to read todos directory")? {
        let entry = entry?;
        if let Some(name) = entry.path().file_stem() {
            ids.push(name.to_string_lossy().to_string());
        }
    }
    ids.sort();
    Ok(ids)
}

/// Delete a todo list from disk.
pub fn delete_todo(workspace: &Path, id: &str) -> Result<()> {
    let path = todos_dir(workspace).join(format!("{id}.json"));
    if path.exists() {
        std::fs::remove_file(&path).with_context(|| format!("failed to delete todo '{id}'"))?;
    }
    Ok(())
}

/// Delete all finished todo lists.
pub fn clear_finished_todos(workspace: &Path) -> Result<usize> {
    let ids = list_todo_ids(workspace)?;
    let mut cleared = 0;
    for id in ids {
        if let Ok(todo) = load_todo(workspace, &id)
            && todo.is_finished()
        {
            delete_todo(workspace, &id)?;
            cleared += 1;
        }
    }
    Ok(cleared)
}

/// Format a summary of all todo lists for display.
pub fn format_todos_summary(workspace: &Path) -> Result<String> {
    let ids = list_todo_ids(workspace)?;
    if ids.is_empty() {
        return Ok("No todo lists. The agent can create them during task execution.".to_string());
    }
    let mut out = String::from("Todo lists:\n");
    for id in &ids {
        if let Ok(todo) = load_todo(workspace, id) {
            let status = if todo.is_finished() { "✓" } else { "…" };
            out.push_str(&format!(
                "  [{status}] {id}: {} ({}/{})\n",
                todo.description,
                todo.completed_count(),
                todo.tasks.len()
            ));
        }
    }
    out.push_str("\nUse /todos view <id> | delete <id> | clear-finished");
    Ok(out)
}

// ---------------------------------------------------------------------------
// Delegate (experimental)
// ---------------------------------------------------------------------------

use adk_session::SessionService;
use std::sync::Arc;

use crate::config::RuntimeConfig;
use crate::runner::{ResolvedRuntimeTools, ToolConfirmationSettings, build_single_runner_for_chat};
use crate::streaming::run_prompt;
use crate::telemetry::TelemetrySink;

pub struct DelegateContext<'a> {
    pub cfg: &'a RuntimeConfig,
    pub session_service: Arc<dyn SessionService>,
    pub runtime_tools: &'a ResolvedRuntimeTools,
    pub tool_confirmation: &'a ToolConfirmationSettings,
    pub telemetry: &'a TelemetrySink,
}

/// Parse `/delegate` arguments. Supports `@agent task`, `--agent NAME task`,
/// and an unqualified task for the default isolated agent.
pub fn parse_delegate_request(input: &str) -> Result<(Option<String>, String)> {
    let mut parts = shlex::split(input).ok_or_else(|| anyhow::anyhow!("invalid quoting"))?;
    if parts.is_empty() {
        return Ok((None, String::new()));
    }
    let agent = if let Some(name) = parts[0].strip_prefix('@') {
        let name = name.trim().to_string();
        parts.remove(0);
        (!name.is_empty()).then_some(name)
    } else if parts[0] == "--agent" {
        if parts.len() < 2 {
            return Err(anyhow::anyhow!("--agent requires a name"));
        }
        parts.remove(0);
        Some(parts.remove(0))
    } else if let Some(name) = parts[0].strip_prefix("--agent=") {
        let name = name.to_string();
        parts.remove(0);
        Some(name)
    } else {
        None
    };
    Ok((agent, parts.join(" ")))
}

/// Result of a delegate run.
#[derive(Debug, Clone)]
pub struct DelegateResult {
    pub task: String,
    pub agent: String,
    pub session_id: String,
    pub output: String,
    pub success: bool,
    pub duration_ms: u128,
}

impl DelegateResult {
    /// Format the delegate result for display.
    pub fn format_display(&self) -> String {
        let status = if self.success { "✓" } else { "✗" };
        format!(
            "[{status}] {} · {} ms · session {}\nTask: {}\n{}",
            self.agent, self.duration_ms, self.session_id, self.task, self.output
        )
    }
}

/// Run a delegate task in an isolated session.
pub async fn run_delegate(
    task: &str,
    cfg: &RuntimeConfig,
    session_service: Arc<dyn SessionService>,
    runtime_tools: &ResolvedRuntimeTools,
    tool_confirmation: &ToolConfirmationSettings,
    telemetry: &TelemetrySink,
) -> DelegateResult {
    fork_sub_agent(
        task,
        None,
        None,
        DelegateContext {
            cfg,
            session_service,
            runtime_tools,
            tool_confirmation,
            telemetry,
        },
    )
    .await
}

/// Run a task with a named catalog agent in an isolated, retained session.
pub async fn run_named_delegate(
    task: &str,
    agent_name: Option<&str>,
    cfg: &RuntimeConfig,
    session_service: Arc<dyn SessionService>,
    runtime_tools: &ResolvedRuntimeTools,
    tool_confirmation: &ToolConfirmationSettings,
    telemetry: &TelemetrySink,
) -> DelegateResult {
    fork_sub_agent(
        task,
        agent_name,
        None,
        DelegateContext {
            cfg,
            session_service,
            runtime_tools,
            tool_confirmation,
            telemetry,
        },
    )
    .await
}

/// Fork a sub-agent with a fresh session, optional file context, and timeout.
///
/// The sub-agent gets a clean message history (no parent context pollution).
/// The session is always cleaned up on completion or error.
pub async fn fork_sub_agent(
    task: &str,
    agent_name: Option<&str>,
    file_context: Option<&str>,
    context: DelegateContext<'_>,
) -> DelegateResult {
    let DelegateContext {
        cfg,
        session_service,
        runtime_tools,
        tool_confirmation,
        telemetry,
    } = context;
    let started = std::time::Instant::now();
    let requested_agent = agent_name.unwrap_or("default");
    let delegate_session_id = format!(
        "fork-{}-{}-{}",
        cfg.session_id,
        requested_agent.replace(|character: char| !character.is_ascii_alphanumeric(), "-"),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis()
    );

    let mut delegate_cfg = cfg.clone();
    delegate_cfg.session_id = delegate_session_id.clone();
    if let Some(name) = agent_name {
        let Some(agent) = cfg.available_agents.iter().find(|agent| agent.name == name) else {
            return DelegateResult {
                task: task.to_string(),
                agent: name.to_string(),
                session_id: delegate_session_id,
                output: format!("Agent '{name}' is not registered"),
                success: false,
                duration_ms: started.elapsed().as_millis(),
            };
        };
        crate::config::apply_agent_overrides(&mut delegate_cfg, agent);
    }
    let scoped_tools = runtime_tools.restricted(
        &delegate_cfg.agent_allow_tools,
        &delegate_cfg.agent_deny_tools,
    );
    let scoped_confirmation = if agent_name.is_some() {
        crate::runner::resolve_tool_confirmation_settings(&delegate_cfg, &scoped_tools)
    } else {
        tool_confirmation.clone()
    };
    let agent = delegate_cfg.agent_name.clone();
    telemetry.emit(
        "subagent.started",
        serde_json::json!({
            "agent": agent,
            "parent_session_id": cfg.session_id,
            "child_session_id": delegate_session_id,
        }),
    );

    // Build prompt with optional file context
    let prompt = match file_context {
        Some(ctx) => format!("{}\n\n<context>\n{}\n</context>", task, ctx),
        None => task.to_string(),
    };

    let timeout =
        std::time::Duration::from_secs(delegate_cfg.agent_timeout_secs.unwrap_or(300).max(1));

    let result = match build_single_runner_for_chat(
        &delegate_cfg,
        session_service.clone(),
        &scoped_tools,
        &scoped_confirmation,
        telemetry,
    )
    .await
    {
        Ok((runner, _, _)) => {
            match tokio::time::timeout(
                timeout,
                run_prompt(&runner, &delegate_cfg, &prompt, telemetry),
            )
            .await
            {
                Ok(Ok(output)) => DelegateResult {
                    task: task.to_string(),
                    agent: agent.clone(),
                    session_id: delegate_session_id.clone(),
                    output,
                    success: true,
                    duration_ms: started.elapsed().as_millis(),
                },
                Ok(Err(e)) => DelegateResult {
                    task: task.to_string(),
                    agent: agent.clone(),
                    session_id: delegate_session_id.clone(),
                    output: format!("Error: {e}"),
                    success: false,
                    duration_ms: started.elapsed().as_millis(),
                },
                Err(_) => DelegateResult {
                    task: task.to_string(),
                    agent: agent.clone(),
                    session_id: delegate_session_id.clone(),
                    output: format!("Sub-agent timed out after {} seconds", timeout.as_secs()),
                    success: false,
                    duration_ms: started.elapsed().as_millis(),
                },
            }
        }
        Err(e) => DelegateResult {
            task: task.to_string(),
            agent: agent.clone(),
            session_id: delegate_session_id.clone(),
            output: format!("Failed to build sub-agent: {e}"),
            success: false,
            duration_ms: started.elapsed().as_millis(),
        },
    };

    telemetry.emit(
        if result.success {
            "subagent.completed"
        } else {
            "subagent.failed"
        },
        serde_json::json!({
            "agent": result.agent,
            "parent_session_id": cfg.session_id,
            "child_session_id": result.session_id,
            "duration_ms": result.duration_ms,
        }),
    );

    result
}
