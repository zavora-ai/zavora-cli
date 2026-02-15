pub mod fs_read;
pub mod fs_write;
pub mod execute_bash;
pub mod github_ops;

use std::sync::Arc;

use adk_rust::prelude::*;
use serde_json::{Value, json};

pub const FS_WRITE_TOOL_NAME: &str = "fs_write";
pub const EXECUTE_BASH_TOOL_NAME: &str = "execute_bash";
pub const GITHUB_OPS_TOOL_NAME: &str = "github_ops";

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

    let fs_read = FunctionTool::new(
        "fs_read",
        "Reads file content or directory entries within the workspace using path policy checks. \
         Args: path (required), start_line, max_lines, max_bytes, max_entries.",
        |_ctx, args| async move { Ok(fs_read::fs_read_tool_response(&args)) },
    );

    let fs_write = FunctionTool::new(
        "fs_write",
        "Writes files within the workspace with safe modes. \
         Args: path (required), mode=create|overwrite|append|patch, content, patch={find,replace,replace_all}.",
        |_ctx, args| async move { Ok(fs_write::fs_write_tool_response(&args)) },
    );

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

    vec![
        Arc::new(current_time),
        Arc::new(release_template),
        Arc::new(fs_read),
        Arc::new(fs_write),
        Arc::new(execute_bash),
        Arc::new(github_ops),
    ]
}
