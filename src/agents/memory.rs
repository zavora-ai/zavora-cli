/// Persistent memory backed by adk-memory SQLite.
///
/// Uses oneshot channels to bridge async_trait lifetime constraints.
/// The pool-clone fix in adk-memory helps SqliteMemoryService methods
/// be Send, but MemoryServiceAdapter's async_trait still needs isolation.
use anyhow::{Context, Result};
use std::sync::Arc;
use tokio::sync::OnceCell;

const DB_PATH: &str = ".zavora/memory.db";
const APP_NAME: &str = "zavora-cli";
const USER_ID: &str = "default";

static MEMORY: OnceCell<Arc<adk_memory::MemoryServiceAdapter>> = OnceCell::const_new();

async fn get_memory() -> Result<Arc<adk_memory::MemoryServiceAdapter>> {
    MEMORY
        .get_or_try_init(|| async {
            std::fs::create_dir_all(".zavora").ok();
            let svc = adk_memory::SqliteMemoryService::new(&format!("sqlite:{DB_PATH}"))
                .await
                .context("failed to open memory database")?;
            svc.migrate().await.context("memory migration failed")?;
            Ok(Arc::new(adk_memory::MemoryServiceAdapter::new(
                Arc::new(svc), APP_NAME, USER_ID,
            )))
        })
        .await
        .cloned()
}

/// Spawn a memory operation on a dedicated thread to avoid async_trait lifetime issues.
fn spawn_memory<F, T>(f: F) -> tokio::sync::oneshot::Receiver<Result<T>>
where
    F: FnOnce(Arc<adk_memory::MemoryServiceAdapter>) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<T>> + Send>>
        + Send + 'static,
    T: Send + 'static,
{
    let (tx, rx) = tokio::sync::oneshot::channel();
    std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let result = rt.block_on(async {
            let mem = get_memory().await?;
            f(mem).await
        });
        let _ = tx.send(result);
    });
    rx
}

pub async fn recall(query: &str, limit: usize) -> Result<Vec<String>> {
    let q = query.to_string();
    spawn_memory(move |mem| Box::pin(async move {
        use adk_rust::Memory;
        let entries = mem.search(&q).await.map_err(|e| anyhow::anyhow!("{e}"))?;
        Ok(entries.into_iter().take(limit).filter_map(|e| {
            e.content.parts.into_iter().find_map(|p| match p {
                adk_rust::Part::Text { text } => Some(text),
                _ => None,
            })
        }).collect())
    })).await.map_err(|_| anyhow::anyhow!("memory channel closed"))?
}

pub async fn remember(text: &str) -> Result<()> {
    let t = text.to_string();
    spawn_memory(move |mem| Box::pin(async move {
        use adk_rust::Memory;
        mem.add(adk_rust::MemoryEntry {
            content: adk_rust::Content {
                role: "user".into(),
                parts: vec![adk_rust::Part::Text { text: t }],
            },
            author: USER_ID.into(),
        }).await.map_err(|e| anyhow::anyhow!("{e}"))
    })).await.map_err(|_| anyhow::anyhow!("memory channel closed"))?
}

pub async fn forget(query: &str) -> Result<u64> {
    let q = query.to_string();
    spawn_memory(move |mem| Box::pin(async move {
        use adk_rust::Memory;
        mem.delete(&q).await.map_err(|e| anyhow::anyhow!("{e}"))
    })).await.map_err(|_| anyhow::anyhow!("memory channel closed"))?
}

#[cfg(test)]
mod tests {
    // SQLite memory tests require isolated cwd — tested via integration tests.
}
