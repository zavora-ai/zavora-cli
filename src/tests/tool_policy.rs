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

#[test]
fn provider_safe_names_preserve_valid_tools_and_normalize_mcp_names() {
    let tools = make_provider_safe_tool_names(vec![
        make_mock_tool("fs_read"),
        make_mock_tool("mcp:docx-mcp:add_bookmark"),
    ]);
    let names = tools.iter().map(|tool| tool.name()).collect::<Vec<_>>();
    assert_eq!(names, vec!["fs_read", "mcp__docx-mcp__add_bookmark"]);
}

#[test]
fn provider_safe_names_bound_length_and_disambiguate_collisions() {
    let long = format!("mcp:server:{}", "very_long_tool_name_".repeat(5));
    let tools = make_provider_safe_tool_names(vec![
        make_mock_tool("mcp:server:a.b"),
        make_mock_tool("mcp:server:a/b"),
        make_mock_tool(&long),
    ]);
    let names = tools
        .iter()
        .map(|tool| tool.name().to_string())
        .collect::<Vec<_>>();

    assert_ne!(names[0], names[1]);
    assert_eq!(
        names.len(),
        names.iter().collect::<std::collections::HashSet<_>>().len()
    );
    for name in names {
        assert!(name.len() <= PROVIDER_TOOL_NAME_MAX_LEN);
        assert!(
            name.chars().all(
                |character| character.is_ascii_alphanumeric() || matches!(character, '_' | '-')
            ),
            "provider-unsafe name: {name}"
        );
    }
}
