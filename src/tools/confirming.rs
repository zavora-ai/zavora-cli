use std::collections::HashSet;
use std::io::{self, Write};
use std::sync::{Arc, Mutex};

use adk_rust::prelude::*;
use serde_json::Value;

use crate::theme::{self, BOLD, CYAN, DIM, GREEN, RESET};

const RED: &str = "\x1b[31m";

// Diff background colors (truecolor, matching Q CLI's base16-ocean.dark)
const BG_DELETE: &str = "\x1b[48;2;36;25;28m";
const BG_INSERT: &str = "\x1b[48;2;24;38;30m";
const BG_GUTTER_DELETE: &str = "\x1b[48;2;79;40;40m";
const BG_GUTTER_INSERT: &str = "\x1b[48;2;40;67;43m";
const CLEAR_LINE: &str = "\x1b[K";

use std::sync::LazyLock;
use syntect::easy::HighlightLines;
use syntect::highlighting::ThemeSet;
use syntect::parsing::SyntaxSet;
use syntect::util::as_24_bit_terminal_escaped;

static SYNTAX_SET: LazyLock<SyntaxSet> = LazyLock::new(SyntaxSet::load_defaults_newlines);
static THEME_SET: LazyLock<ThemeSet> = LazyLock::new(ThemeSet::load_defaults);

/// Syntax-highlight a single line of code. Returns the line with ANSI escapes, or the plain line on failure.
fn highlight_line(line: &str, highlighter: &mut Option<HighlightLines<'_>>) -> String {
    if let Some(h) = highlighter.as_mut() {
        if let Ok(ranges) = h.highlight_line(line, &SYNTAX_SET) {
            return as_24_bit_terminal_escaped(&ranges, false);
        }
    }
    line.to_string()
}

/// Try to create a syntax highlighter for the given file path.
fn make_highlighter(path: &str) -> Option<HighlightLines<'static>> {
    let ext = std::path::Path::new(path).extension()?.to_str()?;
    let syntax = SYNTAX_SET.find_syntax_by_extension(ext)?;
    let theme = &THEME_SET.themes["base16-ocean.dark"];
    Some(HighlightLines::new(syntax, theme))
}

/// Display tool result after execution.
fn display_result(tool_name: &str, result: &Value) {
    match tool_name {
        "execute_bash" => {
            let stdout = result.get("stdout").and_then(|v| v.as_str()).unwrap_or("");
            let stderr = result.get("stderr").and_then(|v| v.as_str()).unwrap_or("");
            let status = result.get("status").and_then(|v| v.as_str()).unwrap_or("");
            if !stdout.is_empty() {
                eprint!("{stdout}");
                if !stdout.ends_with('\n') {
                    eprintln!();
                }
            }
            if !stderr.is_empty() {
                eprint!("{RED}{stderr}{RESET}");
                if !stderr.ends_with('\n') {
                    eprintln!();
                }
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
        Arc::new(Self {
            inner: tool,
            display_only: false,
        })
    }

    /// Wrap a tool in display-only mode: shows what it's doing but auto-approves.
    pub fn wrap_display_only(tool: Arc<dyn Tool>) -> Arc<dyn Tool> {
        Arc::new(Self {
            inner: tool,
            display_only: true,
        })
    }

    /// Execute inner tool and display the result.
    async fn execute_and_display(
        &self,
        ctx: Arc<dyn ToolContext>,
        args: Value,
    ) -> adk_rust::Result<Value> {
        let result = self.inner.execute(ctx, args).await?;
        display_result(self.inner.name(), &result);
        Ok(result)
    }
}

/// Format a file diff for the confirmation dialog with syntax highlighting.
fn format_fs_write_diff(args: &Value) -> String {
    let path = args.get("path").and_then(|v| v.as_str()).unwrap_or("?");
    let mode = args
        .get("mode")
        .and_then(|v| v.as_str())
        .unwrap_or("create");

    let mut out = format!("{BOLD}{CYAN}{path}{RESET}\n");
    let mut hl = make_highlighter(path);

    match mode {
        "create" | "overwrite" => {
            let new_content = args.get("content").and_then(|v| v.as_str()).unwrap_or("");
            let old_content = if mode == "overwrite" {
                std::fs::read_to_string(path).unwrap_or_default()
            } else {
                String::new()
            };
            out.push_str(&render_diff(&old_content, new_content, &mut hl));
        }
        "append" => {
            let content = args.get("content").and_then(|v| v.as_str()).unwrap_or("");
            out.push_str(&format!("{DIM}  ... existing content ...{RESET}\n"));
            for line in content.lines() {
                let hl_line = highlight_line(line, &mut hl);
                out.push_str(&format!(
                    "{BG_GUTTER_INSERT} + {RESET}{BG_INSERT} {hl_line}{RESET}{CLEAR_LINE}\n"
                ));
            }
        }
        "patch" => {
            if let Some(patch) = args.get("patch") {
                let find = patch.get("find").and_then(|v| v.as_str()).unwrap_or("");
                let replace = patch.get("replace").and_then(|v| v.as_str()).unwrap_or("");
                out.push_str(&render_diff(find, replace, &mut hl));
            }
        }
        _ => {
            let pretty = serde_json::to_string_pretty(args).unwrap_or_else(|_| args.to_string());
            out.push_str(&format!("{DIM}{pretty}{RESET}\n"));
        }
    }

    out
}

/// Render a unified diff between old and new text with syntax highlighting and line numbers.
fn render_diff(old: &str, new: &str, hl: &mut Option<HighlightLines<'_>>) -> String {
    use similar::{ChangeTag, TextDiff};

    let diff = TextDiff::from_lines(old, new);
    let mut out = String::new();

    // Compute max line number width for gutter alignment
    let max_line = old.lines().count().max(new.lines().count()) + 1;
    let width = max_line.to_string().len().max(1);

    let mut old_line = 1usize;
    let mut new_line = 1usize;

    for change in diff.iter_all_changes() {
        let text = change.value().trim_end_matches('\n');
        let hl_text = highlight_line(text, hl);

        match change.tag() {
            ChangeTag::Delete => {
                out.push_str(&format!(
                    "{BG_GUTTER_DELETE} - {old_line:>width$}    {RESET}{BG_DELETE} {hl_text}{RESET}{CLEAR_LINE}\n"
                ));
                old_line += 1;
            }
            ChangeTag::Insert => {
                out.push_str(&format!(
                    "{BG_GUTTER_INSERT} +    {new_line:>width$} {RESET}{BG_INSERT} {hl_text}{RESET}{CLEAR_LINE}\n"
                ));
                new_line += 1;
            }
            ChangeTag::Equal => {
                out.push_str(&format!(
                    "{DIM}   {old_line:>width$}, {new_line:>width$} {RESET} {hl_text}{RESET}{CLEAR_LINE}\n"
                ));
                old_line += 1;
                new_line += 1;
            }
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
            format!(
                "{BOLD}{CYAN}{}{RESET} {}\n",
                self.inner.name(),
                format_tool_args(&args)
            )
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

        // Auto-approve read-only shell commands (git status, ls, grep, etc.)
        if self.inner.name() == "execute_bash" {
            if let Some(cmd) = args.get("command").and_then(|v| v.as_str()) {
                if crate::tools::execute_bash::is_read_only_command(cmd) {
                    theme::resume_spinner();
                    let mut approved_args = args;
                    if let Some(obj) = approved_args.as_object_mut() {
                        obj.insert("approved".to_string(), Value::Bool(true));
                    }
                    return self.execute_and_display(ctx, approved_args).await;
                }
            }
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
                TRUSTED_TOOLS
                    .lock()
                    .unwrap()
                    .insert(self.inner.name().to_string());
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
