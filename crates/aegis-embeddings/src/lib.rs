//! Embedding provider contracts and per-Core embedding cache.
//!
//! Retrieval remains provider-independent. The only guaranteed local strategy
//! is deterministic feature hashing; it is a lexical embedding, not a learned
//! semantic model. A real Ollama embeddings adapter is provided for owners who
//! configure a local model.

mod embed;
mod error;
mod store;

pub use embed::{
    content_digest, cosine_similarity, EmbeddingProvider, HashingEmbeddingProvider,
    OllamaEmbeddingProvider,
};
pub use error::EmbeddingError;
pub use store::{
    EmbeddingRecord, EmbeddingStore, EMBEDDING_APPLICATION_ID, EMBEDDING_COMPONENT,
    EMBEDDING_DB_FILE, MAX_DIMENSIONS,
};
