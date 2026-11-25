//! Core-neutral Aegis Platform foundation for Batch 1.

mod service;
mod store;

pub use service::{InitializationResult, PlatformService, ServiceError};
pub use store::{PlatformIdentity, PlatformStore};

use thiserror::Error;

#[derive(Debug, Error)]
pub enum PlatformError {
    #[error("Platform storage failed: {0}")]
    Storage(#[from] aegis_storage::StorageError),
    #[error("Platform SQLite operation failed: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("Platform identity is missing")]
    IdentityMissing,
    #[error("invalid Platform state: {0}")]
    InvalidState(&'static str),
}
