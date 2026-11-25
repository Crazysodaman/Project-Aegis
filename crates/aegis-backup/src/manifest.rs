use std::collections::BTreeSet;
use std::path::Path;

use aegis_contracts::core::{validate_core_id, validate_core_number, validate_display_name};
use aegis_contracts::integrity::validate_sha256;
use aegis_contracts::lifecycle::CoreLifecycleState;
use aegis_contracts::roster::validate_storage_path;
use serde::{Deserialize, Serialize};

use crate::error::BackupError;
use crate::pathutil::validate_relative_path;

/// Schema identifier written into every backup manifest.
pub const BACKUP_MANIFEST_SCHEMA: &str = "aegis.backup.manifest.v1";
/// Manifest format revision understood by this crate.
pub const BACKUP_FORMAT_VERSION: u32 = 1;
/// File name of the manifest inside a backup directory.
pub const BACKUP_MANIFEST_FILE: &str = "backup.manifest.json";

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
}

/// One copied file, identified by a relative path from the backup root.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackupFile {
    pub relative_path: String,
    pub sha256: String,
    pub bytes: u64,
    pub kind: BackupFileKind,
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

pub(crate) fn validate_manifest(manifest: &BackupManifest) -> Result<(), BackupError> {
    if manifest.schema != BACKUP_MANIFEST_SCHEMA {
        return Err(BackupError::InvalidBackup(format!(
            "unsupported manifest schema {}",
            manifest.schema
        )));
    }
    if manifest.format_version != BACKUP_FORMAT_VERSION {
        return Err(BackupError::InvalidBackup(format!(
            "unsupported backup format version {}",
            manifest.format_version
        )));
    }
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
    let mut kinds = std::collections::BTreeMap::new();
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
        kinds.insert(file.relative_path.clone(), file.kind);
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
        let uuid = validate_core_id(&core.core_uuid).map_err(|_| {
            BackupError::InvalidBackup(format!("invalid Core UUID {}", core.core_uuid))
        })?;
        let canonical = uuid.hyphenated().to_string();
        if canonical != core.core_uuid {
            return Err(BackupError::InvalidBackup(format!(
                "Core UUID is not canonical: {}",
                core.core_uuid
            )));
        }
        if !seen_cores.insert(core.core_uuid.clone()) {
            return Err(BackupError::InvalidBackup(format!(
                "duplicate Core UUID {}",
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
    }

    for file in &manifest.files {
        let expected = match file.kind {
            BackupFileKind::CoreDatabase => {
                let uuid = file
                    .relative_path
                    .strip_prefix("cores/")
                    .and_then(|rest| rest.strip_suffix("/core.db"));
                uuid.map(|uuid| format!("cores/{uuid}/core.db"))
            }
            BackupFileKind::CoreIntegrityManifest => {
                let uuid = file
                    .relative_path
                    .strip_prefix("cores/")
                    .and_then(|rest| rest.strip_suffix("/core.integrity.json"));
                uuid.map(|uuid| format!("cores/{uuid}/core.integrity.json"))
            }
            BackupFileKind::CoreInitializationMarker => {
                let uuid = file
                    .relative_path
                    .strip_prefix("init/")
                    .and_then(|rest| rest.strip_suffix(".initialization.marker"));
                uuid.map(|uuid| format!("init/{uuid}.initialization.marker"))
            }
            BackupFileKind::PlatformDatabase
            | BackupFileKind::SafeDatabase
            | BackupFileKind::OwnerProof => continue,
        };
        let Some(expected) = expected else {
            return Err(BackupError::InvalidBackup(format!(
                "Core file path is malformed: {}",
                file.relative_path
            )));
        };
        if expected != file.relative_path {
            return Err(BackupError::InvalidBackup(format!(
                "Core file path is not canonical: {}",
                file.relative_path
            )));
        }
        let core_uuid = match file.kind {
            BackupFileKind::CoreDatabase | BackupFileKind::CoreIntegrityManifest => file
                .relative_path
                .strip_prefix("cores/")
                .and_then(|rest| rest.split('/').next()),
            BackupFileKind::CoreInitializationMarker => file
                .relative_path
                .strip_prefix("init/")
                .and_then(|rest| rest.split('.').next()),
            BackupFileKind::PlatformDatabase
            | BackupFileKind::SafeDatabase
            | BackupFileKind::OwnerProof => None,
        };
        let Some(core_uuid) = core_uuid else {
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
