pub mod confirming;
pub mod execute_bash;
pub mod file_edit;
pub mod fs_read;
pub mod fs_write;
pub mod github_ops;
pub mod glob;
pub mod grep;
pub mod bash_security;
#[cfg(feature = "browser")]
pub mod browser;
#[cfg(feature = "lsp")]
pub mod lsp;
#[cfg(feature = "rag")]
pub mod rag;
#[cfg(feature = "sandbox")]
pub mod sandbox;
pub mod tool_search;
#[cfg(feature = "web-fetch")]
pub mod web_fetch;

use std::sync::Arc;

use adk_rust::prelude::*;
use serde_json::{Value, json};

use crate::todos;

pub const FS_READ_TOOL_NAME: &str = "fs_read";
pub const FS_WRITE_TOOL_NAME: &str = "fs_write";
pub const FILE_EDIT_TOOL_NAME: &str = "file_edit";
pub const EXECUTE_BASH_TOOL_NAME: &str = "execute_bash";
pub const GITHUB_OPS_TOOL_NAME: &str = "github_ops";
pub const GLOB_TOOL_NAME: &str = "glob";
pub const GREP_TOOL_NAME: &str = "grep";
pub const TODO_TOOL_NAME: &str = "todo_list";

pub fn build_builtin_tools() -> Vec<Arc<dyn Tool>> {
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
    )
    .with_read_only(true)
    .with_concurrency_safe(true);

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
    )
    .with_read_only(true)
    .with_concurrency_safe(true);

    let fs_read = FunctionTool::new(
        "fs_read",
        "Reads file content or directory entries within the workspace using path policy checks. \
         Args: path (required), start_line, max_lines, max_bytes, max_entries.",
        |_ctx, args| async move { Ok(fs_read::fs_read_tool_response(&args)) },
    )
    .with_read_only(true)
    .with_concurrency_safe(true);

    let fs_write = FunctionTool::new(
        "fs_write",
        "Writes files within the workspace with safe modes. \
         Args: path (required), mode=create|overwrite|append|patch, content, patch={find,replace,replace_all}.",
        |_ctx, args| async move { Ok(fs_write::fs_write_tool_response(&args)) },
    );

    let file_edit = FunctionTool::new(
        "file_edit",
        "Makes surgical text replacements in files. Preferred over fs_write for editing existing files. \
         Args: file_path (required), old_string (required, exact text to find), \
         new_string (required, replacement text), replace_all (optional bool, default false). \
         Returns a unified diff of the change. Fails if old_string is not found or matches multiple locations (unless replace_all=true).",
        |_ctx, args| async move { Ok(file_edit::file_edit_tool_response(&args)) },
    );

    let glob_tool = FunctionTool::new(
        "glob",
        "Finds files matching a glob pattern. Respects .gitignore. \
         Args: pattern (required, e.g. '**/*.rs', 'src/**/*.{ts,tsx}'), path (optional search root, default cwd). \
         Returns { numFiles, filenames, truncated, durationMs }. Max 100 results.",
        |_ctx, args| async move { Ok(glob::glob_tool_response(&args)) },
    )
    .with_read_only(true)
    .with_concurrency_safe(true);

    let grep_tool = FunctionTool::new(
        "grep",
        "Searches file contents using regex patterns (ripgrep). \
         Args: pattern (required regex), path (optional search root), glob (file filter e.g. '*.rs'), \
         output_mode ('content'|'files_with_matches'|'count', default 'files_with_matches'), \
         -i (case insensitive), -B/-A/-C (context lines, content mode), \
         file_type (e.g. 'rust','py'), multiline (bool), head_limit (default 250), offset. \
         Falls back to grep -rn if rg is not installed.",
        |_ctx, args| async move { Ok(grep::grep_tool_response(&args)) },
    )
    .with_read_only(true)
    .with_concurrency_safe(true);

    #[cfg(feature = "web-fetch")]
    let web_fetch_tool = FunctionTool::new(
        "web_fetch",
        "Fetches a URL and returns content as markdown. Requires confirmation. \
         Args: url (required), prompt (required, instruction for processing the content). \
         Converts HTML to markdown, pretty-prints JSON, passes text through. \
         Blocks localhost/private IPs/metadata endpoints. Max 100KB. \
         Returns { url, code, codeText, bytes, result, prompt, durationMs }.",
        |_ctx, args| async move { Ok(web_fetch::web_fetch_tool_response(&args).await) },
    );

    #[cfg(feature = "lsp")]
    let lsp_tool = FunctionTool::new(
        "lsp",
        "Semantic code intelligence via Language Server Protocol. \
         Args: operation (required: goToDefinition|findReferences|hover|documentSymbol|workspaceSymbol|goToImplementation|prepareCallHierarchy|incomingCalls|outgoingCalls), \
         filePath (required), line (1-based), character (1-based). \
         Requires `zavora lsp init` to configure language servers.",
        |_ctx, args| async move { Ok(lsp::lsp_tool_response(&args).await) },
    )
    .with_read_only(true)
    .with_concurrency_safe(true);

    let execute_bash = FunctionTool::new(
        "execute_bash",
        "Executes shell commands with policy checks and approval gates. \
         Args: command (required), approved, allow_dangerous, timeout_secs, retry_attempts, retry_delay_ms, max_output_chars.",
        |_ctx, args| async move { Ok(execute_bash::execute_bash_tool_response(&args).await) },
    );

    let github_ops = FunctionTool::new(
        "github_ops",
        "Runs GitHub workflow operations through gh CLI. \
         Args: action=issue_create|issue_update|pr_create|project_item_update plus action-specific fields.",
        |_ctx, args| async move { Ok(github_ops::github_ops_tool_response(&args)) },
    );

    let _todo_list = FunctionTool::new(
        "todo_list",
        "Manage task lists for structured execution planning. \
         Args: action=create|complete|view|list|delete. \
         create: {id, description, tasks: [string]}. \
         complete: {id, task_index: number}. \
         view: {id}. list: {}. delete: {id}.",
        |_ctx, args| async move { Ok(todo_tool_response(&args)) },
    );

    let todo_list = FunctionTool::new(
        "todo_list",
        "Manage task lists for structured execution planning. \
         Args: action=create|complete|view|list|delete. \
         create: {id, description, tasks: [string]}. \
         complete: {id, task_index: number}. \
         view: {id}. list: {}. delete: {id}.",
        |_ctx, args| async move { Ok(todo_tool_response(&args)) },
    );

    // Agent tools
    let workspace = std::env::current_dir().unwrap_or_default();
    let time_agent = crate::agents::tools::TimeAgentTool::new();
    let memory_agent = crate::agents::tools::MemoryAgentTool::new(workspace);

    #[allow(unused_mut)]
    let mut tools: Vec<Arc<dyn Tool>> = vec![
        Arc::new(current_time),
        Arc::new(release_template),
        Arc::new(fs_read),
        Arc::new(fs_write),
        Arc::new(file_edit),
        Arc::new(glob_tool),
        Arc::new(grep_tool),
        #[cfg(feature = "web-fetch")]
        Arc::new(web_fetch_tool),
        #[cfg(feature = "lsp")]
        Arc::new(lsp_tool),
        Arc::new(execute_bash),
        Arc::new(github_ops),
        Arc::new(todo_list),
        Arc::new(time_agent),
        Arc::new(memory_agent),
    ];

    // Feature-gated: sandbox code execution
    #[cfg(feature = "sandbox")]
    tools.push(sandbox::build_sandbox_tool());

    // Feature-gated: RAG retrieval tool
    #[cfg(feature = "rag")]
    if let Ok(rag_tool) = rag::build_rag_tool() {
        tools.push(rag_tool);
    }

    tools
}

fn todo_tool_response(args: &Value) -> Value {
    let workspace = std::env::current_dir().unwrap_or_default();
    let action = args.get("action").and_then(Value::as_str).unwrap_or("");

    match action {
        "create" => {
            let id = args.get("id").and_then(Value::as_str).unwrap_or("untitled");
            let description = args
                .get("description")
                .and_then(Value::as_str)
                .unwrap_or("");
            let tasks: Vec<String> = args
                .get("tasks")
                .and_then(Value::as_array)
                .map(|arr| {
                    arr.iter()
                        .filter_map(Value::as_str)
                        .map(String::from)
                        .collect()
                })
                .unwrap_or_default();
            let todo = todos::TodoList::new(id, description, tasks);
            match todos::save_todo(&workspace, &todo) {
                Ok(()) => json!({"status": "created", "id": id, "tasks": todo.tasks.len()}),
                Err(e) => json!({"error": e.to_string()}),
            }
        }
        "complete" => {
            let id = args.get("id").and_then(Value::as_str).unwrap_or("");
            let index = args.get("task_index").and_then(Value::as_u64).unwrap_or(0) as usize;
            match todos::load_todo(&workspace, id) {
                Ok(mut todo) => {
                    if todo.complete_task(index) {
                        let _ = todos::save_todo(&workspace, &todo);
                        json!({"status": "completed", "id": id, "task_index": index})
                    } else {
                        json!({"error": format!("task index {index} out of range")})
                    }
                }
                Err(e) => json!({"error": e.to_string()}),
            }
        }
        "view" => {
            let id = args.get("id").and_then(Value::as_str).unwrap_or("");
            match todos::load_todo(&workspace, id) {
                Ok(todo) => json!({
                    "id": todo.id,
                    "description": todo.description,
                    "tasks": todo.tasks.iter().map(|t| json!({
                        "description": t.description,
                        "completed": t.completed,
                    })).collect::<Vec<_>>(),
                    "completed": todo.completed_count(),
                    "total": todo.tasks.len(),
                }),
                Err(e) => json!({"error": e.to_string()}),
            }
        }
        "list" => match todos::list_todo_ids(&workspace) {
            Ok(ids) => json!({"todo_lists": ids}),
            Err(e) => json!({"error": e.to_string()}),
        },
        "delete" => {
            let id = args.get("id").and_then(Value::as_str).unwrap_or("");
            match todos::delete_todo(&workspace, id) {
                Ok(()) => json!({"status": "deleted", "id": id}),
                Err(e) => json!({"error": e.to_string()}),
            }
        }
        _ => {
            json!({"error": format!("unknown action '{action}'. Use create|complete|view|list|delete")})
        }
    }
}
