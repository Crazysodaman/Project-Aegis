//! Platform installation, upgrade, and uninstall orchestration.
//!
//! This crate is deliberately conservative:
//!
//! * Installation initializes the Platform, Safe boundary, and module registry,
//!   then records a versioned installation record inside `platform.db`.
//! * Upgrade refuses downgrades, takes a verified backup outside the storage
//!   root before applying anything, and preserves all Core data and identity.
//! * Uninstall preserves data by default. Deleting data requires an explicit
//!   confirmation equal to the Platform callsign, a verified backup, and an
//!   explicit delete mode.
//!
//! This crate never creates Cores, never runs Cores, and never touches the
//! legacy Sofía repository.

use std::fs;
use std::path::{Path, PathBuf};

use aegis_platform::{PlatformService, PlatformStore};
use aegis_safe::{SafeError, SafeStore};
use aegis_storage::{
    now_rfc3339, open_initialized, StorageError, StoragePaths, PLATFORM_APPLICATION_ID,
};
use chrono::Utc;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Aegis version recorded by this build.
pub const AEGIS_VERSION: &str = env!("CARGO_PKG_VERSION");
/// Schema identifier for the persisted installation record.
pub const INSTALLATION_SCHEMA: &str = "aegis.installation.v1";
/// Component name used when validating the Platform database schema.
const PLATFORM_COMPONENT: &str = "platform";

const KEY_SCHEMA: &str = "installation.schema";
const KEY_VERSION: &str = "installation.version";
const KEY_STATE: &str = "installation.state";
const KEY_MODE: &str = "installation.mode";
const KEY_INSTALLED_AT: &str = "installation.installed_at";
const KEY_UPDATED_AT: &str = "installation.updated_at";
const KEY_PLATFORM_ID: &str = "installation.platform_id";

#[derive(Debug, Error)]
pub enum InstallerError {
    #[error("Platform operation failed: {0}")]
    Platform(#[from] aegis_platform::ServiceError),
    #[error("Platform store failed: {0}")]
    PlatformStore(#[from] aegis_platform::PlatformError),
    #[error("Safe operation failed: {0}")]
    Safe(#[from] SafeError),
    #[error("storage failed: {0}")]
    Storage(#[from] StorageError),
    #[error("backup or restore failed: {0}")]
    Backup(#[from] aegis_backup::BackupError),
    #[error("SQLite operation failed: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("installation record JSON failed: {0}")]
    Json(#[from] serde_json::Error),
    #[error("Platform is not initialized at {0}")]
    NotInstalled(String),
    #[error("owner authorization failed")]
    Unauthorized,
    #[error("installed version {installed} is newer than this build {build}")]
    IncompatibleVersion { installed: String, build: String },
    #[error("version string is invalid: {0}")]
    InvalidVersion(String),
    #[error("downgrade from {installed} to {build} is refused")]
    DowngradeRefused { installed: String, build: String },
    #[error("a backup destination is required before deleting data")]
    BackupRequired,
    #[error("deletion confirmation must match the Platform callsign")]
    ConfirmationRequired,
    #[error("uninstall of a running Platform service is refused")]
    ServiceRunning,
    #[error("refusing to delete a redirected or junction path: {0}")]
    UnsafePath(String),
    #[error("installation record is missing or corrupt")]
    InvalidRecord,
}

/// Installation mode selected by the owner.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InstallMode {
    Guided,
    Headless,
}

impl InstallMode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Guided => "guided",
            Self::Headless => "headless",
        }
    }
}

/// Persisted, versioned installation record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstallationRecord {
    pub schema: String,
    pub aegis_version: String,
    pub platform_id: String,
    pub callsign: String,
    pub state: String,
    pub mode: String,
    pub installed_at: String,
    pub updated_at: String,
}

/// Result of an install request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstallReport {
    pub newly_initialized: bool,
    pub record: InstallationRecord,
}

/// Result of an upgrade request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpgradeReport {
    pub from_version: String,
    pub to_version: String,
    pub already_current: bool,
    pub backup: Option<PathBuf>,
    pub record: InstallationRecord,
}

/// Owner choice for uninstall.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UninstallMode {
    /// Leave every Platform and Core file in place; record the uninstall.
    Preserve,
    /// Delete Aegis-owned data after a verified backup.
    DeleteData,
}

/// Result of an uninstall request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UninstallReport {
    pub mode: UninstallMode,
    pub deleted: Vec<String>,
    pub backup: Option<PathBuf>,
    pub record: InstallationRecord,
}

/// Read the persisted installation record, if any.
pub fn installation_record(
    root: impl AsRef<Path>,
) -> Result<Option<InstallationRecord>, InstallerError> {
    let paths = StoragePaths::new(root)?;
    read_record(&paths)
}

/// Install the Platform into an empty (or partially initialized) root.
///
/// Repeating an install is safe: the existing Platform identity is preserved,
/// a missing installation record is backfilled, and no Core data is touched.
pub fn install(root: impl AsRef<Path>, mode: InstallMode) -> Result<InstallReport, InstallerError> {
    let paths = StoragePaths::new(root)?;
    if paths.platform_db().is_file() {
        let identity = PlatformStore::new(paths.clone()).identity()?;
        let record = match read_record(&paths)? {
            Some(record) => {
                ensure_compatible(&record.aegis_version)?;
                record
            }
            None => {
                let record = new_record(&identity.platform_id, &identity.callsign, mode);
                write_record(&paths, &record)?;
                record
            }
        };
        return Ok(InstallReport {
            newly_initialized: false,
            record,
        });
    }

    let initialized = PlatformService::initialize(paths.root())?;
    let record = new_record(&initialized.platform_id, &initialized.callsign, mode);
    write_record(&paths, &record)?;
    Ok(InstallReport {
        newly_initialized: true,
        record,
    })
}

/// Upgrade an existing installation to this build's version.
///
/// A verified backup outside the storage root is taken before any change. All
/// Core databases, manifests, initialization markers, and owner proof are left
/// byte-for-byte untouched by the upgrade itself.
pub fn upgrade(
    root: impl AsRef<Path>,
    owner_token: &str,
    backup_to: Option<&Path>,
) -> Result<UpgradeReport, InstallerError> {
    let paths = StoragePaths::new(root)?;
    authorize(&paths, owner_token)?;
    if !paths.platform_db().is_file() {
        return Err(InstallerError::NotInstalled(
            paths.root().display().to_string(),
        ));
    }

    let identity = PlatformStore::new(paths.clone()).identity()?;
    let existing = read_record(&paths)?;
    let from_version = existing
        .as_ref()
        .map(|record| record.aegis_version.clone())
        .unwrap_or_else(|| "0.0.0".to_owned());
    compare_versions(&from_version, AEGIS_VERSION)?;

    if same_version(&from_version, AEGIS_VERSION) {
        let record = existing.unwrap_or_else(|| {
            new_record(
                &identity.platform_id,
                &identity.callsign,
                InstallMode::Headless,
            )
        });
        write_record(&paths, &record)?;
        return Ok(UpgradeReport {
            from_version,
            to_version: AEGIS_VERSION.to_owned(),
            already_current: true,
            backup: None,
            record,
        });
    }

    let destination = match backup_to {
        Some(path) => path.to_path_buf(),
        None => default_backup_path(paths.root())?,
    };
    let report = aegis_backup::backup(paths.root(), &destination)?;
    aegis_backup::verify_backup(&report.destination)?;

    PlatformService::initialize(paths.root())?;

    let record = InstallationRecord {
        schema: INSTALLATION_SCHEMA.to_owned(),
        aegis_version: AEGIS_VERSION.to_owned(),
        platform_id: identity.platform_id,
        callsign: identity.callsign,
        state: "installed".to_owned(),
        mode: existing
            .as_ref()
            .map(|record| record.mode.clone())
            .unwrap_or_else(|| InstallMode::Headless.as_str().to_owned()),
        installed_at: existing
            .as_ref()
            .map(|record| record.installed_at.clone())
            .unwrap_or_else(|| Utc::now().to_rfc3339()),
        updated_at: Utc::now().to_rfc3339(),
    };
    write_record(&paths, &record)?;
    Ok(UpgradeReport {
        from_version,
        to_version: AEGIS_VERSION.to_owned(),
        already_current: false,
        backup: Some(report.destination),
        record,
    })
}

/// Uninstall the Platform while preserving data by default.
///
/// `confirmation` must equal the Platform callsign when `DeleteData` is
/// selected, and a verified backup must have been requested.
pub fn uninstall(
    root: impl AsRef<Path>,
    owner_token: &str,
    mode: UninstallMode,
    confirmation: Option<&str>,
    backup_to: Option<&Path>,
) -> Result<UninstallReport, InstallerError> {
    let paths = StoragePaths::new(root)?;
    authorize(&paths, owner_token)?;
    if !paths.platform_db().is_file() {
        return Err(InstallerError::NotInstalled(
            paths.root().display().to_string(),
        ));
    }
    if aegis_service_running(&paths) {
        return Err(InstallerError::ServiceRunning);
    }

    let identity = PlatformStore::new(paths.clone()).identity()?;
    let existing = read_record(&paths)?.unwrap_or_else(|| {
        new_record(
            &identity.platform_id,
            &identity.callsign,
            InstallMode::Headless,
        )
    });

    match mode {
        UninstallMode::Preserve => {
            let record = InstallationRecord {
                state: "uninstalled_preserved".to_owned(),
                updated_at: Utc::now().to_rfc3339(),
                ..existing
            };
            write_record(&paths, &record)?;
            Ok(UninstallReport {
                mode,
                deleted: Vec::new(),
                backup: None,
                record,
            })
        }
        UninstallMode::DeleteData => {
            let backup_to = backup_to.ok_or(InstallerError::BackupRequired)?;
            if confirmation.map(str::trim) != Some(identity.callsign.as_str()) {
                return Err(InstallerError::ConfirmationRequired);
            }
            let report = aegis_backup::backup(paths.root(), backup_to)?;
            aegis_backup::verify_backup(&report.destination)?;

            let deleted = delete_owned_artifacts(paths.root())?;
            Ok(UninstallReport {
                mode,
                deleted,
                backup: Some(report.destination),
                record: InstallationRecord {
                    state: "uninstalled_deleted".to_owned(),
                    updated_at: Utc::now().to_rfc3339(),
                    ..existing
                },
            })
        }
    }
}

fn new_record(platform_id: &str, callsign: &str, mode: InstallMode) -> InstallationRecord {
    let now = Utc::now().to_rfc3339();
    InstallationRecord {
        schema: INSTALLATION_SCHEMA.to_owned(),
        aegis_version: AEGIS_VERSION.to_owned(),
        platform_id: platform_id.to_owned(),
        callsign: callsign.to_owned(),
        state: "installed".to_owned(),
        mode: mode.as_str().to_owned(),
        installed_at: now.clone(),
        updated_at: now,
    }
}

fn authorize(paths: &StoragePaths, owner_token: &str) -> Result<(), InstallerError> {
    let safe = SafeStore::new(paths.clone());
    if !safe.verify_owner(owner_token)? {
        return Err(InstallerError::Unauthorized);
    }
    Ok(())
}

fn read_record(paths: &StoragePaths) -> Result<Option<InstallationRecord>, InstallerError> {
    if !paths.platform_db().is_file() {
        return Ok(None);
    }
    let connection = open_initialized(
        &paths.platform_db(),
        PLATFORM_APPLICATION_ID,
        PLATFORM_COMPONENT,
    )?;
    if !table_exists(&connection, "platform_config")? {
        return Ok(None);
    }
    let Some(schema) = read_config(&connection, KEY_SCHEMA)? else {
        return Ok(None);
    };
    if schema != INSTALLATION_SCHEMA {
        return Err(InstallerError::InvalidRecord);
    }
    let required = |key: &str| -> Result<String, InstallerError> {
        read_config(&connection, key)?.ok_or(InstallerError::InvalidRecord)
    };
    let callsign = PlatformStore::new(paths.clone()).identity()?.callsign;
    Ok(Some(InstallationRecord {
        schema,
        aegis_version: required(KEY_VERSION)?,
        platform_id: required(KEY_PLATFORM_ID)?,
        callsign,
        state: required(KEY_STATE)?,
        mode: required(KEY_MODE)?,
        installed_at: required(KEY_INSTALLED_AT)?,
        updated_at: required(KEY_UPDATED_AT)?,
    }))
}

fn write_record(paths: &StoragePaths, record: &InstallationRecord) -> Result<(), InstallerError> {
    let connection = open_initialized(
        &paths.platform_db(),
        PLATFORM_APPLICATION_ID,
        PLATFORM_COMPONENT,
    )?;
    connection.execute_batch(
        "CREATE TABLE IF NOT EXISTS platform_config (
            config_key TEXT PRIMARY KEY,
            config_value TEXT NOT NULL,
            updated_at TEXT NOT NULL
        );",
    )?;
    let now = now_rfc3339()?;
    for (key, value) in [
        (KEY_SCHEMA, record.schema.as_str()),
        (KEY_VERSION, record.aegis_version.as_str()),
        (KEY_STATE, record.state.as_str()),
        (KEY_MODE, record.mode.as_str()),
        (KEY_INSTALLED_AT, record.installed_at.as_str()),
        (KEY_UPDATED_AT, record.updated_at.as_str()),
        (KEY_PLATFORM_ID, record.platform_id.as_str()),
    ] {
        connection.execute(
            "INSERT INTO platform_config(config_key, config_value, updated_at)
             VALUES(?1, ?2, ?3)
             ON CONFLICT(config_key) DO UPDATE SET
                 config_value = excluded.config_value,
                 updated_at = excluded.updated_at",
            params![key, value, now],
        )?;
    }
    Ok(())
}

fn read_config(connection: &Connection, key: &str) -> Result<Option<String>, InstallerError> {
    Ok(connection
        .query_row(
            "SELECT config_value FROM platform_config WHERE config_key = ?1",
            [key],
            |row| row.get(0),
        )
        .optional()?)
}

fn table_exists(connection: &Connection, table: &str) -> Result<bool, InstallerError> {
    let found: Option<String> = connection
        .query_row(
            "SELECT name FROM sqlite_master WHERE type = 'table' AND name = ?1",
            [table],
            |row| row.get(0),
        )
        .optional()?;
    Ok(found.is_some())
}

fn ensure_compatible(installed: &str) -> Result<(), InstallerError> {
    let installed_version = parse_version(installed)?;
    let build = parse_version(AEGIS_VERSION)?;
    if installed_version > build {
        return Err(InstallerError::IncompatibleVersion {
            installed: installed.to_owned(),
            build: AEGIS_VERSION.to_owned(),
        });
    }
    Ok(())
}

fn compare_versions(installed: &str, build: &str) -> Result<(), InstallerError> {
    let installed_version = parse_version(installed)?;
    let build_version = parse_version(build)?;
    if installed_version > build_version {
        return Err(InstallerError::DowngradeRefused {
            installed: installed.to_owned(),
            build: build.to_owned(),
        });
    }
    Ok(())
}

fn same_version(left: &str, right: &str) -> bool {
    match (parse_version(left), parse_version(right)) {
        (Ok(left), Ok(right)) => left == right,
        _ => false,
    }
}

fn parse_version(value: &str) -> Result<(u64, u64, u64), InstallerError> {
    let core = value.split(['-', '+']).next().unwrap_or_default();
    let parts: Vec<&str> = core.split('.').collect();
    if parts.len() > 3 {
        return Err(InstallerError::InvalidVersion(value.to_owned()));
    }
    let mut numbers = [0u64; 3];
    for (index, part) in parts.iter().enumerate() {
        numbers[index] = part
            .parse::<u64>()
            .map_err(|_| InstallerError::InvalidVersion(value.to_owned()))?;
    }
    Ok((numbers[0], numbers[1], numbers[2]))
}

fn default_backup_path(root: &Path) -> Result<PathBuf, InstallerError> {
    let parent = root.parent().ok_or_else(|| {
        InstallerError::UnsafePath(format!("{} has no parent directory", root.display()))
    })?;
    let name = root
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .ok_or_else(|| InstallerError::UnsafePath(format!("{} has no name", root.display())))?;
    let stamp = Utc::now().format("%Y%m%dT%H%M%SZ");
    Ok(parent.join(format!("{name}-pre-upgrade-{stamp}")))
}

fn aegis_service_running(paths: &StoragePaths) -> bool {
    let state = paths.root().join("run").join("service.json");
    match fs::read(&state) {
        Ok(bytes) => serde_json::from_slice::<serde_json::Value>(&bytes)
            .ok()
            .and_then(|value| {
                value
                    .get("pid")
                    .and_then(serde_json::Value::as_u64)
                    .and_then(|pid| u32::try_from(pid).ok())
            })
            .is_some_and(aegis_service::process_is_alive),
        Err(_) => false,
    }
}

fn delete_owned_artifacts(root: &Path) -> Result<Vec<String>, InstallerError> {
    let mut deleted = Vec::new();
    let fixed_files = [
        "platform.db",
        "platform.db-wal",
        "platform.db-shm",
        "platform.db-journal",
        "safe.db",
        "safe.db-wal",
        "safe.db-shm",
        "safe.db-journal",
        "owner.proof",
    ];
    for relative in fixed_files {
        let path = root.join(relative);
        if remove_path(&path)? {
            deleted.push(relative.to_owned());
        }
    }
    for relative in ["cores", "init", "run"] {
        let path = root.join(relative);
        if remove_path(&path)? {
            deleted.push(relative.to_owned());
        }
    }
    Ok(deleted)
}

fn remove_path(path: &Path) -> Result<bool, InstallerError> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error.into()),
    };
    if is_redirected(&metadata) {
        return Err(InstallerError::UnsafePath(path.display().to_string()));
    }
    if metadata.is_dir() {
        remove_tree(path)?;
    } else {
        fs::remove_file(path)?;
    }
    Ok(true)
}

fn remove_tree(path: &Path) -> Result<(), InstallerError> {
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        let entry_path = entry.path();
        let metadata = fs::symlink_metadata(&entry_path)?;
        if is_redirected(&metadata) {
            return Err(InstallerError::UnsafePath(entry_path.display().to_string()));
        }
        if metadata.is_dir() {
            remove_tree(&entry_path)?;
        } else {
            fs::remove_file(&entry_path)?;
        }
    }
    fs::remove_dir(path)?;
    Ok(())
}

#[cfg(windows)]
fn is_redirected(metadata: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0000_0400;
    metadata.file_type().is_symlink()
        || (metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT) != 0
}

#[cfg(not(windows))]
fn is_redirected(metadata: &fs::Metadata) -> bool {
    metadata.file_type().is_symlink()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_parse_and_compare() {
        assert_eq!(parse_version("0.1.0").expect("version"), (0, 1, 0));
        assert_eq!(parse_version("1.2.3-rc.1").expect("version"), (1, 2, 3));
        assert!(same_version("0.1.0", "0.1.0+local"));
        assert!(matches!(
            compare_versions("2.0.0", "0.1.0"),
            Err(InstallerError::DowngradeRefused { .. })
        ));
        assert!(matches!(
            ensure_compatible("9.9.9"),
            Err(InstallerError::IncompatibleVersion { .. })
        ));
    }

    #[test]
    fn malformed_versions_are_rejected() {
        assert!(matches!(
            parse_version("not.a.version"),
            Err(InstallerError::InvalidVersion(_))
        ));
        assert!(matches!(
            parse_version("1.2.3.4"),
            Err(InstallerError::InvalidVersion(_))
        ));
    }
}
