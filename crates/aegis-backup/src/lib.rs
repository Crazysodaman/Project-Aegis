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
//! # Limitations
//!
//! * Platform and Safe databases are copied as raw byte streams. A caller must
//!   not back up a root while another process is writing to those databases.
//! * Runtime scratch state (`run/`, heartbeats, stop sentinels) is not part of a
//!   backup. Verification ignores that directory; restore refuses when it
//!   indicates a running Core.
//! * A failure during the copy phase leaves a partial destination with no
//!   manifest. Such a directory is intentionally never treated as a backup and
//!   can be removed by the operator.
//! * On Windows `owner.proof` is DPAPI-protected for the current user. The bytes
//!   are preserved exactly, but restoring under a different Windows user or
//!   machine will not decrypt it.
//! * Integrity is content integrity, not authenticity. A party able to rewrite
//!   both a database and its manifest can produce a self-consistent pair.

mod backup;
mod error;
mod manifest;
mod pathutil;
mod restore;
mod state;
mod verify;

pub use backup::{backup, BackupReport};
pub use error::BackupError;
pub use manifest::{
    BackupCore, BackupFile, BackupFileKind, BackupManifest, BACKUP_FORMAT_VERSION,
    BACKUP_MANIFEST_FILE, BACKUP_MANIFEST_SCHEMA,
};
pub use restore::{restore, RestoreOptions, RestoreReport};
pub use verify::{
    inspect, list_backups, read_manifest, verify_backup, BackupVerification,
    BackupVerificationStatus, VerificationIssue, VerificationIssueKind,
};
