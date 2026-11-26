//! Platform and Core backup, verification, and restore.
//!
//! This crate treats a Platform storage root as an identity-bearing unit: the
//! Platform databases, the owner proof, every registered Core database, its
//! integrity manifest, and its initialization marker. A backup is a manifest
//! plus a byte-for-byte mirror of that fixed allowlist.
//!
//! # Safety posture
//!
//! * Only the fixed allowlist is ever copied. Arbitrary files, path traversal,
//!   symlinks, and Windows reparse points are rejected.
//! * Core integrity is verified with
//!   [`aegis_core_integrity::verify_core`] before and after every copy.
//! * Verification never repairs anything and never writes to a backup.
//! * Restore refuses a non-empty target unless the caller explicitly opts in,
//!   and an opted-in restore first captures the existing target.
//! * Running Core detection is fail-closed.
//!
//! `owner.proof` is sensitive. Its bytes are copied and hashed but never parsed,
//! logged, or returned through an error message.
//!
//! # Live database consistency
//!
//! Every SQLite database (Platform, Safe, Core, and sidecar databases) is
//! snapshotted through SQLite with `VACUUM INTO` rather than copied as raw
//! bytes. A raw file copy is only used as a fallback when SQLite cannot open the
//! database *and* no `-wal`/`-shm`/`-journal` sidecar is present, which is the
//! only case where the main file is already a self-contained snapshot. Committed
//! WAL content is therefore captured without ever copying a `-wal` file.
//!
//! A Core's integrity manifest is re-sealed for the snapshot, preserving the
//! Core identity, database instance UUID, initialization mode, schema revision,
//! and ownership checks. Ownership is verified again on the snapshot, so a
//! live snapshot cannot swap ownership or weaken identity validation.
//!
//! # Limitations
//!
//! * Runtime scratch state (`run/`, heartbeats, stop sentinels) is not part of a
//!   backup. Verification ignores that directory; restore refuses when it
//!   indicates a running Core.
//! * A database that has a live `-wal`, `-shm`, or `-journal` sidecar but cannot
//!   be snapshotted through SQLite is refused. Raw-copying such a database would
//!   be inconsistent, so there is no unsafe fallback.
//! * When a Core database has uncheckpointed WAL content, there is no sealed
//!   digest for that committed state. The live database is validated with
//!   SQLite `integrity_check` and ownership checks, then the snapshot is
//!   re-sealed. Content integrity for the backup itself is still recorded in the
//!   v2 manifest.
//! * A failure during the copy phase leaves a partial destination with no
//!   manifest. Such a directory is intentionally never treated as a backup and
//!   can be removed by the operator.
//! * On Windows `owner.proof` is DPAPI-protected for the current user. The bytes
//!   are preserved exactly, but restoring under a different Windows user or
//!   machine will not decrypt it.
//! * Integrity is content integrity, not authenticity. A party able to rewrite
//!   both a database and its manifest can produce a self-consistent pair.

mod assets;
mod backup;
mod error;
mod manifest;
mod pathutil;
mod percore;
mod restore;
mod snapshot;
mod state;
mod verify;

pub use backup::{backup, BackupReport};
pub use error::BackupError;
pub use manifest::{
    BackupCore, BackupDatabase, BackupFile, BackupFileKind, BackupManifest, BACKUP_FORMAT_VERSION,
    BACKUP_FORMAT_VERSION_V1, BACKUP_MANIFEST_FILE, BACKUP_MANIFEST_SCHEMA,
    BACKUP_MANIFEST_SCHEMA_V1, CORE_DATABASE_FILE,
};
pub use restore::{restore, RestoreOptions, RestoreReport};
pub use verify::{
    inspect, list_backups, read_manifest, verify_backup, BackupVerification,
    BackupVerificationStatus, VerificationIssue, VerificationIssueKind,
};
