/// Context compaction for long-running chat sessions.
///
/// Provides both manual (`/compact`) and automatic compaction. Manual compaction
/// replaces the session with a text summary. Automatic compaction uses ADK's
/// `EventsCompactionConfig` to periodically summarize older events.
use adk_rust::{
    BaseEventsSummarizer, Content, Event, EventActions, EventCompaction, EventsCompactionConfig,
    Part,
};
use adk_session::SessionService;
use anyhow::{Context as _, Result};
use async_trait::async_trait;
use std::sync::Arc;

use crate::config::RuntimeConfig;
use crate::context::estimate_tokens;
use crate::session::ensure_session_exists;

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

/// Strategy for manual `/compact`.
#[derive(Debug, Clone)]
pub struct CompactStrategy {
    /// Number of recent user/assistant pairs to keep verbatim.
    pub messages_to_keep: usize,
    /// Maximum chars per event when truncating.
    pub max_event_chars: usize,
    /// Use LLM to generate summary (vs simple text extraction).
    pub use_llm_summary: bool,
}

impl Default for CompactStrategy {
    fn default() -> Self {
        Self {
            messages_to_keep: 2,
            max_event_chars: 4000,
            use_llm_summary: true,
        }
    }
}

// ---------------------------------------------------------------------------
// Event text extraction
// ---------------------------------------------------------------------------

/// Extract displayable text from an event's content parts.
pub fn extract_event_text(event: &Event) -> String {
    event
        .llm_response
        .content
        .as_ref()
        .map(|c| {
            c.parts
                .iter()
                .filter_map(|p| match p {
                    Part::Text { text } => Some(text.as_str()),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join(" ")
        })
        .unwrap_or_default()
}

/// Build a condensed text summary from a slice of events.
pub fn summarize_events_text(events: &[Event], max_chars: usize) -> String {
    let mut summary = String::from("[Compacted conversation summary]\n");
    for event in events {
        let text = extract_event_text(event);
        if text.is_empty() {
            continue;
        }
        let role = if event.author == "user" {
            "User"
        } else {
            "Assistant"
        };
        let truncated = if text.len() > max_chars {
            format!("{}…", &text[..max_chars])
        } else {
            text
        };
        summary.push_str(&format!("{role}: {truncated}\n"));
    }
    summary
}

// ---------------------------------------------------------------------------
// LLM-based summarization
// ---------------------------------------------------------------------------

const SUMMARY_PROMPT: &str = "[SYSTEM NOTE: This is an automated summarization request]\n\n\
FORMAT REQUIREMENTS: Create a structured, concise summary in bullet-point format. DO NOT respond conversationally.\n\n\
Your task is to create a structured summary document containing:\n\
1) A bullet-point list of key topics/questions covered\n\
2) Bullet points for all significant tools executed and their results\n\
3) Bullet points for any code or technical information shared\n\
4) A section of key insights gained\n\
5) The ID of the currently loaded todo list, if any\n\n\
FORMAT THE SUMMARY IN THIRD PERSON. Example format:\n\n\
## CONVERSATION SUMMARY\n\
* Topic 1: Key information\n\
* Topic 2: Key information\n\n\
## TOOLS EXECUTED\n\
* Tool X: Result Y\n\n\
## TODO ID\n\
* <id>\n\n\
Remember this is a DOCUMENT not a chat response.\n\
FILTER OUT CHAT CONVENTIONS (greetings, offers to help, etc).";

/// Generate an LLM-based summary of conversation events.
async fn generate_llm_summary(
    session_service: &Arc<dyn SessionService>,
    cfg: &RuntimeConfig,
    events: &[Event],
) -> Result<String> {
    use crate::provider::resolve_model;
    use crate::runner::build_runner;
    use adk_rust::prelude::*;

    // Build conversation history for summarization
    let mut history_text = String::new();
    for event in events {
        let text = extract_event_text(event);
        if text.is_empty() {
            continue;
        }
        let role = if event.author == "user" { "User" } else { "Assistant" };
        history_text.push_str(&format!("{role}: {text}\n\n"));
    }

    let prompt = format!("{}\n\nCONVERSATION TO SUMMARIZE:\n\n{}", SUMMARY_PROMPT, history_text);

    // Create temporary session for summary generation
    let summary_session_id = format!("summary-{}", cfg.session_id);
    let mut summary_cfg = cfg.clone();
    summary_cfg.session_id = summary_session_id.clone();

    // Build minimal agent for summarization (no tools)
    let (model, _, _) = resolve_model(&summary_cfg)?;
    let agent = LlmAgentBuilder::new("summarizer")
        .instruction("You are a conversation summarizer. Create structured, factual summaries.")
        .model(model)
        .build()?;

    let runner = build_runner(Arc::new(agent), &summary_cfg).await?;

    // Generate summary using streaming
    use adk_rust::futures::StreamExt;
    let mut stream = runner
        .run(
            summary_cfg.user_id.clone(),
            summary_cfg.session_id.clone(),
            Content::new("user").with_text(&prompt),
        )
        .await?;

    let mut summary = String::new();
    while let Some(event_result) = stream.next().await {
        if let Ok(event) = event_result {
            if event.author != "user" {
                if let Some(content) = &event.llm_response.content {
                    for part in &content.parts {
                        if let Part::Text { text } = part {
                            summary.push_str(text);
                        }
                    }
                }
            }
        }
    }

    // Clean up temporary session
    let _ = session_service
        .delete(adk_session::DeleteRequest {
            app_name: summary_cfg.app_name.clone(),
            user_id: summary_cfg.user_id.clone(),
            session_id: summary_session_id,
        })
        .await;

    Ok(summary)
}

// ---------------------------------------------------------------------------
// Manual compaction (/compact)
// ---------------------------------------------------------------------------

/// Perform manual compaction: read session events, build summary, replace session.
///
/// Returns the summary text for display, or `None` if the session was too short.
pub async fn compact_session(
    session_service: &Arc<dyn SessionService>,
    cfg: &RuntimeConfig,
    strategy: &CompactStrategy,
) -> Result<Option<String>> {
    let session = session_service
        .get(adk_session::GetRequest {
            app_name: cfg.app_name.clone(),
            user_id: cfg.user_id.clone(),
            session_id: cfg.session_id.clone(),
            num_recent_events: None,
            after: None,
        })
        .await
        .context("failed to load session for compaction")?;

    let events = session.events().all();
    if events.len() < 3 {
        return Ok(None);
    }

    // Split: compact older events, keep recent ones
    let keep = strategy.messages_to_keep.min(events.len());
    let to_compact = &events[..events.len() - keep];
    let to_keep = &events[events.len() - keep..];

    let summary_text = if strategy.use_llm_summary {
        // Use LLM to generate structured summary
        generate_llm_summary(session_service, cfg, to_compact).await?
    } else {
        // Fallback to simple text extraction
        summarize_events_text(to_compact, strategy.max_event_chars)
    };

    // Delete and recreate session
    session_service
        .delete(adk_session::DeleteRequest {
            app_name: cfg.app_name.clone(),
            user_id: cfg.user_id.clone(),
            session_id: cfg.session_id.clone(),
        })
        .await
        .context("failed to delete session for compaction")?;

    ensure_session_exists(session_service, cfg).await?;

    // Append summary as a system event
    let mut summary_event = Event::new("compaction");
    summary_event.author = "system".to_string();
    summary_event.llm_response.content = Some(Content {
        role: "model".to_string(),
        parts: vec![Part::Text {
            text: summary_text.clone(),
        }],
    });
    summary_event.actions = EventActions {
        compaction: Some(EventCompaction {
            start_timestamp: to_compact.first().unwrap().timestamp,
            end_timestamp: to_compact.last().unwrap().timestamp,
            compacted_content: Content {
                role: "model".to_string(),
                parts: vec![Part::Text {
                    text: summary_text.clone(),
                }],
            },
        }),
        ..Default::default()
    };

    session_service
        .append_event(&cfg.session_id, summary_event)
        .await
        .context("failed to append compaction summary")?;

    // Re-append kept events
    for event in to_keep {
        session_service
            .append_event(&cfg.session_id, event.clone())
            .await
            .context("failed to re-append kept event")?;
    }

    let compacted_count = to_compact.len();
    let token_est = estimate_tokens(summary_text.len());
    Ok(Some(format!(
        "Compacted {compacted_count} events into ~{token_est} tokens. Kept {keep} recent messages."
    )))
}

// ---------------------------------------------------------------------------
// Auto-compaction summarizer (for ADK EventsCompactionConfig)
// ---------------------------------------------------------------------------

/// Simple text-extraction summarizer for ADK's auto-compaction.
pub struct TextSummarizer {
    pub max_event_chars: usize,
}

impl Default for TextSummarizer {
    fn default() -> Self {
        Self {
            max_event_chars: 4000,
        }
    }
}

#[async_trait]
impl BaseEventsSummarizer for TextSummarizer {
    async fn summarize_events(&self, events: &[Event]) -> adk_rust::Result<Option<Event>> {
        if events.is_empty() {
            return Ok(None);
        }

        let summary_text = summarize_events_text(events, self.max_event_chars);

        let mut event = Event::new("auto-compaction");
        event.author = "system".to_string();
        event.llm_response.content = Some(Content {
            role: "model".to_string(),
            parts: vec![Part::Text {
                text: summary_text.clone(),
            }],
        });
        event.actions = EventActions {
            compaction: Some(EventCompaction {
                start_timestamp: events.first().unwrap().timestamp,
                end_timestamp: events.last().unwrap().timestamp,
                compacted_content: Content {
                    role: "model".to_string(),
                    parts: vec![Part::Text { text: summary_text }],
                },
            }),
            ..Default::default()
        };

        Ok(Some(event))
    }
}

/// Build an `EventsCompactionConfig` for the runner when auto-compaction is enabled.
pub fn build_compaction_config(interval: u32, overlap: u32) -> EventsCompactionConfig {
    EventsCompactionConfig {
        compaction_interval: interval,
        overlap_size: overlap,
        summarizer: Arc::new(TextSummarizer::default()),
    }
}

/// Compact session repeatedly until target utilization is reached.
pub async fn compact_to_target(
    session_service: &Arc<dyn SessionService>,
    cfg: &RuntimeConfig,
    target_utilization: f64,
) -> Result<String> {
    use crate::context::compute_context_usage;

    let provider_str = format!("{:?}", cfg.provider).to_ascii_lowercase();
    let model_name = cfg.model.as_deref().unwrap_or("unknown");

    let mut iterations = 0;
    let max_iterations = 5;

    loop {
        let session = session_service
            .get(adk_session::GetRequest {
                app_name: cfg.app_name.clone(),
                user_id: cfg.user_id.clone(),
                session_id: cfg.session_id.clone(),
                num_recent_events: None,
                after: None,
            })
            .await?;

        let events = session.events().all();
        let usage = compute_context_usage(&events, &provider_str, model_name);

        if usage.utilization() <= target_utilization || events.len() < 3 {
            return Ok(format!(
                "Compaction complete. Usage: {:.0}%",
                usage.utilization() * 100.0
            ));
        }

        iterations += 1;
        if iterations > max_iterations {
            return Ok(format!(
                "Reached max iterations. Usage: {:.0}%",
                usage.utilization() * 100.0
            ));
        }

        // Compact with aggressive strategy
        let strategy = CompactStrategy {
            messages_to_keep: 1,
            max_event_chars: 2000,
            use_llm_summary: true,
        };

        compact_session(session_service, cfg, &strategy).await?;
    }
}
