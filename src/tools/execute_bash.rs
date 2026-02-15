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
pub const EXECUTE_BASH_READ_ONLY_PREFIXES: &[&str] = &[
    "ls",
    "pwd",
    "cat ",
    "rg ",
    "grep ",
    "head ",
    "tail ",
    "wc ",
    "find ",
    "stat ",
    "git status",
    "git diff",
    "git log",
    "echo ",
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

pub fn execute_bash_error_payload(command: &str, err: ExecuteBashToolError, attempts: u32) -> Value {
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

pub fn parse_execute_bash_request(args: &Value) -> Result<ExecuteBashRequest, ExecuteBashToolError> {
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
    let normalized = command.trim().to_ascii_lowercase();
    if normalized.is_empty() {
        return false;
    }

    let has_read_only_prefix = EXECUTE_BASH_READ_ONLY_PREFIXES
        .iter()
        .any(|prefix| normalized == *prefix || normalized.starts_with(prefix));

    if !has_read_only_prefix {
        return false;
    }

    // Reject command chaining operators that could smuggle writes after a read-only prefix.
    if contains_command_chaining(&normalized) {
        return false;
    }

    true
}

pub fn contains_command_chaining(command: &str) -> bool {
    for pattern in &[";", "&&", "||", "|", "$(", "`"] {
        if command.contains(pattern) {
            return true;
        }
    }
    false
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

