use std::io;
use std::path::PathBuf;

use aegis_contracts::integrity::IntegrityStatus;
use aegis_core_integrity::IntegrityError;
use aegis_core_storage::CoreStorageError;
use thiserror::Error;

/// Errors returned by backup, verification, and restore.
///
/// No variant carries file contents. Paths and digests are safe to display;
/// `owner.proof` bytes are never included.
#[derive(Debug, Error)]
pub enum BackupError {
    #[error("source root is not a valid Aegis Platform root: {0}")]
    InvalidSource(String),
    #[error("backup destination already exists: {0}")]
    DestinationExists(PathBuf),
    #[error("backup destination is inside the source root")]
    DestinationInsideSource,
    #[error("source root is inside the backup destination")]
    SourceInsideDestination,
    #[error("unsafe path: {0}")]
    UnsafePath(String),
    #[error("required file is missing: {0}")]
    MissingFile(PathBuf),
    #[error("unsupported file (not a regular file): {0}")]
    UnsupportedFile(PathBuf),
    #[error("digest mismatch for {path}: expected {expected}, actual {actual}")]
    DigestMismatch {
        path: PathBuf,
        expected: String,
        actual: String,
    },
    #[error("Core {core_uuid} integrity failed ({status:?}): {detail}")]
    IntegrityFailed {
        core_uuid: String,
        status: IntegrityStatus,
        detail: String,
    },
    #[error("restore target is not empty: {0}")]
    RestoreTargetNotEmpty(PathBuf),
    #[error("restore target is inside the backup")]
    RestoreTargetInsideBackup,
    #[error("backup is inside the restore target")]
    BackupInsideRestoreTarget,
    #[error("Core {0} appears to be running; refusing to continue")]
    RunningCore(String),
    #[error("backup is invalid: {0}")]
    InvalidBackup(String),
    #[error("storage I/O failed: {0}")]
    Io(#[from] io::Error),
    #[error("SQLite operation failed: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("backup JSON failed: {0}")]
    Json(#[from] serde_json::Error),
    #[error("storage paths failed: {0}")]
    Storage(#[from] aegis_storage::StorageError),
    #[error("Core storage failed: {0}")]
    CoreStorage(#[from] CoreStorageError),
    #[error("Core integrity failed: {0}")]
    Integrity(#[from] IntegrityError),
}
