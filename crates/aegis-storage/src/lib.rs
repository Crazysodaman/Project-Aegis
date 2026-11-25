//! Storage paths and SQLite guard utilities shared by Platform and Safe.
//!
//! This crate owns paths, application markers, schema revision checks, and
//! file permissions. It owns no domain tables and no authorization policy.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use chrono::Utc;
use rusqlite::{Connection, OpenFlags};
use thiserror::Error;

pub const PLATFORM_APPLICATION_ID: i64 = 0x4145_4750; // "AEGP"
pub const SAFE_APPLICATION_ID: i64 = 0x4145_4753; // "AEGS"
pub const SCHEMA_REVISION: i64 = 1;

#[derive(Debug, Error)]
pub enum StorageError {
    #[error("storage I/O failed: {0}")]
    Io(#[from] io::Error),
    #[error("SQLite operation failed: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("storage root must be absolute")]
    RelativeRoot,
    #[error("storage root is not a directory")]
    RootNotDirectory,
    #[error("storage root is a symbolic link")]
    SymlinkRoot,
    #[error("database path is not a regular file")]
    NotRegularFile,
    #[error("application identity mismatch: expected {expected}, found {actual}")]
    WrongApplicationId { expected: i64, actual: i64 },
    #[error("schema revision mismatch for {component}: expected {expected}, found {actual}")]
    SchemaMismatch {
        component: String,
        expected: i64,
        actual: i64,
    },
    #[error("clock is before the Unix epoch")]
    Clock,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoragePaths {
    root: PathBuf,
}

impl StoragePaths {
    pub fn new(root: impl AsRef<Path>) -> Result<Self, StorageError> {
        let root = root.as_ref();
        if !root.is_absolute() {
            return Err(StorageError::RelativeRoot);
        }
        fs::create_dir_all(root)?;
        let metadata = fs::symlink_metadata(root)?;
        if metadata.file_type().is_symlink() {
            return Err(StorageError::SymlinkRoot);
        }
        if !metadata.is_dir() {
            return Err(StorageError::RootNotDirectory);
        }
        set_dir_permissions(root)?;
        let root = fs::canonicalize(root)?;
        Ok(Self { root })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn platform_db(&self) -> PathBuf {
        self.root.join("platform.db")
    }

    pub fn safe_db(&self) -> PathBuf {
        self.root.join("safe.db")
    }

    pub fn owner_proof(&self) -> PathBuf {
        self.root.join("owner.proof")
    }
}

pub fn now_rfc3339() -> Result<String, StorageError> {
    Ok(Utc::now().to_rfc3339())
}

pub fn open_initialized(
    path: &Path,
    application_id: i64,
    component: &str,
) -> Result<Connection, StorageError> {
    let existed = path.exists();
    if existed {
        let metadata = fs::symlink_metadata(path)?;
        if !metadata.is_file() {
            return Err(StorageError::NotRegularFile);
        }
    }
    let connection = Connection::open(path)?;
    if existed {
        verify_application_id(&connection, application_id)?;
    } else {
        connection.execute_batch(&format!("PRAGMA application_id = {application_id};"))?;
    }
    connection.execute_batch(
        "PRAGMA foreign_keys = ON;
         PRAGMA journal_mode = DELETE;
         PRAGMA synchronous = FULL;",
    )?;
    ensure_schema(&connection, component)?;
    set_file_permissions(path)?;
    Ok(connection)
}

pub fn open_read_only(path: &Path, application_id: i64) -> Result<Connection, StorageError> {
    let connection = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    verify_application_id(&connection, application_id)?;
    Ok(connection)
}

pub fn verify_application_id(connection: &Connection, expected: i64) -> Result<(), StorageError> {
    let actual: i64 = connection.query_row("PRAGMA application_id", [], |row| row.get(0))?;
    if actual != expected {
        return Err(StorageError::WrongApplicationId { expected, actual });
    }
    Ok(())
}

fn ensure_schema(connection: &Connection, component: &str) -> Result<(), StorageError> {
    connection.execute_batch(
        "CREATE TABLE IF NOT EXISTS aegis_schema (
            component TEXT PRIMARY KEY,
            revision INTEGER NOT NULL CHECK(revision >= 1),
            created_at TEXT NOT NULL
        );",
    )?;
    let existing: Option<i64> = connection
        .query_row(
            "SELECT revision FROM aegis_schema WHERE component = ?1",
            [component],
            |row| row.get(0),
        )
        .optional()?;
    match existing {
        None => {
            connection.execute(
                "INSERT INTO aegis_schema(component, revision, created_at) VALUES(?1, ?2, ?3)",
                rusqlite::params![component, SCHEMA_REVISION, now_rfc3339()?],
            )?;
        }
        Some(actual) if actual != SCHEMA_REVISION => {
            return Err(StorageError::SchemaMismatch {
                component: component.to_owned(),
                expected: SCHEMA_REVISION,
                actual,
            });
        }
        Some(_) => {}
    }
    Ok(())
}

use rusqlite::OptionalExtension;

#[cfg(unix)]
fn set_dir_permissions(path: &Path) -> Result<(), StorageError> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    Ok(())
}

#[cfg(not(unix))]
fn set_dir_permissions(_path: &Path) -> Result<(), StorageError> {
    Ok(())
}

#[cfg(unix)]
fn set_file_permissions(path: &Path) -> Result<(), StorageError> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    Ok(())
}

#[cfg(not(unix))]
fn set_file_permissions(_path: &Path) -> Result<(), StorageError> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn platform_and_safe_are_distinct_files_and_markers() {
        let temp = tempfile::tempdir().expect("tempdir");
        let paths = StoragePaths::new(temp.path()).expect("paths");
        let platform = open_initialized(&paths.platform_db(), PLATFORM_APPLICATION_ID, "platform")
            .expect("platform db");
        let safe =
            open_initialized(&paths.safe_db(), SAFE_APPLICATION_ID, "safe").expect("safe db");
        assert_ne!(paths.platform_db(), paths.safe_db());
        assert_eq!(
            platform
                .query_row("PRAGMA application_id", [], |row| row.get::<_, i64>(0))
                .expect("platform marker"),
            PLATFORM_APPLICATION_ID
        );
        assert_eq!(
            safe.query_row("PRAGMA application_id", [], |row| row.get::<_, i64>(0))
                .expect("safe marker"),
            SAFE_APPLICATION_ID
        );
    }

    #[test]
    fn wrong_application_marker_is_rejected() {
        let temp = tempfile::tempdir().expect("tempdir");
        let paths = StoragePaths::new(temp.path()).expect("paths");
        open_initialized(&paths.safe_db(), SAFE_APPLICATION_ID, "safe").expect("safe db");
        let error = open_initialized(&paths.safe_db(), PLATFORM_APPLICATION_ID, "platform")
            .expect_err("wrong marker must fail");
        assert!(matches!(error, StorageError::WrongApplicationId { .. }));
    }

    #[test]
    fn relative_root_is_rejected() {
        assert!(matches!(
            StoragePaths::new("relative-root"),
            Err(StorageError::RelativeRoot)
        ));
    }

    #[test]
    fn schema_revision_mismatch_is_rejected() {
        let temp = tempfile::tempdir().expect("tempdir");
        let paths = StoragePaths::new(temp.path()).expect("paths");
        {
            let connection =
                open_initialized(&paths.platform_db(), PLATFORM_APPLICATION_ID, "platform")
                    .expect("platform db");
            connection
                .execute(
                    "UPDATE aegis_schema SET revision = 2 WHERE component = 'platform'",
                    [],
                )
                .expect("bump revision");
        }
        let error = open_initialized(&paths.platform_db(), PLATFORM_APPLICATION_ID, "platform")
            .expect_err("revision mismatch must fail");
        assert!(matches!(error, StorageError::SchemaMismatch { .. }));
    }

    #[test]
    fn application_marker_survives_reopen() {
        let temp = tempfile::tempdir().expect("tempdir");
        let paths = StoragePaths::new(temp.path()).expect("paths");
        open_initialized(&paths.safe_db(), SAFE_APPLICATION_ID, "safe").expect("first open");
        open_initialized(&paths.safe_db(), SAFE_APPLICATION_ID, "safe").expect("second open");
    }
}
