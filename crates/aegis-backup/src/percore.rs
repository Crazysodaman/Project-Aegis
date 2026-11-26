//! Per-Core database inventory and ownership validation.
//!
//! Each Core owns one mandatory database (`core.db`) plus a set of optional
//! sidecar databases created lazily by the feature stores. This module records
//! the approved file names, their ownership tables, and the read-only checks
//! used to prove that a database belongs to a Core before it is backed up or
//! after it is restored.

use std::path::Path;

use aegis_contracts::core::CORE_IDENTITY_TABLE;
use rusqlite::{Connection, OpenFlags};

use crate::error::BackupError;
use crate::manifest::BackupFileKind;
use crate::state::{ownership_failure, table_exists};

/// One optional per-Core database and the table that records its owner.
pub(crate) struct SidecarDatabase {
    pub file_name: &'static str,
    pub kind: BackupFileKind,
    pub ownership_table: &'static str,
}

/// Optional sidecar databases, in deterministic order.
///
/// `core.db` is mandatory and is handled separately because it also carries the
/// Core identity row and the integrity manifest.
pub(crate) const SIDECAR_DATABASES: &[SidecarDatabase] = &[
    SidecarDatabase {
        file_name: "memory.db",
        kind: BackupFileKind::CoreMemoryDatabase,
        ownership_table: "core_memory_ownership",
    },
    SidecarDatabase {
        file_name: "conversation.db",
        kind: BackupFileKind::CoreConversationDatabase,
        ownership_table: "conversation_ownership",
    },
    SidecarDatabase {
        file_name: "assets.db",
        kind: BackupFileKind::CoreAssetsDatabase,
        ownership_table: "asset_ownership",
    },
    SidecarDatabase {
        file_name: "social.db",
        kind: BackupFileKind::CoreSocialDatabase,
        ownership_table: "relationship_ownership",
    },
    SidecarDatabase {
        file_name: "embeddings.db",
        kind: BackupFileKind::CoreEmbeddingsDatabase,
        ownership_table: "embedding_ownership",
    },
    SidecarDatabase {
        file_name: "personal.db",
        kind: BackupFileKind::CorePersonalDatabase,
        ownership_table: "personal_ownership",
    },
];

/// Look up a sidecar database by its bare file name.
pub(crate) fn sidecar_for_file_name(file_name: &str) -> Option<&'static SidecarDatabase> {
    SIDECAR_DATABASES
        .iter()
        .find(|database| database.file_name == file_name)
}

/// Look up a sidecar database from a manifest-relative path such as
/// `cores/<uuid>/memory.db`.
pub(crate) fn sidecar_for_relative_path(relative: &str) -> Option<&'static SidecarDatabase> {
    let rest = relative.strip_prefix("cores/")?;
    let (_, file_name) = rest.split_once('/')?;
    if file_name.contains('/') {
        return None;
    }
    sidecar_for_file_name(file_name)
}

/// Prove that `database_path` belongs to `core_uuid` by reading its ownership
/// table. A present database whose ownership cannot be proven is rejected.
pub(crate) fn verify_database_ownership(
    core_uuid: &str,
    database_path: &Path,
    ownership_table: &str,
) -> Result<(), BackupError> {
    let connection = Connection::open_with_flags(database_path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|_| {
        ownership_failure(
            core_uuid,
            "database could not be opened for ownership validation",
        )
    })?;
    verify_database_ownership_with(&connection, core_uuid, ownership_table)
}

/// Ownership validation for an already-open connection.
pub(crate) fn verify_database_ownership_with(
    connection: &Connection,
    core_uuid: &str,
    ownership_table: &str,
) -> Result<(), BackupError> {
    if !table_exists(connection, ownership_table)? {
        return Err(ownership_failure(
            core_uuid,
            &format!("ownership table {ownership_table} is missing"),
        ));
    }
    let statement = format!("SELECT core_uuid FROM {ownership_table}");
    let mut prepared = connection.prepare(&statement)?;
    let owners = prepared
        .query_map([], |row| row.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    if owners.len() != 1 {
        return Err(ownership_failure(
            core_uuid,
            &format!("ownership table {ownership_table} must contain exactly one row"),
        ));
    }
    if owners[0] != core_uuid {
        return Err(ownership_failure(
            core_uuid,
            &format!("ownership table {ownership_table} belongs to another Core"),
        ));
    }
    Ok(())
}

/// Verify the identity ownership row of `core.db` and, when present, that its
/// database instance UUID equals `expected_instance_uuid`.
pub(crate) fn verify_core_database_ownership(
    core_uuid: &str,
    database_path: &Path,
    expected_instance_uuid: Option<&str>,
) -> Result<(), BackupError> {
    verify_database_ownership(core_uuid, database_path, CORE_IDENTITY_TABLE)?;
    if let Some(expected) = expected_instance_uuid {
        let connection =
            Connection::open_with_flags(database_path, OpenFlags::SQLITE_OPEN_READ_ONLY).map_err(
                |_| {
                    ownership_failure(
                        core_uuid,
                        "core database could not be opened for instance validation",
                    )
                },
            )?;
        if !table_exists(&connection, CORE_IDENTITY_TABLE)? {
            return Err(ownership_failure(
                core_uuid,
                "core_identity table is missing",
            ));
        }
        let stored = crate::state::read_core_instance_uuid_with(&connection)?;
        match stored {
            Some(stored) if stored == expected => {}
            Some(_) => {
                return Err(ownership_failure(
                    core_uuid,
                    "database instance UUID does not match the Core identity",
                ))
            }
            None => {
                return Err(ownership_failure(
                    core_uuid,
                    "database instance UUID is missing from the Core identity",
                ))
            }
        }
    }
    Ok(())
}
