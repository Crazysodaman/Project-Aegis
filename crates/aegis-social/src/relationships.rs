//! Per-Core private relationship records.
//!
//! Each Core owns its relationships in `<core>/social.db`, bound to the Core
//! UUID and database instance UUID. Relationship data is never stored in the
//! shared group database and is not visible to other Cores by default.

use std::path::{Path, PathBuf};

use aegis_contracts::core::canonicalize_core_id;
use aegis_core_storage::{CoreDatabase, CoreStorageLayout};
use aegis_storage::{now_rfc3339, open_initialized};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use crate::error::SocialError;

/// File name of the per-Core relationship database.
pub const RELATIONSHIP_DB_FILE: &str = "social.db";
/// SQLite application marker for a relationship database ("AEGR").
pub const RELATIONSHIP_APPLICATION_ID: i64 = 0x4145_4752;
/// Component name registered in the shared `aegis_schema` table.
pub const RELATIONSHIP_COMPONENT: &str = "social_relationships";
/// Schema identifier for a relationship.
pub const RELATIONSHIP_SCHEMA: &str = "aegis.social.relationship.v1";
/// Schema identifier for a relationship event.
pub const RELATIONSHIP_EVENT_SCHEMA: &str = "aegis.social.relationship_event.v1";
/// Maximum character length of an explicitly recorded relationship event type.
pub const MAX_RELATIONSHIP_EVENT_TYPE_CHARS: usize = 64;
/// Maximum serialized byte length of an explicit relationship event detail.
pub const MAX_RELATIONSHIP_EVENT_DETAIL_BYTES: usize = 8 * 1024;

/// What kind of entity a relationship refers to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RelationshipTargetKind {
    Core,
    Person,
    Other,
}

impl RelationshipTargetKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Core => "core",
            Self::Person => "person",
            Self::Other => "other",
        }
    }

    pub fn parse(value: &str) -> Result<Self, SocialError> {
        match value {
            "core" => Ok(Self::Core),
            "person" => Ok(Self::Person),
            "other" => Ok(Self::Other),
            _ => Err(SocialError::InvalidInput("target kind")),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Relationship {
    pub schema: String,
    pub relationship_id: String,
    pub core_uuid: String,
    pub target_kind: RelationshipTargetKind,
    pub target_id: String,
    pub label: String,
    pub trust: u8,
    pub notes: String,
    pub tags: Vec<String>,
    pub created_at: String,
    pub updated_at: String,
    pub revision: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RelationshipEvent {
    pub schema: String,
    pub relationship_id: String,
    pub event_type: String,
    pub detail: Value,
    pub created_at: String,
    /// Relationship revision this event was recorded at. Created/updated events
    /// use the revision they produced; explicitly recorded events advance the
    /// relationship revision by one before being appended.
    pub revision: u64,
}

/// Private relationship store for one Core.
#[derive(Debug)]
pub struct RelationshipStore {
    connection: Connection,
    core_uuid: String,
    instance_uuid: String,
    path: PathBuf,
}

impl RelationshipStore {
    pub fn open(root: impl AsRef<Path>, core_uuid: &str) -> Result<Self, SocialError> {
        let core_uuid =
            canonicalize_core_id(core_uuid).map_err(|_| SocialError::InvalidCoreUuid)?;
        let layout = CoreStorageLayout::new(root.as_ref())?;
        let snapshot = CoreDatabase::read_snapshot(&layout, &core_uuid)?;
        if snapshot.identity.core_uuid != core_uuid {
            return Err(SocialError::OwnershipMismatch);
        }
        let core_dir = layout.core_dir(&core_uuid)?;
        let path = core_dir.join(RELATIONSHIP_DB_FILE);
        let connection =
            open_initialized(&path, RELATIONSHIP_APPLICATION_ID, RELATIONSHIP_COMPONENT)?;
        ensure_tables(&connection)?;
        verify_ownership(
            &connection,
            &core_uuid,
            &snapshot.metadata.database_instance_uuid,
        )?;
        Ok(Self {
            connection,
            core_uuid,
            instance_uuid: snapshot.metadata.database_instance_uuid,
            path,
        })
    }

    pub fn core_uuid(&self) -> &str {
        &self.core_uuid
    }

    pub fn database_instance_uuid(&self) -> &str {
        &self.instance_uuid
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn upsert(
        &self,
        target_kind: RelationshipTargetKind,
        target_id: &str,
        label: &str,
        trust: u8,
        notes: &str,
        tags: Vec<String>,
    ) -> Result<Relationship, SocialError> {
        if trust > 100 {
            return Err(SocialError::InvalidInput("trust"));
        }
        let target_id = normalize_target(target_kind, target_id)?;
        let label = validate_text(label, 128, "label")?;
        let notes = validate_optional_text(notes, 4096, "notes")?;
        validate_tags(&tags)?;
        let existing = self.find(target_kind, &target_id)?;
        let now = now_rfc3339()?;
        let tags_json = serde_json::to_string(&tags)?;
        if let Some(existing) = existing {
            let revision = existing.revision.saturating_add(1);
            self.connection.execute(
                "UPDATE relationships
                 SET label = ?1, trust = ?2, notes = ?3, tags_json = ?4,
                     updated_at = ?5, revision = ?6
                 WHERE relationship_id = ?7",
                params![
                    label,
                    i64::from(trust),
                    notes,
                    tags_json,
                    now,
                    revision as i64,
                    existing.relationship_id
                ],
            )?;
            self.append_event(
                &existing.relationship_id,
                "relationship.updated",
                &serde_json::json!({"revision": revision, "trust": trust}),
                revision,
            )?;
            return self.relationship(&existing.relationship_id);
        }
        let relationship_id = Uuid::new_v4().to_string();
        self.connection.execute(
            "INSERT INTO relationships(
                relationship_id, core_uuid, target_kind, target_id, label,
                trust, notes, tags_json, created_at, updated_at, revision
             ) VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?9, 1)",
            params![
                relationship_id,
                self.core_uuid,
                target_kind.as_str(),
                target_id,
                label,
                i64::from(trust),
                notes,
                tags_json,
                now,
            ],
        )?;
        self.append_event(
            &relationship_id,
            "relationship.created",
            &serde_json::json!({"target_kind": target_kind.as_str(), "target_id": target_id}),
            1,
        )?;
        self.relationship(&relationship_id)
    }

    pub fn relationship(&self, relationship_id: &str) -> Result<Relationship, SocialError> {
        self.connection
            .query_row(
                "SELECT relationship_id, core_uuid, target_kind, target_id, label,
                        trust, notes, tags_json, created_at, updated_at, revision
                 FROM relationships WHERE relationship_id = ?1 AND core_uuid = ?2",
                params![relationship_id, self.core_uuid],
                read_relationship,
            )
            .optional()?
            .ok_or(SocialError::MemberNotFound)
    }

    pub fn list(&self) -> Result<Vec<Relationship>, SocialError> {
        let mut statement = self.connection.prepare(
            "SELECT relationship_id, core_uuid, target_kind, target_id, label,
                    trust, notes, tags_json, created_at, updated_at, revision
             FROM relationships WHERE core_uuid = ?1
             ORDER BY created_at ASC, relationship_id ASC",
        )?;
        let mut rows = statement.query([self.core_uuid.as_str()])?;
        let mut relationships = Vec::new();
        while let Some(row) = rows.next()? {
            relationships.push(read_relationship(row)?);
        }
        Ok(relationships)
    }

    pub fn delete(
        &self,
        target_kind: RelationshipTargetKind,
        target_id: &str,
    ) -> Result<bool, SocialError> {
        let Some(existing) = self.find(target_kind, target_id)? else {
            return Ok(false);
        };
        let revision = existing.revision.saturating_add(1);
        self.append_event(
            &existing.relationship_id,
            "relationship.deleted",
            &serde_json::json!({"target_kind": target_kind.as_str()}),
            revision,
        )?;
        self.connection.execute(
            "DELETE FROM relationships WHERE relationship_id = ?1",
            [existing.relationship_id.as_str()],
        )?;
        Ok(true)
    }

    /// Append one explicit event to a relationship and advance its revision.
    ///
    /// This is the only supported way to add a caller-defined event (for
    /// example an interaction or observation). The event is appended, never
    /// rewritten, and the relationship's `revision` is incremented in the same
    /// transaction so the event carries the exact revision it produced. The
    /// caller supplies every field; nothing is inferred from other Cores or
    /// from group membership.
    pub fn record_event(
        &self,
        target_kind: RelationshipTargetKind,
        target_id: &str,
        event_type: &str,
        detail: Value,
    ) -> Result<RelationshipEvent, SocialError> {
        let event_type = validate_event_type(event_type)?;
        let detail_json = serde_json::to_string(&detail)?;
        if detail_json.len() > MAX_RELATIONSHIP_EVENT_DETAIL_BYTES {
            return Err(SocialError::InvalidInput("event detail"));
        }
        let existing = self
            .find(target_kind, target_id)?
            .ok_or(SocialError::MemberNotFound)?;
        let revision = existing.revision.saturating_add(1);
        let revision_db = i64::try_from(revision)
            .map_err(|_| SocialError::CorruptData("relationship revision"))?;
        let now = now_rfc3339()?;
        let transaction = self.connection.unchecked_transaction()?;
        transaction.execute(
            "UPDATE relationships
             SET updated_at = ?1, revision = ?2
             WHERE relationship_id = ?3 AND core_uuid = ?4",
            params![now, revision_db, existing.relationship_id, self.core_uuid],
        )?;
        insert_relationship_event(
            &transaction,
            &existing.relationship_id,
            &event_type,
            &detail,
            &now,
            revision_db,
        )?;
        transaction.commit()?;
        Ok(RelationshipEvent {
            schema: RELATIONSHIP_EVENT_SCHEMA.to_owned(),
            relationship_id: existing.relationship_id,
            event_type,
            detail,
            created_at: now,
            revision,
        })
    }

    pub fn events(&self, relationship_id: &str) -> Result<Vec<RelationshipEvent>, SocialError> {
        let mut statement = self.connection.prepare(
            "SELECT event_type, detail_json, created_at, revision FROM relationship_events
             WHERE relationship_id = ?1 ORDER BY event_id ASC",
        )?;
        let mut rows = statement.query([relationship_id])?;
        let mut events = Vec::new();
        while let Some(row) = rows.next()? {
            let detail_json: String = row.get(1)?;
            let revision: i64 = row.get(3)?;
            events.push(RelationshipEvent {
                schema: RELATIONSHIP_EVENT_SCHEMA.to_owned(),
                relationship_id: relationship_id.to_owned(),
                event_type: row.get(0)?,
                detail: serde_json::from_str(&detail_json)?,
                created_at: row.get(2)?,
                revision: u64::try_from(revision)
                    .map_err(|_| SocialError::CorruptData("relationship event revision"))?,
            });
        }
        Ok(events)
    }

    fn find(
        &self,
        target_kind: RelationshipTargetKind,
        target_id: &str,
    ) -> Result<Option<Relationship>, SocialError> {
        let target_id = normalize_target(target_kind, target_id)?;
        Ok(self
            .connection
            .query_row(
                "SELECT relationship_id, core_uuid, target_kind, target_id, label,
                        trust, notes, tags_json, created_at, updated_at, revision
                 FROM relationships
                 WHERE core_uuid = ?1 AND target_kind = ?2 AND target_id = ?3",
                params![self.core_uuid, target_kind.as_str(), target_id],
                read_relationship,
            )
            .optional()?)
    }

    fn append_event(
        &self,
        relationship_id: &str,
        event_type: &str,
        detail: &Value,
        revision: u64,
    ) -> Result<(), SocialError> {
        let revision = i64::try_from(revision)
            .map_err(|_| SocialError::CorruptData("relationship revision"))?;
        insert_relationship_event(
            &self.connection,
            relationship_id,
            event_type,
            detail,
            &now_rfc3339()?,
            revision,
        )
    }
}

fn insert_relationship_event(
    connection: &Connection,
    relationship_id: &str,
    event_type: &str,
    detail: &Value,
    created_at: &str,
    revision: i64,
) -> Result<(), SocialError> {
    connection.execute(
        "INSERT INTO relationship_events(
            relationship_id, schema, event_type, detail_json, created_at, revision
         ) VALUES(?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            relationship_id,
            RELATIONSHIP_EVENT_SCHEMA,
            event_type,
            serde_json::to_string(detail)?,
            created_at,
            revision
        ],
    )?;
    Ok(())
}

fn ensure_tables(connection: &Connection) -> Result<(), SocialError> {
    connection.execute_batch(
        "CREATE TABLE IF NOT EXISTS relationship_ownership (
            singleton INTEGER PRIMARY KEY CHECK(singleton = 1),
            core_uuid TEXT NOT NULL UNIQUE,
            database_instance_uuid TEXT NOT NULL,
            created_at TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS relationships (
            relationship_id TEXT PRIMARY KEY,
            core_uuid TEXT NOT NULL,
            target_kind TEXT NOT NULL CHECK(target_kind IN ('core', 'person', 'other')),
            target_id TEXT NOT NULL,
            label TEXT NOT NULL,
            trust INTEGER NOT NULL CHECK(trust >= 0 AND trust <= 100),
            notes TEXT NOT NULL,
            tags_json TEXT NOT NULL,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            revision INTEGER NOT NULL CHECK(revision >= 1),
            UNIQUE(core_uuid, target_kind, target_id)
        );
        CREATE TABLE IF NOT EXISTS relationship_events (
            event_id INTEGER PRIMARY KEY AUTOINCREMENT,
            relationship_id TEXT NOT NULL,
            schema TEXT NOT NULL,
            event_type TEXT NOT NULL,
            detail_json TEXT NOT NULL,
            created_at TEXT NOT NULL,
            revision INTEGER NOT NULL DEFAULT 0
        );
        CREATE INDEX IF NOT EXISTS relationship_events_by_relationship
            ON relationship_events(relationship_id, event_id);
        CREATE TRIGGER IF NOT EXISTS relationship_ownership_no_update
        BEFORE UPDATE ON relationship_ownership
        BEGIN
            SELECT RAISE(ABORT, 'relationship ownership is immutable');
        END;
        CREATE TRIGGER IF NOT EXISTS relationship_ownership_no_delete
        BEFORE DELETE ON relationship_ownership
        BEGIN
            SELECT RAISE(ABORT, 'relationship ownership is immutable');
        END;
        CREATE TRIGGER IF NOT EXISTS relationship_events_no_update
        BEFORE UPDATE ON relationship_events
        BEGIN
            SELECT RAISE(ABORT, 'relationship events are append-only');
        END;
        CREATE TRIGGER IF NOT EXISTS relationship_events_no_delete
        BEFORE DELETE ON relationship_events
        BEGIN
            SELECT RAISE(ABORT, 'relationship events are append-only');
        END;",
    )?;
    ensure_event_revision_column(connection)?;
    Ok(())
}

fn ensure_event_revision_column(connection: &Connection) -> Result<(), SocialError> {
    let mut has_revision = false;
    {
        let mut statement = connection.prepare("PRAGMA table_info(relationship_events)")?;
        let mut rows = statement.query([])?;
        while let Some(row) = rows.next()? {
            let name: String = row.get(1)?;
            if name == "revision" {
                has_revision = true;
                break;
            }
        }
    }
    if !has_revision {
        connection.execute(
            "ALTER TABLE relationship_events ADD COLUMN revision INTEGER NOT NULL DEFAULT 0",
            [],
        )?;
    }
    Ok(())
}

fn verify_ownership(
    connection: &Connection,
    core_uuid: &str,
    instance_uuid: &str,
) -> Result<(), SocialError> {
    let existing: Option<(String, String)> = connection
        .query_row(
            "SELECT core_uuid, database_instance_uuid FROM relationship_ownership
             WHERE singleton = 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    match existing {
        None => {
            connection.execute(
                "INSERT INTO relationship_ownership(
                    singleton, core_uuid, database_instance_uuid, created_at
                 ) VALUES(1, ?1, ?2, ?3)",
                params![core_uuid, instance_uuid, now_rfc3339()?],
            )?;
            Ok(())
        }
        Some((stored_core, stored_instance))
            if stored_core == core_uuid && stored_instance == instance_uuid =>
        {
            Ok(())
        }
        Some(_) => Err(SocialError::OwnershipMismatch),
    }
}

fn read_relationship(row: &rusqlite::Row<'_>) -> rusqlite::Result<Relationship> {
    let kind: String = row.get(2)?;
    let target_kind = RelationshipTargetKind::parse(&kind).map_err(|_| {
        rusqlite::Error::FromSqlConversionFailure(
            2,
            rusqlite::types::Type::Text,
            Box::new(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "invalid target kind",
            )),
        )
    })?;
    let tags_json: String = row.get(7)?;
    let trust: i64 = row.get(5)?;
    let revision: i64 = row.get(10)?;
    if !(0..=100).contains(&trust) || revision < 1 {
        return Err(rusqlite::Error::InvalidColumnType(
            5,
            "relationship bounds".to_owned(),
            rusqlite::types::Type::Integer,
        ));
    }
    Ok(Relationship {
        schema: RELATIONSHIP_SCHEMA.to_owned(),
        relationship_id: row.get(0)?,
        core_uuid: row.get(1)?,
        target_kind,
        target_id: row.get(3)?,
        label: row.get(4)?,
        trust: trust as u8,
        notes: row.get(6)?,
        tags: serde_json::from_str(&tags_json).map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(
                7,
                rusqlite::types::Type::Text,
                Box::new(error),
            )
        })?,
        created_at: row.get(8)?,
        updated_at: row.get(9)?,
        revision: revision as u64,
    })
}

fn normalize_target(
    target_kind: RelationshipTargetKind,
    target_id: &str,
) -> Result<String, SocialError> {
    match target_kind {
        RelationshipTargetKind::Core => {
            canonicalize_core_id(target_id).map_err(|_| SocialError::InvalidCoreUuid)
        }
        RelationshipTargetKind::Person | RelationshipTargetKind::Other => {
            validate_text(target_id, 128, "target id")
        }
    }
}

fn validate_text(value: &str, maximum: usize, field: &'static str) -> Result<String, SocialError> {
    let trimmed = value.trim();
    if trimmed.is_empty() || value.chars().count() > maximum || value.chars().any(char::is_control)
    {
        return Err(SocialError::InvalidInput(field));
    }
    Ok(value.to_owned())
}

fn validate_event_type(value: &str) -> Result<String, SocialError> {
    if value.trim().is_empty()
        || value.chars().count() > MAX_RELATIONSHIP_EVENT_TYPE_CHARS
        || value.chars().any(char::is_control)
    {
        return Err(SocialError::InvalidInput("event type"));
    }
    Ok(value.to_owned())
}

fn validate_optional_text(
    value: &str,
    maximum: usize,
    field: &'static str,
) -> Result<String, SocialError> {
    if value.chars().count() > maximum || value.chars().any(char::is_control) {
        return Err(SocialError::InvalidInput(field));
    }
    Ok(value.to_owned())
}

fn validate_tags(tags: &[String]) -> Result<(), SocialError> {
    if tags.len() > 32 {
        return Err(SocialError::InvalidInput("tags"));
    }
    for tag in tags {
        if tag.trim().is_empty() || tag.chars().count() > 64 || tag.chars().any(char::is_control) {
            return Err(SocialError::InvalidInput("tags"));
        }
    }
    Ok(())
}
