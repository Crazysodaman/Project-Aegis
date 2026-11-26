use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use aegis_contracts::integrity::IntegrityStatus;
use aegis_core_integrity::{
    compute_identity_sha256, seal_manifest_with_metadata, verify_core, IntegrityManifestStore,
};
use aegis_core_storage::{CoreDatabase, CoreStorageLayout};
use aegis_storage::StoragePaths;
use chrono::Utc;
use rusqlite::{Connection, OpenFlags};
use uuid::Uuid;

use crate::assets::{collect_asset_files, AssetFile};
use crate::error::BackupError;
use crate::manifest::{
    BackupCore, BackupDatabase, BackupFile, BackupFileKind, BackupManifest, BACKUP_FORMAT_VERSION,
    BACKUP_MANIFEST_FILE, BACKUP_MANIFEST_SCHEMA, CORE_DATABASE_FILE,
};
use crate::pathutil::{
    absolute_path, comparable_path, copy_hashed, create_private_dir, hash_file, is_redirected,
    require_regular_file, set_dir_private, set_file_private,
};
use crate::percore::{
    verify_core_database_ownership, verify_database_ownership, SidecarDatabase, SIDECAR_DATABASES,
};
use crate::snapshot::{sidecar_path, vacuum_into};
use crate::state::{
    detect_running, open_safe_read_only, read_core_identity, read_core_instance_uuid,
    read_core_registry, read_platform_identity, verify_core_files, RegistryCore,
};

/// Result of a successful backup.
#[derive(Debug, Clone)]
pub struct BackupReport {
    pub manifest: BackupManifest,
    pub destination: PathBuf,
    pub total_bytes: u64,
    pub file_count: usize,
    /// Optional per-Core databases that did not exist and were therefore not
    /// copied, expressed as manifest-relative paths.
    pub missing_databases: Vec<String>,
}

struct PreparedCore {
    registry: RegistryCore,
    identity: aegis_contracts::core::CoreIdentity,
    schema_revision: u64,
    initialization_mode: String,
    database_path: PathBuf,
    initialization_marker: Option<PathBuf>,
    database_instance_uuid: Option<String>,
    sidecar_databases: Vec<(&'static SidecarDatabase, PathBuf)>,
    missing_databases: Vec<String>,
    asset_files: Vec<AssetFile>,
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
        if sidecar_path(&snapshot.database_path).is_some() {
            verify_live_core_source(core, &snapshot, &manifest_path)?;
        } else {
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

        let database_instance_uuid = read_core_instance_uuid(&snapshot.database_path)?;
        let core_dir = layout.core_dir(&core.core_uuid)?;
        let mut sidecar_databases = Vec::new();
        let mut missing_databases = Vec::new();
        for spec in SIDECAR_DATABASES {
            let path = core_dir.join(spec.file_name);
            match fs::symlink_metadata(&path) {
                Ok(metadata) => {
                    if is_redirected(&metadata) {
                        return Err(BackupError::UnsafePath(format!(
                            "{} is a symlink or reparse point",
                            path.display()
                        )));
                    }
                    if !metadata.is_file() {
                        return Err(BackupError::UnsupportedFile(path));
                    }
                    verify_database_ownership(&core.core_uuid, &path, spec.ownership_table)?;
                    sidecar_databases.push((spec, path));
                }
                Err(error) if error.kind() == io::ErrorKind::NotFound => {
                    missing_databases.push(spec.file_name.to_owned());
                }
                Err(error) => return Err(error.into()),
            }
        }
        let asset_files = collect_asset_files(&core_dir, &core.core_uuid)?;

        prepared.push(PreparedCore {
            registry: core.clone(),
            identity: snapshot.identity,
            schema_revision: snapshot.metadata.schema_revision,
            initialization_mode: snapshot.metadata.initialization_mode,
            database_path: snapshot.database_path,
            initialization_marker,
            database_instance_uuid,
            sidecar_databases,
            missing_databases,
            asset_files,
        });
    }

    fs::create_dir_all(&destination)?;
    set_dir_private(&destination)?;

    let mut files = Vec::new();
    files.push(snapshot_database_file(
        &platform_db,
        &destination,
        "platform.db",
        BackupFileKind::PlatformDatabase,
    )?);
    files.push(snapshot_database_file(
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
    let mut missing_databases = Vec::new();
    for core in &prepared {
        let core_uuid = &core.registry.core_uuid;
        let database_relative = format!("cores/{core_uuid}/core.db");
        let manifest_relative = format!("cores/{core_uuid}/core.integrity.json");

        let database_file = snapshot_database_file(
            &core.database_path,
            &destination,
            &database_relative,
            BackupFileKind::CoreDatabase,
        )?;

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
        verify_core_database_ownership(
            core_uuid,
            &copied_database,
            core.database_instance_uuid.as_deref(),
        )?;
        let copied_instance = read_core_instance_uuid(&copied_database)?;
        if copied_instance != core.database_instance_uuid {
            return Err(BackupError::IntegrityFailed {
                core_uuid: core_uuid.clone(),
                status: IntegrityStatus::OwnershipMismatch,
                detail: "copied database instance UUID does not match the source".to_owned(),
            });
        }

        let manifest_file = seal_snapshot_manifest(core, &destination, &manifest_relative)?;

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
        if let (Some(source_instance), Some(manifest_instance)) =
            (&copied_instance, &integrity_manifest.database_instance_uuid)
        {
            if source_instance != manifest_instance {
                return Err(BackupError::IntegrityFailed {
                    core_uuid: core_uuid.clone(),
                    status: IntegrityStatus::OwnershipMismatch,
                    detail: "database instance UUID does not match the integrity manifest"
                        .to_owned(),
                });
            }
        }

        let mut databases = vec![BackupDatabase {
            file_name: CORE_DATABASE_FILE.to_owned(),
            relative_path: database_relative.clone(),
            kind: BackupFileKind::CoreDatabase,
            ownership_table: aegis_contracts::core::CORE_IDENTITY_TABLE.to_owned(),
            sha256: database_file.sha256.clone(),
            bytes: database_file.bytes,
        }];

        let database_sha256 = database_file.sha256.clone();
        let manifest_sha256 = manifest_file.sha256.clone();
        files.push(database_file);
        files.push(manifest_file);

        for (spec, path) in &core.sidecar_databases {
            let relative = format!("cores/{core_uuid}/{}", spec.file_name);
            let file = snapshot_database_file(path, &destination, &relative, spec.kind)?;
            verify_database_ownership(
                core_uuid,
                &destination.join(&relative),
                spec.ownership_table,
            )?;
            databases.push(BackupDatabase {
                file_name: spec.file_name.to_owned(),
                relative_path: relative,
                kind: spec.kind,
                ownership_table: spec.ownership_table.to_owned(),
                sha256: file.sha256.clone(),
                bytes: file.bytes,
            });
            files.push(file);
        }

        for asset in &core.asset_files {
            let file = copy_backup_file(
                &asset.absolute,
                &destination,
                &asset.relative_path,
                BackupFileKind::CoreAssetFile,
            )?;
            files.push(file);
        }

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
        if let Some(marker_file) = marker_file {
            files.push(marker_file);
        }

        for name in &core.missing_databases {
            missing_databases.push(format!("cores/{core_uuid}/{name}"));
        }

        backup_cores.push(BackupCore {
            core_uuid: core_uuid.clone(),
            display_name: core.registry.display_name.clone(),
            core_number: core.registry.core_number,
            lifecycle_state: core.registry.lifecycle_state.as_str().to_owned(),
            database_relative_path: database_relative,
            manifest_relative_path: manifest_relative,
            database_sha256,
            manifest_sha256,
            integrity_status: report.status.as_str().to_owned(),
            databases,
            missing_optional_databases: core.missing_databases.clone(),
        });
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
        missing_databases,
    })
}

fn verify_live_core_source(
    core: &RegistryCore,
    snapshot: &aegis_core_storage::CoreStorageSnapshot,
    manifest_path: &Path,
) -> Result<(), BackupError> {
    let connection =
        Connection::open_with_flags(&snapshot.database_path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let check: String = connection.query_row("PRAGMA integrity_check", [], |row| row.get(0))?;
    if check != "ok" {
        return Err(BackupError::IntegrityFailed {
            core_uuid: core.core_uuid.clone(),
            status: IntegrityStatus::SqliteCorrupt,
            detail: format!("live SQLite integrity_check reported: {check}"),
        });
    }
    drop(connection);

    let manifest = IntegrityManifestStore::new(manifest_path)
        .read()?
        .ok_or_else(|| BackupError::IntegrityFailed {
            core_uuid: core.core_uuid.clone(),
            status: IntegrityStatus::MissingManifest,
            detail: format!(
                "integrity manifest is missing at {}",
                manifest_path.display()
            ),
        })?;
    if manifest.core_uuid != core.core_uuid {
        return Err(BackupError::IntegrityFailed {
            core_uuid: core.core_uuid.clone(),
            status: IntegrityStatus::OwnershipMismatch,
            detail: "integrity manifest belongs to another Core".to_owned(),
        });
    }
    let identity_sha256 = compute_identity_sha256(&snapshot.identity)?;
    if manifest.identity_sha256 != identity_sha256 {
        return Err(BackupError::IntegrityFailed {
            core_uuid: core.core_uuid.clone(),
            status: IntegrityStatus::OwnershipMismatch,
            detail: "integrity manifest identity does not match the live Core identity".to_owned(),
        });
    }

    verify_core_database_ownership(
        &core.core_uuid,
        &snapshot.database_path,
        Some(&snapshot.metadata.database_instance_uuid),
    )
}

fn snapshot_database_file(
    source: &Path,
    destination_root: &Path,
    relative: &str,
    kind: BackupFileKind,
) -> Result<BackupFile, BackupError> {
    let destination = destination_root.join(relative);
    let parent = destination.parent().ok_or_else(|| {
        BackupError::UnsafePath(format!("{} has no parent directory", destination.display()))
    })?;
    create_private_dir(parent)?;
    let file_name = destination
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| {
            BackupError::UnsafePath(format!("{} has no valid file name", destination.display()))
        })?;
    let temporary = parent.join(format!(
        ".{file_name}.aegis-backup-{}",
        Uuid::new_v4().simple()
    ));

    let result = (|| -> Result<(String, u64), BackupError> {
        match vacuum_into(source, &temporary) {
            Ok(result) => Ok(result),
            Err(snapshot_error) => {
                let _ = fs::remove_file(&temporary);
                if let Some(sidecar) = sidecar_path(source) {
                    return Err(BackupError::InvalidSource(format!(
                        "live SQLite database {} has sidecar {} and could not be snapshotted consistently: {snapshot_error}",
                        source.display(),
                        sidecar.display()
                    )));
                }
                copy_hashed(source, &temporary)
            }
        }
    })();

    let (digest, bytes) = match result {
        Ok(result) => result,
        Err(error) => {
            let _ = fs::remove_file(&temporary);
            return Err(error);
        }
    };

    let finish = (|| -> Result<(), BackupError> {
        set_file_private(&temporary)?;
        fs::rename(&temporary, &destination)?;
        Ok(())
    })();
    if let Err(error) = finish {
        let _ = fs::remove_file(&temporary);
        return Err(error);
    }

    Ok(BackupFile {
        relative_path: relative.to_owned(),
        sha256: digest,
        bytes,
        kind,
    })
}

fn seal_snapshot_manifest(
    core: &PreparedCore,
    destination_root: &Path,
    manifest_relative: &str,
) -> Result<BackupFile, BackupError> {
    let core_uuid = &core.registry.core_uuid;
    let database_relative = format!("cores/{core_uuid}/{CORE_DATABASE_FILE}");
    let database_path = destination_root.join(&database_relative);
    let manifest_path = destination_root.join(manifest_relative);
    let parent = manifest_path.parent().ok_or_else(|| {
        BackupError::UnsafePath(format!(
            "{} has no parent directory",
            manifest_path.display()
        ))
    })?;
    create_private_dir(parent)?;

    seal_manifest_with_metadata(
        core_uuid,
        &database_path,
        &manifest_path,
        core.schema_revision,
        &core.identity,
        core.database_instance_uuid.as_deref(),
        Some(&core.initialization_mode),
        Some(core.registry.core_number),
    )?;
    set_file_private(&manifest_path)?;

    let (sha256, bytes) = hash_file(&manifest_path)?;
    Ok(BackupFile {
        relative_path: manifest_relative.to_owned(),
        sha256,
        bytes,
        kind: BackupFileKind::CoreIntegrityManifest,
    })
}

fn copy_backup_file(
    source: &Path,
    destination_root: &Path,
    relative: &str,
    kind: BackupFileKind,
) -> Result<BackupFile, BackupError> {
    let destination = destination_root.join(relative);
    let parent = destination.parent().ok_or_else(|| {
        BackupError::UnsafePath(format!("{} has no parent directory", destination.display()))
    })?;
    create_private_dir(parent)?;
    let file_name = destination
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| {
            BackupError::UnsafePath(format!("{} has no valid file name", destination.display()))
        })?;
    let temporary = parent.join(format!(
        ".{file_name}.aegis-backup-{}",
        Uuid::new_v4().simple()
    ));

    let (source_hash, source_bytes) = hash_file(source)?;
    let result = (|| -> Result<(String, u64), BackupError> {
        let (copied_hash, copied_bytes) = copy_hashed(source, &temporary)?;
        if source_hash != copied_hash || source_bytes != copied_bytes {
            return Err(BackupError::DigestMismatch {
                path: source.to_path_buf(),
                expected: source_hash.clone(),
                actual: copied_hash,
            });
        }
        set_file_private(&temporary)?;
        fs::rename(&temporary, &destination)?;
        Ok((copied_hash, copied_bytes))
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    let (copied_hash, copied_bytes) = result?;
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
