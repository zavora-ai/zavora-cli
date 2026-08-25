//! Todo persistence and delegated-result tests.

// ---------------------------------------------------------------------------
// Todo list and delegate tests
// ---------------------------------------------------------------------------

use crate::todos::*;

#[test]
fn test_todo_list_new() {
    let todo = TodoList::new(
        "t1",
        "Build feature",
        vec!["Step 1".into(), "Step 2".into()],
    );
    assert_eq!(todo.id, "t1");
    assert_eq!(todo.tasks.len(), 2);
    assert!(!todo.tasks[0].completed);
    assert_eq!(todo.completed_count(), 0);
    assert!(!todo.is_finished());
}

#[test]
fn test_todo_complete_task() {
    let mut todo = TodoList::new("t1", "Test", vec!["A".into(), "B".into()]);
    assert!(todo.complete_task(0));
    assert_eq!(todo.completed_count(), 1);
    assert!(!todo.is_finished());

    assert!(todo.complete_task(1));
    assert!(todo.is_finished());

    assert!(!todo.complete_task(99)); // out of bounds
}

#[test]
fn test_todo_format_display() {
    let mut todo = TodoList::new("t1", "My plan", vec!["Do X".into(), "Do Y".into()]);
    todo.complete_task(0);
    let output = todo.format_display();
    assert!(output.contains("My plan"));
    assert!(output.contains("[✓] 0: Do X"));
    assert!(output.contains("[ ] 1: Do Y"));
    assert!(output.contains("1/2"));
}

#[test]
fn test_todo_persistence_roundtrip() {
    let dir = tempfile::tempdir().unwrap();
    let workspace = dir.path();

    let todo = TodoList::new("test-rt", "Roundtrip", vec!["Task A".into()]);
    save_todo(workspace, &todo).unwrap();

    let loaded = load_todo(workspace, "test-rt").unwrap();
    assert_eq!(loaded.id, "test-rt");
    assert_eq!(loaded.description, "Roundtrip");
    assert_eq!(loaded.tasks.len(), 1);
}

#[test]
fn test_todo_list_ids() {
    let dir = tempfile::tempdir().unwrap();
    let workspace = dir.path();

    assert!(list_todo_ids(workspace).unwrap().is_empty());

    save_todo(workspace, &TodoList::new("b", "B", vec!["x".into()])).unwrap();
    save_todo(workspace, &TodoList::new("a", "A", vec!["y".into()])).unwrap();

    let ids = list_todo_ids(workspace).unwrap();
    assert_eq!(ids, vec!["a", "b"]); // sorted
}

#[test]
fn test_todo_delete() {
    let dir = tempfile::tempdir().unwrap();
    let workspace = dir.path();

    save_todo(
        workspace,
        &TodoList::new("del", "Delete me", vec!["x".into()]),
    )
    .unwrap();
    assert!(load_todo(workspace, "del").is_ok());

    delete_todo(workspace, "del").unwrap();
    assert!(load_todo(workspace, "del").is_err());
}

#[test]
fn test_clear_finished_todos() {
    let dir = tempfile::tempdir().unwrap();
    let workspace = dir.path();

    let mut finished = TodoList::new("done", "Done", vec!["x".into()]);
    finished.complete_task(0);
    save_todo(workspace, &finished).unwrap();

    save_todo(workspace, &TodoList::new("wip", "WIP", vec!["y".into()])).unwrap();

    let cleared = clear_finished_todos(workspace).unwrap();
    assert_eq!(cleared, 1);
    assert!(load_todo(workspace, "done").is_err());
    assert!(load_todo(workspace, "wip").is_ok());
}

#[test]
fn test_delegate_result_format() {
    let result = DelegateResult {
        task: "Build X".to_string(),
        agent: "developer_agent".to_string(),
        session_id: "delegate-1".to_string(),
        output: "Done.".to_string(),
        success: true,
        duration_ms: 42,
    };
    let display = result.format_display();
    assert!(display.contains("✓"));
    assert!(display.contains("Build X"));
    assert!(display.contains("delegate-1"));
}

#[test]
fn delegate_parser_supports_named_and_default_agents() {
    assert_eq!(
        parse_delegate_request("@reviewer_agent review the patch").unwrap(),
        (
            Some("reviewer_agent".to_string()),
            "review the patch".to_string()
        )
    );
    assert_eq!(
        parse_delegate_request("--agent developer_agent 'fix the bug'").unwrap(),
        (
            Some("developer_agent".to_string()),
            "fix the bug".to_string()
        )
    );
    assert_eq!(
        parse_delegate_request("summarize this").unwrap(),
        (None, "summarize this".to_string())
    );
}
