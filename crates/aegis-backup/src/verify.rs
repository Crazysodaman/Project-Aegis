use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::error::BackupError;
use crate::manifest::{
    validate_manifest, BackupFile, BackupManifest, BACKUP_MANIFEST_FILE, CORE_DATABASE_FILE,
};
use crate::pathutil::{
    absolute_path, ensure_no_redirects, hash_file, is_redirected, validate_relative_path,
};
use crate::percore::verify_database_ownership;
use crate::state::{read_core_identity, verify_core_files};

/// Overall result of verifying a backup directory.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackupVerificationStatus {
    Verified,
    Invalid,
}

/// Category of a verification issue.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VerificationIssueKind {
    Missing,
    Extra,
    Tampered,
    SizeMismatch,
    UnsafePath,
    Unsupported,
    Integrity,
}

/// One concrete problem found while verifying a backup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerificationIssue {
    pub kind: VerificationIssueKind,
    pub relative_path: Option<String>,
    pub detail: String,
}

impl VerificationIssue {
    fn new(
        kind: VerificationIssueKind,
        relative_path: Option<String>,
        detail: impl Into<String>,
    ) -> Self {
        Self {
            kind,
            relative_path,
            detail: detail.into(),
        }
    }
}

/// Result of verifying a backup. Verification never repairs anything.
#[derive(Debug, Clone)]
pub struct BackupVerification {
    pub destination: PathBuf,
    pub status: BackupVerificationStatus,
    pub manifest: BackupManifest,
    pub issues: Vec<VerificationIssue>,
    pub file_count: usize,
    pub total_bytes: u64,
}

/// Verify every manifest entry, detect extra files, and re-run Core integrity
/// checks against the copied bytes.
pub fn verify_backup(destination: impl AsRef<Path>) -> Result<BackupVerification, BackupError> {
    let destination = absolute_path(destination.as_ref())?;
    let metadata = match fs::symlink_metadata(&destination) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Err(BackupError::InvalidBackup(format!(
                "{} does not exist",
                destination.display()
            )))
        }
        Err(error) => return Err(error.into()),
    };
    if is_redirected(&metadata) {
        return Err(BackupError::UnsafePath(format!(
            "{} is a symlink or reparse point",
            destination.display()
        )));
    }
    if !metadata.is_dir() {
        return Err(BackupError::InvalidBackup(format!(
            "{} is not a directory",
            destination.display()
        )));
    }

    let manifest = read_manifest(&destination)?;
    let expected: BTreeMap<&str, &BackupFile> = manifest
        .files
        .iter()
        .map(|file| (file.relative_path.as_str(), file))
        .collect();
    let mut issues = Vec::new();
    let mut healthy = BTreeSet::new();

    for file in &manifest.files {
        let relative = file.relative_path.clone();
        if let Err(error) = validate_relative_path(&relative) {
            issues.push(VerificationIssue::new(
                VerificationIssueKind::UnsafePath,
                Some(relative),
                error.to_string(),
            ));
            continue;
        }
        if let Err(error) = ensure_no_redirects(&destination, &relative) {
            issues.push(VerificationIssue::new(
                VerificationIssueKind::UnsafePath,
                Some(relative),
                error.to_string(),
            ));
            continue;
        }
        let path = destination.join(&relative);
        let metadata = match fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                issues.push(VerificationIssue::new(
                    VerificationIssueKind::Missing,
                    Some(relative),
                    format!("{} is missing", path.display()),
                ));
                continue;
            }
            Err(error) => return Err(error.into()),
        };
        if is_redirected(&metadata) {
            issues.push(VerificationIssue::new(
                VerificationIssueKind::UnsafePath,
                Some(relative),
                format!("{} is a symlink or reparse point", path.display()),
            ));
            continue;
        }
        if !metadata.is_file() {
            issues.push(VerificationIssue::new(
                VerificationIssueKind::Unsupported,
                Some(relative),
                format!("{} is not a regular file", path.display()),
            ));
            continue;
        }
        let (actual_sha256, actual_bytes) = match hash_file(&path) {
            Ok(result) => result,
            Err(error) => {
                issues.push(VerificationIssue::new(
                    VerificationIssueKind::Tampered,
                    Some(relative),
                    error.to_string(),
                ));
                continue;
            }
        };
        let mut healthy_file = true;
        if actual_bytes != file.bytes {
            healthy_file = false;
            issues.push(VerificationIssue::new(
                VerificationIssueKind::SizeMismatch,
                Some(relative.clone()),
                format!(
                    "{} is {actual_bytes} bytes, manifest records {}",
                    path.display(),
                    file.bytes
                ),
            ));
        }
        if actual_sha256 != file.sha256 {
            healthy_file = false;
            issues.push(VerificationIssue::new(
                VerificationIssueKind::Tampered,
                Some(relative.clone()),
                format!("{} does not match the manifest digest", path.display()),
            ));
        }
        if healthy_file {
            healthy.insert(relative);
        }
    }

    collect_extra_files(&destination, &expected, &mut issues)?;

    for core in &manifest.cores {
        if !healthy.contains(&core.database_relative_path)
            || !healthy.contains(&core.manifest_relative_path)
        {
            continue;
        }
        let database_path = destination.join(&core.database_relative_path);
        let manifest_path = destination.join(&core.manifest_relative_path);
        let identity = match read_core_identity(&core.core_uuid, &database_path) {
            Ok(identity) => identity,
            Err(error) => {
                issues.push(VerificationIssue::new(
                    VerificationIssueKind::Integrity,
                    Some(core.database_relative_path.clone()),
                    error.to_string(),
                ));
                continue;
            }
        };
        if identity.core_number != core.core_number {
            issues.push(VerificationIssue::new(
                VerificationIssueKind::Integrity,
                Some(core.database_relative_path.clone()),
                format!("Core {} number does not match the manifest", core.core_uuid),
            ));
            continue;
        }
        match verify_core_files(
            &core.core_uuid,
            &database_path,
            &manifest_path,
            Some(&identity),
        ) {
            Ok((integrity_manifest, _report)) => {
                if integrity_manifest.database_sha256 != core.database_sha256 {
                    issues.push(VerificationIssue::new(
                        VerificationIssueKind::Integrity,
                        Some(core.database_relative_path.clone()),
                        format!(
                            "Core {} integrity digest does not match the backup manifest",
                            core.core_uuid
                        ),
                    ));
                }
            }
            Err(error) => issues.push(VerificationIssue::new(
                VerificationIssueKind::Integrity,
                Some(core.database_relative_path.clone()),
                error.to_string(),
            )),
        }

        for database in &core.databases {
            if database.file_name == CORE_DATABASE_FILE {
                continue;
            }
            if !healthy.contains(&database.relative_path) {
                continue;
            }
            let path = destination.join(&database.relative_path);
            if let Err(error) =
                verify_database_ownership(&core.core_uuid, &path, &database.ownership_table)
            {
                issues.push(VerificationIssue::new(
                    VerificationIssueKind::Integrity,
                    Some(database.relative_path.clone()),
                    error.to_string(),
                ));
            }
        }
    }

    let status = if issues.is_empty() {
        BackupVerificationStatus::Verified
    } else {
        BackupVerificationStatus::Invalid
    };
    let total_bytes = manifest.files.iter().map(|file| file.bytes).sum();
    let file_count = manifest.files.len();

    Ok(BackupVerification {
        destination,
        status,
        manifest,
        issues,
        file_count,
        total_bytes,
    })
}

/// Alias for [`verify_backup`].
pub fn inspect(destination: impl AsRef<Path>) -> Result<BackupVerification, BackupError> {
    verify_backup(destination)
}

/// Read and structurally validate a backup manifest without hashing files.
pub fn read_manifest(destination: impl AsRef<Path>) -> Result<BackupManifest, BackupError> {
    let destination = absolute_path(destination.as_ref())?;
    let path = destination.join(BACKUP_MANIFEST_FILE);
    let metadata = match fs::symlink_metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Err(BackupError::InvalidBackup(format!(
                "manifest is missing at {}",
                path.display()
            )))
        }
        Err(error) => return Err(error.into()),
    };
    if is_redirected(&metadata) {
        return Err(BackupError::UnsafePath(format!(
            "{} is a symlink or reparse point",
            path.display()
        )));
    }
    if !metadata.is_file() {
        return Err(BackupError::UnsupportedFile(path));
    }
    let bytes = fs::read(&path)?;
    let manifest: BackupManifest = serde_json::from_slice(&bytes)?;
    validate_manifest(&manifest)?;
    Ok(manifest)
}

/// List immediate child directories of `parent` that contain a valid manifest.
pub fn list_backups(parent: impl AsRef<Path>) -> Result<Vec<PathBuf>, BackupError> {
    let parent = absolute_path(parent.as_ref())?;
    let mut backups = Vec::new();
    for entry in fs::read_dir(&parent)? {
        let entry = entry?;
        let metadata = entry.metadata()?;
        if is_redirected(&metadata) || !metadata.is_dir() {
            continue;
        }
        let path = entry.path();
        if !path.join(BACKUP_MANIFEST_FILE).is_file() {
            continue;
        }
        read_manifest(&path)?;
        backups.push(path);
    }
    backups.sort();
    Ok(backups)
}

fn collect_extra_files(
    destination: &Path,
    expected: &BTreeMap<&str, &BackupFile>,
    issues: &mut Vec<VerificationIssue>,
) -> Result<(), BackupError> {
    for entry in fs::read_dir(destination)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if name == BACKUP_MANIFEST_FILE {
            continue;
        }
        let metadata = entry.metadata()?;
        if is_redirected(&metadata) {
            issues.push(VerificationIssue::new(
                VerificationIssueKind::UnsafePath,
                Some(name),
                "entry is a symlink or reparse point",
            ));
            continue;
        }
        if metadata.is_dir() {
            if name == "cores" || name == "init" {
                walk_subtree(destination, &name, expected, issues)?;
            } else if name == "run" {
                // Runtime scratch space is never part of a backup; it is
                // inspected separately by running-Core detection.
            } else {
                issues.push(VerificationIssue::new(
                    VerificationIssueKind::Extra,
                    Some(name),
                    "unexpected directory in backup root",
                ));
            }
            continue;
        }
        if !expected.contains_key(name.as_str()) {
            issues.push(VerificationIssue::new(
                VerificationIssueKind::Extra,
                Some(name),
                "unexpected file in backup root",
            ));
        }
    }
    Ok(())
}

fn walk_subtree(
    destination: &Path,
    root: &str,
    expected: &BTreeMap<&str, &BackupFile>,
    issues: &mut Vec<VerificationIssue>,
) -> Result<(), BackupError> {
    let mut stack = vec![(destination.join(root), root.to_owned())];
    while let Some((directory, relative_directory)) = stack.pop() {
        let entries = match fs::read_dir(&directory) {
            Ok(entries) => entries,
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error.into()),
        };
        for entry in entries {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().into_owned();
            let relative = format!("{relative_directory}/{name}");
            let metadata = entry.metadata()?;
            if is_redirected(&metadata) {
                issues.push(VerificationIssue::new(
                    VerificationIssueKind::UnsafePath,
                    Some(relative),
                    "entry is a symlink or reparse point",
                ));
                continue;
            }
            if metadata.is_dir() {
                stack.push((entry.path(), relative));
                continue;
            }
            if !expected.contains_key(relative.as_str()) {
                issues.push(VerificationIssue::new(
                    VerificationIssueKind::Extra,
                    Some(relative),
                    "unexpected file in backup",
                ));
            }
        }
    }
    Ok(())
}
