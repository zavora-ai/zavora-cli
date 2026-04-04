/// Persistent memory backed by adk-memory SQLite.
///
/// Uses a dedicated thread with its own tokio runtime to avoid SQLx
/// lifetime issues with `async_trait` desugaring.
use anyhow::{Context, Result};
use std::sync::Arc;

const DB_PATH: &str = ".zavora/memory.db";
const APP_NAME: &str = "zavora-cli";
const USER_ID: &str = "default";

fn run_blocking<F, T>(f: F) -> Result<T>
where
    F: FnOnce(adk_memory::MemoryServiceAdapter) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<T>> + Send>>
        + Send
        + 'static,
    T: Send + 'static,
{
    std::fs::create_dir_all(".zavora").ok();
    std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .context("failed to build runtime")?;
        rt.block_on(async {
            let svc = adk_memory::SqliteMemoryService::new(&format!("sqlite:{DB_PATH}"))
                .await
                .context("failed to open memory database")?;
            svc.migrate().await.context("memory migration failed")?;
            let mem = adk_memory::MemoryServiceAdapter::new(Arc::new(svc), APP_NAME, USER_ID);
            f(mem).await
        })
    })
    .join()
    .map_err(|_| anyhow::anyhow!("memory thread panicked"))?
}

pub async fn recall(query: &str, limit: usize) -> Result<Vec<String>> {
    let q = query.to_string();
    tokio::task::spawn_blocking(move || {
        run_blocking(move |mem| {
            Box::pin(async move {
                use adk_rust::Memory;
                let entries = mem.search(&q).await.map_err(|e| anyhow::anyhow!("{e}"))?;
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
            })
        })
    })
    .await?
}

pub async fn remember(text: &str) -> Result<()> {
    let t = text.to_string();
    tokio::task::spawn_blocking(move || {
        run_blocking(move |mem| {
            Box::pin(async move {
                use adk_rust::Memory;
                let entry = adk_rust::MemoryEntry {
                    content: adk_rust::Content {
                        role: "user".into(),
                        parts: vec![adk_rust::Part::Text { text: t }],
                    },
                    author: USER_ID.into(),
                };
                mem.add(entry).await.map_err(|e| anyhow::anyhow!("{e}"))
            })
        })
    })
    .await?
}

pub async fn forget(query: &str) -> Result<u64> {
    let q = query.to_string();
    tokio::task::spawn_blocking(move || {
        run_blocking(move |mem| {
            Box::pin(async move {
                use adk_rust::Memory;
                mem.delete(&q).await.map_err(|e| anyhow::anyhow!("{e}"))
            })
        })
    })
    .await?
}

#[cfg(test)]
mod tests {
    // Note: SQLite memory tests require a real filesystem and are tested
    // via integration tests to avoid cwd pollution from set_current_dir.
    // The module compiles and is validated by cargo check.
}
