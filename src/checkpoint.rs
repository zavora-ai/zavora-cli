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
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::Arc;

use crate::config::RuntimeConfig;
use crate::session::ensure_session_exists;

// ---------------------------------------------------------------------------
// Checkpoint
// ---------------------------------------------------------------------------

/// A saved snapshot of conversation events.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Checkpoint {
    pub tag: usize,
    pub label: String,
    pub timestamp: String,
    pub events: Vec<Event>,
}

/// Persistent store of conversation checkpoints.
#[derive(Debug, Default, Serialize, Deserialize)]
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

    // -----------------------------------------------------------------------
    // Persistence
    // -----------------------------------------------------------------------

    /// Save the checkpoint store to disk.
    pub fn save_to_disk(&self, workspace: &Path) -> Result<()> {
        let dir = workspace.join(".zavora");
        std::fs::create_dir_all(&dir).context("failed to create .zavora directory")?;
        let path = dir.join("checkpoints.json");
        let json = serde_json::to_string_pretty(self).context("failed to serialize checkpoints")?;
        std::fs::write(&path, json).context("failed to write checkpoints file")?;
        Ok(())
    }

    /// Load the checkpoint store from disk, or return a new empty store.
    pub fn load_from_disk(workspace: &Path) -> Self {
        let path = workspace.join(".zavora").join("checkpoints.json");
        if !path.exists() {
            return Self::default();
        }
        std::fs::read_to_string(&path)
            .ok()
            .and_then(|json| serde_json::from_str(&json).ok())
            .unwrap_or_default()
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

// ---------------------------------------------------------------------------
// Shadow-git checkpoint manager (feature: checkpoints)
// ---------------------------------------------------------------------------

#[cfg(feature = "checkpoints")]
pub mod shadow {
    use anyhow::{Context as _, Result, bail};
    use serde::{Deserialize, Serialize};
    use std::path::{Path, PathBuf};
    use std::process::{Command, Output};

    /// File change statistics between two checkpoints.
    #[derive(Debug, Clone, Default, Serialize, Deserialize)]
    pub struct FileStats {
        pub added: usize,
        pub modified: usize,
        pub deleted: usize,
    }

    /// A single shadow-git checkpoint.
    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct ShadowCheckpoint {
        pub tag: String,
        pub description: String,
        pub timestamp: String,
    }

    /// Manages a shadow bare git repo for tracking workspace file changes.
    #[derive(Debug)]
    pub struct ShadowCheckpointManager {
        shadow_path: PathBuf,
        work_tree: PathBuf,
        next_tag: usize,
        pub checkpoints: Vec<ShadowCheckpoint>,
    }

    impl ShadowCheckpointManager {
        /// Initialize a new shadow checkpoint manager. Creates the bare repo if needed.
        pub fn init(workspace: &Path) -> Result<Self> {
            if !is_git_installed() {
                bail!("git is required for checkpoints but not installed");
            }

            let shadow_path = workspace.join(".zavora").join("shadow-repo");
            std::fs::create_dir_all(&shadow_path)
                .context("failed to create shadow repo directory")?;

            // Init bare repo if not already initialized.
            if !shadow_path.join("HEAD").exists() {
                run_git(
                    &shadow_path,
                    None,
                    &["init", "--bare", &shadow_path.to_string_lossy()],
                )?;
                run_git(&shadow_path, None, &["config", "user.name", "zavora"])?;
                run_git(
                    &shadow_path,
                    None,
                    &["config", "user.email", "zavora@local"],
                )?;
                run_git(&shadow_path, None, &["config", "core.preloadindex", "true"])?;

                // Initial commit.
                stage_commit_tag(&shadow_path, workspace, "Initial state", "0")?;
            }

            Ok(Self {
                shadow_path,
                work_tree: workspace.to_path_buf(),
                next_tag: 1,
                checkpoints: vec![ShadowCheckpoint {
                    tag: "0".into(),
                    description: "Initial state".into(),
                    timestamp: chrono_now(),
                }],
            })
        }

        /// Create a checkpoint of the current workspace state.
        pub fn create(&mut self, description: &str) -> Result<&ShadowCheckpoint> {
            let tag = self.next_tag.to_string();
            self.next_tag += 1;

            stage_commit_tag(&self.shadow_path, &self.work_tree, description, &tag)?;

            self.checkpoints.push(ShadowCheckpoint {
                tag: tag.clone(),
                description: description.to_string(),
                timestamp: chrono_now(),
            });
            Ok(self.checkpoints.last().unwrap())
        }

        /// Restore workspace files to a checkpoint.
        pub fn restore(&self, tag: &str) -> Result<()> {
            if !self.checkpoints.iter().any(|c| c.tag == tag) {
                bail!("checkpoint '{tag}' not found");
            }
            let out = run_git(
                &self.shadow_path,
                Some(&self.work_tree),
                &["checkout", tag, "--", "."],
            )?;
            if !out.status.success() {
                bail!("restore failed: {}", String::from_utf8_lossy(&out.stderr));
            }
            Ok(())
        }

        /// Show file change summary between two checkpoints.
        pub fn diff(&self, from: &str, to: &str) -> Result<String> {
            let out = run_git(&self.shadow_path, None, &["diff", "--stat", from, to])?;
            Ok(String::from_utf8_lossy(&out.stdout).to_string())
        }

        /// Compute file stats between two checkpoints.
        pub fn file_stats(&self, from: &str, to: &str) -> Result<FileStats> {
            let out = run_git(
                &self.shadow_path,
                None,
                &["diff", "--name-status", from, to],
            )?;
            let mut stats = FileStats::default();
            for line in String::from_utf8_lossy(&out.stdout).lines() {
                match line.chars().next() {
                    Some('A') => stats.added += 1,
                    Some('M') => stats.modified += 1,
                    Some('D') => stats.deleted += 1,
                    Some('R' | 'C') => stats.modified += 1,
                    _ => {}
                }
            }
            Ok(stats)
        }

        /// List all checkpoints.
        pub fn list(&self) -> &[ShadowCheckpoint] {
            &self.checkpoints
        }

        /// Format checkpoint list for display.
        pub fn format_list(&self) -> String {
            if self.checkpoints.is_empty() {
                return "No checkpoints.".into();
            }
            let mut out = String::from("Checkpoints:\n");
            for cp in &self.checkpoints {
                out.push_str(&format!(
                    "  [{}] {} ({})\n",
                    cp.tag, cp.description, cp.timestamp
                ));
            }
            out
        }
    }

    impl Drop for ShadowCheckpointManager {
        fn drop(&mut self) {
            // Clean up shadow repo on exit.
            let _ = std::fs::remove_dir_all(&self.shadow_path);
        }
    }

    // -- helpers --

    fn is_git_installed() -> bool {
        Command::new("git")
            .arg("--version")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }

    fn run_git(dir: &Path, work_tree: Option<&Path>, args: &[&str]) -> Result<Output> {
        let mut cmd = Command::new("git");
        cmd.arg(format!("--git-dir={}", dir.display()));
        if let Some(wt) = work_tree {
            cmd.arg(format!("--work-tree={}", wt.display()));
        }
        cmd.args(args);
        cmd.output().context("failed to run git command")
    }

    fn stage_commit_tag(shadow: &Path, work_tree: &Path, message: &str, tag: &str) -> Result<()> {
        run_git(shadow, Some(work_tree), &["add", "-A"])?;
        let out = run_git(
            shadow,
            Some(work_tree),
            &["commit", "--allow-empty", "--no-verify", "-m", message],
        )?;
        if !out.status.success() {
            bail!("commit failed: {}", String::from_utf8_lossy(&out.stderr));
        }
        let out = run_git(shadow, None, &["tag", tag, "-f"])?;
        if !out.status.success() {
            bail!("tag failed: {}", String::from_utf8_lossy(&out.stderr));
        }
        Ok(())
    }

    fn chrono_now() -> String {
        // Use the same format as Event timestamps without adding chrono dependency.
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let secs = now % 86400;
        format!(
            "{:02}:{:02}:{:02}",
            secs / 3600,
            (secs % 3600) / 60,
            secs % 60
        )
    }
}
