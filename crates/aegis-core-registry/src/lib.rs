//! Platform-owned Core registry and lifecycle safeguards (Batch 2 / Wave 1b).
//!
//! This crate records Platform-visible Core registrations and append-only
//! lifecycle changes. It never creates Core databases, generates Core
//! identities, deletes Core data, or grants Core authority. Destructive
//! uninstall and deletion are intentionally not implemented; retirement is
//! recorded through [`CoreRegistry::uninstall_preserve`], which leaves the
//! Core database, manifest, and storage path untouched.
//!
//! [`CoreRegistry::reconcile`] (also exposed as
//! [`CoreRegistry::migrate_legacy_platform_core`]) performs a conservative,
//! one-way import of Batch 1 `platform_core` rows only when `core_registry`
//! is empty. It never deletes or rewrites `platform_core`.

use aegis_contracts::core::{validate_core_id, validate_core_number, validate_display_name};
use aegis_contracts::lifecycle::CoreLifecycleState;
use aegis_contracts::roster::{
    validate_storage_path, CoreLifecycleChange, CoreRegistrationRequest, CoreRegistryEntry,
    RosterContractError, CORE_LIFECYCLE_CHANGE_SCHEMA, CORE_REGISTRY_SCHEMA,
};
use aegis_storage::{now_rfc3339, open_initialized, StoragePaths, PLATFORM_APPLICATION_ID};
use rusqlite::{params, Connection, OptionalExtension};
use thiserror::Error;

const REGISTRY_COMPONENT: &str = "core_registry";
const LEGACY_PLATFORM_CORE_TABLE: &str = "platform_core";

const ENTRY_COLUMNS: &str = "schema, core_uuid, core_number, display_name, lifecycle_state, \
                             storage_relative_path, initialized_at, updated_at";

#[derive(Debug, Error)]
pub enum RegistryError {
    #[error("Core registry storage failed: {0}")]
    Storage(#[from] aegis_storage::StorageError),
    #[error("Core registry SQLite operation failed: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("Core registration request is invalid: {0}")]
    InvalidRequest(#[from] RosterContractError),
    #[error("Core UUID is invalid")]
    InvalidCoreUuid,
    #[error("Core display name is invalid")]
    InvalidDisplayName,
    #[error("Core UUID is already registered: {0}")]
    DuplicateCoreUuid(String),
    #[error("Core number is already registered: {0}")]
    DuplicateCoreNumber(u8),
    #[error("registry entry conflicts with an existing foreign entry owned by {0}")]
    ForeignEntry(String),
    #[error("Core UUID is not registered: {0}")]
    CoreNotFound(String),
    #[error("Core lifecycle transition from {from} to {to} is not permitted")]
    IllegalTransition { from: String, to: String },
    #[error("Core lifecycle change reason is required")]
    MissingReason,
    #[error("Core storage path must not reference a Platform database")]
    ReservedStoragePath,
    #[error("Core lifecycle state is invalid: {0}")]
    InvalidLifecycleState(String),
}

/// Outcome of a conservative legacy-registry reconciliation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LegacyMigrationReport {
    /// Whether the Batch 1 `platform_core` table exists.
    pub platform_core_present: bool,
    /// Whether `core_registry` had zero rows when reconciliation started.
    pub registry_was_empty: bool,
    /// Number of legacy rows imported into `core_registry`.
    pub imported: usize,
    /// Number of legacy rows skipped because they were invalid or conflicted.
    pub skipped: usize,
}

/// Platform-side Core registry backed by the dedicated `platform.db`.
#[derive(Debug, Clone)]
pub struct CoreRegistry {
    paths: StoragePaths,
}

impl CoreRegistry {
    pub fn new(paths: StoragePaths) -> Self {
        Self { paths }
    }

    /// Opens the Platform database and ensures the registry tables exist.
    /// A Platform with zero registered Cores is valid.
    pub fn initialize(&self) -> Result<(), RegistryError> {
        let connection = self.open()?;
        connection.execute_batch(
            "CREATE TABLE IF NOT EXISTS core_registry (
                core_uuid TEXT PRIMARY KEY,
                core_number INTEGER NOT NULL UNIQUE CHECK(core_number IN (1, 2, 3, 6, 7)),
                display_name TEXT NOT NULL,
                lifecycle_state TEXT NOT NULL CHECK(lifecycle_state IN (
                    'design_only', 'approved', 'installed', 'active', 'suspended', 'retired'
                )),
                storage_relative_path TEXT NOT NULL UNIQUE,
                initialized_at TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                schema TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS core_lifecycle_change (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                schema TEXT NOT NULL,
                core_uuid TEXT NOT NULL REFERENCES core_registry(core_uuid) ON DELETE RESTRICT,
                from_state TEXT NOT NULL,
                to_state TEXT NOT NULL,
                reason TEXT NOT NULL,
                changed_at TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS core_lifecycle_change_by_core
                ON core_lifecycle_change(core_uuid, id);",
        )?;
        Ok(())
    }

    /// Registers a Core in the `installed` lifecycle state.
    pub fn register(
        &self,
        request: CoreRegistrationRequest,
    ) -> Result<CoreRegistryEntry, RegistryError> {
        request.validate()?;
        let core_uuid = canonical_core_uuid(&request.core_uuid)?;
        self.reject_reserved_storage_path(&request.storage_relative_path)?;

        let mut connection = self.open()?;
        let transaction = connection.transaction()?;

        let duplicate_uuid: Option<i64> = transaction
            .query_row(
                "SELECT 1 FROM core_registry WHERE core_uuid = ?1",
                [core_uuid.as_str()],
                |row| row.get(0),
            )
            .optional()?;
        if duplicate_uuid.is_some() {
            return Err(RegistryError::DuplicateCoreUuid(core_uuid));
        }

        let duplicate_number: Option<i64> = transaction
            .query_row(
                "SELECT 1 FROM core_registry WHERE core_number = ?1",
                [request.core_number],
                |row| row.get(0),
            )
            .optional()?;
        if duplicate_number.is_some() {
            return Err(RegistryError::DuplicateCoreNumber(request.core_number));
        }

        let path_owner: Option<String> = transaction
            .query_row(
                "SELECT core_uuid FROM core_registry WHERE storage_relative_path = ?1",
                [request.storage_relative_path.as_str()],
                |row| row.get(0),
            )
            .optional()?;
        if let Some(owner) = path_owner {
            return Err(RegistryError::ForeignEntry(owner));
        }

        let updated_at = now_rfc3339()?;
        transaction.execute(
            "INSERT INTO core_registry(
                schema, core_uuid, core_number, display_name, lifecycle_state,
                storage_relative_path, initialized_at, updated_at
             ) VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                CORE_REGISTRY_SCHEMA,
                core_uuid,
                request.core_number,
                request.display_name,
                CoreLifecycleState::Installed.as_str(),
                request.storage_relative_path,
                request.initialized_at,
                updated_at,
            ],
        )?;

        let entry = read_entry(&transaction, &core_uuid)?
            .ok_or_else(|| RegistryError::CoreNotFound(core_uuid.clone()))?;
        transaction.commit()?;
        Ok(entry)
    }

    pub fn get(&self, core_uuid: &str) -> Result<Option<CoreRegistryEntry>, RegistryError> {
        let core_uuid = canonical_core_uuid(core_uuid)?;
        self.initialize()?;
        let connection = self.open()?;
        read_entry(&connection, &core_uuid)
    }

    pub fn list(&self) -> Result<Vec<CoreRegistryEntry>, RegistryError> {
        self.initialize()?;
        let connection = self.open()?;
        let mut statement = connection.prepare(&format!(
            "SELECT {ENTRY_COLUMNS} FROM core_registry ORDER BY core_number ASC"
        ))?;
        let mut rows = statement.query([])?;
        let mut entries = Vec::new();
        while let Some(row) = rows.next()? {
            entries.push(build_entry(row)?);
        }
        Ok(entries)
    }

    /// Changes presentation metadata only. UUID, Core number, and storage path
    /// are immutable.
    pub fn update_display_name(
        &self,
        core_uuid: &str,
        new_name: &str,
    ) -> Result<CoreRegistryEntry, RegistryError> {
        let core_uuid = canonical_core_uuid(core_uuid)?;
        validate_display_name(new_name).map_err(|_| RegistryError::InvalidDisplayName)?;
        let updated_at = now_rfc3339()?;

        let mut connection = self.open()?;
        let transaction = connection.transaction()?;
        let changed = transaction.execute(
            "UPDATE core_registry SET display_name = ?1, updated_at = ?2 WHERE core_uuid = ?3",
            params![new_name, updated_at, core_uuid],
        )?;
        if changed == 0 {
            return Err(RegistryError::CoreNotFound(core_uuid));
        }
        let entry = read_entry(&transaction, &core_uuid)?
            .ok_or_else(|| RegistryError::CoreNotFound(core_uuid.clone()))?;
        transaction.commit()?;
        Ok(entry)
    }

    /// Suspends a Core without touching its storage. Only `installed` and
    /// `active` entries may be suspended. Data is preserved.
    pub fn deactivate(
        &self,
        core_uuid: &str,
        reason: &str,
    ) -> Result<CoreRegistryEntry, RegistryError> {
        self.apply_transition(
            core_uuid,
            reason,
            &[CoreLifecycleState::Installed, CoreLifecycleState::Active],
            CoreLifecycleState::Suspended,
        )
    }

    /// Activates an installed or suspended Core after an authorized runtime
    /// start. Active Cores are unchanged only because the caller checks state
    /// before calling; repeating the transition is rejected by the lifecycle
    /// rules. Data and storage path are preserved.
    pub fn activate(
        &self,
        core_uuid: &str,
        reason: &str,
    ) -> Result<CoreRegistryEntry, RegistryError> {
        self.apply_transition(
            core_uuid,
            reason,
            &[CoreLifecycleState::Installed, CoreLifecycleState::Suspended],
            CoreLifecycleState::Active,
        )
    }

    /// Reactivates a suspended Core. Only `suspended -> active` is permitted;
    /// installed, active, and retired Cores are rejected. Data and storage
    /// path are preserved.
    pub fn reactivate(
        &self,
        core_uuid: &str,
        reason: &str,
    ) -> Result<CoreRegistryEntry, RegistryError> {
        self.apply_transition(
            core_uuid,
            reason,
            &[CoreLifecycleState::Suspended],
            CoreLifecycleState::Active,
        )
    }

    /// Retires a Core while preserving all Core-owned data. Only `installed`,
    /// `suspended`, and `active` entries may be retired. This method updates
    /// only the Platform registry row and append-only history; it never
    /// deletes, truncates, renames, or otherwise modifies the Core database,
    /// manifest, or storage path. Destructive deletion is not implemented.
    pub fn uninstall_preserve(
        &self,
        core_uuid: &str,
        reason: &str,
    ) -> Result<CoreRegistryEntry, RegistryError> {
        self.apply_transition(
            core_uuid,
            reason,
            &[
                CoreLifecycleState::Installed,
                CoreLifecycleState::Active,
                CoreLifecycleState::Suspended,
            ],
            CoreLifecycleState::Retired,
        )
    }

    /// Append-only lifecycle history for a registered Core.
    pub fn lifecycle_history(
        &self,
        core_uuid: &str,
    ) -> Result<Vec<CoreLifecycleChange>, RegistryError> {
        let core_uuid = canonical_core_uuid(core_uuid)?;
        self.initialize()?;
        let connection = self.open()?;
        if read_entry(&connection, &core_uuid)?.is_none() {
            return Err(RegistryError::CoreNotFound(core_uuid));
        }
        let mut statement = connection.prepare(
            "SELECT schema, core_uuid, from_state, to_state, reason, changed_at
             FROM core_lifecycle_change WHERE core_uuid = ?1 ORDER BY id ASC",
        )?;
        let mut rows = statement.query([core_uuid.as_str()])?;
        let mut changes = Vec::new();
        while let Some(row) = rows.next()? {
            changes.push(build_change(row)?);
        }
        Ok(changes)
    }

    /// Conservative reconciliation with the Batch 1 `platform_core` table.
    ///
    /// This is a no-op unless `platform_core` exists and `core_registry` is
    /// empty. When both conditions hold, each legacy row is imported with the
    /// following deterministic rule:
    ///
    /// - `core_uuid` is canonicalized (invalid UUIDs are skipped).
    /// - `core_number` is preserved (prohibited/unapproved numbers are skipped).
    /// - `lifecycle_state` is preserved (unknown states are skipped).
    /// - `display_name` becomes `Legacy Core {core_number}`.
    /// - `storage_relative_path` becomes `cores/{core_uuid}/core.db`.
    /// - `initialized_at` and `updated_at` both become the legacy `created_at`.
    ///
    /// Rows are processed in `core_number, core_uuid` order and inserted with
    /// `INSERT OR IGNORE`, so a conflicting legacy row is skipped without
    /// affecting other rows. `platform_core` is never modified or deleted.
    pub fn reconcile(&self) -> Result<LegacyMigrationReport, RegistryError> {
        self.migrate_legacy_platform_core()
    }

    /// Alias for [`CoreRegistry::reconcile`].
    pub fn migrate_legacy_platform_core(&self) -> Result<LegacyMigrationReport, RegistryError> {
        self.initialize()?;
        let mut connection = self.open()?;
        let transaction = connection.transaction()?;

        let platform_core_present = table_exists(&transaction, LEGACY_PLATFORM_CORE_TABLE)?;
        let registry_count: i64 =
            transaction.query_row("SELECT COUNT(*) FROM core_registry", [], |row| row.get(0))?;
        let registry_was_empty = registry_count == 0;

        let mut report = LegacyMigrationReport {
            platform_core_present,
            registry_was_empty,
            imported: 0,
            skipped: 0,
        };

        if !platform_core_present || !registry_was_empty {
            transaction.commit()?;
            return Ok(report);
        }

        let legacy_rows = read_legacy_rows(&transaction)?;
        for (raw_uuid, raw_number, raw_state, created_at) in legacy_rows {
            let Ok(core_number) = u8::try_from(raw_number) else {
                report.skipped += 1;
                continue;
            };
            if validate_core_number(core_number).is_err() {
                report.skipped += 1;
                continue;
            }
            let Ok(core_uuid) = canonical_core_uuid(&raw_uuid) else {
                report.skipped += 1;
                continue;
            };
            let Some(lifecycle_state) = CoreLifecycleState::parse(&raw_state) else {
                report.skipped += 1;
                continue;
            };
            let display_name = format!("Legacy Core {core_number}");
            if validate_display_name(&display_name).is_err() {
                report.skipped += 1;
                continue;
            }
            let storage_relative_path = format!("cores/{core_uuid}/core.db");
            if validate_storage_path(&storage_relative_path).is_err() {
                report.skipped += 1;
                continue;
            }
            if created_at.trim().is_empty() {
                report.skipped += 1;
                continue;
            }

            let inserted = transaction.execute(
                "INSERT OR IGNORE INTO core_registry(
                    schema, core_uuid, core_number, display_name, lifecycle_state,
                    storage_relative_path, initialized_at, updated_at
                 ) VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![
                    CORE_REGISTRY_SCHEMA,
                    core_uuid,
                    core_number,
                    display_name,
                    lifecycle_state.as_str(),
                    storage_relative_path,
                    created_at.clone(),
                    created_at,
                ],
            )?;
            if inserted == 1 {
                report.imported += 1;
            } else {
                report.skipped += 1;
            }
        }

        transaction.commit()?;
        Ok(report)
    }

    fn apply_transition(
        &self,
        core_uuid: &str,
        reason: &str,
        allowed_from: &[CoreLifecycleState],
        to: CoreLifecycleState,
    ) -> Result<CoreRegistryEntry, RegistryError> {
        let core_uuid = canonical_core_uuid(core_uuid)?;
        if reason.trim().is_empty() {
            return Err(RegistryError::MissingReason);
        }

        let mut connection = self.open()?;
        let transaction = connection.transaction()?;
        let current = read_entry(&transaction, &core_uuid)?
            .ok_or_else(|| RegistryError::CoreNotFound(core_uuid.clone()))?;

        let from = current.lifecycle_state;
        if !allowed_from.contains(&from) {
            return Err(RegistryError::IllegalTransition {
                from: from.as_str().to_owned(),
                to: to.as_str().to_owned(),
            });
        }

        let changed_at = now_rfc3339()?;
        transaction.execute(
            "UPDATE core_registry SET lifecycle_state = ?1, updated_at = ?2 WHERE core_uuid = ?3",
            params![to.as_str(), changed_at, core_uuid],
        )?;
        transaction.execute(
            "INSERT INTO core_lifecycle_change(
                schema, core_uuid, from_state, to_state, reason, changed_at
             ) VALUES(?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                CORE_LIFECYCLE_CHANGE_SCHEMA,
                core_uuid,
                from.as_str(),
                to.as_str(),
                reason,
                changed_at,
            ],
        )?;

        let entry = read_entry(&transaction, &core_uuid)?
            .ok_or_else(|| RegistryError::CoreNotFound(core_uuid.clone()))?;
        transaction.commit()?;
        Ok(entry)
    }

    fn open(&self) -> Result<Connection, RegistryError> {
        open_initialized(
            &self.paths.platform_db(),
            PLATFORM_APPLICATION_ID,
            REGISTRY_COMPONENT,
        )
        .map_err(RegistryError::from)
    }

    fn reject_reserved_storage_path(&self, relative: &str) -> Result<(), RegistryError> {
        validate_storage_path(relative)?;
        let joined = self.paths.root().join(relative);
        if joined == self.paths.platform_db() || joined == self.paths.safe_db() {
            return Err(RegistryError::ReservedStoragePath);
        }
        Ok(())
    }
}

fn canonical_core_uuid(core_uuid: &str) -> Result<String, RegistryError> {
    validate_core_id(core_uuid)
        .map(|uuid| uuid.to_string())
        .map_err(|_| RegistryError::InvalidCoreUuid)
}

fn table_exists(connection: &Connection, name: &str) -> Result<bool, RegistryError> {
    let exists: Option<i64> = connection
        .query_row(
            "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1",
            [name],
            |row| row.get(0),
        )
        .optional()?;
    Ok(exists.is_some())
}

fn read_legacy_rows(
    connection: &Connection,
) -> Result<Vec<(String, i64, String, String)>, RegistryError> {
    let mut statement = connection.prepare(
        "SELECT core_uuid, core_number, lifecycle_state, created_at
         FROM platform_core ORDER BY core_number ASC, core_uuid ASC",
    )?;
    let mut rows = statement.query([])?;
    let mut legacy_rows = Vec::new();
    while let Some(row) = rows.next()? {
        legacy_rows.push((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?));
    }
    Ok(legacy_rows)
}

fn read_entry(
    connection: &Connection,
    core_uuid: &str,
) -> Result<Option<CoreRegistryEntry>, RegistryError> {
    let mut statement = connection.prepare(&format!(
        "SELECT {ENTRY_COLUMNS} FROM core_registry WHERE core_uuid = ?1"
    ))?;
    let mut rows = statement.query([core_uuid])?;
    match rows.next()? {
        Some(row) => Ok(Some(build_entry(row)?)),
        None => Ok(None),
    }
}

fn build_entry(row: &rusqlite::Row<'_>) -> Result<CoreRegistryEntry, RegistryError> {
    let lifecycle_raw: String = row.get(4)?;
    let lifecycle_state = CoreLifecycleState::parse(&lifecycle_raw)
        .ok_or_else(|| RegistryError::InvalidLifecycleState(lifecycle_raw.clone()))?;
    let entry = CoreRegistryEntry {
        schema: row.get(0)?,
        core_uuid: row.get(1)?,
        core_number: row.get(2)?,
        display_name: row.get(3)?,
        lifecycle_state,
        storage_relative_path: row.get(5)?,
        initialized_at: row.get(6)?,
        updated_at: row.get(7)?,
    };
    entry.validate()?;
    Ok(entry)
}

fn build_change(row: &rusqlite::Row<'_>) -> Result<CoreLifecycleChange, RegistryError> {
    let from_raw: String = row.get(2)?;
    let to_raw: String = row.get(3)?;
    let from = CoreLifecycleState::parse(&from_raw)
        .ok_or_else(|| RegistryError::InvalidLifecycleState(from_raw.clone()))?;
    let to = CoreLifecycleState::parse(&to_raw)
        .ok_or_else(|| RegistryError::InvalidLifecycleState(to_raw.clone()))?;
    let change = CoreLifecycleChange {
        schema: row.get(0)?,
        core_uuid: row.get(1)?,
        from,
        to,
        reason: row.get(4)?,
        changed_at: row.get(5)?,
    };
    change.validate()?;
    Ok(change)
}
