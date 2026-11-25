//! Platform-owned private group conversations with explicit membership.
//!
//! Group data lives in `platform.db`. Membership is enforced by the Platform;
//! a Core that is not a member cannot list, read, or post to a group. The owner
//! retains administrative access to inspect and manage groups.

use std::path::Path;

use aegis_contracts::core::canonicalize_core_id;
use aegis_storage::{now_rfc3339, open_initialized, StoragePaths, PLATFORM_APPLICATION_ID};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use crate::error::SocialError;

/// Schema identifier for a group.
pub const GROUP_SCHEMA: &str = "aegis.social.group.v1";
/// Schema identifier for a group member.
pub const GROUP_MEMBER_SCHEMA: &str = "aegis.social.group_member.v1";
/// Schema identifier for a group message.
pub const GROUP_MESSAGE_SCHEMA: &str = "aegis.social.group_message.v1";
/// Schema identifier for a group event.
pub const GROUP_EVENT_SCHEMA: &str = "aegis.social.group_event.v1";
/// Maximum group name length.
pub const MAX_GROUP_NAME_CHARS: usize = 160;
/// Maximum message length in characters.
pub const MAX_GROUP_MESSAGE_CHARS: usize = 16 * 1024;
/// Maximum messages returned by one read.
pub const MAX_GROUP_MESSAGES: usize = 500;

const GROUP_COMPONENT: &str = "social_groups";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MemberKind {
    Owner,
    Core,
}

impl MemberKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Owner => "owner",
            Self::Core => "core",
        }
    }

    pub fn parse(value: &str) -> Result<Self, SocialError> {
        match value {
            "owner" => Ok(Self::Owner),
            "core" => Ok(Self::Core),
            _ => Err(SocialError::InvalidInput("member kind")),
        }
    }
}

/// One actor requesting a group operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SocialActor {
    pub kind: MemberKind,
    pub id: String,
}

impl SocialActor {
    pub fn owner() -> Self {
        Self {
            kind: MemberKind::Owner,
            id: "owner".to_owned(),
        }
    }

    pub fn core(core_uuid: &str) -> Result<Self, SocialError> {
        let id = canonicalize_core_id(core_uuid).map_err(|_| SocialError::InvalidCoreUuid)?;
        Ok(Self {
            kind: MemberKind::Core,
            id,
        })
    }

    pub fn is_owner(&self) -> bool {
        self.kind == MemberKind::Owner
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GroupMember {
    pub schema: String,
    pub member_kind: MemberKind,
    pub member_id: String,
    pub joined_at: String,
    pub invited_by: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SocialGroup {
    pub schema: String,
    pub group_id: String,
    pub name: String,
    pub visibility: String,
    pub created_by: String,
    pub created_at: String,
    pub updated_at: String,
    pub revision: u64,
    pub member_count: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GroupMessage {
    pub schema: String,
    pub message_id: u64,
    pub group_id: String,
    pub sender_kind: MemberKind,
    pub sender_id: String,
    pub content: String,
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GroupEvent {
    pub schema: String,
    pub group_id: String,
    pub event_type: String,
    pub detail: Value,
    pub created_at: String,
}

/// Platform-owned group store.
#[derive(Debug, Clone)]
pub struct GroupStore {
    paths: StoragePaths,
}

impl GroupStore {
    pub fn new(paths: StoragePaths) -> Self {
        Self { paths }
    }

    pub fn initialize(&self) -> Result<(), SocialError> {
        self.open().map(|_| ())
    }

    pub fn create_group(
        &self,
        name: &str,
        actor: &SocialActor,
    ) -> Result<SocialGroup, SocialError> {
        let name = validate_text(name, MAX_GROUP_NAME_CHARS, "group name")?;
        validate_actor(actor)?;
        let group_id = Uuid::new_v4().to_string();
        let now = now_rfc3339()?;
        let connection = self.open()?;
        let transaction = connection.unchecked_transaction()?;
        transaction.execute(
            "INSERT INTO social_groups(
                group_id, schema, name, visibility, created_by, created_at, updated_at, revision
             ) VALUES(?1, ?2, ?3, 'private', ?4, ?5, ?5, 1)",
            params![group_id, GROUP_SCHEMA, name, actor.id, now],
        )?;
        transaction.execute(
            "INSERT INTO social_group_members(
                group_id, schema, member_kind, member_id, joined_at, invited_by
             ) VALUES(?1, ?2, ?3, ?4, ?5, 'platform')",
            params![
                group_id,
                GROUP_MEMBER_SCHEMA,
                actor.kind.as_str(),
                actor.id,
                now
            ],
        )?;
        insert_event(
            &transaction,
            &group_id,
            "group.created",
            &serde_json::json!({"created_by": actor.id}),
            &now,
        )?;
        transaction.commit()?;
        self.group(&group_id, actor)
    }

    /// Read one group; the owner may read any group, a Core must be a member.
    pub fn group(&self, group_id: &str, actor: &SocialActor) -> Result<SocialGroup, SocialError> {
        validate_actor(actor)?;
        let connection = self.open()?;
        require_group(&connection, group_id)?;
        require_access(&connection, group_id, actor)?;
        read_group(&connection, group_id)
    }

    /// Groups visible to the actor: all groups for the owner, joined groups for
    /// a Core.
    pub fn list_groups(&self, actor: &SocialActor) -> Result<Vec<SocialGroup>, SocialError> {
        validate_actor(actor)?;
        let connection = self.open()?;
        let mut statement = connection.prepare(
            "SELECT g.group_id, g.name, g.visibility, g.created_by, g.created_at,
                    g.updated_at, g.revision,
                    (SELECT COUNT(*) FROM social_group_members m WHERE m.group_id = g.group_id)
             FROM social_groups g
             WHERE ?1 = 'owner' OR EXISTS (
                 SELECT 1 FROM social_group_members m
                 WHERE m.group_id = g.group_id
                   AND m.member_kind = ?2
                   AND m.member_id = ?3
             )
             ORDER BY g.created_at ASC, g.group_id ASC",
        )?;
        let mut rows =
            statement.query(params![actor.kind.as_str(), actor.kind.as_str(), actor.id])?;
        let mut groups = Vec::new();
        while let Some(row) = rows.next()? {
            groups.push(read_group_row(row)?);
        }
        Ok(groups)
    }

    /// Add a member. Only the owner may administer membership.
    pub fn add_member(
        &self,
        group_id: &str,
        member_kind: MemberKind,
        member_id: &str,
        actor: &SocialActor,
    ) -> Result<Vec<GroupMember>, SocialError> {
        if !actor.is_owner() {
            return Err(SocialError::NotAuthorized);
        }
        let member_id = normalize_member(member_kind, member_id)?;
        let connection = self.open()?;
        require_group(&connection, group_id)?;
        let existing: Option<String> = connection
            .query_row(
                "SELECT member_id FROM social_group_members
                 WHERE group_id = ?1 AND member_kind = ?2 AND member_id = ?3",
                params![group_id, member_kind.as_str(), member_id],
                |row| row.get(0),
            )
            .optional()?;
        if existing.is_some() {
            return Err(SocialError::DuplicateMember);
        }
        let now = now_rfc3339()?;
        connection.execute(
            "INSERT INTO social_group_members(
                group_id, schema, member_kind, member_id, joined_at, invited_by
             ) VALUES(?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                group_id,
                GROUP_MEMBER_SCHEMA,
                member_kind.as_str(),
                member_id,
                now,
                actor.id
            ],
        )?;
        touch_group(&connection, group_id, &now)?;
        insert_event(
            &connection,
            group_id,
            "member.added",
            &serde_json::json!({"member_kind": member_kind.as_str(), "member_id": member_id}),
            &now,
        )?;
        self.members(group_id, actor)
    }

    /// Remove a member. Only the owner may administer membership.
    pub fn remove_member(
        &self,
        group_id: &str,
        member_kind: MemberKind,
        member_id: &str,
        actor: &SocialActor,
    ) -> Result<Vec<GroupMember>, SocialError> {
        if !actor.is_owner() {
            return Err(SocialError::NotAuthorized);
        }
        let member_id = normalize_member(member_kind, member_id)?;
        let connection = self.open()?;
        require_group(&connection, group_id)?;
        let changed = connection.execute(
            "DELETE FROM social_group_members
             WHERE group_id = ?1 AND member_kind = ?2 AND member_id = ?3",
            params![group_id, member_kind.as_str(), member_id],
        )?;
        if changed == 0 {
            return Err(SocialError::MemberNotFound);
        }
        let now = now_rfc3339()?;
        touch_group(&connection, group_id, &now)?;
        insert_event(
            &connection,
            group_id,
            "member.removed",
            &serde_json::json!({"member_kind": member_kind.as_str(), "member_id": member_id}),
            &now,
        )?;
        self.members(group_id, actor)
    }

    pub fn members(
        &self,
        group_id: &str,
        actor: &SocialActor,
    ) -> Result<Vec<GroupMember>, SocialError> {
        validate_actor(actor)?;
        let connection = self.open()?;
        require_group(&connection, group_id)?;
        require_access(&connection, group_id, actor)?;
        let mut statement = connection.prepare(
            "SELECT member_kind, member_id, joined_at, invited_by
             FROM social_group_members WHERE group_id = ?1
             ORDER BY member_kind ASC, member_id ASC",
        )?;
        let mut rows = statement.query([group_id])?;
        let mut members = Vec::new();
        while let Some(row) = rows.next()? {
            let kind: String = row.get(0)?;
            members.push(GroupMember {
                schema: GROUP_MEMBER_SCHEMA.to_owned(),
                member_kind: MemberKind::parse(&kind)?,
                member_id: row.get(1)?,
                joined_at: row.get(2)?,
                invited_by: row.get(3)?,
            });
        }
        Ok(members)
    }

    pub fn post_message(
        &self,
        group_id: &str,
        actor: &SocialActor,
        content: &str,
    ) -> Result<GroupMessage, SocialError> {
        validate_actor(actor)?;
        let content = validate_text(content, MAX_GROUP_MESSAGE_CHARS, "message")?;
        let connection = self.open()?;
        require_group(&connection, group_id)?;
        require_access(&connection, group_id, actor)?;
        let now = now_rfc3339()?;
        connection.execute(
            "INSERT INTO social_group_messages(
                schema, group_id, sender_kind, sender_id, content, created_at
             ) VALUES(?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                GROUP_MESSAGE_SCHEMA,
                group_id,
                actor.kind.as_str(),
                actor.id,
                content,
                now
            ],
        )?;
        let message_id = u64::try_from(connection.last_insert_rowid())
            .map_err(|_| SocialError::CorruptData("message id"))?;
        touch_group(&connection, group_id, &now)?;
        insert_event(
            &connection,
            group_id,
            "message.posted",
            &serde_json::json!({
                "message_id": message_id,
                "sender_kind": actor.kind.as_str(),
            }),
            &now,
        )?;
        self.message(group_id, message_id, actor)
    }

    pub fn message(
        &self,
        group_id: &str,
        message_id: u64,
        actor: &SocialActor,
    ) -> Result<GroupMessage, SocialError> {
        validate_actor(actor)?;
        let connection = self.open()?;
        require_group(&connection, group_id)?;
        require_access(&connection, group_id, actor)?;
        connection
            .query_row(
                "SELECT message_id, sender_kind, sender_id, content, created_at
                 FROM social_group_messages WHERE group_id = ?1 AND message_id = ?2",
                params![group_id, message_id as i64],
                |row| read_message(row, group_id),
            )
            .optional()?
            .ok_or(SocialError::CorruptData("message not found"))
    }

    pub fn messages(
        &self,
        group_id: &str,
        actor: &SocialActor,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<GroupMessage>, SocialError> {
        validate_actor(actor)?;
        let connection = self.open()?;
        require_group(&connection, group_id)?;
        require_access(&connection, group_id, actor)?;
        let limit = limit.clamp(1, MAX_GROUP_MESSAGES);
        let mut statement = connection.prepare(
            "SELECT message_id, sender_kind, sender_id, content, created_at
             FROM social_group_messages WHERE group_id = ?1
             ORDER BY message_id ASC LIMIT ?2 OFFSET ?3",
        )?;
        let mut rows = statement.query(params![group_id, limit as i64, offset as i64])?;
        let mut messages = Vec::new();
        while let Some(row) = rows.next()? {
            messages.push(read_message(row, group_id)?);
        }
        Ok(messages)
    }

    pub fn events(
        &self,
        group_id: &str,
        actor: &SocialActor,
    ) -> Result<Vec<GroupEvent>, SocialError> {
        validate_actor(actor)?;
        let connection = self.open()?;
        require_group(&connection, group_id)?;
        require_access(&connection, group_id, actor)?;
        let mut statement = connection.prepare(
            "SELECT event_type, detail_json, created_at FROM social_group_events
             WHERE group_id = ?1 ORDER BY event_id ASC",
        )?;
        let mut rows = statement.query([group_id])?;
        let mut events = Vec::new();
        while let Some(row) = rows.next()? {
            let detail_json: String = row.get(1)?;
            events.push(GroupEvent {
                schema: GROUP_EVENT_SCHEMA.to_owned(),
                group_id: group_id.to_owned(),
                event_type: row.get(0)?,
                detail: serde_json::from_str(&detail_json)?,
                created_at: row.get(2)?,
            });
        }
        Ok(events)
    }

    pub fn is_member(&self, group_id: &str, actor: &SocialActor) -> Result<bool, SocialError> {
        if actor.is_owner() {
            return Ok(true);
        }
        let connection = self.open()?;
        require_group(&connection, group_id)?;
        member_exists(&connection, group_id, actor)
    }

    fn open(&self) -> Result<Connection, SocialError> {
        if !self.paths.platform_db().is_file() {
            return Err(SocialError::NotInitialized(
                self.paths.root().display().to_string(),
            ));
        }
        let connection = open_initialized(
            &self.paths.platform_db(),
            PLATFORM_APPLICATION_ID,
            GROUP_COMPONENT,
        )?;
        ensure_tables(&connection)?;
        Ok(connection)
    }
}

fn ensure_tables(connection: &Connection) -> Result<(), SocialError> {
    connection.execute_batch(
        "CREATE TABLE IF NOT EXISTS social_groups (
            group_id TEXT PRIMARY KEY,
            schema TEXT NOT NULL,
            name TEXT NOT NULL,
            visibility TEXT NOT NULL CHECK(visibility IN ('private')),
            created_by TEXT NOT NULL,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            revision INTEGER NOT NULL CHECK(revision >= 1)
        );
        CREATE TABLE IF NOT EXISTS social_group_members (
            group_id TEXT NOT NULL REFERENCES social_groups(group_id) ON DELETE RESTRICT,
            schema TEXT NOT NULL,
            member_kind TEXT NOT NULL CHECK(member_kind IN ('owner', 'core')),
            member_id TEXT NOT NULL,
            joined_at TEXT NOT NULL,
            invited_by TEXT NOT NULL,
            PRIMARY KEY(group_id, member_kind, member_id)
        );
        CREATE TABLE IF NOT EXISTS social_group_messages (
            message_id INTEGER PRIMARY KEY AUTOINCREMENT,
            schema TEXT NOT NULL,
            group_id TEXT NOT NULL REFERENCES social_groups(group_id) ON DELETE RESTRICT,
            sender_kind TEXT NOT NULL CHECK(sender_kind IN ('owner', 'core')),
            sender_id TEXT NOT NULL,
            content TEXT NOT NULL,
            created_at TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS social_group_messages_by_group
            ON social_group_messages(group_id, message_id);
        CREATE TABLE IF NOT EXISTS social_group_events (
            event_id INTEGER PRIMARY KEY AUTOINCREMENT,
            schema TEXT NOT NULL,
            group_id TEXT NOT NULL REFERENCES social_groups(group_id) ON DELETE RESTRICT,
            event_type TEXT NOT NULL,
            detail_json TEXT NOT NULL,
            created_at TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS social_group_events_by_group
            ON social_group_events(group_id, event_id);
        CREATE TRIGGER IF NOT EXISTS social_group_messages_no_update
        BEFORE UPDATE ON social_group_messages
        BEGIN
            SELECT RAISE(ABORT, 'group messages are immutable');
        END;
        CREATE TRIGGER IF NOT EXISTS social_group_messages_no_delete
        BEFORE DELETE ON social_group_messages
        BEGIN
            SELECT RAISE(ABORT, 'group messages are immutable');
        END;
        CREATE TRIGGER IF NOT EXISTS social_group_events_no_update
        BEFORE UPDATE ON social_group_events
        BEGIN
            SELECT RAISE(ABORT, 'group events are append-only');
        END;
        CREATE TRIGGER IF NOT EXISTS social_group_events_no_delete
        BEFORE DELETE ON social_group_events
        BEGIN
            SELECT RAISE(ABORT, 'group events are append-only');
        END;",
    )?;
    Ok(())
}

fn require_group(connection: &Connection, group_id: &str) -> Result<(), SocialError> {
    let exists: Option<String> = connection
        .query_row(
            "SELECT group_id FROM social_groups WHERE group_id = ?1",
            [group_id],
            |row| row.get(0),
        )
        .optional()?;
    exists
        .map(|_| ())
        .ok_or_else(|| SocialError::GroupNotFound(group_id.to_owned()))
}

fn require_access(
    connection: &Connection,
    group_id: &str,
    actor: &SocialActor,
) -> Result<(), SocialError> {
    if actor.is_owner() || member_exists(connection, group_id, actor)? {
        Ok(())
    } else {
        Err(SocialError::NotMember)
    }
}

fn member_exists(
    connection: &Connection,
    group_id: &str,
    actor: &SocialActor,
) -> Result<bool, SocialError> {
    let found: Option<String> = connection
        .query_row(
            "SELECT member_id FROM social_group_members
             WHERE group_id = ?1 AND member_kind = ?2 AND member_id = ?3",
            params![group_id, actor.kind.as_str(), actor.id],
            |row| row.get(0),
        )
        .optional()?;
    Ok(found.is_some())
}

fn read_group(connection: &Connection, group_id: &str) -> Result<SocialGroup, SocialError> {
    connection
        .query_row(
            "SELECT g.group_id, g.name, g.visibility, g.created_by, g.created_at,
                    g.updated_at, g.revision,
                    (SELECT COUNT(*) FROM social_group_members m WHERE m.group_id = g.group_id)
             FROM social_groups g WHERE g.group_id = ?1",
            [group_id],
            read_group_row,
        )
        .optional()?
        .ok_or_else(|| SocialError::GroupNotFound(group_id.to_owned()))
}

fn read_group_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<SocialGroup> {
    let revision: i64 = row.get(6)?;
    let member_count: i64 = row.get(7)?;
    if revision < 1 || member_count < 0 {
        return Err(rusqlite::Error::InvalidColumnType(
            6,
            "group bounds".to_owned(),
            rusqlite::types::Type::Integer,
        ));
    }
    Ok(SocialGroup {
        schema: GROUP_SCHEMA.to_owned(),
        group_id: row.get(0)?,
        name: row.get(1)?,
        visibility: row.get(2)?,
        created_by: row.get(3)?,
        created_at: row.get(4)?,
        updated_at: row.get(5)?,
        revision: revision as u64,
        member_count: member_count as u64,
    })
}

fn read_message(row: &rusqlite::Row<'_>, group_id: &str) -> rusqlite::Result<GroupMessage> {
    let kind: String = row.get(1)?;
    let sender_kind = MemberKind::parse(&kind).map_err(|_| {
        rusqlite::Error::FromSqlConversionFailure(
            1,
            rusqlite::types::Type::Text,
            Box::new(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "invalid sender kind",
            )),
        )
    })?;
    let message_id: i64 = row.get(0)?;
    Ok(GroupMessage {
        schema: GROUP_MESSAGE_SCHEMA.to_owned(),
        message_id: u64::try_from(message_id).map_err(|_| {
            rusqlite::Error::InvalidColumnType(
                0,
                "message_id".to_owned(),
                rusqlite::types::Type::Integer,
            )
        })?,
        group_id: group_id.to_owned(),
        sender_kind,
        sender_id: row.get(2)?,
        content: row.get(3)?,
        created_at: row.get(4)?,
    })
}

fn touch_group(connection: &Connection, group_id: &str, now: &str) -> Result<(), SocialError> {
    connection.execute(
        "UPDATE social_groups
         SET updated_at = ?1, revision = revision + 1 WHERE group_id = ?2",
        params![now, group_id],
    )?;
    Ok(())
}

fn insert_event(
    connection: &Connection,
    group_id: &str,
    event_type: &str,
    detail: &Value,
    now: &str,
) -> Result<(), SocialError> {
    connection.execute(
        "INSERT INTO social_group_events(
            schema, group_id, event_type, detail_json, created_at
         ) VALUES(?1, ?2, ?3, ?4, ?5)",
        params![
            GROUP_EVENT_SCHEMA,
            group_id,
            event_type,
            serde_json::to_string(detail)?,
            now
        ],
    )?;
    Ok(())
}

fn validate_actor(actor: &SocialActor) -> Result<(), SocialError> {
    match actor.kind {
        MemberKind::Owner => {
            if actor.id == "owner" {
                Ok(())
            } else {
                Err(SocialError::InvalidInput("owner id"))
            }
        }
        MemberKind::Core => {
            let canonical =
                canonicalize_core_id(&actor.id).map_err(|_| SocialError::InvalidCoreUuid)?;
            if canonical == actor.id {
                Ok(())
            } else {
                Err(SocialError::InvalidCoreUuid)
            }
        }
    }
}

fn normalize_member(kind: MemberKind, member_id: &str) -> Result<String, SocialError> {
    match kind {
        MemberKind::Owner => {
            if member_id == "owner" {
                Ok("owner".to_owned())
            } else {
                Err(SocialError::InvalidInput("owner id"))
            }
        }
        MemberKind::Core => {
            canonicalize_core_id(member_id).map_err(|_| SocialError::InvalidCoreUuid)
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

/// Convenience helper for callers that already have a root path.
pub fn group_store(root: &Path) -> Result<GroupStore, SocialError> {
    let paths = StoragePaths::new(root)?;
    let store = GroupStore::new(paths);
    store.initialize()?;
    Ok(store)
}
