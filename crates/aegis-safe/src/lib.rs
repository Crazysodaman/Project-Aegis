//! Aegis Safe boundary.
//!
//! This crate owns owner proof, STOP state, and the append-only security audit.
//! It does not own Platform registry state or Core private data.

mod owner_proof;
mod store;

pub use owner_proof::{LocalOwnerProof, OWNER_PROOF_KIND};
pub use store::SafeStore;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum SafeError {
    #[error("Safe storage failed: {0}")]
    Storage(#[from] aegis_storage::StorageError),
    #[error("Safe SQLite operation failed: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("Safe file operation failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("owner proof has not been initialized")]
    OwnerNotInitialized,
    #[error("owner proof is already initialized")]
    OwnerAlreadyInitialized,
    #[error("owner proof is invalid or unreadable")]
    OwnerProofInvalid,
    #[error("owner authorization failed")]
    Unauthorized,
    #[error("invalid Safe input: {0}")]
    InvalidInput(&'static str),
    #[error("security audit chain is invalid")]
    AuditChainInvalid,
}
