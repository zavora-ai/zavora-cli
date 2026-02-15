/// Checkpoint and tangent conversation branching.
///
/// Checkpoints save conversation state snapshots. Tangent mode branches a
/// temporary conversation and returns to the saved baseline on exit.
///
/// Design follows the working style rule: "Prefer simple state machines over
/// complex orchestration for conversation features."
use adk_rust::Event;
use adk_session::SessionService;
use anyhow::{Context as _, Result};
use std::sync::Arc;

use crate::config::RuntimeConfig;
use crate::session::ensure_session_exists;

// ---------------------------------------------------------------------------
// Checkpoint
// ---------------------------------------------------------------------------

/// A saved snapshot of conversation events.
#[derive(Debug, Clone)]
pub struct Checkpoint {
    pub tag: usize,
    pub label: String,
    pub timestamp: String,
    pub events: Vec<Event>,
}

/// In-memory store of conversation checkpoints.
#[derive(Debug, Default)]
pub struct CheckpointStore {
    checkpoints: Vec<Checkpoint>,
    next_tag: usize,
    /// When in tangent mode, holds the checkpoint tag to restore on exit.
    tangent_baseline: Option<usize>,
}

impl CheckpointStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Save a checkpoint from the current session events.
    pub fn save(&mut self, label: &str, events: Vec<Event>) -> &Checkpoint {
        let tag = self.next_tag;
        self.next_tag += 1;
        // Use the last event's timestamp or a placeholder
        let timestamp = events
            .last()
            .map(|e| e.timestamp.format("%H:%M:%S").to_string())
            .unwrap_or_else(|| "now".to_string());
        self.checkpoints.push(Checkpoint {
            tag,
            label: if label.is_empty() {
                format!("checkpoint-{tag}")
            } else {
                label.to_string()
            },
            timestamp,
            events,
        });
        self.checkpoints.last().unwrap()
    }

    /// List all saved checkpoints.
    pub fn list(&self) -> &[Checkpoint] {
        &self.checkpoints
    }

    /// Find a checkpoint by tag number.
    pub fn get(&self, tag: usize) -> Option<&Checkpoint> {
        self.checkpoints.iter().find(|c| c.tag == tag)
    }

    /// Whether tangent mode is active.
    pub fn in_tangent(&self) -> bool {
        self.tangent_baseline.is_some()
    }

    /// Enter tangent mode: auto-save a checkpoint and record the baseline tag.
    pub fn enter_tangent(&mut self, events: Vec<Event>) -> usize {
        let cp = self.save("tangent-baseline", events);
        let tag = cp.tag;
        self.tangent_baseline = Some(tag);
        tag
    }

    /// Exit tangent mode and return the baseline checkpoint events.
    /// Returns `None` if not in tangent mode.
    pub fn exit_tangent(&mut self) -> Option<Vec<Event>> {
        let tag = self.tangent_baseline.take()?;
        self.get(tag).map(|cp| cp.events.clone())
    }

    /// Exit tangent mode but keep the last user+assistant exchange.
    /// Returns the baseline events with the tail appended, or `None` if not in tangent.
    pub fn exit_tangent_tail(&mut self, current_events: &[Event]) -> Option<Vec<Event>> {
        let tag = self.tangent_baseline.take()?;
        let baseline = self.get(tag)?.events.clone();

        // Find the last user message and everything after it in current events
        let tail_start = current_events
            .iter()
            .rposition(|e| e.author == "user")
            .unwrap_or(current_events.len());

        let mut result = baseline;
        if tail_start < current_events.len() {
            result.extend_from_slice(&current_events[tail_start..]);
        }
        Some(result)
    }
}

// ---------------------------------------------------------------------------
// Session operations
// ---------------------------------------------------------------------------

/// Read all events from the current session.
pub async fn snapshot_session_events(
    session_service: &Arc<dyn SessionService>,
    cfg: &RuntimeConfig,
) -> Result<Vec<Event>> {
    let session = session_service
        .get(adk_session::GetRequest {
            app_name: cfg.app_name.clone(),
            user_id: cfg.user_id.clone(),
            session_id: cfg.session_id.clone(),
            num_recent_events: None,
            after: None,
        })
        .await
        .context("failed to load session for checkpoint")?;
    Ok(session.events().all())
}

/// Replace session contents with the given events.
pub async fn restore_session_events(
    session_service: &Arc<dyn SessionService>,
    cfg: &RuntimeConfig,
    events: &[Event],
) -> Result<()> {
    session_service
        .delete(adk_session::DeleteRequest {
            app_name: cfg.app_name.clone(),
            user_id: cfg.user_id.clone(),
            session_id: cfg.session_id.clone(),
        })
        .await
        .context("failed to delete session for restore")?;

    ensure_session_exists(session_service, cfg).await?;

    for event in events {
        session_service
            .append_event(&cfg.session_id, event.clone())
            .await
            .context("failed to re-append event during restore")?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Display helpers
// ---------------------------------------------------------------------------

/// Format checkpoint list for display.
pub fn format_checkpoint_list(store: &CheckpointStore) -> String {
    let checkpoints = store.list();
    if checkpoints.is_empty() {
        return "No checkpoints saved. Use /checkpoint save [label] to create one.".to_string();
    }
    let mut out = String::from("Checkpoints:\n");
    for cp in checkpoints {
        out.push_str(&format!(
            "  [{}] {} ({} events, {})\n",
            cp.tag,
            cp.label,
            cp.events.len(),
            cp.timestamp,
        ));
    }
    if store.in_tangent() {
        out.push_str("  (tangent mode active)\n");
    }
    out
}
