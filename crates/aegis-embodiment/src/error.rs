//! Embodiment and asset errors.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum EmbodimentError {
    #[error("embodiment storage failed: {0}")]
    Storage(#[from] aegis_storage::StorageError),
    #[error("Core storage failed: {0}")]
    CoreStorage(#[from] aegis_core_storage::CoreStorageError),
    #[error("embodiment SQLite operation failed: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("embodiment I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("embodiment JSON failed: {0}")]
    Json(#[from] serde_json::Error),
    #[error("Core UUID is invalid")]
    InvalidCoreUuid,
    #[error("asset database belongs to a different Core or instance")]
    OwnershipMismatch,
    #[error("asset or appearance record is missing")]
    NotFound,
    #[error("asset was deleted")]
    Deleted,
    #[error("asset content integrity check failed")]
    IntegrityMismatch,
    #[error("asset file is missing or is not a regular file")]
    AssetFileMissing,
    #[error("asset kind is invalid")]
    InvalidAssetKind,
    #[error("invalid embodiment input: {0}")]
    InvalidInput(&'static str),
    #[error("referenced asset has the wrong kind: {0}")]
    WrongAssetKind(String),
    #[error("embodiment database is corrupt: {0}")]
    CorruptData(&'static str),
}
