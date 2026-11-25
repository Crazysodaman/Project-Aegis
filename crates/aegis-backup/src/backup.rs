use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use aegis_contracts::integrity::IntegrityStatus;
use aegis_core_integrity::verify_core;
use aegis_core_storage::{CoreDatabase, CoreStorageLayout};
use aegis_storage::StoragePaths;
use chrono::Utc;
use uuid::Uuid;

use crate::error::BackupError;
use crate::manifest::{
    BackupCore, BackupFile, BackupFileKind, BackupManifest, BACKUP_FORMAT_VERSION,
    BACKUP_MANIFEST_FILE, BACKUP_MANIFEST_SCHEMA,
};
use crate::pathutil::{
    absolute_path, comparable_path, copy_hashed, create_private_dir, hash_file, is_redirected,
    require_regular_file, set_dir_private, set_file_private,
};
use crate::state::{
    detect_running, open_safe_read_only, read_core_identity, read_core_registry,
    read_platform_identity, verify_core_files, RegistryCore,
};

/// Result of a successful backup.
#[derive(Debug, Clone)]
pub struct BackupReport {
    pub manifest: BackupManifest,
    pub destination: PathBuf,
    pub total_bytes: u64,
    pub file_count: usize,
}

struct PreparedCore {
    registry: RegistryCore,
    identity: aegis_contracts::core::CoreIdentity,
    schema_revision: u64,
    database_path: PathBuf,
    manifest_path: PathBuf,
    initialization_marker: Option<PathBuf>,
}

/// Back up a Platform storage root into a new directory.
///
/// The source must be a valid Platform root. The destination must not exist and
/// must not overlap the source. Every Core is verified before its bytes are
/// copied and verified again after the copy. The manifest is written last, so
/// a failed backup never looks complete.
pub fn backup(
    root: impl AsRef<Path>,
    destination: impl AsRef<Path>,
) -> Result<BackupReport, BackupError> {
    let source_input = root.as_ref();
    let source_metadata = match fs::symlink_metadata(source_input) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Err(BackupError::InvalidSource(format!(
                "{} does not exist",
                source_input.display()
            )))
        }
        Err(error) => return Err(error.into()),
    };
    if is_redirected(&source_metadata) {
        return Err(BackupError::UnsafePath(format!(
            "{} is a symlink or reparse point",
            source_input.display()
        )));
    }
    if !source_metadata.is_dir() {
        return Err(BackupError::InvalidSource(format!(
            "{} is not a directory",
            source_input.display()
        )));
    }

    let paths = StoragePaths::new(source_input)?;
    let source_root = paths.root().to_path_buf();
    let source_comparable = comparable_path(&source_root);

    let platform_db = paths.platform_db();
    let safe_db = paths.safe_db();
    let owner_proof = paths.owner_proof();

    let (platform_id, platform_callsign) = read_platform_identity(&platform_db)?;
    drop(open_safe_read_only(&safe_db)?);
    require_regular_file(&owner_proof)?;

    let registry = read_core_registry(&platform_db)?;

    let destination = absolute_path(destination.as_ref())?;
    let destination_comparable = comparable_path(&destination);
    if destination_comparable.starts_with(&source_comparable) {
        return Err(BackupError::DestinationInsideSource);
    }
    if source_comparable.starts_with(&destination_comparable) {
        return Err(BackupError::SourceInsideDestination);
    }
    match fs::symlink_metadata(&destination) {
        Ok(_) => return Err(BackupError::DestinationExists(destination)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }

    detect_running(&source_root)?;

    let layout = if registry.is_empty() {
        None
    } else {
        Some(CoreStorageLayout::new(&source_root)?)
    };
    let mut prepared = Vec::with_capacity(registry.len());
    for core in &registry {
        let layout = layout
            .as_ref()
            .ok_or_else(|| BackupError::InvalidSource("Core layout is unavailable".to_owned()))?;
        let snapshot = CoreDatabase::read_snapshot(layout, &core.core_uuid)?;
        if snapshot.identity.core_uuid != core.core_uuid
            || snapshot.identity.core_number != core.core_number
        {
            return Err(BackupError::InvalidSource(format!(
                "Core {} registry identity does not match its database",
                core.core_uuid
            )));
        }
        let manifest_path = layout.integrity_path(&core.core_uuid)?;
        require_regular_file(&manifest_path)?;
        let report = verify_core(
            &core.core_uuid,
            &snapshot.database_path,
            &manifest_path,
            snapshot.metadata.schema_revision,
            Some(&snapshot.identity),
        );
        if report.status != IntegrityStatus::Verified {
            return Err(BackupError::IntegrityFailed {
                core_uuid: core.core_uuid.clone(),
                status: report.status,
                detail: report
                    .detail
                    .unwrap_or_else(|| "integrity verification failed".to_owned()),
            });
        }
        let marker_path = layout.initialization_marker_path(&core.core_uuid)?;
        let initialization_marker = match fs::symlink_metadata(&marker_path) {
            Ok(metadata) => {
                if is_redirected(&metadata) {
                    return Err(BackupError::UnsafePath(format!(
                        "{} is a symlink or reparse point",
                        marker_path.display()
                    )));
                }
                if !metadata.is_file() {
                    return Err(BackupError::UnsupportedFile(marker_path));
                }
                Some(marker_path)
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => None,
            Err(error) => return Err(error.into()),
        };
        prepared.push(PreparedCore {
            registry: core.clone(),
            identity: snapshot.identity,
            schema_revision: snapshot.metadata.schema_revision,
            database_path: snapshot.database_path,
            manifest_path,
            initialization_marker,
        });
    }

    fs::create_dir_all(&destination)?;
    set_dir_private(&destination)?;

    let mut files = Vec::new();
    files.push(copy_backup_file(
        &platform_db,
        &destination,
        "platform.db",
        BackupFileKind::PlatformDatabase,
    )?);
    files.push(copy_backup_file(
        &safe_db,
        &destination,
        "safe.db",
        BackupFileKind::SafeDatabase,
    )?);
    files.push(copy_backup_file(
        &owner_proof,
        &destination,
        "owner.proof",
        BackupFileKind::OwnerProof,
    )?);

    let mut backup_cores = Vec::with_capacity(prepared.len());
    for core in &prepared {
        let core_uuid = &core.registry.core_uuid;
        let database_relative = format!("cores/{core_uuid}/core.db");
        let manifest_relative = format!("cores/{core_uuid}/core.integrity.json");

        let database_file = copy_backup_file(
            &core.database_path,
            &destination,
            &database_relative,
            BackupFileKind::CoreDatabase,
        )?;
        let manifest_file = copy_backup_file(
            &core.manifest_path,
            &destination,
            &manifest_relative,
            BackupFileKind::CoreIntegrityManifest,
        )?;

        let marker_file = match &core.initialization_marker {
            Some(marker) => {
                let relative = format!("init/{core_uuid}.initialization.marker");
                Some(copy_backup_file(
                    marker,
                    &destination,
                    &relative,
                    BackupFileKind::CoreInitializationMarker,
                )?)
            }
            None => None,
        };

        let copied_database = destination.join(&database_relative);
        let copied_manifest = destination.join(&manifest_relative);
        let copied_identity = read_core_identity(core_uuid, &copied_database)?;
        if copied_identity.core_uuid != *core_uuid
            || copied_identity.core_number != core.registry.core_number
            || copied_identity.display_name != core.identity.display_name
        {
            return Err(BackupError::IntegrityFailed {
                core_uuid: core_uuid.clone(),
                status: IntegrityStatus::OwnershipMismatch,
                detail: "copied Core identity does not match the source identity".to_owned(),
            });
        }
        let (integrity_manifest, report) = verify_core_files(
            core_uuid,
            &copied_database,
            &copied_manifest,
            Some(&copied_identity),
        )?;
        if integrity_manifest.database_sha256 != database_file.sha256
            || integrity_manifest.schema_revision != core.schema_revision
        {
            return Err(BackupError::IntegrityFailed {
                core_uuid: core_uuid.clone(),
                status: IntegrityStatus::DigestMismatch,
                detail: "copied database digest does not match its integrity manifest".to_owned(),
            });
        }

        backup_cores.push(BackupCore {
            core_uuid: core_uuid.clone(),
            display_name: core.registry.display_name.clone(),
            core_number: core.registry.core_number,
            lifecycle_state: core.registry.lifecycle_state.as_str().to_owned(),
            database_relative_path: database_relative,
            manifest_relative_path: manifest_relative,
            database_sha256: database_file.sha256.clone(),
            manifest_sha256: manifest_file.sha256.clone(),
            integrity_status: report.status.as_str().to_owned(),
        });
        files.push(database_file);
        files.push(manifest_file);
        if let Some(marker_file) = marker_file {
            files.push(marker_file);
        }
    }

    let total_bytes = files.iter().map(|file| file.bytes).sum();
    let file_count = files.len();
    let manifest = BackupManifest {
        schema: BACKUP_MANIFEST_SCHEMA.to_owned(),
        format_version: BACKUP_FORMAT_VERSION,
        created_at: Utc::now().to_rfc3339(),
        platform_id,
        platform_callsign,
        source_root: source_comparable.to_string_lossy().into_owned(),
        files,
        cores: backup_cores,
    };
    write_manifest_atomic(&destination, &manifest)?;

    Ok(BackupReport {
        manifest,
        destination,
        total_bytes,
        file_count,
    })
}

fn copy_backup_file(
    source: &Path,
    destination_root: &Path,
    relative: &str,
    kind: BackupFileKind,
) -> Result<BackupFile, BackupError> {
    let destination = destination_root.join(relative);
    if let Some(parent) = destination.parent() {
        create_private_dir(parent)?;
    }
    let (source_hash, source_bytes) = hash_file(source)?;
    let (copied_hash, copied_bytes) = copy_hashed(source, &destination)?;
    if source_hash != copied_hash || source_bytes != copied_bytes {
        return Err(BackupError::DigestMismatch {
            path: source.to_path_buf(),
            expected: source_hash,
            actual: copied_hash,
        });
    }
    set_file_private(&destination)?;
    Ok(BackupFile {
        relative_path: relative.to_owned(),
        sha256: copied_hash,
        bytes: copied_bytes,
        kind,
    })
}

fn write_manifest_atomic(destination: &Path, manifest: &BackupManifest) -> Result<(), BackupError> {
    let payload = serde_json::to_vec_pretty(manifest)?;
    let final_path = destination.join(BACKUP_MANIFEST_FILE);
    let temporary = destination.join(format!(
        ".{BACKUP_MANIFEST_FILE}.tmp-{}",
        Uuid::new_v4().simple()
    ));
    let result = (|| -> Result<(), BackupError> {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        file.write_all(&payload)?;
        file.write_all(b"\n")?;
        file.sync_all()?;
        drop(file);
        set_file_private(&temporary)?;
        fs::rename(&temporary, &final_path)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}
