//! Platform host identity and explicit enrollment state.
//!
//! The host identity is Platform-owned and stored in `platform.db`. It is
//! deliberately separate from any Core identity. Enrollment is an explicit,
//! owner-authorized trust transition; a host is never enrolled implicitly.
//!
//! This crate records host-local trust only. It performs no network discovery,
//! contacts no remote service, and grants no Core or module privileges.

use aegis_safe::{SafeError, SafeStore};
use aegis_storage::{
    now_rfc3339, open_initialized, StorageError, StoragePaths, PLATFORM_APPLICATION_ID,
};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

/// Schema identifier for the persisted host identity.
pub const HOST_IDENTITY_SCHEMA: &str = "aegis.host.identity.v1";
/// Schema identifier for the persisted enrollment state.
pub const HOST_ENROLLMENT_SCHEMA: &str = "aegis.host.enrollment.v1";
/// Schema identifier for append-only enrollment history.
pub const HOST_ENROLLMENT_HISTORY_SCHEMA: &str = "aegis.host.enrollment_history.v1";

const PLATFORM_COMPONENT: &str = "platform";

#[derive(Debug, Error)]
pub enum HostError {
    #[error("host storage failed: {0}")]
    Storage(#[from] StorageError),
    #[error("host SQLite operation failed: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("Safe operation failed: {0}")]
    Safe(#[from] SafeError),
    #[error("Platform is not initialized at {0}")]
    NotInitialized(String),
    #[error("owner authorization failed")]
    Unauthorized,
    #[error("host enrollment state is invalid: {0}")]
    InvalidState(&'static str),
}

/// Persisted host identity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostIdentity {
    pub schema: String,
    pub host_uuid: String,
    pub callsign: String,
    pub os: String,
    pub arch: String,
    pub created_at: String,
}

/// Current enrollment state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostEnrollment {
    pub schema: String,
    pub state: String,
    pub revision: u64,
    pub updated_at: String,
    pub updated_by: String,
    pub note: String,
}

/// Combined host status.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostStatus {
    pub identity: HostIdentity,
    pub enrollment: HostEnrollment,
}

/// One append-only enrollment transition.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnrollmentChange {
    pub schema: String,
    pub host_uuid: String,
    pub from_state: String,
    pub to_state: String,
    pub actor: String,
    pub note: String,
    pub changed_at: String,
}

/// Platform host store for one storage root.
#[derive(Debug, Clone)]
pub struct HostStore {
    paths: StoragePaths,
    safe: SafeStore,
}

impl HostStore {
    pub fn new(paths: StoragePaths) -> Self {
        let safe = SafeStore::new(paths.clone());
        Self { paths, safe }
    }

    /// Open the host store, creating host tables and identity on first use.
    ///
    /// The Platform database must already exist; this crate never initializes a
    /// Platform on its own.
    pub fn initialize(&self) -> Result<HostStatus, HostError> {
        if !self.paths.platform_db().is_file() {
            return Err(HostError::NotInitialized(
                self.paths.root().display().to_string(),
            ));
        }
        let connection = self.connection()?;
        connection.execute_batch(
            "CREATE TABLE IF NOT EXISTS platform_host (
                singleton INTEGER PRIMARY KEY CHECK(singleton = 1),
                host_uuid TEXT NOT NULL UNIQUE,
                callsign TEXT NOT NULL UNIQUE,
                os TEXT NOT NULL,
                arch TEXT NOT NULL,
                created_at TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS platform_host_enrollment (
                singleton INTEGER PRIMARY KEY CHECK(singleton = 1),
                state TEXT NOT NULL CHECK(state IN ('unenrolled', 'enrolled')),
                revision INTEGER NOT NULL CHECK(revision >= 0),
                updated_at TEXT NOT NULL,
                updated_by TEXT NOT NULL,
                note TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS platform_host_enrollment_history (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                schema TEXT NOT NULL,
                host_uuid TEXT NOT NULL,
                from_state TEXT NOT NULL,
                to_state TEXT NOT NULL,
                actor TEXT NOT NULL,
                note TEXT NOT NULL,
                changed_at TEXT NOT NULL
            );",
        )?;
        self.ensure_identity(&connection)?;
        self.ensure_enrollment(&connection)?;
        self.read_status(&connection)
    }

    /// Current host status without creating anything that does not exist.
    pub fn status(&self) -> Result<HostStatus, HostError> {
        self.initialize()
    }

    /// Explicitly enroll this host as trusted. Requires owner proof.
    ///
    /// Enrolling an already-enrolled host is idempotent and appends no history.
    pub fn enroll(&self, owner_token: &str) -> Result<HostStatus, HostError> {
        self.authorize(owner_token)?;
        self.transition("enrolled", "owner", "owner enrolled this host")
    }

    /// Revoke host trust. Requires owner proof. Idempotent when unenrolled.
    pub fn revoke(&self, owner_token: &str) -> Result<HostStatus, HostError> {
        self.authorize(owner_token)?;
        self.transition("unenrolled", "owner", "owner revoked host trust")
    }

    /// Append-only enrollment history, oldest first.
    pub fn history(&self) -> Result<Vec<EnrollmentChange>, HostError> {
        let status = self.initialize()?;
        let connection = self.connection()?;
        let mut statement = connection.prepare(
            "SELECT schema, host_uuid, from_state, to_state, actor, note, changed_at
             FROM platform_host_enrollment_history WHERE host_uuid = ?1 ORDER BY id ASC",
        )?;
        let mut rows = statement.query([status.identity.host_uuid.as_str()])?;
        let mut changes = Vec::new();
        while let Some(row) = rows.next()? {
            changes.push(EnrollmentChange {
                schema: row.get(0)?,
                host_uuid: row.get(1)?,
                from_state: row.get(2)?,
                to_state: row.get(3)?,
                actor: row.get(4)?,
                note: row.get(5)?,
                changed_at: row.get(6)?,
            });
        }
        Ok(changes)
    }

    fn transition(&self, target: &str, actor: &str, note: &str) -> Result<HostStatus, HostError> {
        let current = self.initialize()?;
        if current.enrollment.state == target {
            return Ok(current);
        }
        let mut connection = self.connection()?;
        let transaction = connection.transaction()?;
        let revision = current.enrollment.revision + 1;
        let changed_at = now_rfc3339()?;
        transaction.execute(
            "UPDATE platform_host_enrollment
             SET state = ?1, revision = ?2, updated_at = ?3, updated_by = ?4, note = ?5
             WHERE singleton = 1",
            params![target, revision as i64, changed_at, actor, note],
        )?;
        transaction.execute(
            "INSERT INTO platform_host_enrollment_history(
                schema, host_uuid, from_state, to_state, actor, note, changed_at
             ) VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                HOST_ENROLLMENT_HISTORY_SCHEMA,
                current.identity.host_uuid,
                current.enrollment.state,
                target,
                actor,
                note,
                changed_at,
            ],
        )?;
        transaction.commit()?;
        self.read_status(&self.connection()?)
    }

    fn authorize(&self, owner_token: &str) -> Result<(), HostError> {
        if !self.paths.platform_db().is_file() {
            return Err(HostError::NotInitialized(
                self.paths.root().display().to_string(),
            ));
        }
        if !self.safe.verify_owner(owner_token)? {
            return Err(HostError::Unauthorized);
        }
        Ok(())
    }

    fn connection(&self) -> Result<Connection, HostError> {
        Ok(open_initialized(
            &self.paths.platform_db(),
            PLATFORM_APPLICATION_ID,
            PLATFORM_COMPONENT,
        )?)
    }

    fn ensure_identity(&self, connection: &Connection) -> Result<(), HostError> {
        let existing: Option<String> = connection
            .query_row(
                "SELECT host_uuid FROM platform_host WHERE singleton = 1",
                [],
                |row| row.get(0),
            )
            .optional()?;
        if existing.is_some() {
            return Ok(());
        }
        let host_uuid = Uuid::new_v4().to_string();
        let callsign = format!("AEGIS-HOST-{}", &host_uuid[..8].to_ascii_uppercase());
        let created_at = now_rfc3339()?;
        connection.execute(
            "INSERT INTO platform_host(singleton, host_uuid, callsign, os, arch, created_at)
             VALUES(1, ?1, ?2, ?3, ?4, ?5)",
            params![
                host_uuid,
                callsign,
                std::env::consts::OS,
                std::env::consts::ARCH,
                created_at
            ],
        )?;
        Ok(())
    }

    fn ensure_enrollment(&self, connection: &Connection) -> Result<(), HostError> {
        let count: i64 =
            connection.query_row("SELECT COUNT(*) FROM platform_host_enrollment", [], |row| {
                row.get(0)
            })?;
        if count > 0 {
            return Ok(());
        }
        connection.execute(
            "INSERT INTO platform_host_enrollment(
                singleton, state, revision, updated_at, updated_by, note
             ) VALUES(1, 'unenrolled', 0, ?1, 'system', 'awaiting explicit owner enrollment')",
            params![now_rfc3339()?],
        )?;
        Ok(())
    }

    fn read_status(&self, connection: &Connection) -> Result<HostStatus, HostError> {
        let identity = connection.query_row(
            "SELECT host_uuid, callsign, os, arch, created_at
             FROM platform_host WHERE singleton = 1",
            [],
            |row| {
                Ok(HostIdentity {
                    schema: HOST_IDENTITY_SCHEMA.to_owned(),
                    host_uuid: row.get(0)?,
                    callsign: row.get(1)?,
                    os: row.get(2)?,
                    arch: row.get(3)?,
                    created_at: row.get(4)?,
                })
            },
        )?;
        let enrollment = connection.query_row(
            "SELECT state, revision, updated_at, updated_by, note
             FROM platform_host_enrollment WHERE singleton = 1",
            [],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                ))
            },
        )?;
        if enrollment.1 < 0 {
            return Err(HostError::InvalidState("enrollment revision"));
        }
        Ok(HostStatus {
            identity,
            enrollment: HostEnrollment {
                schema: HOST_ENROLLMENT_SCHEMA.to_owned(),
                state: enrollment.0,
                revision: enrollment.1 as u64,
                updated_at: enrollment.2,
                updated_by: enrollment.3,
                note: enrollment.4,
            },
        })
    }
}
