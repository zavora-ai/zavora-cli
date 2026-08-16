pub mod bash_security;
#[cfg(feature = "browser")]
pub mod browser;
pub mod confirming;
pub mod execute_bash;
pub mod file_edit;
pub mod fs_read;
pub mod fs_write;
pub mod github_ops;
pub mod glob;
pub mod grep;
#[cfg(feature = "lsp")]
pub mod lsp;
#[cfg(feature = "rag")]
pub mod rag;
#[cfg(feature = "sandbox")]
pub mod sandbox;
pub mod secret_policy;
pub mod tool_search;
#[cfg(feature = "web-fetch")]
pub mod web_fetch;

use std::sync::Arc;

use adk_rust::prelude::*;
use schemars::JsonSchema;
use serde::Serialize;
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

#[derive(JsonSchema, Serialize)]
struct EmptyArgs {}

#[derive(JsonSchema, Serialize)]
struct ReleaseTemplateArgs {
    /// Number of staged releases to include in the checklist.
    releases: Option<u64>,
}

#[derive(JsonSchema, Serialize)]
struct FsReadArgs {
    /// Workspace-relative file or directory path.
    path: String,
    /// First line to return, using one-based indexing.
    start_line: Option<usize>,
    /// Maximum number of lines to return.
    max_lines: Option<usize>,
    /// Maximum number of bytes to return.
    max_bytes: Option<usize>,
    /// Maximum number of directory entries to return.
    max_entries: Option<usize>,
}

#[derive(JsonSchema, Serialize)]
struct FsWritePatchArgs {
    /// Exact text to replace.
    find: String,
    /// Replacement text.
    replace: String,
    /// Replace every match instead of requiring one unique match.
    replace_all: Option<bool>,
}

#[derive(JsonSchema, Serialize)]
struct FsWriteArgs {
    /// Workspace-relative file path.
    path: String,
    /// Write mode: create, overwrite, append, or patch.
    mode: Option<String>,
    /// File content for create, overwrite, or append.
    content: Option<String>,
    /// Text replacement for patch mode.
    patch: Option<FsWritePatchArgs>,
}

#[derive(JsonSchema, Serialize)]
struct FileEditArgs {
    /// Workspace-relative path of the existing file to edit.
    file_path: String,
    /// Exact text currently present in the file.
    old_string: String,
    /// Replacement text.
    new_string: String,
    /// Replace every match instead of requiring one unique match.
    replace_all: Option<bool>,
}

#[derive(JsonSchema, Serialize)]
struct GlobArgs {
    /// Glob pattern such as **/*.rs or src/**/*.{ts,tsx}.
    pattern: String,
    /// Optional workspace-relative directory to search.
    path: Option<String>,
}

#[derive(JsonSchema, Serialize)]
struct GrepArgs {
    /// Regular expression to search for.
    pattern: String,
    /// Optional workspace-relative path to search.
    path: Option<String>,
    /// Optional file glob such as *.rs.
    glob: Option<String>,
    /// Result shape: content, files_with_matches, or count.
    output_mode: Option<String>,
    /// Search case-insensitively.
    #[serde(rename = "-i")]
    case_insensitive: Option<bool>,
    /// Lines of context before each match.
    #[serde(rename = "-B")]
    before_context: Option<usize>,
    /// Lines of context after each match.
    #[serde(rename = "-A")]
    after_context: Option<usize>,
    /// Lines of context before and after each match.
    #[serde(rename = "-C")]
    context: Option<usize>,
    /// Ripgrep file type such as rust or py.
    file_type: Option<String>,
    /// Enable multiline matching.
    multiline: Option<bool>,
    /// Maximum number of results to return.
    head_limit: Option<usize>,
    /// Number of results to skip.
    offset: Option<usize>,
}

#[derive(JsonSchema, Serialize)]
struct TodoArgs {
    /// Operation: create, complete, view, list, or delete.
    action: String,
    /// List identifier used by create, complete, view, and delete.
    id: Option<String>,
    /// Short description used when creating a list.
    description: Option<String>,
    /// Task descriptions used when creating a list.
    tasks: Option<Vec<String>>,
    /// Zero-based task index used by complete.
    task_index: Option<usize>,
}

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
    .with_parameters_schema::<EmptyArgs>()
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
    .with_parameters_schema::<ReleaseTemplateArgs>()
    .with_read_only(true)
    .with_concurrency_safe(true);

    let fs_read = FunctionTool::new(
        "fs_read",
        "Reads file content or directory entries within the workspace using path policy checks. \
         Args: path (required), start_line, max_lines, max_bytes, max_entries.",
        |_ctx, args| async move { Ok(fs_read::fs_read_tool_response(&args)) },
    )
    .with_parameters_schema::<FsReadArgs>()
    .with_read_only(true)
    .with_concurrency_safe(true);

    let fs_write = FunctionTool::new(
        "fs_write",
        "Writes files within the workspace with safe modes. \
         Args: path (required), mode=create|overwrite|append|patch, content, patch={find,replace,replace_all}.",
        |_ctx, args| async move { Ok(fs_write::fs_write_tool_response(&args)) },
    )
    .with_parameters_schema::<FsWriteArgs>();

    let file_edit = FunctionTool::new(
        "file_edit",
        "Makes surgical text replacements in files. Preferred over fs_write for editing existing files. \
         Args: file_path (required), old_string (required, exact text to find), \
         new_string (required, replacement text), replace_all (optional bool, default false). \
         Returns a unified diff of the change. Fails if old_string is not found or matches multiple locations (unless replace_all=true).",
        |_ctx, args| async move { Ok(file_edit::file_edit_tool_response(&args)) },
    )
    .with_parameters_schema::<FileEditArgs>();

    let glob_tool = FunctionTool::new(
        "glob",
        "Finds files matching a glob pattern. Respects .gitignore. \
         Args: pattern (required, e.g. '**/*.rs', 'src/**/*.{ts,tsx}'), path (optional search root, default cwd). \
         Returns { numFiles, filenames, truncated, durationMs }. Max 100 results.",
        |_ctx, args| async move { Ok(glob::glob_tool_response(&args)) },
    )
    .with_parameters_schema::<GlobArgs>()
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
    .with_parameters_schema::<GrepArgs>()
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
    )
    .with_parameters_schema::<TodoArgs>()
    .with_read_only(true)
    .with_concurrency_safe(true);

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

#[cfg(test)]
mod schema_tests {
    use super::*;

    #[test]
    fn file_tools_publish_the_arguments_models_need() {
        let tools = build_builtin_tools();

        for (name, required_field) in [
            ("fs_read", "path"),
            ("fs_write", "path"),
            ("file_edit", "file_path"),
            ("glob", "pattern"),
            ("grep", "pattern"),
            ("todo_list", "action"),
        ] {
            let tool = tools
                .iter()
                .find(|tool| tool.name() == name)
                .unwrap_or_else(|| panic!("missing built-in tool {name}"));
            let schema = tool
                .parameters_schema()
                .unwrap_or_else(|| panic!("{name} did not publish a parameter schema"));

            assert!(
                schema
                    .get("properties")
                    .and_then(|properties| properties.get(required_field))
                    .is_some(),
                "{name} schema did not describe {required_field}: {schema}"
            );
            assert!(
                schema
                    .get("required")
                    .and_then(Value::as_array)
                    .is_some_and(|required| required.iter().any(|field| field == required_field)),
                "{name} schema did not require {required_field}: {schema}"
            );
        }
    }

    #[test]
    fn provider_zero_for_optional_read_limit_uses_the_default() {
        let args = json!({ "start_line": 0 });
        assert_eq!(
            fs_read::parse_fs_read_usize_arg(&args, "start_line", 1, 1, 1_000_000).unwrap(),
            1
        );
    }
}
