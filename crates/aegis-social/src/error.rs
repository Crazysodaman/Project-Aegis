//! Social and relationship errors.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum SocialError {
    #[error("social storage failed: {0}")]
    Storage(#[from] aegis_storage::StorageError),
    #[error("Core storage failed: {0}")]
    CoreStorage(#[from] aegis_core_storage::CoreStorageError),
    #[error("social SQLite operation failed: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("social I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("social JSON failed: {0}")]
    Json(#[from] serde_json::Error),
    #[error("Platform is not initialized at {0}")]
    NotInitialized(String),
    #[error("Core UUID is invalid")]
    InvalidCoreUuid,
    #[error("relationship database belongs to a different Core or instance")]
    OwnershipMismatch,
    #[error("group was not found: {0}")]
    GroupNotFound(String),
    #[error("group membership is required")]
    NotMember,
    #[error("only the owner or Platform may administer group membership")]
    NotAuthorized,
    #[error("member already exists")]
    DuplicateMember,
    #[error("member was not found")]
    MemberNotFound,
    #[error("invalid social input: {0}")]
    InvalidInput(&'static str),
    #[error("social data is corrupt: {0}")]
    CorruptData(&'static str),
}
