/// Persistent memory backed by adk-memory SQLite.
///
/// Memory is initialized eagerly via `init()` at startup, then accessed
/// via a global Arc. This avoids OnceCell + SQLx lifetime issues in async_trait.
///
/// Supports ADK-Rust v2 project-scoped memory: memories can be isolated
/// per project directory, with global entries visible everywhere.
use anyhow::{Context, Result};
use std::sync::{Arc, OnceLock};

const DB_PATH: &str = ".zavora/memory.db";
const APP_NAME: &str = "zavora-cli";
const USER_ID: &str = "default";

static MEMORY: OnceLock<Arc<adk_memory::MemoryServiceAdapter>> = OnceLock::new();

/// Initialize memory at startup. Call once from main before any memory use.
pub async fn init() -> Result<()> {
    std::fs::create_dir_all(".zavora").ok();
    let svc = adk_memory::SqliteMemoryService::new(&format!("sqlite:{DB_PATH}"))
        .await
        .context("failed to open memory database")?;
    svc.migrate().await.context("memory migration failed")?;
    let adapter = adk_memory::MemoryServiceAdapter::new(Arc::new(svc), APP_NAME, USER_ID);
    let _ = MEMORY.set(Arc::new(adapter));
    Ok(())
}

/// Initialize memory with ADK-Rust v2 project-scoped isolation.
/// Memories stored with a project_id are only visible within that project.
pub async fn init_with_project(project_id: &str) -> Result<()> {
    std::fs::create_dir_all(".zavora").ok();
    let svc = adk_memory::SqliteMemoryService::new(&format!("sqlite:{DB_PATH}"))
        .await
        .context("failed to open memory database")?;
    svc.migrate().await.context("memory migration failed")?;
    let adapter = adk_memory::MemoryServiceAdapter::new(Arc::new(svc), APP_NAME, USER_ID)
        .with_project_id(project_id.to_string());
    let _ = MEMORY.set(Arc::new(adapter));
    Ok(())
}

fn get_memory() -> Result<Arc<adk_memory::MemoryServiceAdapter>> {
    MEMORY
        .get()
        .cloned()
        .context("memory not initialized — call memory::init() first")
}

/// Get the shared memory adapter for wiring into Runner.
pub fn adapter() -> Option<Arc<dyn adk_rust::Memory>> {
    MEMORY.get().map(|m| m.clone() as Arc<dyn adk_rust::Memory>)
}

/// Derive a project ID from the current working directory.
/// Uses the directory name as a stable project identifier.
pub fn detect_project_id() -> Option<String> {
    std::env::current_dir()
        .ok()
        .and_then(|p| p.file_name().map(|n| n.to_string_lossy().to_string()))
}

pub async fn recall(query: &str, limit: usize) -> Result<Vec<String>> {
    use adk_rust::Memory;
    if query.trim().is_empty() {
        return recall_all(limit).await;
    }
    let mem = get_memory()?;
    let entries = mem
        .search(query)
        .await
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    Ok(entries
        .into_iter()
        .take(limit)
        .filter_map(|e| {
            e.content.parts.into_iter().find_map(|p| match p {
                adk_rust::Part::Text { text } => Some(text),
                _ => None,
            })
        })
        .collect())
}

/// List all memories (bypasses FTS5 which can't match empty queries).
async fn recall_all(limit: usize) -> Result<Vec<String>> {
    use sqlx::Row;
    let pool = sqlx::SqlitePool::connect(&format!("sqlite:{DB_PATH}"))
        .await
        .context("failed to open memory db")?;
    let rows = sqlx::query(
        "SELECT content_text FROM memory_entries WHERE app_name = ? AND user_id = ? ORDER BY timestamp DESC LIMIT ?",
    )
    .bind(APP_NAME)
    .bind(USER_ID)
    .bind(limit as i64)
    .fetch_all(&pool)
    .await
    .map_err(|e| anyhow::anyhow!("recall_all failed: {e}"))?;
    Ok(rows
        .iter()
        .map(|r| r.get::<String, _>("content_text"))
        .collect())
}

pub async fn remember(text: &str) -> Result<()> {
    use adk_rust::Memory;
    let mem = get_memory()?;
    mem.add(adk_rust::MemoryEntry {
        content: adk_rust::Content {
            role: "user".into(),
            parts: vec![adk_rust::Part::Text { text: text.into() }],
        },
        author: USER_ID.into(),
    })
    .await
    .map_err(|e| anyhow::anyhow!("{e}"))
}

pub async fn forget(query: &str) -> Result<u64> {
    use adk_rust::Memory;
    let mem = get_memory()?;
    mem.delete(query).await.map_err(|e| anyhow::anyhow!("{e}"))
}

#[cfg(test)]
mod tests {
    // SQLite memory tests require isolated cwd — tested via integration tests.
}
