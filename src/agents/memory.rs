/// Persistent memory backed by adk-memory SQLite.
///
/// Memory is initialized eagerly via `init()` at startup, then accessed
/// via a global Arc. This avoids OnceCell + SQLx lifetime issues in async_trait.
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

fn get_memory() -> Result<Arc<adk_memory::MemoryServiceAdapter>> {
    MEMORY.get().cloned().context("memory not initialized — call memory::init() first")
}

pub async fn recall(query: &str, limit: usize) -> Result<Vec<String>> {
    use adk_rust::Memory;
    let mem = get_memory()?;
    // FTS5 rejects empty queries — use wildcard to match all
    let q = if query.trim().is_empty() { "*" } else { query };
    let entries = mem.search(q).await.map_err(|e| anyhow::anyhow!("{e}"))?;
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

pub async fn remember(text: &str) -> Result<()> {
    use adk_rust::Memory;
    let mem = get_memory()?;
    mem.add(adk_rust::MemoryEntry {
        content: adk_rust::Content {
            role: "user".into(),
            parts: vec![adk_rust::Part::Text { text: text.into() }],
        },
        author: USER_ID.into(),
    }).await.map_err(|e| anyhow::anyhow!("{e}"))
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
