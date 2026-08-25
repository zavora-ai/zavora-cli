//! Checkpoint persistence-format and tangent-mode tests.

use adk_rust::{Content, Event, Part};

// ---------------------------------------------------------------------------
// Checkpoint and tangent tests
// ---------------------------------------------------------------------------

use crate::checkpoint::*;

fn make_test_event(author: &str, text: &str) -> Event {
    let mut event = Event::new("test-inv");
    event.author = author.to_string();
    event.llm_response.content = Some(Content {
        role: author.to_string(),
        parts: vec![Part::Text {
            text: text.to_string(),
        }],
    });
    event
}

#[test]
fn test_checkpoint_save_and_list() {
    let mut store = CheckpointStore::new();
    assert!(store.list().is_empty());

    let events = vec![make_test_event("user", "hello")];
    store.save("first", events.clone());

    assert_eq!(store.list().len(), 1);
    assert_eq!(store.list()[0].tag, 0);
    assert_eq!(store.list()[0].label, "first");
    assert_eq!(store.list()[0].events.len(), 1);
}

#[test]
fn test_checkpoint_save_default_label() {
    let mut store = CheckpointStore::new();
    store.save("", vec![]);
    assert_eq!(store.list()[0].label, "checkpoint-0");
}

#[test]
fn test_checkpoint_get_by_tag() {
    let mut store = CheckpointStore::new();
    store.save("a", vec![make_test_event("user", "one")]);
    store.save("b", vec![make_test_event("user", "two")]);

    assert!(store.get(0).is_some());
    assert_eq!(store.get(0).unwrap().label, "a");
    assert!(store.get(1).is_some());
    assert!(store.get(99).is_none());
}

#[test]
fn test_tangent_enter_exit() {
    let mut store = CheckpointStore::new();
    assert!(!store.in_tangent());

    let baseline = vec![make_test_event("user", "baseline")];
    let tag = store.enter_tangent(baseline.clone());
    assert!(store.in_tangent());
    assert_eq!(tag, 0);

    let restored = store.exit_tangent();
    assert!(!store.in_tangent());
    assert!(restored.is_some());
    assert_eq!(restored.unwrap().len(), 1);
}

#[test]
fn test_tangent_exit_when_not_in_tangent() {
    let mut store = CheckpointStore::new();
    assert!(store.exit_tangent().is_none());
}

#[test]
fn test_tangent_tail_keeps_last_exchange() {
    let mut store = CheckpointStore::new();
    let baseline = vec![
        make_test_event("user", "q1"),
        make_test_event("model", "a1"),
    ];
    store.enter_tangent(baseline);

    // Simulate tangent conversation
    let current = vec![
        make_test_event("user", "q1"),
        make_test_event("model", "a1"),
        make_test_event("user", "tangent-q"),
        make_test_event("model", "tangent-a"),
    ];

    let result = store.exit_tangent_tail(&current).unwrap();
    assert!(!store.in_tangent());
    // Should have baseline (2) + tail (last user + assistant = 2) = 4
    assert_eq!(result.len(), 4);
}

#[test]
fn test_format_checkpoint_list_empty() {
    let store = CheckpointStore::new();
    let output = format_checkpoint_list(&store);
    assert!(output.contains("No checkpoints saved"));
}

#[test]
fn test_format_checkpoint_list_with_entries() {
    let mut store = CheckpointStore::new();
    store.save("test-cp", vec![make_test_event("user", "hi")]);
    let output = format_checkpoint_list(&store);
    assert!(output.contains("[0]"));
    assert!(output.contains("test-cp"));
    assert!(output.contains("1 events"));
}

#[test]
fn test_format_checkpoint_list_tangent_indicator() {
    let mut store = CheckpointStore::new();
    store.enter_tangent(vec![]);
    let output = format_checkpoint_list(&store);
    assert!(output.contains("tangent mode active"));
}
