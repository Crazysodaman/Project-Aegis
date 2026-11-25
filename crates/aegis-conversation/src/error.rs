//! Conversation errors.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum ConversationError {
    #[error("conversation storage failed: {0}")]
    Storage(#[from] aegis_storage::StorageError),
    #[error("Core storage failed: {0}")]
    CoreStorage(#[from] aegis_core_storage::CoreStorageError),
    #[error("conversation SQLite operation failed: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("conversation I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("conversation JSON failed: {0}")]
    Json(#[from] serde_json::Error),
    #[error("Core SDK failed: {0}")]
    Sdk(#[from] aegis_core_sdk::SdkError),
    #[error("inference failed: {0}")]
    Inference(#[from] aegis_cognition::InferenceError),
    #[error("embedding retrieval failed: {0}")]
    Embedding(#[from] aegis_embeddings::EmbeddingError),
    #[error("provider returned no visible response content")]
    EmptyResponse,
    #[error("tool execution failed: {0}")]
    Tool(#[from] aegis_tools::ToolError),
    #[error("Core contract failed: {0}")]
    Contract(#[from] aegis_contracts::core::CoreContractError),
    #[error("Core UUID is invalid")]
    InvalidCoreUuid,
    #[error("conversation database belongs to a different Core or instance")]
    OwnershipMismatch,
    #[error("conversation database ownership record is missing")]
    OwnershipMissing,
    #[error("conversation database is outside its Core directory")]
    ForeignPath,
    #[error("conversation database schema is unsupported")]
    UnsupportedSchema,
    #[error("conversation session was not found: {0}")]
    SessionNotFound(String),
    #[error("conversation session is not active: {0}")]
    SessionNotActive(String),
    #[error("invalid conversation input: {0}")]
    InvalidInput(&'static str),
    #[error("conversation database is corrupt: {0}")]
    CorruptData(&'static str),
}
