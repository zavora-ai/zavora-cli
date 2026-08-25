//! MCP authentication, reachability, and selection diagnostics.

use super::*;

// ---------------------------------------------------------------------------
// MCP diagnostics tests
// ---------------------------------------------------------------------------

#[test]
fn test_check_auth_hint_missing_env() {
    let server = McpServerConfig {
        name: "test-server".to_string(),
        endpoint: "https://mcp.example.com".to_string(),
        enabled: Some(true),
        timeout_secs: Some(10),
        auth_bearer_env: Some("ZAVORA_TEST_NONEXISTENT_TOKEN_XYZ".to_string()),
        tool_allowlist: vec![],
        tool_aliases: HashMap::new(),
        command: None,
        args: vec![],
        env: HashMap::new(),
        oauth: None,
    };
    let hint = check_auth_hint(&server);
    assert!(hint.is_some());
    assert!(hint.unwrap().contains("is not set"));
}

#[test]
fn test_check_auth_hint_no_auth() {
    let server = McpServerConfig {
        name: "test-server".to_string(),
        endpoint: "https://mcp.example.com".to_string(),
        enabled: Some(true),
        timeout_secs: Some(10),
        auth_bearer_env: None,
        tool_allowlist: vec![],
        tool_aliases: HashMap::new(),
        command: None,
        args: vec![],
        env: HashMap::new(),
        oauth: None,
    };
    let hint = check_auth_hint(&server);
    assert!(hint.is_none());
}

#[tokio::test]
async fn test_diagnose_mcp_server_auth_failure() {
    let server = McpServerConfig {
        name: "auth-fail".to_string(),
        endpoint: "https://mcp.example.com".to_string(),
        enabled: Some(true),
        timeout_secs: Some(5),
        auth_bearer_env: Some("ZAVORA_TEST_NONEXISTENT_TOKEN_XYZ".to_string()),
        tool_allowlist: vec![],
        tool_aliases: HashMap::new(),
        command: None,
        args: vec![],
        env: HashMap::new(),
        oauth: None,
    };
    let diag = diagnose_mcp_server(&server, 1, 100).await;
    assert_eq!(diag.name, "auth-fail");
    assert!(matches!(diag.state, McpServerState::AuthFailure { .. }));
}

#[tokio::test]
async fn test_diagnose_mcp_server_unreachable() {
    let server = McpServerConfig {
        name: "bad-endpoint".to_string(),
        endpoint: "http://127.0.0.1:1".to_string(),
        enabled: Some(true),
        timeout_secs: Some(2),
        auth_bearer_env: None,
        tool_allowlist: vec![],
        tool_aliases: HashMap::new(),
        command: None,
        args: vec![],
        env: HashMap::new(),
        oauth: None,
    };
    let diag = diagnose_mcp_server(&server, 1, 100).await;
    assert_eq!(diag.name, "bad-endpoint");
    assert!(matches!(
        diag.state,
        McpServerState::Unreachable { .. } | McpServerState::Timeout { .. }
    ));
}

#[test]
fn test_select_mcp_servers_filters_disabled() {
    let cfg = base_cfg();
    // base_cfg has no MCP servers, so should return empty
    let servers = select_mcp_servers(&cfg, None).unwrap();
    assert!(servers.is_empty());
}

#[test]
fn test_select_mcp_servers_by_name_not_found() {
    let cfg = base_cfg();
    let result = select_mcp_servers(&cfg, Some("nonexistent"));
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("not found"));
}
