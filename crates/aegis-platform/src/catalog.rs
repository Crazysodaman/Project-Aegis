//! Platform-owned shared item catalog backed by `platform.db`.
//!
//! Catalog entries are [`ItemDefinition`] contracts from
//! `aegis_contracts::living`. The Platform owns authority: every mutation
//! requires Safe owner proof, supplied either directly or carried by an
//! explicit [`CatalogActor`]. Cores have no Platform write authorization yet,
//! so a Core cannot mutate the catalog merely by being a Core.

use aegis_contracts::living::{ItemCategory, ItemDefinition, Visibility, MAX_TAG_CHARS};
use aegis_safe::SafeStore;
use aegis_storage::{now_rfc3339, open_initialized, StoragePaths, PLATFORM_APPLICATION_ID};
use rusqlite::types::Value as SqlValue;
use rusqlite::{params, params_from_iter, Connection, OptionalExtension};
use uuid::Uuid;

pub const CATALOG_DEFAULT_LIMIT: usize = 50;
pub const CATALOG_MAX_LIMIT: usize = 200;
pub const CATALOG_MAX_SEARCH_CHARS: usize = 256;
pub const CATALOG_MAX_ACTOR_CHARS: usize = 128;

#[derive(Debug, thiserror::Error)]
pub enum CatalogError {
    #[error("catalog storage failed: {0}")]
    Storage(#[from] aegis_storage::StorageError),
    #[error("catalog SQLite operation failed: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("catalog JSON encoding failed: {0}")]
    Json(#[from] serde_json::Error),
    #[error("catalog Safe operation failed: {0}")]
    Safe(#[from] aegis_safe::SafeError),
    #[error("catalog owner authorization failed")]
    Unauthorized,
    #[error("invalid item definition")]
    InvalidDefinition,
    #[error("stored catalog definition is invalid")]
    Corrupt,
    #[error("catalog definition already exists: {0}")]
    Duplicate(String),
    #[error("catalog definition not found: {0}")]
    NotFound(String),
    #[error("catalog definition is archived: {0}")]
    Archived(String),
    #[error("catalog creator UUID is not canonical")]
    InvalidCreator,
    #[error("catalog creator cannot be changed")]
    CreatorImmutable,
    #[error("catalog revision is invalid")]
    InvalidRevision,
    #[error("catalog filter is invalid: {0}")]
    InvalidFilter(&'static str),
    #[error("catalog search text is invalid")]
    InvalidSearch,
    #[error("catalog actor is invalid")]
    InvalidActor,
}

/// Explicit mutation identity paired with an authorized Safe owner token.
///
/// The token, not the actor label, grants authority. Actor labels are recorded
/// in `catalog_events` so Platform and future Core callers remain auditable.
#[derive(Clone)]
pub struct CatalogActor {
    actor: String,
    owner_token: String,
}

impl CatalogActor {
    pub fn new(
        actor: impl Into<String>,
        owner_token: impl Into<String>,
    ) -> Result<Self, CatalogError> {
        let actor = actor.into();
        if actor.trim().is_empty()
            || actor.chars().count() > CATALOG_MAX_ACTOR_CHARS
            || actor.chars().any(char::is_control)
        {
            return Err(CatalogError::InvalidActor);
        }
        Ok(Self {
            actor,
            owner_token: owner_token.into(),
        })
    }

    pub fn owner(owner_token: impl Into<String>) -> Self {
        Self {
            actor: "owner".to_owned(),
            owner_token: owner_token.into(),
        }
    }

    pub fn actor(&self) -> &str {
        &self.actor
    }
}

impl std::fmt::Debug for CatalogActor {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("CatalogActor")
            .field("actor", &self.actor)
            .field("owner_token", &"<redacted>")
            .finish()
    }
}

/// Bounded query filter for [`CatalogStore::list`].
#[derive(Debug, Clone, Default)]
pub struct CatalogFilter {
    pub category: Option<ItemCategory>,
    pub visibility: Option<Visibility>,
    pub creator_core_uuid: Option<String>,
    pub tag: Option<String>,
    pub include_archived: bool,
    pub limit: Option<usize>,
}

#[derive(Debug, Clone)]
pub struct CatalogStore {
    paths: StoragePaths,
    safe: SafeStore,
}

impl CatalogStore {
    pub fn new(paths: StoragePaths) -> Self {
        Self {
            safe: SafeStore::new(paths.clone()),
            paths,
        }
    }

    pub fn initialize(&self) -> Result<(), CatalogError> {
        let connection = self.connection()?;
        connection.execute_batch(
            "CREATE TABLE IF NOT EXISTS catalog_definitions (
                definition_id TEXT PRIMARY KEY,
                definition_json TEXT NOT NULL,
                creator_core_uuid TEXT,
                category TEXT NOT NULL,
                visibility TEXT NOT NULL,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                revision INTEGER NOT NULL CHECK(revision >= 1),
                archived INTEGER NOT NULL DEFAULT 0 CHECK(archived IN (0, 1)),
                archived_at TEXT,
                archived_by TEXT
            );
            CREATE INDEX IF NOT EXISTS catalog_definitions_category_idx
                ON catalog_definitions(category);
            CREATE INDEX IF NOT EXISTS catalog_definitions_creator_idx
                ON catalog_definitions(creator_core_uuid);
            CREATE INDEX IF NOT EXISTS catalog_definitions_visibility_idx
                ON catalog_definitions(visibility);
            CREATE INDEX IF NOT EXISTS catalog_definitions_updated_idx
                ON catalog_definitions(updated_at, definition_id);
            CREATE TABLE IF NOT EXISTS catalog_events (
                sequence INTEGER PRIMARY KEY AUTOINCREMENT,
                event_type TEXT NOT NULL,
                definition_id TEXT NOT NULL,
                revision INTEGER NOT NULL,
                actor TEXT NOT NULL,
                detail_json TEXT NOT NULL,
                created_at TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS catalog_events_definition_idx
                ON catalog_events(definition_id, sequence);
            CREATE TRIGGER IF NOT EXISTS catalog_events_append_only_update
            BEFORE UPDATE ON catalog_events
            BEGIN
                SELECT RAISE(ABORT, 'catalog_events is append-only');
            END;
            CREATE TRIGGER IF NOT EXISTS catalog_events_append_only_delete
            BEFORE DELETE ON catalog_events
            BEGIN
                SELECT RAISE(ABORT, 'catalog_events is append-only');
            END;",
        )?;
        Ok(())
    }

    /// Create a definition. Requires Safe owner proof and a unique ID.
    pub fn create(
        &self,
        owner_token: &str,
        definition: ItemDefinition,
    ) -> Result<ItemDefinition, CatalogError> {
        self.create_with_actor("owner", owner_token, definition)
    }

    /// Create a definition as an explicit [`CatalogActor`].
    pub fn create_as(
        &self,
        actor: &CatalogActor,
        definition: ItemDefinition,
    ) -> Result<ItemDefinition, CatalogError> {
        self.create_with_actor(&actor.actor, &actor.owner_token, definition)
    }

    fn create_with_actor(
        &self,
        actor: &str,
        owner_token: &str,
        mut definition: ItemDefinition,
    ) -> Result<ItemDefinition, CatalogError> {
        self.require_owner(owner_token)?;
        definition
            .validate()
            .map_err(|_| CatalogError::InvalidDefinition)?;
        validate_creator(&definition.creator_core_uuid)?;
        let now = now_rfc3339()?;
        definition.revision = 1;
        definition.created_at = now.clone();
        definition.updated_at = now.clone();
        let encoded = serde_json::to_string(&definition)?;
        let revision = revision_value(definition.revision)?;

        let mut connection = self.connection()?;
        let transaction = connection.transaction()?;
        let existing: Option<String> = transaction
            .query_row(
                "SELECT definition_id FROM catalog_definitions WHERE definition_id = ?1",
                [&definition.definition_id],
                |row| row.get(0),
            )
            .optional()?;
        if existing.is_some() {
            return Err(CatalogError::Duplicate(definition.definition_id));
        }
        transaction.execute(
            "INSERT INTO catalog_definitions(
                definition_id, definition_json, creator_core_uuid, category, visibility,
                created_at, updated_at, revision, archived
             ) VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 0)",
            params![
                &definition.definition_id,
                encoded,
                definition.creator_core_uuid.as_deref(),
                definition.category.as_str(),
                definition.visibility.as_str(),
                &definition.created_at,
                &definition.updated_at,
                revision,
            ],
        )?;
        append_event(&transaction, "catalog.created", actor, &definition, &now)?;
        transaction.commit()?;
        Ok(definition)
    }

    /// Fetch an active (non-archived) definition.
    pub fn get(&self, definition_id: &str) -> Result<Option<ItemDefinition>, CatalogError> {
        let connection = self.connection()?;
        let encoded: Option<String> = connection
            .query_row(
                "SELECT definition_json FROM catalog_definitions
                 WHERE definition_id = ?1 AND archived = 0",
                [definition_id],
                |row| row.get(0),
            )
            .optional()?;
        encoded.as_deref().map(decode_definition).transpose()
    }

    /// List active definitions with bounded filters and deterministic ordering.
    pub fn list(&self, filter: &CatalogFilter) -> Result<Vec<ItemDefinition>, CatalogError> {
        let limit = effective_limit(filter.limit)?;
        if let Some(creator) = filter.creator_core_uuid.as_deref() {
            validate_creator_value(creator)?;
        }
        if let Some(tag) = filter.tag.as_deref() {
            validate_tag(tag)?;
        }

        let mut sql = String::from("SELECT definition_json FROM catalog_definitions WHERE 1 = 1");
        let mut values = Vec::new();
        if !filter.include_archived {
            sql.push_str(" AND archived = 0");
        }
        if let Some(category) = filter.category {
            sql.push_str(" AND category = ?");
            values.push(SqlValue::Text(category.as_str().to_owned()));
        }
        if let Some(visibility) = filter.visibility {
            sql.push_str(" AND visibility = ?");
            values.push(SqlValue::Text(visibility.as_str().to_owned()));
        }
        if let Some(creator) = filter.creator_core_uuid.as_deref() {
            sql.push_str(" AND creator_core_uuid = ?");
            values.push(SqlValue::Text(creator.to_owned()));
        }
        if let Some(tag) = filter.tag.as_deref() {
            sql.push_str(
                " AND EXISTS (
                    SELECT 1 FROM json_each(catalog_definitions.definition_json, '$.tags')
                    WHERE json_each.value = ?
                )",
            );
            values.push(SqlValue::Text(tag.to_owned()));
        }
        sql.push_str(" ORDER BY updated_at DESC, definition_id ASC LIMIT ?");
        values.push(SqlValue::Integer(limit_value(limit)?));

        let connection = self.connection()?;
        query_definitions(&connection, &sql, values)
    }

    /// Search active definitions by bounded free text (name, description,
    /// tags, category, creator). Results use the same deterministic ordering.
    pub fn search(&self, text: &str) -> Result<Vec<ItemDefinition>, CatalogError> {
        let text = text.trim();
        if text.is_empty()
            || text.chars().count() > CATALOG_MAX_SEARCH_CHARS
            || text.chars().any(char::is_control)
        {
            return Err(CatalogError::InvalidSearch);
        }
        let pattern = format!("%{}%", escape_like(text));
        let sql = "SELECT definition_json FROM catalog_definitions
                   WHERE archived = 0 AND definition_json LIKE ?1 ESCAPE '\\'
                   ORDER BY updated_at DESC, definition_id ASC LIMIT ?2";
        let values = vec![
            SqlValue::Text(pattern),
            SqlValue::Integer(limit_value(CATALOG_DEFAULT_LIMIT)?),
        ];
        let connection = self.connection()?;
        query_definitions(&connection, sql, values)
    }

    /// Replace an existing definition with `revision + 1`. Owner proof is
    /// required; creator identity, `created_at`, and `definition_id` are
    /// authoritative and cannot be rewritten by the replacement body.
    pub fn update_own(
        &self,
        owner_token: &str,
        definition_id: &str,
        replacement: ItemDefinition,
    ) -> Result<ItemDefinition, CatalogError> {
        self.update_own_with_actor("owner", owner_token, definition_id, replacement)
    }

    /// Update as an explicit [`CatalogActor`].
    pub fn update_own_as(
        &self,
        actor: &CatalogActor,
        definition_id: &str,
        replacement: ItemDefinition,
    ) -> Result<ItemDefinition, CatalogError> {
        self.update_own_with_actor(&actor.actor, &actor.owner_token, definition_id, replacement)
    }

    fn update_own_with_actor(
        &self,
        actor: &str,
        owner_token: &str,
        definition_id: &str,
        mut replacement: ItemDefinition,
    ) -> Result<ItemDefinition, CatalogError> {
        self.require_owner(owner_token)?;
        replacement
            .validate()
            .map_err(|_| CatalogError::InvalidDefinition)?;
        validate_creator(&replacement.creator_core_uuid)?;

        let mut connection = self.connection()?;
        let transaction = connection.transaction()?;
        let existing = transaction
            .query_row(
                "SELECT creator_core_uuid, created_at, revision, archived
                 FROM catalog_definitions WHERE definition_id = ?1",
                [definition_id],
                |row| {
                    Ok(ExistingDefinition {
                        creator_core_uuid: row.get(0)?,
                        created_at: row.get(1)?,
                        revision: row.get(2)?,
                        archived: row.get::<_, i64>(3)? != 0,
                    })
                },
            )
            .optional()?;
        let Some(existing) = existing else {
            return Err(CatalogError::NotFound(definition_id.to_owned()));
        };
        if existing.archived {
            return Err(CatalogError::Archived(definition_id.to_owned()));
        }
        if existing.creator_core_uuid != replacement.creator_core_uuid {
            return Err(CatalogError::CreatorImmutable);
        }
        let next_revision = existing
            .revision
            .checked_add(1)
            .ok_or(CatalogError::InvalidRevision)?;
        let next_revision_u64 =
            u64::try_from(next_revision).map_err(|_| CatalogError::InvalidRevision)?;

        let now = now_rfc3339()?;
        replacement.definition_id = definition_id.to_owned();
        replacement.revision = next_revision_u64;
        replacement.created_at = existing.created_at;
        replacement.updated_at = now.clone();
        let encoded = serde_json::to_string(&replacement)?;
        transaction.execute(
            "UPDATE catalog_definitions SET
                definition_json = ?1,
                creator_core_uuid = ?2,
                category = ?3,
                visibility = ?4,
                updated_at = ?5,
                revision = ?6
             WHERE definition_id = ?7 AND archived = 0",
            params![
                encoded,
                replacement.creator_core_uuid.as_deref(),
                replacement.category.as_str(),
                replacement.visibility.as_str(),
                &replacement.updated_at,
                next_revision,
                definition_id,
            ],
        )?;
        append_event(&transaction, "catalog.updated", actor, &replacement, &now)?;
        transaction.commit()?;
        Ok(replacement)
    }

    /// Archive a definition. The row and all events are preserved.
    ///
    /// Returns `true` when the definition was newly archived and `false` when
    /// it was already archived.
    pub fn archive(&self, owner_token: &str, definition_id: &str) -> Result<bool, CatalogError> {
        self.archive_with_actor("owner", owner_token, definition_id)
    }

    /// Archive as an explicit [`CatalogActor`].
    pub fn archive_as(
        &self,
        actor: &CatalogActor,
        definition_id: &str,
    ) -> Result<bool, CatalogError> {
        self.archive_with_actor(&actor.actor, &actor.owner_token, definition_id)
    }

    fn archive_with_actor(
        &self,
        actor: &str,
        owner_token: &str,
        definition_id: &str,
    ) -> Result<bool, CatalogError> {
        self.require_owner(owner_token)?;
        let mut connection = self.connection()?;
        let transaction = connection.transaction()?;
        let existing = transaction
            .query_row(
                "SELECT definition_json, archived FROM catalog_definitions
                 WHERE definition_id = ?1",
                [definition_id],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)? != 0)),
            )
            .optional()?;
        let Some((encoded, archived)) = existing else {
            return Err(CatalogError::NotFound(definition_id.to_owned()));
        };
        if archived {
            transaction.commit()?;
            return Ok(false);
        }
        let now = now_rfc3339()?;
        let definition = decode_definition(&encoded)?;
        transaction.execute(
            "UPDATE catalog_definitions
             SET archived = 1, archived_at = ?1, archived_by = ?2
             WHERE definition_id = ?3",
            params![&now, actor, definition_id],
        )?;
        append_event(&transaction, "catalog.archived", actor, &definition, &now)?;
        transaction.commit()?;
        Ok(true)
    }

    /// Alias for [`CatalogStore::archive`]; catalog history is never deleted.
    pub fn delete(&self, owner_token: &str, definition_id: &str) -> Result<bool, CatalogError> {
        self.archive(owner_token, definition_id)
    }

    /// Alias for [`CatalogStore::archive_as`]; catalog history is never deleted.
    pub fn delete_as(
        &self,
        actor: &CatalogActor,
        definition_id: &str,
    ) -> Result<bool, CatalogError> {
        self.archive_as(actor, definition_id)
    }

    fn require_owner(&self, owner_token: &str) -> Result<(), CatalogError> {
        match self.safe.verify_owner(owner_token) {
            Ok(true) => Ok(()),
            Ok(false) => Err(CatalogError::Unauthorized),
            Err(error) => Err(CatalogError::Safe(error)),
        }
    }

    fn connection(&self) -> Result<Connection, CatalogError> {
        Ok(open_initialized(
            &self.paths.platform_db(),
            PLATFORM_APPLICATION_ID,
            "platform",
        )?)
    }
}

struct ExistingDefinition {
    creator_core_uuid: Option<String>,
    created_at: String,
    revision: i64,
    archived: bool,
}

fn append_event(
    connection: &Connection,
    event_type: &str,
    actor: &str,
    definition: &ItemDefinition,
    created_at: &str,
) -> Result<(), CatalogError> {
    let revision = revision_value(definition.revision)?;
    let detail = serde_json::json!({
        "definition_id": &definition.definition_id,
        "revision": definition.revision,
        "category": definition.category.as_str(),
        "visibility": definition.visibility.as_str(),
        "creator_core_uuid": &definition.creator_core_uuid,
        "tags": &definition.tags,
    });
    let detail_json = serde_json::to_string(&detail)?;
    connection.execute(
        "INSERT INTO catalog_events(
            event_type, definition_id, revision, actor, detail_json, created_at
         ) VALUES(?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            event_type,
            &definition.definition_id,
            revision,
            actor,
            detail_json,
            created_at,
        ],
    )?;
    Ok(())
}

fn query_definitions(
    connection: &Connection,
    sql: &str,
    values: Vec<SqlValue>,
) -> Result<Vec<ItemDefinition>, CatalogError> {
    let mut statement = connection.prepare(sql)?;
    let rows = statement.query_map(params_from_iter(values.iter()), |row| {
        row.get::<_, String>(0)
    })?;
    let mut definitions = Vec::new();
    for row in rows {
        definitions.push(decode_definition(&row?)?);
    }
    Ok(definitions)
}

fn decode_definition(encoded: &str) -> Result<ItemDefinition, CatalogError> {
    let definition: ItemDefinition = serde_json::from_str(encoded)?;
    definition.validate().map_err(|_| CatalogError::Corrupt)?;
    Ok(definition)
}

fn validate_creator(creator: &Option<String>) -> Result<(), CatalogError> {
    if let Some(value) = creator.as_deref() {
        validate_creator_value(value)?;
    }
    Ok(())
}

fn validate_creator_value(value: &str) -> Result<(), CatalogError> {
    let parsed = Uuid::parse_str(value).map_err(|_| CatalogError::InvalidCreator)?;
    if parsed.to_string() != value {
        return Err(CatalogError::InvalidCreator);
    }
    Ok(())
}

fn validate_tag(tag: &str) -> Result<(), CatalogError> {
    if tag.trim().is_empty()
        || tag.chars().count() > MAX_TAG_CHARS
        || tag.chars().any(char::is_control)
    {
        return Err(CatalogError::InvalidFilter("tag"));
    }
    Ok(())
}

fn effective_limit(limit: Option<usize>) -> Result<usize, CatalogError> {
    match limit {
        None => Ok(CATALOG_DEFAULT_LIMIT),
        Some(0) => Err(CatalogError::InvalidFilter("limit")),
        Some(limit) if limit <= CATALOG_MAX_LIMIT => Ok(limit),
        Some(_) => Err(CatalogError::InvalidFilter("limit")),
    }
}

fn limit_value(limit: usize) -> Result<i64, CatalogError> {
    i64::try_from(limit).map_err(|_| CatalogError::InvalidFilter("limit"))
}

fn revision_value(revision: u64) -> Result<i64, CatalogError> {
    i64::try_from(revision).map_err(|_| CatalogError::InvalidRevision)
}

fn escape_like(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for character in text.chars() {
        if matches!(character, '%' | '_' | '\\') {
            escaped.push('\\');
        }
        escaped.push(character);
    }
    escaped
}
