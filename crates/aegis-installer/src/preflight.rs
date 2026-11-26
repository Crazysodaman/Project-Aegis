//! Read-only installation preflight and rollback planning.
//!
//! Everything in this module is inert data. A preflight inspects the filesystem
//! and existing databases with read-only access only: it never creates a
//! directory, writes a file, opens a database read/write, or touches an OS
//! service. A rollback plan is a description of how an operator could undo an
//! action; it is never executed here.
//!
//! The preflight deliberately checks paths, best-effort permissions, recorded
//! versions, and backup destinations so a caller can refuse an upgrade or a
//! destructive uninstall before any mutation happens.

use std::fs;
use std::path::{Component, Path, PathBuf};

use aegis_safe::LocalOwnerProof;
use aegis_storage::PLATFORM_APPLICATION_ID;
use rusqlite::OptionalExtension;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{InstallMode, InstallationRecord, UninstallMode, AEGIS_VERSION};

/// The installer action a preflight or rollback plan describes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PreflightAction {
    Install,
    Upgrade,
    Uninstall,
}

impl PreflightAction {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Install => "install",
            Self::Upgrade => "upgrade",
            Self::Uninstall => "uninstall",
        }
    }
}

/// Outcome of one preflight check.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PreflightStatus {
    /// The condition is satisfied.
    Pass,
    /// The condition is not satisfied but does not block the action.
    Warn,
    /// The condition blocks the action.
    Fail,
}

impl PreflightStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Pass => "pass",
            Self::Warn => "warn",
            Self::Fail => "fail",
        }
    }

    const fn blocks(self) -> bool {
        matches!(self, Self::Fail)
    }
}

/// One named preflight observation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PreflightCheck {
    /// Stable reason code for tests and evidence capture.
    pub code: String,
    pub status: PreflightStatus,
    pub detail: String,
}

impl PreflightCheck {
    pub fn pass(code: impl Into<String>, detail: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            status: PreflightStatus::Pass,
            detail: detail.into(),
        }
    }

    pub fn warn(code: impl Into<String>, detail: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            status: PreflightStatus::Warn,
            detail: detail.into(),
        }
    }

    pub fn fail(code: impl Into<String>, detail: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            status: PreflightStatus::Fail,
            detail: detail.into(),
        }
    }
}

/// The inert result of an install, upgrade, or uninstall preflight.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PreflightReport {
    pub action: PreflightAction,
    pub root: String,
    pub checks: Vec<PreflightCheck>,
    /// Always `false`: a preflight only inspects and never mutates.
    pub mutations_performed: bool,
    /// True when no check failed. Warnings do not block.
    pub ready: bool,
}

impl PreflightReport {
    fn new(action: PreflightAction, root: &Path, checks: Vec<PreflightCheck>) -> Self {
        let ready = !checks.iter().any(|check| check.status.blocks());
        Self {
            action,
            root: root.display().to_string(),
            checks,
            mutations_performed: false,
            ready,
        }
    }

    /// True when the preflight performed no mutation (always true).
    pub fn is_inert(&self) -> bool {
        !self.mutations_performed
    }

    pub fn check(&self, code: &str) -> Option<&PreflightCheck> {
        self.checks.iter().find(|check| check.code == code)
    }

    pub fn failures(&self) -> impl Iterator<Item = &PreflightCheck> {
        self.checks
            .iter()
            .filter(|check| check.status == PreflightStatus::Fail)
    }

    pub fn render(&self) -> String {
        let mut lines = vec![
            format!("action: {}", self.action.as_str()),
            format!("root: {}", self.root),
            format!("mutations-performed: {}", self.mutations_performed),
            format!("ready: {}", self.ready),
        ];
        for check in &self.checks {
            lines.push(format!(
                "[{}] {}: {}",
                check.status.as_str(),
                check.code,
                check.detail
            ));
        }
        lines.join("\n")
    }
}

impl std::fmt::Display for PreflightReport {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.render())
    }
}

/// One reversible step in a rollback plan.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RollbackStep {
    pub order: u32,
    pub code: String,
    pub description: String,
    /// Every documented rollback path preserves owner data.
    pub preserves_data: bool,
}

/// A validated, data-only description of how to undo an action.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RollbackPlan {
    pub action: PreflightAction,
    pub requires_backup: bool,
    /// Always `true`: the plan is never executed by this crate.
    pub dry_run: bool,
    pub steps: Vec<RollbackStep>,
}

/// Structural validation failures for a rollback plan.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum RollbackError {
    #[error("rollback plan has no steps")]
    Empty,
    #[error(
        "rollback order must start at 1 and be contiguous: expected {expected}, found {actual}"
    )]
    NonContiguousOrder { expected: u32, actual: u32 },
    #[error("rollback step {order} ({code}) is not data-preserving")]
    NotDataPreserving { order: u32, code: String },
    #[error("rollback requires a backup but no restore step is documented")]
    MissingBackupStep,
}

impl RollbackPlan {
    /// Check ordering, contiguity, data preservation, and backup coverage.
    pub fn validate(&self) -> Result<(), RollbackError> {
        if self.steps.is_empty() {
            return Err(RollbackError::Empty);
        }
        for (index, step) in self.steps.iter().enumerate() {
            let expected = u32::try_from(index + 1).unwrap_or(u32::MAX);
            if step.order != expected {
                return Err(RollbackError::NonContiguousOrder {
                    expected,
                    actual: step.order,
                });
            }
            if !step.preserves_data {
                return Err(RollbackError::NotDataPreserving {
                    order: step.order,
                    code: step.code.clone(),
                });
            }
        }
        if self.requires_backup && !self.steps.iter().any(|step| step.code == "restore-backup") {
            return Err(RollbackError::MissingBackupStep);
        }
        Ok(())
    }

    pub fn render(&self) -> String {
        let mut lines = vec![
            format!("action: {}", self.action.as_str()),
            format!("requires-backup: {}", self.requires_backup),
            format!("dry-run: {}", self.dry_run),
        ];
        for step in &self.steps {
            lines.push(format!(
                "{}. [{}] {} (preserves-data: {})",
                step.order, step.code, step.description, step.preserves_data
            ));
        }
        lines.join("\n")
    }
}

impl std::fmt::Display for RollbackPlan {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.render())
    }
}

/// Preflight a fresh install or a repeat install. Read-only.
pub fn preflight_install(root: impl AsRef<Path>, mode: InstallMode) -> PreflightReport {
    let root = root.as_ref();
    let mut checks = root_checks(root);
    checks.push(PreflightCheck::pass(
        "install-mode",
        format!("requested {} install", mode.as_str()),
    ));

    match read_record_readonly(root) {
        Ok(None) => checks.push(PreflightCheck::pass(
            "installation",
            "no installation record exists; a fresh install would initialize Platform and Safe",
        )),
        Ok(Some(record)) => {
            checks.push(PreflightCheck::pass(
                "installation",
                format!(
                    "existing installation {} ({}) would be re-used",
                    record.platform_id, record.state
                ),
            ));
            checks.push(version_check(&record.aegis_version));
        }
        Err(error) => checks.push(PreflightCheck::fail("record-readable", error.to_string())),
    }

    checks.push(PreflightCheck::warn(
        "backup-not-required",
        "install does not modify pre-existing owner data, so no backup is required",
    ));
    PreflightReport::new(PreflightAction::Install, root, checks)
}

/// Preflight an upgrade. Read-only: the recorded version, owner proof, and
/// backup destination are inspected without touching any file.
pub fn preflight_upgrade(
    root: impl AsRef<Path>,
    owner_token: &str,
    backup_to: Option<&Path>,
) -> PreflightReport {
    let root = root.as_ref();
    let mut checks = root_checks(root);
    checks.extend(platform_check(root));
    checks.push(owner_check(root, owner_token));

    if service_running(root) {
        checks.push(PreflightCheck::warn(
            "service-stopped",
            "a Platform service appears to be running; stop it before upgrading so the \
             pre-upgrade backup is consistent",
        ));
    } else {
        checks.push(PreflightCheck::pass(
            "service-stopped",
            "no running Platform service was detected",
        ));
    }

    let record = match read_record_readonly(root) {
        Ok(record) => record,
        Err(error) => {
            checks.push(PreflightCheck::fail("record-readable", error.to_string()));
            None
        }
    };

    let from_version = record
        .as_ref()
        .map(|record| record.aegis_version.clone())
        .unwrap_or_else(|| "0.0.0".to_owned());
    let version = version_check(&from_version);
    let needs_backup = version.status == PreflightStatus::Pass;
    checks.push(version);

    if needs_backup {
        match backup_destination(root, backup_to) {
            Some(destination) => checks.extend(backup_checks(root, &destination)),
            None => checks.push(PreflightCheck::fail(
                "backup-destination",
                "no default or explicit backup destination could be derived",
            )),
        }
    } else {
        checks.push(PreflightCheck::warn(
            "backup-not-required",
            "the recorded version is current or newer, so no backup would be taken",
        ));
    }

    PreflightReport::new(PreflightAction::Upgrade, root, checks)
}

/// Preflight an uninstall. Read-only: running-service state and the callsign
/// confirmation are inspected without changing anything.
pub fn preflight_uninstall(
    root: impl AsRef<Path>,
    owner_token: &str,
    mode: UninstallMode,
    confirmation: Option<&str>,
    backup_to: Option<&Path>,
) -> PreflightReport {
    let root = root.as_ref();
    let mut checks = root_checks(root);
    checks.extend(platform_check(root));
    checks.push(owner_check(root, owner_token));

    if service_running(root) {
        checks.push(PreflightCheck::fail(
            "service-stopped",
            "a running Platform service blocks uninstall",
        ));
    } else {
        checks.push(PreflightCheck::pass(
            "service-stopped",
            "no running Platform service was detected",
        ));
    }

    let callsign = match read_callsign_readonly(root) {
        Ok(callsign) => callsign,
        Err(error) => {
            checks.push(PreflightCheck::fail("record-readable", error.to_string()));
            None
        }
    };

    match mode {
        UninstallMode::Preserve => {
            checks.push(PreflightCheck::warn(
                "uninstall-backup",
                "preserve mode deletes nothing, so no backup is required",
            ));
            checks.push(PreflightCheck::pass(
                "uninstall-confirmation",
                "preserve mode does not require a callsign confirmation",
            ));
        }
        UninstallMode::DeleteData => {
            match backup_to {
                Some(destination) => checks.extend(backup_checks(root, destination)),
                None => checks.push(PreflightCheck::fail(
                    "uninstall-backup",
                    "delete mode requires an explicit backup destination",
                )),
            }
            match (&callsign, confirmation.map(str::trim)) {
                (Some(callsign), Some(given)) if callsign == given => {
                    checks.push(PreflightCheck::pass(
                        "uninstall-confirmation",
                        "confirmation matches the Platform callsign",
                    ))
                }
                (Some(_), Some(_)) => checks.push(PreflightCheck::fail(
                    "uninstall-confirmation",
                    "confirmation does not match the Platform callsign",
                )),
                _ => checks.push(PreflightCheck::fail(
                    "uninstall-confirmation",
                    "a callsign confirmation is required to delete data",
                )),
            }
        }
    }

    PreflightReport::new(PreflightAction::Uninstall, root, checks)
}

/// A data-preserving rollback plan for a fresh or repeated install.
pub fn rollback_plan_install() -> RollbackPlan {
    RollbackPlan {
        action: PreflightAction::Install,
        requires_backup: false,
        dry_run: true,
        steps: vec![
            RollbackStep {
                order: 1,
                code: "inspect-prior-install".to_owned(),
                description: "If the root already held an installation, every file is left \
                              unchanged and the rollback is a no-op."
                    .to_owned(),
                preserves_data: true,
            },
            RollbackStep {
                order: 2,
                code: "remove-new-install".to_owned(),
                description: "If this was a fresh install, remove only the files it created \
                              (platform.db, safe.db, owner.proof); no pre-existing owner data is \
                              touched."
                    .to_owned(),
                preserves_data: true,
            },
        ],
    }
}

/// A backup-gated rollback plan for an upgrade.
pub fn rollback_plan_upgrade() -> RollbackPlan {
    RollbackPlan {
        action: PreflightAction::Upgrade,
        requires_backup: true,
        dry_run: true,
        steps: vec![
            RollbackStep {
                order: 1,
                code: "restore-backup".to_owned(),
                description: "Restore the verified pre-upgrade backup taken before any change; \
                              Platform, Safe, owner proof, and every Core database are recovered \
                              byte-for-byte."
                    .to_owned(),
                preserves_data: true,
            },
            RollbackStep {
                order: 2,
                code: "verify-record".to_owned(),
                description: "Confirm the restored installation record reports the pre-upgrade \
                              version and the same Platform identity."
                    .to_owned(),
                preserves_data: true,
            },
        ],
    }
}

/// A rollback plan for uninstall, gated on the selected mode.
pub fn rollback_plan_uninstall(mode: UninstallMode) -> RollbackPlan {
    match mode {
        UninstallMode::Preserve => RollbackPlan {
            action: PreflightAction::Uninstall,
            requires_backup: false,
            dry_run: true,
            steps: vec![RollbackStep {
                order: 1,
                code: "no-file-change".to_owned(),
                description: "Preserve mode deletes nothing; the rollback is a no-op and all \
                              Platform and Core data remains in place."
                    .to_owned(),
                preserves_data: true,
            }],
        },
        UninstallMode::DeleteData => RollbackPlan {
            action: PreflightAction::Uninstall,
            requires_backup: true,
            dry_run: true,
            steps: vec![
                RollbackStep {
                    order: 1,
                    code: "restore-backup".to_owned(),
                    description: "Restore the verified pre-deletion backup to recover Platform, \
                                  Safe, owner proof, and every Core database."
                        .to_owned(),
                    preserves_data: true,
                },
                RollbackStep {
                    order: 2,
                    code: "verify-record".to_owned(),
                    description: "Confirm the restored installation record and Platform identity \
                                  match the pre-deletion state."
                        .to_owned(),
                    preserves_data: true,
                },
            ],
        },
    }
}

fn root_checks(root: &Path) -> Vec<PreflightCheck> {
    let mut checks = Vec::new();
    if root.is_absolute() {
        checks.push(PreflightCheck::pass(
            "root-absolute",
            format!("{} is an absolute path", root.display()),
        ));
    } else {
        checks.push(PreflightCheck::fail(
            "root-absolute",
            format!("{} is not an absolute path", root.display()),
        ));
    }

    if root
        .components()
        .any(|component| matches!(component, Component::ParentDir))
    {
        checks.push(PreflightCheck::fail(
            "root-components",
            format!("{} contains parent-directory components", root.display()),
        ));
    } else {
        checks.push(PreflightCheck::pass(
            "root-components",
            "the root path contains no parent-directory components",
        ));
    }

    match fs::symlink_metadata(root) {
        Ok(metadata) if super::is_redirected(&metadata) => checks.push(PreflightCheck::fail(
            "root-redirect",
            format!("{} is a symlink or reparse point", root.display()),
        )),
        Ok(metadata) if !metadata.is_dir() => checks.push(PreflightCheck::fail(
            "root-redirect",
            format!("{} exists but is not a directory", root.display()),
        )),
        Ok(_) => checks.push(PreflightCheck::pass(
            "root-redirect",
            format!("{} is a real directory", root.display()),
        )),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            checks.push(PreflightCheck::pass(
                "root-redirect",
                format!("{} does not exist yet and would be created", root.display()),
            ))
        }
        Err(error) => checks.push(PreflightCheck::fail("root-redirect", error.to_string())),
    }

    checks.push(writable_check(
        "root-permissions",
        root,
        "the root (or the nearest existing parent) is best-effort writable",
    ));
    checks
}

fn platform_check(root: &Path) -> Vec<PreflightCheck> {
    if root.join("platform.db").is_file() {
        vec![PreflightCheck::pass(
            "platform-present",
            "platform.db exists and can be inspected",
        )]
    } else {
        vec![PreflightCheck::fail(
            "platform-present",
            "platform.db is missing; the Platform is not installed",
        )]
    }
}

fn owner_check(root: &Path, owner_token: &str) -> PreflightCheck {
    let proof = LocalOwnerProof::new(root.join("owner.proof"));
    if !proof.path().is_file() {
        return PreflightCheck::fail("owner-token", "owner proof has not been initialized");
    }
    match proof.verify(owner_token) {
        Ok(true) => PreflightCheck::pass("owner-token", "owner proof accepted"),
        Ok(false) => PreflightCheck::fail("owner-token", "owner authorization failed"),
        Err(error) => PreflightCheck::fail("owner-token", error.to_string()),
    }
}

fn version_check(installed: &str) -> PreflightCheck {
    let installed_version = match super::parse_version(installed) {
        Ok(version) => version,
        Err(error) => {
            return PreflightCheck::fail("version", error.to_string());
        }
    };
    let build = match super::parse_version(AEGIS_VERSION) {
        Ok(version) => version,
        Err(error) => {
            return PreflightCheck::fail("version", error.to_string());
        }
    };
    if installed_version > build {
        PreflightCheck::fail(
            "version",
            format!(
                "installed version {installed} is newer than this build {AEGIS_VERSION}; \
                 downgrade is refused"
            ),
        )
    } else if installed_version == build {
        PreflightCheck::warn(
            "version",
            format!("installed version {installed} is already current"),
        )
    } else {
        PreflightCheck::pass(
            "version",
            format!("upgrade from {installed} to {AEGIS_VERSION} is allowed"),
        )
    }
}

fn backup_destination(root: &Path, backup_to: Option<&Path>) -> Option<PathBuf> {
    match backup_to {
        Some(path) => Some(path.to_path_buf()),
        None => super::default_backup_path(root).ok(),
    }
}

fn backup_checks(root: &Path, destination: &Path) -> Vec<PreflightCheck> {
    let mut checks = Vec::new();
    if destination.as_os_str().is_empty() || !destination.is_absolute() {
        checks.push(PreflightCheck::fail(
            "backup-destination",
            format!(
                "backup destination {} must be an absolute path",
                destination.display()
            ),
        ));
    } else if is_inside(root, destination) {
        checks.push(PreflightCheck::fail(
            "backup-destination",
            format!(
                "backup destination {} must be outside the storage root {}",
                destination.display(),
                root.display()
            ),
        ));
    } else {
        checks.push(PreflightCheck::pass(
            "backup-destination",
            format!(
                "backup destination {} is outside the storage root",
                destination.display()
            ),
        ));
    }
    checks.push(writable_check(
        "backup-permissions",
        destination,
        "the backup destination (or its nearest existing parent) is best-effort writable",
    ));
    checks
}

fn writable_check(code: &str, path: &Path, summary: &str) -> PreflightCheck {
    let Some(existing) = nearest_existing(path) else {
        return PreflightCheck::fail(code, format!("no existing ancestor for {}", path.display()));
    };
    match fs::symlink_metadata(&existing) {
        Ok(metadata) => {
            if super::is_redirected(&metadata) {
                PreflightCheck::fail(
                    code,
                    format!("{} is a symlink or reparse point", existing.display()),
                )
            } else if metadata_writable(&metadata) {
                PreflightCheck::pass(code, summary)
            } else {
                PreflightCheck::fail(
                    code,
                    format!(
                        "{} is marked read-only by filesystem metadata",
                        existing.display()
                    ),
                )
            }
        }
        Err(error) => PreflightCheck::fail(code, error.to_string()),
    }
}

fn nearest_existing(path: &Path) -> Option<PathBuf> {
    let mut current = Some(path);
    while let Some(candidate) = current {
        if candidate.exists() {
            return Some(candidate.to_path_buf());
        }
        current = candidate.parent();
    }
    None
}

#[cfg(unix)]
fn metadata_writable(metadata: &fs::Metadata) -> bool {
    use std::os::unix::fs::PermissionsExt;
    !metadata.permissions().readonly() && (metadata.permissions().mode() & 0o222) != 0
}

#[cfg(not(unix))]
fn metadata_writable(metadata: &fs::Metadata) -> bool {
    !metadata.permissions().readonly()
}

fn is_inside(root: &Path, candidate: &Path) -> bool {
    normalized(candidate).starts_with(normalized(root))
}

fn normalized(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            other => normalized.push(other.as_os_str()),
        }
    }
    normalized
}

fn read_record_readonly(root: &Path) -> Result<Option<InstallationRecord>, crate::InstallerError> {
    let database = root.join("platform.db");
    if !database.is_file() {
        return Ok(None);
    }
    let connection = aegis_storage::open_read_only(&database, PLATFORM_APPLICATION_ID)?;
    if !super::table_exists(&connection, "platform_config")? {
        return Ok(None);
    }
    let Some(schema) = super::read_config(&connection, super::KEY_SCHEMA)? else {
        return Ok(None);
    };
    let required = |key: &str| -> Result<String, crate::InstallerError> {
        super::read_config(&connection, key)?.ok_or(crate::InstallerError::InvalidRecord)
    };
    Ok(Some(InstallationRecord {
        schema,
        aegis_version: required(super::KEY_VERSION)?,
        platform_id: required(super::KEY_PLATFORM_ID)?,
        callsign: read_callsign_connection(&connection)?.unwrap_or_default(),
        state: required(super::KEY_STATE)?,
        mode: required(super::KEY_MODE)?,
        installed_at: required(super::KEY_INSTALLED_AT)?,
        updated_at: required(super::KEY_UPDATED_AT)?,
    }))
}

fn read_callsign_readonly(root: &Path) -> Result<Option<String>, crate::InstallerError> {
    let database = root.join("platform.db");
    if !database.is_file() {
        return Ok(None);
    }
    let connection = aegis_storage::open_read_only(&database, PLATFORM_APPLICATION_ID)?;
    read_callsign_connection(&connection)
}

fn read_callsign_connection(
    connection: &rusqlite::Connection,
) -> Result<Option<String>, crate::InstallerError> {
    if !super::table_exists(connection, "platform_identity")? {
        return Ok(None);
    }
    let callsign: Option<String> = connection
        .query_row(
            "SELECT callsign FROM platform_identity WHERE singleton = 1",
            [],
            |row| row.get(0),
        )
        .optional()?;
    Ok(callsign)
}

fn service_running(root: &Path) -> bool {
    let state = root.join("run").join("service.json");
    let Ok(bytes) = fs::read(&state) else {
        return false;
    };
    serde_json::from_slice::<serde_json::Value>(&bytes)
        .ok()
        .and_then(|value| {
            value
                .get("pid")
                .and_then(serde_json::Value::as_u64)
                .and_then(|pid| u32::try_from(pid).ok())
        })
        .is_some_and(aegis_service::process_is_alive)
}
