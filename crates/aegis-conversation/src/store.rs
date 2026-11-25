//! Per-Core conversation persistence.
//!
//! Conversations live in `<root>/cores/<core_uuid>/conversation.db`, owned by
//! exactly one Core and bound to that Core's immutable UUID and database
//! instance UUID. Conversations are never stored in the Platform or Safe
//! databases, and one Core's store cannot be opened against another Core.

use std::path::PathBuf;

use aegis_contracts::core::canonicalize_core_id;
use aegis_core_storage::{CoreDatabase, CoreStorageLayout};
use aegis_storage::{now_rfc3339, open_initialized};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use crate::error::ConversationError;

/// File name of the per-Core conversation database.
pub const CONVERSATION_DB_FILE: &str = "conversation.db";
/// SQLite application marker for a conversation database ("AEGX").
pub const CONVERSATION_APPLICATION_ID: i64 = 0x4145_4758;
/// Component name registered in the shared `aegis_schema` table.
pub const CONVERSATION_COMPONENT: &str = "conversation";
/// Schema identifier for a persisted session.
pub const SESSION_SCHEMA: &str = "aegis.conversation.session.v1";
/// Schema identifier for a persisted message.
pub const MESSAGE_SCHEMA: &str = "aegis.conversation.message.v1";
/// Schema identifier for a session lifecycle event.
pub const SESSION_EVENT_SCHEMA: &str = "aegis.conversation.event.v1";
/// Schema identifier for a single-session export.
pub const EXPORT_SCHEMA: &str = "aegis.conversation.export.v1";
/// Schema identifier for a full-Core conversation archive.
pub const ARCHIVE_SCHEMA: &str = "aegis.conversation.archive.v1";

/// Maximum title length in characters.
pub const MAX_TITLE_CHARS: usize = 200;
/// Maximum message length in characters.
pub const MAX_MESSAGE_CHARS: usize = 65_536;
/// Maximum message metadata size in bytes.
pub const MAX_METADATA_BYTES: usize = 16 * 1024;
/// Maximum messages returned by one read.
pub const MAX_MESSAGES_PER_READ: usize = 500;
/// Maximum sessions exported at once.
pub const MAX_SESSIONS_PER_ARCHIVE: usize = 500;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionState {
    Active,
    Archived,
    Deleted,
}

impl SessionState {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Archived => "archived",
            Self::Deleted => "deleted",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "active" => Some(Self::Active),
            "archived" => Some(Self::Archived),
            "deleted" => Some(Self::Deleted),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MessageRole {
    System,
    User,
    Assistant,
    Tool,
}

impl MessageRole {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::User => "user",
            Self::Assistant => "assistant",
            Self::Tool => "tool",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "system" => Some(Self::System),
            "user" => Some(Self::User),
            "assistant" => Some(Self::Assistant),
            "tool" => Some(Self::Tool),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConversationSession {
    pub schema: String,
    pub session_id: String,
    pub core_uuid: String,
    pub title: String,
    pub state: SessionState,
    pub provider_id: Option<String>,
    pub model: Option<String>,
    pub message_count: u64,
    pub created_at: String,
    pub updated_at: String,
    pub revision: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConversationMessage {
    pub schema: String,
    pub message_id: u64,
    pub session_id: String,
    pub role: MessageRole,
    pub content: String,
    pub provider_id: Option<String>,
    pub model: Option<String>,
    pub prompt_tokens: Option<u64>,
    pub completion_tokens: Option<u64>,
    pub metadata: Option<Value>,
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionEvent {
    pub schema: String,
    pub event_type: String,
    pub detail: Value,
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConversationExport {
    pub schema: String,
    pub core_uuid: String,
    pub session: ConversationSession,
    pub messages: Vec<ConversationMessage>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConversationArchive {
    pub schema: String,
    pub core_uuid: String,
    pub exported_at: String,
    pub sessions: Vec<ConversationExport>,
}

#[derive(Debug, Clone)]
pub struct NewMessage<'a> {
    pub role: MessageRole,
    pub content: &'a str,
    pub provider_id: Option<&'a str>,
    pub model: Option<&'a str>,
    pub prompt_tokens: Option<u64>,
    pub completion_tokens: Option<u64>,
    pub metadata: Option<&'a Value>,
}

impl<'a> NewMessage<'a> {
    pub fn new(role: MessageRole, content: &'a str) -> Self {
        Self {
            role,
            content,
            provider_id: None,
            model: None,
            prompt_tokens: None,
            completion_tokens: None,
            metadata: None,
        }
    }
}

/// Persisted conversation store for one Core.
#[derive(Debug)]
pub struct ConversationStore {
    connection: Connection,
    core_uuid: String,
    instance_uuid: String,
    path: PathBuf,
}

impl ConversationStore {
    /// Open (or lazily create) the conversation database for an existing Core.
    pub fn open(
        root: impl AsRef<std::path::Path>,
        core_uuid: &str,
    ) -> Result<Self, ConversationError> {
        let core_uuid =
            canonicalize_core_id(core_uuid).map_err(|_| ConversationError::InvalidCoreUuid)?;
        let layout = CoreStorageLayout::new(root.as_ref())?;
        let snapshot = CoreDatabase::read_snapshot(&layout, &core_uuid)?;
        if snapshot.identity.core_uuid != core_uuid {
            return Err(ConversationError::OwnershipMismatch);
        }
        let directory = layout.core_dir(&core_uuid)?;
        let path = directory.join(CONVERSATION_DB_FILE);
        if path.parent() != Some(directory.as_path()) {
            return Err(ConversationError::ForeignPath);
        }
        let connection =
            open_initialized(&path, CONVERSATION_APPLICATION_ID, CONVERSATION_COMPONENT)?;
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

    pub fn path(&self) -> &std::path::Path {
        &self.path
    }

    pub fn create_session(
        &self,
        title: Option<&str>,
        provider_id: Option<&str>,
        model: Option<&str>,
    ) -> Result<ConversationSession, ConversationError> {
        let session_id = Uuid::new_v4().to_string();
        let title = match title {
            Some(value) => validate_text(value, MAX_TITLE_CHARS, "session title")?,
            None => format!("Conversation {}", &session_id[..8]),
        };
        let provider_id = provider_id
            .map(|value| validate_text(value, 128, "provider id"))
            .transpose()?;
        let model = model
            .map(|value| validate_text(value, 200, "model"))
            .transpose()?;
        let created_at = now_rfc3339()?;
        self.connection.execute(
            "INSERT INTO conversation_sessions(
                session_id, core_uuid, title, state, provider_id, model,
                message_count, created_at, updated_at, revision
             ) VALUES(?1, ?2, ?3, 'active', ?4, ?5, 0, ?6, ?6, 1)",
            params![
                session_id,
                self.core_uuid,
                title,
                provider_id,
                model,
                created_at
            ],
        )?;
        self.record_event(
            &session_id,
            "session.created",
            &serde_json::json!({"title": title}),
        )?;
        self.session(&session_id)
    }

    pub fn list_sessions(
        &self,
        include_archived: bool,
        include_deleted: bool,
    ) -> Result<Vec<ConversationSession>, ConversationError> {
        let mut statement = self.connection.prepare(
            "SELECT session_id, core_uuid, title, state, provider_id, model,
                    message_count, created_at, updated_at, revision
             FROM conversation_sessions
             WHERE core_uuid = ?1
               AND (?2 = 1 OR state <> 'archived')
               AND (?3 = 1 OR state <> 'deleted')
             ORDER BY updated_at DESC, session_id ASC",
        )?;
        let mut rows = statement.query(params![
            self.core_uuid,
            i64::from(u8::from(include_archived)),
            i64::from(u8::from(include_deleted))
        ])?;
        let mut sessions = Vec::new();
        while let Some(row) = rows.next()? {
            sessions.push(read_session(row)?);
        }
        Ok(sessions)
    }

    pub fn session(&self, session_id: &str) -> Result<ConversationSession, ConversationError> {
        validate_session_id(session_id)?;
        self.connection
            .query_row(
                "SELECT session_id, core_uuid, title, state, provider_id, model,
                        message_count, created_at, updated_at, revision
                 FROM conversation_sessions WHERE session_id = ?1 AND core_uuid = ?2",
                params![session_id, self.core_uuid],
                read_session,
            )
            .optional()?
            .ok_or_else(|| ConversationError::SessionNotFound(session_id.to_owned()))
    }

    /// Archive a session while retaining every message.
    pub fn archive_session(
        &self,
        session_id: &str,
    ) -> Result<ConversationSession, ConversationError> {
        self.transition(session_id, SessionState::Archived, "session.archived")
    }

    /// Soft-delete a session while retaining its history for owner export.
    pub fn delete_session(
        &self,
        session_id: &str,
    ) -> Result<ConversationSession, ConversationError> {
        self.transition(session_id, SessionState::Deleted, "session.deleted")
    }

    pub fn append_message(
        &self,
        session_id: &str,
        message: NewMessage<'_>,
    ) -> Result<ConversationMessage, ConversationError> {
        let session = self.session(session_id)?;
        if session.state != SessionState::Active {
            return Err(ConversationError::SessionNotActive(session_id.to_owned()));
        }
        validate_text(message.content, MAX_MESSAGE_CHARS, "message content")?;
        let metadata_json = match message.metadata {
            Some(value) => {
                let encoded = serde_json::to_string(value)?;
                if encoded.len() > MAX_METADATA_BYTES {
                    return Err(ConversationError::InvalidInput("message metadata"));
                }
                Some(encoded)
            }
            None => None,
        };
        let created_at = now_rfc3339()?;

        let transaction = self.connection.unchecked_transaction()?;
        transaction.execute(
            "INSERT INTO conversation_messages(
                session_id, role, content, provider_id, model,
                prompt_tokens, completion_tokens, metadata_json, created_at
             ) VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                session_id,
                message.role.as_str(),
                message.content,
                message.provider_id,
                message.model,
                message.prompt_tokens.map(|value| value as i64),
                message.completion_tokens.map(|value| value as i64),
                metadata_json,
                created_at,
            ],
        )?;
        let message_id = u64::try_from(transaction.last_insert_rowid())
            .map_err(|_| ConversationError::CorruptData("message id"))?;
        transaction.execute(
            "UPDATE conversation_sessions
             SET message_count = message_count + 1, updated_at = ?1, revision = revision + 1
             WHERE session_id = ?2",
            params![created_at, session_id],
        )?;
        transaction.execute(
            "INSERT INTO conversation_events(session_id, event_type, detail_json, created_at)
             VALUES(?1, 'message.appended', ?2, ?3)",
            params![
                session_id,
                serde_json::to_string(&serde_json::json!({
                    "message_id": message_id,
                    "role": message.role.as_str(),
                }))?,
                created_at,
            ],
        )?;
        transaction.commit()?;

        self.message(message_id)
    }

    pub fn message(&self, message_id: u64) -> Result<ConversationMessage, ConversationError> {
        let row = self
            .connection
            .query_row(
                &format!(
                    "SELECT {MESSAGE_COLUMNS} FROM conversation_messages
                     WHERE message_id = ?1 AND session_id IN (
                         SELECT session_id FROM conversation_sessions WHERE core_uuid = ?2
                     )"
                ),
                params![message_id as i64, self.core_uuid],
                read_message,
            )
            .optional()?;
        row.ok_or_else(|| ConversationError::CorruptData("message not found"))
    }

    pub fn messages(
        &self,
        session_id: &str,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<ConversationMessage>, ConversationError> {
        self.session(session_id)?;
        let limit = limit.clamp(1, MAX_MESSAGES_PER_READ);
        let mut statement = self.connection.prepare(&format!(
            "SELECT {MESSAGE_COLUMNS} FROM conversation_messages
             WHERE session_id = ?1 ORDER BY message_id ASC LIMIT ?2 OFFSET ?3"
        ))?;
        let mut rows = statement.query(params![session_id, limit as i64, offset as i64])?;
        let mut messages = Vec::new();
        while let Some(row) = rows.next()? {
            messages.push(read_message(row)?);
        }
        Ok(messages)
    }

    /// Most recent `limit` messages in ascending order.
    pub fn recent_messages(
        &self,
        session_id: &str,
        limit: usize,
    ) -> Result<Vec<ConversationMessage>, ConversationError> {
        self.session(session_id)?;
        let limit = limit.clamp(1, MAX_MESSAGES_PER_READ);
        let mut statement = self.connection.prepare(&format!(
            "SELECT {MESSAGE_COLUMNS} FROM (
                SELECT {MESSAGE_COLUMNS} FROM conversation_messages
                WHERE session_id = ?1 ORDER BY message_id DESC LIMIT ?2
             ) ORDER BY message_id ASC"
        ))?;
        let mut rows = statement.query(params![session_id, limit as i64])?;
        let mut messages = Vec::new();
        while let Some(row) = rows.next()? {
            messages.push(read_message(row)?);
        }
        Ok(messages)
    }

    pub fn last_user_message(
        &self,
        session_id: &str,
    ) -> Result<Option<ConversationMessage>, ConversationError> {
        self.session(session_id)?;
        self.connection
            .query_row(
                &format!(
                    "SELECT {MESSAGE_COLUMNS} FROM conversation_messages
                     WHERE session_id = ?1 AND role = 'user'
                     ORDER BY message_id DESC LIMIT 1"
                ),
                [session_id],
                read_message,
            )
            .optional()
            .map_err(ConversationError::from)
    }

    pub fn session_events(&self, session_id: &str) -> Result<Vec<SessionEvent>, ConversationError> {
        self.session(session_id)?;
        let mut statement = self.connection.prepare(
            "SELECT event_type, detail_json, created_at FROM conversation_events
             WHERE session_id = ?1 ORDER BY event_id ASC",
        )?;
        let mut rows = statement.query([session_id])?;
        let mut events = Vec::new();
        while let Some(row) = rows.next()? {
            let detail_json: String = row.get(1)?;
            events.push(SessionEvent {
                schema: SESSION_EVENT_SCHEMA.to_owned(),
                event_type: row.get(0)?,
                detail: serde_json::from_str(&detail_json)?,
                created_at: row.get(2)?,
            });
        }
        Ok(events)
    }

    /// Append a non-message session event (for example an inference failure).
    pub fn append_event(
        &self,
        session_id: &str,
        event_type: &str,
        detail: &Value,
    ) -> Result<(), ConversationError> {
        self.session(session_id)?;
        if event_type.trim().is_empty() || event_type.chars().count() > 128 {
            return Err(ConversationError::InvalidInput("event type"));
        }
        self.record_event(session_id, event_type, detail)
    }

    pub fn export_session(
        &self,
        session_id: &str,
    ) -> Result<ConversationExport, ConversationError> {
        let session = self.session(session_id)?;
        let messages = self.messages(session_id, MAX_MESSAGES_PER_READ, 0)?;
        Ok(ConversationExport {
            schema: EXPORT_SCHEMA.to_owned(),
            core_uuid: self.core_uuid.clone(),
            session,
            messages,
        })
    }

    pub fn export_archive(&self) -> Result<ConversationArchive, ConversationError> {
        let sessions = self.list_sessions(true, true)?;
        if sessions.len() > MAX_SESSIONS_PER_ARCHIVE {
            return Err(ConversationError::InvalidInput(
                "too many sessions to export",
            ));
        }
        let mut exports = Vec::with_capacity(sessions.len());
        for session in sessions {
            exports.push(self.export_session(&session.session_id)?);
        }
        Ok(ConversationArchive {
            schema: ARCHIVE_SCHEMA.to_owned(),
            core_uuid: self.core_uuid.clone(),
            exported_at: now_rfc3339()?,
            sessions: exports,
        })
    }

    fn transition(
        &self,
        session_id: &str,
        target: SessionState,
        event: &str,
    ) -> Result<ConversationSession, ConversationError> {
        let session = self.session(session_id)?;
        if session.state == target {
            return Ok(session);
        }
        if session.state == SessionState::Deleted {
            return Err(ConversationError::SessionNotActive(session_id.to_owned()));
        }
        let updated_at = now_rfc3339()?;
        let transaction = self.connection.unchecked_transaction()?;
        transaction.execute(
            "UPDATE conversation_sessions
             SET state = ?1, updated_at = ?2, revision = revision + 1
             WHERE session_id = ?3",
            params![target.as_str(), updated_at, session_id],
        )?;
        transaction.execute(
            "INSERT INTO conversation_events(session_id, event_type, detail_json, created_at)
             VALUES(?1, ?2, ?3, ?4)",
            params![
                session_id,
                event,
                serde_json::to_string(&serde_json::json!({
                    "from": session.state.as_str(),
                    "to": target.as_str(),
                }))?,
                updated_at,
            ],
        )?;
        transaction.commit()?;
        self.session(session_id)
    }

    fn record_event(
        &self,
        session_id: &str,
        event_type: &str,
        detail: &Value,
    ) -> Result<(), ConversationError> {
        self.connection.execute(
            "INSERT INTO conversation_events(session_id, event_type, detail_json, created_at)
             VALUES(?1, ?2, ?3, ?4)",
            params![
                session_id,
                event_type,
                serde_json::to_string(detail)?,
                now_rfc3339()?
            ],
        )?;
        Ok(())
    }
}

fn verify_ownership(
    connection: &Connection,
    core_uuid: &str,
    instance_uuid: &str,
) -> Result<(), ConversationError> {
    let existing: Option<(String, String)> = connection
        .query_row(
            "SELECT core_uuid, database_instance_uuid FROM conversation_ownership
             WHERE singleton = 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    match existing {
        None => {
            connection.execute(
                "INSERT INTO conversation_ownership(
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
        Some(_) => Err(ConversationError::OwnershipMismatch),
    }
}

fn ensure_tables(connection: &Connection) -> Result<(), ConversationError> {
    connection.execute_batch(
        "CREATE TABLE IF NOT EXISTS conversation_ownership (
            singleton INTEGER PRIMARY KEY CHECK(singleton = 1),
            core_uuid TEXT NOT NULL UNIQUE,
            database_instance_uuid TEXT NOT NULL,
            created_at TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS conversation_sessions (
            session_id TEXT PRIMARY KEY,
            core_uuid TEXT NOT NULL,
            title TEXT NOT NULL,
            state TEXT NOT NULL CHECK(state IN ('active', 'archived', 'deleted')),
            provider_id TEXT,
            model TEXT,
            message_count INTEGER NOT NULL CHECK(message_count >= 0),
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            revision INTEGER NOT NULL CHECK(revision >= 1)
        );
        CREATE INDEX IF NOT EXISTS conversation_sessions_by_core
            ON conversation_sessions(core_uuid, updated_at);
        CREATE TABLE IF NOT EXISTS conversation_messages (
            message_id INTEGER PRIMARY KEY AUTOINCREMENT,
            session_id TEXT NOT NULL REFERENCES conversation_sessions(session_id)
                ON DELETE RESTRICT,
            role TEXT NOT NULL CHECK(role IN ('system', 'user', 'assistant', 'tool')),
            content TEXT NOT NULL,
            provider_id TEXT,
            model TEXT,
            prompt_tokens INTEGER,
            completion_tokens INTEGER,
            metadata_json TEXT,
            created_at TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS conversation_messages_by_session
            ON conversation_messages(session_id, message_id);
        CREATE TABLE IF NOT EXISTS conversation_events (
            event_id INTEGER PRIMARY KEY AUTOINCREMENT,
            session_id TEXT NOT NULL REFERENCES conversation_sessions(session_id)
                ON DELETE RESTRICT,
            event_type TEXT NOT NULL,
            detail_json TEXT NOT NULL,
            created_at TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS conversation_events_by_session
            ON conversation_events(session_id, event_id);
        CREATE TRIGGER IF NOT EXISTS conversation_ownership_no_update
        BEFORE UPDATE ON conversation_ownership
        BEGIN
            SELECT RAISE(ABORT, 'conversation ownership is immutable');
        END;
        CREATE TRIGGER IF NOT EXISTS conversation_ownership_no_delete
        BEFORE DELETE ON conversation_ownership
        BEGIN
            SELECT RAISE(ABORT, 'conversation ownership is immutable');
        END;
        CREATE TRIGGER IF NOT EXISTS conversation_events_no_update
        BEFORE UPDATE ON conversation_events
        BEGIN
            SELECT RAISE(ABORT, 'conversation events are append-only');
        END;
        CREATE TRIGGER IF NOT EXISTS conversation_events_no_delete
        BEFORE DELETE ON conversation_events
        BEGIN
            SELECT RAISE(ABORT, 'conversation events are append-only');
        END;",
    )?;
    Ok(())
}

const MESSAGE_COLUMNS: &str = "message_id, session_id, role, content, provider_id, model, \
                              prompt_tokens, completion_tokens, metadata_json, created_at";

fn read_session(row: &rusqlite::Row<'_>) -> rusqlite::Result<ConversationSession> {
    let state: String = row.get(3)?;
    let state = SessionState::parse(&state).ok_or_else(|| {
        rusqlite::Error::FromSqlConversionFailure(
            3,
            rusqlite::types::Type::Text,
            Box::new(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "invalid session state",
            )),
        )
    })?;
    let message_count: i64 = row.get(6)?;
    let revision: i64 = row.get(9)?;
    if message_count < 0 || revision < 1 {
        return Err(rusqlite::Error::InvalidColumnType(
            6,
            "message_count".to_owned(),
            rusqlite::types::Type::Integer,
        ));
    }
    Ok(ConversationSession {
        schema: SESSION_SCHEMA.to_owned(),
        session_id: row.get(0)?,
        core_uuid: row.get(1)?,
        title: row.get(2)?,
        state,
        provider_id: row.get(4)?,
        model: row.get(5)?,
        message_count: message_count as u64,
        created_at: row.get(7)?,
        updated_at: row.get(8)?,
        revision: revision as u64,
    })
}

fn read_message(row: &rusqlite::Row<'_>) -> rusqlite::Result<ConversationMessage> {
    let role: String = row.get(2)?;
    let role = MessageRole::parse(&role).ok_or_else(|| {
        rusqlite::Error::FromSqlConversionFailure(
            2,
            rusqlite::types::Type::Text,
            Box::new(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "invalid message role",
            )),
        )
    })?;
    let message_id: i64 = row.get(0)?;
    let prompt_tokens: Option<i64> = row.get(6)?;
    let completion_tokens: Option<i64> = row.get(7)?;
    let metadata_json: Option<String> = row.get(8)?;
    let metadata = match metadata_json {
        Some(value) => Some(serde_json::from_str(&value).map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(
                8,
                rusqlite::types::Type::Text,
                Box::new(error),
            )
        })?),
        None => None,
    };
    Ok(ConversationMessage {
        schema: MESSAGE_SCHEMA.to_owned(),
        message_id: u64::try_from(message_id).map_err(|_| {
            rusqlite::Error::InvalidColumnType(
                0,
                "message_id".to_owned(),
                rusqlite::types::Type::Integer,
            )
        })?,
        session_id: row.get(1)?,
        role,
        content: row.get(3)?,
        provider_id: row.get(4)?,
        model: row.get(5)?,
        prompt_tokens: prompt_tokens.map(|value| value as u64),
        completion_tokens: completion_tokens.map(|value| value as u64),
        metadata,
        created_at: row.get(9)?,
    })
}

fn validate_session_id(value: &str) -> Result<(), ConversationError> {
    let parsed =
        Uuid::parse_str(value).map_err(|_| ConversationError::InvalidInput("session id"))?;
    if parsed.is_nil() {
        return Err(ConversationError::InvalidInput("session id"));
    }
    Ok(())
}

fn validate_text(
    value: &str,
    maximum: usize,
    _field: &'static str,
) -> Result<String, ConversationError> {
    let trimmed = value.trim();
    if trimmed.is_empty() || value.chars().count() > maximum || value.chars().any(char::is_control)
    {
        return Err(ConversationError::InvalidInput(_field));
    }
    Ok(value.to_owned())
}
