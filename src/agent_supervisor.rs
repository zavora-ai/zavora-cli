//! Durable supervision for local subagent runs.
//!
//! The supervisor is deliberately separate from ADK session storage. ADK
//! sessions own model conversation history; this store owns operational state:
//! parent/child relationships, lifecycle, process identity, queued follow-ups,
//! worktree ownership, and an inspectable event stream.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::sqlite::SqlitePoolOptions;
use sqlx::{Row, SqlitePool};

use crate::config::RuntimeConfig;
use crate::telemetry::unix_ms_now;

pub const AGENT_RUN_SCHEMA_VERSION: &str = "zavora.agent-runs.v1";
static NEXT_RUN_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentRunStatus {
    Queued,
    Running,
    NeedsInput,
    Completed,
    Failed,
    CancelRequested,
    Cancelled,
}

impl AgentRunStatus {
    pub fn label(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Running => "running",
            Self::NeedsInput => "needs_input",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::CancelRequested => "cancel_requested",
            Self::Cancelled => "cancelled",
        }
    }

    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Failed | Self::Cancelled)
    }

    fn parse(value: &str) -> Result<Self> {
        match value {
            "queued" => Ok(Self::Queued),
            "running" => Ok(Self::Running),
            "needs_input" => Ok(Self::NeedsInput),
            "completed" => Ok(Self::Completed),
            "failed" => Ok(Self::Failed),
            "cancel_requested" => Ok(Self::CancelRequested),
            "cancelled" => Ok(Self::Cancelled),
            other => bail!("unknown agent run status '{other}'"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentRun {
    pub id: String,
    pub parent_run_id: Option<String>,
    pub parent_session_id: String,
    pub session_id: String,
    pub agent: String,
    pub task: String,
    pub status: AgentRunStatus,
    pub depth: u32,
    pub workspace: PathBuf,
    pub worktree_path: Option<PathBuf>,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub pid: Option<u32>,
    pub log_path: PathBuf,
    pub response: Option<String>,
    pub error: Option<String>,
    pub retry_of: Option<String>,
    pub created_at_ms: i64,
    pub started_at_ms: Option<i64>,
    pub completed_at_ms: Option<i64>,
    pub updated_at_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentRunEvent {
    pub sequence: i64,
    pub run_id: String,
    pub timestamp_ms: i64,
    pub kind: String,
    pub payload: Value,
}

#[derive(Debug, Clone)]
pub struct NewAgentRun {
    pub parent_run_id: Option<String>,
    pub parent_session_id: String,
    pub agent: String,
    pub task: String,
    pub workspace: PathBuf,
    pub worktree: bool,
    pub retry_of: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpawnRequest {
    pub agent: String,
    pub task: String,
    pub worktree: bool,
}

pub fn parse_spawn_request(input: &str) -> Result<SpawnRequest> {
    let mut parts = shlex::split(input).ok_or_else(|| anyhow::anyhow!("invalid quoting"))?;
    let worktree = if let Some(index) = parts.iter().position(|part| part == "--worktree") {
        parts.remove(index);
        true
    } else {
        false
    };
    let agent = parts
        .first()
        .map(|value| value.trim_start_matches('@').to_string())
        .filter(|value| !value.is_empty())
        .context("missing agent name")?;
    let task = parts.get(1..).unwrap_or_default().join(" ");
    if task.trim().is_empty() {
        bail!("missing task");
    }
    Ok(SpawnRequest {
        agent,
        task,
        worktree,
    })
}

pub fn format_runs_markdown(runs: &[AgentRun]) -> String {
    if runs.is_empty() {
        return "## Background agents\n\nNo durable agent runs found.".to_string();
    }
    let mut output = String::from(
        "## Background agents\n\n| Run | Status | Agent | Depth | Workspace |\n|---|---|---|---:|---|\n",
    );
    for run in runs {
        output.push_str(&format!(
            "| `{}` | {} | `{}` | {} | `{}` |\n",
            run.id,
            run.status.label(),
            run.agent,
            run.depth,
            run.workspace.display()
        ));
    }
    output.push_str("\nUse `/send RUN_ID MESSAGE` or `/cancel RUN_ID` to control a run.");
    output
}

#[derive(Debug, Clone)]
pub struct SupervisorPolicy {
    pub max_depth: u32,
    pub max_active_children: usize,
}

#[derive(Debug, Clone)]
pub struct Continuation {
    pub run: AgentRun,
    /// A terminal or input-blocked run needs a new detached worker. Messages
    /// sent while the worker is active are consumed by its next turn.
    pub launch_required: bool,
}

impl Default for SupervisorPolicy {
    fn default() -> Self {
        Self {
            max_depth: std::env::var("ZAVORA_AGENT_MAX_DEPTH")
                .ok()
                .and_then(|value| value.parse().ok())
                .unwrap_or(2),
            max_active_children: std::env::var("ZAVORA_AGENT_MAX_ACTIVE")
                .ok()
                .and_then(|value| value.parse().ok())
                .unwrap_or(4),
        }
    }
}

#[derive(Clone)]
pub struct AgentRunStore {
    pool: SqlitePool,
    url: String,
}

impl AgentRunStore {
    pub async fn open_default() -> Result<Self> {
        Self::open(&default_store_url()).await
    }

    pub async fn open(url: &str) -> Result<Self> {
        let url = absolute_sqlite_url(url)?;
        crate::session::ensure_parent_dir_for_sqlite_url(&url)?;
        let pool = SqlitePoolOptions::new()
            .max_connections(8)
            .acquire_timeout(Duration::from_secs(10))
            .connect(&url)
            .await
            .with_context(|| format!("failed to open agent supervisor database '{url}'"))?;
        let store = Self { pool, url };
        store.migrate().await?;
        Ok(store)
    }

    pub fn url(&self) -> &str {
        &self.url
    }

    async fn migrate(&self) -> Result<()> {
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS agent_runs (
                id TEXT PRIMARY KEY,
                parent_run_id TEXT,
                parent_session_id TEXT NOT NULL,
                session_id TEXT NOT NULL,
                agent TEXT NOT NULL,
                task TEXT NOT NULL,
                status TEXT NOT NULL,
                depth INTEGER NOT NULL,
                workspace TEXT NOT NULL,
                worktree_path TEXT,
                provider TEXT,
                model TEXT,
                pid INTEGER,
                log_path TEXT NOT NULL,
                response TEXT,
                error TEXT,
                retry_of TEXT,
                created_at_ms INTEGER NOT NULL,
                started_at_ms INTEGER,
                completed_at_ms INTEGER,
                updated_at_ms INTEGER NOT NULL
            )",
        )
        .execute(&self.pool)
        .await
        .context("failed to migrate agent_runs")?;
        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_agent_runs_parent ON agent_runs(parent_run_id, created_at_ms)",
        )
        .execute(&self.pool)
        .await?;
        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_agent_runs_status ON agent_runs(status, updated_at_ms)",
        )
        .execute(&self.pool)
        .await?;
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS agent_run_events (
                sequence INTEGER PRIMARY KEY AUTOINCREMENT,
                run_id TEXT NOT NULL,
                timestamp_ms INTEGER NOT NULL,
                kind TEXT NOT NULL,
                payload TEXT NOT NULL
            )",
        )
        .execute(&self.pool)
        .await
        .context("failed to migrate agent_run_events")?;
        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_agent_run_events_run ON agent_run_events(run_id, sequence)",
        )
        .execute(&self.pool)
        .await?;
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS agent_run_messages (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                run_id TEXT NOT NULL,
                created_at_ms INTEGER NOT NULL,
                role TEXT NOT NULL,
                content TEXT NOT NULL,
                consumed_at_ms INTEGER
            )",
        )
        .execute(&self.pool)
        .await
        .context("failed to migrate agent_run_messages")?;
        Ok(())
    }

    pub async fn create_run(
        &self,
        request: NewAgentRun,
        policy: &SupervisorPolicy,
    ) -> Result<AgentRun> {
        let parent = match request.parent_run_id.as_deref() {
            Some(parent_id) => Some(
                self.get(parent_id)
                    .await?
                    .ok_or_else(|| anyhow::anyhow!("parent agent run '{parent_id}' not found"))?,
            ),
            None => None,
        };
        let depth = parent.as_ref().map_or(0, |run| run.depth + 1);
        if depth > policy.max_depth {
            bail!(
                "agent depth {depth} exceeds configured maximum {}",
                policy.max_depth
            );
        }
        let active = self.active_count(request.parent_run_id.as_deref()).await?;
        if active >= policy.max_active_children {
            bail!(
                "active child limit reached ({active}/{})",
                policy.max_active_children
            );
        }

        let now = now_i64();
        let id = new_run_id();
        let session_id = format!("agent-session-{id}");
        let log_path = request
            .workspace
            .join(".zavora/agent-runs")
            .join(format!("{id}.jsonl"));
        let mut run = AgentRun {
            id,
            parent_run_id: request.parent_run_id,
            parent_session_id: request.parent_session_id,
            session_id,
            agent: request.agent,
            task: request.task,
            status: AgentRunStatus::Queued,
            depth,
            workspace: request.workspace,
            worktree_path: None,
            provider: None,
            model: None,
            pid: None,
            log_path,
            response: None,
            error: None,
            retry_of: request.retry_of,
            created_at_ms: now,
            started_at_ms: None,
            completed_at_ms: None,
            updated_at_ms: now,
        };

        if request.worktree {
            let path = provision_worktree(&run.workspace, &run.id)?;
            run.workspace = path.clone();
            run.worktree_path = Some(path);
        }

        sqlx::query(
            "INSERT INTO agent_runs (
                id, parent_run_id, parent_session_id, session_id, agent, task,
                status, depth, workspace, worktree_path, provider, model, pid,
                log_path, response, error, retry_of, created_at_ms,
                started_at_ms, completed_at_ms, updated_at_ms
             ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&run.id)
        .bind(&run.parent_run_id)
        .bind(&run.parent_session_id)
        .bind(&run.session_id)
        .bind(&run.agent)
        .bind(&run.task)
        .bind(run.status.label())
        .bind(i64::from(run.depth))
        .bind(path_string(&run.workspace))
        .bind(run.worktree_path.as_deref().map(path_string))
        .bind(&run.provider)
        .bind(&run.model)
        .bind(run.pid.map(i64::from))
        .bind(path_string(&run.log_path))
        .bind(&run.response)
        .bind(&run.error)
        .bind(&run.retry_of)
        .bind(run.created_at_ms)
        .bind(run.started_at_ms)
        .bind(run.completed_at_ms)
        .bind(run.updated_at_ms)
        .execute(&self.pool)
        .await
        .context("failed to create agent run")?;
        self.enqueue_message(&run.id, "user", &run.task).await?;
        self.append_event(
            &run.id,
            "queued",
            json!({
                "agent": run.agent,
                "parent_run_id": run.parent_run_id,
                "parent_session_id": run.parent_session_id,
                "session_id": run.session_id,
                "depth": run.depth,
                "workspace": run.workspace,
                "worktree_path": run.worktree_path,
            }),
        )
        .await?;
        Ok(run)
    }

    pub async fn get(&self, id: &str) -> Result<Option<AgentRun>> {
        let row = sqlx::query("SELECT * FROM agent_runs WHERE id = ?")
            .bind(id)
            .fetch_optional(&self.pool)
            .await?;
        row.map(run_from_row).transpose()
    }

    pub async fn list(&self, limit: usize) -> Result<Vec<AgentRun>> {
        let rows = sqlx::query("SELECT * FROM agent_runs ORDER BY created_at_ms DESC LIMIT ?")
            .bind(i64::try_from(limit.max(1)).unwrap_or(i64::MAX))
            .fetch_all(&self.pool)
            .await?;
        rows.into_iter().map(run_from_row).collect()
    }

    pub async fn children(&self, parent_run_id: &str) -> Result<Vec<AgentRun>> {
        let rows = sqlx::query(
            "SELECT * FROM agent_runs WHERE parent_run_id = ? ORDER BY created_at_ms ASC",
        )
        .bind(parent_run_id)
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter().map(run_from_row).collect()
    }

    pub async fn events(&self, run_id: &str, after: i64) -> Result<Vec<AgentRunEvent>> {
        let rows = sqlx::query(
            "SELECT sequence, run_id, timestamp_ms, kind, payload
             FROM agent_run_events WHERE run_id = ? AND sequence > ? ORDER BY sequence ASC",
        )
        .bind(run_id)
        .bind(after)
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter()
            .map(|row| {
                let payload: String = row.try_get("payload")?;
                Ok(AgentRunEvent {
                    sequence: row.try_get("sequence")?,
                    run_id: row.try_get("run_id")?,
                    timestamp_ms: row.try_get("timestamp_ms")?,
                    kind: row.try_get("kind")?,
                    payload: serde_json::from_str(&payload).unwrap_or_else(|_| json!({})),
                })
            })
            .collect()
    }

    pub async fn append_event(&self, run_id: &str, kind: &str, payload: Value) -> Result<i64> {
        let result = sqlx::query(
            "INSERT INTO agent_run_events (run_id, timestamp_ms, kind, payload) VALUES (?, ?, ?, ?)",
        )
        .bind(run_id)
        .bind(now_i64())
        .bind(kind)
        .bind(serde_json::to_string(&payload)?)
        .execute(&self.pool)
        .await?;
        Ok(result.last_insert_rowid())
    }

    pub async fn enqueue_message(&self, run_id: &str, role: &str, content: &str) -> Result<()> {
        if content.trim().is_empty() {
            bail!("agent message cannot be empty");
        }
        if self.get(run_id).await?.is_none() {
            bail!("agent run '{run_id}' not found");
        }
        sqlx::query(
            "INSERT INTO agent_run_messages (run_id, created_at_ms, role, content) VALUES (?, ?, ?, ?)",
        )
        .bind(run_id)
        .bind(now_i64())
        .bind(role)
        .bind(content)
        .execute(&self.pool)
        .await?;
        self.append_event(
            run_id,
            "message_queued",
            json!({"role": role, "content_chars": content.chars().count()}),
        )
        .await?;
        Ok(())
    }

    pub async fn drain_messages(&self, run_id: &str) -> Result<Vec<String>> {
        let mut transaction = self.pool.begin().await?;
        let rows = sqlx::query(
            "SELECT id, content FROM agent_run_messages
             WHERE run_id = ? AND consumed_at_ms IS NULL ORDER BY id ASC",
        )
        .bind(run_id)
        .fetch_all(&mut *transaction)
        .await?;
        let ids = rows
            .iter()
            .map(|row| row.try_get::<i64, _>("id"))
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let messages = rows
            .iter()
            .map(|row| row.try_get::<String, _>("content"))
            .collect::<std::result::Result<Vec<_>, _>>()?;
        for id in ids {
            sqlx::query("UPDATE agent_run_messages SET consumed_at_ms = ? WHERE id = ?")
                .bind(now_i64())
                .bind(id)
                .execute(&mut *transaction)
                .await?;
        }
        transaction.commit().await?;
        Ok(messages)
    }

    pub async fn set_process(&self, id: &str, pid: u32) -> Result<()> {
        let now = now_i64();
        let changed = sqlx::query(
            "UPDATE agent_runs SET pid = ?, status = 'running', started_at_ms = COALESCE(started_at_ms, ?), updated_at_ms = ?
             WHERE id = ? AND status IN ('queued', 'needs_input', 'completed', 'failed')",
        )
        .bind(i64::from(pid))
        .bind(now)
        .bind(now)
        .bind(id)
        .execute(&self.pool)
        .await?
        .rows_affected();
        if changed == 0 {
            bail!("agent run '{id}' is not launchable");
        }
        self.append_event(id, "started", json!({"pid": pid}))
            .await?;
        Ok(())
    }

    pub async fn complete(
        &self,
        id: &str,
        response: &str,
        provider: &str,
        model: &str,
    ) -> Result<()> {
        let now = now_i64();
        sqlx::query(
            "UPDATE agent_runs SET status = 'completed', response = ?, error = NULL,
             provider = ?, model = ?, pid = NULL, completed_at_ms = ?, updated_at_ms = ? WHERE id = ?",
        )
        .bind(response)
        .bind(provider)
        .bind(model)
        .bind(now)
        .bind(now)
        .bind(id)
        .execute(&self.pool)
        .await?;
        self.append_event(
            id,
            "completed",
            json!({"provider": provider, "model": model, "response_chars": response.chars().count()}),
        )
        .await?;
        Ok(())
    }

    pub async fn fail(&self, id: &str, error: &str) -> Result<()> {
        let now = now_i64();
        sqlx::query(
            "UPDATE agent_runs SET status = 'failed', error = ?, pid = NULL,
             completed_at_ms = ?, updated_at_ms = ? WHERE id = ? AND status != 'cancelled'",
        )
        .bind(error)
        .bind(now)
        .bind(now)
        .bind(id)
        .execute(&self.pool)
        .await?;
        self.append_event(id, "failed", json!({"error": error}))
            .await?;
        Ok(())
    }

    pub async fn request_cancel(&self, id: &str) -> Result<AgentRun> {
        let run = self
            .get(id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("agent run '{id}' not found"))?;
        if run.status.is_terminal() {
            return Ok(run);
        }
        sqlx::query(
            "UPDATE agent_runs SET status = 'cancel_requested', updated_at_ms = ? WHERE id = ?",
        )
        .bind(now_i64())
        .bind(id)
        .execute(&self.pool)
        .await?;
        self.append_event(id, "cancel_requested", json!({"pid": run.pid}))
            .await?;
        Ok(run)
    }

    pub async fn mark_cancelled(&self, id: &str) -> Result<()> {
        let now = now_i64();
        sqlx::query(
            "UPDATE agent_runs SET status = 'cancelled', pid = NULL,
             completed_at_ms = ?, updated_at_ms = ? WHERE id = ?",
        )
        .bind(now)
        .bind(now)
        .bind(id)
        .execute(&self.pool)
        .await?;
        self.append_event(id, "cancelled", json!({})).await?;
        Ok(())
    }

    pub async fn prepare_continuation(&self, id: &str, message: &str) -> Result<Continuation> {
        let run = self
            .get(id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("agent run '{id}' not found"))?;
        if run.status == AgentRunStatus::CancelRequested || run.status == AgentRunStatus::Cancelled
        {
            bail!(
                "agent run '{id}' is {}; retry it instead",
                run.status.label()
            );
        }
        self.enqueue_message(id, "user", message).await?;
        let launch_required = run.status.is_terminal() || run.status == AgentRunStatus::NeedsInput;
        if launch_required {
            sqlx::query(
                "UPDATE agent_runs SET status = 'queued', error = NULL, completed_at_ms = NULL, updated_at_ms = ? WHERE id = ?",
            )
            .bind(now_i64())
            .bind(id)
            .execute(&self.pool)
            .await?;
            self.append_event(id, "resumed", json!({})).await?;
        }
        Ok(Continuation {
            run: self
                .get(id)
                .await?
                .context("continued agent run disappeared")?,
            launch_required,
        })
    }

    pub async fn active_count(&self, parent_run_id: Option<&str>) -> Result<usize> {
        let row = if let Some(parent) = parent_run_id {
            sqlx::query(
                "SELECT COUNT(*) AS count FROM agent_runs WHERE parent_run_id = ? AND status IN ('queued','running','needs_input','cancel_requested')",
            )
            .bind(parent)
            .fetch_one(&self.pool)
            .await?
        } else {
            sqlx::query(
                "SELECT COUNT(*) AS count FROM agent_runs WHERE parent_run_id IS NULL AND status IN ('queued','running','needs_input','cancel_requested')",
            )
            .fetch_one(&self.pool)
            .await?
        };
        let count: i64 = row.try_get("count")?;
        Ok(usize::try_from(count).unwrap_or(usize::MAX))
    }

    pub async fn wait_terminal(&self, id: &str, timeout: Duration) -> Result<AgentRun> {
        let started = std::time::Instant::now();
        loop {
            let run = self
                .get(id)
                .await?
                .ok_or_else(|| anyhow::anyhow!("agent run '{id}' not found"))?;
            if run.status.is_terminal() {
                return Ok(run);
            }
            if started.elapsed() >= timeout {
                bail!("timed out waiting for agent run '{id}'");
            }
            tokio::time::sleep(Duration::from_millis(150)).await;
        }
    }
}

#[derive(Debug, Clone)]
pub struct LaunchOptions<'a> {
    pub cfg: &'a RuntimeConfig,
    pub always_approve: bool,
}

pub async fn launch_worker(
    store: &AgentRunStore,
    run: &AgentRun,
    options: LaunchOptions<'_>,
) -> Result<u32> {
    if let Some(parent) = run.log_path.parent() {
        std::fs::create_dir_all(parent).with_context(|| {
            format!(
                "failed to create agent log directory '{}'",
                parent.display()
            )
        })?;
    }
    let stdout = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&run.log_path)
        .with_context(|| format!("failed to open agent log '{}'", run.log_path.display()))?;
    let stderr = stdout.try_clone()?;
    let executable = std::env::current_exe().context("failed to resolve Zavora executable")?;
    let config_path = absolute_from(
        &std::env::current_dir()?,
        Path::new(&options.cfg.config_path),
    );
    let mut command = Command::new(executable);
    command
        .arg("--config")
        .arg(config_path)
        .arg("--profile")
        .arg(&options.cfg.profile)
        .arg("--output-format")
        .arg("stream-json")
        .arg("--no-stdin")
        .arg("agents")
        .arg("worker")
        .arg("--run-id")
        .arg(&run.id)
        .current_dir(&run.workspace)
        .stdin(Stdio::null())
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::from(stderr))
        .env("ZAVORA_AGENT_RUNS_DB", store.url())
        .env("ZAVORA_SESSION_BACKEND", "sqlite")
        .env("ZAVORA_SESSION_DB_URL", default_agent_session_url())
        .env("ZAVORA_SESSION_ID", &run.session_id)
        .env("ZAVORA_SUPERVISOR_RUN_ID", &run.id)
        .env(
            "ZAVORA_WORKER_PROVIDER",
            format!("{:?}", options.cfg.worker_provider).to_ascii_lowercase(),
        )
        .env("ZAVORA_WORKER_MODEL", &options.cfg.worker_model);
    if options.always_approve {
        command.arg("--always-approve");
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(error) => {
            let error =
                anyhow::Error::new(error).context("failed to launch supervised agent worker");
            store.fail(&run.id, &error.to_string()).await?;
            return Err(error);
        }
    };
    let pid = child.id();
    if let Err(error) = store.set_process(&run.id, pid).await {
        let _ = child.kill();
        store.fail(&run.id, &error.to_string()).await?;
        return Err(error);
    }
    Ok(pid)
}

pub async fn cancel_run(store: &AgentRunStore, id: &str) -> Result<AgentRun> {
    let original = store.request_cancel(id).await?;
    if original.status.is_terminal() {
        return Ok(original);
    }
    if let Some(pid) = original.pid
        && !terminate_pid(pid, id)?
    {
        store
            .append_event(
                id,
                "process_identity_mismatch",
                json!({"pid": pid, "action": "signal_refused"}),
            )
            .await?;
    }
    store.mark_cancelled(id).await?;
    store
        .get(id)
        .await?
        .context("cancelled agent run disappeared")
}

pub fn remove_worktree(run: &AgentRun, force: bool) -> Result<()> {
    let Some(path) = run.worktree_path.as_deref() else {
        bail!("agent run '{}' does not own a worktree", run.id);
    };
    if !force {
        bail!("worktree removal is destructive; re-run with --force");
    }
    let git_root = git_root(&run.workspace).or_else(|_| git_root(path))?;
    let output = Command::new("git")
        .args(["worktree", "remove", "--force"])
        .arg(path)
        .current_dir(git_root)
        .output()
        .context("failed to invoke git worktree remove")?;
    if !output.status.success() {
        bail!(
            "git worktree remove failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(())
}

pub fn default_store_url() -> String {
    std::env::var("ZAVORA_AGENT_RUNS_DB")
        .unwrap_or_else(|_| "sqlite://.zavora/agent-runs.db".to_string())
}

pub fn default_agent_session_url() -> String {
    let configured = std::env::var("ZAVORA_AGENT_SESSION_DB")
        .or_else(|_| std::env::var("ZAVORA_SESSION_DB_URL"))
        .unwrap_or_else(|_| "sqlite://.zavora/agent-sessions.db".to_string());
    absolute_sqlite_url(&configured).unwrap_or(configured)
}

fn absolute_sqlite_url(url: &str) -> Result<String> {
    let Some(path) = crate::session::sqlite_path_from_url(url) else {
        return Ok(url.to_string());
    };
    if path.is_absolute() {
        return Ok(url.to_string());
    }
    let absolute = std::env::current_dir()?.join(path);
    Ok(format!("sqlite://{}", absolute.display()))
}

fn provision_worktree(workspace: &Path, run_id: &str) -> Result<PathBuf> {
    let root = git_root(workspace)?;
    let repo_name = root
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("repository");
    let digest = format!("{:x}", md5::compute(root.to_string_lossy().as_bytes()));
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .context("HOME is required to create a durable worktree")?;
    let path = home
        .join(".zavora/worktrees")
        .join(format!("{repo_name}-{}", &digest[..8]))
        .join(run_id);
    if path.exists() {
        bail!("worktree path '{}' already exists", path.display());
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let output = Command::new("git")
        .args(["worktree", "add", "--detach"])
        .arg(&path)
        .arg("HEAD")
        .current_dir(&root)
        .output()
        .context("failed to invoke git worktree add")?;
    if !output.status.success() {
        bail!(
            "git worktree add failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(path)
}

fn git_root(workspace: &Path) -> Result<PathBuf> {
    let output = Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .current_dir(workspace)
        .output()
        .context("failed to inspect git workspace")?;
    if !output.status.success() {
        bail!("worktree isolation requires a git repository");
    }
    Ok(PathBuf::from(
        String::from_utf8(output.stdout)?.trim().to_string(),
    ))
}

#[cfg(unix)]
fn terminate_pid(pid: u32, run_id: &str) -> Result<bool> {
    let inspection = Command::new("ps")
        .args(["-p", &pid.to_string(), "-o", "command="])
        .output()
        .context("failed to inspect supervised agent worker")?;
    if !inspection.status.success() {
        return Ok(false);
    }
    let command = String::from_utf8_lossy(&inspection.stdout);
    let marker = format!("worker --run-id {run_id}");
    if !command.contains(&marker) {
        return Ok(false);
    }
    let pid = i32::try_from(pid).context("worker pid exceeds platform range")?;
    let result = unsafe { libc::kill(pid, libc::SIGTERM) };
    if result != 0 {
        let error = std::io::Error::last_os_error();
        if error.raw_os_error() != Some(libc::ESRCH) {
            return Err(error).context("failed to terminate supervised agent worker");
        }
    }
    Ok(true)
}

#[cfg(not(unix))]
fn terminate_pid(pid: u32, _run_id: &str) -> Result<bool> {
    let status = Command::new("taskkill")
        .args(["/PID", &pid.to_string(), "/T"])
        .status()
        .context("failed to invoke taskkill")?;
    if !status.success() {
        bail!("taskkill failed for worker pid {pid}");
    }
    Ok(true)
}

fn run_from_row(row: sqlx::sqlite::SqliteRow) -> Result<AgentRun> {
    let status: String = row.try_get("status")?;
    let depth: i64 = row.try_get("depth")?;
    let pid: Option<i64> = row.try_get("pid")?;
    Ok(AgentRun {
        id: row.try_get("id")?,
        parent_run_id: row.try_get("parent_run_id")?,
        parent_session_id: row.try_get("parent_session_id")?,
        session_id: row.try_get("session_id")?,
        agent: row.try_get("agent")?,
        task: row.try_get("task")?,
        status: AgentRunStatus::parse(&status)?,
        depth: u32::try_from(depth).context("invalid stored agent depth")?,
        workspace: PathBuf::from(row.try_get::<String, _>("workspace")?),
        worktree_path: row
            .try_get::<Option<String>, _>("worktree_path")?
            .map(PathBuf::from),
        provider: row.try_get("provider")?,
        model: row.try_get("model")?,
        pid: pid
            .map(u32::try_from)
            .transpose()
            .context("invalid stored worker pid")?,
        log_path: PathBuf::from(row.try_get::<String, _>("log_path")?),
        response: row.try_get("response")?,
        error: row.try_get("error")?,
        retry_of: row.try_get("retry_of")?,
        created_at_ms: row.try_get("created_at_ms")?,
        started_at_ms: row.try_get("started_at_ms")?,
        completed_at_ms: row.try_get("completed_at_ms")?,
        updated_at_ms: row.try_get("updated_at_ms")?,
    })
}

fn new_run_id() -> String {
    format!(
        "ar-{}-{}-{}",
        unix_ms_now(),
        std::process::id(),
        NEXT_RUN_ID.fetch_add(1, Ordering::Relaxed)
    )
}

fn now_i64() -> i64 {
    i64::try_from(unix_ms_now()).unwrap_or(i64::MAX)
}

fn path_string(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

fn absolute_from(base: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        base.join(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn store() -> (tempfile::TempDir, AgentRunStore) {
        let temp = tempfile::tempdir().unwrap();
        let url = format!("sqlite://{}", temp.path().join("runs.db").display());
        let store = AgentRunStore::open(&url).await.unwrap();
        (temp, store)
    }

    #[tokio::test]
    async fn run_graph_messages_and_events_round_trip() {
        let (temp, store) = store().await;
        let parent = store
            .create_run(
                NewAgentRun {
                    parent_run_id: None,
                    parent_session_id: "parent-session".into(),
                    agent: "developer_agent".into(),
                    task: "inspect".into(),
                    workspace: temp.path().into(),
                    worktree: false,
                    retry_of: None,
                },
                &SupervisorPolicy::default(),
            )
            .await
            .unwrap();
        let child = store
            .create_run(
                NewAgentRun {
                    parent_run_id: Some(parent.id.clone()),
                    parent_session_id: parent.session_id.clone(),
                    agent: "reviewer_agent".into(),
                    task: "review".into(),
                    workspace: temp.path().into(),
                    worktree: false,
                    retry_of: None,
                },
                &SupervisorPolicy::default(),
            )
            .await
            .unwrap();
        assert_eq!(child.depth, 1);
        assert_eq!(store.children(&parent.id).await.unwrap().len(), 1);
        assert_eq!(store.drain_messages(&child.id).await.unwrap(), ["review"]);
        assert!(!store.events(&child.id, 0).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn depth_and_active_limits_are_enforced() {
        let (temp, store) = store().await;
        let policy = SupervisorPolicy {
            max_depth: 0,
            max_active_children: 1,
        };
        let first = store
            .create_run(
                NewAgentRun {
                    parent_run_id: None,
                    parent_session_id: "s".into(),
                    agent: "one".into(),
                    task: "one".into(),
                    workspace: temp.path().into(),
                    worktree: false,
                    retry_of: None,
                },
                &policy,
            )
            .await
            .unwrap();
        let second = store
            .create_run(
                NewAgentRun {
                    parent_run_id: None,
                    parent_session_id: "s".into(),
                    agent: "two".into(),
                    task: "two".into(),
                    workspace: temp.path().into(),
                    worktree: false,
                    retry_of: None,
                },
                &policy,
            )
            .await;
        assert!(
            second
                .unwrap_err()
                .to_string()
                .contains("active child limit")
        );
        store.mark_cancelled(&first.id).await.unwrap();
    }

    #[tokio::test]
    async fn followups_queue_while_busy_and_resume_terminal_sessions() {
        let (temp, store) = store().await;
        let run = store
            .create_run(
                NewAgentRun {
                    parent_run_id: None,
                    parent_session_id: "s".into(),
                    agent: "developer_agent".into(),
                    task: "first".into(),
                    workspace: temp.path().into(),
                    worktree: false,
                    retry_of: None,
                },
                &SupervisorPolicy::default(),
            )
            .await
            .unwrap();
        let queued = store.prepare_continuation(&run.id, "second").await.unwrap();
        assert!(!queued.launch_required);
        assert_eq!(
            store.drain_messages(&run.id).await.unwrap(),
            ["first", "second"]
        );
        store
            .complete(&run.id, "done", "test", "test-model")
            .await
            .unwrap();
        let resumed = store.prepare_continuation(&run.id, "third").await.unwrap();
        assert!(resumed.launch_required);
        assert_eq!(resumed.run.session_id, run.session_id);
        assert_eq!(resumed.run.status, AgentRunStatus::Queued);
    }

    #[test]
    fn interactive_spawn_parser_supports_worktrees_and_quoted_tasks() {
        let request =
            parse_spawn_request("@developer_agent --worktree 'implement the focused change'")
                .unwrap();
        assert_eq!(request.agent, "developer_agent");
        assert_eq!(request.task, "implement the focused change");
        assert!(request.worktree);
    }
}
