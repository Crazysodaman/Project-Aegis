//! Core database integrity manifests and verification.
//!
//! This crate provides content-integrity primitives for a Core SQLite
//! database: SHA-256 digests, an atomically written integrity manifest, and a
//! non-panicking verification report.
//!
//! # Integrity is not authenticity
//!
//! A digest establishes **content integrity only**. It answers "are these the
//! exact bytes that were sealed?" It does **not** establish authenticity,
//! package trust, publisher identity, or installation authorization. An
//! attacker who can rewrite both the database and the manifest can produce a
//! self-consistent pair of files. Trust in a manifest must come from a
//! separate, authenticated channel that is outside this crate.
//!
//! # Canonical identity JSON
//!
//! [`compute_identity_sha256`] hashes the UTF-8 JSON produced by
//! `serde_json::to_vec` for [`CoreIdentity`]. Canonicalization relies on:
//!
//! * struct fields being serialized in their declared order, and
//! * `serde_json`'s default map ordering (`BTreeMap`, i.e. sorted keys). The
//!   default map ordering is acceptable because this crate does not enable the
//!   `preserve_order` feature and the identity contract currently has no map
//!   fields.
//!
//! No whitespace, trailing newline, or other formatting is included. Changing
//! identity field order or types is a schema change and must be versioned.

use std::fmt::Write as _;
use std::fs::{self, File};
use std::io::{self, Read, Write as _};
use std::path::{Path, PathBuf};

use aegis_contracts::core::CoreContractError;
use aegis_contracts::integrity::{
    IntegrityContractError, DIGEST_ALGORITHM_SHA256, INTEGRITY_MANIFEST_SCHEMA,
    INTEGRITY_REPORT_SCHEMA,
};
use chrono::{SecondsFormat, Utc};
use rusqlite::{Connection, OpenFlags, OptionalExtension};
use sha2::{Digest, Sha256};
use tempfile::NamedTempFile;
use thiserror::Error;

pub use aegis_contracts::core::{CoreIdentity, CORE_IDENTITY_TABLE};
pub use aegis_contracts::integrity::{IntegrityManifest, IntegrityReport, IntegrityStatus};

/// Component name used by the Core database schema marker.
pub const CORE_SCHEMA_COMPONENT: &str = "core";

const READ_CHUNK_BYTES: usize = 64 * 1024;

#[derive(Debug, Error)]
pub enum IntegrityError {
    #[error("integrity I/O failed: {0}")]
    Io(#[from] io::Error),
    #[error("SQLite operation failed: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("integrity JSON failed: {0}")]
    Json(#[from] serde_json::Error),
    #[error("integrity manifest contract is invalid: {0}")]
    ManifestContract(#[from] IntegrityContractError),
    #[error("Core identity contract is invalid: {0}")]
    IdentityContract(#[from] CoreContractError),
    #[error("database path has no valid UTF-8 file name")]
    MissingFileName,
    #[error("database file does not exist: {0}")]
    DatabaseMissing(PathBuf),
    #[error("Core UUID does not match the supplied identity")]
    IdentityMismatch,
    #[error("schema revision is invalid")]
    InvalidSchemaRevision,
}

pub type Result<T> = std::result::Result<T, IntegrityError>;

/// Hash the exact file bytes at `path` with SHA-256, returning lowercase hex.
pub fn compute_file_sha256(path: impl AsRef<Path>) -> Result<String> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; READ_CHUNK_BYTES];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hex_lower(&hasher.finalize()))
}

/// Hash an in-memory byte slice with SHA-256, returning lowercase hex.
pub fn compute_bytes_sha256(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hex_lower(&hasher.finalize())
}

/// Hash the canonical identity JSON described at the crate root.
///
/// The identity is validated before hashing so that every digest corresponds
/// to a contract-valid [`CoreIdentity`].
pub fn compute_identity_sha256(identity: &CoreIdentity) -> Result<String> {
    identity.validate()?;
    let canonical = serde_json::to_vec(identity)?;
    Ok(compute_bytes_sha256(&canonical))
}

/// Read the schema revision for `component` from `aegis_schema`.
pub fn schema_revision_for_component(path: impl AsRef<Path>, component: &str) -> Result<u64> {
    let connection = connection_read_only(path.as_ref())?;
    let revision: i64 = connection.query_row(
        "SELECT revision FROM aegis_schema WHERE component = ?1",
        [component],
        |row| row.get(0),
    )?;
    u64::try_from(revision)
        .ok()
        .filter(|revision| *revision >= 1)
        .ok_or(IntegrityError::InvalidSchemaRevision)
}

/// Read the Core schema revision from the `core` row of `aegis_schema`.
pub fn schema_revision(path: impl AsRef<Path>) -> Result<u64> {
    schema_revision_for_component(path, CORE_SCHEMA_COMPONENT)
}

/// Atomic reader/writer for an [`IntegrityManifest`] JSON file.
#[derive(Debug, Clone)]
pub struct IntegrityManifestStore {
    path: PathBuf,
}

impl IntegrityManifestStore {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Validate and atomically replace the manifest file.
    ///
    /// The JSON is written to a temporary file in the same directory, flushed
    /// to disk, and then persisted over the destination so a crash cannot leave
    /// a partially written manifest.
    pub fn write(&self, manifest: &IntegrityManifest) -> Result<()> {
        manifest.validate()?;
        let mut payload = serde_json::to_vec_pretty(manifest)?;
        payload.push(b'\n');

        let parent = self
            .path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        let mut temporary = NamedTempFile::new_in(parent)?;
        temporary.write_all(&payload)?;
        temporary.as_file().sync_all()?;
        temporary
            .persist(&self.path)
            .map_err(|error| IntegrityError::Io(error.error))?;
        set_manifest_permissions(&self.path)?;
        Ok(())
    }

    /// Read and validate the manifest, or `None` when the file is absent.
    pub fn read(&self) -> Result<Option<IntegrityManifest>> {
        let bytes = match fs::read(&self.path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.into()),
        };
        let manifest: IntegrityManifest = serde_json::from_slice(&bytes)?;
        manifest.validate()?;
        Ok(Some(manifest))
    }
}

/// Seal a Core database by hashing its exact bytes into an integrity manifest.
///
/// The caller is responsible for closing or checkpointing the database first;
/// this function hashes whatever bytes are currently on disk. Only the
/// database file name is recorded, never its directory.
pub fn seal_manifest(
    core_uuid: &str,
    database_path: &Path,
    manifest_path: &Path,
    schema_revision: u64,
    identity: &CoreIdentity,
) -> Result<IntegrityManifest> {
    seal_manifest_with_metadata(
        core_uuid,
        database_path,
        manifest_path,
        schema_revision,
        identity,
        None,
        None,
        None,
    )
}

/// Seal a Core database and record database-instance identity fields.
///
/// The optional fields are cross-checked during [`verify_core`] when present.
/// Older manifests without them remain readable through serde defaults, but do
/// not get the stronger database-instance ownership check.
#[allow(clippy::too_many_arguments)]
pub fn seal_manifest_with_metadata(
    core_uuid: &str,
    database_path: &Path,
    manifest_path: &Path,
    schema_revision: u64,
    identity: &CoreIdentity,
    database_instance_uuid: Option<&str>,
    initialization_mode: Option<&str>,
    core_number: Option<u8>,
) -> Result<IntegrityManifest> {
    if identity.core_uuid != core_uuid {
        return Err(IntegrityError::IdentityMismatch);
    }

    let metadata = fs::metadata(database_path)?;
    if !metadata.is_file() {
        return Err(IntegrityError::DatabaseMissing(database_path.to_path_buf()));
    }

    let database_file = database_path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or(IntegrityError::MissingFileName)?
        .to_owned();
    let database_sha256 = compute_file_sha256(database_path)?;
    let identity_sha256 = compute_identity_sha256(identity)?;

    let manifest = IntegrityManifest {
        schema: INTEGRITY_MANIFEST_SCHEMA.to_owned(),
        core_uuid: core_uuid.to_owned(),
        database_file,
        database_sha256,
        database_bytes: metadata.len(),
        schema_revision,
        identity_sha256,
        algorithm: DIGEST_ALGORITHM_SHA256.to_owned(),
        created_at: now_rfc3339(),
        database_instance_uuid: database_instance_uuid.map(str::to_owned),
        initialization_mode: initialization_mode.map(str::to_owned),
        core_number,
    };
    manifest.validate()?;
    IntegrityManifestStore::new(manifest_path).write(&manifest)?;
    Ok(manifest)
}

/// Verify a Core database against its integrity manifest.
///
/// This function never panics. Every failure mode is represented by an
/// [`IntegrityStatus`] in a contract-validated [`IntegrityReport`]. The caller
/// must supply a valid Core UUID; that UUID is echoed into the report.
///
/// Checks run in this order:
///
/// 1. manifest present and contract-valid,
/// 2. manifest Core UUID matches the requested Core,
/// 3. database file present,
/// 4. read-only `PRAGMA integrity_check`,
/// 5. database digest and byte length match the manifest,
/// 6. database schema revision matches the expected revision and the manifest,
/// 7. when `expected_identity` is supplied, the manifest identity digest and
///    the stored `core_identity` row agree with it. A missing identity row is
///    an ownership failure for a Core database; it is never silently skipped.
///
/// The `core_identity` row is read from columns matching the identity
/// contract: `schema`, `core_uuid`, `core_number`, `display_name`,
/// `created_at`, and `identity_revision`.
#[allow(clippy::too_many_arguments)]
pub fn verify_core(
    core_uuid: &str,
    database_path: &Path,
    manifest_path: &Path,
    expected_schema_revision: u64,
    expected_identity: Option<&CoreIdentity>,
) -> IntegrityReport {
    let mut observations = Observations::default();
    let (status, detail) = evaluate(
        core_uuid,
        database_path,
        manifest_path,
        expected_schema_revision,
        expected_identity,
        &mut observations,
    );

    let report = IntegrityReport {
        schema: INTEGRITY_REPORT_SCHEMA.to_owned(),
        core_uuid: core_uuid.to_owned(),
        status,
        expected_sha256: observations.expected_sha256,
        actual_sha256: observations.actual_sha256,
        expected_schema_revision: Some(expected_schema_revision),
        actual_schema_revision: observations.actual_schema_revision,
        checked_at: now_rfc3339(),
        detail,
    };
    let _ = report.validate();
    report
}

#[derive(Default)]
struct Observations {
    expected_sha256: Option<String>,
    actual_sha256: Option<String>,
    actual_schema_revision: Option<u64>,
}

fn evaluate(
    core_uuid: &str,
    database_path: &Path,
    manifest_path: &Path,
    expected_schema_revision: u64,
    expected_identity: Option<&CoreIdentity>,
    observations: &mut Observations,
) -> (IntegrityStatus, Option<String>) {
    let manifest = match IntegrityManifestStore::new(manifest_path).read() {
        Ok(Some(manifest)) => manifest,
        Ok(None) => {
            return (
                IntegrityStatus::MissingManifest,
                Some(format!(
                    "integrity manifest is missing at {}",
                    manifest_path.display()
                )),
            )
        }
        Err(error) => {
            return (
                IntegrityStatus::InvalidManifest,
                Some(format!("integrity manifest is invalid: {error}")),
            )
        }
    };
    observations.expected_sha256 = Some(manifest.database_sha256.clone());

    if manifest.core_uuid != core_uuid {
        return (
            IntegrityStatus::OwnershipMismatch,
            Some(format!(
                "manifest Core UUID {} does not match requested Core UUID {core_uuid}",
                manifest.core_uuid
            )),
        );
    }

    if !database_path.is_file() {
        return (
            IntegrityStatus::DatabaseMissing,
            Some(format!(
                "database file is missing at {}",
                database_path.display()
            )),
        );
    }

    if let Some(sidecar) = unexpected_sidecar(database_path) {
        return (
            IntegrityStatus::DigestMismatch,
            Some(format!(
                "unexpected SQLite sidecar is present: {}",
                sidecar.display()
            )),
        );
    }

    if let Err(detail) = check_sqlite_integrity(database_path) {
        return (IntegrityStatus::SqliteCorrupt, Some(detail));
    }

    let actual_sha256 = match compute_file_sha256(database_path) {
        Ok(digest) => digest,
        Err(error) => {
            return (
                IntegrityStatus::SqliteCorrupt,
                Some(format!("database bytes could not be read: {error}")),
            )
        }
    };
    observations.actual_sha256 = Some(actual_sha256.clone());
    if actual_sha256 != manifest.database_sha256 {
        return (
            IntegrityStatus::DigestMismatch,
            Some(format!(
                "database digest does not match manifest (expected {}, actual {actual_sha256})",
                manifest.database_sha256
            )),
        );
    }

    let database_bytes = match fs::metadata(database_path) {
        Ok(metadata) => metadata.len(),
        Err(error) => {
            return (
                IntegrityStatus::DatabaseMissing,
                Some(format!("database metadata could not be read: {error}")),
            )
        }
    };
    if database_bytes != manifest.database_bytes {
        return (
            IntegrityStatus::DigestMismatch,
            Some(format!(
                "database byte length does not match manifest (expected {}, actual {database_bytes})",
                manifest.database_bytes
            )),
        );
    }

    let actual_schema_revision = match schema_revision(database_path) {
        Ok(revision) => revision,
        Err(error) => {
            return (
                IntegrityStatus::SchemaMismatch,
                Some(format!("schema revision could not be read: {error}")),
            )
        }
    };
    observations.actual_schema_revision = Some(actual_schema_revision);
    if actual_schema_revision != expected_schema_revision {
        return (
            IntegrityStatus::SchemaMismatch,
            Some(format!(
                "schema revision mismatch (expected {expected_schema_revision}, actual {actual_schema_revision})"
            )),
        );
    }
    if actual_schema_revision != manifest.schema_revision {
        return (
            IntegrityStatus::SchemaMismatch,
            Some(format!(
                "manifest schema revision {} does not match database revision {actual_schema_revision}",
                manifest.schema_revision
            )),
        );
    }

    if let Some(expected_identity) = expected_identity {
        let expected_digest = match compute_identity_sha256(expected_identity) {
            Ok(digest) => digest,
            Err(error) => {
                return (
                    IntegrityStatus::OwnershipMismatch,
                    Some(format!("expected identity is invalid: {error}")),
                )
            }
        };
        if manifest.identity_sha256 != expected_digest {
            return (
                IntegrityStatus::OwnershipMismatch,
                Some("manifest identity digest does not match the expected identity".to_owned()),
            );
        }

        match read_stored_identity(database_path) {
            Ok(Some(stored)) => {
                if stored.identity.core_uuid != expected_identity.core_uuid {
                    return (
                        IntegrityStatus::OwnershipMismatch,
                        Some(format!(
                            "stored Core UUID {} does not match expected Core UUID {}",
                            stored.identity.core_uuid, expected_identity.core_uuid
                        )),
                    );
                }
                match compute_identity_sha256(&stored.identity) {
                    Ok(stored_digest) if stored_digest == expected_digest => {}
                    Ok(_) => {
                        return (
                            IntegrityStatus::OwnershipMismatch,
                            Some("stored identity does not match the expected identity".to_owned()),
                        )
                    }
                    Err(error) => {
                        return (
                            IntegrityStatus::OwnershipMismatch,
                            Some(format!("stored identity is invalid: {error}")),
                        )
                    }
                }
                if let Some(instance) = &manifest.database_instance_uuid {
                    if &stored.database_instance_uuid != instance {
                        return (
                            IntegrityStatus::OwnershipMismatch,
                            Some(
                                "manifest database instance UUID does not match the database"
                                    .to_owned(),
                            ),
                        );
                    }
                }
                if let Some(mode) = &manifest.initialization_mode {
                    if &stored.initialization_mode != mode {
                        return (
                            IntegrityStatus::OwnershipMismatch,
                            Some(
                                "manifest initialization mode does not match the database"
                                    .to_owned(),
                            ),
                        );
                    }
                }
                if let Some(core_number) = manifest.core_number {
                    if stored.identity.core_number != core_number {
                        return (
                            IntegrityStatus::OwnershipMismatch,
                            Some(
                                "manifest Core number does not match the database identity"
                                    .to_owned(),
                            ),
                        );
                    }
                }
            }
            Ok(None) => {
                return (
                    IntegrityStatus::OwnershipMismatch,
                    Some("stored core_identity row is missing".to_owned()),
                )
            }
            Err(error) => {
                return (
                    IntegrityStatus::OwnershipMismatch,
                    Some(format!("stored identity could not be read: {error}")),
                )
            }
        }
    }

    (
        IntegrityStatus::Verified,
        Some(
            "database digest, schema revision, SQLite integrity, and ownership are consistent"
                .to_owned(),
        ),
    )
}

struct StoredIdentity {
    identity: CoreIdentity,
    database_instance_uuid: String,
    initialization_mode: String,
}

fn read_stored_identity(path: &Path) -> Result<Option<StoredIdentity>> {
    let connection = connection_read_only(path)?;
    let table_exists: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1)",
        [CORE_IDENTITY_TABLE],
        |row| row.get(0),
    )?;
    if !table_exists {
        return Ok(None);
    }
    let full = connection
        .query_row(
            "SELECT schema, core_uuid, core_number, display_name, created_at, identity_revision, \
                    database_instance_uuid, initialization_mode \
             FROM core_identity LIMIT 1",
            [],
            |row| {
                Ok(StoredIdentity {
                    identity: CoreIdentity {
                        schema: row.get(0)?,
                        core_uuid: row.get(1)?,
                        core_number: row.get(2)?,
                        display_name: row.get(3)?,
                        created_at: row.get(4)?,
                        identity_revision: row.get(5)?,
                    },
                    database_instance_uuid: row.get(6)?,
                    initialization_mode: row.get(7)?,
                })
            },
        )
        .optional();
    if let Ok(identity) = full {
        return Ok(identity);
    }

    let legacy = connection
        .query_row(
            "SELECT schema, core_uuid, core_number, display_name, created_at, identity_revision \
             FROM core_identity LIMIT 1",
            [],
            |row| {
                Ok(StoredIdentity {
                    identity: CoreIdentity {
                        schema: row.get(0)?,
                        core_uuid: row.get(1)?,
                        core_number: row.get(2)?,
                        display_name: row.get(3)?,
                        created_at: row.get(4)?,
                        identity_revision: row.get(5)?,
                    },
                    database_instance_uuid: String::new(),
                    initialization_mode: String::new(),
                })
            },
        )
        .optional()?;
    Ok(legacy)
}

fn check_sqlite_integrity(path: &Path) -> std::result::Result<(), String> {
    let connection = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|error| format!("SQLite could not be opened read-only: {error}"))?;
    match connection.query_row("PRAGMA integrity_check", [], |row| row.get::<_, String>(0)) {
        Ok(value) if value == "ok" => Ok(()),
        Ok(value) => Err(format!("SQLite integrity_check reported: {value}")),
        Err(error) => Err(format!("SQLite integrity_check failed: {error}")),
    }
}

fn unexpected_sidecar(database_path: &Path) -> Option<PathBuf> {
    for suffix in ["-wal", "-shm"] {
        let mut name = database_path.as_os_str().to_os_string();
        name.push(suffix);
        let path = PathBuf::from(name);
        if path.exists() {
            return Some(path);
        }
    }
    None
}

fn connection_read_only(path: &Path) -> Result<Connection> {
    Ok(Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY,
    )?)
}

fn hex_lower(bytes: &[u8]) -> String {
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        let _ = write!(encoded, "{byte:02x}");
    }
    encoded
}

fn now_rfc3339() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true)
}

#[cfg(unix)]
fn set_manifest_permissions(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    Ok(())
}

#[cfg(not(unix))]
fn set_manifest_permissions(_path: &Path) -> Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Seek, SeekFrom};

    const CORE_A: &str = "a9a0a604-a4e7-4a88-a985-3f071e2c8e7d";
    const CORE_B: &str = "b9a0a604-a4e7-4a88-a985-3f071e2c8e7d";
    const CREATED_AT: &str = "2026-11-22T00:00:00Z";

    fn identity(core_uuid: &str, display_name: &str) -> CoreIdentity {
        CoreIdentity::new(core_uuid, 1, display_name, CREATED_AT).expect("identity")
    }

    fn create_database(path: &Path, stored_identity: Option<&CoreIdentity>, revision: i64) {
        let connection = Connection::open(path).expect("open fixture database");
        connection
            .execute_batch(
                "CREATE TABLE aegis_schema (
                    component TEXT PRIMARY KEY,
                    revision INTEGER NOT NULL,
                    created_at TEXT NOT NULL
                );
                CREATE TABLE core_identity (
                    schema TEXT NOT NULL,
                    core_uuid TEXT NOT NULL,
                    core_number INTEGER NOT NULL,
                    display_name TEXT NOT NULL,
                    created_at TEXT NOT NULL,
                    identity_revision INTEGER NOT NULL
                );
                CREATE TABLE payload (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    value TEXT NOT NULL
                );",
            )
            .expect("create fixture schema");
        connection
            .execute(
                "INSERT INTO aegis_schema(component, revision, created_at) VALUES(?1, ?2, ?3)",
                rusqlite::params![CORE_SCHEMA_COMPONENT, revision, CREATED_AT],
            )
            .expect("insert schema revision");
        if let Some(stored) = stored_identity {
            connection
                .execute(
                    "INSERT INTO core_identity(
                        schema, core_uuid, core_number, display_name, created_at, identity_revision
                     ) VALUES(?1, ?2, ?3, ?4, ?5, ?6)",
                    rusqlite::params![
                        stored.schema,
                        stored.core_uuid,
                        stored.core_number,
                        stored.display_name,
                        stored.created_at,
                        stored.identity_revision,
                    ],
                )
                .expect("insert identity");
        }
        drop(connection);
    }

    fn fixture() -> (tempfile::TempDir, PathBuf, PathBuf) {
        let directory = tempfile::tempdir().expect("temp dir");
        let database_path = directory.path().join("core.db");
        let manifest_path = directory.path().join("core.integrity.json");
        (directory, database_path, manifest_path)
    }

    #[test]
    fn bytes_sha256_matches_known_vector() {
        assert_eq!(
            compute_bytes_sha256(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn identity_digest_is_stable_and_display_name_sensitive() {
        let first = identity(CORE_A, "SofA-a");
        let second = identity(CORE_A, "SofA-a");
        let renamed = first.with_display_name("SofA-a Ada Lyra").expect("rename");

        let first_digest = compute_identity_sha256(&first).expect("digest");
        let second_digest = compute_identity_sha256(&second).expect("digest");
        let renamed_digest = compute_identity_sha256(&renamed).expect("digest");

        assert_eq!(first_digest, second_digest);
        assert_eq!(first_digest.len(), 64);
        assert_ne!(first_digest, renamed_digest);
    }

    #[test]
    fn seal_and_verify_success() {
        let (_directory, database_path, manifest_path) = fixture();
        let identity = identity(CORE_A, "SofA-a");
        create_database(&database_path, Some(&identity), 1);

        let manifest = seal_manifest(CORE_A, &database_path, &manifest_path, 1, &identity)
            .expect("seal manifest");
        assert_eq!(manifest.database_file, "core.db");
        assert_eq!(
            manifest.database_bytes,
            fs::metadata(&database_path).unwrap().len()
        );
        assert_eq!(manifest.algorithm, DIGEST_ALGORITHM_SHA256);
        assert_eq!(
            manifest.identity_sha256,
            compute_identity_sha256(&identity).unwrap()
        );

        let stored = IntegrityManifestStore::new(&manifest_path)
            .read()
            .expect("read manifest")
            .expect("manifest present");
        assert_eq!(stored, manifest);

        let report = verify_core(CORE_A, &database_path, &manifest_path, 1, Some(&identity));
        assert_eq!(report.status, IntegrityStatus::Verified);
        assert_eq!(
            report.expected_sha256.as_deref(),
            Some(manifest.database_sha256.as_str())
        );
        assert_eq!(report.actual_sha256, report.expected_sha256);
        assert_eq!(report.actual_schema_revision, Some(1));
        assert!(report.validate().is_ok());
    }

    #[test]
    fn modified_database_is_detected() {
        let (_directory, database_path, manifest_path) = fixture();
        let identity = identity(CORE_A, "SofA-a");
        create_database(&database_path, Some(&identity), 1);
        seal_manifest(CORE_A, &database_path, &manifest_path, 1, &identity).expect("seal");

        let connection = Connection::open(&database_path).expect("open database");
        connection
            .execute("INSERT INTO payload(value) VALUES('tampered')", [])
            .expect("modify database");
        drop(connection);

        let report = verify_core(CORE_A, &database_path, &manifest_path, 1, Some(&identity));
        assert_eq!(report.status, IntegrityStatus::DigestMismatch);
        assert_ne!(report.actual_sha256, report.expected_sha256);
    }

    #[test]
    fn missing_manifest_is_reported() {
        let (_directory, database_path, manifest_path) = fixture();
        let identity = identity(CORE_A, "SofA-a");
        create_database(&database_path, Some(&identity), 1);

        let report = verify_core(CORE_A, &database_path, &manifest_path, 1, Some(&identity));
        assert_eq!(report.status, IntegrityStatus::MissingManifest);
        assert!(report.validate().is_ok());
    }

    #[test]
    fn invalid_manifest_is_reported() {
        let (_directory, database_path, manifest_path) = fixture();
        let identity = identity(CORE_A, "SofA-a");
        create_database(&database_path, Some(&identity), 1);
        fs::write(&manifest_path, b"{\"schema\":\"not-a-manifest\"}").expect("write bad manifest");

        let report = verify_core(CORE_A, &database_path, &manifest_path, 1, Some(&identity));
        assert_eq!(report.status, IntegrityStatus::InvalidManifest);
        assert!(report.validate().is_ok());
    }

    #[test]
    fn missing_database_is_reported() {
        let (_directory, database_path, manifest_path) = fixture();
        let identity = identity(CORE_A, "SofA-a");
        create_database(&database_path, Some(&identity), 1);
        seal_manifest(CORE_A, &database_path, &manifest_path, 1, &identity).expect("seal");
        fs::remove_file(&database_path).expect("remove database");

        let report = verify_core(CORE_A, &database_path, &manifest_path, 1, Some(&identity));
        assert_eq!(report.status, IntegrityStatus::DatabaseMissing);
        assert!(report.validate().is_ok());
    }

    #[test]
    fn schema_mismatch_is_reported() {
        let (_directory, database_path, manifest_path) = fixture();
        let identity = identity(CORE_A, "SofA-a");
        create_database(&database_path, Some(&identity), 1);
        seal_manifest(CORE_A, &database_path, &manifest_path, 1, &identity).expect("seal");

        let report = verify_core(CORE_A, &database_path, &manifest_path, 2, Some(&identity));
        assert_eq!(report.status, IntegrityStatus::SchemaMismatch);
        assert_eq!(report.actual_schema_revision, Some(1));
        assert_eq!(report.expected_schema_revision, Some(2));
        assert!(report.validate().is_ok());
    }

    #[test]
    fn ownership_mismatch_is_reported() {
        let (_directory, database_path, manifest_path) = fixture();
        let expected = identity(CORE_A, "SofA-a");
        let stored = identity(CORE_B, "SofA-a");
        create_database(&database_path, Some(&stored), 1);
        seal_manifest(CORE_A, &database_path, &manifest_path, 1, &expected).expect("seal");

        let report = verify_core(CORE_A, &database_path, &manifest_path, 1, Some(&expected));
        assert_eq!(report.status, IntegrityStatus::OwnershipMismatch);
        assert!(report.validate().is_ok());
    }

    #[test]
    fn digest_is_regenerated_after_authorized_reseal() {
        let (_directory, database_path, manifest_path) = fixture();
        let identity = identity(CORE_A, "SofA-a");
        create_database(&database_path, Some(&identity), 1);
        let first = seal_manifest(CORE_A, &database_path, &manifest_path, 1, &identity)
            .expect("first seal");

        let connection = Connection::open(&database_path).expect("open database");
        connection
            .execute("INSERT INTO payload(value) VALUES('authorized change')", [])
            .expect("modify database");
        drop(connection);

        let second =
            seal_manifest(CORE_A, &database_path, &manifest_path, 1, &identity).expect("resel");
        assert_ne!(first.database_sha256, second.database_sha256);
        assert_eq!(
            second.database_sha256,
            compute_file_sha256(&database_path).unwrap()
        );

        let report = verify_core(CORE_A, &database_path, &manifest_path, 1, Some(&identity));
        assert_eq!(report.status, IntegrityStatus::Verified);
        assert_eq!(
            report.actual_sha256.as_deref(),
            Some(second.database_sha256.as_str())
        );
    }

    #[test]
    fn corrupt_sqlite_is_detected() {
        let (_directory, database_path, manifest_path) = fixture();
        let identity = identity(CORE_A, "SofA-a");
        create_database(&database_path, Some(&identity), 1);
        seal_manifest(CORE_A, &database_path, &manifest_path, 1, &identity).expect("seal");

        let length = fs::metadata(&database_path)
            .expect("database metadata")
            .len();
        let mut file = fs::OpenOptions::new()
            .write(true)
            .open(&database_path)
            .expect("open database for corruption");
        file.seek(SeekFrom::Start(100)).expect("seek");
        file.write_all(&vec![0xffu8; (length - 100) as usize])
            .expect("corrupt database");
        file.sync_all().expect("sync corrupt database");
        drop(file);

        let report = verify_core(CORE_A, &database_path, &manifest_path, 1, Some(&identity));
        assert_eq!(report.status, IntegrityStatus::SqliteCorrupt);
        assert!(report.validate().is_ok());
    }

    #[test]
    fn manifest_store_returns_none_when_absent() {
        let (_directory, _database_path, manifest_path) = fixture();
        let store = IntegrityManifestStore::new(&manifest_path);
        assert!(store.read().expect("read absent manifest").is_none());
    }

    #[test]
    fn seal_rejects_identity_uuid_mismatch() {
        let (_directory, database_path, manifest_path) = fixture();
        let identity = identity(CORE_A, "SofA-a");
        create_database(&database_path, Some(&identity), 1);

        let error = seal_manifest(CORE_B, &database_path, &manifest_path, 1, &identity)
            .expect_err("mismatched identity must be rejected");
        assert!(matches!(error, IntegrityError::IdentityMismatch));
    }

    const INSTANCE_A: &str = "22222222-2222-4222-8222-222222222222";
    const INSTANCE_B: &str = "33333333-3333-4333-8333-333333333333";

    fn add_instance_metadata(database_path: &Path, instance: &str, mode: &str) {
        let connection = Connection::open(database_path).expect("open fixture database");
        connection
            .execute_batch(
                "ALTER TABLE core_identity ADD COLUMN database_instance_uuid TEXT NOT NULL DEFAULT '';
                 ALTER TABLE core_identity ADD COLUMN initialization_mode TEXT NOT NULL DEFAULT '';
                 UPDATE core_identity
                 SET database_instance_uuid = '22222222-2222-4222-8222-222222222222',
                     initialization_mode = 'fresh_no_migration';",
            )
            .expect("add instance metadata");
        connection
            .execute(
                "UPDATE core_identity SET database_instance_uuid = ?1, initialization_mode = ?2",
                rusqlite::params![instance, mode],
            )
            .expect("set instance metadata");
    }

    #[test]
    fn manifest_database_instance_metadata_is_verified() {
        let (_directory, database_path, manifest_path) = fixture();
        let identity = identity(CORE_A, "SofA-a");
        create_database(&database_path, Some(&identity), 1);
        add_instance_metadata(&database_path, INSTANCE_A, "fresh_no_migration");

        let manifest = seal_manifest_with_metadata(
            CORE_A,
            &database_path,
            &manifest_path,
            1,
            &identity,
            Some(INSTANCE_A),
            Some("fresh_no_migration"),
            Some(1),
        )
        .expect("seal with metadata");
        assert_eq!(manifest.database_instance_uuid.as_deref(), Some(INSTANCE_A));
        assert_eq!(
            verify_core(CORE_A, &database_path, &manifest_path, 1, Some(&identity)).status,
            IntegrityStatus::Verified
        );

        let mut tampered = manifest.clone();
        tampered.database_instance_uuid = Some(INSTANCE_B.to_owned());
        IntegrityManifestStore::new(&manifest_path)
            .write(&tampered)
            .expect("write tampered manifest");
        let report = verify_core(CORE_A, &database_path, &manifest_path, 1, Some(&identity));
        assert_eq!(report.status, IntegrityStatus::OwnershipMismatch);
    }

    #[test]
    fn manifest_core_number_mismatch_is_detected() {
        let (_directory, database_path, manifest_path) = fixture();
        let identity = identity(CORE_A, "SofA-a");
        create_database(&database_path, Some(&identity), 1);

        let mut manifest = seal_manifest(CORE_A, &database_path, &manifest_path, 1, &identity)
            .expect("seal manifest");
        manifest.core_number = Some(2);
        IntegrityManifestStore::new(&manifest_path)
            .write(&manifest)
            .expect("write manifest");
        let report = verify_core(CORE_A, &database_path, &manifest_path, 1, Some(&identity));
        assert_eq!(report.status, IntegrityStatus::OwnershipMismatch);
    }

    #[test]
    fn unexpected_wal_sidecar_fails_verification() {
        let (_directory, database_path, manifest_path) = fixture();
        let identity = identity(CORE_A, "SofA-a");
        create_database(&database_path, Some(&identity), 1);
        seal_manifest(CORE_A, &database_path, &manifest_path, 1, &identity).expect("seal manifest");

        let mut sidecar = database_path.as_os_str().to_os_string();
        sidecar.push("-wal");
        fs::write(PathBuf::from(sidecar), b"uncheckpointed").expect("write sidecar");

        let report = verify_core(CORE_A, &database_path, &manifest_path, 1, Some(&identity));
        assert_eq!(report.status, IntegrityStatus::DigestMismatch);
        assert!(report
            .detail
            .as_deref()
            .is_some_and(|detail| detail.contains("sidecar")));
    }
}
