//! Platform-owned module registry and lifecycle (Stage 5).
//!
//! Modules are registered from a validated manifest into `platform.db`. This
//! stage stores declarations and lifecycle state only: modules are not
//! executed, not loaded, and not sandboxed, and no module receives Platform or
//! Core data access. Disable and uninstall preserve module data.

use aegis_storage::{now_rfc3339, open_initialized, StoragePaths, PLATFORM_APPLICATION_ID};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::manifest::{
    is_reserved_module_id, is_sha256, validate_module_id, ManifestError, ModuleManifest,
    MODULE_MANIFEST_SCHEMA,
};

pub const MODULE_REGISTRY_SCHEMA: &str = "aegis.module.registry.v1";
pub const MODULE_EVENT_SCHEMA: &str = "aegis.module.event.v1";
pub const MODULE_VERIFICATION_SCHEMA: &str = "aegis.module.verification.v1";

const REGISTRY_COMPONENT: &str = "modules";

const RECORD_COLUMNS: &str = "schema, module_id, name, version, publisher, lifecycle_state, \
                             capabilities_json, permissions_json, manifest_sha256, \
                             installed_at, updated_at";

const RECORD_AND_MANIFEST_COLUMNS: &str =
    "schema, module_id, name, version, publisher, lifecycle_state, capabilities_json, \
     permissions_json, manifest_sha256, installed_at, updated_at, manifest_json";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModuleLifecycleState {
    Installed,
    Enabled,
    Disabled,
    Failed,
    Uninstalled,
}

impl ModuleLifecycleState {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Installed => "installed",
            Self::Enabled => "enabled",
            Self::Disabled => "disabled",
            Self::Failed => "failed",
            Self::Uninstalled => "uninstalled",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "installed" => Some(Self::Installed),
            "enabled" => Some(Self::Enabled),
            "disabled" => Some(Self::Disabled),
            "failed" => Some(Self::Failed),
            "uninstalled" => Some(Self::Uninstalled),
            _ => None,
        }
    }

    fn is_available_dependency(self) -> bool {
        !matches!(self, Self::Uninstalled | Self::Failed)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModuleRecord {
    pub schema: String,
    pub module_id: String,
    pub name: String,
    pub version: String,
    pub publisher: String,
    pub lifecycle_state: ModuleLifecycleState,
    pub capabilities: Vec<String>,
    pub permissions: Vec<String>,
    pub manifest_sha256: String,
    pub installed_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModuleEvent {
    pub schema: String,
    pub module_id: String,
    pub action: String,
    pub from_state: Option<ModuleLifecycleState>,
    pub to_state: ModuleLifecycleState,
    pub detail: String,
    pub event_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModuleInspection {
    pub record: ModuleRecord,
    pub manifest: ModuleManifest,
    pub events: Vec<ModuleEvent>,
}

/// A tool declared by a module, with its Platform-facing effective name.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModuleToolBinding {
    pub module_id: String,
    pub module_version: String,
    pub tool_name: String,
    pub effective_name: String,
    pub description: String,
    pub permission: String,
    pub sensitive: bool,
    pub approval_required: bool,
    pub timeout_ms: u64,
    pub input_schema: serde_json::Value,
    pub enabled: bool,
}

/// Stable Platform-facing tool name for a module declaration.
pub fn effective_module_tool_name(module_id: &str, tool_name: &str) -> String {
    format!("module.{module_id}.{tool_name}")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModuleVerificationStatus {
    Verified,
    InvalidManifest,
    DigestMismatch,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModuleVerification {
    pub schema: String,
    pub module_id: String,
    pub status: ModuleVerificationStatus,
    pub expected_sha256: Option<String>,
    pub actual_sha256: Option<String>,
    pub checked_at: String,
    pub detail: Option<String>,
}

#[derive(Debug, Error)]
pub enum ModuleError {
    #[error("module storage failed: {0}")]
    Storage(#[from] aegis_storage::StorageError),
    #[error("module registry SQLite operation failed: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("module manifest JSON is invalid: {0}")]
    Json(#[from] serde_json::Error),
    #[error("module package I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("module package is invalid: {0}")]
    Package(String),
    #[error("module package is already materialized: {0}")]
    PackageAlreadyMaterialized(String),
    #[error("module manifest is invalid: {0}")]
    Manifest(#[from] ManifestError),
    #[error("module already registered: {0}")]
    DuplicateModuleId(String),
    #[error("module dependency is not installed: {0}")]
    UnknownDependency(String),
    #[error("module dependency {module_id} is unavailable in state {state}")]
    DependencyUnavailable { module_id: String, state: String },
    #[error("module dependency {module_id} requires {requirement}, found {actual}")]
    DependencyVersionMismatch {
        module_id: String,
        requirement: String,
        actual: String,
    },
    #[error("module {module_id} is required by enabled module {dependent}")]
    DependencyInUse {
        module_id: String,
        dependent: String,
    },
    #[error("module is not registered: {0}")]
    ModuleNotFound(String),
    #[error("module {module_id} has no declared permission requests")]
    NoDeclaredPermissions { module_id: String },
    #[error("module lifecycle transition from {from} to {to} is not permitted")]
    IllegalTransition { from: String, to: String },
    #[error("module lifecycle state is invalid: {0}")]
    InvalidLifecycleState(String),
    #[error("expected module digest is invalid")]
    InvalidExpectedDigest,
    #[error("module manifest digest mismatch: expected {expected}, actual {actual}")]
    ExpectedDigestMismatch { expected: String, actual: String },
    #[error("owner authorization failed")]
    Unauthorized,
    #[error("Safe authorization failed: {0}")]
    Safe(#[from] aegis_safe::SafeError),
}

/// Platform-side module registry backed by the dedicated `platform.db`.
#[derive(Debug, Clone)]
pub struct ModuleRegistry {
    paths: StoragePaths,
}

impl ModuleRegistry {
    pub fn new(paths: StoragePaths) -> Self {
        Self { paths }
    }

    /// Opens the Platform database and ensures the module tables exist.
    pub fn initialize(&self) -> Result<(), ModuleError> {
        self.open().map(|_| ())
    }

    /// Validates and records a manifest in the `installed` state.
    ///
    /// `expected_sha256`, when supplied, must equal the manifest's computed
    /// content digest. The digest is content integrity, not authenticity.
    pub fn install(
        &self,
        manifest_json: &str,
        expected_sha256: Option<&str>,
    ) -> Result<ModuleRecord, ModuleError> {
        let manifest: ModuleManifest = serde_json::from_str(manifest_json)?;
        manifest.validate()?;
        let digest = manifest.computed_digest()?;

        if let Some(expected) = expected_sha256 {
            if !is_sha256(expected) {
                return Err(ModuleError::InvalidExpectedDigest);
            }
            if expected != digest {
                return Err(ModuleError::ExpectedDigestMismatch {
                    expected: expected.to_owned(),
                    actual: digest,
                });
            }
        }

        self.install_validated(manifest, digest)
    }

    fn install_validated(
        &self,
        manifest: ModuleManifest,
        digest: String,
    ) -> Result<ModuleRecord, ModuleError> {
        let canonical = serde_json::to_string(&manifest)?;
        let capabilities_json = serde_json::to_string(&manifest.capabilities)?;
        let permissions: Vec<&str> = manifest
            .permissions
            .iter()
            .map(|permission| permission.name.as_str())
            .collect();
        let permissions_json = serde_json::to_string(&permissions)?;
        let timestamp = now_rfc3339()?;

        let mut connection = self.open()?;
        let transaction = connection.transaction()?;

        if read_record(&transaction, &manifest.module_id)?.is_some() {
            return Err(ModuleError::DuplicateModuleId(manifest.module_id));
        }
        ensure_dependencies_available(&transaction, &manifest)?;

        transaction.execute(
            "INSERT INTO aegis_modules(
                module_id, schema, name, version, publisher, lifecycle_state,
                capabilities_json, permissions_json, manifest_json, manifest_sha256,
                installed_at, updated_at
             ) VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            params![
                manifest.module_id,
                MODULE_MANIFEST_SCHEMA,
                manifest.name,
                manifest.version,
                manifest.publisher,
                ModuleLifecycleState::Installed.as_str(),
                capabilities_json,
                permissions_json,
                canonical,
                digest,
                timestamp,
                timestamp,
            ],
        )?;
        insert_event(
            &transaction,
            &manifest.module_id,
            "install",
            None,
            ModuleLifecycleState::Installed,
            &format!("installed version {}", manifest.version),
            &timestamp,
        )?;

        let record = read_record(&transaction, &manifest.module_id)?
            .ok_or_else(|| ModuleError::ModuleNotFound(manifest.module_id.clone()))?;
        transaction.commit()?;
        Ok(record)
    }

    /// Read-only enable precondition check.
    ///
    /// Verifies module existence, lifecycle transition legality, declared
    /// permissions, stored manifest integrity, and dependency availability and
    /// version compatibility. It never mutates the registry, so callers (UI,
    /// CLI, automation) can preview whether [`Self::enable`] will succeed.
    pub fn check_enable(&self, module_id: &str) -> Result<(), ModuleError> {
        validate_module_id(module_id)?;
        let connection = self.open()?;
        check_enable_preconditions(&connection, module_id)
    }

    /// Read-only disable precondition check.
    ///
    /// Verifies module existence, lifecycle transition legality, and that no
    /// currently enabled module depends on this one. It never mutates the
    /// registry.
    pub fn check_disable(&self, module_id: &str) -> Result<(), ModuleError> {
        validate_module_id(module_id)?;
        let connection = self.open()?;
        check_disable_preconditions(&connection, module_id)
    }

    /// Enables a module only after its declared permission requests, manifest
    /// integrity, and dependency compatibility are checked. No data access is
    /// granted.
    pub fn enable(&self, module_id: &str) -> Result<ModuleRecord, ModuleError> {
        validate_module_id(module_id)?;
        let mut connection = self.open()?;
        let transaction = connection.transaction()?;

        check_enable_preconditions(&transaction, module_id)?;
        let record = read_record(&transaction, module_id)?
            .ok_or_else(|| ModuleError::ModuleNotFound(module_id.to_owned()))?;
        let from = record.lifecycle_state;

        let timestamp = now_rfc3339()?;
        transaction.execute(
            "UPDATE aegis_modules SET lifecycle_state = ?1, updated_at = ?2 WHERE module_id = ?3",
            params![ModuleLifecycleState::Enabled.as_str(), timestamp, module_id],
        )?;
        insert_event(
            &transaction,
            module_id,
            "enable",
            Some(from),
            ModuleLifecycleState::Enabled,
            "enabled without granting data access",
            &timestamp,
        )?;

        let updated = read_record(&transaction, module_id)?
            .ok_or_else(|| ModuleError::ModuleNotFound(module_id.to_owned()))?;
        transaction.commit()?;
        Ok(updated)
    }

    /// Marks a module disabled without touching its manifest or data.
    ///
    /// Refused while another module is enabled and declares a dependency on
    /// this one, so an enabled dependent never loses a required provider.
    pub fn disable(&self, module_id: &str) -> Result<ModuleRecord, ModuleError> {
        validate_module_id(module_id)?;
        let mut connection = self.open()?;
        let transaction = connection.transaction()?;
        check_disable_preconditions(&transaction, module_id)?;
        let record = read_record(&transaction, module_id)?
            .ok_or_else(|| ModuleError::ModuleNotFound(module_id.to_owned()))?;
        let from = record.lifecycle_state;

        let timestamp = now_rfc3339()?;
        transaction.execute(
            "UPDATE aegis_modules SET lifecycle_state = ?1, updated_at = ?2 WHERE module_id = ?3",
            params![
                ModuleLifecycleState::Disabled.as_str(),
                timestamp,
                module_id
            ],
        )?;
        insert_event(
            &transaction,
            module_id,
            "disable",
            Some(from),
            ModuleLifecycleState::Disabled,
            "disabled while preserving data",
            &timestamp,
        )?;

        let updated = read_record(&transaction, module_id)?
            .ok_or_else(|| ModuleError::ModuleNotFound(module_id.to_owned()))?;
        transaction.commit()?;
        Ok(updated)
    }

    /// Marks a module uninstalled without deleting its manifest, events, or
    /// data by default.
    pub fn uninstall(&self, module_id: &str) -> Result<ModuleRecord, ModuleError> {
        self.transition(
            module_id,
            &[
                ModuleLifecycleState::Installed,
                ModuleLifecycleState::Enabled,
                ModuleLifecycleState::Disabled,
                ModuleLifecycleState::Failed,
            ],
            ModuleLifecycleState::Uninstalled,
            "uninstall",
            "uninstalled while preserving data",
        )
    }

    pub fn list(&self) -> Result<Vec<ModuleRecord>, ModuleError> {
        let connection = self.open()?;
        let mut statement = connection.prepare(&format!(
            "SELECT {RECORD_COLUMNS} FROM aegis_modules ORDER BY module_id ASC"
        ))?;
        let mut rows = statement.query([])?;
        let mut records = Vec::new();
        while let Some(row) = rows.next()? {
            records.push(build_record(row)?);
        }
        Ok(records)
    }

    /// Every declared tool from installed modules, including disabled ones so
    /// the Platform can show availability honestly.
    pub fn tool_bindings(&self) -> Result<Vec<ModuleToolBinding>, ModuleError> {
        let connection = self.open()?;
        let mut statement = connection.prepare(
            "SELECT module_id, version, lifecycle_state, manifest_json
             FROM aegis_modules WHERE lifecycle_state <> 'uninstalled'
             ORDER BY module_id ASC",
        )?;
        let mut rows = statement.query([])?;
        let mut bindings = Vec::new();
        while let Some(row) = rows.next()? {
            let module_id: String = row.get(0)?;
            let module_version: String = row.get(1)?;
            let state: String = row.get(2)?;
            let manifest_json: String = row.get(3)?;
            let enabled = ModuleLifecycleState::parse(&state)
                .is_some_and(|state| state == ModuleLifecycleState::Enabled);
            let manifest: ModuleManifest = serde_json::from_str(&manifest_json)?;
            for tool in &manifest.tools {
                bindings.push(ModuleToolBinding {
                    module_id: module_id.clone(),
                    module_version: module_version.clone(),
                    tool_name: tool.name.clone(),
                    effective_name: effective_module_tool_name(&module_id, &tool.name),
                    description: tool.description.clone(),
                    permission: tool.permission.clone(),
                    sensitive: tool.sensitive,
                    approval_required: tool.approval_required || tool.sensitive,
                    timeout_ms: tool
                        .timeout_ms
                        .unwrap_or(crate::manifest::DEFAULT_TOOL_TIMEOUT_MS),
                    input_schema: tool.input_schema.clone(),
                    enabled,
                });
            }
        }
        Ok(bindings)
    }

    pub fn inspect(&self, module_id: &str) -> Result<ModuleInspection, ModuleError> {
        validate_module_id(module_id)?;
        let connection = self.open()?;
        let (record, manifest_json) = read_record_and_manifest(&connection, module_id)?
            .ok_or_else(|| ModuleError::ModuleNotFound(module_id.to_owned()))?;
        let manifest: ModuleManifest = serde_json::from_str(&manifest_json)?;
        let events = read_events(&connection, module_id)?;
        Ok(ModuleInspection {
            record,
            manifest,
            events,
        })
    }

    /// Recomputes the stored manifest digest. A `verified` report means the
    /// stored bytes are consistent with the digest recorded at install time; it
    /// does not establish publisher authenticity or installation trust.
    pub fn verify(&self, module_id: &str) -> Result<ModuleVerification, ModuleError> {
        validate_module_id(module_id)?;
        let connection = self.open()?;
        let (record, manifest_json) = read_record_and_manifest(&connection, module_id)?
            .ok_or_else(|| ModuleError::ModuleNotFound(module_id.to_owned()))?;
        let checked_at = now_rfc3339()?;
        let expected = record.manifest_sha256;

        let manifest: ModuleManifest = match serde_json::from_str(&manifest_json) {
            Ok(manifest) => manifest,
            Err(error) => {
                return Ok(ModuleVerification {
                    schema: MODULE_VERIFICATION_SCHEMA.to_owned(),
                    module_id: module_id.to_owned(),
                    status: ModuleVerificationStatus::InvalidManifest,
                    expected_sha256: Some(expected),
                    actual_sha256: None,
                    checked_at,
                    detail: Some(error.to_string()),
                });
            }
        };

        if let Err(error) = manifest.validate_structure() {
            return Ok(ModuleVerification {
                schema: MODULE_VERIFICATION_SCHEMA.to_owned(),
                module_id: module_id.to_owned(),
                status: ModuleVerificationStatus::InvalidManifest,
                expected_sha256: Some(expected),
                actual_sha256: None,
                checked_at,
                detail: Some(error.to_string()),
            });
        }

        let actual = manifest.computed_digest()?;
        if actual != expected || manifest.integrity.digest != expected {
            return Ok(ModuleVerification {
                schema: MODULE_VERIFICATION_SCHEMA.to_owned(),
                module_id: module_id.to_owned(),
                status: ModuleVerificationStatus::DigestMismatch,
                expected_sha256: Some(expected),
                actual_sha256: Some(actual),
                checked_at,
                detail: Some("stored manifest digest does not match its content".to_owned()),
            });
        }

        Ok(ModuleVerification {
            schema: MODULE_VERIFICATION_SCHEMA.to_owned(),
            module_id: module_id.to_owned(),
            status: ModuleVerificationStatus::Verified,
            expected_sha256: Some(expected),
            actual_sha256: Some(actual),
            checked_at,
            detail: None,
        })
    }

    pub fn lifecycle_events(&self, module_id: &str) -> Result<Vec<ModuleEvent>, ModuleError> {
        validate_module_id(module_id)?;
        let connection = self.open()?;
        if read_record(&connection, module_id)?.is_none() {
            return Err(ModuleError::ModuleNotFound(module_id.to_owned()));
        }
        read_events(&connection, module_id)
    }

    fn transition(
        &self,
        module_id: &str,
        allowed_from: &[ModuleLifecycleState],
        to: ModuleLifecycleState,
        action: &str,
        detail: &str,
    ) -> Result<ModuleRecord, ModuleError> {
        validate_module_id(module_id)?;
        let mut connection = self.open()?;
        let transaction = connection.transaction()?;
        let record = read_record(&transaction, module_id)?
            .ok_or_else(|| ModuleError::ModuleNotFound(module_id.to_owned()))?;

        let from = record.lifecycle_state;
        if !allowed_from.contains(&from) {
            return Err(ModuleError::IllegalTransition {
                from: from.as_str().to_owned(),
                to: to.as_str().to_owned(),
            });
        }

        let timestamp = now_rfc3339()?;
        transaction.execute(
            "UPDATE aegis_modules SET lifecycle_state = ?1, updated_at = ?2 WHERE module_id = ?3",
            params![to.as_str(), timestamp, module_id],
        )?;
        insert_event(
            &transaction,
            module_id,
            action,
            Some(from),
            to,
            detail,
            &timestamp,
        )?;

        let updated = read_record(&transaction, module_id)?
            .ok_or_else(|| ModuleError::ModuleNotFound(module_id.to_owned()))?;
        transaction.commit()?;
        Ok(updated)
    }

    fn open(&self) -> Result<Connection, ModuleError> {
        let connection = open_initialized(
            &self.paths.platform_db(),
            PLATFORM_APPLICATION_ID,
            REGISTRY_COMPONENT,
        )?;
        ensure_tables(&connection)?;
        Ok(connection)
    }
}

fn ensure_tables(connection: &Connection) -> Result<(), ModuleError> {
    connection.execute_batch(
        "CREATE TABLE IF NOT EXISTS aegis_modules (
            module_id TEXT PRIMARY KEY,
            schema TEXT NOT NULL,
            name TEXT NOT NULL,
            version TEXT NOT NULL,
            publisher TEXT NOT NULL,
            lifecycle_state TEXT NOT NULL CHECK(lifecycle_state IN (
                'installed', 'enabled', 'disabled', 'failed', 'uninstalled'
            )),
            capabilities_json TEXT NOT NULL,
            permissions_json TEXT NOT NULL,
            manifest_json TEXT NOT NULL,
            manifest_sha256 TEXT NOT NULL,
            installed_at TEXT NOT NULL,
            updated_at TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS aegis_module_events (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            schema TEXT NOT NULL,
            module_id TEXT NOT NULL REFERENCES aegis_modules(module_id) ON DELETE RESTRICT,
            action TEXT NOT NULL,
            from_state TEXT,
            to_state TEXT NOT NULL,
            detail TEXT NOT NULL,
            event_at TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS aegis_module_events_by_module
            ON aegis_module_events(module_id, id);
        CREATE TRIGGER IF NOT EXISTS aegis_module_events_no_update
            BEFORE UPDATE ON aegis_module_events
            BEGIN
                SELECT RAISE(ABORT, 'module lifecycle events are append-only');
            END;
        CREATE TRIGGER IF NOT EXISTS aegis_module_events_no_delete
            BEFORE DELETE ON aegis_module_events
            BEGIN
                SELECT RAISE(ABORT, 'module lifecycle events are append-only');
            END;",
    )?;
    Ok(())
}

fn check_enable_preconditions(connection: &Connection, module_id: &str) -> Result<(), ModuleError> {
    let (record, manifest_json) = read_record_and_manifest(connection, module_id)?
        .ok_or_else(|| ModuleError::ModuleNotFound(module_id.to_owned()))?;
    let from = record.lifecycle_state;
    if !matches!(
        from,
        ModuleLifecycleState::Installed
            | ModuleLifecycleState::Disabled
            | ModuleLifecycleState::Failed
    ) {
        return Err(ModuleError::IllegalTransition {
            from: from.as_str().to_owned(),
            to: ModuleLifecycleState::Enabled.as_str().to_owned(),
        });
    }
    if record.permissions.is_empty() {
        return Err(ModuleError::NoDeclaredPermissions {
            module_id: module_id.to_owned(),
        });
    }

    let manifest: ModuleManifest = serde_json::from_str(&manifest_json)?;
    manifest.validate_structure()?;
    let actual = manifest.computed_digest()?;
    if manifest.integrity.digest != actual || actual != record.manifest_sha256 {
        return Err(ModuleError::ExpectedDigestMismatch {
            expected: record.manifest_sha256,
            actual,
        });
    }
    ensure_dependencies_available(connection, &manifest)
}

fn check_disable_preconditions(
    connection: &Connection,
    module_id: &str,
) -> Result<(), ModuleError> {
    let record = read_record(connection, module_id)?
        .ok_or_else(|| ModuleError::ModuleNotFound(module_id.to_owned()))?;
    let from = record.lifecycle_state;
    if !matches!(
        from,
        ModuleLifecycleState::Installed | ModuleLifecycleState::Enabled
    ) {
        return Err(ModuleError::IllegalTransition {
            from: from.as_str().to_owned(),
            to: ModuleLifecycleState::Disabled.as_str().to_owned(),
        });
    }
    if let Some(dependent) = first_enabled_dependent(connection, module_id)? {
        return Err(ModuleError::DependencyInUse {
            module_id: module_id.to_owned(),
            dependent,
        });
    }
    Ok(())
}

/// First currently enabled module that declares `module_id` as a dependency.
fn first_enabled_dependent(
    connection: &Connection,
    module_id: &str,
) -> Result<Option<String>, ModuleError> {
    let mut statement = connection.prepare(
        "SELECT module_id, manifest_json FROM aegis_modules
         WHERE lifecycle_state = ?1 AND module_id <> ?2 ORDER BY module_id ASC",
    )?;
    let mut rows = statement.query(params![ModuleLifecycleState::Enabled.as_str(), module_id])?;
    while let Some(row) = rows.next()? {
        let candidate_id: String = row.get(0)?;
        let manifest_json: String = row.get(1)?;
        let manifest: ModuleManifest = serde_json::from_str(&manifest_json)?;
        if manifest
            .dependencies
            .iter()
            .any(|dependency| dependency.module_id == module_id)
        {
            return Ok(Some(candidate_id));
        }
    }
    Ok(None)
}

fn ensure_dependencies_available(
    connection: &Connection,
    manifest: &ModuleManifest,
) -> Result<(), ModuleError> {
    for dependency in &manifest.dependencies {
        if is_reserved_module_id(&dependency.module_id) {
            return Err(ModuleError::Manifest(ManifestError::InvalidDependency(
                dependency.module_id.clone(),
            )));
        }
        let state: Option<String> = connection
            .query_row(
                "SELECT lifecycle_state FROM aegis_modules WHERE module_id = ?1",
                [dependency.module_id.as_str()],
                |row| row.get(0),
            )
            .optional()?;
        match state {
            None => return Err(ModuleError::UnknownDependency(dependency.module_id.clone())),
            Some(raw) => {
                let parsed = ModuleLifecycleState::parse(&raw)
                    .ok_or_else(|| ModuleError::InvalidLifecycleState(raw.clone()))?;
                if !parsed.is_available_dependency() {
                    return Err(ModuleError::DependencyUnavailable {
                        module_id: dependency.module_id.clone(),
                        state: raw,
                    });
                }
                if dependency.has_version_constraint() {
                    let actual: String = connection.query_row(
                        "SELECT version FROM aegis_modules WHERE module_id = ?1",
                        [dependency.module_id.as_str()],
                        |row| row.get(0),
                    )?;
                    if !dependency.version_matches(&actual) {
                        return Err(ModuleError::DependencyVersionMismatch {
                            module_id: dependency.module_id.clone(),
                            requirement: dependency.version_expectation(),
                            actual,
                        });
                    }
                }
            }
        }
    }
    Ok(())
}

fn insert_event(
    connection: &Connection,
    module_id: &str,
    action: &str,
    from_state: Option<ModuleLifecycleState>,
    to_state: ModuleLifecycleState,
    detail: &str,
    event_at: &str,
) -> Result<(), ModuleError> {
    connection.execute(
        "INSERT INTO aegis_module_events(
            schema, module_id, action, from_state, to_state, detail, event_at
         ) VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            MODULE_EVENT_SCHEMA,
            module_id,
            action,
            from_state.map(ModuleLifecycleState::as_str),
            to_state.as_str(),
            detail,
            event_at,
        ],
    )?;
    Ok(())
}

fn read_record(
    connection: &Connection,
    module_id: &str,
) -> Result<Option<ModuleRecord>, ModuleError> {
    let mut statement = connection.prepare(&format!(
        "SELECT {RECORD_COLUMNS} FROM aegis_modules WHERE module_id = ?1"
    ))?;
    let mut rows = statement.query([module_id])?;
    match rows.next()? {
        Some(row) => Ok(Some(build_record(row)?)),
        None => Ok(None),
    }
}

fn read_record_and_manifest(
    connection: &Connection,
    module_id: &str,
) -> Result<Option<(ModuleRecord, String)>, ModuleError> {
    let mut statement = connection.prepare(&format!(
        "SELECT {RECORD_AND_MANIFEST_COLUMNS} FROM aegis_modules WHERE module_id = ?1"
    ))?;
    let mut rows = statement.query([module_id])?;
    match rows.next()? {
        Some(row) => {
            let record = build_record(row)?;
            let manifest_json: String = row.get(11)?;
            Ok(Some((record, manifest_json)))
        }
        None => Ok(None),
    }
}

fn read_events(connection: &Connection, module_id: &str) -> Result<Vec<ModuleEvent>, ModuleError> {
    let mut statement = connection.prepare(
        "SELECT schema, module_id, action, from_state, to_state, detail, event_at
         FROM aegis_module_events WHERE module_id = ?1 ORDER BY id ASC",
    )?;
    let mut rows = statement.query([module_id])?;
    let mut events = Vec::new();
    while let Some(row) = rows.next()? {
        events.push(build_event(row)?);
    }
    Ok(events)
}

fn build_record(row: &rusqlite::Row<'_>) -> Result<ModuleRecord, ModuleError> {
    let state_raw: String = row.get(5)?;
    let lifecycle_state = ModuleLifecycleState::parse(&state_raw)
        .ok_or_else(|| ModuleError::InvalidLifecycleState(state_raw.clone()))?;
    let capabilities_json: String = row.get(6)?;
    let permissions_json: String = row.get(7)?;
    Ok(ModuleRecord {
        schema: row.get(0)?,
        module_id: row.get(1)?,
        name: row.get(2)?,
        version: row.get(3)?,
        publisher: row.get(4)?,
        lifecycle_state,
        capabilities: serde_json::from_str(&capabilities_json)?,
        permissions: serde_json::from_str(&permissions_json)?,
        manifest_sha256: row.get(8)?,
        installed_at: row.get(9)?,
        updated_at: row.get(10)?,
    })
}

fn build_event(row: &rusqlite::Row<'_>) -> Result<ModuleEvent, ModuleError> {
    let from_raw: Option<String> = row.get(3)?;
    let from_state = match from_raw {
        Some(raw) => Some(
            ModuleLifecycleState::parse(&raw)
                .ok_or_else(|| ModuleError::InvalidLifecycleState(raw.clone()))?,
        ),
        None => None,
    };
    let to_raw: String = row.get(4)?;
    let to_state = ModuleLifecycleState::parse(&to_raw)
        .ok_or_else(|| ModuleError::InvalidLifecycleState(to_raw.clone()))?;
    Ok(ModuleEvent {
        schema: row.get(0)?,
        module_id: row.get(1)?,
        action: row.get(2)?,
        from_state,
        to_state,
        detail: row.get(5)?,
        event_at: row.get(6)?,
    })
}
