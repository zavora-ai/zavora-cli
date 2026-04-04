/// File history — snapshot files before modification, support /undo.
///
/// Stores snapshots in `.zavora/file_history/<path_hash>/<timestamp>`.
/// Max 20 snapshots per file, oldest pruned automatically.
use anyhow::{Context, Result};
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

const HISTORY_DIR: &str = ".zavora/file_history";
const MAX_SNAPSHOTS: usize = 20;

static UNDO_STACK: Mutex<Option<VecDeque<UndoEntry>>> = Mutex::new(None);

#[derive(Debug, Clone)]
struct UndoEntry {
    path: PathBuf,
    snapshot: PathBuf,
}

fn history_dir(file_path: &Path) -> PathBuf {
    let hash = format!("{:x}", md5::compute(file_path.to_string_lossy().as_bytes()));
    PathBuf::from(HISTORY_DIR).join(hash)
}

/// Snapshot a file before modification.
pub fn snapshot_file(file_path: &Path) -> Result<()> {
    if !file_path.exists() {
        return Ok(()); // New file, nothing to snapshot
    }

    let dir = history_dir(file_path);
    std::fs::create_dir_all(&dir).context("failed to create history dir")?;

    let ts = chrono::Utc::now().format("%Y%m%dT%H%M%S%.3f").to_string();
    let snapshot_path = dir.join(&ts);
    std::fs::copy(file_path, &snapshot_path).context("failed to snapshot file")?;

    // Push to undo stack
    let mut stack = UNDO_STACK.lock().unwrap();
    let stack = stack.get_or_insert_with(VecDeque::new);
    stack.push_back(UndoEntry {
        path: file_path.to_path_buf(),
        snapshot: snapshot_path,
    });

    // Prune old snapshots
    let mut entries: Vec<_> = std::fs::read_dir(&dir)?
        .filter_map(|e| e.ok())
        .collect();
    if entries.len() > MAX_SNAPSHOTS {
        entries.sort_by_key(|e| e.file_name());
        for entry in &entries[..entries.len() - MAX_SNAPSHOTS] {
            let _ = std::fs::remove_file(entry.path());
        }
    }

    Ok(())
}

/// Undo the last file modification by restoring the most recent snapshot.
pub fn undo_last() -> Result<String> {
    let mut stack = UNDO_STACK.lock().unwrap();
    let stack = stack.get_or_insert_with(VecDeque::new);

    let entry = stack.pop_back()
        .context("nothing to undo")?;

    if !entry.snapshot.exists() {
        return Err(anyhow::anyhow!("snapshot file missing: {}", entry.snapshot.display()));
    }

    std::fs::copy(&entry.snapshot, &entry.path)
        .context("failed to restore snapshot")?;

    Ok(format!("Restored {}", entry.path.display()))
}

/// List recent snapshots for a file.
pub fn list_snapshots(file_path: &Path) -> Vec<String> {
    let dir = history_dir(file_path);
    if !dir.exists() {
        return Vec::new();
    }
    let mut entries: Vec<_> = std::fs::read_dir(&dir)
        .into_iter()
        .flatten()
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().to_string())
        .collect();
    entries.sort();
    entries.reverse();
    entries
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_snapshot_and_undo() {
        let dir = tempdir().unwrap();
        let file = dir.path().join("test.txt");
        std::fs::write(&file, "original").unwrap();

        // Snapshot before edit
        snapshot_file(&file).unwrap();
        std::fs::write(&file, "modified").unwrap();

        assert_eq!(std::fs::read_to_string(&file).unwrap(), "modified");

        // Undo
        let msg = undo_last().unwrap();
        assert!(msg.contains("test.txt"));
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "original");
    }
}
