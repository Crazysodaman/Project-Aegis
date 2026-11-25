//! Embedding-backed memory retrieval with an explicit keyword fallback.
//!
//! The retriever embeds the current user turn and each live private memory,
//! caches vectors in the Core's `embeddings.db`, ranks by cosine similarity, and
//! reports the method actually used. If the embedding provider or cache is
//! unavailable, it falls back to keyword retrieval and says so.

use std::path::Path;

use aegis_core_sdk::{CoreMemoryStore, MemoryEntry, MemoryQuery, SdkError};
use aegis_embeddings::{content_digest, cosine_similarity, EmbeddingProvider, EmbeddingStore};

use crate::error::ConversationError;
use crate::retrieval::{KeywordRetriever, MemoryRetriever, RetrievalOutcome};

/// Maximum memories considered for semantic ranking in one call.
pub const MAX_SEMANTIC_CANDIDATES: usize = 500;

#[derive(Debug)]
pub struct EmbeddingRetriever {
    provider: Box<dyn EmbeddingProvider>,
    store: EmbeddingStore,
    fallback: KeywordRetriever,
}

impl EmbeddingRetriever {
    pub fn open(
        root: impl AsRef<Path>,
        core_uuid: &str,
        provider: Box<dyn EmbeddingProvider>,
    ) -> Result<Self, ConversationError> {
        let store = EmbeddingStore::open(root, core_uuid)?;
        Ok(Self {
            provider,
            store,
            fallback: KeywordRetriever,
        })
    }

    pub fn provider_name(&self) -> &str {
        self.provider.name()
    }

    pub fn model(&self) -> &str {
        self.provider.model()
    }

    pub fn store(&self) -> &EmbeddingStore {
        &self.store
    }

    fn run(
        &self,
        memory: &CoreMemoryStore,
        text: &str,
        limit: usize,
    ) -> Result<(Vec<MemoryEntry>, String, bool), SdkError> {
        let limit = limit.min(50);
        if limit == 0 {
            return Ok((Vec::new(), self.method(), self.provider.semantic()));
        }
        let candidates = memory.search(&MemoryQuery::new().with_limit(MAX_SEMANTIC_CANDIDATES))?;
        if candidates.is_empty() {
            return Ok((Vec::new(), self.method(), self.provider.semantic()));
        }

        let mut cache_error = false;
        let mut vectors: Vec<Option<Vec<f32>>> = vec![None; candidates.len()];
        let mut missing = Vec::new();
        for (index, entry) in candidates.iter().enumerate() {
            let digest = content_digest(&entry.content);
            match self.store.get(
                &entry.id,
                self.provider.name(),
                self.provider.model(),
                &digest,
            ) {
                Ok(Some(vector)) => vectors[index] = Some(vector),
                Ok(None) => missing.push((index, digest)),
                Err(_) => {
                    cache_error = true;
                    missing.push((index, digest));
                }
            }
        }

        if !missing.is_empty() {
            let texts: Vec<String> = missing
                .iter()
                .map(|(index, _)| candidates[*index].content.clone())
                .collect();
            let embedded = match self.provider.embed(&texts) {
                Ok(vectors) => vectors,
                Err(error) => return self.fallback(memory, text, limit, &error.to_string()),
            };
            if embedded.len() != missing.len() {
                return self.fallback(memory, text, limit, "embedding count mismatch");
            }
            for ((index, digest), vector) in missing.into_iter().zip(embedded.into_iter()) {
                if vector.is_empty() || vector.iter().any(|value| !value.is_finite()) {
                    return self.fallback(memory, text, limit, "invalid embedding");
                }
                if self
                    .store
                    .put(
                        &candidates[index].id,
                        self.provider.name(),
                        self.provider.model(),
                        &digest,
                        &vector,
                    )
                    .is_err()
                {
                    cache_error = true;
                }
                vectors[index] = Some(vector);
            }
        }

        let query = match self.provider.embed(&[text.to_owned()]) {
            Ok(mut vectors) if vectors.len() == 1 => vectors.remove(0),
            Ok(_) => return self.fallback(memory, text, limit, "query embedding count"),
            Err(error) => return self.fallback(memory, text, limit, &error.to_string()),
        };
        if query.is_empty() || query.iter().any(|value| !value.is_finite()) {
            return self.fallback(memory, text, limit, "invalid query embedding");
        }

        let mut scored: Vec<(f32, &MemoryEntry)> = Vec::new();
        for (entry, vector) in candidates.iter().zip(vectors.iter()) {
            let Some(vector) = vector else {
                continue;
            };
            if vector.len() != query.len() {
                continue;
            }
            let score = cosine_similarity(&query, vector);
            if score > 0.0 {
                scored.push((score, entry));
            }
        }
        if scored.is_empty() {
            // A semantic pass ran but found no positive similarity.
            return Ok((Vec::new(), self.method(), self.provider.semantic()));
        }
        scored.sort_by(|left, right| {
            right
                .0
                .partial_cmp(&left.0)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| left.1.created_at.cmp(&right.1.created_at))
                .then_with(|| left.1.id.cmp(&right.1.id))
        });
        let entries = scored
            .into_iter()
            .take(limit)
            .map(|(_, entry)| entry.clone())
            .collect();
        let method = if cache_error {
            format!("{} (cache_error)", self.method())
        } else {
            self.method()
        };
        Ok((entries, method, self.provider.semantic()))
    }

    fn method(&self) -> String {
        format!(
            "embedding:{}:{}",
            self.provider.name(),
            self.provider.model()
        )
    }

    fn fallback(
        &self,
        memory: &CoreMemoryStore,
        text: &str,
        limit: usize,
        reason: &str,
    ) -> Result<(Vec<MemoryEntry>, String, bool), SdkError> {
        let outcome = self.fallback.retrieve_with_meta(memory, text, limit)?;
        Ok((
            outcome.entries,
            format!("keyword(fallback: {reason})"),
            false,
        ))
    }
}

impl MemoryRetriever for EmbeddingRetriever {
    fn name(&self) -> &'static str {
        "embedding"
    }

    fn semantic(&self) -> bool {
        self.provider.semantic()
    }

    fn retrieve(
        &self,
        memory: &CoreMemoryStore,
        text: &str,
        limit: usize,
    ) -> Result<Vec<MemoryEntry>, SdkError> {
        Ok(self.run(memory, text, limit)?.0)
    }

    fn retrieve_with_meta(
        &self,
        memory: &CoreMemoryStore,
        text: &str,
        limit: usize,
    ) -> Result<RetrievalOutcome, SdkError> {
        let (entries, method, semantic) = self.run(memory, text, limit)?;
        Ok(RetrievalOutcome {
            entries,
            method,
            semantic,
        })
    }
}
