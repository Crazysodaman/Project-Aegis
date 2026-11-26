//! Versioned per-Core memory store with provenance, history, and isolation.

use std::collections::BTreeSet;
use std::ffi::OsStr;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use aegis_core_storage::{CoreDatabase, CoreStorageLayout};
use aegis_storage::open_initialized;
use rusqlite::{params, Connection, OptionalExtension, Transaction};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use uuid::Uuid;

use crate::personality::{PersonalityProfile, PERSONALITY_SCHEMA};
use crate::{
    canonical_core_id, chars_within, now_rfc3339, CoreCapability, SdkError, CORE_SDK_VERSION,
    MEMORY_APPLICATION_ID, MEMORY_COMPONENT, MEMORY_CONTRACT_SCHEMA, MEMORY_DB_FILE,
};

/// Schema identifier for a single memory event.
pub const MEMORY_EVENT_SCHEMA: &str = "aegis.core.memory.event.v1";
/// Schema identifier for a versioned memory export.
pub const MEMORY_EXPORT_SCHEMA: &str = "aegis.core.memory.export.v1";
/// Table holding the immutable Core ownership row.
pub const MEMORY_OWNERSHIP_TABLE: &str = "core_memory_ownership";
/// Table holding memory entries.
pub const MEMORY_ENTRIES_TABLE: &str = "memory_entries";
/// Table holding the append-only memory event log.
pub const MEMORY_EVENTS_TABLE: &str = "memory_events";
/// Table holding the current personality profile.
pub const PERSONALITY_TABLE: &str = "core_personality";
/// Table holding the append-only personality growth history.
pub const PERSONALITY_GROWTH_TABLE: &str = "personality_growth";

/// Maximum stored content size per entry (64 KiB).
pub const MAX_CONTENT_BYTES: usize = 64 * 1024;
/// Hard upper bound accepted for imported content (1 MiB); entries still must
/// satisfy [`MAX_CONTENT_BYTES`].
pub const MAX_IMPORT_CONTENT_BYTES: usize = 1024 * 1024;
/// Maximum character length of a memory source.
pub const MAX_SOURCE_CHARS: usize = 512;
/// Maximum character length of provenance metadata.
pub const MAX_PROVENANCE_CHARS: usize = 1024;
/// Maximum number of tags per entry.
pub const MAX_TAGS: usize = 32;
/// Maximum character length of a single tag.
pub const MAX_TAG_CHARS: usize = 128;
/// Maximum character length of a search string.
pub const MAX_QUERY_TEXT_CHARS: usize = 1024;
/// Maximum number of entries returned by one search.
pub const MAX_QUERY_LIMIT: usize = 500;
/// Maximum number of entries accepted by one import.
pub const MAX_IMPORT_ENTRIES: usize = 10_000;
/// Maximum number of events accepted by one import.
pub const MAX_IMPORT_EVENTS: usize = 100_000;
/// Maximum character length of an event detail field.
pub const MAX_EVENT_DETAIL_CHARS: usize = 2048;
/// Maximum character length of an `Other` memory kind.
pub const MAX_KIND_CHARS: usize = 64;

const ENTRY_COLUMNS: &str = "id, core_uuid, kind, content, source, provenance, confidence, \
     tags_json, created_at, updated_at, archived, deleted, revision";

const SCHEMA_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS core_memory_ownership (
    singleton INTEGER NOT NULL PRIMARY KEY CHECK (singleton = 1),
    core_uuid TEXT NOT NULL,
    schema TEXT NOT NULL,
    created_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS core_personality (
    core_uuid TEXT NOT NULL PRIMARY KEY,
    revision INTEGER NOT NULL CHECK (revision >= 1),
    profile_json TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS personality_growth (
    core_uuid TEXT NOT NULL,
    revision INTEGER NOT NULL CHECK (revision >= 1),
    profile_json TEXT NOT NULL,
    created_at TEXT NOT NULL,
    PRIMARY KEY (core_uuid, revision)
);
CREATE TABLE IF NOT EXISTS memory_entries (
    id TEXT NOT NULL PRIMARY KEY,
    core_uuid TEXT NOT NULL,
    kind TEXT NOT NULL,
    content TEXT NOT NULL,
    source TEXT NOT NULL,
    provenance TEXT NOT NULL,
    confidence REAL NOT NULL CHECK (confidence >= 0.0 AND confidence <= 1.0),
    tags_json TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    archived INTEGER NOT NULL CHECK (archived IN (0, 1)),
    deleted INTEGER NOT NULL CHECK (deleted IN (0, 1)),
    revision INTEGER NOT NULL CHECK (revision >= 1)
);
CREATE INDEX IF NOT EXISTS memory_entries_core_idx ON memory_entries(core_uuid);
CREATE INDEX IF NOT EXISTS memory_entries_core_kind_idx ON memory_entries(core_uuid, kind);
CREATE TABLE IF NOT EXISTS memory_events (
    sequence INTEGER NOT NULL PRIMARY KEY AUTOINCREMENT,
    core_uuid TEXT NOT NULL,
    memory_id TEXT NOT NULL,
    action TEXT NOT NULL,
    detail TEXT NOT NULL,
    created_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS memory_events_core_idx ON memory_events(core_uuid, sequence);
CREATE TRIGGER IF NOT EXISTS memory_events_no_update
BEFORE UPDATE ON memory_events
BEGIN
    SELECT RAISE(ABORT, 'memory_events is append-only');
END;
CREATE TRIGGER IF NOT EXISTS memory_events_no_delete
BEFORE DELETE ON memory_events
BEGIN
    SELECT RAISE(ABORT, 'memory_events is append-only');
END;
CREATE TRIGGER IF NOT EXISTS personality_growth_no_update
BEFORE UPDATE ON personality_growth
BEGIN
    SELECT RAISE(ABORT, 'personality_growth is append-only');
END;
CREATE TRIGGER IF NOT EXISTS personality_growth_no_delete
BEFORE DELETE ON personality_growth
BEGIN
    SELECT RAISE(ABORT, 'personality_growth is append-only');
END;
CREATE TRIGGER IF NOT EXISTS core_memory_ownership_no_update
BEFORE UPDATE ON core_memory_ownership
BEGIN
    SELECT RAISE(ABORT, 'core_memory_ownership is immutable');
END;
CREATE TRIGGER IF NOT EXISTS core_memory_ownership_no_delete
BEFORE DELETE ON core_memory_ownership
BEGIN
    SELECT RAISE(ABORT, 'core_memory_ownership is immutable');
END;
"#;

/// Bounded, snake_case memory classification.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum MemoryKind {
    Fact,
    Preference,
    Episode,
    Reflection,
    Relationship,
    Goal,
    Other(String),
}

impl MemoryKind {
    /// Stable storage identifier.
    pub fn as_str(&self) -> &str {
        match self {
            Self::Fact => "fact",
            Self::Preference => "preference",
            Self::Episode => "episode",
            Self::Reflection => "reflection",
            Self::Relationship => "relationship",
            Self::Goal => "goal",
            Self::Other(value) => value.as_str(),
        }
    }

    /// Parse a storage identifier, validating custom kinds.
    pub fn parse(value: &str) -> Result<Self, SdkError> {
        let trimmed = value.trim();
        let parsed = match trimmed {
            "fact" => Self::Fact,
            "preference" => Self::Preference,
            "episode" => Self::Episode,
            "reflection" => Self::Reflection,
            "relationship" => Self::Relationship,
            "goal" => Self::Goal,
            _ => {
                if trimmed.is_empty()
                    || !chars_within(trimmed, MAX_KIND_CHARS)
                    || !trimmed.chars().all(|character| {
                        character.is_ascii_alphanumeric() || character == '_' || character == '-'
                    })
                {
                    return Err(SdkError::InvalidMemoryKind);
                }
                Self::Other(trimmed.to_owned())
            }
        };
        Ok(parsed)
    }
}

impl Serialize for MemoryKind {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for MemoryKind {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::parse(&value).map_err(serde::de::Error::custom)
    }
}

/// A stored memory entry.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MemoryEntry {
    pub id: String,
    pub core_uuid: String,
    pub kind: MemoryKind,
    pub content: String,
    pub source: String,
    pub provenance: String,
    pub confidence: f64,
    pub tags: Vec<String>,
    pub created_at: String,
    pub updated_at: String,
    pub archived: bool,
    pub deleted: bool,
    pub revision: u64,
}

impl MemoryEntry {
    /// Validate bounds and ownership for this entry.
    pub fn validate(&self, expected_core: &str) -> Result<(), SdkError> {
        let id = Uuid::parse_str(&self.id).map_err(|_| SdkError::InvalidMemory("invalid id"))?;
        if id.is_nil() {
            return Err(SdkError::InvalidMemory("invalid id"));
        }
        if self.core_uuid != expected_core {
            return Err(SdkError::ForeignCore);
        }
        validate_content(&self.content)?;
        validate_bounded(&self.source, MAX_SOURCE_CHARS, "source is too long")?;
        validate_bounded(
            &self.provenance,
            MAX_PROVENANCE_CHARS,
            "provenance is too long",
        )?;
        if !self.confidence.is_finite() || !(0.0..=1.0).contains(&self.confidence) {
            return Err(SdkError::InvalidMemory("confidence is out of range"));
        }
        validate_tags(&self.tags)?;
        if self.created_at.trim().is_empty() || self.updated_at.trim().is_empty() {
            return Err(SdkError::InvalidMemory("timestamps are required"));
        }
        if self.revision == 0 {
            return Err(SdkError::InvalidMemory("revision must be at least 1"));
        }
        MemoryKind::parse(self.kind.as_str())?;
        Ok(())
    }
}

/// Input used to create a new memory entry.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NewMemory {
    pub kind: MemoryKind,
    pub content: String,
    pub source: String,
    pub provenance: String,
    pub confidence: f64,
    pub tags: Vec<String>,
}

impl NewMemory {
    /// Create a new input with conservative defaults.
    pub fn new(kind: MemoryKind, content: impl Into<String>) -> Self {
        Self {
            kind,
            content: content.into(),
            source: "core".to_owned(),
            provenance: "core_sdk".to_owned(),
            confidence: 1.0,
            tags: Vec::new(),
        }
    }

    pub fn with_source(mut self, source: impl Into<String>) -> Self {
        self.source = source.into();
        self
    }

    pub fn with_provenance(mut self, provenance: impl Into<String>) -> Self {
        self.provenance = provenance.into();
        self
    }

    pub fn with_confidence(mut self, confidence: f64) -> Self {
        self.confidence = confidence;
        self
    }

    pub fn with_tags(mut self, tags: Vec<String>) -> Self {
        self.tags = tags;
        self
    }
}

/// Bounded search request.
#[derive(Debug, Clone, PartialEq)]
pub struct MemoryQuery {
    pub text: Option<String>,
    pub kind: Option<MemoryKind>,
    pub include_archived: bool,
    pub include_deleted: bool,
    pub limit: usize,
    pub offset: usize,
}

impl Default for MemoryQuery {
    fn default() -> Self {
        Self {
            text: None,
            kind: None,
            include_archived: false,
            include_deleted: false,
            limit: 100,
            offset: 0,
        }
    }
}

impl MemoryQuery {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_text(mut self, text: impl Into<String>) -> Self {
        self.text = Some(text.into());
        self
    }

    pub fn with_kind(mut self, kind: MemoryKind) -> Self {
        self.kind = Some(kind);
        self
    }

    pub fn including_archived(mut self) -> Self {
        self.include_archived = true;
        self
    }

    pub fn including_deleted(mut self) -> Self {
        self.include_deleted = true;
        self
    }

    pub fn with_limit(mut self, limit: usize) -> Self {
        self.limit = limit;
        self
    }

    pub fn with_offset(mut self, offset: usize) -> Self {
        self.offset = offset;
        self
    }

    pub fn validate(&self) -> Result<(), SdkError> {
        if self.limit > MAX_QUERY_LIMIT {
            return Err(SdkError::InvalidQuery("limit exceeds the maximum"));
        }
        if let Some(text) = &self.text {
            if !chars_within(text, MAX_QUERY_TEXT_CHARS) {
                return Err(SdkError::InvalidQuery("search text is too long"));
            }
        }
        Ok(())
    }
}

/// Append-only event describing a memory mutation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryEvent {
    pub schema: String,
    pub sequence: u64,
    pub memory_id: String,
    pub action: String,
    pub detail: String,
    pub created_at: String,
}

impl MemoryEvent {
    /// Validate an event received from an external source.
    pub fn validate(&self) -> Result<(), SdkError> {
        if self.schema != MEMORY_EVENT_SCHEMA {
            return Err(SdkError::InvalidImport("event schema is unsupported"));
        }
        let id =
            Uuid::parse_str(&self.memory_id).map_err(|_| SdkError::InvalidImport("event id"))?;
        if id.is_nil() {
            return Err(SdkError::InvalidImport("event id"));
        }
        if self.action.trim().is_empty() {
            return Err(SdkError::InvalidImport("event action is required"));
        }
        if !chars_within(&self.action, MAX_KIND_CHARS) {
            return Err(SdkError::InvalidImport("event action is too long"));
        }
        if !chars_within(&self.detail, MAX_EVENT_DETAIL_CHARS) {
            return Err(SdkError::InvalidImport("event detail is too long"));
        }
        if self.created_at.trim().is_empty() {
            return Err(SdkError::InvalidImport("event timestamp is required"));
        }
        Ok(())
    }
}

/// Versioned transfer object for export and import.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MemoryExport {
    pub schema: String,
    pub sdk_version: String,
    pub core_uuid: String,
    pub exported_at: String,
    pub entries: Vec<MemoryEntry>,
    pub events: Vec<MemoryEvent>,
}

/// Behaviour when importing an entry whose identifier already exists.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImportMode {
    RejectExisting,
    SkipExisting,
    ReplaceExisting,
}

/// Counts describing an import result.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ImportOutcome {
    pub inserted: usize,
    pub skipped: usize,
    pub replaced: usize,
}

/// One writable per-Core memory database.
///
/// The store owns a `rusqlite::Connection` bound to the Core's own
/// `memory.db`. It never opens Platform, Safe, or another Core's database.
pub struct CoreMemoryStore {
    core_uuid: String,
    core_dir: PathBuf,
    database_path: PathBuf,
    connection: Connection,
}

impl std::fmt::Debug for CoreMemoryStore {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("CoreMemoryStore")
            .field("core_uuid", &self.core_uuid)
            .field("database_path", &self.database_path)
            .finish_non_exhaustive()
    }
}

impl CoreMemoryStore {
    /// Open (or initialize) the memory database for an existing Core.
    ///
    /// The owning Core must already exist; its identity is confirmed by a
    /// read-only snapshot of `core.db`. If the Core directory already contains
    /// a `memory.db`, its immutable ownership row must match `core_uuid`.
    pub fn open(root: impl AsRef<Path>, core_uuid: &str) -> Result<Self, SdkError> {
        let core_uuid = canonical_core_id(core_uuid)?;
        let layout = CoreStorageLayout::new(root)?;
        let snapshot = CoreDatabase::read_snapshot(&layout, &core_uuid)?;
        if snapshot.identity.core_uuid != core_uuid {
            return Err(SdkError::OwnershipMismatch);
        }

        let core_dir = layout.core_dir(&core_uuid)?;
        let memory_path = core_dir.join(MEMORY_DB_FILE);
        let existed = match fs::symlink_metadata(&memory_path) {
            Ok(metadata) => {
                if metadata.file_type().is_symlink() || !metadata.is_file() {
                    return Err(SdkError::ForeignPath);
                }
                true
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => false,
            Err(error) => return Err(error.into()),
        };

        let connection = open_initialized(&memory_path, MEMORY_APPLICATION_ID, MEMORY_COMPONENT)?;

        let canonical_dir = fs::canonicalize(&core_dir)?;
        let canonical_db = fs::canonicalize(&memory_path)?;
        if canonical_db.parent() != Some(canonical_dir.as_path())
            || canonical_db.file_name() != Some(OsStr::new(MEMORY_DB_FILE))
        {
            return Err(SdkError::ForeignPath);
        }

        let has_ownership = table_exists(&connection, MEMORY_OWNERSHIP_TABLE)?;
        if existed && !has_ownership {
            return Err(SdkError::OwnershipMissing);
        }
        if has_ownership {
            verify_ownership(&connection, &core_uuid)?;
        }

        initialize_schema(&connection)?;

        if !has_ownership {
            connection.execute(
                "INSERT INTO core_memory_ownership(singleton, core_uuid, schema, created_at)
                 VALUES(1, ?1, ?2, ?3)",
                params![core_uuid, MEMORY_CONTRACT_SCHEMA, now_rfc3339()?],
            )?;
        }

        Ok(Self {
            core_uuid,
            core_dir: canonical_dir,
            database_path: canonical_db,
            connection,
        })
    }

    /// Immutable Core UUID bound to this store.
    pub fn core_uuid(&self) -> &str {
        &self.core_uuid
    }

    /// Canonical Core directory containing the memory database.
    pub fn core_dir(&self) -> &Path {
        &self.core_dir
    }

    /// Canonical path of the memory database.
    pub fn database_path(&self) -> &Path {
        &self.database_path
    }

    /// Create a new memory entry and append a `created` event.
    pub fn create(
        &self,
        new: impl std::borrow::Borrow<NewMemory>,
    ) -> Result<MemoryEntry, SdkError> {
        let new = new.borrow();
        validate_content(&new.content)?;
        validate_bounded(&new.source, MAX_SOURCE_CHARS, "source is too long")?;
        validate_bounded(
            &new.provenance,
            MAX_PROVENANCE_CHARS,
            "provenance is too long",
        )?;
        if !new.confidence.is_finite() || !(0.0..=1.0).contains(&new.confidence) {
            return Err(SdkError::InvalidMemory("confidence is out of range"));
        }
        validate_tags(&new.tags)?;
        MemoryKind::parse(new.kind.as_str())?;

        let now = now_rfc3339()?;
        let entry = MemoryEntry {
            id: Uuid::new_v4().to_string(),
            core_uuid: self.core_uuid.clone(),
            kind: new.kind.clone(),
            content: new.content.clone(),
            source: new.source.clone(),
            provenance: new.provenance.clone(),
            confidence: new.confidence,
            tags: new.tags.clone(),
            created_at: now.clone(),
            updated_at: now,
            archived: false,
            deleted: false,
            revision: 1,
        };
        entry.validate(&self.core_uuid)?;

        let transaction = self.connection.unchecked_transaction()?;
        insert_entry(&transaction, &entry)?;
        append_event(
            &transaction,
            &self.core_uuid,
            &entry.id,
            "created",
            entry.kind.as_str(),
        )?;
        transaction.commit()?;
        Ok(entry)
    }

    /// Fetch one entry owned by this Core.
    pub fn get(&self, id: &str) -> Result<Option<MemoryEntry>, SdkError> {
        let mut statement = self.connection.prepare(&format!(
            "SELECT {ENTRY_COLUMNS} FROM memory_entries WHERE core_uuid = ?1 AND id = ?2"
        ))?;
        let raw = statement
            .query_row(params![self.core_uuid, id], read_raw)
            .optional()?;
        match raw {
            Some(raw) => Ok(Some(raw.into_entry(&self.core_uuid)?)),
            None => Ok(None),
        }
    }

    /// Fetch entries whose `source` exactly matches the supplied key.
    ///
    /// Archived and soft-deleted entries are included so callers can use this
    /// for stable-key idempotency checks without creating duplicates. Results
    /// are ordered oldest first.
    pub fn find_by_source(&self, source: &str) -> Result<Vec<MemoryEntry>, SdkError> {
        validate_bounded(source, MAX_SOURCE_CHARS, "source is too long")?;
        let mut statement = self.connection.prepare(&format!(
            "SELECT {ENTRY_COLUMNS} FROM memory_entries
             WHERE core_uuid = ?1 AND source = ?2
             ORDER BY created_at ASC, id ASC"
        ))?;
        let mut rows = statement.query(params![self.core_uuid, source])?;
        let mut entries = Vec::new();
        while let Some(row) = rows.next()? {
            entries.push(read_raw(row)?.into_entry(&self.core_uuid)?);
        }
        Ok(entries)
    }

    /// Search entries by case-insensitive text (content or tags) and kind.
    pub fn search(&self, query: &MemoryQuery) -> Result<Vec<MemoryEntry>, SdkError> {
        query.validate()?;
        let limit = query.limit;
        let needle = query
            .text
            .as_ref()
            .map(|text| text.to_lowercase())
            .filter(|text| !text.is_empty());
        let kind = query.kind.as_ref().map(MemoryKind::as_str);

        let mut statement = self.connection.prepare(&format!(
            "SELECT {ENTRY_COLUMNS} FROM memory_entries
             WHERE core_uuid = ?1
               AND (?2 IS NULL OR kind = ?2)
               AND (?3 = 1 OR archived = 0)
               AND (?4 = 1 OR deleted = 0)
             ORDER BY created_at ASC, id ASC"
        ))?;
        let mut rows = statement.query(params![
            self.core_uuid,
            kind,
            i64::from(u8::from(query.include_archived)),
            i64::from(u8::from(query.include_deleted))
        ])?;

        let mut skipped = 0usize;
        let mut entries = Vec::new();
        while let Some(row) = rows.next()? {
            let entry = read_raw(row)?.into_entry(&self.core_uuid)?;
            if let Some(needle) = &needle {
                let content_hit = entry.content.to_lowercase().contains(needle);
                let tag_hit = entry
                    .tags
                    .iter()
                    .any(|tag| tag.to_lowercase().contains(needle));
                if !content_hit && !tag_hit {
                    continue;
                }
            }
            if skipped < query.offset {
                skipped += 1;
                continue;
            }
            if entries.len() >= limit {
                break;
            }
            entries.push(entry);
        }
        Ok(entries)
    }

    /// Replace content and append a `content_updated` event.
    pub fn update_content(&self, id: &str, content: &str) -> Result<MemoryEntry, SdkError> {
        validate_content(content)?;
        let transaction = self.connection.unchecked_transaction()?;
        let mut entry = load_entry(&transaction, &self.core_uuid, id)?.ok_or(SdkError::NotFound)?;
        if entry.deleted {
            return Err(SdkError::Deleted);
        }
        entry.content = content.to_owned();
        entry.updated_at = now_rfc3339()?;
        entry.revision = entry.revision.saturating_add(1);
        entry.validate(&self.core_uuid)?;
        update_entry(&transaction, &entry)?;
        append_event(
            &transaction,
            &self.core_uuid,
            id,
            "content_updated",
            "content updated",
        )?;
        transaction.commit()?;
        Ok(entry)
    }

    /// Correct content and source, recording the reason in the event log.
    pub fn correct(
        &self,
        id: &str,
        content: &str,
        source: &str,
        reason: &str,
    ) -> Result<MemoryEntry, SdkError> {
        validate_content(content)?;
        validate_bounded(source, MAX_SOURCE_CHARS, "source is too long")?;
        let transaction = self.connection.unchecked_transaction()?;
        let mut entry = load_entry(&transaction, &self.core_uuid, id)?.ok_or(SdkError::NotFound)?;
        if entry.deleted {
            return Err(SdkError::Deleted);
        }
        entry.content = content.to_owned();
        entry.source = source.to_owned();
        entry.updated_at = now_rfc3339()?;
        entry.revision = entry.revision.saturating_add(1);
        entry.validate(&self.core_uuid)?;
        update_entry(&transaction, &entry)?;
        append_event(
            &transaction,
            &self.core_uuid,
            id,
            "corrected",
            &truncate_chars(reason, MAX_EVENT_DETAIL_CHARS),
        )?;
        transaction.commit()?;
        Ok(entry)
    }

    /// Replace the tag list and append a `tags_set` event.
    pub fn set_tags(&self, id: &str, tags: impl AsRef<[String]>) -> Result<MemoryEntry, SdkError> {
        let tags = tags.as_ref();
        validate_tags(tags)?;
        let transaction = self.connection.unchecked_transaction()?;
        let mut entry = load_entry(&transaction, &self.core_uuid, id)?.ok_or(SdkError::NotFound)?;
        if entry.deleted {
            return Err(SdkError::Deleted);
        }
        entry.tags = tags.to_vec();
        entry.updated_at = now_rfc3339()?;
        entry.revision = entry.revision.saturating_add(1);
        entry.validate(&self.core_uuid)?;
        update_entry(&transaction, &entry)?;
        append_event(
            &transaction,
            &self.core_uuid,
            id,
            "tags_set",
            "tags updated",
        )?;
        transaction.commit()?;
        Ok(entry)
    }

    /// Archive an entry so it is excluded from default searches.
    pub fn archive(&self, id: &str) -> Result<MemoryEntry, SdkError> {
        let transaction = self.connection.unchecked_transaction()?;
        let mut entry = load_entry(&transaction, &self.core_uuid, id)?.ok_or(SdkError::NotFound)?;
        if entry.deleted {
            return Err(SdkError::Deleted);
        }
        if entry.archived {
            transaction.commit()?;
            return Ok(entry);
        }
        entry.archived = true;
        entry.updated_at = now_rfc3339()?;
        entry.revision = entry.revision.saturating_add(1);
        update_entry(&transaction, &entry)?;
        append_event(
            &transaction,
            &self.core_uuid,
            id,
            "archived",
            "entry archived",
        )?;
        transaction.commit()?;
        Ok(entry)
    }

    /// Restore a previously archived entry.
    pub fn restore_archived(&self, id: &str) -> Result<MemoryEntry, SdkError> {
        let transaction = self.connection.unchecked_transaction()?;
        let mut entry = load_entry(&transaction, &self.core_uuid, id)?.ok_or(SdkError::NotFound)?;
        if entry.deleted {
            return Err(SdkError::Deleted);
        }
        if !entry.archived {
            transaction.commit()?;
            return Ok(entry);
        }
        entry.archived = false;
        entry.updated_at = now_rfc3339()?;
        entry.revision = entry.revision.saturating_add(1);
        update_entry(&transaction, &entry)?;
        append_event(
            &transaction,
            &self.core_uuid,
            id,
            "restored",
            "entry restored from archive",
        )?;
        transaction.commit()?;
        Ok(entry)
    }

    /// Mark an entry deleted without removing its content.
    pub fn soft_delete(&self, id: &str) -> Result<MemoryEntry, SdkError> {
        let transaction = self.connection.unchecked_transaction()?;
        let mut entry = load_entry(&transaction, &self.core_uuid, id)?.ok_or(SdkError::NotFound)?;
        if entry.deleted {
            transaction.commit()?;
            return Ok(entry);
        }
        entry.deleted = true;
        entry.updated_at = now_rfc3339()?;
        entry.revision = entry.revision.saturating_add(1);
        update_entry(&transaction, &entry)?;
        append_event(
            &transaction,
            &self.core_uuid,
            id,
            "soft_deleted",
            "entry soft deleted",
        )?;
        transaction.commit()?;
        Ok(entry)
    }

    /// Physically delete an entry and append a `purged` event.
    ///
    /// Callers are responsible for enforcing owner/Platform authorization
    /// before invoking a physical purge. This store performs no authorization.
    pub fn purge(&self, id: &str) -> Result<(), SdkError> {
        let transaction = self.connection.unchecked_transaction()?;
        let exists: Option<String> = transaction
            .query_row(
                "SELECT id FROM memory_entries WHERE core_uuid = ?1 AND id = ?2",
                params![self.core_uuid, id],
                |row| row.get(0),
            )
            .optional()?;
        if exists.is_none() {
            return Err(SdkError::NotFound);
        }
        transaction.execute(
            "DELETE FROM memory_entries WHERE core_uuid = ?1 AND id = ?2",
            params![self.core_uuid, id],
        )?;
        append_event(&transaction, &self.core_uuid, id, "purged", "entry purged")?;
        transaction.commit()?;
        Ok(())
    }

    /// List events for this Core, optionally filtered to one memory id.
    pub fn list_events(&self, memory_id: Option<&str>) -> Result<Vec<MemoryEvent>, SdkError> {
        let mut events = Vec::new();
        match memory_id {
            Some(memory_id) => {
                let mut statement = self.connection.prepare(
                    "SELECT sequence, memory_id, action, detail, created_at
                     FROM memory_events WHERE core_uuid = ?1 AND memory_id = ?2
                     ORDER BY sequence ASC",
                )?;
                let mut rows = statement.query(params![self.core_uuid, memory_id])?;
                while let Some(row) = rows.next()? {
                    events.push(read_event(row)?);
                }
            }
            None => {
                let mut statement = self.connection.prepare(
                    "SELECT sequence, memory_id, action, detail, created_at
                     FROM memory_events WHERE core_uuid = ?1 ORDER BY sequence ASC",
                )?;
                let mut rows = statement.query(params![self.core_uuid])?;
                while let Some(row) = rows.next()? {
                    events.push(read_event(row)?);
                }
            }
        }
        Ok(events)
    }

    /// Export every entry and event for this Core.
    pub fn export(&self) -> Result<MemoryExport, SdkError> {
        let entries = self.all_entries()?;
        let events = self.list_events(None)?;
        Ok(MemoryExport {
            schema: MEMORY_EXPORT_SCHEMA.to_owned(),
            sdk_version: CORE_SDK_VERSION.to_owned(),
            core_uuid: self.core_uuid.clone(),
            exported_at: now_rfc3339()?,
            entries,
            events,
        })
    }

    /// Import a versioned export in one all-or-nothing transaction.
    ///
    /// The export must belong to this Core. Events are appended with fresh
    /// sequence numbers; existing sequence values are not reused.
    pub fn import(
        &self,
        export: impl std::borrow::Borrow<MemoryExport>,
        mode: ImportMode,
    ) -> Result<ImportOutcome, SdkError> {
        let export = export.borrow();
        if export.schema != MEMORY_EXPORT_SCHEMA {
            return Err(SdkError::UnsupportedImportSchema);
        }
        if canonical_core_id(&export.core_uuid)? != self.core_uuid {
            return Err(SdkError::ForeignCore);
        }
        if export.entries.len() > MAX_IMPORT_ENTRIES {
            return Err(SdkError::InvalidImport("too many entries"));
        }
        if export.events.len() > MAX_IMPORT_EVENTS {
            return Err(SdkError::InvalidImport("too many events"));
        }

        let mut seen = BTreeSet::new();
        let mut prepared = Vec::with_capacity(export.entries.len());
        for entry in &export.entries {
            if entry.content.len() > MAX_IMPORT_CONTENT_BYTES {
                return Err(SdkError::InvalidImport("content exceeds the import bound"));
            }
            entry.validate(&self.core_uuid)?;
            if !seen.insert(entry.id.clone()) {
                return Err(SdkError::InvalidImport("duplicate entry id"));
            }
            prepared.push(entry.clone());
        }
        for event in &export.events {
            event.validate()?;
        }

        let transaction = self.connection.unchecked_transaction()?;
        let mut outcome = ImportOutcome::default();
        for entry in prepared {
            let existing: Option<String> = transaction
                .query_row(
                    "SELECT id FROM memory_entries WHERE core_uuid = ?1 AND id = ?2",
                    params![self.core_uuid, entry.id],
                    |row| row.get(0),
                )
                .optional()?;
            match existing {
                None => {
                    insert_entry(&transaction, &entry)?;
                    outcome.inserted += 1;
                }
                Some(_) => match mode {
                    ImportMode::RejectExisting => {
                        return Err(SdkError::ImportConflict(entry.id));
                    }
                    ImportMode::SkipExisting => {
                        outcome.skipped += 1;
                    }
                    ImportMode::ReplaceExisting => {
                        update_entry(&transaction, &entry)?;
                        outcome.replaced += 1;
                    }
                },
            }
        }

        for event in &export.events {
            insert_event_at(
                &transaction,
                &self.core_uuid,
                &event.memory_id,
                &event.action,
                &event.detail,
                &event.created_at,
            )?;
        }
        transaction.commit()?;
        Ok(outcome)
    }

    /// Load the current personality profile, if one has been stored.
    pub fn load_personality(&self) -> Result<Option<PersonalityProfile>, SdkError> {
        let json: Option<String> = self
            .connection
            .query_row(
                "SELECT profile_json FROM core_personality WHERE core_uuid = ?1",
                params![self.core_uuid],
                |row| row.get(0),
            )
            .optional()?;
        match json {
            None => Ok(None),
            Some(json) => {
                let profile: PersonalityProfile = serde_json::from_str(&json)?;
                if canonical_core_id(&profile.core_uuid)? != self.core_uuid {
                    return Err(SdkError::OwnershipMismatch);
                }
                profile.validate()?;
                Ok(Some(profile))
            }
        }
    }

    /// Store a personality profile, incrementing its revision and appending an
    /// immutable growth-history record.
    pub fn store_personality(
        &self,
        profile: &PersonalityProfile,
    ) -> Result<PersonalityProfile, SdkError> {
        if canonical_core_id(&profile.core_uuid)? != self.core_uuid {
            return Err(SdkError::ForeignCore);
        }

        let transaction = self.connection.unchecked_transaction()?;
        let current: Option<i64> = transaction
            .query_row(
                "SELECT revision FROM core_personality WHERE core_uuid = ?1",
                params![self.core_uuid],
                |row| row.get(0),
            )
            .optional()?;
        let growth_max: Option<i64> = transaction.query_row(
            "SELECT MAX(revision) FROM personality_growth WHERE core_uuid = ?1",
            params![self.core_uuid],
            |row| row.get::<_, Option<i64>>(0),
        )?;
        let baseline = current.unwrap_or(0).max(growth_max.unwrap_or(0));
        let baseline = u64::try_from(baseline)
            .map_err(|_| SdkError::InvalidPersonality("revision is out of range"))?;
        let revision = baseline.saturating_add(1);
        let revision_sql = i64::try_from(revision)
            .map_err(|_| SdkError::InvalidPersonality("revision is out of range"))?;
        let now = now_rfc3339()?;

        let mut stored = profile.clone();
        stored.schema = PERSONALITY_SCHEMA.to_owned();
        stored.core_uuid = self.core_uuid.clone();
        stored.updated_at = now.clone();
        stored.revision = revision;
        stored.validate()?;

        let json = serde_json::to_string(&stored)?;
        transaction.execute(
            "INSERT INTO core_personality(core_uuid, revision, profile_json, updated_at)
             VALUES(?1, ?2, ?3, ?4)
             ON CONFLICT(core_uuid) DO UPDATE SET
                 revision = excluded.revision,
                 profile_json = excluded.profile_json,
                 updated_at = excluded.updated_at",
            params![self.core_uuid, revision_sql, json, now],
        )?;
        transaction.execute(
            "INSERT INTO personality_growth(core_uuid, revision, profile_json, created_at)
             VALUES(?1, ?2, ?3, ?4)",
            params![self.core_uuid, revision_sql, json, now],
        )?;
        transaction.commit()?;
        Ok(stored)
    }

    /// Alias for [`CoreMemoryStore::store_personality`].
    pub fn save_personality(
        &self,
        profile: &PersonalityProfile,
    ) -> Result<PersonalityProfile, SdkError> {
        self.store_personality(profile)
    }

    /// Alias for [`CoreMemoryStore::load_personality`].
    pub fn get_personality(&self) -> Result<Option<PersonalityProfile>, SdkError> {
        self.load_personality()
    }

    /// Alias for [`CoreMemoryStore::store_personality`].
    pub fn update_personality(
        &self,
        profile: &PersonalityProfile,
    ) -> Result<PersonalityProfile, SdkError> {
        self.store_personality(profile)
    }

    /// Full append-only personality growth history, oldest first.
    pub fn personality_growth(&self) -> Result<Vec<PersonalityProfile>, SdkError> {
        let mut statement = self.connection.prepare(
            "SELECT profile_json FROM personality_growth
             WHERE core_uuid = ?1 ORDER BY revision ASC",
        )?;
        let rows = statement.query_map(params![self.core_uuid], |row| row.get::<_, String>(0))?;
        let mut profiles = Vec::new();
        for row in rows {
            let json = row?;
            let profile: PersonalityProfile = serde_json::from_str(&json)?;
            profile.validate()?;
            profiles.push(profile);
        }
        Ok(profiles)
    }

    /// Alias for [`CoreMemoryStore::personality_growth`].
    pub fn personality_history(&self) -> Result<Vec<PersonalityProfile>, SdkError> {
        self.personality_growth()
    }

    /// All events for this Core, oldest first.
    pub fn events(&self) -> Result<Vec<MemoryEvent>, SdkError> {
        self.list_events(None)
    }

    /// Capabilities advertised by this store's contract (descriptive only).
    pub fn capabilities(&self) -> &'static [CoreCapability] {
        CoreCapability::all()
    }

    fn all_entries(&self) -> Result<Vec<MemoryEntry>, SdkError> {
        let mut statement = self.connection.prepare(&format!(
            "SELECT {ENTRY_COLUMNS} FROM memory_entries WHERE core_uuid = ?1
             ORDER BY created_at ASC, id ASC"
        ))?;
        let mut rows = statement.query(params![self.core_uuid])?;
        let mut entries = Vec::new();
        while let Some(row) = rows.next()? {
            entries.push(read_raw(row)?.into_entry(&self.core_uuid)?);
        }
        Ok(entries)
    }
}

fn initialize_schema(connection: &Connection) -> Result<(), SdkError> {
    connection.execute_batch(SCHEMA_SQL)?;
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

fn verify_ownership(connection: &Connection, expected_core: &str) -> Result<(), SdkError> {
    let row: Option<(String, String)> = connection
        .query_row(
            "SELECT core_uuid, schema FROM core_memory_ownership WHERE singleton = 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    let Some((stored_core, schema)) = row else {
        return Err(SdkError::OwnershipMissing);
    };
    if schema != MEMORY_CONTRACT_SCHEMA {
        return Err(SdkError::UnsupportedMemorySchema);
    }
    let stored_core = canonical_core_id(&stored_core).map_err(|_| SdkError::OwnershipMismatch)?;
    if stored_core != expected_core {
        return Err(SdkError::OwnershipMismatch);
    }
    Ok(())
}

fn validate_content(content: &str) -> Result<(), SdkError> {
    if content.len() > MAX_CONTENT_BYTES {
        return Err(SdkError::InvalidMemory(
            "content exceeds the 64 KiB entry bound",
        ));
    }
    Ok(())
}

fn validate_bounded(value: &str, max_chars: usize, message: &'static str) -> Result<(), SdkError> {
    if !chars_within(value, max_chars) || value.chars().any(char::is_control) {
        return Err(SdkError::InvalidMemory(message));
    }
    Ok(())
}

fn validate_tags(tags: &[String]) -> Result<(), SdkError> {
    if tags.len() > MAX_TAGS {
        return Err(SdkError::InvalidMemory("too many tags"));
    }
    for tag in tags {
        let trimmed = tag.trim();
        if trimmed.is_empty()
            || !chars_within(trimmed, MAX_TAG_CHARS)
            || trimmed.chars().any(char::is_control)
        {
            return Err(SdkError::InvalidMemory("tag is invalid"));
        }
    }
    Ok(())
}

fn truncate_chars(value: &str, max_chars: usize) -> String {
    if value.chars().count() <= max_chars {
        return value.to_owned();
    }
    value.chars().take(max_chars).collect()
}

struct RawEntry {
    id: String,
    core_uuid: String,
    kind: String,
    content: String,
    source: String,
    provenance: String,
    confidence: f64,
    tags_json: String,
    created_at: String,
    updated_at: String,
    archived: i64,
    deleted: i64,
    revision: i64,
}

impl RawEntry {
    fn into_entry(self, expected_core: &str) -> Result<MemoryEntry, SdkError> {
        let kind = MemoryKind::parse(&self.kind)?;
        let tags: Vec<String> = serde_json::from_str(&self.tags_json)?;
        let revision = u64::try_from(self.revision)
            .map_err(|_| SdkError::CorruptData("revision is out of range"))?;
        let entry = MemoryEntry {
            id: self.id,
            core_uuid: self.core_uuid,
            kind,
            content: self.content,
            source: self.source,
            provenance: self.provenance,
            confidence: self.confidence,
            tags,
            created_at: self.created_at,
            updated_at: self.updated_at,
            archived: self.archived != 0,
            deleted: self.deleted != 0,
            revision,
        };
        entry.validate(expected_core)?;
        Ok(entry)
    }
}

fn read_raw(row: &rusqlite::Row<'_>) -> rusqlite::Result<RawEntry> {
    Ok(RawEntry {
        id: row.get(0)?,
        core_uuid: row.get(1)?,
        kind: row.get(2)?,
        content: row.get(3)?,
        source: row.get(4)?,
        provenance: row.get(5)?,
        confidence: row.get(6)?,
        tags_json: row.get(7)?,
        created_at: row.get(8)?,
        updated_at: row.get(9)?,
        archived: row.get(10)?,
        deleted: row.get(11)?,
        revision: row.get(12)?,
    })
}

fn read_event(row: &rusqlite::Row<'_>) -> rusqlite::Result<MemoryEvent> {
    let sequence: i64 = row.get(0)?;
    let sequence = u64::try_from(sequence)
        .map_err(|error| rusqlite::Error::ToSqlConversionFailure(Box::new(error)))?;
    Ok(MemoryEvent {
        schema: MEMORY_EVENT_SCHEMA.to_owned(),
        sequence,
        memory_id: row.get(1)?,
        action: row.get(2)?,
        detail: row.get(3)?,
        created_at: row.get(4)?,
    })
}

fn load_entry(
    transaction: &Transaction<'_>,
    expected_core: &str,
    id: &str,
) -> Result<Option<MemoryEntry>, SdkError> {
    let mut statement = transaction.prepare(&format!(
        "SELECT {ENTRY_COLUMNS} FROM memory_entries WHERE core_uuid = ?1 AND id = ?2"
    ))?;
    let raw = statement
        .query_row(params![expected_core, id], read_raw)
        .optional()?;
    match raw {
        Some(raw) => Ok(Some(raw.into_entry(expected_core)?)),
        None => Ok(None),
    }
}

fn insert_entry(transaction: &Transaction<'_>, entry: &MemoryEntry) -> Result<(), SdkError> {
    let tags_json = serde_json::to_string(&entry.tags)?;
    let revision = i64::try_from(entry.revision)
        .map_err(|_| SdkError::InvalidMemory("revision is out of range"))?;
    transaction.execute(
        "INSERT INTO memory_entries(
            id, core_uuid, kind, content, source, provenance, confidence, tags_json,
            created_at, updated_at, archived, deleted, revision
         ) VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
        params![
            entry.id,
            entry.core_uuid,
            entry.kind.as_str(),
            entry.content,
            entry.source,
            entry.provenance,
            entry.confidence,
            tags_json,
            entry.created_at,
            entry.updated_at,
            i64::from(u8::from(entry.archived)),
            i64::from(u8::from(entry.deleted)),
            revision
        ],
    )?;
    Ok(())
}

fn update_entry(transaction: &Transaction<'_>, entry: &MemoryEntry) -> Result<(), SdkError> {
    let tags_json = serde_json::to_string(&entry.tags)?;
    let revision = i64::try_from(entry.revision)
        .map_err(|_| SdkError::InvalidMemory("revision is out of range"))?;
    let changed = transaction.execute(
        "UPDATE memory_entries SET
            kind = ?3,
            content = ?4,
            source = ?5,
            provenance = ?6,
            confidence = ?7,
            tags_json = ?8,
            created_at = ?9,
            updated_at = ?10,
            archived = ?11,
            deleted = ?12,
            revision = ?13
         WHERE id = ?1 AND core_uuid = ?2",
        params![
            entry.id,
            entry.core_uuid,
            entry.kind.as_str(),
            entry.content,
            entry.source,
            entry.provenance,
            entry.confidence,
            tags_json,
            entry.created_at,
            entry.updated_at,
            i64::from(u8::from(entry.archived)),
            i64::from(u8::from(entry.deleted)),
            revision
        ],
    )?;
    if changed != 1 {
        return Err(SdkError::NotFound);
    }
    Ok(())
}

fn append_event(
    transaction: &Transaction<'_>,
    core_uuid: &str,
    memory_id: &str,
    action: &str,
    detail: &str,
) -> Result<(), SdkError> {
    let created_at = now_rfc3339()?;
    insert_event_at(
        transaction,
        core_uuid,
        memory_id,
        action,
        detail,
        &created_at,
    )
}

fn insert_event_at(
    transaction: &Transaction<'_>,
    core_uuid: &str,
    memory_id: &str,
    action: &str,
    detail: &str,
    created_at: &str,
) -> Result<(), SdkError> {
    transaction.execute(
        "INSERT INTO memory_events(core_uuid, memory_id, action, detail, created_at)
         VALUES(?1, ?2, ?3, ?4, ?5)",
        params![core_uuid, memory_id, action, detail, created_at],
    )?;
    Ok(())
}
