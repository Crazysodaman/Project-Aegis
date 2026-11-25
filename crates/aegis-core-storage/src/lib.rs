//! Per-Core SQLite storage (Batch 2 / Wave 1b).
//!
//! This crate owns the composition, isolation, ownership, and lifecycle of the
//! single SQLite database that belongs to one Core. It never touches the
//! Platform or Safe databases and never opens legacy Sofia data.

use std::ffi::OsStr;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use aegis_contracts::core::{
    validate_core_id, validate_display_name, CoreContractError, CoreIdentity, CoreStorageMetadata,
    CORE_IDENTITY_SCHEMA, CORE_IDENTITY_TABLE, CORE_STORAGE_SCHEMA,
};
use aegis_contracts::initialization::FRESH_INITIALIZATION_MODE;
use aegis_storage::{
    open_initialized, open_read_only, verify_application_id, StorageError, SCHEMA_REVISION,
};
use rusqlite::{params, Connection, OptionalExtension};
use thiserror::Error;
use uuid::Uuid;

/// SQLite application marker for a per-Core database ("AEGC").
pub const CORE_APPLICATION_ID: i64 = 0x4145_4743;

/// File name of the Core-owned SQLite database.
pub const CORE_DB_FILE: &str = "core.db";
/// File name of the Core integrity manifest.
pub const CORE_INTEGRITY_FILE: &str = "core.integrity.json";
/// File name of the Core initialization marker.
pub const CORE_INITIALIZATION_MARKER_FILE: &str = "initialization.marker";
/// Component name registered in the shared `aegis_schema` table.
pub const CORE_COMPONENT: &str = "core";

/// Errors returned by per-Core storage composition.
#[derive(Debug, Error)]
pub enum CoreStorageError {
    #[error("storage root must be absolute")]
    RelativeRoot,
    #[error("storage root is not a directory")]
    RootNotDirectory,
    #[error("storage root is redirected (symlink or reparse point)")]
    RedirectedRoot,
    #[error("cores root is not a directory")]
    CoresRootNotDirectory,
    #[error("storage component is redirected (symlink or reparse point)")]
    RedirectedComponent,
    #[error("Core identifier is invalid")]
    InvalidCoreId,
    #[error("Core path is outside the approved cores root")]
    ForeignPath,
    #[error("Core directory is not a directory")]
    CoreDirectoryNotDirectory,
    #[error("Core directory is in a partial or interrupted state")]
    PartialState,
    #[error("Core database is missing")]
    DatabaseMissing,
    #[error("Core database is not a regular file")]
    DatabaseNotRegularFile,
    #[error("Core database belongs to a different Core")]
    ForeignOwnership,
    #[error("Core identity row is missing")]
    IdentityMissing,
    #[error("Core identity is invalid")]
    InvalidIdentity,
    #[error("SQLite integrity check failed: {0}")]
    IntegrityFailure(String),
    #[error("storage I/O failed: {0}")]
    Io(#[from] io::Error),
    #[error("SQLite operation failed: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("storage guard rejected the operation: {0}")]
    Storage(StorageError),
    #[error("Core contract violation: {0}")]
    Contract(#[from] CoreContractError),
}

impl From<StorageError> for CoreStorageError {
    fn from(error: StorageError) -> Self {
        match error {
            StorageError::WrongApplicationId { .. } => Self::ForeignOwnership,
            other => Self::Storage(other),
        }
    }
}

/// Approved on-disk layout for all Core databases.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoreStorageLayout {
    root: PathBuf,
    cores_root: PathBuf,
    init_root: PathBuf,
}

impl CoreStorageLayout {
    /// Build a layout rooted at an absolute directory.
    ///
    /// The root must not be redirected. A `<root>/cores` directory is created
    /// and becomes the only approved parent for Core databases.
    pub fn new(root: impl AsRef<Path>) -> Result<Self, CoreStorageError> {
        let root = root.as_ref();
        if !root.is_absolute() {
            return Err(CoreStorageError::RelativeRoot);
        }

        fs::create_dir_all(root)?;
        let root_metadata = fs::symlink_metadata(root)?;
        if is_redirected(&root_metadata) {
            return Err(CoreStorageError::RedirectedRoot);
        }
        if !root_metadata.is_dir() {
            return Err(CoreStorageError::RootNotDirectory);
        }
        let root = fs::canonicalize(root)?;

        let cores_root = root.join("cores");
        fs::create_dir_all(&cores_root)?;
        let cores_metadata = fs::symlink_metadata(&cores_root)?;
        if is_redirected(&cores_metadata) {
            return Err(CoreStorageError::RedirectedRoot);
        }
        if !cores_metadata.is_dir() {
            return Err(CoreStorageError::CoresRootNotDirectory);
        }
        let cores_root = fs::canonicalize(&cores_root)?;
        if !cores_root.starts_with(&root) {
            return Err(CoreStorageError::ForeignPath);
        }

        let init_root = root.join("init");
        fs::create_dir_all(&init_root)?;
        let init_metadata = fs::symlink_metadata(&init_root)?;
        if is_redirected(&init_metadata) {
            return Err(CoreStorageError::RedirectedRoot);
        }
        if !init_metadata.is_dir() {
            return Err(CoreStorageError::CoresRootNotDirectory);
        }
        let init_root = fs::canonicalize(&init_root)?;
        if !init_root.starts_with(&root) {
            return Err(CoreStorageError::ForeignPath);
        }

        Ok(Self {
            root,
            cores_root,
            init_root,
        })
    }

    /// Canonical storage root.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Canonical approved parent directory for Core databases.
    pub fn cores_root(&self) -> &Path {
        &self.cores_root
    }

    /// Directory that holds one Core's database and sidecar files.
    pub fn core_dir(&self, core_uuid: &str) -> Result<PathBuf, CoreStorageError> {
        let normalized = normalize_core_id(core_uuid)?;
        let path = self.cores_root.join(normalized);
        self.ensure_approved(&path)?;
        Ok(path)
    }

    /// SQLite database path for one Core.
    pub fn database_path(&self, core_uuid: &str) -> Result<PathBuf, CoreStorageError> {
        let directory = self.core_dir(core_uuid)?;
        let path = directory.join(CORE_DB_FILE);
        self.ensure_approved_database(&path, &directory)?;
        Ok(path)
    }

    /// Integrity manifest path for one Core.
    pub fn integrity_path(&self, core_uuid: &str) -> Result<PathBuf, CoreStorageError> {
        let directory = self.core_dir(core_uuid)?;
        let path = directory.join(CORE_INTEGRITY_FILE);
        self.ensure_approved(&path)?;
        Ok(path)
    }

    /// Initialization marker path for one Core.
    ///
    /// The marker lives in a separate `init` directory so an interrupted
    /// initialization does not make the Core database directory look like a
    /// non-empty, partially initialized database directory.
    pub fn initialization_marker_path(&self, core_uuid: &str) -> Result<PathBuf, CoreStorageError> {
        let normalized = normalize_core_id(core_uuid)?;
        let path = self
            .init_root
            .join(format!("{normalized}.{CORE_INITIALIZATION_MARKER_FILE}"));
        self.ensure_approved_init(&path)?;
        Ok(path)
    }

    fn ensure_approved_init(&self, path: &Path) -> Result<(), CoreStorageError> {
        if !path.starts_with(&self.init_root) {
            return Err(CoreStorageError::ForeignPath);
        }
        Ok(())
    }

    fn ensure_approved(&self, path: &Path) -> Result<(), CoreStorageError> {
        if !path.starts_with(&self.cores_root) {
            return Err(CoreStorageError::ForeignPath);
        }
        Ok(())
    }

    fn ensure_approved_database(
        &self,
        path: &Path,
        directory: &Path,
    ) -> Result<(), CoreStorageError> {
        self.ensure_approved(path)?;
        if path.file_name() != Some(OsStr::new(CORE_DB_FILE)) {
            return Err(CoreStorageError::ForeignPath);
        }
        if path != directory.join(CORE_DB_FILE) {
            return Err(CoreStorageError::ForeignPath);
        }
        if path == self.root.join("platform.db") || path == self.root.join("safe.db") {
            return Err(CoreStorageError::ForeignPath);
        }
        Ok(())
    }

    fn verify_database_file(&self, path: &Path) -> Result<(), CoreStorageError> {
        self.ensure_approved(path)?;
        let metadata = match fs::symlink_metadata(path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                return Err(CoreStorageError::DatabaseMissing)
            }
            Err(error) => return Err(error.into()),
        };
        if is_redirected(&metadata) {
            return Err(CoreStorageError::RedirectedComponent);
        }
        if !metadata.is_file() {
            return Err(CoreStorageError::DatabaseNotRegularFile);
        }
        let canonical = fs::canonicalize(path)?;
        self.ensure_approved(&canonical)?;
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct CoreStorageSnapshot {
    pub identity: CoreIdentity,
    pub metadata: CoreStorageMetadata,
    pub database_path: PathBuf,
}

/// One writable per-Core database handle.
///
/// The handle owns a `rusqlite::Connection`, so it is intentionally not
/// `Sync`. Callers must not share it across threads.
pub struct CoreDatabase {
    layout: CoreStorageLayout,
    core_uuid: String,
    database_path: PathBuf,
    connection: Connection,
}

impl std::fmt::Debug for CoreDatabase {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("CoreDatabase")
            .field("core_uuid", &self.core_uuid)
            .field("database_path", &self.database_path)
            .finish_non_exhaustive()
    }
}

impl CoreDatabase {
    /// Read identity and storage metadata without opening a writable handle.
    pub fn read_snapshot(
        layout: &CoreStorageLayout,
        core_uuid: &str,
    ) -> Result<CoreStorageSnapshot, CoreStorageError> {
        let core_uuid = normalize_core_id(core_uuid)?;
        let database_path = layout.database_path(&core_uuid)?;
        layout.verify_database_file(&database_path)?;
        let connection = open_read_only(&database_path, CORE_APPLICATION_ID)?;
        let stored = read_stored_identity(&connection, &core_uuid)?;
        let metadata = CoreStorageMetadata {
            schema: CORE_STORAGE_SCHEMA.to_owned(),
            core_uuid: stored.identity.core_uuid.clone(),
            database_instance_uuid: stored.database_instance_uuid,
            core_number: stored.identity.core_number,
            display_name: stored.identity.display_name.clone(),
            schema_revision: stored.schema_revision,
            initialization_mode: stored.initialization_mode,
            created_at: stored.identity.created_at.clone(),
        };
        metadata.validate()?;
        Ok(CoreStorageSnapshot {
            identity: stored.identity,
            metadata,
            database_path,
        })
    }

    /// Create a fresh Core database, or reopen the same Core if one already
    /// exists and can be verified safely.
    pub fn create_fresh(
        layout: &CoreStorageLayout,
        identity: &CoreIdentity,
    ) -> Result<Self, CoreStorageError> {
        identity.validate()?;

        let core_uuid = normalize_core_id(&identity.core_uuid)?;
        let directory = layout.core_dir(&core_uuid)?;
        let database_path = layout.database_path(&core_uuid)?;
        layout.ensure_approved_database(&database_path, &directory)?;

        if database_exists(&database_path)? {
            let existing = match Self::open(layout, &core_uuid) {
                Ok(existing) => existing,
                Err(CoreStorageError::IdentityMissing) => {
                    return Err(CoreStorageError::PartialState)
                }
                Err(error) => return Err(error),
            };
            let existing_identity = existing.identity()?;
            if existing_identity.core_number != identity.core_number {
                return Err(CoreStorageError::ForeignOwnership);
            }
            return Ok(existing);
        }

        match fs::symlink_metadata(&directory) {
            Ok(metadata) => {
                if is_redirected(&metadata) {
                    return Err(CoreStorageError::RedirectedComponent);
                }
                if !metadata.is_dir() {
                    return Err(CoreStorageError::CoreDirectoryNotDirectory);
                }
                if fs::read_dir(&directory)?.next().is_some() {
                    return Err(CoreStorageError::PartialState);
                }
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }

        fs::create_dir_all(&directory)?;
        let directory_metadata = fs::symlink_metadata(&directory)?;
        if is_redirected(&directory_metadata) {
            return Err(CoreStorageError::RedirectedComponent);
        }
        if !directory_metadata.is_dir() {
            return Err(CoreStorageError::CoreDirectoryNotDirectory);
        }
        let canonical_directory = fs::canonicalize(&directory)?;
        if !canonical_directory.starts_with(layout.cores_root()) {
            return Err(CoreStorageError::ForeignPath);
        }

        let connection = open_initialized(&database_path, CORE_APPLICATION_ID, CORE_COMPONENT)?;

        let database_instance_uuid = Uuid::new_v4().to_string();
        let write_result =
            write_fresh_identity(&connection, &core_uuid, identity, &database_instance_uuid);
        if let Err(error) = write_result {
            drop(connection);
            let _ = fs::remove_file(&database_path);
            return Err(error);
        }

        let stored = read_stored_identity(&connection, &core_uuid)?;
        if stored.identity.core_number != identity.core_number {
            drop(connection);
            let _ = fs::remove_file(&database_path);
            return Err(CoreStorageError::ForeignOwnership);
        }
        if stored.identity.display_name != identity.display_name {
            drop(connection);
            let _ = fs::remove_file(&database_path);
            return Err(CoreStorageError::InvalidIdentity);
        }

        layout.verify_database_file(&database_path)?;
        verify_application_id(&connection, CORE_APPLICATION_ID)?;

        Ok(Self {
            layout: layout.clone(),
            core_uuid,
            database_path,
            connection,
        })
    }

    /// Open an existing Core database after a read-only ownership check.
    pub fn open(layout: &CoreStorageLayout, core_uuid: &str) -> Result<Self, CoreStorageError> {
        let core_uuid = normalize_core_id(core_uuid)?;
        let database_path = layout.database_path(&core_uuid)?;
        layout.verify_database_file(&database_path)?;

        {
            let read_only = open_read_only(&database_path, CORE_APPLICATION_ID)?;
            read_stored_identity(&read_only, &core_uuid)?;
        }

        let connection = open_initialized(&database_path, CORE_APPLICATION_ID, CORE_COMPONENT)?;
        read_stored_identity(&connection, &core_uuid)?;
        verify_application_id(&connection, CORE_APPLICATION_ID)?;

        Ok(Self {
            layout: layout.clone(),
            core_uuid,
            database_path,
            connection,
        })
    }

    /// Current immutable identity plus mutable display name and revision.
    pub fn identity(&self) -> Result<CoreIdentity, CoreStorageError> {
        Ok(read_stored_identity(&self.connection, &self.core_uuid)?.identity)
    }

    /// Storage metadata derived from the validated identity row.
    pub fn metadata(&self) -> Result<CoreStorageMetadata, CoreStorageError> {
        let stored = read_stored_identity(&self.connection, &self.core_uuid)?;
        let metadata = CoreStorageMetadata {
            schema: CORE_STORAGE_SCHEMA.to_owned(),
            core_uuid: stored.identity.core_uuid.clone(),
            database_instance_uuid: stored.database_instance_uuid,
            core_number: stored.identity.core_number,
            display_name: stored.identity.display_name.clone(),
            schema_revision: stored.schema_revision,
            initialization_mode: stored.initialization_mode.clone(),
            created_at: stored.identity.created_at.clone(),
        };
        metadata.validate()?;
        Ok(metadata)
    }

    /// Change only the display name; UUID and Core number stay immutable.
    pub fn rename_display_name(
        &self,
        new_name: impl AsRef<str>,
    ) -> Result<CoreIdentity, CoreStorageError> {
        let new_name = new_name.as_ref();
        validate_display_name(new_name)?;

        let stored = read_stored_identity(&self.connection, &self.core_uuid)?;
        let renamed = stored.identity.with_display_name(new_name)?;
        let revision = i64::try_from(renamed.identity_revision)
            .map_err(|_| CoreStorageError::InvalidIdentity)?;

        let transaction = self.connection.unchecked_transaction()?;
        let changed = transaction.execute(
            "UPDATE core_identity
             SET display_name = ?1, identity_revision = ?2
             WHERE singleton = 1 AND core_uuid = ?3",
            params![renamed.display_name, revision, renamed.core_uuid],
        )?;
        if changed != 1 {
            return Err(CoreStorageError::IdentityMissing);
        }
        transaction.commit()?;

        Ok(renamed)
    }

    /// Run `PRAGMA integrity_check` against the open Core database.
    pub fn verify_sqlite_integrity(&self) -> Result<(), CoreStorageError> {
        let mut statement = self.connection.prepare("PRAGMA integrity_check")?;
        let mut rows = statement.query([])?;
        let mut messages = Vec::new();
        while let Some(row) = rows.next()? {
            messages.push(row.get::<_, String>(0)?);
        }
        if messages.len() == 1 && messages[0] == "ok" {
            Ok(())
        } else {
            Err(CoreStorageError::IntegrityFailure(messages.join("; ")))
        }
    }

    /// Path of the open Core database.
    pub fn database_path(&self) -> &Path {
        &self.database_path
    }

    /// Layout the handle was opened with.
    pub fn layout(&self) -> &CoreStorageLayout {
        &self.layout
    }

    /// Immutable Core UUID.
    pub fn core_uuid(&self) -> &str {
        &self.core_uuid
    }

    /// Per-database instance UUID (changes when a new database is created).
    pub fn database_instance_uuid(&self) -> Result<String, CoreStorageError> {
        Ok(read_stored_identity(&self.connection, &self.core_uuid)?.database_instance_uuid)
    }

    /// Close the SQLite connection and consume the handle.
    pub fn close(self) -> Result<(), CoreStorageError> {
        self.connection
            .close()
            .map_err(|(_, error)| CoreStorageError::Sqlite(error))
    }
}

struct StoredIdentity {
    identity: CoreIdentity,
    database_instance_uuid: String,
    initialization_mode: String,
    schema_revision: u64,
}

type StoredIdentityRow = (
    String,
    String,
    i64,
    String,
    String,
    i64,
    String,
    i64,
    String,
);

fn normalize_core_id(core_uuid: &str) -> Result<String, CoreStorageError> {
    let parsed = validate_core_id(core_uuid).map_err(|_| CoreStorageError::InvalidCoreId)?;
    Ok(parsed.hyphenated().to_string())
}

fn database_exists(path: &Path) -> Result<bool, CoreStorageError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            if is_redirected(&metadata) {
                Err(CoreStorageError::RedirectedComponent)
            } else if !metadata.is_file() {
                Err(CoreStorageError::DatabaseNotRegularFile)
            } else {
                Ok(true)
            }
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error.into()),
    }
}

fn write_fresh_identity(
    connection: &Connection,
    core_uuid: &str,
    identity: &CoreIdentity,
    database_instance_uuid: &str,
) -> Result<(), CoreStorageError> {
    let transaction = connection.unchecked_transaction()?;
    transaction.execute_batch(
        "CREATE TABLE IF NOT EXISTS core_identity (
            singleton INTEGER NOT NULL PRIMARY KEY CHECK (singleton = 1),
            schema TEXT NOT NULL,
            core_uuid TEXT NOT NULL,
            database_instance_uuid TEXT NOT NULL UNIQUE,
            core_number INTEGER NOT NULL CHECK (core_number BETWEEN 1 AND 255),
            display_name TEXT NOT NULL,
            created_at TEXT NOT NULL,
            identity_revision INTEGER NOT NULL CHECK (identity_revision >= 1),
            initialization_mode TEXT NOT NULL,
            schema_revision INTEGER NOT NULL CHECK (schema_revision >= 1)
        );",
    )?;
    let revision =
        i64::try_from(identity.identity_revision).map_err(|_| CoreStorageError::InvalidIdentity)?;
    transaction.execute(
        "INSERT INTO core_identity(
            singleton, schema, core_uuid, database_instance_uuid, core_number,
            display_name, created_at, identity_revision, initialization_mode,
            schema_revision
         ) VALUES(1, ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![
            CORE_IDENTITY_SCHEMA,
            core_uuid,
            database_instance_uuid,
            i64::from(identity.core_number),
            identity.display_name,
            identity.created_at,
            revision,
            FRESH_INITIALIZATION_MODE,
            SCHEMA_REVISION
        ],
    )?;
    transaction.commit()?;
    Ok(())
}

fn read_stored_identity(
    connection: &Connection,
    expected_core_uuid: &str,
) -> Result<StoredIdentity, CoreStorageError> {
    let expected =
        validate_core_id(expected_core_uuid).map_err(|_| CoreStorageError::InvalidCoreId)?;

    let has_table: Option<String> = connection
        .query_row(
            "SELECT name FROM sqlite_master WHERE type = 'table' AND name = ?1",
            [CORE_IDENTITY_TABLE],
            |row| row.get(0),
        )
        .optional()?;
    if has_table.is_none() {
        return Err(CoreStorageError::IdentityMissing);
    }

    let row: Option<StoredIdentityRow> = connection
        .query_row(
            "SELECT schema, core_uuid, core_number, display_name, created_at,
                    identity_revision, initialization_mode, schema_revision,
                    database_instance_uuid
             FROM core_identity WHERE singleton = 1",
            [],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                    row.get(6)?,
                    row.get(7)?,
                    row.get(8)?,
                ))
            },
        )
        .optional()?;
    let Some((
        schema,
        stored_uuid,
        core_number,
        display_name,
        created_at,
        revision,
        initialization_mode,
        schema_revision,
        database_instance_uuid,
    )) = row
    else {
        return Err(CoreStorageError::IdentityMissing);
    };

    if schema != CORE_IDENTITY_SCHEMA {
        return Err(CoreStorageError::InvalidIdentity);
    }
    let stored_uuid =
        validate_core_id(&stored_uuid).map_err(|_| CoreStorageError::ForeignOwnership)?;
    if stored_uuid != expected {
        return Err(CoreStorageError::ForeignOwnership);
    }
    let database_instance =
        validate_core_id(&database_instance_uuid).map_err(|_| CoreStorageError::InvalidIdentity)?;
    if database_instance == stored_uuid {
        return Err(CoreStorageError::InvalidIdentity);
    }

    let core_number = u8::try_from(core_number).map_err(|_| CoreStorageError::InvalidIdentity)?;
    let identity_revision =
        u64::try_from(revision).map_err(|_| CoreStorageError::InvalidIdentity)?;
    let schema_revision =
        u64::try_from(schema_revision).map_err(|_| CoreStorageError::InvalidIdentity)?;
    if initialization_mode.trim().is_empty() {
        return Err(CoreStorageError::InvalidIdentity);
    }

    let identity = CoreIdentity {
        schema: CORE_IDENTITY_SCHEMA.to_owned(),
        core_uuid: stored_uuid.hyphenated().to_string(),
        core_number,
        display_name,
        created_at,
        identity_revision,
    };
    identity.validate()?;

    Ok(StoredIdentity {
        identity,
        database_instance_uuid: database_instance.hyphenated().to_string(),
        initialization_mode,
        schema_revision,
    })
}

#[cfg(windows)]
fn is_redirected(metadata: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;

    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0000_0400;
    metadata.file_type().is_symlink()
        || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

#[cfg(not(windows))]
fn is_redirected(metadata: &fs::Metadata) -> bool {
    metadata.file_type().is_symlink()
}
