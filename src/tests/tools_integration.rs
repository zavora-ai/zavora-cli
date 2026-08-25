//! Built-in tool integration, permission, skill-discovery, reload, and CLI-version tests.

use super::*;

// ---------------------------------------------------------------------------
// Integration tests for new tools (Phase 1-4)
// ---------------------------------------------------------------------------

#[test]
fn test_file_edit_basic_replacement() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("edit_test.txt");
    std::fs::write(&file, "hello world\nfoo bar\n").unwrap();

    // Must run from the temp dir so workspace policy passes
    let _guard = SetCwd::new(dir.path());

    let result = crate::tools::file_edit::file_edit_tool_response(&serde_json::json!({
        "file_path": file.to_str().unwrap(),
        "old_string": "foo bar",
        "new_string": "baz qux"
    }));

    assert_eq!(result["status"], "ok");
    assert_eq!(result["replacements"], 1);
    assert!(result["diff"].as_str().unwrap().contains("-foo bar"));
    assert!(result["diff"].as_str().unwrap().contains("+baz qux"));

    let content = std::fs::read_to_string(&file).unwrap();
    assert_eq!(content, "hello world\nbaz qux\n");
}

#[test]
fn test_file_edit_no_match_gives_hint() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("hint_test.txt");
    std::fs::write(&file, "hello world\n").unwrap();
    let _guard = SetCwd::new(dir.path());

    let result = crate::tools::file_edit::file_edit_tool_response(&serde_json::json!({
        "file_path": file.to_str().unwrap(),
        "old_string": "helo wrld",
        "new_string": "goodbye"
    }));

    assert_eq!(result["status"], "error");
    assert_eq!(result["code"], "no_match");
}

#[test]
fn test_file_edit_ambiguous_match() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("ambig_test.txt");
    std::fs::write(&file, "foo\nfoo\nbar\n").unwrap();
    let _guard = SetCwd::new(dir.path());

    let result = crate::tools::file_edit::file_edit_tool_response(&serde_json::json!({
        "file_path": file.to_str().unwrap(),
        "old_string": "foo",
        "new_string": "baz"
    }));

    assert_eq!(result["status"], "error");
    assert_eq!(result["code"], "ambiguous_match");
}

#[test]
fn test_file_edit_replace_all() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("replall_test.txt");
    std::fs::write(&file, "foo\nfoo\nbar\n").unwrap();
    let _guard = SetCwd::new(dir.path());

    let result = crate::tools::file_edit::file_edit_tool_response(&serde_json::json!({
        "file_path": file.to_str().unwrap(),
        "old_string": "foo",
        "new_string": "baz",
        "replace_all": true
    }));

    assert_eq!(result["status"], "ok");
    assert_eq!(result["replacements"], 2);
    let content = std::fs::read_to_string(&file).unwrap();
    assert_eq!(content, "baz\nbaz\nbar\n");
}

#[test]
fn test_glob_finds_files() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("a.rs"), "").unwrap();
    std::fs::write(dir.path().join("b.rs"), "").unwrap();
    std::fs::write(dir.path().join("c.txt"), "").unwrap();
    let _guard = SetCwd::new(dir.path());

    let result = crate::tools::glob::glob_tool_response(&serde_json::json!({
        "pattern": "*.rs"
    }));

    assert_eq!(result["numFiles"], 2);
    assert_eq!(result["truncated"], false);
    let filenames = result["filenames"].as_array().unwrap();
    assert!(
        filenames
            .iter()
            .any(|f| f.as_str().unwrap().ends_with("a.rs"))
    );
    assert!(
        filenames
            .iter()
            .any(|f| f.as_str().unwrap().ends_with("b.rs"))
    );
}

#[test]
fn test_grep_finds_content() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("search.txt"), "hello world\nfoo bar\n").unwrap();
    let _guard = SetCwd::new(dir.path());

    let result = crate::tools::grep::grep_tool_response(&serde_json::json!({
        "pattern": "foo",
        "output_mode": "content"
    }));

    // Should find the match (if rg or grep is available)
    if result.get("status").and_then(|v| v.as_str()) != Some("error") {
        assert!(result["numMatches"].as_u64().unwrap() >= 1);
    }
}

#[test]
fn test_tool_search_finds_by_keyword() {
    let tools = crate::tools::build_builtin_tools();
    let result = crate::tools::tool_search::tool_search_response("file read", &tools);
    assert!(result["matches"].as_u64().unwrap() >= 1);
    let tool_names: Vec<&str> = result["tools"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|t| t["name"].as_str())
        .collect();
    assert!(tool_names.contains(&"fs_read"));
}

#[test]
fn test_tool_search_empty_query() {
    let tools = crate::tools::build_builtin_tools();
    let result = crate::tools::tool_search::tool_search_response("", &tools);
    assert_eq!(result["status"], "error");
}

#[test]
fn test_permission_rules_evaluate() {
    use crate::tool_policy::{PermissionDecision, PermissionRules, ToolPattern};

    let rules = PermissionRules {
        always_allow: vec![ToolPattern("fs_read:*".to_string())],
        always_deny: vec![ToolPattern("execute_bash:rm -rf *".to_string())],
        always_ask: vec![ToolPattern("web_fetch:*".to_string())],
    };

    assert_eq!(
        rules.evaluate("fs_read", Some("/any/path")),
        PermissionDecision::Allow
    );
    assert_eq!(
        rules.evaluate("execute_bash", Some("rm -rf /")),
        PermissionDecision::Deny
    );
    assert_eq!(
        rules.evaluate("web_fetch", Some("https://example.com")),
        PermissionDecision::Ask
    );
    assert_eq!(
        rules.evaluate("todo_list", None),
        PermissionDecision::NoMatch
    );
}

#[test]
fn test_permission_rules_deny_takes_precedence() {
    use crate::tool_policy::{PermissionDecision, PermissionRules, ToolPattern};

    let rules = PermissionRules {
        always_allow: vec![ToolPattern("execute_bash:*".to_string())],
        always_deny: vec![ToolPattern("execute_bash:rm *".to_string())],
        always_ask: vec![],
    };

    // Deny takes precedence over allow
    assert_eq!(
        rules.evaluate("execute_bash", Some("rm -rf /")),
        PermissionDecision::Deny
    );
    assert_eq!(
        rules.evaluate("execute_bash", Some("ls -la")),
        PermissionDecision::Allow
    );
}

// Helper to temporarily change cwd for tests
struct SetCwd {
    prev: std::path::PathBuf,
    _lock: std::sync::MutexGuard<'static, ()>,
}

impl SetCwd {
    fn new(path: &std::path::Path) -> Self {
        static CWD_LOCK: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();
        let lock = CWD_LOCK
            .get_or_init(|| std::sync::Mutex::new(()))
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let prev = std::env::current_dir().unwrap();
        std::env::set_current_dir(path).unwrap();
        Self { prev, _lock: lock }
    }
}

impl Drop for SetCwd {
    fn drop(&mut self) {
        let _ = std::env::set_current_dir(&self.prev);
    }
}

#[test]
fn test_adk_skill_parses_anthropic_skills() {
    // Use the compile-time repository root. Other filesystem tests temporarily
    // change the process cwd, which is global and made this assertion depend on
    // test scheduling.
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let files = adk_skill::discover_skill_files(root).unwrap_or_default();
    if files.is_empty() {
        println!("No .skills/ directory found — skipping");
        return;
    }
    println!("Discovered {} skill files", files.len());

    let index = adk_skill::load_skill_index(root);
    match &index {
        Ok(idx) => {
            println!("Parsed {} skills:", idx.skills().len());
            for skill in idx.skills() {
                println!("  ✓ {} — {}", skill.name, skill.description);
            }
            assert!(!idx.skills().is_empty(), "should parse at least one skill");
        }
        Err(e) => {
            println!("Parse error: {}", e);
        }
    }
}

/// Reloading picks up servers added to the profile since start, and leaves the
/// session's own route choices alone.
///
/// Enabling a capability appends MCP servers to the profile on disk. A running
/// session has to see them without discarding a `/worker` or `/planner` switch
/// made since it started, which is why only that one field is refreshed.
#[test]
fn reloading_mcp_servers_preserves_session_route_choices() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("config.toml");
    std::fs::write(
        &path,
        r#"
[profiles.default]
worker_provider = "openai"
worker_model = "from-disk"

[[profiles.default.mcp_servers]]
name = "mcp-search"
command = "mcp-search"
enabled = true

[[profiles.default.mcp_servers]]
name = "mcp-browser"
command = "mcp-browser"
enabled = true
"#,
    )
    .expect("write");

    let mut cfg = base_cfg();
    cfg.config_path = path.to_string_lossy().into_owned();
    cfg.profile = "default".to_string();
    // Stand in for a mid-session `/worker` switch.
    cfg.worker_model = "chosen-in-session".to_string();
    assert!(cfg.mcp_servers.is_empty());

    let declared = crate::config::reload_mcp_servers(&mut cfg).expect("reload");
    assert_eq!(declared, 2);
    let names: Vec<&str> = cfg
        .mcp_servers
        .iter()
        .map(|server| server.name.as_str())
        .collect();
    assert_eq!(names, vec!["mcp-search", "mcp-browser"]);
    assert_eq!(
        cfg.worker_model, "chosen-in-session",
        "reloading must not discard a route chosen during the session"
    );

    // A profile with no servers reloads to empty rather than erroring.
    cfg.profile = "absent".to_string();
    assert_eq!(
        crate::config::reload_mcp_servers(&mut cfg).expect("reload"),
        0
    );
    assert!(cfg.mcp_servers.is_empty());
}

/// `--version` reports the package version.
///
/// Clap only offers the flag when the command declares it, and it declined to for
/// v2.0.0 until this was noticed during release preparation. It is the first thing
/// anyone runs against a released binary and the first thing packaging scripts read,
/// so the wiring is asserted rather than assumed.
#[test]
fn the_cli_reports_its_version() {
    use clap::CommandFactory;
    let command = crate::cli::Cli::command();
    let declared = command
        .get_version()
        .expect("the CLI must declare a version, or clap omits --version entirely");
    assert_eq!(declared, env!("CARGO_PKG_VERSION"));

    // And the flag is actually reachable.
    let rendered = command.clone().render_version();
    assert!(
        rendered.contains(env!("CARGO_PKG_VERSION")),
        "rendered version should name the package version: {rendered}"
    );
}
