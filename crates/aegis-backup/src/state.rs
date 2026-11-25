use std::fs::{self, File};
use std::io::{self, Read};
use std::path::Path;
use std::time::{Duration, SystemTime};

use aegis_contracts::core::{
    validate_core_id, validate_core_number, validate_display_name, CoreIdentity,
    CORE_IDENTITY_TABLE,
};
use aegis_contracts::integrity::IntegrityStatus;
use aegis_contracts::lifecycle::CoreLifecycleState;
use aegis_contracts::roster::validate_storage_path;
use aegis_core_integrity::{
    verify_core, IntegrityManifest, IntegrityManifestStore, IntegrityReport,
};
use aegis_storage::{verify_application_id, PLATFORM_APPLICATION_ID, SAFE_APPLICATION_ID};
use chrono::Utc;
use rusqlite::{Connection, OpenFlags, OptionalExtension};

use crate::error::BackupError;
use crate::pathutil::{is_redirected, require_regular_file};

/// One registry row needed to back up a Core.
#[derive(Debug, Clone)]
pub(crate) struct RegistryCore {
    pub core_uuid: String,
    pub core_number: u8,
    pub display_name: String,
    pub lifecycle_state: CoreLifecycleState,
}

const HEARTBEAT_STALE_MS: i64 = 1_500;
const MAX_HEARTBEAT_BYTES: u64 = 8 * 1024;

pub(crate) fn open_platform_read_only(path: &Path) -> Result<Connection, BackupError> {
    require_regular_file(path)?;
    let connection = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    verify_application_id(&connection, PLATFORM_APPLICATION_ID)?;
    Ok(connection)
}

pub(crate) fn open_safe_read_only(path: &Path) -> Result<Connection, BackupError> {
    require_regular_file(path)?;
    let connection = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    verify_application_id(&connection, SAFE_APPLICATION_ID)?;
    Ok(connection)
}

pub(crate) fn read_platform_identity(platform_db: &Path) -> Result<(String, String), BackupError> {
    let connection = open_platform_read_only(platform_db)?;
    if !table_exists(&connection, "platform_identity")? {
        return Err(BackupError::InvalidSource(
            "platform_identity table is missing".to_owned(),
        ));
    }
    let row: Option<(String, String)> = connection
        .query_row(
            "SELECT platform_id, callsign FROM platform_identity WHERE singleton = 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    match row {
        Some((platform_id, callsign))
            if !platform_id.trim().is_empty() && !callsign.trim().is_empty() =>
        {
            Ok((platform_id, callsign))
        }
        _ => Err(BackupError::InvalidSource(
            "Platform identity row is missing or empty".to_owned(),
        )),
    }
}

pub(crate) fn read_core_registry(platform_db: &Path) -> Result<Vec<RegistryCore>, BackupError> {
    let connection = open_platform_read_only(platform_db)?;
    if !table_exists(&connection, "core_registry")? {
        if table_exists(&connection, "platform_core")? {
            let count: i64 =
                connection.query_row("SELECT COUNT(*) FROM platform_core", [], |row| row.get(0))?;
            if count > 0 {
                return Err(BackupError::InvalidSource(
                    "legacy platform_core rows require registry migration before backup".to_owned(),
                ));
            }
        }
        return Ok(Vec::new());
    }

    let mut statement = connection.prepare(
        "SELECT core_uuid, core_number, display_name, lifecycle_state, storage_relative_path
         FROM core_registry ORDER BY core_number ASC, core_uuid ASC",
    )?;
    let mut rows = statement.query([])?;
    let mut cores = Vec::new();
    while let Some(row) = rows.next()? {
        let raw_uuid: String = row.get(0)?;
        let raw_number: i64 = row.get(1)?;
        let display_name: String = row.get(2)?;
        let raw_lifecycle: String = row.get(3)?;
        let storage_relative_path: String = row.get(4)?;

        let uuid = validate_core_id(&raw_uuid).map_err(|_| {
            BackupError::InvalidSource("registry contains an invalid Core UUID".to_owned())
        })?;
        let core_uuid = uuid.hyphenated().to_string();
        let core_number = u8::try_from(raw_number).map_err(|_| {
            BackupError::InvalidSource(format!("Core {core_uuid} has an invalid number"))
        })?;
        validate_core_number(core_number).map_err(|_| {
            BackupError::InvalidSource(format!("Core {core_uuid} has an unapproved number"))
        })?;
        validate_display_name(&display_name).map_err(|_| {
            BackupError::InvalidSource(format!("Core {core_uuid} has an invalid display name"))
        })?;
        let lifecycle_state = CoreLifecycleState::parse(&raw_lifecycle).ok_or_else(|| {
            BackupError::InvalidSource(format!("Core {core_uuid} has an invalid lifecycle state"))
        })?;
        validate_storage_path(&storage_relative_path).map_err(|_| {
            BackupError::InvalidSource(format!("Core {core_uuid} has an invalid storage path"))
        })?;
        let expected = format!("cores/{core_uuid}/core.db");
        if storage_relative_path != expected {
            return Err(BackupError::UnsafePath(format!(
                "Core {core_uuid} registry path {storage_relative_path} does not match the approved layout"
            )));
        }

        cores.push(RegistryCore {
            core_uuid,
            core_number,
            display_name,
            lifecycle_state,
        });
    }
    Ok(cores)
}

pub(crate) fn check_markers(root: &Path) -> Result<(), BackupError> {
    let platform_db = root.join("platform.db");
    let safe_db = root.join("safe.db");
    drop(open_platform_read_only(&platform_db)?);
    drop(open_safe_read_only(&safe_db)?);
    Ok(())
}

pub(crate) fn read_core_identity(
    core_uuid: &str,
    database_path: &Path,
) -> Result<CoreIdentity, BackupError> {
    let connection = Connection::open_with_flags(database_path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    if !table_exists(&connection, CORE_IDENTITY_TABLE)? {
        return Err(ownership_failure(
            core_uuid,
            "stored core_identity table is missing",
        ));
    }
    let row: Option<(String, String, i64, String, String, i64)> = connection
        .query_row(
            "SELECT schema, core_uuid, core_number, display_name, created_at, identity_revision
             FROM core_identity LIMIT 1",
            [],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                ))
            },
        )
        .optional()?;
    let Some((schema, raw_uuid, raw_number, display_name, created_at, raw_revision)) = row else {
        return Err(ownership_failure(
            core_uuid,
            "stored core_identity row is missing",
        ));
    };

    let stored_uuid = validate_core_id(&raw_uuid)
        .map_err(|_| ownership_failure(core_uuid, "stored Core UUID is invalid"))?;
    let core_number = u8::try_from(raw_number)
        .map_err(|_| ownership_failure(core_uuid, "stored Core number is invalid"))?;
    let identity_revision = u64::try_from(raw_revision)
        .map_err(|_| ownership_failure(core_uuid, "stored identity revision is invalid"))?;
    let identity = CoreIdentity {
        schema,
        core_uuid: stored_uuid.hyphenated().to_string(),
        core_number,
        display_name,
        created_at,
        identity_revision,
    };
    identity
        .validate()
        .map_err(|_| ownership_failure(core_uuid, "stored Core identity is invalid"))?;
    if identity.core_uuid != core_uuid {
        return Err(ownership_failure(
            core_uuid,
            "stored Core UUID does not match the requested Core",
        ));
    }
    Ok(identity)
}

pub(crate) fn verify_core_files(
    core_uuid: &str,
    database_path: &Path,
    manifest_path: &Path,
    expected_identity: Option<&CoreIdentity>,
) -> Result<(IntegrityManifest, IntegrityReport), BackupError> {
    let store = IntegrityManifestStore::new(manifest_path);
    let manifest = match store.read() {
        Ok(Some(manifest)) => manifest,
        Ok(None) => {
            return Err(BackupError::IntegrityFailed {
                core_uuid: core_uuid.to_owned(),
                status: IntegrityStatus::MissingManifest,
                detail: format!(
                    "integrity manifest is missing at {}",
                    manifest_path.display()
                ),
            })
        }
        Err(error) => return Err(error.into()),
    };
    let report = verify_core(
        core_uuid,
        database_path,
        manifest_path,
        manifest.schema_revision,
        expected_identity,
    );
    if report.status != IntegrityStatus::Verified {
        return Err(BackupError::IntegrityFailed {
            core_uuid: core_uuid.to_owned(),
            status: report.status,
            detail: report
                .detail
                .unwrap_or_else(|| "integrity verification failed".to_owned()),
        });
    }
    Ok((manifest, report))
}

/// Conservatively refuse when any Core appears to be running.
///
/// A live recorded state or a fresh heartbeat is a refusal. A heartbeat that
/// cannot be parsed falls back to its modification time and is treated as
/// running when it is recent or its age is unknown.
pub(crate) fn detect_running(root: &Path) -> Result<(), BackupError> {
    let platform_db = root.join("platform.db");
    if let Ok(metadata) = fs::symlink_metadata(&platform_db) {
        if metadata.is_file() && !is_redirected(&metadata) {
            let connection =
                Connection::open_with_flags(&platform_db, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
            if table_exists(&connection, "core_runtime_state")? {
                let mut statement = connection.prepare(
                    "SELECT core_uuid FROM core_runtime_state
                     WHERE state IN ('starting', 'running', 'stopping')
                     ORDER BY core_uuid ASC LIMIT 1",
                )?;
                let mut rows = statement.query([])?;
                if let Some(row) = rows.next()? {
                    let core_uuid: String = row.get(0)?;
                    return Err(BackupError::RunningCore(core_uuid));
                }
            }
        }
    }

    let run_dir = root.join("run");
    let metadata = match fs::symlink_metadata(&run_dir) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
    };
    if is_redirected(&metadata) {
        return Err(BackupError::RunningCore("unknown".to_owned()));
    }
    if !metadata.is_dir() {
        return Err(BackupError::UnsafePath(format!(
            "{} is not a directory",
            run_dir.display()
        )));
    }

    for entry in fs::read_dir(&run_dir)? {
        let entry = entry?;
        let file_name = entry.file_name();
        let name = file_name.to_string_lossy();
        let Some(core_uuid) = name.strip_suffix(".heartbeat") else {
            continue;
        };
        let metadata = entry.metadata()?;
        if is_redirected(&metadata) {
            return Err(BackupError::RunningCore(core_uuid.to_owned()));
        }
        if !metadata.is_file() {
            continue;
        }
        if heartbeat_is_fresh(&entry.path(), &metadata)? {
            return Err(BackupError::RunningCore(core_uuid.to_owned()));
        }
    }
    Ok(())
}

fn heartbeat_is_fresh(path: &Path, metadata: &fs::Metadata) -> Result<bool, BackupError> {
    if metadata.len() > MAX_HEARTBEAT_BYTES {
        return Ok(true);
    }
    let mut file = match File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error.into()),
    };
    let mut bytes = Vec::new();
    file.by_ref()
        .take(MAX_HEARTBEAT_BYTES)
        .read_to_end(&mut bytes)?;

    if let Ok(value) = serde_json::from_slice::<serde_json::Value>(&bytes) {
        let timestamp = value.get("timestamp_ms").and_then(|value| {
            value
                .as_i64()
                .or_else(|| value.as_u64().and_then(|v| i64::try_from(v).ok()))
        });
        let Some(timestamp) = timestamp else {
            return Ok(true);
        };
        let now = Utc::now().timestamp_millis();
        return Ok(now.saturating_sub(timestamp) <= HEARTBEAT_STALE_MS);
    }

    match metadata.modified() {
        Ok(modified) => match SystemTime::now().duration_since(modified) {
            Ok(age) => {
                Ok(age <= Duration::from_millis(u64::try_from(HEARTBEAT_STALE_MS).unwrap_or(0)))
            }
            Err(_) => Ok(true),
        },
        Err(_) => Ok(true),
    }
}

fn ownership_failure(core_uuid: &str, detail: &str) -> BackupError {
    BackupError::IntegrityFailed {
        core_uuid: core_uuid.to_owned(),
        status: IntegrityStatus::OwnershipMismatch,
        detail: detail.to_owned(),
    }
}

fn table_exists(connection: &Connection, name: &str) -> Result<bool, BackupError> {
    let exists: Option<i64> = connection
        .query_row(
            "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1",
            [name],
            |row| row.get(0),
        )
        .optional()?;
    Ok(exists.is_some())
}
