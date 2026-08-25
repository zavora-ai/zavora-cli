//! Conversation compaction and summarizer tests.

// ---------------------------------------------------------------------------
// Compaction tests
// ---------------------------------------------------------------------------

use crate::compact::*;
use adk_rust::{BaseEventsSummarizer, Content, Event, Part};

#[test]
fn test_extract_event_text_with_content() {
    let mut event = Event::new("inv-1");
    event.llm_response.content = Some(Content {
        role: "user".to_string(),
        parts: vec![Part::Text {
            text: "Hello world".to_string(),
        }],
    });
    assert_eq!(extract_event_text(&event), "Hello world");
}

#[test]
fn test_extract_event_text_empty() {
    let event = Event::new("inv-1");
    assert_eq!(extract_event_text(&event), "");
}

#[test]
fn test_summarize_events_text() {
    let mut e1 = Event::new("inv-1");
    e1.author = "user".to_string();
    e1.llm_response.content = Some(Content {
        role: "user".to_string(),
        parts: vec![Part::Text {
            text: "What is Rust?".to_string(),
        }],
    });

    let mut e2 = Event::new("inv-1");
    e2.author = "model".to_string();
    e2.llm_response.content = Some(Content {
        role: "model".to_string(),
        parts: vec![Part::Text {
            text: "Rust is a systems programming language.".to_string(),
        }],
    });

    let summary = summarize_events_text(&[e1, e2], 4000);
    assert!(summary.contains("[Compacted conversation summary]"));
    assert!(summary.contains("User: What is Rust?"));
    assert!(summary.contains("Assistant: Rust is a systems programming language."));
}

#[test]
fn test_summarize_events_truncates_long_text() {
    let mut event = Event::new("inv-1");
    event.author = "user".to_string();
    event.llm_response.content = Some(Content {
        role: "user".to_string(),
        parts: vec![Part::Text {
            text: "x".repeat(200),
        }],
    });

    let summary = summarize_events_text(&[event], 50);
    // Should contain truncated text with ellipsis
    assert!(summary.contains("…"));
    assert!(summary.len() < 200);
}

#[test]
fn test_compact_strategy_defaults() {
    let strategy = CompactStrategy::default();
    assert_eq!(strategy.messages_to_keep, 2);
    assert_eq!(strategy.max_event_chars, 4000);
}

#[tokio::test]
async fn test_text_summarizer_empty_events() {
    let summarizer = TextSummarizer::default();
    let result = summarizer.summarize_events(&[]).await.unwrap();
    assert!(result.is_none());
}

#[tokio::test]
async fn test_text_summarizer_produces_compaction_event() {
    let summarizer = TextSummarizer::default();

    let mut e1 = Event::new("inv-1");
    e1.author = "user".to_string();
    e1.llm_response.content = Some(Content {
        role: "user".to_string(),
        parts: vec![Part::Text {
            text: "Hello".to_string(),
        }],
    });

    let result = summarizer.summarize_events(&[e1]).await.unwrap();
    assert!(result.is_some());
    let event = result.unwrap();
    assert_eq!(event.author, "system");
    assert!(event.actions.compaction.is_some());
    let compaction = event.actions.compaction.unwrap();
    let text = compaction
        .compacted_content
        .parts
        .iter()
        .filter_map(|p| match p {
            Part::Text { text } => Some(text.as_str()),
            _ => None,
        })
        .collect::<String>();
    assert!(text.contains("User: Hello"));
}

#[test]
fn test_build_compaction_config() {
    let config = build_compaction_config(10, 2);
    assert_eq!(config.compaction_interval, 10);
    assert_eq!(config.overlap_size, 2);
}
