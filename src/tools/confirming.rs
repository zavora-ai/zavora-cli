use std::collections::HashSet;
use std::io::{self, Write};
use std::sync::{Arc, Mutex};

use adk_rust::prelude::*;
use serde_json::Value;

use crate::theme::{self, BOLD, CYAN, DIM, GREEN, RESET};

const RED: &str = "\x1b[31m";

/// Display tool result after execution.
fn display_result(tool_name: &str, result: &Value) {
    match tool_name {
        "execute_bash" => {
            let stdout = result.get("stdout").and_then(|v| v.as_str()).unwrap_or("");
            let stderr = result.get("stderr").and_then(|v| v.as_str()).unwrap_or("");
            let status = result.get("status").and_then(|v| v.as_str()).unwrap_or("");
            if !stdout.is_empty() {
                eprint!("{stdout}");
                if !stdout.ends_with('\n') { eprintln!(); }
            }
            if !stderr.is_empty() {
                eprint!("{RED}{stderr}{RESET}");
                if !stderr.ends_with('\n') { eprintln!(); }
            }
            if status == "error" {
                if let Some(err) = result.get("error").and_then(|v| v.as_str()) {
                    eprintln!("{RED}{err}{RESET}");
                }
            }
        }
        "fs_write" => {
            let path = result.get("path").and_then(|v| v.as_str()).unwrap_or("");
            if result.get("error").is_some() {
                if let Some(err) = result.get("error").and_then(|v| v.as_str()) {
                    eprintln!("{RED}{err}{RESET}");
                }
            } else if !path.is_empty() {
                eprintln!("{DIM}  ✓ wrote {path}{RESET}");
            }
        }
        _ => {}
    }
}

/// Set of tool names trusted for the session (skip future prompts).
static TRUSTED_TOOLS: std::sync::LazyLock<Mutex<HashSet<String>>> =
    std::sync::LazyLock::new(|| Mutex::new(HashSet::new()));

/// Trust a tool for the remainder of the session.
pub fn trust_tool(name: &str) {
    TRUSTED_TOOLS.lock().unwrap().insert(name.to_string());
}

/// Check if agent mode is active (all core tools trusted).
pub fn is_agent_mode() -> bool {
    let set = TRUSTED_TOOLS.lock().unwrap();
    set.contains("fs_read") && set.contains("fs_write") && set.contains("execute_bash")
}

/// Wraps a tool with an interactive confirmation prompt.
pub struct ConfirmingTool {
    inner: Arc<dyn Tool>,
    /// When true, show what the tool is doing but don't prompt — auto-approve.
    display_only: bool,
}

impl ConfirmingTool {
    pub fn wrap(tool: Arc<dyn Tool>) -> Arc<dyn Tool> {
        Arc::new(Self { inner: tool, display_only: false })
    }

    /// Wrap a tool in display-only mode: shows what it's doing but auto-approves.
    pub fn wrap_display_only(tool: Arc<dyn Tool>) -> Arc<dyn Tool> {
        Arc::new(Self { inner: tool, display_only: true })
    }

    /// Execute inner tool and display the result.
    async fn execute_and_display(&self, ctx: Arc<dyn ToolContext>, args: Value) -> adk_rust::Result<Value> {
        let result = self.inner.execute(ctx, args).await?;
        display_result(self.inner.name(), &result);
        Ok(result)
    }
}

/// Format a file diff for the confirmation dialog.
fn format_fs_write_diff(args: &Value) -> String {
    let path = args.get("path").and_then(|v| v.as_str()).unwrap_or("?");
    let mode = args.get("mode").and_then(|v| v.as_str()).unwrap_or("create");

    let mut out = format!("{BOLD}{CYAN}{path}{RESET}\n");

    match mode {
        "create" | "overwrite" => {
            if let Some(content) = args.get("content").and_then(|v| v.as_str()) {
                // Show existing file content as removals for overwrite
                if mode == "overwrite" {
                    if let Ok(existing) = std::fs::read_to_string(path) {
                        for line in existing.lines() {
                            out.push_str(&format!("{RED}- {line}{RESET}\n"));
                        }
                    }
                }
                for line in content.lines() {
                    out.push_str(&format!("{GREEN}+ {line}{RESET}\n"));
                }
            }
        }
        "append" => {
            if let Some(content) = args.get("content").and_then(|v| v.as_str()) {
                out.push_str(&format!("{DIM}... existing content ...{RESET}\n"));
                for line in content.lines() {
                    out.push_str(&format!("{GREEN}+ {line}{RESET}\n"));
                }
            }
        }
        "patch" => {
            if let Some(patch) = args.get("patch") {
                let find = patch.get("find").and_then(|v| v.as_str()).unwrap_or("");
                let replace = patch.get("replace").and_then(|v| v.as_str()).unwrap_or("");
                for line in find.lines() {
                    out.push_str(&format!("{RED}- {line}{RESET}\n"));
                }
                for line in replace.lines() {
                    out.push_str(&format!("{GREEN}+ {line}{RESET}\n"));
                }
            }
        }
        _ => {
            // Fallback: show raw args
            let pretty = serde_json::to_string_pretty(args).unwrap_or_else(|_| args.to_string());
            out.push_str(&format!("{DIM}{pretty}{RESET}\n"));
        }
    }

    out
}

/// Format generic tool args for display.
fn format_tool_args(args: &Value) -> String {
    let pretty = serde_json::to_string_pretty(args).unwrap_or_else(|_| args.to_string());
    if pretty.len() > 400 {
        format!("{DIM}{}...{RESET}", &pretty[..400])
    } else {
        format!("{DIM}{pretty}{RESET}")
    }
}

#[async_trait::async_trait]
impl Tool for ConfirmingTool {
    fn name(&self) -> &str {
        self.inner.name()
    }

    fn description(&self) -> &str {
        self.inner.description()
    }

    fn parameters_schema(&self) -> Option<Value> {
        self.inner.parameters_schema()
    }

    fn response_schema(&self) -> Option<Value> {
        self.inner.response_schema()
    }

    async fn execute(&self, ctx: Arc<dyn ToolContext>, args: Value) -> adk_rust::Result<Value> {
        let trusted = TRUSTED_TOOLS.lock().unwrap().contains(self.inner.name());

        theme::pause_spinner();

        // Always show what the tool is doing (Q CLI pattern: transparency even when trusted)
        let display = if self.inner.name() == "fs_write" {
            format_fs_write_diff(&args)
        } else if self.inner.name() == "execute_bash" {
            let cmd = args.get("command").and_then(|v| v.as_str()).unwrap_or("?");
            format!("{BOLD}{CYAN}${RESET} {cmd}\n")
        } else if self.inner.name() == "fs_read" {
            let path = args.get("path").and_then(|v| v.as_str()).unwrap_or("?");
            let range = match (
                args.get("start_line").and_then(|v| v.as_i64()),
                args.get("end_line").and_then(|v| v.as_i64()),
            ) {
                (Some(s), Some(e)) => format!(" {DIM}(lines {s}–{e}){RESET}"),
                (Some(s), None) => format!(" {DIM}(from line {s}){RESET}"),
                _ => String::new(),
            };
            format!("{DIM}📖 {RESET}{BOLD}{CYAN}{path}{RESET}{range}\n")
        } else {
            format!("{BOLD}{CYAN}{}{RESET} {}\n", self.inner.name(), format_tool_args(&args))
        };

        eprint!("{display}");

        // If trusted or display-only, show action and execute immediately
        if trusted || self.display_only {
            theme::resume_spinner();
            let mut approved_args = args;
            if let Some(obj) = approved_args.as_object_mut() {
                obj.insert("approved".to_string(), Value::Bool(true));
            }
            return self.execute_and_display(ctx, approved_args).await;
        }

        eprintln!(
            "{DIM}Allow this action? Use '{GREEN}t{DIM}' to trust this tool for the session. [{GREEN}y{DIM}/{GREEN}n{DIM}/{GREEN}t{DIM}]:{RESET}"
        );
        eprint!("{BOLD}> {RESET}");
        let _ = io::stderr().flush();

        let input = tokio::task::spawn_blocking(|| {
            let mut buf = String::new();
            let _ = io::stdin().read_line(&mut buf);
            buf.trim().to_lowercase()
        })
        .await
        .unwrap_or_default();

        theme::resume_spinner();

        match input.as_str() {
            "t" | "trust" => {
                TRUSTED_TOOLS.lock().unwrap().insert(self.inner.name().to_string());
                let mut approved_args = args;
                if let Some(obj) = approved_args.as_object_mut() {
                    obj.insert("approved".to_string(), Value::Bool(true));
                }
                self.execute_and_display(ctx, approved_args).await
            }
            "y" | "yes" => {
                let mut approved_args = args;
                if let Some(obj) = approved_args.as_object_mut() {
                    obj.insert("approved".to_string(), Value::Bool(true));
                }
                self.execute_and_display(ctx, approved_args).await
            }
            _ => {
                eprintln!("  {DIM}Tool denied.{RESET}");
                Ok(serde_json::json!({
                    "error": format!("Tool '{}' denied by user", self.inner.name())
                }))
            }
        }
    }
}
