use std::collections::BTreeSet;
use std::path::Path;

use aegis_contracts::core::{validate_core_id, validate_core_number, validate_display_name};
use aegis_contracts::integrity::validate_sha256;
use aegis_contracts::lifecycle::CoreLifecycleState;
use aegis_contracts::roster::validate_storage_path;
use serde::{Deserialize, Serialize};

use crate::error::BackupError;
use crate::pathutil::validate_relative_path;
use crate::percore::{sidecar_for_file_name, sidecar_for_relative_path};

/// Legacy (v1) manifest schema. Still readable and restorable.
pub const BACKUP_MANIFEST_SCHEMA_V1: &str = "aegis.backup.manifest.v1";
/// Schema identifier written into every new backup manifest.
pub const BACKUP_MANIFEST_SCHEMA: &str = "aegis.backup.manifest.v2";
/// Legacy (v1) manifest format revision.
pub const BACKUP_FORMAT_VERSION_V1: u32 = 1;
/// Manifest format revision understood and written by this crate.
pub const BACKUP_FORMAT_VERSION: u32 = 2;
/// File name of the manifest inside a backup directory.
pub const BACKUP_MANIFEST_FILE: &str = "backup.manifest.json";

/// The mandatory per-Core database file name.
pub const CORE_DATABASE_FILE: &str = "core.db";

/// Role of one file inside a backup.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BackupFileKind {
    PlatformDatabase,
    SafeDatabase,
    OwnerProof,
    CoreDatabase,
    CoreIntegrityManifest,
    CoreInitializationMarker,
    CoreMemoryDatabase,
    CoreConversationDatabase,
    CoreAssetsDatabase,
    CoreSocialDatabase,
    CoreEmbeddingsDatabase,
    CorePersonalDatabase,
    CorePossessionsDatabase,
    CoreConsentDatabase,
    CoreAssetFile,
}

/// One copied file, identified by a relative path from the backup root.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackupFile {
    pub relative_path: String,
    pub sha256: String,
    pub bytes: u64,
    pub kind: BackupFileKind,
}

/// One per-Core database recorded in the manifest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackupDatabase {
    /// Bare file name, for example `memory.db`.
    pub file_name: String,
    /// Manifest-relative path, for example `cores/<uuid>/memory.db`.
    pub relative_path: String,
    pub kind: BackupFileKind,
    /// Ownership table that must identify this Core.
    pub ownership_table: String,
    pub sha256: String,
    pub bytes: u64,
}

/// Registry and integrity metadata for one backed-up Core.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackupCore {
    pub core_uuid: String,
    pub display_name: String,
    pub core_number: u8,
    pub lifecycle_state: String,
    pub database_relative_path: String,
    pub manifest_relative_path: String,
    pub database_sha256: String,
    pub manifest_sha256: String,
    pub integrity_status: String,
    /// Every backed-up database for this Core. Empty in legacy v1 manifests.
    #[serde(default)]
    pub databases: Vec<BackupDatabase>,
    /// Optional databases that were absent when the backup was taken. Recorded
    /// for completeness; their absence is never fatal.
    #[serde(default)]
    pub missing_optional_databases: Vec<String>,
}

/// Complete description of a backup. The manifest is written last and
/// atomically, so its presence means every listed file was copied.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackupManifest {
    pub schema: String,
    pub format_version: u32,
    pub created_at: String,
    pub platform_id: String,
    pub platform_callsign: String,
    pub source_root: String,
    pub files: Vec<BackupFile>,
    pub cores: Vec<BackupCore>,
}

impl BackupManifest {
    /// Structural validation: schema, version, allowlist, relative paths,
    /// digests, Core metadata, and internal consistency.
    pub fn validate(&self) -> Result<(), BackupError> {
        validate_manifest(self)
    }

    /// Look up a file entry by relative path.
    pub fn file(&self, relative_path: &str) -> Option<&BackupFile> {
        self.files
            .iter()
            .find(|file| file.relative_path == relative_path)
    }
}

/// Normalized manifest format revision (1 or 2), rejecting unknown pairs.
fn manifest_format(manifest: &BackupManifest) -> Result<u32, BackupError> {
    match (manifest.schema.as_str(), manifest.format_version) {
        (BACKUP_MANIFEST_SCHEMA_V1, BACKUP_FORMAT_VERSION_V1) => Ok(BACKUP_FORMAT_VERSION_V1),
        (BACKUP_MANIFEST_SCHEMA, BACKUP_FORMAT_VERSION) => Ok(BACKUP_FORMAT_VERSION),
        _ => Err(BackupError::InvalidBackup(format!(
            "unsupported manifest schema {} / format version {}",
            manifest.schema, manifest.format_version
        ))),
    }
}

pub(crate) fn validate_manifest(manifest: &BackupManifest) -> Result<(), BackupError> {
    let format = manifest_format(manifest)?;

    if !Path::new(&manifest.source_root).is_absolute() {
        return Err(BackupError::InvalidBackup(
            "manifest source_root is not absolute".to_owned(),
        ));
    }
    if manifest.platform_id.trim().is_empty() || manifest.platform_callsign.trim().is_empty() {
        return Err(BackupError::InvalidBackup(
            "manifest Platform identity is empty".to_owned(),
        ));
    }
    if manifest.created_at.trim().is_empty() {
        return Err(BackupError::InvalidBackup(
            "manifest creation timestamp is empty".to_owned(),
        ));
    }

    let mut seen_paths = BTreeSet::new();
    for file in &manifest.files {
        validate_relative_path(&file.relative_path)?;
        validate_sha256(&file.sha256).map_err(|_| {
            BackupError::InvalidBackup(format!("invalid digest for {}", file.relative_path))
        })?;
        if !seen_paths.insert(file.relative_path.clone()) {
            return Err(BackupError::InvalidBackup(format!(
                "duplicate path in manifest: {}",
                file.relative_path
            )));
        }
        validate_file_kind(&file.relative_path, file.kind)?;
    }

    for required in ["platform.db", "safe.db", "owner.proof"] {
        if !seen_paths.contains(required) {
            return Err(BackupError::InvalidBackup(format!(
                "manifest is missing required file {required}"
            )));
        }
    }

    let mut seen_cores = BTreeSet::new();
    for core in &manifest.cores {
        validate_core_entry(manifest, core, &seen_paths)?;
        if !seen_cores.insert(core.core_uuid.clone()) {
            return Err(BackupError::InvalidBackup(format!(
                "duplicate Core UUID {}",
                core.core_uuid
            )));
        }
        if format >= BACKUP_FORMAT_VERSION {
            validate_core_databases(manifest, core)?;
        }
    }

    validate_core_file_association(manifest, &seen_cores)?;

    Ok(())
}

fn validate_core_entry(
    manifest: &BackupManifest,
    core: &BackupCore,
    seen_paths: &BTreeSet<String>,
) -> Result<(), BackupError> {
    let uuid = validate_core_id(&core.core_uuid)
        .map_err(|_| BackupError::InvalidBackup(format!("invalid Core UUID {}", core.core_uuid)))?;
    let canonical = uuid.hyphenated().to_string();
    if canonical != core.core_uuid {
        return Err(BackupError::InvalidBackup(format!(
            "Core UUID is not canonical: {}",
            core.core_uuid
        )));
    }
    validate_core_number(core.core_number).map_err(|_| {
        BackupError::InvalidBackup(format!("invalid Core number for {}", core.core_uuid))
    })?;
    validate_display_name(&core.display_name).map_err(|_| {
        BackupError::InvalidBackup(format!("invalid display name for {}", core.core_uuid))
    })?;
    if CoreLifecycleState::parse(&core.lifecycle_state).is_none() {
        return Err(BackupError::InvalidBackup(format!(
            "invalid lifecycle state for {}",
            core.core_uuid
        )));
    }
    validate_storage_path(&core.database_relative_path).map_err(|_| {
        BackupError::InvalidBackup(format!("invalid Core database path for {}", core.core_uuid))
    })?;
    let expected_database = format!("cores/{}/core.db", core.core_uuid);
    let expected_manifest = format!("cores/{}/core.integrity.json", core.core_uuid);
    if core.database_relative_path != expected_database
        || core.manifest_relative_path != expected_manifest
    {
        return Err(BackupError::InvalidBackup(format!(
            "Core paths do not match Core UUID {}",
            core.core_uuid
        )));
    }
    if !seen_paths.contains(&core.database_relative_path)
        || !seen_paths.contains(&core.manifest_relative_path)
    {
        return Err(BackupError::InvalidBackup(format!(
            "Core files are not listed for {}",
            core.core_uuid
        )));
    }

    let database = manifest.file(&core.database_relative_path).ok_or_else(|| {
        BackupError::InvalidBackup(format!(
            "Core database is not listed for {}",
            core.core_uuid
        ))
    })?;
    let integrity = manifest.file(&core.manifest_relative_path).ok_or_else(|| {
        BackupError::InvalidBackup(format!(
            "Core integrity manifest is not listed for {}",
            core.core_uuid
        ))
    })?;
    if database.kind != BackupFileKind::CoreDatabase
        || integrity.kind != BackupFileKind::CoreIntegrityManifest
    {
        return Err(BackupError::InvalidBackup(format!(
            "Core file kind mismatch for {}",
            core.core_uuid
        )));
    }
    if database.sha256 != core.database_sha256 || integrity.sha256 != core.manifest_sha256 {
        return Err(BackupError::InvalidBackup(format!(
            "Core digest mismatch for {}",
            core.core_uuid
        )));
    }
    if core.integrity_status != "verified" {
        return Err(BackupError::InvalidBackup(format!(
            "Core {} was not verified when backed up",
            core.core_uuid
        )));
    }
    Ok(())
}

fn validate_core_databases(
    manifest: &BackupManifest,
    core: &BackupCore,
) -> Result<(), BackupError> {
    let mut seen = BTreeSet::new();
    let mut has_core_database = false;
    for database in &core.databases {
        if !seen.insert(database.file_name.clone()) {
            return Err(BackupError::InvalidBackup(format!(
                "duplicate per-Core database {} for {}",
                database.file_name, core.core_uuid
            )));
        }
        validate_relative_path(&database.relative_path)?;
        let expected_relative = format!("cores/{}/{}", core.core_uuid, database.file_name);
        if database.relative_path != expected_relative {
            return Err(BackupError::InvalidBackup(format!(
                "per-Core database path is not canonical for {}",
                core.core_uuid
            )));
        }

        if database.file_name == CORE_DATABASE_FILE {
            has_core_database = true;
            if database.kind != BackupFileKind::CoreDatabase {
                return Err(BackupError::InvalidBackup(format!(
                    "core.db has the wrong kind for {}",
                    core.core_uuid
                )));
            }
            if database.relative_path != core.database_relative_path
                || database.sha256 != core.database_sha256
            {
                return Err(BackupError::InvalidBackup(format!(
                    "core.db metadata does not match {}",
                    core.core_uuid
                )));
            }
        } else if let Some(sidecar) = sidecar_for_file_name(&database.file_name) {
            if database.kind != sidecar.kind || database.ownership_table != sidecar.ownership_table
            {
                return Err(BackupError::InvalidBackup(format!(
                    "per-Core database metadata does not match {}",
                    database.file_name
                )));
            }
        } else {
            return Err(BackupError::InvalidBackup(format!(
                "unknown per-Core database {} for {}",
                database.file_name, core.core_uuid
            )));
        }

        let file = manifest.file(&database.relative_path).ok_or_else(|| {
            BackupError::InvalidBackup(format!(
                "per-Core database {} is not listed as a file",
                database.file_name
            ))
        })?;
        if file.kind != database.kind
            || file.sha256 != database.sha256
            || file.bytes != database.bytes
        {
            return Err(BackupError::InvalidBackup(format!(
                "per-Core database {} digest does not match its file",
                database.file_name
            )));
        }
    }

    if !has_core_database {
        return Err(BackupError::InvalidBackup(format!(
            "Core {} is missing its mandatory core.db entry",
            core.core_uuid
        )));
    }

    let mut seen_missing = BTreeSet::new();
    for file_name in &core.missing_optional_databases {
        if file_name == CORE_DATABASE_FILE {
            return Err(BackupError::InvalidBackup(format!(
                "core.db cannot be reported missing for {}",
                core.core_uuid
            )));
        }
        if sidecar_for_file_name(file_name).is_none() {
            return Err(BackupError::InvalidBackup(format!(
                "unknown optional database {file_name} for {}",
                core.core_uuid
            )));
        }
        if !seen_missing.insert(file_name.clone()) {
            return Err(BackupError::InvalidBackup(format!(
                "duplicate missing database {file_name} for {}",
                core.core_uuid
            )));
        }
        if seen.contains(file_name) {
            return Err(BackupError::InvalidBackup(format!(
                "database {file_name} is both present and missing for {}",
                core.core_uuid
            )));
        }
    }

    // Every per-Core database file in the manifest must be declared in the
    // database list, so the two views cannot drift.
    for file in &manifest.files {
        if core_uuid_for_file(file) != Some(core.core_uuid.as_str()) {
            continue;
        }
        if !is_core_database_kind(file.kind) {
            continue;
        }
        let Some(file_name) = file.relative_path.rsplit('/').next() else {
            continue;
        };
        if !seen.contains(file_name) {
            return Err(BackupError::InvalidBackup(format!(
                "per-Core database file {} is not declared for {}",
                file.relative_path, core.core_uuid
            )));
        }
    }

    Ok(())
}

fn validate_core_file_association(
    manifest: &BackupManifest,
    seen_cores: &BTreeSet<String>,
) -> Result<(), BackupError> {
    for file in &manifest.files {
        if !is_core_file_kind(file.kind) {
            continue;
        }
        let Some(core_uuid) = core_uuid_for_file(file) else {
            return Err(BackupError::InvalidBackup(format!(
                "Core file path is malformed: {}",
                file.relative_path
            )));
        };
        if !seen_cores.contains(core_uuid) {
            return Err(BackupError::InvalidBackup(format!(
                "Core file is not associated with a listed Core: {}",
                file.relative_path
            )));
        }
    }
    Ok(())
}

fn is_core_database_kind(kind: BackupFileKind) -> bool {
    matches!(
        kind,
        BackupFileKind::CoreDatabase
            | BackupFileKind::CoreMemoryDatabase
            | BackupFileKind::CoreConversationDatabase
            | BackupFileKind::CoreAssetsDatabase
            | BackupFileKind::CoreSocialDatabase
            | BackupFileKind::CoreEmbeddingsDatabase
            | BackupFileKind::CorePersonalDatabase
            | BackupFileKind::CorePossessionsDatabase
            | BackupFileKind::CoreConsentDatabase
    )
}

fn is_core_file_kind(kind: BackupFileKind) -> bool {
    is_core_database_kind(kind)
        || matches!(
            kind,
            BackupFileKind::CoreIntegrityManifest
                | BackupFileKind::CoreInitializationMarker
                | BackupFileKind::CoreAssetFile
        )
}

fn core_uuid_for_file(file: &BackupFile) -> Option<&str> {
    match file.kind {
        BackupFileKind::CoreInitializationMarker => file
            .relative_path
            .strip_prefix("init/")
            .and_then(|rest| rest.split('.').next()),
        kind if is_core_file_kind(kind) => file
            .relative_path
            .strip_prefix("cores/")
            .and_then(|rest| rest.split('/').next()),
        _ => None,
    }
}

fn validate_file_kind(relative: &str, kind: BackupFileKind) -> Result<(), BackupError> {
    let expected = if relative == "platform.db" {
        BackupFileKind::PlatformDatabase
    } else if relative == "safe.db" {
        BackupFileKind::SafeDatabase
    } else if relative == "owner.proof" {
        BackupFileKind::OwnerProof
    } else if relative.starts_with("cores/") && relative.ends_with("/core.db") {
        BackupFileKind::CoreDatabase
    } else if relative.starts_with("cores/") && relative.ends_with("/core.integrity.json") {
        BackupFileKind::CoreIntegrityManifest
    } else if relative.starts_with("init/") && relative.ends_with(".initialization.marker") {
        BackupFileKind::CoreInitializationMarker
    } else if relative.starts_with("cores/") && relative.contains("/assets/") {
        BackupFileKind::CoreAssetFile
    } else if let Some(sidecar) = sidecar_for_relative_path(relative) {
        sidecar.kind
    } else {
        return Err(BackupError::InvalidBackup(format!(
            "file is outside the backup allowlist: {relative}"
        )));
    };

    if expected != kind {
        return Err(BackupError::InvalidBackup(format!(
            "file kind does not match path: {relative}"
        )));
    }
    Ok(())
}
