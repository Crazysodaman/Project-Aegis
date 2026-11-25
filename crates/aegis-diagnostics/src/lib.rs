//! Aegis diagnostics aggregation (Stage 2).
//!
//! Diagnostics is a read-only observer. It opens Platform, Safe, and Core
//! databases with SQLite read-only connections and never creates files,
//! directories, tables, rows, owner proofs, or audit entries. It never includes
//! secret material (owner tokens, DPAPI blobs, encryption keys) in a report.
//! Per-Core failures are captured as data so one damaged Core cannot abort the
//! whole report.

use std::fs;
use std::path::{Path, PathBuf};

use aegis_contracts::core::{CoreIdentity, CORE_IDENTITY_TABLE};
use aegis_contracts::lifecycle::CoreLifecycleState;
use aegis_contracts::roster::CoreRegistryEntry;
use aegis_core_integrity::{verify_core, IntegrityManifestStore};
use aegis_core_storage::CORE_APPLICATION_ID;
use aegis_storage::{open_read_only, PLATFORM_APPLICATION_ID, SAFE_APPLICATION_ID};
use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Version marker for the aggregated diagnostics report.
pub const DIAGNOSTICS_REPORT_SCHEMA: &str = "aegis.diagnostics.report.v1";

/// Table that Stage 5 module framework owns inside `platform.db`.
pub const MODULES_TABLE: &str = "aegis_modules";

/// Maximum characters of free-text STOP reason retained in a report.
pub const MAX_REPORTED_REASON_CHARS: usize = 160;

/// Overall Platform health.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OverallStatus {
    Healthy,
    Degraded,
    Critical,
}

impl OverallStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Healthy => "healthy",
            Self::Degraded => "degraded",
            Self::Critical => "critical",
        }
    }
}

/// Availability of the module registry table.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModuleStatus {
    Available,
    Unavailable,
    Error,
}

impl ModuleStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Available => "available",
            Self::Unavailable => "unavailable",
            Self::Error => "error",
        }
    }
}

#[derive(Debug, Error)]
pub enum DiagnosticsError {
    #[error("diagnostics root must be an absolute path")]
    RootNotAbsolute,
    #[error("diagnostics root exists but is not a directory")]
    RootNotDirectory,
}

/// Platform identity and zero-Core status.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlatformSection {
    pub available: bool,
    pub platform_id: Option<String>,
    pub callsign: Option<String>,
    pub zero_core: Option<bool>,
    pub core_count: Option<u32>,
    pub stop_active: Option<bool>,
    pub authority: Option<String>,
    pub error: Option<String>,
}

/// Safe STOP state and owner-proof presence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SafeSection {
    pub available: bool,
    pub owner_proof_present: bool,
    pub stop_active: Option<bool>,
    pub stop_revision: Option<u64>,
    pub stop_updated_at: Option<String>,
    pub stop_updated_by: Option<String>,
    pub stop_reason: Option<String>,
    pub error: Option<String>,
}

/// Per-Core integrity observation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CoreIntegrity {
    /// `verified`, a contract integrity status, `missing`, `unknown`, or
    /// `path_mismatch`.
    pub status: String,
    pub detail: Option<String>,
    pub expected_sha256: Option<String>,
    pub actual_sha256: Option<String>,
    pub expected_schema_revision: Option<u64>,
    pub actual_schema_revision: Option<u64>,
}

impl CoreIntegrity {
    fn new(status: impl Into<String>, detail: Option<String>) -> Self {
        Self {
            status: status.into(),
            detail,
            expected_sha256: None,
            actual_sha256: None,
            expected_schema_revision: None,
            actual_schema_revision: None,
        }
    }

    fn from_report(report: aegis_contracts::integrity::IntegrityReport) -> Self {
        Self {
            status: report.status.as_str().to_owned(),
            detail: report.detail,
            expected_sha256: report.expected_sha256,
            actual_sha256: report.actual_sha256,
            expected_schema_revision: report.expected_schema_revision,
            actual_schema_revision: report.actual_schema_revision,
        }
    }

    fn is_verified(&self) -> bool {
        self.status == "verified"
    }
}

/// One registered Core with its registry entry, database facts, and integrity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CoreReport {
    pub core_uuid: String,
    pub core_number: u8,
    pub display_name: String,
    pub lifecycle_state: String,
    pub storage_relative_path: String,
    pub database_path: String,
    pub database_bytes: Option<u64>,
    pub integrity: CoreIntegrity,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CoresSection {
    pub available: bool,
    pub count: usize,
    pub error: Option<String>,
    pub entries: Vec<CoreReport>,
}

/// Read-only module registry observation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModulesSection {
    pub status: ModuleStatus,
    pub count: Option<u64>,
    pub detail: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiagnosticsReport {
    pub schema: String,
    pub generated_at: String,
    pub status: OverallStatus,
    pub reasons: Vec<String>,
    pub platform: PlatformSection,
    pub safe: SafeSection,
    pub cores: CoresSection,
    pub modules: ModulesSection,
}

impl DiagnosticsReport {
    /// Serialize the versioned report.
    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(self)
    }
}

/// Read-only diagnostics aggregator for one Aegis storage root.
#[derive(Debug, Clone)]
pub struct Diagnostics {
    root: PathBuf,
}

impl Diagnostics {
    pub fn new(root: impl AsRef<Path>) -> Result<Self, DiagnosticsError> {
        let root = root.as_ref();
        if !root.is_absolute() {
            return Err(DiagnosticsError::RootNotAbsolute);
        }
        if root.exists() && !root.is_dir() {
            return Err(DiagnosticsError::RootNotDirectory);
        }
        Ok(Self {
            root: root.to_path_buf(),
        })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    fn platform_db(&self) -> PathBuf {
        self.root.join("platform.db")
    }

    fn safe_db(&self) -> PathBuf {
        self.root.join("safe.db")
    }

    fn owner_proof(&self) -> PathBuf {
        self.root.join("owner.proof")
    }

    /// Collect a full report. This method never returns an error: unavailable
    /// components and per-Core failures are represented in the report.
    pub fn collect(&self) -> DiagnosticsReport {
        let generated_at = chrono::Utc::now().to_rfc3339();
        let mut reasons = Vec::new();

        let platform = self.collect_platform();
        let safe = self.collect_safe();
        let cores = self.collect_cores();
        let modules = self.collect_modules();

        let platform_available = platform.available;
        let safe_available = safe.available;

        if !platform_available {
            reasons.push(format!(
                "Platform is unavailable: {}",
                platform.error.as_deref().unwrap_or("unknown error")
            ));
        }
        if !safe_available {
            reasons.push(format!(
                "Safe is unavailable: {}",
                safe.error.as_deref().unwrap_or("unknown error")
            ));
        }

        if let Some(error) = &cores.error {
            reasons.push(format!("Core registry is unavailable: {error}"));
        }
        for core in &cores.entries {
            if !core.integrity.is_verified() {
                reasons.push(format!(
                    "Core {} is {}: {}",
                    core.core_uuid,
                    core.integrity.status,
                    core.integrity.detail.as_deref().unwrap_or("no detail")
                ));
            }
        }
        match modules.status {
            ModuleStatus::Available => {}
            ModuleStatus::Unavailable => reasons.push(format!(
                "Module registry is unavailable: {}",
                modules.detail.as_deref().unwrap_or("unknown reason")
            )),
            ModuleStatus::Error => reasons.push(format!(
                "Module registry could not be read: {}",
                modules.detail.as_deref().unwrap_or("unknown error")
            )),
        }

        let status = if !platform_available || !safe_available {
            OverallStatus::Critical
        } else if cores.error.is_some()
            || !cores.available
            || modules.status != ModuleStatus::Available
            || cores
                .entries
                .iter()
                .any(|core| !core.integrity.is_verified())
        {
            OverallStatus::Degraded
        } else {
            OverallStatus::Healthy
        };

        DiagnosticsReport {
            schema: DIAGNOSTICS_REPORT_SCHEMA.to_owned(),
            generated_at,
            status,
            reasons,
            platform,
            safe,
            cores,
            modules,
        }
    }

    fn collect_platform(&self) -> PlatformSection {
        let database = self.platform_db();
        if !database.is_file() {
            return unavailable_platform("Platform database is missing");
        }
        let connection = match open_read_only(&database, PLATFORM_APPLICATION_ID) {
            Ok(connection) => connection,
            Err(error) => return unavailable_platform(error.to_string()),
        };

        let identity: Option<(String, String)> = match connection
            .query_row(
                "SELECT platform_id, callsign FROM platform_identity WHERE singleton = 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()
        {
            Ok(identity) => identity,
            Err(error) => return unavailable_platform(error.to_string()),
        };
        let Some((platform_id, callsign)) = identity else {
            return unavailable_platform("Platform identity row is missing");
        };

        let core_count = match read_core_count(&connection) {
            Ok(count) => count,
            Err(error) => return unavailable_platform(error),
        };

        let stop_active = match self.read_stop_active() {
            Ok(value) => value,
            Err(error) => return unavailable_platform(error),
        };

        PlatformSection {
            available: true,
            platform_id: Some(platform_id),
            callsign: Some(callsign),
            zero_core: Some(core_count == 0),
            core_count: Some(core_count),
            stop_active,
            authority: Some("local_single".to_owned()),
            error: None,
        }
    }

    fn read_stop_active(&self) -> Result<Option<bool>, String> {
        let database = self.safe_db();
        if !database.is_file() {
            return Ok(None);
        }
        let connection =
            open_read_only(&database, SAFE_APPLICATION_ID).map_err(|error| error.to_string())?;
        let active: Option<i64> = connection
            .query_row(
                "SELECT active FROM safe_stop WHERE singleton = 1",
                [],
                |row| row.get(0),
            )
            .optional()
            .map_err(|error| error.to_string())?;
        Ok(active.map(|value| value != 0))
    }

    fn collect_safe(&self) -> SafeSection {
        let owner_proof_present = self.owner_proof().is_file();
        let database = self.safe_db();
        if !database.is_file() {
            return SafeSection {
                available: false,
                owner_proof_present,
                stop_active: None,
                stop_revision: None,
                stop_updated_at: None,
                stop_updated_by: None,
                stop_reason: None,
                error: Some("Safe database is missing".to_owned()),
            };
        }

        let connection = match open_read_only(&database, SAFE_APPLICATION_ID) {
            Ok(connection) => connection,
            Err(error) => {
                return SafeSection {
                    available: false,
                    owner_proof_present,
                    stop_active: None,
                    stop_revision: None,
                    stop_updated_at: None,
                    stop_updated_by: None,
                    stop_reason: None,
                    error: Some(error.to_string()),
                }
            }
        };

        let row: Result<Option<StopRow>, rusqlite::Error> = connection
            .query_row(
                "SELECT active, revision, updated_at, updated_by, reason
                 FROM safe_stop WHERE singleton = 1",
                [],
                |row| {
                    Ok(StopRow {
                        active: row.get::<_, i64>(0)? != 0,
                        revision: row.get(1)?,
                        updated_at: row.get(2)?,
                        updated_by: row.get(3)?,
                        reason: row.get(4)?,
                    })
                },
            )
            .optional();

        match row {
            Ok(Some(stop)) => {
                let available = owner_proof_present;
                SafeSection {
                    available,
                    owner_proof_present,
                    stop_active: Some(stop.active),
                    stop_revision: u64::try_from(stop.revision).ok(),
                    stop_updated_at: Some(stop.updated_at),
                    stop_updated_by: Some(redact(&stop.updated_by, 64)),
                    stop_reason: Some(redact(&stop.reason, MAX_REPORTED_REASON_CHARS)),
                    error: if owner_proof_present {
                        None
                    } else {
                        Some("owner proof is missing".to_owned())
                    },
                }
            }
            Ok(None) => SafeSection {
                available: false,
                owner_proof_present,
                stop_active: None,
                stop_revision: None,
                stop_updated_at: None,
                stop_updated_by: None,
                stop_reason: None,
                error: Some("Safe STOP row is missing".to_owned()),
            },
            Err(error) => SafeSection {
                available: false,
                owner_proof_present,
                stop_active: None,
                stop_revision: None,
                stop_updated_at: None,
                stop_updated_by: None,
                stop_reason: None,
                error: Some(error.to_string()),
            },
        }
    }

    fn collect_cores(&self) -> CoresSection {
        let database = self.platform_db();
        if !database.is_file() {
            return CoresSection {
                available: false,
                count: 0,
                error: Some("Platform database is missing".to_owned()),
                entries: Vec::new(),
            };
        }

        let registry_exists = match read_only_table_exists(&database, "core_registry") {
            Ok(exists) => exists,
            Err(error) => {
                return CoresSection {
                    available: false,
                    count: 0,
                    error: Some(error),
                    entries: Vec::new(),
                }
            }
        };
        if !registry_exists {
            return CoresSection {
                available: true,
                count: 0,
                error: None,
                entries: Vec::new(),
            };
        }

        let entries = match read_registry_entries(&database) {
            Ok(entries) => entries,
            Err(error) => {
                return CoresSection {
                    available: false,
                    count: 0,
                    error: Some(error),
                    entries: Vec::new(),
                }
            }
        };
        let skipped = entries.skipped;
        let reports = entries
            .entries
            .iter()
            .map(|entry| self.collect_core(entry))
            .collect::<Vec<_>>();
        let error =
            (skipped > 0).then(|| format!("{skipped} registry rows are invalid or unreadable"));

        CoresSection {
            available: true,
            count: reports.len(),
            error,
            entries: reports,
        }
    }

    fn collect_core(&self, entry: &CoreRegistryEntry) -> CoreReport {
        let directory = self.root.join("cores").join(&entry.core_uuid);
        let database_path = directory.join("core.db");
        let manifest_path = directory.join("core.integrity.json");
        let expected_relative = format!("cores/{}/core.db", entry.core_uuid);
        let database_bytes = fs::metadata(&database_path)
            .ok()
            .filter(|metadata| metadata.is_file())
            .map(|metadata| metadata.len());

        let integrity = if entry.storage_relative_path != expected_relative {
            CoreIntegrity::new(
                "path_mismatch",
                Some(format!(
                    "registry path {} does not match the canonical Core path",
                    entry.storage_relative_path
                )),
            )
        } else if !database_path.is_file() {
            CoreIntegrity::new("missing", Some("Core database file is missing".to_owned()))
        } else if !manifest_path.is_file() {
            CoreIntegrity::new(
                "missing_manifest",
                Some("Core integrity manifest is missing".to_owned()),
            )
        } else {
            inspect_core(&entry.core_uuid, &database_path, &manifest_path)
        };

        CoreReport {
            core_uuid: entry.core_uuid.clone(),
            core_number: entry.core_number,
            display_name: entry.display_name.clone(),
            lifecycle_state: entry.lifecycle_state.as_str().to_owned(),
            storage_relative_path: entry.storage_relative_path.clone(),
            database_path: database_path.display().to_string(),
            database_bytes,
            integrity,
        }
    }

    fn collect_modules(&self) -> ModulesSection {
        let database = self.platform_db();
        if !database.is_file() {
            return ModulesSection {
                status: ModuleStatus::Unavailable,
                count: None,
                detail: Some("Platform database is missing".to_owned()),
            };
        }
        match read_only_table_count(&database, MODULES_TABLE) {
            Ok(Some(count)) => ModulesSection {
                status: ModuleStatus::Available,
                count: Some(count),
                detail: None,
            },
            Ok(None) => ModulesSection {
                status: ModuleStatus::Unavailable,
                count: None,
                detail: Some(format!("{MODULES_TABLE} table is absent")),
            },
            Err(error) => ModulesSection {
                status: ModuleStatus::Error,
                count: None,
                detail: Some(error),
            },
        }
    }
}

struct StopRow {
    active: bool,
    revision: i64,
    updated_at: String,
    updated_by: String,
    reason: String,
}

struct RegistryEntries {
    entries: Vec<CoreRegistryEntry>,
    skipped: usize,
}

fn unavailable_platform(message: impl Into<String>) -> PlatformSection {
    PlatformSection {
        available: false,
        platform_id: None,
        callsign: None,
        zero_core: None,
        core_count: None,
        stop_active: None,
        authority: None,
        error: Some(message.into()),
    }
}

fn read_core_count(connection: &Connection) -> Result<u32, String> {
    let registry_table: Option<String> = connection
        .query_row(
            "SELECT name FROM sqlite_master WHERE type = 'table' AND name = 'core_registry'",
            [],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| error.to_string())?;
    let table = if registry_table.is_some() {
        "core_registry"
    } else {
        "platform_core"
    };
    let count: i64 = connection
        .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
            row.get(0)
        })
        .map_err(|error| error.to_string())?;
    u32::try_from(count).map_err(|_| "Platform core count is negative".to_owned())
}

fn inspect_core(core_uuid: &str, database_path: &Path, manifest_path: &Path) -> CoreIntegrity {
    let identity = read_identity(database_path);
    let expected_revision = IntegrityManifestStore::new(manifest_path)
        .read()
        .ok()
        .flatten()
        .map(|manifest| manifest.schema_revision)
        .unwrap_or(1);
    let report = verify_core(
        core_uuid,
        database_path,
        manifest_path,
        expected_revision,
        identity.as_ref(),
    );
    let status = report.status;
    let mut integrity = CoreIntegrity::from_report(report);
    if status == aegis_contracts::integrity::IntegrityStatus::Verified && identity.is_none() {
        integrity.status = "unknown".to_owned();
        integrity.detail = Some("Core identity could not be established".to_owned());
    }
    integrity
}

fn read_identity(database_path: &Path) -> Option<CoreIdentity> {
    let connection = open_read_only(database_path, CORE_APPLICATION_ID).ok()?;
    let table_exists: bool = connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1)",
            [CORE_IDENTITY_TABLE],
            |row| row.get(0),
        )
        .ok()?;
    if !table_exists {
        return None;
    }
    let identity = connection
        .query_row(
            "SELECT schema, core_uuid, core_number, display_name, created_at, identity_revision
             FROM core_identity WHERE singleton = 1",
            [],
            |row| {
                Ok(CoreIdentity {
                    schema: row.get(0)?,
                    core_uuid: row.get(1)?,
                    core_number: row.get(2)?,
                    display_name: row.get(3)?,
                    created_at: row.get(4)?,
                    identity_revision: row.get(5)?,
                })
            },
        )
        .optional()
        .ok()??;
    identity.validate().ok()?;
    Some(identity)
}

fn read_registry_entries(database: &Path) -> Result<RegistryEntries, String> {
    let connection =
        open_read_only(database, PLATFORM_APPLICATION_ID).map_err(|error| error.to_string())?;
    let mut statement = connection
        .prepare(
            "SELECT schema, core_uuid, core_number, display_name, lifecycle_state,
                    storage_relative_path, initialized_at, updated_at
             FROM core_registry ORDER BY core_number ASC",
        )
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, String>(6)?,
                row.get::<_, String>(7)?,
            ))
        })
        .map_err(|error| error.to_string())?;

    let mut entries = Vec::new();
    let mut skipped = 0;
    for row in rows {
        let (schema, core_uuid, core_number, display_name, state, path, initialized_at, updated_at) =
            match row {
                Ok(row) => row,
                Err(_) => {
                    skipped += 1;
                    continue;
                }
            };
        let Some(core_number) = u8::try_from(core_number).ok() else {
            skipped += 1;
            continue;
        };
        let Some(lifecycle_state) = CoreLifecycleState::parse(&state) else {
            skipped += 1;
            continue;
        };
        let entry = CoreRegistryEntry {
            schema,
            core_uuid,
            core_number,
            display_name,
            lifecycle_state,
            storage_relative_path: path,
            initialized_at,
            updated_at,
        };
        if entry.validate().is_err() {
            skipped += 1;
            continue;
        }
        entries.push(entry);
    }
    Ok(RegistryEntries { entries, skipped })
}

fn read_only_table_exists(path: &Path, table: &str) -> Result<bool, String> {
    let connection = open_read_only(path, PLATFORM_APPLICATION_ID)
        .map_err(|error| format!("Platform database could not be opened: {error}"))?;
    let exists: Option<i64> = connection
        .query_row(
            "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1",
            [table],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| error.to_string())?;
    Ok(exists.is_some())
}

fn read_only_table_count(path: &Path, table: &str) -> Result<Option<u64>, String> {
    if !read_only_table_exists(path, table)? {
        return Ok(None);
    }
    let connection = open_read_only(path, PLATFORM_APPLICATION_ID)
        .map_err(|error| format!("Platform database could not be opened: {error}"))?;
    let count: i64 = connection
        .query_row(&format!("SELECT COUNT(*) FROM \"{table}\""), [], |row| {
            row.get(0)
        })
        .map_err(|error| error.to_string())?;
    u64::try_from(count)
        .map(Some)
        .map_err(|_| "module count is negative".to_owned())
}

fn redact(value: &str, maximum: usize) -> String {
    let mut cleaned: String = value
        .chars()
        .filter(|character| !character.is_control())
        .take(maximum)
        .collect();
    if value.chars().count() > maximum {
        cleaned.push_str("...");
    }
    cleaned
}
