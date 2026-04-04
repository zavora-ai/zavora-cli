/// RAG pipeline via adk-rag (feature-gated: `rag`).
///
/// Uses in-memory vector store with a simple bag-of-words embedding.
use adk_rust::prelude::*;
use std::sync::Arc;

const COLLECTION: &str = "default";
const DIMENSIONS: usize = 256;

/// Simple bag-of-words embedding (no external API needed).
struct BowEmbedding;

#[async_trait::async_trait]
impl adk_rag::EmbeddingProvider for BowEmbedding {
    async fn embed(&self, text: &str) -> adk_rag::Result<Vec<f32>> {
        let mut vec = vec![0.0f32; DIMENSIONS];
        for word in text.split_whitespace() {
            let h = hash_word(word);
            vec[h % DIMENSIONS] += 1.0;
        }
        let norm: f32 = vec.iter().map(|x| x * x).sum::<f32>().sqrt();
        if norm > 0.0 { vec.iter_mut().for_each(|x| *x /= norm); }
        Ok(vec)
    }
    fn dimensions(&self) -> usize { DIMENSIONS }
}

fn hash_word(word: &str) -> usize {
    let mut h: usize = 5381;
    for b in word.to_lowercase().bytes() {
        h = h.wrapping_mul(33).wrapping_add(b as usize);
    }
    h
}

/// Build the RAG retrieval tool.
pub fn build_rag_tool() -> anyhow::Result<Arc<dyn Tool>> {
    let config = adk_rag::RagConfig::default();
    let pipeline = adk_rag::RagPipeline::builder()
        .config(config)
        .embedding_provider(Arc::new(BowEmbedding))
        .vector_store(Arc::new(adk_rag::InMemoryVectorStore::new()))
        .chunker(Arc::new(adk_rag::RecursiveChunker::new(512, 100)))
        .build()
        .map_err(|e| anyhow::anyhow!("{e}"))?;

    Ok(Arc::new(adk_rag::RagTool::new(Arc::new(pipeline), COLLECTION)))
}

/// Build a RAG pipeline for programmatic use (ingest + query).
pub fn build_rag_pipeline() -> anyhow::Result<adk_rag::RagPipeline> {
    let config = adk_rag::RagConfig::default();
    adk_rag::RagPipeline::builder()
        .config(config)
        .embedding_provider(Arc::new(BowEmbedding))
        .vector_store(Arc::new(adk_rag::InMemoryVectorStore::new()))
        .chunker(Arc::new(adk_rag::RecursiveChunker::new(512, 100)))
        .build()
        .map_err(|e| anyhow::anyhow!("{e}"))
}
