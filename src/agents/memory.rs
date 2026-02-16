/// Native memory agent for persistent learnings across sessions.
use anyhow::{Context, Result};
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryEntry {
    pub text: String,
    pub tags: Vec<String>,
    pub confidence: f32,
    pub created_at: DateTime<Utc>,
    pub ttl: Option<Duration>,
}

impl MemoryEntry {
    pub fn is_expired(&self) -> bool {
        if let Some(ttl) = self.ttl {
            Utc::now() > self.created_at + ttl
        } else {
            false
        }
    }
}

pub struct MemoryAgent {
    store: HashMap<String, MemoryEntry>,
    storage_path: std::path::PathBuf,
}

impl MemoryAgent {
    /// Create or load memory agent from disk.
    pub fn new(workspace: &Path) -> Result<Self> {
        let storage_path = workspace.join(".zavora").join("memory.json");
        let store = if storage_path.exists() {
            let content = std::fs::read_to_string(&storage_path)
                .context("failed to read memory storage")?;
            serde_json::from_str(&content).unwrap_or_default()
        } else {
            HashMap::new()
        };

        Ok(Self {
            store,
            storage_path,
        })
    }

    /// Recall memories matching query and tags.
    pub fn recall(&self, query: &str, tags: &[String], top_k: usize) -> Vec<MemoryEntry> {
        let query_lower = query.to_lowercase();
        let mut matches: Vec<(f32, MemoryEntry)> = self
            .store
            .values()
            .filter(|entry| !entry.is_expired())
            .filter_map(|entry| {
                // Tag matching
                let tag_match = if tags.is_empty() {
                    true
                } else {
                    tags.iter().any(|t| entry.tags.contains(t))
                };

                if !tag_match {
                    return None;
                }

                // Text similarity (simple contains check)
                let text_lower = entry.text.to_lowercase();
                let score = if text_lower.contains(&query_lower) {
                    entry.confidence
                } else {
                    0.0
                };

                if score > 0.0 {
                    Some((score, entry.clone()))
                } else {
                    None
                }
            })
            .collect();

        matches.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap());
        matches.into_iter().take(top_k).map(|(_, e)| e).collect()
    }

    /// Store a new memory.
    pub fn remember(
        &mut self,
        text: String,
        tags: Vec<String>,
        confidence: f32,
        ttl: Option<Duration>,
    ) -> Result<()> {
        let key = format!("{:x}", md5::compute(&text));
        let entry = MemoryEntry {
            text,
            tags,
            confidence,
            created_at: Utc::now(),
            ttl,
        };

        self.store.insert(key, entry);
        self.persist()?;
        Ok(())
    }

    /// Forget memories matching selector (tag or text pattern).
    pub fn forget(&mut self, selector: &str) -> Result<usize> {
        let selector_lower = selector.to_lowercase();
        let before_count = self.store.len();

        self.store.retain(|_, entry| {
            let text_match = entry.text.to_lowercase().contains(&selector_lower);
            let tag_match = entry.tags.iter().any(|t| t.to_lowercase() == selector_lower);
            !(text_match || tag_match)
        });

        let removed = before_count - self.store.len();
        if removed > 0 {
            self.persist()?;
        }
        Ok(removed)
    }

    /// Clean up expired entries.
    pub fn cleanup_expired(&mut self) -> Result<usize> {
        let before_count = self.store.len();
        self.store.retain(|_, entry| !entry.is_expired());
        let removed = before_count - self.store.len();
        if removed > 0 {
            self.persist()?;
        }
        Ok(removed)
    }

    /// Persist to disk.
    fn persist(&self) -> Result<()> {
        if let Some(parent) = self.storage_path.parent() {
            std::fs::create_dir_all(parent).context("failed to create memory directory")?;
        }
        let json = serde_json::to_string_pretty(&self.store)
            .context("failed to serialize memory")?;
        std::fs::write(&self.storage_path, json).context("failed to write memory storage")?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_remember_and_recall() {
        let dir = tempdir().unwrap();
        let mut agent = MemoryAgent::new(dir.path()).unwrap();

        agent
            .remember(
                "Use Nairobi timezone".to_string(),
                vec!["preference".to_string()],
                0.9,
                None,
            )
            .unwrap();

        let results = agent.recall("timezone", &[], 10);
        assert_eq!(results.len(), 1);
        assert!(results[0].text.contains("Nairobi"));
    }

    #[test]
    fn test_forget() {
        let dir = tempdir().unwrap();
        let mut agent = MemoryAgent::new(dir.path()).unwrap();

        agent
            .remember("Test memory".to_string(), vec!["test".to_string()], 0.8, None)
            .unwrap();

        let removed = agent.forget("test").unwrap();
        assert_eq!(removed, 1);
        assert_eq!(agent.store.len(), 0);
    }
}
