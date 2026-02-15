use std::io;

use serde_json::{Value, json};
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitHubOpsError {
    pub code: &'static str,
    pub message: String,
}

impl GitHubOpsError {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitHubCliOutput {
    pub success: bool,
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
}

pub fn github_ops_error_payload(action: &str, err: GitHubOpsError) -> Value {
    json!({
        "status": "error",
        "kind": "github_ops",
        "action": action,
        "code": err.code,
        "error": err.message
    })
}

pub fn github_token_present() -> bool {
    ["GH_TOKEN", "GITHUB_TOKEN"].iter().any(|key| {
        std::env::var(key)
            .map(|value| !value.trim().is_empty())
            .unwrap_or(false)
    })
}

pub fn parse_required_string_arg(args: &Value, key: &str) -> Result<String, GitHubOpsError> {
    args.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .ok_or_else(|| GitHubOpsError::new("invalid_args", format!("'{key}' is required")))
}

pub fn parse_optional_string_arg(args: &Value, key: &str) -> Option<String> {
    args.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

pub fn parse_optional_string_list(args: &Value, key: &str) -> Result<Vec<String>, GitHubOpsError> {
    let Some(raw_value) = args.get(key) else {
        return Ok(Vec::new());
    };

    let Some(values) = raw_value.as_array() else {
        return Err(GitHubOpsError::new(
            "invalid_args",
            format!("'{key}' must be an array of strings"),
        ));
    };

    Ok(values
        .iter()
        .filter_map(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .collect::<Vec<String>>())
}

pub fn build_github_ops_command(args: &Value) -> Result<(String, Vec<String>), GitHubOpsError> {
    let action = parse_required_string_arg(args, "action")?.to_ascii_lowercase();
    match action.as_str() {
        "issue_create" => {
            let repo = parse_required_string_arg(args, "repo")?;
            let title = parse_required_string_arg(args, "title")?;
            let body = parse_required_string_arg(args, "body")?;
            let labels = parse_optional_string_list(args, "labels")?;

            let mut command = vec![
                "issue".to_string(),
                "create".to_string(),
                "--repo".to_string(),
                repo,
                "--title".to_string(),
                title,
                "--body".to_string(),
                body,
            ];
            for label in labels {
                command.push("--label".to_string());
                command.push(label);
            }
            Ok((action, command))
        }
        "issue_update" => {
            let repo = parse_required_string_arg(args, "repo")?;
            let issue_number = parse_required_string_arg(args, "issue_number")?;
            let state = parse_optional_string_arg(args, "state")
                .map(|value| value.to_ascii_lowercase())
                .unwrap_or_default();
            if state == "closed" {
                return Ok((
                    action,
                    vec![
                        "issue".to_string(),
                        "close".to_string(),
                        issue_number,
                        "--repo".to_string(),
                        repo,
                    ],
                ));
            }
            if state == "open" {
                return Ok((
                    action,
                    vec![
                        "issue".to_string(),
                        "reopen".to_string(),
                        issue_number,
                        "--repo".to_string(),
                        repo,
                    ],
                ));
            }

            let title = parse_optional_string_arg(args, "title");
            let body = parse_optional_string_arg(args, "body");
            let add_labels = parse_optional_string_list(args, "add_labels")?;
            let remove_labels = parse_optional_string_list(args, "remove_labels")?;
            if title.is_none()
                && body.is_none()
                && add_labels.is_empty()
                && remove_labels.is_empty()
            {
                return Err(GitHubOpsError::new(
                    "invalid_args",
                    "issue_update requires at least one of title/body/add_labels/remove_labels/state",
                ));
            }

            let mut command = vec![
                "issue".to_string(),
                "edit".to_string(),
                issue_number,
                "--repo".to_string(),
                repo,
            ];
            if let Some(title) = title {
                command.push("--title".to_string());
                command.push(title);
            }
            if let Some(body) = body {
                command.push("--body".to_string());
                command.push(body);
            }
            for label in add_labels {
                command.push("--add-label".to_string());
                command.push(label);
            }
            for label in remove_labels {
                command.push("--remove-label".to_string());
                command.push(label);
            }
            Ok((action, command))
        }
        "pr_create" => {
            let repo = parse_required_string_arg(args, "repo")?;
            let title = parse_required_string_arg(args, "title")?;
            let body = parse_required_string_arg(args, "body")?;
            let head = parse_optional_string_arg(args, "head");
            let base = parse_optional_string_arg(args, "base");
            let draft = args.get("draft").and_then(Value::as_bool).unwrap_or(true);

            let mut command = vec![
                "pr".to_string(),
                "create".to_string(),
                "--repo".to_string(),
                repo,
                "--title".to_string(),
                title,
                "--body".to_string(),
                body,
            ];
            if draft {
                command.push("--draft".to_string());
            }
            if let Some(head) = head {
                command.push("--head".to_string());
                command.push(head);
            }
            if let Some(base) = base {
                command.push("--base".to_string());
                command.push(base);
            }
            Ok((action, command))
        }
        "project_item_update" => {
            let project_id = parse_required_string_arg(args, "project_id")?;
            let item_id = parse_required_string_arg(args, "item_id")?;
            let field_id = parse_required_string_arg(args, "field_id")?;
            let status_option_id = parse_required_string_arg(args, "status_option_id")?;
            Ok((
                action,
                vec![
                    "project".to_string(),
                    "item-edit".to_string(),
                    "--project-id".to_string(),
                    project_id,
                    "--id".to_string(),
                    item_id,
                    "--field-id".to_string(),
                    field_id,
                    "--single-select-option-id".to_string(),
                    status_option_id,
                ],
            ))
        }
        _ => Err(GitHubOpsError::new(
            "invalid_args",
            "action must be one of: issue_create, issue_update, pr_create, project_item_update",
        )),
    }
}

pub fn run_gh_command(args: &[String]) -> Result<GitHubCliOutput, GitHubOpsError> {
    let output = std::process::Command::new("gh")
        .args(args)
        .output()
        .map_err(|err| {
            if err.kind() == io::ErrorKind::NotFound {
                GitHubOpsError::new(
                    "gh_missing",
                    "GitHub CLI 'gh' was not found. Install gh and retry.",
                )
            } else {
                GitHubOpsError::new("io_error", format!("failed to run gh command: {err}"))
            }
        })?;

    Ok(GitHubCliOutput {
        success: output.status.success(),
        exit_code: output.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&output.stdout).to_string(),
        stderr: String::from_utf8_lossy(&output.stderr).to_string(),
    })
}

pub fn github_ops_tool_response_with_runner<F>(
    args: &Value,
    token_present: bool,
    mut runner: F,
) -> Value
where
    F: FnMut(&[String]) -> Result<GitHubCliOutput, GitHubOpsError>,
{
    let (action, command) = match build_github_ops_command(args) {
        Ok(parsed) => parsed,
        Err(err) => return github_ops_error_payload("unknown", err),
    };

    if !token_present {
        let auth_command = vec!["auth".to_string(), "status".to_string()];
        let auth_ok = runner(&auth_command)
            .map(|out| out.success)
            .unwrap_or(false);
        if !auth_ok {
            return github_ops_error_payload(
                &action,
                GitHubOpsError::new(
                    "auth_required",
                    "GitHub auth not detected. Set GH_TOKEN/GITHUB_TOKEN or run `gh auth login`.",
                ),
            );
        }
    }

    match runner(&command) {
        Ok(output) => {
            if output.success {
                json!({
                    "status": "ok",
                    "kind": "github_ops",
                    "action": action,
                    "command": format!("gh {}", command.join(" ")),
                    "exit_code": output.exit_code,
                    "stdout": output.stdout,
                    "stderr": output.stderr
                })
            } else {
                json!({
                    "status": "error",
                    "kind": "github_ops",
                    "action": action,
                    "code": "github_command_failed",
                    "error": format!("gh command exited with non-zero status: {}", output.exit_code),
                    "command": format!("gh {}", command.join(" ")),
                    "exit_code": output.exit_code,
                    "stdout": output.stdout,
                    "stderr": output.stderr
                })
            }
        }
        Err(err) => github_ops_error_payload(&action, err),
    }
}

pub fn github_ops_tool_response(args: &Value) -> Value {
    github_ops_tool_response_with_runner(args, github_token_present(), run_gh_command)
}
