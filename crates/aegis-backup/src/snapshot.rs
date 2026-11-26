//! Consistent SQLite snapshots.
//!
//! A raw byte copy of a SQLite database is only consistent while the database is
//! closed and has no journal or WAL sidecars. Live databases, especially in WAL
//! mode, keep committed state outside the main file, so copying the main file
//! alone loses data and copying `-wal` separately is racy. Every database is
//! therefore snapshotted through SQLite itself with `VACUUM INTO`, which yields a
//! single, transactionally consistent file that already contains committed WAL
//! content.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use rusqlite::{Connection, OpenFlags};

use crate::error::BackupError;
use crate::pathutil::hash_file;

const BUSY_TIMEOUT: Duration = Duration::from_secs(10);
const SQLITE_SIDECARS: [&str; 3] = ["-wal", "-shm", "-journal"];

/// Return the first SQLite sidecar (`-wal`, `-shm`, or `-journal`) that exists
/// for `database`, if any.
///
/// A sidecar means the main file alone is not a consistent snapshot, so it must
/// never be used as the source of a raw file-copy fallback.
pub(crate) fn sidecar_path(database: &Path) -> Option<PathBuf> {
    for suffix in SQLITE_SIDECARS {
        let mut name = database.as_os_str().to_os_string();
        name.push(suffix);
        let candidate = PathBuf::from(name);
        if fs::symlink_metadata(&candidate).is_ok() {
            return Some(candidate);
        }
    }
    None
}

/// Snapshot `source` into `destination` using `VACUUM INTO`.
///
/// The source is opened read-only, so snapshotting never modifies it. The
/// destination path is supplied as a bound SQL parameter, which makes filenames
/// containing quotes or other SQL metacharacters safe. `destination` must not
/// already exist. Returns the snapshot digest and byte length.
pub(crate) fn vacuum_into(source: &Path, destination: &Path) -> Result<(String, u64), BackupError> {
    let destination_text = destination.to_str().ok_or_else(|| {
        BackupError::UnsafePath(format!(
            "snapshot destination {} is not valid UTF-8",
            destination.display()
        ))
    })?;

    let connection = Connection::open_with_flags(source, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    connection.busy_timeout(BUSY_TIMEOUT)?;
    connection.execute("VACUUM INTO ?1", [destination_text])?;
    drop(connection);

    let result = hash_file(destination);
    if result.is_err() {
        let _ = fs::remove_file(destination);
    }
    result
}
