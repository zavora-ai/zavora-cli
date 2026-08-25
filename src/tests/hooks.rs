//! Hook parsing, matching, execution, blocking, ordering, and timeout tests.

use super::*;

// ---------------------------------------------------------------------------
// Hook lifecycle tests
// ---------------------------------------------------------------------------

use crate::hooks::*;

#[test]
fn test_hook_point_display() {
    assert_eq!(HookPoint::AgentSpawn.to_string(), "agent_spawn");
    assert_eq!(HookPoint::PreTool.to_string(), "pre_tool");
    assert_eq!(HookPoint::PostTool.to_string(), "post_tool");
    assert_eq!(HookPoint::PromptSubmit.to_string(), "prompt_submit");
    assert_eq!(HookPoint::Stop.to_string(), "stop");
}

#[test]
fn test_parse_hooks_map() {
    let mut raw = HashMap::new();
    raw.insert(
        "pre_tool".to_string(),
        vec![HookConfig {
            command: "echo check".to_string(),
            timeout_ms: 5000,
            max_output: 1024,
            matcher: Some("fs_*".to_string()),
        }],
    );
    raw.insert(
        "stop".to_string(),
        vec![HookConfig {
            command: "echo done".to_string(),
            timeout_ms: 5000,
            max_output: 1024,
            matcher: None,
        }],
    );
    raw.insert("unknown_point".to_string(), vec![]);

    let parsed = parse_hooks_map(&raw);
    assert!(parsed.contains_key(&HookPoint::PreTool));
    assert!(parsed.contains_key(&HookPoint::Stop));
    assert!(!parsed.contains_key(&HookPoint::AgentSpawn));
    assert_eq!(parsed[&HookPoint::PreTool].len(), 1);
    assert_eq!(
        parsed[&HookPoint::PreTool][0].matcher.as_deref(),
        Some("fs_*")
    );
}

#[tokio::test]
async fn test_hook_executor_fires_at_point() {
    let mut hooks = HashMap::new();
    hooks.insert(
        HookPoint::AgentSpawn,
        vec![HookConfig {
            command: "echo spawned".to_string(),
            timeout_ms: 5000,
            max_output: 1024,
            matcher: None,
        }],
    );
    let executor = HookExecutor::new(hooks);
    let results = executor.run(HookPoint::AgentSpawn, None, None).await;
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].exit_code, 0);
    assert!(results[0].output.contains("spawned"));
}

#[tokio::test]
async fn test_hook_matcher_scoping() {
    let mut hooks = HashMap::new();
    hooks.insert(
        HookPoint::PreTool,
        vec![HookConfig {
            command: "echo matched".to_string(),
            timeout_ms: 5000,
            max_output: 1024,
            matcher: Some("fs_*".to_string()),
        }],
    );
    let executor = HookExecutor::new(hooks);

    // Should fire for fs_write
    let ctx_match = HookToolContext {
        tool_name: "fs_write".to_string(),
        tool_input: json!({}),
        tool_response: None,
    };
    let results = executor
        .run(HookPoint::PreTool, None, Some(&ctx_match))
        .await;
    assert_eq!(results.len(), 1);

    // Should NOT fire for execute_bash
    let ctx_no_match = HookToolContext {
        tool_name: "execute_bash".to_string(),
        tool_input: json!({}),
        tool_response: None,
    };
    let results = executor
        .run(HookPoint::PreTool, None, Some(&ctx_no_match))
        .await;
    assert_eq!(results.len(), 0);
}

#[tokio::test]
async fn test_pre_tool_block_prevents_execution() {
    let mut hooks = HashMap::new();
    hooks.insert(
        HookPoint::PreTool,
        vec![HookConfig {
            command: "echo 'blocked' >&2; exit 2".to_string(),
            timeout_ms: 5000,
            max_output: 1024,
            matcher: None,
        }],
    );
    let executor = HookExecutor::new(hooks);

    let ctx = HookToolContext {
        tool_name: "fs_write".to_string(),
        tool_input: json!({"path": "/etc/passwd"}),
        tool_response: None,
    };
    let (blocked, results) = executor.run_pre_tool(&ctx).await;
    assert!(blocked);
    assert_eq!(results[0].exit_code, HOOK_EXIT_BLOCK);
    assert!(results[0].output.contains("blocked"));
}

#[tokio::test]
async fn test_hook_execution_order() {
    let mut hooks = HashMap::new();
    hooks.insert(
        HookPoint::PreTool,
        vec![
            HookConfig {
                command: "echo first".to_string(),
                timeout_ms: 5000,
                max_output: 1024,
                matcher: None,
            },
            HookConfig {
                command: "echo second".to_string(),
                timeout_ms: 5000,
                max_output: 1024,
                matcher: None,
            },
        ],
    );
    let executor = HookExecutor::new(hooks);

    let ctx = HookToolContext {
        tool_name: "fs_read".to_string(),
        tool_input: json!({}),
        tool_response: None,
    };
    let results = executor.run(HookPoint::PreTool, None, Some(&ctx)).await;
    assert_eq!(results.len(), 2);
    assert!(results[0].output.contains("first"));
    assert!(results[1].output.contains("second"));
}

#[tokio::test]
async fn test_hook_timeout() {
    let mut hooks = HashMap::new();
    hooks.insert(
        HookPoint::Stop,
        vec![HookConfig {
            command: "sleep 10".to_string(),
            timeout_ms: 100,
            max_output: 1024,
            matcher: None,
        }],
    );
    let executor = HookExecutor::new(hooks);
    let results = executor.run(HookPoint::Stop, None, None).await;
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].exit_code, -1);
    assert!(results[0].output.contains("timed out"));
}

#[tokio::test]
async fn test_hook_empty_executor() {
    let executor = HookExecutor::new(HashMap::new());
    assert!(executor.is_empty());
    let results = executor.run(HookPoint::AgentSpawn, None, None).await;
    assert!(results.is_empty());
}
