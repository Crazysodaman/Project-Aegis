//! Platform-owned registry state. No Core private data belongs here.

use aegis_storage::{now_rfc3339, open_initialized, StoragePaths, PLATFORM_APPLICATION_ID};
use rusqlite::{params, OptionalExtension};
use uuid::Uuid;

use crate::PlatformError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlatformIdentity {
    pub platform_id: String,
    pub callsign: String,
    pub created_at: String,
}

#[derive(Debug, Clone)]
pub struct PlatformStore {
    paths: StoragePaths,
}

impl PlatformStore {
    pub fn new(paths: StoragePaths) -> Self {
        Self { paths }
    }

    pub fn initialize(&self) -> Result<PlatformIdentity, PlatformError> {
        let connection = open_initialized(
            &self.paths.platform_db(),
            PLATFORM_APPLICATION_ID,
            "platform",
        )?;
        connection.execute_batch(
            "CREATE TABLE IF NOT EXISTS platform_identity (
                singleton INTEGER PRIMARY KEY CHECK(singleton = 1),
                platform_id TEXT NOT NULL UNIQUE,
                callsign TEXT NOT NULL UNIQUE,
                created_at TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS platform_core (
                core_uuid TEXT PRIMARY KEY,
                core_number INTEGER NOT NULL CHECK(core_number IN (1, 2, 3, 6, 7)),
                lifecycle_state TEXT NOT NULL CHECK(lifecycle_state IN (
                    'design_only', 'approved', 'installed', 'active', 'suspended', 'retired'
                )),
                created_at TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS platform_config (
                config_key TEXT PRIMARY KEY,
                config_value TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );",
        )?;
        if let Some(identity) = read_identity(&connection)? {
            return Ok(identity);
        }
        let platform_id = Uuid::new_v4().to_string();
        let callsign = format!("AEGIS-PLATFORM-{}", &platform_id[..8].to_ascii_uppercase());
        let created_at = now_rfc3339()?;
        connection.execute(
            "INSERT INTO platform_identity(singleton, platform_id, callsign, created_at)
             VALUES(1, ?1, ?2, ?3)",
            params![platform_id, callsign, created_at],
        )?;
        Ok(PlatformIdentity {
            platform_id,
            callsign,
            created_at,
        })
    }

    pub fn identity(&self) -> Result<PlatformIdentity, PlatformError> {
        let connection = open_initialized(
            &self.paths.platform_db(),
            PLATFORM_APPLICATION_ID,
            "platform",
        )?;
        read_identity(&connection)?.ok_or(PlatformError::IdentityMissing)
    }

    pub fn core_count(&self) -> Result<u32, PlatformError> {
        let connection = open_initialized(
            &self.paths.platform_db(),
            PLATFORM_APPLICATION_ID,
            "platform",
        )?;
        let registry_table: Option<String> = connection
            .query_row(
                "SELECT name FROM sqlite_master WHERE type = 'table' AND name = 'core_registry'",
                [],
                |row| row.get(0),
            )
            .optional()?;
        let table = if registry_table.is_some() {
            "core_registry"
        } else {
            "platform_core"
        };
        let count: i64 =
            connection.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                row.get(0)
            })?;
        u32::try_from(count).map_err(|_| PlatformError::InvalidState("core count"))
    }
}

fn read_identity(
    connection: &rusqlite::Connection,
) -> Result<Option<PlatformIdentity>, PlatformError> {
    let identity = connection
        .query_row(
            "SELECT platform_id, callsign, created_at FROM platform_identity
             WHERE singleton = 1",
            [],
            |row| {
                Ok(PlatformIdentity {
                    platform_id: row.get(0)?,
                    callsign: row.get(1)?,
                    created_at: row.get(2)?,
                })
            },
        )
        .optional()?;
    Ok(identity)
}
