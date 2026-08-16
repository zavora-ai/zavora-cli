use std::collections::HashSet;
use std::io::{self, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use adk_rust::prelude::*;
use serde_json::Value;

use crate::theme::{
    self, BG_DELETE, BG_GUTTER_DELETE, BG_GUTTER_INSERT, BG_INSERT, BOLD, CLEAR_LINE, CYAN, DIM,
    GREEN, RED, RESET,
};

/// Render a single line in the diff body. Kept as a helper so syntax highlighting can be reintroduced later.
fn highlight_line(line: &str, _highlighter: &mut Option<()>) -> String {
    line.to_string()
}

/// Placeholder for optional future syntax highlighter state.
fn make_highlighter(_path: &str) -> Option<()> {
    None
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
            if status == "error"
                && let Some(err) = result.get("error").and_then(|v| v.as_str())
            {
                eprintln!("{RED}{err}{RESET}");
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
static SESSION_RULES: std::sync::LazyLock<Mutex<crate::tool_policy::PermissionRules>> =
    std::sync::LazyLock::new(|| Mutex::new(crate::tool_policy::PermissionRules::default()));
static HEADLESS_MODE: AtomicBool = AtomicBool::new(false);

pub fn set_headless_mode(enabled: bool) {
    HEADLESS_MODE.store(enabled, Ordering::SeqCst);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApprovalDecision {
    AllowOnce,
    TrustSession,
    Deny,
}

pub struct ApprovalRequest {
    pub tool: String,
    pub detail: String,
    pub response: tokio::sync::oneshot::Sender<ApprovalDecision>,
}

static APPROVAL_SENDER: std::sync::LazyLock<
    Mutex<Option<tokio::sync::mpsc::UnboundedSender<ApprovalRequest>>>,
> = std::sync::LazyLock::new(|| Mutex::new(None));

pub fn install_approval_bridge() -> tokio::sync::mpsc::UnboundedReceiver<ApprovalRequest> {
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    *APPROVAL_SENDER.lock().unwrap() = Some(tx);
    rx
}

pub fn clear_approval_bridge() {
    *APPROVAL_SENDER.lock().unwrap() = None;
}

fn tui_active() -> bool {
    APPROVAL_SENDER.lock().unwrap().is_some()
}

/// Trust a tool for the remainder of the session.
pub fn trust_tool(name: &str) {
    TRUSTED_TOOLS.lock().unwrap().insert(name.to_string());
    SESSION_RULES
        .lock()
        .unwrap()
        .always_allow
        .push(crate::tool_policy::ToolPattern(name.to_string()));
}

/// Deny a tool or tool-content pattern for the remainder of the session.
pub fn deny_tool(pattern: &str) {
    SESSION_RULES
        .lock()
        .unwrap()
        .always_deny
        .push(crate::tool_policy::ToolPattern(pattern.to_string()));
}

/// Arguments a model must never be able to set, because they relax a safety
/// decision this wrapper is responsible for making.
///
/// `approved` exists so the wrapper can record a human's yes; `allow_dangerous`
/// exists so a human can override a denied pattern. Both are legitimate — from
/// the enforcement layer, not from the model.
pub const MODEL_FORBIDDEN_SAFETY_ARGS: &[&str] = &["approved", "allow_dangerous"];

/// Lifecycle hooks for the pre/post-tool stage of the enforcement pipeline.
///
/// A process-global, matching how trust rules, the approval bridge, and headless
/// mode are already installed here. The executor is set once when the tool
/// surface is sealed, so every wrapped tool sees the same hooks.
static HOOK_EXECUTOR: Mutex<Option<Arc<crate::hooks::HookExecutor>>> = Mutex::new(None);

/// Install the hook executor for this process. Called during tool-surface seal.
pub fn install_hook_executor(executor: Arc<crate::hooks::HookExecutor>) {
    *HOOK_EXECUTOR.lock().unwrap_or_else(|err| err.into_inner()) = Some(executor);
}

/// Remove any installed hook executor. Used by tests and by runtime rebuilds.
pub fn clear_hook_executor() {
    *HOOK_EXECUTOR.lock().unwrap_or_else(|err| err.into_inner()) = None;
}

fn hook_executor() -> Option<Arc<crate::hooks::HookExecutor>> {
    HOOK_EXECUTOR
        .lock()
        .unwrap_or_else(|err| err.into_inner())
        .clone()
}

/// Remove safety-relaxing keys from a model-supplied argument object.
///
/// Requirement 7.6; Correctness Property 7: the enforcement decision for any
/// argument set must equal the decision for that set with these keys removed.
pub fn scrub_model_supplied_safety_args(mut args: Value) -> Value {
    if let Some(object) = args.as_object_mut() {
        for key in MODEL_FORBIDDEN_SAFETY_ARGS {
            if object.remove(*key).is_some() {
                tracing::debug!(
                    argument = key,
                    "stripped model-supplied safety argument before policy evaluation"
                );
            }
        }
    }
    args
}

fn session_permission_decision(
    tool_name: &str,
    content: Option<&str>,
) -> crate::tool_policy::PermissionDecision {
    SESSION_RULES.lock().unwrap().evaluate(tool_name, content)
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
    ///
    /// This is the single place a wrapped tool actually runs, so it is where the
    /// pre/post-tool hook stage belongs: after the approval decision, around the
    /// call. A `pre_tool` hook exiting with `HOOK_EXIT_BLOCK` stops the call.
    /// Requirement 7.8.
    async fn execute_and_display(
        &self,
        ctx: Arc<dyn ToolContext>,
        args: Value,
    ) -> adk_rust::Result<Value> {
        let executor = hook_executor().filter(|executor| !executor.is_empty());

        if let Some(executor) = executor.as_ref() {
            let tool_ctx = crate::hooks::HookToolContext {
                tool_name: self.inner.name().to_string(),
                tool_input: args.clone(),
                tool_response: None,
            };
            let results = executor
                .run(crate::hooks::HookPoint::PreTool, None, Some(&tool_ctx))
                .await;
            if let Some(block) = results.iter().find(|result| result.is_block()) {
                tracing::info!(
                    tool = self.inner.name(),
                    hook = %block.command,
                    "pre_tool hook blocked the call"
                );
                return Ok(serde_json::json!({
                    "error": format!(
                        "Tool '{}' blocked by pre_tool hook: {}",
                        self.inner.name(),
                        if block.output.trim().is_empty() {
                            block.command.as_str()
                        } else {
                            block.output.trim()
                        }
                    )
                }));
            }
        }

        let result = self.inner.execute(ctx, args.clone()).await?;

        if let Some(executor) = executor.as_ref() {
            let tool_ctx = crate::hooks::HookToolContext {
                tool_name: self.inner.name().to_string(),
                tool_input: args,
                tool_response: Some(result.clone()),
            };
            // post_tool cannot veto a call that already ran; results are
            // observational.
            let _ = executor
                .run(crate::hooks::HookPoint::PostTool, None, Some(&tool_ctx))
                .await;
        }

        if !tui_active() {
            display_result(self.inner.name(), &result);
        }
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

/// Render a unified diff between old and new text with line numbers.
fn render_diff(old: &str, new: &str, hl: &mut Option<()>) -> String {
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
    format!("{DIM}{}{RESET}", crate::text::truncate(&pretty, 400, "..."))
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

    /// Forwarded so wrapping does not erase the tool's declared capability.
    ///
    /// The trait defaults are both `false`, so before this existed every
    /// wrapped tool reported itself read-write and concurrency-unsafe. That
    /// silently disabled ADK's parallel tool execution for the entire runtime
    /// and forced policy to consult a name list instead. Requirement 7.3.
    fn is_read_only(&self) -> bool {
        self.inner.is_read_only()
    }

    fn is_concurrency_safe(&self) -> bool {
        self.inner.is_concurrency_safe()
    }

    async fn execute(&self, ctx: Arc<dyn ToolContext>, args: Value) -> adk_rust::Result<Value> {
        // Scrub model-supplied safety arguments before anything reads them.
        //
        // `execute_bash` accepts `approved` and `allow_dangerous` so that this
        // wrapper can grant approval after a human says yes. Nothing stops a
        // model from setting them itself, which would let it overrule a Deny
        // verdict the enforcement layer already reached. Stripping them here
        // means the decision is a function of the request alone.
        // Requirement 7.6; Correctness Property 7.
        let args = scrub_model_supplied_safety_args(args);

        let content = match self.inner.name() {
            "execute_bash" => args.get("command").and_then(Value::as_str),
            "fs_read" | "fs_write" | "file_edit" => args.get("path").and_then(Value::as_str),
            _ => None,
        };
        let session_decision = session_permission_decision(self.inner.name(), content);
        if session_decision == crate::tool_policy::PermissionDecision::Deny {
            return Ok(serde_json::json!({
                "error": format!("Tool '{}' denied by session policy", self.inner.name())
            }));
        }
        let trusted = TRUSTED_TOOLS.lock().unwrap().contains(self.inner.name())
            || session_decision == crate::tool_policy::PermissionDecision::Allow;

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

        if !tui_active() {
            eprint!("{display}");
        }

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
        if self.inner.name() == "execute_bash"
            && let Some(cmd) = args.get("command").and_then(|v| v.as_str())
            && crate::tools::execute_bash::is_read_only_command(cmd)
        {
            theme::resume_spinner();
            let mut approved_args = args;
            if let Some(obj) = approved_args.as_object_mut() {
                obj.insert("approved".to_string(), Value::Bool(true));
            }
            return self.execute_and_display(ctx, approved_args).await;
        }

        if HEADLESS_MODE.load(Ordering::SeqCst) {
            theme::resume_spinner();
            return Ok(serde_json::json!({
                "error": format!(
                    "Tool '{}' requires approval in headless mode; use --approve-tool {} or --always-approve",
                    self.inner.name(),
                    self.inner.name()
                )
            }));
        }

        let approval_sender = APPROVAL_SENDER.lock().unwrap().clone();
        if let Some(sender) = approval_sender {
            let detail = if self.inner.name() == "execute_bash" {
                args.get("command")
                    .and_then(Value::as_str)
                    .unwrap_or("?")
                    .to_string()
            } else {
                crate::text::truncate(&args.to_string(), 300, "…")
            };
            let (response_tx, response_rx) = tokio::sync::oneshot::channel();
            if sender
                .send(ApprovalRequest {
                    tool: self.inner.name().to_string(),
                    detail,
                    response: response_tx,
                })
                .is_err()
            {
                return Ok(serde_json::json!({"error": "approval interface unavailable"}));
            }
            match response_rx.await.unwrap_or(ApprovalDecision::Deny) {
                ApprovalDecision::TrustSession => {
                    trust_tool(self.inner.name());
                    let mut approved_args = args;
                    if let Some(obj) = approved_args.as_object_mut() {
                        obj.insert("approved".to_string(), Value::Bool(true));
                    }
                    return self.execute_and_display(ctx, approved_args).await;
                }
                ApprovalDecision::AllowOnce => {
                    let mut approved_args = args;
                    if let Some(obj) = approved_args.as_object_mut() {
                        obj.insert("approved".to_string(), Value::Bool(true));
                    }
                    return self.execute_and_display(ctx, approved_args).await;
                }
                ApprovalDecision::Deny => {
                    return Ok(serde_json::json!({
                        "error": format!("Tool '{}' denied by user", self.inner.name())
                    }));
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_deny_rules_override_allow_rules_and_match_content() {
        trust_tool("test_shell:git *");
        deny_tool("test_shell:git push*");

        assert_eq!(
            session_permission_decision("test_shell", Some("git status")),
            crate::tool_policy::PermissionDecision::Allow
        );
        assert_eq!(
            session_permission_decision("test_shell", Some("git push origin main")),
            crate::tool_policy::PermissionDecision::Deny
        );
    }
}
