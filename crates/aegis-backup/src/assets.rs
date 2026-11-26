//! Safe, bounded walk of a Core's asset directory.
//!
//! Asset bytes live outside SQLite under `<core>/assets/<asset_id>/<file>`.
//! They are part of the Core's persistent state and must travel with a backup.
//! The walk rejects symlinks and Windows reparse points, keeps every path inside
//! the Core directory, and enforces a file-count, depth, and byte budget so a
//! hostile or corrupt tree cannot exhaust the backup process.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::error::BackupError;
use crate::pathutil::{comparable_path, is_redirected, validate_relative_path};

/// Maximum number of asset files copied for one Core.
pub(crate) const MAX_ASSET_FILES: usize = 100_000;
/// Maximum directory nesting below `<core>/assets`.
pub(crate) const MAX_ASSET_DEPTH: usize = 32;
/// Maximum total asset bytes copied for one Core.
pub(crate) const MAX_ASSET_TOTAL_BYTES: u64 = 128 * 1024 * 1024 * 1024;

/// One regular asset file selected for backup.
pub(crate) struct AssetFile {
    pub absolute: PathBuf,
    pub relative_path: String,
}

/// Collect every regular file under `<core_dir>/assets`.
///
/// A missing `assets` directory yields an empty set. Empty directories are not
/// represented. Results are sorted by relative path for deterministic manifests.
pub(crate) fn collect_asset_files(
    core_dir: &Path,
    core_uuid: &str,
) -> Result<Vec<AssetFile>, BackupError> {
    let assets_dir = core_dir.join("assets");
    let metadata = match fs::symlink_metadata(&assets_dir) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error.into()),
    };
    if is_redirected(&metadata) {
        return Err(BackupError::UnsafePath(format!(
            "{} is a symlink or reparse point",
            assets_dir.display()
        )));
    }
    if !metadata.is_dir() {
        return Err(BackupError::UnsupportedFile(assets_dir));
    }

    let core_comparable = comparable_path(core_dir);
    let mut files = Vec::new();
    let mut total_bytes = 0u64;
    let mut stack = vec![(assets_dir, String::from("assets"), 0usize)];

    while let Some((directory, relative_prefix, depth)) = stack.pop() {
        if depth > MAX_ASSET_DEPTH {
            return Err(BackupError::InvalidSource(
                "asset directory is nested too deeply".to_owned(),
            ));
        }
        for entry in fs::read_dir(&directory)? {
            let entry = entry?;
            let file_name = entry.file_name();
            let name = file_name.to_str().ok_or_else(|| {
                BackupError::UnsafePath("asset path is not valid UTF-8".to_owned())
            })?;
            let relative_within = format!("{relative_prefix}/{name}");
            validate_relative_path(&relative_within)?;

            let absolute = entry.path();
            if !comparable_path(&absolute).starts_with(&core_comparable) {
                return Err(BackupError::UnsafePath(format!(
                    "{} escapes the Core directory",
                    absolute.display()
                )));
            }
            let metadata = fs::symlink_metadata(&absolute)?;
            if is_redirected(&metadata) {
                return Err(BackupError::UnsafePath(format!(
                    "{} is a symlink or reparse point",
                    absolute.display()
                )));
            }
            if metadata.is_dir() {
                stack.push((absolute, relative_within, depth + 1));
                continue;
            }
            if !metadata.is_file() {
                return Err(BackupError::UnsupportedFile(absolute));
            }
            if files.len() >= MAX_ASSET_FILES {
                return Err(BackupError::InvalidSource(format!(
                    "Core {core_uuid} has too many asset files"
                )));
            }
            total_bytes = total_bytes.saturating_add(metadata.len());
            if total_bytes > MAX_ASSET_TOTAL_BYTES {
                return Err(BackupError::InvalidSource(format!(
                    "Core {core_uuid} asset bytes exceed the backup budget"
                )));
            }
            files.push(AssetFile {
                absolute,
                relative_path: format!("cores/{core_uuid}/{relative_within}"),
            });
        }
    }

    files.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
    Ok(files)
}
