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
    "env",
    "printenv",
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

pub async fn run_execute_bash_once(
    command: &str,
    timeout_secs: u64,
) -> Result<std::process::Output, ExecuteBashToolError> {
    let child = tokio::process::Command::new("sh")
        .arg("-lc")
        .arg(command)
        .output();
    match tokio::time::timeout(Duration::from_secs(timeout_secs), child).await {
        Ok(result) => result
            .map_err(|_| ExecuteBashToolError::new("io_error", "failed to launch shell command")),
        Err(_) => Err(ExecuteBashToolError::new(
            "timeout",
            format!("command timed out after {timeout_secs}s"),
        )),
    }
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
