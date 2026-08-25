//! Classic theme, command suggestion, prompt, and onboarding tests.

use crate::checkpoint::CheckpointStore;
use crate::context::ContextUsage;

// ---------------------------------------------------------------------------
// Theme, command palette, and onboarding tests
// ---------------------------------------------------------------------------

use crate::theme::*;

#[test]
fn test_fuzzy_match_exact() {
    assert_eq!(
        fuzzy_match_command("help"),
        FuzzyResult::Exact("help".into())
    );
    assert_eq!(
        fuzzy_match_command("exit"),
        FuzzyResult::Exact("exit".into())
    );
}

#[test]
fn test_fuzzy_match_prefix() {
    assert_eq!(fuzzy_match_command("he"), FuzzyResult::Exact("help".into()));
    assert_eq!(
        fuzzy_match_command("us"),
        FuzzyResult::Exact("usage".into())
    );
    assert_eq!(
        fuzzy_match_command("com"),
        FuzzyResult::Exact("compact".into())
    );
}

#[test]
fn test_fuzzy_match_ambiguous() {
    // "to" matches "tools" and "todos"
    match fuzzy_match_command("to") {
        FuzzyResult::Ambiguous(cmds) => {
            assert!(cmds.contains(&"tools".to_string()));
            assert!(cmds.contains(&"todos".to_string()));
        }
        other => panic!("Expected Ambiguous, got {:?}", other),
    }
}

#[test]
fn test_fuzzy_match_no_match() {
    assert_eq!(fuzzy_match_command("zzz"), FuzzyResult::NoMatch);
}

#[test]
fn test_suggest_command_exact() {
    let suggestion = suggest_command("he").unwrap();
    assert!(suggestion.contains("/help"));
}

#[test]
fn test_suggest_command_ambiguous() {
    let suggestion = suggest_command("to").unwrap();
    assert!(suggestion.contains("/tools"));
    assert!(suggestion.contains("/todos"));
}

#[test]
fn test_suggest_command_none() {
    assert!(suggest_command("zzz").is_none());
}

#[test]
fn test_build_prompt_default() {
    let store = CheckpointStore::new();
    let prompt = build_prompt(&store, None);
    assert!(prompt.contains("zavora>"));
}

#[test]
fn test_build_prompt_tangent_mode() {
    let mut store = CheckpointStore::new();
    store.enter_tangent(vec![]);
    let prompt = build_prompt(&store, None);
    assert!(prompt.contains("↯tangent"));
}

#[test]
fn test_build_prompt_with_budget() {
    let store = CheckpointStore::new();
    let _usage = ContextUsage {
        user_chars: 3200,
        assistant_chars: 0,
        tool_chars: 0,
        system_chars: 0,
        context_window_tokens: 10_000,
        api_total_tokens: 0,
        event_count: 0,
    };
    // 3200/4 = 800 + 1500 overhead = 2300 tokens, 2300/10000 = 23% => but let's use api_total_tokens
    // Actually use api_total_tokens to bypass overhead
    let usage = ContextUsage {
        user_chars: 3200,
        assistant_chars: 0,
        tool_chars: 0,
        system_chars: 0,
        context_window_tokens: 1000,
        api_total_tokens: 800,
        event_count: 0,
    };
    // 800/1000 = 80% => Warning
    let prompt = build_prompt(&store, Some(&usage));
    assert!(prompt.contains("⚠"));
}

#[test]
fn test_is_first_run() {
    let dir = tempfile::tempdir().unwrap();
    assert!(is_first_run(dir.path()));
    std::fs::create_dir_all(dir.path().join(".zavora")).unwrap();
    assert!(!is_first_run(dir.path()));
}

#[test]
fn test_format_command_palette() {
    let palette = format_command_palette();
    assert!(palette.contains("/help"));
    assert!(palette.contains("/compact"));
    assert!(palette.contains("/checkpoint"));
    assert!(palette.contains("/tangent"));
    assert!(palette.contains("/todos"));
    assert!(palette.contains("/delegate"));
}
