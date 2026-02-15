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
    let json = std::fs::read_to_string(&path)
        .with_context(|| format!("failed to read todo '{id}'"))?;
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
        if let Ok(todo) = load_todo(workspace, &id) {
            if todo.is_finished() {
                delete_todo(workspace, &id)?;
                cleared += 1;
            }
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
    out.push_str(&format!("\nUse /todos view <id> | delete <id> | clear-finished"));
    Ok(out)
}

// ---------------------------------------------------------------------------
// Delegate (experimental)
// ---------------------------------------------------------------------------

/// A delegate request to run an isolated sub-agent task.
#[derive(Debug, Clone)]
pub struct DelegateRequest {
    pub task: String,
    pub session_id: String,
}

/// Result of a delegate run.
#[derive(Debug, Clone)]
pub struct DelegateResult {
    pub task: String,
    pub session_id: String,
    pub output: String,
    pub success: bool,
}

impl DelegateResult {
    /// Format the delegate result for display.
    pub fn format_display(&self) -> String {
        let status = if self.success { "✓" } else { "✗" };
        format!(
            "[{status}] Delegate '{}' (session: {})\n{}",
            self.task, self.session_id, self.output
        )
    }
}
