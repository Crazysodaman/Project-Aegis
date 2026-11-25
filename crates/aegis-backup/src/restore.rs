use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use aegis_contracts::integrity::IntegrityStatus;
use chrono::Utc;
use uuid::Uuid;

use crate::backup::backup;
use crate::error::BackupError;
use crate::manifest::BackupManifest;
use crate::pathutil::{
    absolute_path, comparable_path, copy_hashed, copy_new_checked, copy_replace_checked,
    create_private_dir, ensure_no_redirects, hash_file, is_redirected, set_dir_private,
};
use crate::state::{
    check_markers, detect_running, open_platform_read_only, open_safe_read_only,
    read_core_identity, verify_core_files,
};
use crate::verify::{verify_backup, BackupVerificationStatus};

/// Options controlling restore behavior.
#[derive(Debug, Clone, Default)]
pub struct RestoreOptions {
    /// Permit restoring over a non-empty target after preserving it.
    pub allow_existing: bool,
    /// Where to place the rollback backup of the existing target.
    pub backup_existing_to: Option<PathBuf>,
}

/// Result of a successful restore.
#[derive(Debug, Clone)]
pub struct RestoreReport {
    pub source_backup: PathBuf,
    pub target: PathBuf,
    pub restored_cores: Vec<String>,
    pub preserved_source: Option<PathBuf>,
    pub total_bytes: u64,
    pub file_count: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TargetState {
    Missing,
    Empty,
    NonEmpty,
}

/// Restore a verified backup into a target storage root.
///
/// A non-empty target is refused unless [`RestoreOptions::allow_existing`] is
/// set. In that case the existing target is first captured as a full backup
/// (or a raw rollback copy when it is not a Platform root) before any file is
/// overwritten. Nothing in the target is deleted.
pub fn restore(
    backup_dir: impl AsRef<Path>,
    target_root: impl AsRef<Path>,
    options: RestoreOptions,
) -> Result<RestoreReport, BackupError> {
    let backup_root = absolute_path(backup_dir.as_ref())?;
    let verification = verify_backup(&backup_root)?;
    if verification.status != BackupVerificationStatus::Verified {
        return Err(BackupError::InvalidBackup(format!(
            "verification found {} issue(s); refusing to restore",
            verification.issues.len()
        )));
    }
    let manifest = verification.manifest;

    let target_root = absolute_path(target_root.as_ref())?;
    let backup_comparable = comparable_path(&backup_root);
    let target_comparable = comparable_path(&target_root);
    if target_comparable.starts_with(&backup_comparable) {
        return Err(BackupError::RestoreTargetInsideBackup);
    }
    if backup_comparable.starts_with(&target_comparable) {
        return Err(BackupError::BackupInsideRestoreTarget);
    }

    let state = target_state(&target_root)?;

    detect_running(&backup_root)?;
    if state != TargetState::Missing {
        detect_running(&target_root)?;
    }

    let preserved_source = match state {
        TargetState::Missing | TargetState::Empty => {
            staged_restore(&backup_root, &target_root, &manifest)?;
            None
        }
        TargetState::NonEmpty => {
            if !options.allow_existing {
                return Err(BackupError::RestoreTargetNotEmpty(target_root));
            }
            Some(existing_restore(
                &backup_root,
                &target_root,
                &manifest,
                &options,
            )?)
        }
    };

    let total_bytes = manifest.files.iter().map(|file| file.bytes).sum();
    let file_count = manifest.files.len();
    let restored_cores = manifest
        .cores
        .iter()
        .map(|core| core.core_uuid.clone())
        .collect();

    Ok(RestoreReport {
        source_backup: backup_root,
        target: target_root,
        restored_cores,
        preserved_source,
        total_bytes,
        file_count,
    })
}

fn target_state(target: &Path) -> Result<TargetState, BackupError> {
    match fs::symlink_metadata(target) {
        Ok(metadata) => {
            if is_redirected(&metadata) {
                return Err(BackupError::UnsafePath(format!(
                    "{} is a symlink or reparse point",
                    target.display()
                )));
            }
            if !metadata.is_dir() {
                return Ok(TargetState::NonEmpty);
            }
            if fs::read_dir(target)?.next().is_some() {
                Ok(TargetState::NonEmpty)
            } else {
                Ok(TargetState::Empty)
            }
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(TargetState::Missing),
        Err(error) => Err(error.into()),
    }
}

fn staged_restore(
    backup_root: &Path,
    target: &Path,
    manifest: &BackupManifest,
) -> Result<(), BackupError> {
    let parent = target.parent().ok_or_else(|| {
        BackupError::UnsafePath(format!("{} has no parent directory", target.display()))
    })?;
    fs::create_dir_all(parent)?;
    let staging = parent.join(format!(
        ".aegis-restore-staging-{}",
        Uuid::new_v4().simple()
    ));

    let result = (|| -> Result<(), BackupError> {
        fs::create_dir(&staging)?;
        set_dir_private(&staging)?;
        copy_files_to_staging(backup_root, &staging, manifest)?;
        verify_restored_root(&staging, manifest)?;
        match target_state(target)? {
            TargetState::Empty => fs::remove_dir(target)?,
            TargetState::NonEmpty => {
                return Err(BackupError::RestoreTargetNotEmpty(target.to_path_buf()))
            }
            TargetState::Missing => {}
        }
        fs::rename(&staging, target)?;
        Ok(())
    })();

    if let Err(error) = result {
        let _ = fs::remove_dir_all(&staging);
        return Err(error);
    }
    verify_restored_root(target, manifest)?;
    Ok(())
}

fn copy_files_to_staging(
    backup_root: &Path,
    staging: &Path,
    manifest: &BackupManifest,
) -> Result<(), BackupError> {
    for file in &manifest.files {
        ensure_no_redirects(backup_root, &file.relative_path)?;
        let source = backup_root.join(&file.relative_path);
        let destination = staging.join(&file.relative_path);
        if let Some(parent) = destination.parent() {
            create_private_dir(parent)?;
        }
        copy_new_checked(&source, &destination, &file.sha256, file.bytes)?;
    }
    Ok(())
}

fn existing_restore(
    backup_root: &Path,
    target: &Path,
    manifest: &BackupManifest,
    options: &RestoreOptions,
) -> Result<PathBuf, BackupError> {
    let preserved = select_preserved_path(target, backup_root, options)?;
    ensure_preserved_path_is_safe(target, backup_root, &preserved)?;
    prepare_preserved_path(&preserved)?;

    if !(looks_like_platform_root(target) && backup(target, &preserved).is_ok()) {
        if preserved.exists() {
            let _ = fs::remove_dir_all(&preserved);
        }
        if let Err(error) = raw_copy_tree(target, &preserved) {
            let _ = fs::remove_dir_all(&preserved);
            return Err(error);
        }
    }

    for file in &manifest.files {
        ensure_no_redirects(target, &file.relative_path)?;
        let source = backup_root.join(&file.relative_path);
        let destination = target.join(&file.relative_path);
        if let Some(parent) = destination.parent() {
            create_private_dir(parent)?;
        }
        copy_replace_checked(&source, &destination, &file.sha256, file.bytes)?;
    }
    verify_restored_root(target, manifest)?;
    Ok(preserved)
}

fn select_preserved_path(
    target: &Path,
    backup_root: &Path,
    options: &RestoreOptions,
) -> Result<PathBuf, BackupError> {
    if let Some(path) = &options.backup_existing_to {
        return absolute_path(path);
    }
    let parent = target.parent().ok_or_else(|| {
        BackupError::UnsafePath(format!("{} has no parent directory", target.display()))
    })?;
    if parent == backup_root {
        return Err(BackupError::UnsafePath(
            "rollback backup would be written inside the backup".to_owned(),
        ));
    }
    Ok(parent.join(format!(
        "restore-pre-existing-{}-{}",
        Utc::now().format("%Y%m%dT%H%M%S"),
        Uuid::new_v4().simple()
    )))
}

fn prepare_preserved_path(path: &Path) -> Result<(), BackupError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            if is_redirected(&metadata) {
                return Err(BackupError::UnsafePath(format!(
                    "{} is a symlink or reparse point",
                    path.display()
                )));
            }
            if !metadata.is_dir() {
                return Err(BackupError::DestinationExists(path.to_path_buf()));
            }
            if fs::read_dir(path)?.next().is_some() {
                return Err(BackupError::DestinationExists(path.to_path_buf()));
            }
            fs::remove_dir(path)?;
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    Ok(())
}

fn ensure_preserved_path_is_safe(
    target: &Path,
    backup_root: &Path,
    preserved: &Path,
) -> Result<(), BackupError> {
    let target = comparable_path(target);
    let backup = comparable_path(backup_root);
    let preserved_comparable = comparable_path(preserved);
    if preserved_comparable == target
        || preserved_comparable.starts_with(&target)
        || target.starts_with(&preserved_comparable)
        || preserved_comparable.starts_with(&backup)
        || backup.starts_with(&preserved_comparable)
    {
        return Err(BackupError::UnsafePath(format!(
            "rollback path {} overlaps the restore target or backup",
            preserved.display()
        )));
    }
    Ok(())
}

fn raw_copy_tree(source: &Path, destination: &Path) -> Result<(), BackupError> {
    match fs::symlink_metadata(destination) {
        Ok(metadata) => {
            if is_redirected(&metadata) {
                return Err(BackupError::UnsafePath(format!(
                    "{} is a symlink or reparse point",
                    destination.display()
                )));
            }
            if !metadata.is_dir() {
                return Err(BackupError::DestinationExists(destination.to_path_buf()));
            }
            if fs::read_dir(destination)?.next().is_some() {
                return Err(BackupError::DestinationExists(destination.to_path_buf()));
            }
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            create_private_dir(destination)?;
        }
        Err(error) => return Err(error.into()),
    }

    let mut stack = vec![(source.to_path_buf(), PathBuf::new())];
    while let Some((directory, relative_directory)) = stack.pop() {
        for entry in fs::read_dir(&directory)? {
            let entry = entry?;
            let name = entry.file_name();
            let relative = relative_directory.join(&name);
            let source_path = entry.path();
            let metadata = entry.metadata()?;
            if is_redirected(&metadata) {
                return Err(BackupError::UnsafePath(format!(
                    "{} is a symlink or reparse point",
                    source_path.display()
                )));
            }
            let destination_path = destination.join(&relative);
            if metadata.is_dir() {
                create_private_dir(&destination_path)?;
                stack.push((source_path, relative));
            } else if metadata.is_file() {
                if let Some(parent) = destination_path.parent() {
                    create_private_dir(parent)?;
                }
                copy_hashed(&source_path, &destination_path)?;
            } else {
                return Err(BackupError::UnsupportedFile(source_path));
            }
        }
    }
    Ok(())
}

fn verify_restored_root(root: &Path, manifest: &BackupManifest) -> Result<(), BackupError> {
    check_markers(root)?;
    for core in &manifest.cores {
        let database_file = manifest.file(&core.database_relative_path).ok_or_else(|| {
            BackupError::InvalidBackup(format!("Core {} database is not listed", core.core_uuid))
        })?;
        let manifest_file = manifest.file(&core.manifest_relative_path).ok_or_else(|| {
            BackupError::InvalidBackup(format!(
                "Core {} integrity manifest is not listed",
                core.core_uuid
            ))
        })?;
        let database_path = root.join(&core.database_relative_path);
        let manifest_path = root.join(&core.manifest_relative_path);

        let (database_sha256, database_bytes) = hash_file(&database_path)?;
        if database_sha256 != core.database_sha256 || database_bytes != database_file.bytes {
            return Err(BackupError::IntegrityFailed {
                core_uuid: core.core_uuid.clone(),
                status: IntegrityStatus::DigestMismatch,
                detail: "restored Core database does not match the backup manifest".to_owned(),
            });
        }
        let (manifest_sha256, manifest_bytes) = hash_file(&manifest_path)?;
        if manifest_sha256 != core.manifest_sha256 || manifest_bytes != manifest_file.bytes {
            return Err(BackupError::IntegrityFailed {
                core_uuid: core.core_uuid.clone(),
                status: IntegrityStatus::DigestMismatch,
                detail: "restored Core integrity manifest does not match the backup manifest"
                    .to_owned(),
            });
        }

        let identity = read_core_identity(&core.core_uuid, &database_path)?;
        if identity.core_number != core.core_number {
            return Err(BackupError::IntegrityFailed {
                core_uuid: core.core_uuid.clone(),
                status: IntegrityStatus::OwnershipMismatch,
                detail: "restored Core number does not match the backup manifest".to_owned(),
            });
        }
        verify_core_files(
            &core.core_uuid,
            &database_path,
            &manifest_path,
            Some(&identity),
        )?;
    }
    Ok(())
}

fn looks_like_platform_root(root: &Path) -> bool {
    let platform_db = root.join("platform.db");
    let safe_db = root.join("safe.db");
    if !platform_db.is_file() || !safe_db.is_file() {
        return false;
    }
    open_platform_read_only(&platform_db).is_ok() && open_safe_read_only(&safe_db).is_ok()
}
