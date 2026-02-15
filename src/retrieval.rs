use std::sync::Arc;

use anyhow::{Context, Result};

use crate::cli::RetrievalBackend;
use crate::config::RuntimeConfig;

#[derive(Debug, Clone)]
pub struct RetrievedChunk {
    pub source: String,
    pub text: String,
    pub score: usize,
}

pub trait RetrievalService: Send + Sync {
    fn backend_name(&self) -> &'static str;
    fn retrieve(&self, query: &str, max_chunks: usize) -> Result<Vec<RetrievedChunk>>;
}

pub struct DisabledRetrievalService;

impl RetrievalService for DisabledRetrievalService {
    fn backend_name(&self) -> &'static str {
        "disabled"
    }

    fn retrieve(&self, _query: &str, _max_chunks: usize) -> Result<Vec<RetrievedChunk>> {
        Ok(Vec::new())
    }
}

pub struct LocalFileRetrievalService {
    pub chunks: Vec<RetrievedChunk>,
}

pub fn load_retrieval_chunks(path: &str, source_prefix: &str) -> Result<Vec<RetrievedChunk>> {
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read retrieval doc at '{}'", path))?;
    let chunks = content
        .split("\n\n")
        .map(str::trim)
        .filter(|chunk| !chunk.is_empty())
        .enumerate()
        .map(|(index, text)| RetrievedChunk {
            source: format!("{source_prefix}:{path}#{}", index + 1),
            text: text.to_string(),
            score: 0,
        })
        .collect::<Vec<RetrievedChunk>>();
    Ok(chunks)
}

pub fn query_terms(query: &str) -> Vec<String> {
    query
        .split_whitespace()
        .map(|token| token.trim_matches(|c: char| !c.is_ascii_alphanumeric()))
        .map(str::to_ascii_lowercase)
        .filter(|token| token.len() > 2)
        .collect::<Vec<String>>()
}

impl LocalFileRetrievalService {
    pub fn load(path: &str) -> Result<Self> {
        Ok(Self {
            chunks: load_retrieval_chunks(path, "local")?,
        })
    }
}

impl RetrievalService for LocalFileRetrievalService {
    fn backend_name(&self) -> &'static str {
        "local"
    }

    fn retrieve(&self, query: &str, max_chunks: usize) -> Result<Vec<RetrievedChunk>> {
        let terms = query_terms(query);

        if terms.is_empty() {
            return Ok(Vec::new());
        }

        let mut scored = self
            .chunks
            .iter()
            .filter_map(|chunk| {
                let body = chunk.text.to_ascii_lowercase();
                let score = terms
                    .iter()
                    .map(|term| body.matches(term.as_str()).count())
                    .sum::<usize>();
                (score > 0).then_some(RetrievedChunk {
                    source: chunk.source.clone(),
                    text: chunk.text.clone(),
                    score,
                })
            })
            .collect::<Vec<RetrievedChunk>>();

        scored.sort_by_key(|chunk| std::cmp::Reverse(chunk.score));
        scored.truncate(max_chunks.max(1));
        Ok(scored)
    }
}

#[cfg(feature = "semantic-search")]
pub struct SemanticLocalRetrievalService {
    pub chunks: Vec<RetrievedChunk>,
}

#[cfg(feature = "semantic-search")]
impl SemanticLocalRetrievalService {
    pub fn load(path: &str) -> Result<Self> {
        Ok(Self {
            chunks: load_retrieval_chunks(path, "semantic")?,
        })
    }
}

#[cfg(feature = "semantic-search")]
impl RetrievalService for SemanticLocalRetrievalService {
    fn backend_name(&self) -> &'static str {
        "semantic"
    }

    fn retrieve(&self, query: &str, max_chunks: usize) -> Result<Vec<RetrievedChunk>> {
        let query_lower = query.to_ascii_lowercase();
        let terms = query_terms(query);
        if query_lower.trim().is_empty() {
            return Ok(Vec::new());
        }

        let mut scored = self
            .chunks
            .iter()
            .filter_map(|chunk| {
                let body = chunk.text.to_ascii_lowercase();
                let similarity = strsim::jaro_winkler(&query_lower, &body);
                let lexical_hits = terms
                    .iter()
                    .map(|term| body.matches(term.as_str()).count())
                    .sum::<usize>();
                let score = ((similarity * 1000.0) as usize) + (lexical_hits * 25);
                (score > 0).then_some(RetrievedChunk {
                    source: chunk.source.clone(),
                    text: chunk.text.clone(),
                    score,
                })
            })
            .collect::<Vec<RetrievedChunk>>();

        scored.sort_by_key(|chunk| std::cmp::Reverse(chunk.score));
        scored.truncate(max_chunks.max(1));
        Ok(scored)
    }
}

pub fn build_retrieval_service(cfg: &RuntimeConfig) -> Result<Arc<dyn RetrievalService>> {
    match cfg.retrieval_backend {
        RetrievalBackend::Disabled => Ok(Arc::new(DisabledRetrievalService)),
        RetrievalBackend::Local => {
            let path = cfg.retrieval_doc_path.as_deref().ok_or_else(|| {
                anyhow::anyhow!(
                    "retrieval backend 'local' requires --retrieval-doc-path or profile.retrieval_doc_path"
                )
            })?;
            let service = LocalFileRetrievalService::load(path)?;
            Ok(Arc::new(service))
        }
        RetrievalBackend::Semantic => {
            let path = cfg.retrieval_doc_path.as_deref().ok_or_else(|| {
                anyhow::anyhow!(
                    "retrieval backend 'semantic' requires --retrieval-doc-path or profile.retrieval_doc_path"
                )
            })?;

            #[cfg(feature = "semantic-search")]
            {
                let service = SemanticLocalRetrievalService::load(path)?;
                Ok(Arc::new(service))
            }

            #[cfg(not(feature = "semantic-search"))]
            {
                let _ = path;
                Err(anyhow::anyhow!(
                    "retrieval backend 'semantic' requires feature 'semantic-search'. Rebuild with: cargo run --features semantic-search -- ..."
                ))
            }
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct RetrievalPolicy {
    pub max_chunks: usize,
    pub max_chars: usize,
    pub min_score: usize,
}

pub fn augment_prompt_with_retrieval(
    retrieval: &dyn RetrievalService,
    prompt: &str,
    policy: RetrievalPolicy,
) -> Result<String> {
    let chunks = retrieval.retrieve(prompt, policy.max_chunks)?;
    let mut used_chars = 0usize;
    let mut filtered = Vec::new();

    for chunk in chunks {
        if chunk.score < policy.min_score {
            continue;
        }

        if used_chars >= policy.max_chars {
            break;
        }

        let remaining = policy.max_chars - used_chars;
        if remaining == 0 {
            break;
        }

        let mut text = chunk.text;
        if text.len() > remaining {
            text.truncate(remaining);
        }

        if text.trim().is_empty() {
            continue;
        }

        used_chars += text.len();
        filtered.push(RetrievedChunk {
            source: chunk.source,
            text,
            score: chunk.score,
        });
    }

    if filtered.is_empty() {
        return Ok(prompt.to_string());
    }

    let mut out = String::new();
    out.push_str("Retrieved context (use if relevant):\n");
    for (index, chunk) in filtered.iter().enumerate() {
        out.push_str(&format!(
            "[{}] {} (score={})\n{}\n",
            index + 1,
            chunk.source,
            chunk.score,
            chunk.text
        ));
    }
    out.push_str("\nUser request:\n");
    out.push_str(prompt);
    Ok(out)
}
