//! Embedding errors.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum EmbeddingError {
    #[error("embedding storage failed: {0}")]
    Storage(#[from] aegis_storage::StorageError),
    #[error("Core storage failed: {0}")]
    CoreStorage(#[from] aegis_core_storage::CoreStorageError),
    #[error("embedding SQLite operation failed: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("embedding I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("embedding JSON failed: {0}")]
    Json(#[from] serde_json::Error),
    #[error("embedding database belongs to a different Core or instance")]
    OwnershipMismatch,
    #[error("invalid embedding provider URL: {0}")]
    InvalidUrl(String),
    #[error("unsupported embedding provider URL scheme: {0}")]
    UnsupportedScheme(String),
    #[error("embedding network error: {0}")]
    Network(String),
    #[error("embedding request timed out: {0}")]
    Timeout(String),
    #[error("embedding provider returned HTTP status {status}")]
    HttpStatus { status: u16, body: String },
    #[error("embedding provider returned an invalid response: {0}")]
    InvalidResponse(String),
    #[error("embedding provider is unavailable: {0}")]
    Unavailable(String),
    #[error("embedding dimensions are invalid or inconsistent")]
    Dimensions,
}
