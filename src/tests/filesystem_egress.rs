//! Filesystem, shell-policy, and GitHub egress tests.

use super::*;

#[test]
fn fs_read_reads_allowed_file_content() {
    let dir = tempdir().expect("temp directory should create");
    std::fs::write(dir.path().join("notes.txt"), "alpha\nbeta\ngamma\n")
        .expect("fixture file should write");
    let workspace_root = dir
        .path()
        .canonicalize()
        .expect("workspace root should resolve");

    let payload = fs_read_tool_response_with_root(
        &json!({
            "path": "notes.txt",
            "start_line": 2,
            "max_lines": 1
        }),
        &workspace_root,
    );

    assert_eq!(payload["status"], "ok");
    assert_eq!(payload["kind"], "file");
    assert_eq!(payload["content"], "beta");
    assert_eq!(payload["line_count"], 1);
}

#[test]
fn fs_read_lists_directory_entries() {
    let dir = tempdir().expect("temp directory should create");
    std::fs::create_dir_all(dir.path().join("docs")).expect("fixture dir should create");
    std::fs::write(dir.path().join("README.md"), "hello").expect("fixture file should write");
    let workspace_root = dir
        .path()
        .canonicalize()
        .expect("workspace root should resolve");

    let payload = fs_read_tool_response_with_root(
        &json!({
            "path": ".",
            "max_entries": 10
        }),
        &workspace_root,
    );

    assert_eq!(payload["status"], "ok");
    assert_eq!(payload["kind"], "directory");
    assert_eq!(payload["entry_count"], 2);
    let entries = payload["entries"]
        .as_array()
        .expect("entries should be an array");
    assert!(
        entries
            .iter()
            .any(|entry| entry.get("name") == Some(&Value::String("README.md".to_string())))
    );
    assert!(
        entries
            .iter()
            .any(|entry| entry.get("name") == Some(&Value::String("docs".to_string())))
    );
}

#[test]
fn fs_read_denies_blocked_paths() {
    let dir = tempdir().expect("temp directory should create");
    std::fs::write(dir.path().join(".env"), "OPENAI_API_KEY=test").expect("fixture file");
    let workspace_root = dir
        .path()
        .canonicalize()
        .expect("workspace root should resolve");

    let payload = fs_read_tool_response_with_root(&json!({ "path": ".env" }), &workspace_root);
    assert_eq!(payload["status"], "error");
    assert_eq!(payload["code"], "denied_path");
    assert!(
        extract_tool_failure_message(&payload)
            .as_deref()
            .unwrap_or_default()
            .contains("denied path")
    );
}

#[test]
fn fs_read_reports_invalid_paths() {
    let dir = tempdir().expect("temp directory should create");
    let workspace_root = dir
        .path()
        .canonicalize()
        .expect("workspace root should resolve");
    let payload =
        fs_read_tool_response_with_root(&json!({ "path": "missing.txt" }), &workspace_root);
    assert_eq!(payload["status"], "error");
    assert_eq!(payload["code"], "invalid_path");
}

#[test]
fn fs_write_creates_file_when_mode_is_create() {
    let dir = tempdir().expect("temp directory should create");
    let workspace_root = dir
        .path()
        .canonicalize()
        .expect("workspace root should resolve");

    let payload = fs_write_tool_response_with_root(
        &json!({
            "path": "docs/new.txt",
            "mode": "create",
            "content": "release-ready"
        }),
        &workspace_root,
    );

    assert_eq!(payload["status"], "ok");
    assert_eq!(payload["mode"], "create");
    let content = std::fs::read_to_string(dir.path().join("docs/new.txt"))
        .expect("created file should be readable");
    assert_eq!(content, "release-ready");
}

#[test]
fn fs_write_patch_updates_existing_content() {
    let dir = tempdir().expect("temp directory should create");
    let workspace_root = dir
        .path()
        .canonicalize()
        .expect("workspace root should resolve");
    std::fs::write(dir.path().join("plan.md"), "Ship alpha then beta")
        .expect("fixture file should write");

    let payload = fs_write_tool_response_with_root(
        &json!({
            "path": "plan.md",
            "mode": "patch",
            "patch": {
                "find": "beta",
                "replace": "rc"
            }
        }),
        &workspace_root,
    );

    assert_eq!(payload["status"], "ok");
    assert_eq!(payload["mode"], "patch");
    assert_eq!(payload["replaced_count"], 1);
    let content =
        std::fs::read_to_string(dir.path().join("plan.md")).expect("patched file should read");
    assert_eq!(content, "Ship alpha then rc");
}

#[test]
fn fs_write_denies_blocked_paths() {
    let dir = tempdir().expect("temp directory should create");
    let workspace_root = dir
        .path()
        .canonicalize()
        .expect("workspace root should resolve");

    let payload = fs_write_tool_response_with_root(
        &json!({
            "path": ".env",
            "mode": "overwrite",
            "content": "should-not-write"
        }),
        &workspace_root,
    );

    assert_eq!(payload["status"], "error");
    assert_eq!(payload["code"], "denied_path");
}

#[test]
fn fs_write_rejects_malformed_patch_requests() {
    let dir = tempdir().expect("temp directory should create");
    let workspace_root = dir
        .path()
        .canonicalize()
        .expect("workspace root should resolve");
    std::fs::write(dir.path().join("plan.md"), "alpha").expect("fixture file should write");

    let payload = fs_write_tool_response_with_root(
        &json!({
            "path": "plan.md",
            "mode": "patch",
            "patch": {
                "find": "",
                "replace": "beta"
            }
        }),
        &workspace_root,
    );

    assert_eq!(payload["status"], "error");
    assert_eq!(payload["code"], "malformed_edit");
}

#[test]
fn execute_bash_policy_denies_blocked_patterns_without_override() {
    let request = test_execute_bash_request("rm -rf .");
    let err = evaluate_execute_bash_policy(&request).expect_err("dangerous pattern should fail");
    assert_eq!(err.code, "denied_command");
}

#[test]
fn execute_bash_policy_allows_dangerous_override_when_approved() {
    let mut request = test_execute_bash_request("rm -rf ./tmp");
    request.allow_dangerous = true;
    request.approved = true;

    let decision = evaluate_execute_bash_policy(&request).expect("approved override should pass");
    assert!(!decision.read_only_auto_allow);
}

#[test]
fn execute_bash_policy_auto_allows_read_only_commands() {
    let request = test_execute_bash_request("git status");
    let decision = evaluate_execute_bash_policy(&request).expect("read-only should pass");
    assert!(decision.read_only_auto_allow);
}

#[tokio::test]
async fn execute_bash_retries_failed_commands_when_configured() {
    let payload = execute_bash_tool_response(&json!({
        "command": "false",
        "approved": true,
        "retry_attempts": 2,
        "retry_delay_ms": 0
    }))
    .await;

    assert_eq!(payload["status"], "error");
    assert_eq!(payload["code"], "command_failed");
    assert_eq!(payload["attempts"], 2);
}

#[test]
fn github_ops_issue_create_runs_expected_mocked_command() {
    let calls = std::cell::RefCell::new(Vec::<Vec<String>>::new());
    let payload = github_ops_tool_response_with_runner(
        &json!({
            "action": "issue_create",
            "repo": "zavora-ai/zavora-cli",
            "title": "Test issue",
            "body": "Issue body",
            "labels": ["bug", "sprint:8"]
        }),
        true,
        |args| {
            calls.borrow_mut().push(args.to_vec());
            Ok(GitHubCliOutput {
                success: true,
                exit_code: 0,
                stdout: "https://github.com/zavora-ai/zavora-cli/issues/999".to_string(),
                stderr: String::new(),
            })
        },
    );

    assert_eq!(payload["status"], "ok");
    assert_eq!(payload["action"], "issue_create");
    let calls = calls.borrow();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0][0], "issue");
    assert_eq!(calls[0][1], "create");
}

#[test]
fn github_ops_preflight_requires_auth_without_token() {
    let payload = github_ops_tool_response_with_runner(
        &json!({
            "action": "issue_create",
            "repo": "zavora-ai/zavora-cli",
            "title": "Needs auth",
            "body": "body"
        }),
        false,
        |_args| {
            Ok(GitHubCliOutput {
                success: false,
                exit_code: 1,
                stdout: String::new(),
                stderr: "not logged in".to_string(),
            })
        },
    );

    assert_eq!(payload["status"], "error");
    assert_eq!(payload["code"], "auth_required");
}

#[test]
fn github_ops_project_item_update_runs_expected_mocked_command() {
    let calls = std::cell::RefCell::new(Vec::<Vec<String>>::new());
    let payload = github_ops_tool_response_with_runner(
        &json!({
            "action": "project_item_update",
            "project_id": "PVT_kwDOBVKgdc4BPPxU",
            "item_id": "PVTI_lADOBVKgdc4BPPxUzglepjM",
            "field_id": "PVTSSF_lADOBVKgdc4BPPxUzg9te4w",
            "status_option_id": "98236657"
        }),
        true,
        |args| {
            calls.borrow_mut().push(args.to_vec());
            Ok(GitHubCliOutput {
                success: true,
                exit_code: 0,
                stdout: String::new(),
                stderr: String::new(),
            })
        },
    );

    assert_eq!(payload["status"], "ok");
    assert_eq!(payload["action"], "project_item_update");
    let calls = calls.borrow();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0][0], "project");
    assert_eq!(calls[0][1], "item-edit");
}
