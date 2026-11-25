use std::fs::{self, File, Metadata, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Component, Path, PathBuf};

use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::error::BackupError;

const READ_CHUNK_BYTES: usize = 64 * 1024;

/// Whether a metadata entry is a symlink or a Windows reparse point.
#[cfg(windows)]
pub(crate) fn is_redirected(metadata: &Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;

    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0000_0400;
    metadata.file_type().is_symlink()
        || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

/// Whether a metadata entry is a symbolic link.
#[cfg(not(windows))]
pub(crate) fn is_redirected(metadata: &Metadata) -> bool {
    metadata.file_type().is_symlink()
}

pub(crate) fn validate_relative_path(relative: &str) -> Result<(), BackupError> {
    let path = Path::new(relative);
    if relative.is_empty()
        || relative.starts_with('/')
        || relative.starts_with('\\')
        || relative.contains(':')
        || relative.contains('\\')
        || relative.contains('\0')
        || path.is_absolute()
    {
        return Err(BackupError::UnsafePath(format!(
            "not a safe relative path: {relative}"
        )));
    }
    for component in path.components() {
        match component {
            Component::Normal(part) if !part.is_empty() => {}
            _ => {
                return Err(BackupError::UnsafePath(format!(
                    "not a safe relative path: {relative}"
                )))
            }
        }
    }
    Ok(())
}

pub(crate) fn absolute_path(path: &Path) -> Result<PathBuf, BackupError> {
    if path.as_os_str().is_empty() {
        return Err(BackupError::UnsafePath("path is empty".to_owned()));
    }
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()?.join(path)
    };
    Ok(normalize_lexical(&absolute))
}

pub(crate) fn normalize_lexical(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Prefix(prefix) => normalized.push(prefix.as_os_str()),
            Component::RootDir => normalized.push(component.as_os_str()),
            Component::CurDir => {}
            Component::ParentDir => {
                if !normalized.pop() {
                    normalized.push("..");
                }
            }
            Component::Normal(part) => normalized.push(part),
        }
    }
    normalized
}

/// Normalize a path for containment comparison.
///
/// Windows `canonicalize` returns verbatim (`\\\\?\\`) paths while
/// `current_dir`-based paths do not. Stripping the verbatim prefix keeps
/// containment checks meaningful without changing how the path is opened.
pub(crate) fn comparable_path(path: &Path) -> PathBuf {
    let normalized = normalize_lexical(path);
    strip_verbatim_prefix(&normalized)
}

#[cfg(windows)]
fn strip_verbatim_prefix(path: &Path) -> PathBuf {
    let text = path.as_os_str().to_string_lossy();
    if let Some(rest) = text.strip_prefix(r"\\?\UNC\") {
        return PathBuf::from(format!(r"\\{rest}"));
    }
    if let Some(rest) = text.strip_prefix(r"\\?\") {
        return PathBuf::from(rest);
    }
    path.to_path_buf()
}

#[cfg(not(windows))]
fn strip_verbatim_prefix(path: &Path) -> PathBuf {
    path.to_path_buf()
}

pub(crate) fn require_regular_file(path: &Path) -> Result<Metadata, BackupError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            if is_redirected(&metadata) {
                return Err(BackupError::UnsafePath(format!(
                    "{} is a symlink or reparse point",
                    path.display()
                )));
            }
            if !metadata.is_file() {
                return Err(BackupError::UnsupportedFile(path.to_path_buf()));
            }
            Ok(metadata)
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            Err(BackupError::MissingFile(path.to_path_buf()))
        }
        Err(error) => Err(error.into()),
    }
}

pub(crate) fn ensure_no_redirects(base: &Path, relative: &str) -> Result<(), BackupError> {
    validate_relative_path(relative)?;
    let mut current = base.to_path_buf();
    for component in Path::new(relative).components() {
        current.push(component);
        match fs::symlink_metadata(&current) {
            Ok(metadata) if is_redirected(&metadata) => {
                return Err(BackupError::UnsafePath(format!(
                    "{} is a symlink or reparse point",
                    current.display()
                )))
            }
            Ok(_) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    Ok(())
}

pub(crate) fn hash_file(path: &Path) -> Result<(String, u64), BackupError> {
    require_regular_file(path)?;
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; READ_CHUNK_BYTES];
    let mut total = 0u64;
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
        total = total.saturating_add(u64::try_from(read).unwrap_or(0));
    }
    Ok((hex_lower(&hasher.finalize()), total))
}

pub(crate) fn copy_hashed(source: &Path, destination: &Path) -> Result<(String, u64), BackupError> {
    require_regular_file(source)?;
    let mut reader = File::open(source)?;
    let mut writer = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(destination)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; READ_CHUNK_BYTES];
    let mut total = 0u64;
    let result = (|| -> Result<(), BackupError> {
        loop {
            let read = reader.read(&mut buffer)?;
            if read == 0 {
                break;
            }
            hasher.update(&buffer[..read]);
            writer.write_all(&buffer[..read])?;
            total = total.saturating_add(u64::try_from(read).unwrap_or(0));
        }
        writer.sync_all()?;
        Ok(())
    })();
    if let Err(error) = result {
        drop(writer);
        let _ = fs::remove_file(destination);
        return Err(error);
    }
    drop(writer);
    set_file_private(destination)?;
    Ok((hex_lower(&hasher.finalize()), total))
}

pub(crate) fn copy_new_checked(
    source: &Path,
    destination: &Path,
    expected_sha256: &str,
    expected_bytes: u64,
) -> Result<u64, BackupError> {
    let (digest, bytes) = copy_hashed(source, destination)?;
    if digest != expected_sha256 || bytes != expected_bytes {
        let _ = fs::remove_file(destination);
        return Err(BackupError::DigestMismatch {
            path: source.to_path_buf(),
            expected: expected_sha256.to_owned(),
            actual: digest,
        });
    }
    Ok(bytes)
}

pub(crate) fn copy_replace_checked(
    source: &Path,
    destination: &Path,
    expected_sha256: &str,
    expected_bytes: u64,
) -> Result<u64, BackupError> {
    let parent = destination.parent().ok_or_else(|| {
        BackupError::UnsafePath(format!("{} has no parent", destination.display()))
    })?;
    fs::create_dir_all(parent)?;
    let file_name = destination
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| {
            BackupError::UnsafePath(format!("{} has no valid file name", destination.display()))
        })?;
    let temporary = parent.join(format!(
        ".{file_name}.aegis-copy-{}",
        Uuid::new_v4().simple()
    ));

    let result = (|| -> Result<u64, BackupError> {
        let bytes = copy_new_checked(source, &temporary, expected_sha256, expected_bytes)?;
        fs::rename(&temporary, destination)?;
        let (digest, final_bytes) = hash_file(destination)?;
        if digest != expected_sha256 || final_bytes != expected_bytes {
            return Err(BackupError::DigestMismatch {
                path: destination.to_path_buf(),
                expected: expected_sha256.to_owned(),
                actual: digest,
            });
        }
        Ok(bytes)
    })();

    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

pub(crate) fn create_private_dir(path: &Path) -> Result<(), BackupError> {
    fs::create_dir_all(path)?;
    set_dir_private(path)
}

#[cfg(unix)]
pub(crate) fn set_dir_private(path: &Path) -> Result<(), BackupError> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    Ok(())
}

#[cfg(not(unix))]
pub(crate) fn set_dir_private(_path: &Path) -> Result<(), BackupError> {
    Ok(())
}

#[cfg(unix)]
pub(crate) fn set_file_private(path: &Path) -> Result<(), BackupError> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    Ok(())
}

#[cfg(not(unix))]
pub(crate) fn set_file_private(_path: &Path) -> Result<(), BackupError> {
    Ok(())
}

pub(crate) fn hex_lower(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        let _ = write!(encoded, "{byte:02x}");
    }
    encoded
}
