//! Per-Core consent registry: affirmative, scoped, revocable grants and an
//! append-only consent event ledger.
//!
//! Consent lives in a dedicated `<core>/consent.db` bound to both the Core UUID
//! and the Core database instance UUID. It is never inferred from trust,
//! ownership, personality, or memory: a grant exists only because it was
//! explicitly recorded here, and revocation or expiry makes checks fail
//! immediately. The store never opens another Core's database and never writes
//! to `core.db`, `memory.db`, `personal.db`, or `possessions.db`.

use std::ffi::OsStr;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use aegis_contracts::core::validate_core_id;
use aegis_contracts::living::LIVING_CONTRACT_VERSION;
pub use aegis_contracts::living::{ConsentGrant, ConsentScope};
use aegis_core_storage::{CoreDatabase, CoreStorageLayout};
use aegis_storage::open_initialized;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::{canonical_core_id, now_rfc3339, SdkError};

/// File name of the per-Core consent database.
pub const CONSENT_DB_FILE: &str = "consent.db";
/// SQLite application marker for a per-Core consent database ("AEGQ").
pub const CONSENT_APPLICATION_ID: i64 = 0x4145_4751;
/// Component name registered in the shared `aegis_schema` table.
pub const CONSENT_COMPONENT: &str = "core_consent";
/// Schema identifier for the consent database.
pub const CONSENT_SCHEMA: &str = "aegis.core.consent.v1";
/// Schema identifier for one consent event.
pub const CONSENT_EVENT_SCHEMA: &str = "aegis.core.consent.event.v1";
/// Current consent database schema revision.
pub const CONSENT_SCHEMA_REVISION: u64 = 1;

/// Ownership table for the consent database.
pub const CONSENT_OWNERSHIP_TABLE: &str = "consent_ownership";
/// Schema marker table for the consent database.
pub const CONSENT_SCHEMA_TABLE: &str = "consent_schema";
/// Consent grant table.
pub const CONSENT_GRANTS_TABLE: &str = "consent_grants";
/// Append-only consent event table.
pub const CONSENT_EVENTS_TABLE: &str = "consent_events";

/// Maximum number of grants returned by one listing.
pub const MAX_CONSENT_LIST: usize = 256;
/// Maximum number of consent events returned by one listing.
pub const MAX_CONSENT_EVENT_LIST: usize = 500;
/// Maximum serialized size of a consent event detail payload.
pub const MAX_CONSENT_EVENT_DETAIL_BYTES: usize = 16 * 1024;

/// Event action recorded when a grant is created.
pub const CONSENT_GRANT_ACTION: &str = "granted";
/// Event action recorded when a grant is revoked.
pub const CONSENT_REVOKE_ACTION: &str = "revoked";
/// Event action recorded when a grant is observed to have expired.
pub const CONSENT_EXPIRE_ACTION: &str = "expired";

const SCHEMA_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS consent_ownership (
    singleton INTEGER NOT NULL PRIMARY KEY CHECK (singleton = 1),
    core_uuid TEXT NOT NULL,
    database_instance_uuid TEXT NOT NULL,
    schema TEXT NOT NULL,
    created_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS consent_schema (
    singleton INTEGER NOT NULL PRIMARY KEY CHECK (singleton = 1),
    schema TEXT NOT NULL,
    revision INTEGER NOT NULL CHECK (revision >= 1),
    created_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS consent_grants (
    grant_id TEXT NOT NULL PRIMARY KEY,
    core_uuid TEXT NOT NULL,
    scope TEXT NOT NULL,
    capability TEXT NOT NULL,
    participants_json TEXT NOT NULL,
    context TEXT NOT NULL,
    granted_at TEXT NOT NULL,
    expires_at TEXT,
    revoked_at TEXT,
    revision INTEGER NOT NULL CHECK (revision >= 1)
);
CREATE INDEX IF NOT EXISTS consent_grants_core_granted_idx
    ON consent_grants(core_uuid, granted_at);
CREATE INDEX IF NOT EXISTS consent_grants_core_scope_idx
    ON consent_grants(core_uuid, scope);
CREATE TABLE IF NOT EXISTS consent_events (
    sequence INTEGER NOT NULL PRIMARY KEY AUTOINCREMENT,
    core_uuid TEXT NOT NULL,
    grant_id TEXT NOT NULL,
    action TEXT NOT NULL,
    detail_json TEXT NOT NULL,
    created_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS consent_events_core_idx
    ON consent_events(core_uuid, sequence);
CREATE TRIGGER IF NOT EXISTS consent_ownership_no_update
BEFORE UPDATE ON consent_ownership
BEGIN
    SELECT RAISE(ABORT, 'consent_ownership is immutable');
END;
CREATE TRIGGER IF NOT EXISTS consent_ownership_no_delete
BEFORE DELETE ON consent_ownership
BEGIN
    SELECT RAISE(ABORT, 'consent_ownership is immutable');
END;
CREATE TRIGGER IF NOT EXISTS consent_events_no_update
BEFORE UPDATE ON consent_events
BEGIN
    SELECT RAISE(ABORT, 'consent_events is append-only');
END;
CREATE TRIGGER IF NOT EXISTS consent_events_no_delete
BEFORE DELETE ON consent_events
BEGIN
    SELECT RAISE(ABORT, 'consent_events is append-only');
END;
"#;

/// One append-only consent event.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConsentEvent {
    /// Event schema identifier.
    pub schema: String,
    /// Monotonic sequence within this Core's database.
    pub sequence: u64,
    /// Owning Core.
    pub core_uuid: String,
    /// Grant this event concerns.
    pub grant_id: String,
    /// Action name (`granted`, `revoked`, `expired`).
    pub action: String,
    /// JSON detail payload.
    pub detail: Value,
    /// Creation timestamp.
    pub created_at: String,
}

/// Writable per-Core consent store.
///
/// Owns a `rusqlite::Connection` bound to `<core>/consent.db`. It never opens
/// another Core's database and never writes to any other per-Core database.
pub struct ConsentStore {
    core_uuid: String,
    database_instance_uuid: String,
    core_dir: PathBuf,
    database_path: PathBuf,
    connection: Connection,
}

impl std::fmt::Debug for ConsentStore {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ConsentStore")
            .field("core_uuid", &self.core_uuid)
            .field("database_instance_uuid", &self.database_instance_uuid)
            .field("database_path", &self.database_path)
            .finish_non_exhaustive()
    }
}

impl ConsentStore {
    /// Open (or initialize) `<core>/consent.db` for an existing Core.
    ///
    /// The Core database is read only; it supplies the immutable Core UUID and
    /// database instance UUID. An existing consent database must carry exactly
    /// one ownership row matching both values.
    pub fn open(root: impl AsRef<Path>, core_uuid: &str) -> Result<Self, SdkError> {
        let core_uuid = canonical_core_id(core_uuid)?;
        let layout = CoreStorageLayout::new(root)?;
        let snapshot = CoreDatabase::read_snapshot(&layout, &core_uuid)?;
        if snapshot.identity.core_uuid != core_uuid {
            return Err(SdkError::OwnershipMismatch);
        }
        let database_instance_uuid = snapshot.metadata.database_instance_uuid.clone();
        validate_core_id(&database_instance_uuid).map_err(|_| SdkError::OwnershipMismatch)?;

        let core_dir = layout.core_dir(&core_uuid)?;
        let consent_path = core_dir.join(CONSENT_DB_FILE);
        let existed = match fs::symlink_metadata(&consent_path) {
            Ok(metadata) => {
                if metadata.file_type().is_symlink() || !metadata.is_file() {
                    return Err(SdkError::ForeignPath);
                }
                true
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => false,
            Err(error) => return Err(error.into()),
        };

        let connection =
            open_initialized(&consent_path, CONSENT_APPLICATION_ID, CONSENT_COMPONENT)?;

        let canonical_dir = fs::canonicalize(&core_dir)?;
        let canonical_db = fs::canonicalize(&consent_path)?;
        if canonical_db.parent() != Some(canonical_dir.as_path())
            || canonical_db.file_name() != Some(OsStr::new(CONSENT_DB_FILE))
        {
            return Err(SdkError::ForeignPath);
        }

        let has_ownership = table_exists(&connection, CONSENT_OWNERSHIP_TABLE)?;
        if existed && !has_ownership {
            return Err(SdkError::OwnershipMissing);
        }
        if has_ownership {
            verify_ownership(&connection, &core_uuid, &database_instance_uuid)?;
        }

        initialize_schema(&connection)?;
        ensure_schema_marker(&connection)?;

        if !has_ownership {
            connection.execute(
                "INSERT INTO consent_ownership(
                    singleton, core_uuid, database_instance_uuid, schema, created_at
                 ) VALUES(1, ?1, ?2, ?3, ?4)",
                params![
                    core_uuid,
                    database_instance_uuid,
                    CONSENT_SCHEMA,
                    now_rfc3339()?
                ],
            )?;
        }

        Ok(Self {
            core_uuid,
            database_instance_uuid,
            core_dir: canonical_dir,
            database_path: canonical_db,
            connection,
        })
    }

    /// Immutable Core UUID bound to this store.
    pub fn core_uuid(&self) -> &str {
        &self.core_uuid
    }

    /// Core database instance UUID this store is bound to.
    pub fn database_instance_uuid(&self) -> &str {
        &self.database_instance_uuid
    }

    /// Canonical Core directory.
    pub fn core_dir(&self) -> &Path {
        &self.core_dir
    }

    /// Canonical path of `consent.db`.
    pub fn database_path(&self) -> &Path {
        &self.database_path
    }

    /// Record an explicit consent grant for this Core.
    ///
    /// The grant is validated, must belong to this Core, and must not already
    /// exist. A `granted` event is appended in the same transaction.
    pub fn grant(&self, grant: &ConsentGrant) -> Result<ConsentGrant, SdkError> {
        grant
            .validate()
            .map_err(|_| SdkError::InvalidMemory("invalid consent grant"))?;
        if grant.revoked_at.is_some() {
            return Err(SdkError::InvalidMemory(
                "consent grant cannot already be revoked",
            ));
        }
        if grant.capability.trim().is_empty() {
            return Err(SdkError::InvalidMemory("consent capability is invalid"));
        }
        let core_uuid = canonical_core_id(&grant.core_uuid)?;
        if core_uuid != self.core_uuid {
            return Err(SdkError::OwnershipMismatch);
        }
        if let Some(expires_at) = &grant.expires_at {
            if expires_at <= &grant.granted_at {
                return Err(SdkError::InvalidMemory(
                    "consent expiry must follow the grant time",
                ));
            }
        }

        let mut grant = grant.clone();
        grant.core_uuid = core_uuid;
        let participants_json = serde_json::to_string(&grant.participants)?;
        let now = now_rfc3339()?;

        let transaction = self.connection.unchecked_transaction()?;
        let existing: Option<i64> = transaction
            .query_row(
                "SELECT 1 FROM consent_grants WHERE grant_id = ?1",
                params![grant.grant_id],
                |row| row.get(0),
            )
            .optional()?;
        if existing.is_some() {
            return Err(SdkError::InvalidMemory("consent grant already exists"));
        }
        transaction.execute(
            "INSERT INTO consent_grants(
                grant_id, core_uuid, scope, capability, participants_json, context,
                granted_at, expires_at, revoked_at, revision
             ) VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, NULL, 1)",
            params![
                grant.grant_id,
                self.core_uuid,
                scope_as_str(grant.scope),
                grant.capability,
                participants_json,
                grant.context,
                grant.granted_at,
                grant.expires_at,
            ],
        )?;
        insert_event(
            &transaction,
            &self.core_uuid,
            &grant.grant_id,
            CONSENT_GRANT_ACTION,
            &json!({
                "grant_id": grant.grant_id,
                "scope": scope_as_str(grant.scope),
                "capability": grant.capability,
                "participants": grant.participants,
                "revision": 1,
            }),
            &now,
        )?;
        transaction.commit()?;
        Ok(grant)
    }

    /// Fetch one grant owned by this Core.
    pub fn get(&self, grant_id: &str) -> Result<Option<ConsentGrant>, SdkError> {
        if !valid_id(grant_id) {
            return Ok(None);
        }
        Ok(read_grant(&self.connection, &self.core_uuid, grant_id)?.map(|(grant, _)| grant))
    }

    /// Grants for this Core, most recently granted first, capped at `limit`.
    pub fn list(&self, limit: usize) -> Result<Vec<ConsentGrant>, SdkError> {
        let Some(limit) = limit_value(limit, MAX_CONSENT_LIST)? else {
            return Ok(Vec::new());
        };
        let mut statement = self.connection.prepare(
            "SELECT grant_id, core_uuid, scope, capability, participants_json, context,
                    granted_at, expires_at, revoked_at, revision
             FROM consent_grants
             WHERE core_uuid = ?1
             ORDER BY granted_at DESC, grant_id ASC LIMIT ?2",
        )?;
        let rows = statement.query_map(params![self.core_uuid, limit], |row| {
            read_grant_row(row, &self.core_uuid)
        })?;
        let mut grants = Vec::new();
        for row in rows {
            grants.push(row??.0);
        }
        Ok(grants)
    }

    /// Revoke a grant. Revocation is immediate: every later [`Self::check`]
    /// returns `false`. A `revoked` event is appended and the revision advances.
    ///
    /// Revoking an already revoked grant is idempotent.
    pub fn revoke(&self, grant_id: &str) -> Result<ConsentGrant, SdkError> {
        if !valid_id(grant_id) {
            return Err(SdkError::NotFound);
        }
        let transaction = self.connection.unchecked_transaction()?;
        let (mut grant, revision) =
            read_grant(&transaction, &self.core_uuid, grant_id)?.ok_or(SdkError::NotFound)?;
        if grant.revoked_at.is_some() {
            transaction.commit()?;
            return Ok(grant);
        }
        let now = now_rfc3339()?;
        let next_revision = revision.saturating_add(1);
        let next_revision = revision_value(next_revision)?;
        let changed = transaction.execute(
            "UPDATE consent_grants
             SET revoked_at = ?3, revision = ?4
             WHERE core_uuid = ?1 AND grant_id = ?2 AND revoked_at IS NULL",
            params![self.core_uuid, grant_id, now, next_revision],
        )?;
        if changed != 1 {
            return Err(SdkError::CorruptData("consent grant row is missing"));
        }
        insert_event(
            &transaction,
            &self.core_uuid,
            grant_id,
            CONSENT_REVOKE_ACTION,
            &json!({
                "grant_id": grant_id,
                "revoked_at": now,
                "revision": next_revision,
            }),
            &now,
        )?;
        transaction.commit()?;
        grant.revoked_at = Some(now);
        Ok(grant)
    }

    /// Active grants for this Core matching the supplied optional filters.
    ///
    /// Expiry is evaluated against the supplied `now`; expired grants are never
    /// returned even before an explicit revocation. Observing an expired grant
    /// appends a single `expired` event to the ledger.
    pub fn active(
        &self,
        scope: Option<ConsentScope>,
        capability: Option<&str>,
        participant: Option<&str>,
        now: &str,
    ) -> Result<Vec<ConsentGrant>, SdkError> {
        let grants = self.list(MAX_CONSENT_LIST)?;
        self.record_expirations(&grants, now)?;
        Ok(grants
            .into_iter()
            .filter(|grant| grant.is_active(now))
            .filter(|grant| scope.is_none_or(|scope| grant.scope == scope))
            .filter(|grant| capability.is_none_or(|capability| grant.capability == capability))
            .filter(|grant| {
                participant.is_none_or(|participant| {
                    grant
                        .participants
                        .iter()
                        .any(|candidate| candidate == participant)
                })
            })
            .collect())
    }

    /// Check whether an active grant exists for exactly this scope, capability,
    /// participant, and point in time.
    ///
    /// Revocation and expiry take effect immediately. Consent is never inferred
    /// from trust, ownership, personality, or memory.
    pub fn check(
        &self,
        scope: ConsentScope,
        capability: &str,
        participant: &str,
        now: &str,
    ) -> bool {
        self.active(Some(scope), Some(capability), Some(participant), now)
            .map(|grants| !grants.is_empty())
            .unwrap_or(false)
    }

    /// Consent events, newest first, capped at `limit`.
    pub fn list_events(&self, limit: usize) -> Result<Vec<ConsentEvent>, SdkError> {
        let Some(limit) = limit_value(limit, MAX_CONSENT_EVENT_LIST)? else {
            return Ok(Vec::new());
        };
        let mut statement = self.connection.prepare(
            "SELECT sequence, grant_id, action, detail_json, created_at
             FROM consent_events
             WHERE core_uuid = ?1
             ORDER BY sequence DESC LIMIT ?2",
        )?;
        let rows = statement.query_map(params![self.core_uuid, limit], |row| {
            read_event_row(row, &self.core_uuid)
        })?;
        let mut events = Vec::new();
        for row in rows {
            events.push(row??);
        }
        Ok(events)
    }

    fn record_expirations(&self, grants: &[ConsentGrant], now: &str) -> Result<(), SdkError> {
        for grant in grants {
            let expired = grant
                .expires_at
                .as_deref()
                .is_some_and(|expires_at| expires_at <= now);
            if grant.revoked_at.is_some() || !expired {
                continue;
            }
            let recorded: Option<i64> = self
                .connection
                .query_row(
                    "SELECT 1 FROM consent_events
                     WHERE core_uuid = ?1 AND grant_id = ?2 AND action = ?3
                     LIMIT 1",
                    params![self.core_uuid, grant.grant_id, CONSENT_EXPIRE_ACTION],
                    |row| row.get(0),
                )
                .optional()?;
            if recorded.is_none() {
                let created_at = now_rfc3339()?;
                insert_event(
                    &self.connection,
                    &self.core_uuid,
                    &grant.grant_id,
                    CONSENT_EXPIRE_ACTION,
                    &json!({
                        "grant_id": grant.grant_id,
                        "expires_at": grant.expires_at,
                    }),
                    &created_at,
                )?;
            }
        }
        Ok(())
    }
}

fn initialize_schema(connection: &Connection) -> Result<(), SdkError> {
    connection.execute_batch(SCHEMA_SQL)?;
    Ok(())
}

fn ensure_schema_marker(connection: &Connection) -> Result<(), SdkError> {
    let existing: Option<(String, i64)> = connection
        .query_row(
            "SELECT schema, revision FROM consent_schema WHERE singleton = 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    match existing {
        None => {
            let revision = revision_value(CONSENT_SCHEMA_REVISION)?;
            connection.execute(
                "INSERT INTO consent_schema(singleton, schema, revision, created_at)
                 VALUES(1, ?1, ?2, ?3)",
                params![CONSENT_SCHEMA, revision, now_rfc3339()?],
            )?;
        }
        Some((schema, revision)) => {
            let revision = u64::try_from(revision)
                .map_err(|_| SdkError::CorruptData("consent schema revision is invalid"))?;
            if schema != CONSENT_SCHEMA || revision > CONSENT_SCHEMA_REVISION {
                return Err(SdkError::CorruptData("consent schema is unsupported"));
            }
        }
    }
    Ok(())
}

fn table_exists(connection: &Connection, name: &str) -> Result<bool, SdkError> {
    let found: Option<String> = connection
        .query_row(
            "SELECT name FROM sqlite_master WHERE type = 'table' AND name = ?1",
            [name],
            |row| row.get(0),
        )
        .optional()?;
    Ok(found.is_some())
}

fn verify_ownership(
    connection: &Connection,
    expected_core: &str,
    expected_instance: &str,
) -> Result<(), SdkError> {
    let count: i64 = connection.query_row("SELECT COUNT(*) FROM consent_ownership", [], |row| {
        row.get(0)
    })?;
    if count != 1 {
        return Err(SdkError::OwnershipMissing);
    }
    let row: Option<(String, String, String)> = connection
        .query_row(
            "SELECT core_uuid, database_instance_uuid, schema
             FROM consent_ownership WHERE singleton = 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()?;
    let Some((stored_core, stored_instance, schema)) = row else {
        return Err(SdkError::OwnershipMissing);
    };
    if schema != CONSENT_SCHEMA {
        return Err(SdkError::CorruptData("consent schema is unsupported"));
    }
    let stored_core = canonical_core_id(&stored_core).map_err(|_| SdkError::OwnershipMismatch)?;
    if stored_core != expected_core {
        return Err(SdkError::OwnershipMismatch);
    }
    let stored_instance = validate_core_id(&stored_instance)
        .map_err(|_| SdkError::OwnershipMismatch)?
        .hyphenated()
        .to_string();
    if stored_instance != expected_instance {
        return Err(SdkError::OwnershipMismatch);
    }
    Ok(())
}

fn read_grant(
    connection: &Connection,
    expected_core: &str,
    grant_id: &str,
) -> Result<Option<(ConsentGrant, u64)>, SdkError> {
    let row = connection
        .query_row(
            "SELECT grant_id, core_uuid, scope, capability, participants_json, context,
                    granted_at, expires_at, revoked_at, revision
             FROM consent_grants
             WHERE core_uuid = ?1 AND grant_id = ?2",
            params![expected_core, grant_id],
            |row| read_grant_row(row, expected_core),
        )
        .optional()?;
    row.transpose()
}

fn read_grant_row(
    row: &rusqlite::Row<'_>,
    expected_core: &str,
) -> Result<Result<(ConsentGrant, u64), SdkError>, rusqlite::Error> {
    let grant_id: String = row.get(0)?;
    let core_uuid: String = row.get(1)?;
    let scope: String = row.get(2)?;
    let capability: String = row.get(3)?;
    let participants_json: String = row.get(4)?;
    let context: String = row.get(5)?;
    let granted_at: String = row.get(6)?;
    let expires_at: Option<String> = row.get(7)?;
    let revoked_at: Option<String> = row.get(8)?;
    let revision: i64 = row.get(9)?;
    Ok((|| {
        let stored_core = canonical_core_id(&core_uuid).map_err(|_| SdkError::OwnershipMismatch)?;
        if stored_core != expected_core {
            return Err(SdkError::OwnershipMismatch);
        }
        let participants: Vec<String> = serde_json::from_str(&participants_json)
            .map_err(|_| SdkError::CorruptData("stored consent participants are invalid"))?;
        let grant = ConsentGrant {
            schema: LIVING_CONTRACT_VERSION.to_owned(),
            grant_id,
            core_uuid: stored_core,
            scope: parse_scope(&scope)
                .ok_or(SdkError::CorruptData("stored consent scope is invalid"))?,
            capability,
            participants,
            granted_at,
            expires_at,
            revoked_at,
            context,
        };
        grant
            .validate()
            .map_err(|_| SdkError::CorruptData("stored consent grant is invalid"))?;
        let revision = u64::try_from(revision)
            .map_err(|_| SdkError::CorruptData("stored consent revision is invalid"))?;
        Ok((grant, revision))
    })())
}

fn read_event_row(
    row: &rusqlite::Row<'_>,
    expected_core: &str,
) -> Result<Result<ConsentEvent, SdkError>, rusqlite::Error> {
    let sequence: i64 = row.get(0)?;
    let grant_id: String = row.get(1)?;
    let action: String = row.get(2)?;
    let detail_json: String = row.get(3)?;
    let created_at: String = row.get(4)?;
    Ok((|| {
        let detail: Value = serde_json::from_str(&detail_json)
            .map_err(|_| SdkError::CorruptData("stored consent event detail is invalid"))?;
        Ok(ConsentEvent {
            schema: CONSENT_EVENT_SCHEMA.to_owned(),
            sequence: u64::try_from(sequence)
                .map_err(|_| SdkError::CorruptData("stored consent event sequence is invalid"))?,
            core_uuid: expected_core.to_owned(),
            grant_id,
            action,
            detail,
            created_at,
        })
    })())
}

fn insert_event(
    connection: &Connection,
    core_uuid: &str,
    grant_id: &str,
    action: &str,
    detail: &Value,
    created_at: &str,
) -> Result<u64, SdkError> {
    let detail_json = serde_json::to_string(detail)?;
    if detail_json.len() > MAX_CONSENT_EVENT_DETAIL_BYTES {
        return Err(SdkError::InvalidMemory("consent event detail is too large"));
    }
    connection.execute(
        "INSERT INTO consent_events(
            core_uuid, grant_id, action, detail_json, created_at
         ) VALUES(?1, ?2, ?3, ?4, ?5)",
        params![core_uuid, grant_id, action, detail_json, created_at],
    )?;
    let sequence = connection.last_insert_rowid();
    u64::try_from(sequence).map_err(|_| SdkError::CorruptData("consent event sequence is invalid"))
}

fn scope_as_str(scope: ConsentScope) -> &'static str {
    match scope {
        ConsentScope::Conversation => "conversation",
        ConsentScope::Affection => "affection",
        ConsentScope::Romantic => "romantic",
        ConsentScope::Adult => "adult",
        ConsentScope::Object => "object",
        ConsentScope::SharedProject => "shared_project",
    }
}

fn parse_scope(value: &str) -> Option<ConsentScope> {
    match value {
        "conversation" => Some(ConsentScope::Conversation),
        "affection" => Some(ConsentScope::Affection),
        "romantic" => Some(ConsentScope::Romantic),
        "adult" => Some(ConsentScope::Adult),
        "object" => Some(ConsentScope::Object),
        "shared_project" => Some(ConsentScope::SharedProject),
        _ => None,
    }
}

fn valid_id(value: &str) -> bool {
    !value.is_empty()
        && aegis_contracts::living::MAX_ID_CHARS >= value.chars().count()
        && value.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.' | ':')
        })
}

fn revision_value(revision: u64) -> Result<i64, SdkError> {
    i64::try_from(revision).map_err(|_| SdkError::CorruptData("consent revision is out of range"))
}

fn limit_value(limit: usize, cap: usize) -> Result<Option<i64>, SdkError> {
    let limit = limit.min(cap);
    if limit == 0 {
        return Ok(None);
    }
    i64::try_from(limit)
        .map(Some)
        .map_err(|_| SdkError::InvalidQuery("limit is out of range"))
}
