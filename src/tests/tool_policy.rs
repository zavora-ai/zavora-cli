//! Tool pattern, filtering, and alias-policy tests.

use super::*;

// ---------------------------------------------------------------------------
// Tool policy: wildcard matching
// ---------------------------------------------------------------------------

#[test]
fn test_wildcard_exact_match() {
    assert!(matches_wildcard("fs_read", "fs_read"));
    assert!(!matches_wildcard("fs_read", "fs_write"));
}

#[test]
fn test_wildcard_star_suffix() {
    assert!(matches_wildcard("github_ops.*", "github_ops.issue_create"));
    assert!(matches_wildcard("github_ops.*", "github_ops.pr_create"));
    assert!(!matches_wildcard("github_ops.*", "fs_read"));
}

#[test]
fn test_wildcard_star_prefix() {
    assert!(matches_wildcard("*_create", "github_ops.issue_create"));
    assert!(!matches_wildcard("*_create", "github_ops.issue_update"));
}

#[test]
fn test_wildcard_star_middle() {
    assert!(matches_wildcard("execute_bash.*_rf", "execute_bash.rm_rf"));
    assert!(!matches_wildcard("execute_bash.*_rf", "execute_bash.rm_r"));
}

#[test]
fn test_wildcard_star_only() {
    assert!(matches_wildcard("*", "anything"));
    assert!(matches_wildcard("*", ""));
}

// ---------------------------------------------------------------------------
// Tool policy: filter_tools_by_policy
// ---------------------------------------------------------------------------

fn make_mock_tool(name: &str) -> Arc<dyn Tool> {
    Arc::new(StubTool {
        tool_name: name.to_string(),
    })
}

#[test]
fn test_filter_allow_wildcard() {
    let tools = vec![
        make_mock_tool("fs_read"),
        make_mock_tool("fs_write"),
        make_mock_tool("github_ops.issue_create"),
    ];
    let allow = vec!["fs_*".to_string()];
    let deny = vec![];
    let filtered = filter_tools_by_policy(tools, &allow, &deny);
    let names: Vec<&str> = filtered.iter().map(|t| t.name()).collect();
    assert_eq!(names, vec!["fs_read", "fs_write"]);
}

#[test]
fn test_filter_deny_wildcard() {
    let tools = vec![
        make_mock_tool("fs_read"),
        make_mock_tool("fs_write"),
        make_mock_tool("execute_bash"),
    ];
    let allow = vec![];
    let deny = vec!["fs_*".to_string()];
    let filtered = filter_tools_by_policy(tools, &allow, &deny);
    let names: Vec<&str> = filtered.iter().map(|t| t.name()).collect();
    assert_eq!(names, vec!["execute_bash"]);
}

#[test]
fn test_filter_deny_overrides_allow() {
    let tools = vec![
        make_mock_tool("fs_read"),
        make_mock_tool("fs_write"),
        make_mock_tool("execute_bash"),
    ];
    let allow = vec!["*".to_string()];
    let deny = vec!["fs_write".to_string()];
    let filtered = filter_tools_by_policy(tools, &allow, &deny);
    let names: Vec<&str> = filtered.iter().map(|t| t.name()).collect();
    assert_eq!(names, vec!["fs_read", "execute_bash"]);
}

// ---------------------------------------------------------------------------
// Tool policy: alias resolution
// ---------------------------------------------------------------------------

#[test]
fn test_alias_renames_tool() {
    let tools = vec![make_mock_tool("search_docs")];
    let mut aliases = HashMap::new();
    aliases.insert("search_docs".to_string(), "doc_search".to_string());
    let aliased = apply_tool_aliases(tools, &aliases);
    assert_eq!(aliased[0].name(), "doc_search");
}

#[test]
fn test_alias_no_match_passes_through() {
    let tools = vec![make_mock_tool("fs_read")];
    let mut aliases = HashMap::new();
    aliases.insert("other_tool".to_string(), "renamed".to_string());
    let result = apply_tool_aliases(tools, &aliases);
    assert_eq!(result[0].name(), "fs_read");
}

#[test]
fn test_alias_plus_wildcard_deny() {
    let tools = vec![make_mock_tool("search_docs"), make_mock_tool("run_query")];
    let mut aliases = HashMap::new();
    aliases.insert("search_docs".to_string(), "safe_search".to_string());
    let aliased = apply_tool_aliases(tools, &aliases);
    // deny the aliased name
    let deny = vec!["safe_*".to_string()];
    let filtered = filter_tools_by_policy(aliased, &[], &deny);
    let names: Vec<&str> = filtered.iter().map(|t| t.name()).collect();
    assert_eq!(names, vec!["run_query"]);
}
