//! Per-Core possessions: Core-authored items, outfits, documents, world
//! objects, and an append-only possession event ledger.
//!
//! Possessions live in a dedicated `<core>/possessions.db` bound to both the
//! Core UUID and the Core database instance UUID. The database is the personal
//! property record of exactly one Core: it never opens another Core's database
//! and never writes to `core.db`, `memory.db`, or `personal.db`.
//!
//! Ownership and wearing are separate concepts. `owner_core_uuid` is the
//! durable owner (always this Core for rows created here); `state` and
//! `holder_core_uuid` describe current use. Creating, wearing, or updating an
//! item never changes its owner. Cross-Core delivery requires the Phase 5
//! transfer subsystem; [`PossessionsStore::create_transfer_offer`] only records
//! an offer.

use std::collections::BTreeSet;
use std::ffi::OsStr;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use aegis_contracts::core::validate_core_id;
use aegis_contracts::living::{
    Document, DocumentFormat, ItemCategory, ItemInstance, Outfit, ProvenanceEntry, Visibility,
    WorldObject, LIVING_CONTRACT_VERSION, MAX_DOCUMENT_BODY_CHARS, MAX_ID_CHARS, MAX_LIST_ITEMS,
    MAX_NAME_CHARS, MAX_TAG_CHARS, MAX_TEXT_CHARS,
};
use aegis_core_storage::{CoreDatabase, CoreStorageLayout};
use aegis_storage::open_initialized;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use uuid::Uuid;

use crate::{canonical_core_id, chars_within, now_rfc3339, SdkError};

/// File name of the per-Core possessions database.
pub const POSSESSIONS_DB_FILE: &str = "possessions.db";
/// SQLite application marker for a per-Core possessions database ("AEGO").
pub const POSSESSIONS_APPLICATION_ID: i64 = 0x4145_474F;
/// Component name registered in the shared `aegis_schema` table.
pub const POSSESSIONS_COMPONENT: &str = "core_possessions";
/// Schema identifier for the possessions database.
pub const POSSESSIONS_SCHEMA: &str = "aegis.core.possessions.v1";
/// Schema identifier for one possession event.
pub const POSSESSION_EVENT_SCHEMA: &str = "aegis.core.possessions.event.v1";
/// Current possessions database schema revision.
pub const POSSESSIONS_SCHEMA_REVISION: u64 = 1;

/// Ownership table for the possessions database.
pub const POSSESSIONS_OWNERSHIP_TABLE: &str = "possessions_ownership";
/// Schema marker table for the possessions database.
pub const POSSESSIONS_SCHEMA_TABLE: &str = "possessions_schema";
/// Item instance table.
pub const ITEM_INSTANCES_TABLE: &str = "item_instances";
/// Outfit table.
pub const OUTFITS_TABLE: &str = "outfits";
/// Document table.
pub const DOCUMENTS_TABLE: &str = "documents";
/// World object table.
pub const WORLD_OBJECTS_TABLE: &str = "world_objects";
/// Append-only possession event table.
pub const POSSESSION_EVENTS_TABLE: &str = "possession_events";

/// Maximum number of rows returned by one possession listing.
pub const MAX_POSSESSION_LIST: usize = 256;
/// Maximum number of possession events returned by one listing.
pub const MAX_EVENT_LIST: usize = 500;
/// Maximum serialized size of an item/event JSON property payload.
pub const MAX_PROPERTIES_BYTES: usize = 64 * 1024;
/// Maximum serialized size of an event detail payload.
pub const MAX_EVENT_DETAIL_BYTES: usize = 16 * 1024;
/// Maximum character length of an event entity kind.
pub const MAX_ENTITY_KIND_CHARS: usize = 32;
/// Maximum character length of an event action.
pub const MAX_ACTION_CHARS: usize = 64;

/// Default state of a newly created item instance.
pub const DEFAULT_ITEM_STATE: &str = "stored";
/// Default state of a newly created world object.
pub const DEFAULT_OBJECT_STATE: &str = "present";
/// Event action recorded by a transfer offer.
pub const TRANSFER_OFFER_ACTION: &str = "transfer_offered";
/// Status recorded for an offer that has not been delivered.
pub const TRANSFER_OFFER_STATUS: &str = "offered";
/// Reason recorded on a transfer offer event.
pub const TRANSFER_DELIVERY_PENDING: &str = "pending_phase5_transfer_subsystem";

const SCHEMA_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS possessions_ownership (
    singleton INTEGER NOT NULL PRIMARY KEY CHECK (singleton = 1),
    core_uuid TEXT NOT NULL,
    database_instance_uuid TEXT NOT NULL,
    schema TEXT NOT NULL,
    created_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS possessions_schema (
    singleton INTEGER NOT NULL PRIMARY KEY CHECK (singleton = 1),
    schema TEXT NOT NULL,
    revision INTEGER NOT NULL CHECK (revision >= 1),
    created_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS item_instances (
    instance_id TEXT NOT NULL PRIMARY KEY,
    core_uuid TEXT NOT NULL,
    holder_core_uuid TEXT,
    definition_id TEXT,
    name TEXT NOT NULL,
    category TEXT NOT NULL,
    state TEXT NOT NULL,
    visibility TEXT NOT NULL,
    condition_percent INTEGER NOT NULL CHECK (condition_percent BETWEEN 0 AND 100),
    location TEXT,
    payload_json TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    revision INTEGER NOT NULL CHECK (revision >= 1)
);
CREATE INDEX IF NOT EXISTS item_instances_core_category_idx
    ON item_instances(core_uuid, category);
CREATE INDEX IF NOT EXISTS item_instances_core_state_idx
    ON item_instances(core_uuid, state);
CREATE INDEX IF NOT EXISTS item_instances_core_updated_idx
    ON item_instances(core_uuid, updated_at);
CREATE TABLE IF NOT EXISTS outfits (
    outfit_id TEXT NOT NULL PRIMARY KEY,
    core_uuid TEXT NOT NULL,
    name TEXT NOT NULL,
    item_instance_ids_json TEXT NOT NULL,
    visibility TEXT NOT NULL,
    archived INTEGER NOT NULL DEFAULT 0 CHECK (archived IN (0, 1)),
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    revision INTEGER NOT NULL CHECK (revision >= 1)
);
CREATE INDEX IF NOT EXISTS outfits_core_archived_idx
    ON outfits(core_uuid, archived, updated_at);
CREATE TABLE IF NOT EXISTS documents (
    document_id TEXT NOT NULL PRIMARY KEY,
    core_uuid TEXT NOT NULL,
    title TEXT NOT NULL,
    body TEXT NOT NULL,
    format TEXT NOT NULL,
    visibility TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    revision INTEGER NOT NULL CHECK (revision >= 1)
);
CREATE INDEX IF NOT EXISTS documents_core_updated_idx
    ON documents(core_uuid, updated_at);
CREATE TABLE IF NOT EXISTS world_objects (
    object_id TEXT NOT NULL PRIMARY KEY,
    core_uuid TEXT NOT NULL,
    definition_id TEXT,
    owner_core_uuid TEXT,
    holder_core_uuid TEXT,
    location TEXT NOT NULL,
    state TEXT NOT NULL,
    affordances_json TEXT NOT NULL,
    visibility TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    revision INTEGER NOT NULL CHECK (revision >= 1)
);
CREATE INDEX IF NOT EXISTS world_objects_core_updated_idx
    ON world_objects(core_uuid, updated_at);
CREATE TABLE IF NOT EXISTS possession_events (
    sequence INTEGER NOT NULL PRIMARY KEY AUTOINCREMENT,
    core_uuid TEXT NOT NULL,
    entity_kind TEXT NOT NULL,
    entity_id TEXT NOT NULL,
    action TEXT NOT NULL,
    detail_json TEXT NOT NULL,
    visibility TEXT NOT NULL,
    created_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS possession_events_core_idx
    ON possession_events(core_uuid, sequence);
CREATE TRIGGER IF NOT EXISTS possessions_ownership_no_update
BEFORE UPDATE ON possessions_ownership
BEGIN
    SELECT RAISE(ABORT, 'possessions_ownership is immutable');
END;
CREATE TRIGGER IF NOT EXISTS possessions_ownership_no_delete
BEFORE DELETE ON possessions_ownership
BEGIN
    SELECT RAISE(ABORT, 'possessions_ownership is immutable');
END;
CREATE TRIGGER IF NOT EXISTS possession_events_no_update
BEFORE UPDATE ON possession_events
BEGIN
    SELECT RAISE(ABORT, 'possession_events is append-only');
END;
CREATE TRIGGER IF NOT EXISTS possession_events_no_delete
BEFORE DELETE ON possession_events
BEGIN
    SELECT RAISE(ABORT, 'possession_events is append-only');
END;
"#;

/// One stored item instance together with the Core-authored identity metadata
/// that the shared [`ItemInstance`] contract does not carry.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PossessionItem {
    /// Shared living-environment item instance contract.
    pub instance: ItemInstance,
    /// Displays name authored by the owning Core.
    pub name: String,
    /// Free-form description authored by the owning Core.
    pub description: String,
    /// Category used for inventory grouping.
    pub category: ItemCategory,
    /// Extensible JSON properties. Bounded by [`MAX_PROPERTIES_BYTES`].
    pub properties: Value,
    /// Last update timestamp for this record.
    pub updated_at: String,
}

impl PossessionItem {
    /// Immutable instance identifier.
    pub fn instance_id(&self) -> &str {
        &self.instance.instance_id
    }

    /// Current use state (for example `stored`, `worn`, `held`).
    pub fn state(&self) -> &str {
        &self.instance.state
    }

    /// Durable owning Core.
    pub fn owner_core_uuid(&self) -> &str {
        &self.instance.owner_core_uuid
    }

    /// Current holder, when set.
    pub fn holder_core_uuid(&self) -> Option<&str> {
        self.instance.holder_core_uuid.as_deref()
    }

    /// Stored visibility.
    pub fn visibility(&self) -> Visibility {
        self.instance.visibility
    }
}

/// Input for [`PossessionsStore::create_instance`].
#[derive(Debug, Clone, PartialEq)]
pub struct NewInstance {
    /// Optional catalog definition this instance derives from. The catalog is
    /// not consulted in this milestone; the identifier is stored for a later
    /// catalog link.
    pub definition_id: Option<String>,
    /// Core-authored display name.
    pub name: String,
    /// Core-authored description.
    pub description: String,
    /// Inventory category.
    pub category: ItemCategory,
    /// Visibility, private by default.
    pub visibility: Visibility,
    /// Extensible JSON properties.
    pub properties: Value,
}

impl NewInstance {
    /// Start a private, catalog-free item authored by this Core.
    pub fn new(name: impl Into<String>, category: ItemCategory) -> Self {
        Self {
            definition_id: None,
            name: name.into(),
            description: String::new(),
            category,
            visibility: Visibility::Private,
            properties: Value::Object(serde_json::Map::new()),
        }
    }
}

/// Input for [`PossessionsStore::update_instance`].
#[derive(Debug, Clone, PartialEq)]
pub struct InstanceUpdate {
    /// Replacement display name.
    pub name: String,
    /// Replacement description.
    pub description: String,
    /// Replacement category.
    pub category: ItemCategory,
    /// Replacement visibility.
    pub visibility: Visibility,
    /// Replacement JSON properties.
    pub properties: Value,
    /// Replacement condition, 0 ..= 100.
    pub condition_percent: u8,
    /// Optional location label.
    pub location: Option<String>,
}

/// Input for [`PossessionsStore::create_outfit`].
#[derive(Debug, Clone, PartialEq)]
pub struct NewOutfit {
    /// Outfit name.
    pub name: String,
    /// Ordered item instance identifiers. Every item must be owned or held by
    /// this Core.
    pub item_instance_ids: Vec<String>,
    /// Visibility, private by default.
    pub visibility: Visibility,
}

impl NewOutfit {
    /// Start a private outfit from an ordered item list.
    pub fn new(name: impl Into<String>, item_instance_ids: Vec<String>) -> Self {
        Self {
            name: name.into(),
            item_instance_ids,
            visibility: Visibility::Private,
        }
    }
}

/// Input for [`PossessionsStore::create_object`].
#[derive(Debug, Clone, PartialEq)]
pub struct NewObject {
    /// Optional catalog definition identifier.
    pub definition_id: Option<String>,
    /// Location label.
    pub location: String,
    /// Initial object state.
    pub state: String,
    /// Declared affordances.
    pub affordances: Vec<String>,
    /// Visibility, private by default.
    pub visibility: Visibility,
}

impl NewObject {
    /// Start a private object at a location.
    pub fn new(location: impl Into<String>) -> Self {
        Self {
            definition_id: None,
            location: location.into(),
            state: DEFAULT_OBJECT_STATE.to_owned(),
            affordances: Vec::new(),
            visibility: Visibility::Private,
        }
    }
}

/// Input for [`PossessionsStore::update_object`].
#[derive(Debug, Clone, PartialEq)]
pub struct ObjectUpdate {
    /// Replacement location.
    pub location: String,
    /// Replacement state.
    pub state: String,
    /// Replacement affordances.
    pub affordances: Vec<String>,
    /// Replacement visibility.
    pub visibility: Visibility,
}

/// One append-only possession event.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PossessionEvent {
    /// Event schema identifier.
    pub schema: String,
    /// Monotonic sequence within this Core's database.
    pub sequence: u64,
    /// Owning Core.
    pub core_uuid: String,
    /// Entity kind (`item_instance`, `outfit`, `document`, `world_object`).
    pub entity_kind: String,
    /// Entity identifier.
    pub entity_id: String,
    /// Action name.
    pub action: String,
    /// JSON detail payload.
    pub detail: Value,
    /// Event visibility.
    pub visibility: Visibility,
    /// Creation timestamp.
    pub created_at: String,
}

/// Input for [`PossessionsStore::record_event`].
#[derive(Debug, Clone, PartialEq)]
pub struct NewPossessionEvent {
    /// Entity kind.
    pub entity_kind: String,
    /// Entity identifier.
    pub entity_id: String,
    /// Action name.
    pub action: String,
    /// JSON detail payload.
    pub detail: Value,
    /// Event visibility.
    pub visibility: Visibility,
}

/// A recorded (not delivered) cross-Core transfer offer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransferOffer {
    /// Offer identifier embedded in the event detail.
    pub offer_id: String,
    /// Item instance being offered.
    pub instance_id: String,
    /// Offering Core (always this store's Core).
    pub from_core_uuid: String,
    /// Target Core.
    pub to_core_uuid: String,
    /// Offer status; always [`TRANSFER_OFFER_STATUS`] in this milestone.
    pub status: String,
    /// Offer timestamp.
    pub created_at: String,
}

/// Writable per-Core possessions store.
///
/// Owns a `rusqlite::Connection` bound to `<core>/possessions.db`. It never
/// opens another Core's database and never writes to `core.db`, `memory.db`, or
/// `personal.db`.
pub struct PossessionsStore {
    core_uuid: String,
    database_instance_uuid: String,
    core_dir: PathBuf,
    database_path: PathBuf,
    connection: Connection,
}

impl std::fmt::Debug for PossessionsStore {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PossessionsStore")
            .field("core_uuid", &self.core_uuid)
            .field("database_instance_uuid", &self.database_instance_uuid)
            .field("database_path", &self.database_path)
            .finish_non_exhaustive()
    }
}

impl PossessionsStore {
    /// Open (or initialize) `<core>/possessions.db` for an existing Core.
    ///
    /// The Core database is read only; it supplies the immutable Core UUID and
    /// database instance UUID. An existing possessions database must carry
    /// exactly one ownership row matching both values.
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
        let possessions_path = core_dir.join(POSSESSIONS_DB_FILE);
        let existed = match fs::symlink_metadata(&possessions_path) {
            Ok(metadata) => {
                if metadata.file_type().is_symlink() || !metadata.is_file() {
                    return Err(SdkError::ForeignPath);
                }
                true
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => false,
            Err(error) => return Err(error.into()),
        };

        let connection = open_initialized(
            &possessions_path,
            POSSESSIONS_APPLICATION_ID,
            POSSESSIONS_COMPONENT,
        )?;

        let canonical_dir = fs::canonicalize(&core_dir)?;
        let canonical_db = fs::canonicalize(&possessions_path)?;
        if canonical_db.parent() != Some(canonical_dir.as_path())
            || canonical_db.file_name() != Some(OsStr::new(POSSESSIONS_DB_FILE))
        {
            return Err(SdkError::ForeignPath);
        }

        let has_ownership = table_exists(&connection, POSSESSIONS_OWNERSHIP_TABLE)?;
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
                "INSERT INTO possessions_ownership(
                    singleton, core_uuid, database_instance_uuid, schema, created_at
                 ) VALUES(1, ?1, ?2, ?3, ?4)",
                params![
                    core_uuid,
                    database_instance_uuid,
                    POSSESSIONS_SCHEMA,
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

    /// Canonical path of `possessions.db`.
    pub fn database_path(&self) -> &Path {
        &self.database_path
    }

    /// Create a new item instance owned by this Core.
    ///
    /// The instance may be authored from scratch; no catalog definition is
    /// required. Name, description, category, and properties are persisted.
    pub fn create_instance(&self, new: &NewInstance) -> Result<PossessionItem, SdkError> {
        validate_instance_input(new)?;

        let now = now_rfc3339()?;
        let mut instance = ItemInstance::new(&self.core_uuid, &now);
        instance.instance_id = Uuid::new_v4().to_string();
        instance.definition_id = new.definition_id.clone();
        instance.state = DEFAULT_ITEM_STATE.to_owned();
        instance.visibility = new.visibility;
        instance.provenance.push(ProvenanceEntry {
            at: now.clone(),
            actor: self.core_uuid.clone(),
            action: "created".to_owned(),
            note: String::new(),
        });
        instance
            .validate()
            .map_err(|_| SdkError::InvalidMemory("item instance failed validation"))?;

        let item = PossessionItem {
            instance,
            name: new.name.clone(),
            description: new.description.clone(),
            category: new.category,
            properties: new.properties.clone(),
            updated_at: now,
        };
        validate_stored_item(&item, &self.core_uuid)?;
        let payload_json = serde_json::to_string(&item)?;
        let revision = revision_value(item.instance.revision)?;

        let transaction = self.connection.unchecked_transaction()?;
        transaction.execute(
            "INSERT INTO item_instances(
                instance_id, core_uuid, holder_core_uuid, definition_id, name, category,
                state, visibility, condition_percent, location, payload_json,
                created_at, updated_at, revision
             ) VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
            params![
                item.instance.instance_id,
                self.core_uuid,
                item.instance.holder_core_uuid,
                item.instance.definition_id,
                item.name,
                item.category.as_str(),
                item.instance.state,
                item.instance.visibility.as_str(),
                i64::from(item.instance.condition_percent),
                item.instance.location,
                payload_json,
                item.instance.created_at,
                item.updated_at,
                revision
            ],
        )?;
        insert_event(
            &transaction,
            &self.core_uuid,
            "item_instance",
            &item.instance.instance_id,
            "created",
            &json!({
                "name": item.name,
                "category": item.category.as_str(),
                "visibility": item.instance.visibility.as_str(),
            }),
            Visibility::Private,
            &now_rfc3339()?,
        )?;
        transaction.commit()?;
        Ok(item)
    }

    /// Inventory for this Core, most recently updated first, capped at `limit`.
    pub fn list_inventory(&self, limit: usize) -> Result<Vec<PossessionItem>, SdkError> {
        let Some(limit) = limit_value(limit, MAX_POSSESSION_LIST)? else {
            return Ok(Vec::new());
        };
        let mut statement = self.connection.prepare(
            "SELECT payload_json FROM item_instances
             WHERE core_uuid = ?1
             ORDER BY updated_at DESC, instance_id ASC LIMIT ?2",
        )?;
        let rows = statement.query_map(params![self.core_uuid, limit], |row| {
            row.get::<_, String>(0)
        })?;
        let mut items = Vec::new();
        for row in rows {
            let payload = row?;
            items.push(decode_item(&payload, &self.core_uuid)?);
        }
        Ok(items)
    }

    /// Fetch one item instance owned by this Core.
    pub fn get_instance(&self, instance_id: &str) -> Result<Option<PossessionItem>, SdkError> {
        if !valid_id(instance_id) {
            return Ok(None);
        }
        read_item(&self.connection, &self.core_uuid, instance_id)
    }

    /// Change only the use state of an owned item (`stored`, `worn`, `held`,
    /// and Core-defined states are accepted). Ownership and holder are not
    /// changed by this operation.
    pub fn set_state(&self, instance_id: &str, state: &str) -> Result<PossessionItem, SdkError> {
        let state = state.trim();
        validate_tag(state, MAX_TAG_CHARS, "item state is invalid")?;

        let transaction = self.connection.unchecked_transaction()?;
        let mut item =
            read_item(&transaction, &self.core_uuid, instance_id)?.ok_or(SdkError::NotFound)?;
        let now = now_rfc3339()?;
        item.instance.state = state.to_owned();
        item.instance.revision = item.instance.revision.saturating_add(1);
        item.updated_at = now.clone();
        push_provenance(
            &mut item.instance,
            &self.core_uuid,
            "state_changed",
            state,
            &now,
        );
        validate_stored_item(&item, &self.core_uuid)?;
        let payload_json = serde_json::to_string(&item)?;
        let revision = revision_value(item.instance.revision)?;

        let changed = transaction.execute(
            "UPDATE item_instances
             SET state = ?3, visibility = ?4, payload_json = ?5, updated_at = ?6, revision = ?7
             WHERE core_uuid = ?1 AND instance_id = ?2",
            params![
                self.core_uuid,
                instance_id,
                item.instance.state,
                item.instance.visibility.as_str(),
                payload_json,
                item.updated_at,
                revision
            ],
        )?;
        if changed != 1 {
            return Err(SdkError::CorruptData("item instance row is missing"));
        }
        insert_event(
            &transaction,
            &self.core_uuid,
            "item_instance",
            instance_id,
            "state_changed",
            &json!({ "state": item.instance.state }),
            Visibility::Private,
            &now,
        )?;
        transaction.commit()?;
        Ok(item)
    }

    /// Update Core-authored item metadata for an item owned by this Core.
    ///
    /// Definition identity, owner, and holder are immutable here.
    pub fn update_instance(
        &self,
        instance_id: &str,
        update: &InstanceUpdate,
    ) -> Result<PossessionItem, SdkError> {
        validate_item_fields(&update.name, &update.description, &update.properties)?;
        if update.condition_percent > 100 {
            return Err(SdkError::InvalidMemory("item condition is invalid"));
        }
        if let Some(location) = &update.location {
            validate_name(location, "item location is invalid")?;
        }

        let transaction = self.connection.unchecked_transaction()?;
        let mut item =
            read_item(&transaction, &self.core_uuid, instance_id)?.ok_or(SdkError::NotFound)?;
        let now = now_rfc3339()?;
        item.name = update.name.clone();
        item.description = update.description.clone();
        item.category = update.category;
        item.properties = update.properties.clone();
        item.instance.visibility = update.visibility;
        item.instance.condition_percent = update.condition_percent;
        item.instance.location = update.location.clone();
        item.instance.revision = item.instance.revision.saturating_add(1);
        item.updated_at = now.clone();
        push_provenance(
            &mut item.instance,
            &self.core_uuid,
            "updated",
            update.category.as_str(),
            &now,
        );
        validate_stored_item(&item, &self.core_uuid)?;
        let payload_json = serde_json::to_string(&item)?;
        let revision = revision_value(item.instance.revision)?;

        let changed = transaction.execute(
            "UPDATE item_instances
             SET name = ?3, category = ?4, visibility = ?5, condition_percent = ?6,
                 location = ?7, payload_json = ?8, updated_at = ?9, revision = ?10
             WHERE core_uuid = ?1 AND instance_id = ?2",
            params![
                self.core_uuid,
                instance_id,
                item.name,
                item.category.as_str(),
                item.instance.visibility.as_str(),
                i64::from(item.instance.condition_percent),
                item.instance.location,
                payload_json,
                item.updated_at,
                revision
            ],
        )?;
        if changed != 1 {
            return Err(SdkError::CorruptData("item instance row is missing"));
        }
        insert_event(
            &transaction,
            &self.core_uuid,
            "item_instance",
            instance_id,
            "updated",
            &json!({ "name": item.name, "category": item.category.as_str() }),
            Visibility::Private,
            &now,
        )?;
        transaction.commit()?;
        Ok(item)
    }

    /// Create an outfit. Every referenced item must be owned or held by this
    /// Core; foreign or unknown identifiers are rejected.
    pub fn create_outfit(&self, new: &NewOutfit) -> Result<Outfit, SdkError> {
        validate_outfit_fields(&new.name, &new.item_instance_ids)?;
        self.validate_outfit_items(&new.item_instance_ids)?;

        let now = now_rfc3339()?;
        let outfit = Outfit {
            schema: LIVING_CONTRACT_VERSION.to_owned(),
            outfit_id: Uuid::new_v4().to_string(),
            core_uuid: self.core_uuid.clone(),
            name: new.name.clone(),
            item_instance_ids: new.item_instance_ids.clone(),
            visibility: new.visibility,
            created_at: now.clone(),
            updated_at: now.clone(),
            revision: 1,
        };
        outfit
            .validate()
            .map_err(|_| SdkError::InvalidMemory("outfit failed validation"))?;
        let item_ids_json = serde_json::to_string(&outfit.item_instance_ids)?;
        let revision = revision_value(outfit.revision)?;

        let transaction = self.connection.unchecked_transaction()?;
        transaction.execute(
            "INSERT INTO outfits(
                outfit_id, core_uuid, name, item_instance_ids_json, visibility, archived,
                created_at, updated_at, revision
             ) VALUES(?1, ?2, ?3, ?4, ?5, 0, ?6, ?7, ?8)",
            params![
                outfit.outfit_id,
                self.core_uuid,
                outfit.name,
                item_ids_json,
                outfit.visibility.as_str(),
                outfit.created_at,
                outfit.updated_at,
                revision
            ],
        )?;
        insert_event(
            &transaction,
            &self.core_uuid,
            "outfit",
            &outfit.outfit_id,
            "created",
            &json!({
                "name": outfit.name,
                "item_count": outfit.item_instance_ids.len(),
            }),
            Visibility::Private,
            &now,
        )?;
        transaction.commit()?;
        Ok(outfit)
    }

    /// Non-archived outfits for this Core, most recently updated first.
    pub fn list_outfits(&self, limit: usize) -> Result<Vec<Outfit>, SdkError> {
        let Some(limit) = limit_value(limit, MAX_POSSESSION_LIST)? else {
            return Ok(Vec::new());
        };
        let mut statement = self.connection.prepare(
            "SELECT outfit_id, name, item_instance_ids_json, visibility,
                    created_at, updated_at, revision
             FROM outfits
             WHERE core_uuid = ?1 AND archived = 0
             ORDER BY updated_at DESC, outfit_id ASC LIMIT ?2",
        )?;
        let rows = statement.query_map(params![self.core_uuid, limit], |row| {
            read_outfit_row(row, &self.core_uuid)
        })?;
        let mut outfits = Vec::new();
        for row in rows {
            outfits.push(row??);
        }
        Ok(outfits)
    }

    /// Replace an outfit's metadata and item list.
    pub fn update_outfit(
        &self,
        outfit_id: &str,
        name: &str,
        item_instance_ids: &[String],
        visibility: Visibility,
    ) -> Result<Outfit, SdkError> {
        if !valid_id(outfit_id) {
            return Err(SdkError::NotFound);
        }
        validate_outfit_fields(name, item_instance_ids)?;
        self.validate_outfit_items(item_instance_ids)?;

        let transaction = self.connection.unchecked_transaction()?;
        let existing =
            read_outfit(&transaction, &self.core_uuid, outfit_id)?.ok_or(SdkError::NotFound)?;
        let now = now_rfc3339()?;
        let outfit = Outfit {
            schema: LIVING_CONTRACT_VERSION.to_owned(),
            outfit_id: existing.outfit_id,
            core_uuid: self.core_uuid.clone(),
            name: name.to_owned(),
            item_instance_ids: item_instance_ids.to_vec(),
            visibility,
            created_at: existing.created_at,
            updated_at: now.clone(),
            revision: existing.revision.saturating_add(1),
        };
        outfit
            .validate()
            .map_err(|_| SdkError::InvalidMemory("outfit failed validation"))?;
        let item_ids_json = serde_json::to_string(&outfit.item_instance_ids)?;
        let revision = revision_value(outfit.revision)?;

        let changed = transaction.execute(
            "UPDATE outfits
             SET name = ?3, item_instance_ids_json = ?4, visibility = ?5,
                 updated_at = ?6, revision = ?7
             WHERE core_uuid = ?1 AND outfit_id = ?2 AND archived = 0",
            params![
                self.core_uuid,
                outfit.outfit_id,
                outfit.name,
                item_ids_json,
                outfit.visibility.as_str(),
                outfit.updated_at,
                revision
            ],
        )?;
        if changed != 1 {
            return Err(SdkError::NotFound);
        }
        insert_event(
            &transaction,
            &self.core_uuid,
            "outfit",
            &outfit.outfit_id,
            "updated",
            &json!({ "name": outfit.name, "item_count": outfit.item_instance_ids.len() }),
            Visibility::Private,
            &now,
        )?;
        transaction.commit()?;
        Ok(outfit)
    }

    /// Archive an outfit. Archived outfits are omitted from listings.
    pub fn archive_outfit(&self, outfit_id: &str) -> Result<(), SdkError> {
        if !valid_id(outfit_id) {
            return Err(SdkError::NotFound);
        }
        let transaction = self.connection.unchecked_transaction()?;
        let now = now_rfc3339()?;
        let changed = transaction.execute(
            "UPDATE outfits
             SET archived = 1, updated_at = ?3, revision = revision + 1
             WHERE core_uuid = ?1 AND outfit_id = ?2 AND archived = 0",
            params![self.core_uuid, outfit_id, now],
        )?;
        if changed == 0 {
            let exists: Option<i64> = transaction
                .query_row(
                    "SELECT 1 FROM outfits WHERE core_uuid = ?1 AND outfit_id = ?2",
                    params![self.core_uuid, outfit_id],
                    |row| row.get(0),
                )
                .optional()?;
            if exists.is_none() {
                return Err(SdkError::NotFound);
            }
        } else {
            insert_event(
                &transaction,
                &self.core_uuid,
                "outfit",
                outfit_id,
                "archived",
                &json!({ "outfit_id": outfit_id }),
                Visibility::Private,
                &now,
            )?;
        }
        transaction.commit()?;
        Ok(())
    }

    /// Create a document with a real persisted body.
    pub fn create_document(
        &self,
        title: &str,
        body: &str,
        format: DocumentFormat,
        visibility: Visibility,
    ) -> Result<Document, SdkError> {
        validate_document_fields(title, body)?;
        let now = now_rfc3339()?;
        let document = Document {
            schema: LIVING_CONTRACT_VERSION.to_owned(),
            document_id: Uuid::new_v4().to_string(),
            owner_core_uuid: self.core_uuid.clone(),
            title: title.to_owned(),
            body: body.to_owned(),
            format,
            visibility,
            created_at: now.clone(),
            updated_at: now.clone(),
            revision: 1,
        };
        validate_stored_document(&document, &self.core_uuid)?;
        let revision = revision_value(document.revision)?;

        let transaction = self.connection.unchecked_transaction()?;
        transaction.execute(
            "INSERT INTO documents(
                document_id, core_uuid, title, body, format, visibility,
                created_at, updated_at, revision
             ) VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                document.document_id,
                self.core_uuid,
                document.title,
                document.body,
                format_as_str(document.format),
                document.visibility.as_str(),
                document.created_at,
                document.updated_at,
                revision
            ],
        )?;
        insert_event(
            &transaction,
            &self.core_uuid,
            "document",
            &document.document_id,
            "created",
            &json!({ "title": document.title, "format": format_as_str(document.format) }),
            Visibility::Private,
            &now,
        )?;
        transaction.commit()?;
        Ok(document)
    }

    /// Fetch one document owned by this Core.
    pub fn get_document(&self, document_id: &str) -> Result<Option<Document>, SdkError> {
        if !valid_id(document_id) {
            return Ok(None);
        }
        read_document(&self.connection, &self.core_uuid, document_id)
    }

    /// Replace document content and increment its revision.
    pub fn update_document(
        &self,
        document_id: &str,
        title: &str,
        body: &str,
        format: DocumentFormat,
        visibility: Visibility,
    ) -> Result<Document, SdkError> {
        if !valid_id(document_id) {
            return Err(SdkError::NotFound);
        }
        validate_document_fields(title, body)?;

        let transaction = self.connection.unchecked_transaction()?;
        let existing =
            read_document(&transaction, &self.core_uuid, document_id)?.ok_or(SdkError::NotFound)?;
        let now = now_rfc3339()?;
        let document = Document {
            schema: LIVING_CONTRACT_VERSION.to_owned(),
            document_id: existing.document_id,
            owner_core_uuid: self.core_uuid.clone(),
            title: title.to_owned(),
            body: body.to_owned(),
            format,
            visibility,
            created_at: existing.created_at,
            updated_at: now.clone(),
            revision: existing.revision.saturating_add(1),
        };
        validate_stored_document(&document, &self.core_uuid)?;
        let revision = revision_value(document.revision)?;

        let changed = transaction.execute(
            "UPDATE documents
             SET title = ?3, body = ?4, format = ?5, visibility = ?6,
                 updated_at = ?7, revision = ?8
             WHERE core_uuid = ?1 AND document_id = ?2",
            params![
                self.core_uuid,
                document.document_id,
                document.title,
                document.body,
                format_as_str(document.format),
                document.visibility.as_str(),
                document.updated_at,
                revision
            ],
        )?;
        if changed != 1 {
            return Err(SdkError::CorruptData("document row is missing"));
        }
        insert_event(
            &transaction,
            &self.core_uuid,
            "document",
            &document.document_id,
            "updated",
            &json!({
                "title": document.title,
                "revision": document.revision,
                "body_chars": document.body.chars().count(),
            }),
            Visibility::Private,
            &now,
        )?;
        transaction.commit()?;
        Ok(document)
    }

    /// Documents owned by this Core, most recently updated first.
    pub fn list_documents(&self, limit: usize) -> Result<Vec<Document>, SdkError> {
        let Some(limit) = limit_value(limit, MAX_POSSESSION_LIST)? else {
            return Ok(Vec::new());
        };
        let mut statement = self.connection.prepare(
            "SELECT document_id, title, body, format, visibility,
                    created_at, updated_at, revision
             FROM documents
             WHERE core_uuid = ?1
             ORDER BY updated_at DESC, document_id ASC LIMIT ?2",
        )?;
        let rows = statement.query_map(params![self.core_uuid, limit], |row| {
            read_document_row(row, &self.core_uuid)
        })?;
        let mut documents = Vec::new();
        for row in rows {
            documents.push(row??);
        }
        Ok(documents)
    }

    /// Create a persistent world object owned and held by this Core.
    pub fn create_object(&self, new: &NewObject) -> Result<WorldObject, SdkError> {
        validate_object_fields(&new.location, &new.state, &new.affordances)?;
        if let Some(definition_id) = &new.definition_id {
            if !valid_id(definition_id) {
                return Err(SdkError::InvalidMemory("object definition id is invalid"));
            }
        }

        let now = now_rfc3339()?;
        let object = WorldObject {
            schema: LIVING_CONTRACT_VERSION.to_owned(),
            object_id: Uuid::new_v4().to_string(),
            definition_id: new.definition_id.clone(),
            owner_core_uuid: Some(self.core_uuid.clone()),
            holder_core_uuid: Some(self.core_uuid.clone()),
            location: new.location.clone(),
            state: new.state.clone(),
            affordances: new.affordances.clone(),
            visibility: new.visibility,
            created_at: now.clone(),
            updated_at: now.clone(),
            revision: 1,
        };
        object
            .validate()
            .map_err(|_| SdkError::InvalidMemory("world object failed validation"))?;
        let affordances_json = serde_json::to_string(&object.affordances)?;
        let revision = revision_value(object.revision)?;

        let transaction = self.connection.unchecked_transaction()?;
        transaction.execute(
            "INSERT INTO world_objects(
                object_id, core_uuid, definition_id, owner_core_uuid, holder_core_uuid,
                location, state, affordances_json, visibility, created_at, updated_at,
                revision
             ) VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            params![
                object.object_id,
                self.core_uuid,
                object.definition_id,
                object.owner_core_uuid,
                object.holder_core_uuid,
                object.location,
                object.state,
                affordances_json,
                object.visibility.as_str(),
                object.created_at,
                object.updated_at,
                revision
            ],
        )?;
        insert_event(
            &transaction,
            &self.core_uuid,
            "world_object",
            &object.object_id,
            "created",
            &json!({ "location": object.location, "state": object.state }),
            Visibility::Private,
            &now,
        )?;
        transaction.commit()?;
        Ok(object)
    }

    /// Update state, location, affordances, or visibility of an owned object.
    pub fn update_object(
        &self,
        object_id: &str,
        update: &ObjectUpdate,
    ) -> Result<WorldObject, SdkError> {
        if !valid_id(object_id) {
            return Err(SdkError::NotFound);
        }
        validate_object_fields(&update.location, &update.state, &update.affordances)?;

        let transaction = self.connection.unchecked_transaction()?;
        let existing =
            read_object(&transaction, &self.core_uuid, object_id)?.ok_or(SdkError::NotFound)?;
        let now = now_rfc3339()?;
        let object = WorldObject {
            schema: LIVING_CONTRACT_VERSION.to_owned(),
            object_id: existing.object_id,
            definition_id: existing.definition_id,
            owner_core_uuid: Some(self.core_uuid.clone()),
            holder_core_uuid: Some(self.core_uuid.clone()),
            location: update.location.clone(),
            state: update.state.clone(),
            affordances: update.affordances.clone(),
            visibility: update.visibility,
            created_at: existing.created_at,
            updated_at: now.clone(),
            revision: existing.revision.saturating_add(1),
        };
        object
            .validate()
            .map_err(|_| SdkError::InvalidMemory("world object failed validation"))?;
        let affordances_json = serde_json::to_string(&object.affordances)?;
        let revision = revision_value(object.revision)?;

        let changed = transaction.execute(
            "UPDATE world_objects
             SET location = ?3, state = ?4, affordances_json = ?5, visibility = ?6,
                 updated_at = ?7, revision = ?8
             WHERE core_uuid = ?1 AND object_id = ?2",
            params![
                self.core_uuid,
                object.object_id,
                object.location,
                object.state,
                affordances_json,
                object.visibility.as_str(),
                object.updated_at,
                revision
            ],
        )?;
        if changed != 1 {
            return Err(SdkError::CorruptData("world object row is missing"));
        }
        insert_event(
            &transaction,
            &self.core_uuid,
            "world_object",
            &object.object_id,
            "updated",
            &json!({ "location": object.location, "state": object.state }),
            Visibility::Private,
            &now,
        )?;
        transaction.commit()?;
        Ok(object)
    }

    /// World objects owned by this Core, most recently updated first.
    pub fn list_objects(&self, limit: usize) -> Result<Vec<WorldObject>, SdkError> {
        let Some(limit) = limit_value(limit, MAX_POSSESSION_LIST)? else {
            return Ok(Vec::new());
        };
        let mut statement = self.connection.prepare(
            "SELECT object_id, definition_id, owner_core_uuid, holder_core_uuid,
                    location, state, affordances_json, visibility,
                    created_at, updated_at, revision
             FROM world_objects
             WHERE core_uuid = ?1
             ORDER BY updated_at DESC, object_id ASC LIMIT ?2",
        )?;
        let rows = statement.query_map(params![self.core_uuid, limit], |row| {
            read_object_row(row, &self.core_uuid)
        })?;
        let mut objects = Vec::new();
        for row in rows {
            objects.push(row??);
        }
        Ok(objects)
    }

    /// Append one possession event. Events are immutable once written.
    pub fn record_event(&self, new: &NewPossessionEvent) -> Result<PossessionEvent, SdkError> {
        validate_event_fields(&new.entity_kind, &new.entity_id, &new.action, &new.detail)?;
        let now = now_rfc3339()?;
        let sequence = insert_event(
            &self.connection,
            &self.core_uuid,
            &new.entity_kind,
            &new.entity_id,
            &new.action,
            &new.detail,
            new.visibility,
            &now,
        )?;
        Ok(PossessionEvent {
            schema: POSSESSION_EVENT_SCHEMA.to_owned(),
            sequence,
            core_uuid: self.core_uuid.clone(),
            entity_kind: new.entity_kind.clone(),
            entity_id: new.entity_id.clone(),
            action: new.action.clone(),
            detail: new.detail.clone(),
            visibility: new.visibility,
            created_at: now,
        })
    }

    /// Possession events, newest first, capped at `limit`.
    pub fn list_events(&self, limit: usize) -> Result<Vec<PossessionEvent>, SdkError> {
        let Some(limit) = limit_value(limit, MAX_EVENT_LIST)? else {
            return Ok(Vec::new());
        };
        let mut statement = self.connection.prepare(
            "SELECT sequence, entity_kind, entity_id, action, detail_json,
                    visibility, created_at
             FROM possession_events
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

    /// Record an offer to transfer an owned item to another Core.
    ///
    /// This does **not** transfer ownership, change the holder, move the item,
    /// or create a copy. Delivery requires the Phase 5 transfer subsystem; the
    /// returned status is always [`TRANSFER_OFFER_STATUS`].
    pub fn create_transfer_offer(
        &self,
        instance_id: &str,
        to_core_uuid: &str,
    ) -> Result<TransferOffer, SdkError> {
        let target = canonical_core_id(to_core_uuid)?;
        if target == self.core_uuid {
            return Err(SdkError::InvalidMemory(
                "transfer target must be a different Core",
            ));
        }
        let _item = self.get_instance(instance_id)?.ok_or(SdkError::NotFound)?;

        let offer_id = Uuid::new_v4().to_string();
        let event = self.record_event(&NewPossessionEvent {
            entity_kind: "item_instance".to_owned(),
            entity_id: instance_id.to_owned(),
            action: TRANSFER_OFFER_ACTION.to_owned(),
            detail: json!({
                "offer_id": offer_id,
                "from_core_uuid": self.core_uuid,
                "to_core_uuid": target,
                "status": TRANSFER_OFFER_STATUS,
                "delivery": TRANSFER_DELIVERY_PENDING,
            }),
            visibility: Visibility::Private,
        })?;
        Ok(TransferOffer {
            offer_id,
            instance_id: instance_id.to_owned(),
            from_core_uuid: self.core_uuid.clone(),
            to_core_uuid: target,
            status: TRANSFER_OFFER_STATUS.to_owned(),
            created_at: event.created_at,
        })
    }

    fn validate_outfit_items(&self, item_instance_ids: &[String]) -> Result<(), SdkError> {
        for instance_id in item_instance_ids {
            let row: Option<(String, Option<String>)> = self
                .connection
                .query_row(
                    "SELECT core_uuid, holder_core_uuid FROM item_instances
                     WHERE instance_id = ?1",
                    params![instance_id],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .optional()?;
            let Some((owner, holder)) = row else {
                return Err(SdkError::InvalidMemory(
                    "outfit references an unknown item instance",
                ));
            };
            let owned = owner == self.core_uuid;
            let held = holder.as_deref() == Some(self.core_uuid.as_str());
            if !owned && !held {
                return Err(SdkError::ForeignCore);
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
            "SELECT schema, revision FROM possessions_schema WHERE singleton = 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    match existing {
        None => {
            let revision = revision_value(POSSESSIONS_SCHEMA_REVISION)?;
            connection.execute(
                "INSERT INTO possessions_schema(singleton, schema, revision, created_at)
                 VALUES(1, ?1, ?2, ?3)",
                params![POSSESSIONS_SCHEMA, revision, now_rfc3339()?],
            )?;
        }
        Some((schema, revision)) => {
            let revision = u64::try_from(revision)
                .map_err(|_| SdkError::CorruptData("possessions schema revision is invalid"))?;
            if schema != POSSESSIONS_SCHEMA || revision > POSSESSIONS_SCHEMA_REVISION {
                return Err(SdkError::CorruptData("possessions schema is unsupported"));
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
    let count: i64 =
        connection.query_row("SELECT COUNT(*) FROM possessions_ownership", [], |row| {
            row.get(0)
        })?;
    if count != 1 {
        return Err(SdkError::OwnershipMissing);
    }
    let row: Option<(String, String, String)> = connection
        .query_row(
            "SELECT core_uuid, database_instance_uuid, schema
             FROM possessions_ownership WHERE singleton = 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()?;
    let Some((stored_core, stored_instance, schema)) = row else {
        return Err(SdkError::OwnershipMissing);
    };
    if schema != POSSESSIONS_SCHEMA {
        return Err(SdkError::CorruptData("possessions schema is unsupported"));
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

fn read_item(
    connection: &Connection,
    expected_core: &str,
    instance_id: &str,
) -> Result<Option<PossessionItem>, SdkError> {
    let payload: Option<String> = connection
        .query_row(
            "SELECT payload_json FROM item_instances
             WHERE core_uuid = ?1 AND instance_id = ?2",
            params![expected_core, instance_id],
            |row| row.get(0),
        )
        .optional()?;
    payload
        .map(|payload| decode_item(&payload, expected_core))
        .transpose()
}

fn decode_item(payload: &str, expected_core: &str) -> Result<PossessionItem, SdkError> {
    let item: PossessionItem = serde_json::from_str(payload)
        .map_err(|_| SdkError::CorruptData("stored item payload is invalid"))?;
    validate_stored_item(&item, expected_core)?;
    Ok(item)
}

fn validate_stored_item(item: &PossessionItem, expected_core: &str) -> Result<(), SdkError> {
    if item.instance.owner_core_uuid != expected_core {
        return Err(SdkError::OwnershipMismatch);
    }
    item.instance
        .validate()
        .map_err(|_| SdkError::CorruptData("stored item instance is invalid"))?;
    if item.name.trim().is_empty()
        || !bounded(&item.name, MAX_NAME_CHARS)
        || !bounded_multiline(&item.description, MAX_TEXT_CHARS)
        || !bounded(&item.updated_at, 64)
        || json_size(&item.properties)? > MAX_PROPERTIES_BYTES
    {
        return Err(SdkError::CorruptData("stored item payload is invalid"));
    }
    Ok(())
}

fn read_outfit(
    connection: &Connection,
    expected_core: &str,
    outfit_id: &str,
) -> Result<Option<Outfit>, SdkError> {
    let row = connection
        .query_row(
            "SELECT outfit_id, name, item_instance_ids_json, visibility,
                    created_at, updated_at, revision
             FROM outfits
             WHERE core_uuid = ?1 AND outfit_id = ?2 AND archived = 0",
            params![expected_core, outfit_id],
            |row| read_outfit_row(row, expected_core),
        )
        .optional()?;
    row.transpose()
}

fn read_outfit_row(
    row: &rusqlite::Row<'_>,
    expected_core: &str,
) -> Result<Result<Outfit, SdkError>, rusqlite::Error> {
    let outfit_id: String = row.get(0)?;
    let name: String = row.get(1)?;
    let item_ids_json: String = row.get(2)?;
    let visibility: String = row.get(3)?;
    let created_at: String = row.get(4)?;
    let updated_at: String = row.get(5)?;
    let revision: i64 = row.get(6)?;
    Ok((|| {
        let item_instance_ids: Vec<String> = serde_json::from_str(&item_ids_json)
            .map_err(|_| SdkError::CorruptData("stored outfit item list is invalid"))?;
        let outfit = Outfit {
            schema: LIVING_CONTRACT_VERSION.to_owned(),
            outfit_id,
            core_uuid: expected_core.to_owned(),
            name,
            item_instance_ids,
            visibility: parse_visibility(&visibility)
                .ok_or(SdkError::CorruptData("stored outfit visibility is invalid"))?,
            created_at,
            updated_at,
            revision: u64::try_from(revision)
                .map_err(|_| SdkError::CorruptData("stored outfit revision is invalid"))?,
        };
        outfit
            .validate()
            .map_err(|_| SdkError::CorruptData("stored outfit is invalid"))?;
        Ok(outfit)
    })())
}

fn read_document(
    connection: &Connection,
    expected_core: &str,
    document_id: &str,
) -> Result<Option<Document>, SdkError> {
    let row = connection
        .query_row(
            "SELECT document_id, title, body, format, visibility,
                    created_at, updated_at, revision
             FROM documents
             WHERE core_uuid = ?1 AND document_id = ?2",
            params![expected_core, document_id],
            |row| read_document_row(row, expected_core),
        )
        .optional()?;
    row.transpose()
}

fn read_document_row(
    row: &rusqlite::Row<'_>,
    expected_core: &str,
) -> Result<Result<Document, SdkError>, rusqlite::Error> {
    let document_id: String = row.get(0)?;
    let title: String = row.get(1)?;
    let body: String = row.get(2)?;
    let format: String = row.get(3)?;
    let visibility: String = row.get(4)?;
    let created_at: String = row.get(5)?;
    let updated_at: String = row.get(6)?;
    let revision: i64 = row.get(7)?;
    Ok((|| {
        let document = Document {
            schema: LIVING_CONTRACT_VERSION.to_owned(),
            document_id,
            owner_core_uuid: expected_core.to_owned(),
            title,
            body,
            format: parse_format(&format)
                .ok_or(SdkError::CorruptData("stored document format is invalid"))?,
            visibility: parse_visibility(&visibility).ok_or(SdkError::CorruptData(
                "stored document visibility is invalid",
            ))?,
            created_at,
            updated_at,
            revision: u64::try_from(revision)
                .map_err(|_| SdkError::CorruptData("stored document revision is invalid"))?,
        };
        validate_stored_document(&document, expected_core)?;
        Ok(document)
    })())
}

fn validate_stored_document(document: &Document, expected_core: &str) -> Result<(), SdkError> {
    if document.schema != LIVING_CONTRACT_VERSION
        || document.owner_core_uuid != expected_core
        || !valid_id(&document.document_id)
        || !valid_id(&document.owner_core_uuid)
        || document.title.trim().is_empty()
        || !bounded(&document.title, MAX_NAME_CHARS)
        || !bounded_multiline(&document.body, MAX_DOCUMENT_BODY_CHARS)
        || !bounded(&document.created_at, 64)
        || !bounded(&document.updated_at, 64)
        || document.revision == 0
    {
        return Err(SdkError::CorruptData("stored document is invalid"));
    }
    Ok(())
}

fn read_object(
    connection: &Connection,
    expected_core: &str,
    object_id: &str,
) -> Result<Option<WorldObject>, SdkError> {
    let row = connection
        .query_row(
            "SELECT object_id, definition_id, owner_core_uuid, holder_core_uuid,
                    location, state, affordances_json, visibility,
                    created_at, updated_at, revision
             FROM world_objects
             WHERE core_uuid = ?1 AND object_id = ?2",
            params![expected_core, object_id],
            |row| read_object_row(row, expected_core),
        )
        .optional()?;
    row.transpose()
}

fn read_object_row(
    row: &rusqlite::Row<'_>,
    expected_core: &str,
) -> Result<Result<WorldObject, SdkError>, rusqlite::Error> {
    let object_id: String = row.get(0)?;
    let definition_id: Option<String> = row.get(1)?;
    let owner_core_uuid: Option<String> = row.get(2)?;
    let holder_core_uuid: Option<String> = row.get(3)?;
    let location: String = row.get(4)?;
    let state: String = row.get(5)?;
    let affordances_json: String = row.get(6)?;
    let visibility: String = row.get(7)?;
    let created_at: String = row.get(8)?;
    let updated_at: String = row.get(9)?;
    let revision: i64 = row.get(10)?;
    Ok((|| {
        if owner_core_uuid.as_deref() != Some(expected_core) {
            return Err(SdkError::OwnershipMismatch);
        }
        let affordances: Vec<String> = serde_json::from_str(&affordances_json)
            .map_err(|_| SdkError::CorruptData("stored object affordances are invalid"))?;
        let object = WorldObject {
            schema: LIVING_CONTRACT_VERSION.to_owned(),
            object_id,
            definition_id,
            owner_core_uuid,
            holder_core_uuid,
            location,
            state,
            affordances,
            visibility: parse_visibility(&visibility)
                .ok_or(SdkError::CorruptData("stored object visibility is invalid"))?,
            created_at,
            updated_at,
            revision: u64::try_from(revision)
                .map_err(|_| SdkError::CorruptData("stored object revision is invalid"))?,
        };
        object
            .validate()
            .map_err(|_| SdkError::CorruptData("stored world object is invalid"))?;
        Ok(object)
    })())
}

fn read_event_row(
    row: &rusqlite::Row<'_>,
    expected_core: &str,
) -> Result<Result<PossessionEvent, SdkError>, rusqlite::Error> {
    let sequence: i64 = row.get(0)?;
    let entity_kind: String = row.get(1)?;
    let entity_id: String = row.get(2)?;
    let action: String = row.get(3)?;
    let detail_json: String = row.get(4)?;
    let visibility: String = row.get(5)?;
    let created_at: String = row.get(6)?;
    Ok((|| {
        let detail: Value = serde_json::from_str(&detail_json)
            .map_err(|_| SdkError::CorruptData("stored event detail is invalid"))?;
        Ok(PossessionEvent {
            schema: POSSESSION_EVENT_SCHEMA.to_owned(),
            sequence: u64::try_from(sequence)
                .map_err(|_| SdkError::CorruptData("stored event sequence is invalid"))?,
            core_uuid: expected_core.to_owned(),
            entity_kind,
            entity_id,
            action,
            detail,
            visibility: parse_visibility(&visibility)
                .ok_or(SdkError::CorruptData("stored event visibility is invalid"))?,
            created_at,
        })
    })())
}

#[allow(clippy::too_many_arguments)]
fn insert_event(
    connection: &Connection,
    core_uuid: &str,
    entity_kind: &str,
    entity_id: &str,
    action: &str,
    detail: &Value,
    visibility: Visibility,
    created_at: &str,
) -> Result<u64, SdkError> {
    let detail_json = serde_json::to_string(detail)?;
    connection.execute(
        "INSERT INTO possession_events(
            core_uuid, entity_kind, entity_id, action, detail_json, visibility, created_at
         ) VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            core_uuid,
            entity_kind,
            entity_id,
            action,
            detail_json,
            visibility.as_str(),
            created_at
        ],
    )?;
    let sequence = connection.last_insert_rowid();
    u64::try_from(sequence).map_err(|_| SdkError::CorruptData("event sequence is invalid"))
}

fn push_provenance(instance: &mut ItemInstance, actor: &str, action: &str, note: &str, at: &str) {
    if instance.provenance.len() >= MAX_LIST_ITEMS {
        return;
    }
    instance.provenance.push(ProvenanceEntry {
        at: at.to_owned(),
        actor: actor.to_owned(),
        action: action.to_owned(),
        note: note.to_owned(),
    });
}

fn validate_instance_input(new: &NewInstance) -> Result<(), SdkError> {
    if let Some(definition_id) = &new.definition_id {
        if !valid_id(definition_id) {
            return Err(SdkError::InvalidMemory("item definition id is invalid"));
        }
    }
    validate_item_fields(&new.name, &new.description, &new.properties)
}

fn validate_item_fields(name: &str, description: &str, properties: &Value) -> Result<(), SdkError> {
    validate_name(name, "item name is invalid")?;
    if !bounded_multiline(description, MAX_TEXT_CHARS) {
        return Err(SdkError::InvalidMemory("item description is invalid"));
    }
    if json_size(properties)? > MAX_PROPERTIES_BYTES {
        return Err(SdkError::InvalidMemory("item properties are too large"));
    }
    Ok(())
}

fn validate_outfit_fields(name: &str, item_instance_ids: &[String]) -> Result<(), SdkError> {
    validate_name(name, "outfit name is invalid")?;
    if item_instance_ids.len() > MAX_LIST_ITEMS {
        return Err(SdkError::InvalidMemory("outfit has too many items"));
    }
    let mut seen = BTreeSet::new();
    for instance_id in item_instance_ids {
        if !valid_id(instance_id) {
            return Err(SdkError::InvalidMemory("outfit item id is invalid"));
        }
        if !seen.insert(instance_id.as_str()) {
            return Err(SdkError::InvalidMemory("outfit item ids must be unique"));
        }
    }
    Ok(())
}

fn validate_document_fields(title: &str, body: &str) -> Result<(), SdkError> {
    validate_name(title, "document title is invalid")?;
    if !bounded_multiline(body, MAX_DOCUMENT_BODY_CHARS) {
        return Err(SdkError::InvalidMemory("document body is invalid"));
    }
    Ok(())
}

fn validate_object_fields(
    location: &str,
    state: &str,
    affordances: &[String],
) -> Result<(), SdkError> {
    validate_name(location, "object location is invalid")?;
    validate_tag(state, MAX_TAG_CHARS, "object state is invalid")?;
    if affordances.len() > MAX_LIST_ITEMS {
        return Err(SdkError::InvalidMemory("object has too many affordances"));
    }
    for affordance in affordances {
        if affordance.trim().is_empty() || !bounded(affordance, MAX_TAG_CHARS) {
            return Err(SdkError::InvalidMemory("object affordance is invalid"));
        }
    }
    Ok(())
}

fn validate_event_fields(
    entity_kind: &str,
    entity_id: &str,
    action: &str,
    detail: &Value,
) -> Result<(), SdkError> {
    validate_tag(
        entity_kind,
        MAX_ENTITY_KIND_CHARS,
        "event entity kind is invalid",
    )?;
    if !valid_id(entity_id) {
        return Err(SdkError::InvalidMemory("event entity id is invalid"));
    }
    validate_tag(action, MAX_ACTION_CHARS, "event action is invalid")?;
    if json_size(detail)? > MAX_EVENT_DETAIL_BYTES {
        return Err(SdkError::InvalidMemory("event detail is too large"));
    }
    Ok(())
}

fn validate_name(value: &str, message: &'static str) -> Result<(), SdkError> {
    if value.trim().is_empty() || !bounded(value, MAX_NAME_CHARS) {
        return Err(SdkError::InvalidMemory(message));
    }
    Ok(())
}

fn validate_tag(value: &str, maximum: usize, message: &'static str) -> Result<(), SdkError> {
    if value.trim().is_empty() || !bounded(value, maximum) {
        return Err(SdkError::InvalidMemory(message));
    }
    Ok(())
}

fn valid_id(value: &str) -> bool {
    !value.is_empty()
        && chars_within(value, MAX_ID_CHARS)
        && value.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.' | ':')
        })
}

fn bounded(value: &str, maximum: usize) -> bool {
    !value.chars().any(char::is_control) && value.chars().count() <= maximum
}

fn bounded_multiline(value: &str, maximum: usize) -> bool {
    !value
        .chars()
        .any(|character| character.is_control() && !matches!(character, '\n' | '\r' | '\t'))
        && value.chars().count() <= maximum
}

fn json_size(value: &Value) -> Result<usize, SdkError> {
    Ok(serde_json::to_string(value)?.len())
}

fn parse_visibility(value: &str) -> Option<Visibility> {
    match value {
        "private" => Some(Visibility::Private),
        "shared" => Some(Visibility::Shared),
        "owner_only" => Some(Visibility::OwnerOnly),
        "public_catalog" => Some(Visibility::PublicCatalog),
        _ => None,
    }
}

fn parse_format(value: &str) -> Option<DocumentFormat> {
    match value {
        "plain_text" => Some(DocumentFormat::PlainText),
        "markdown" => Some(DocumentFormat::Markdown),
        "note" => Some(DocumentFormat::Note),
        "letter" => Some(DocumentFormat::Letter),
        _ => None,
    }
}

fn format_as_str(format: DocumentFormat) -> &'static str {
    match format {
        DocumentFormat::PlainText => "plain_text",
        DocumentFormat::Markdown => "markdown",
        DocumentFormat::Note => "note",
        DocumentFormat::Letter => "letter",
    }
}

fn revision_value(revision: u64) -> Result<i64, SdkError> {
    i64::try_from(revision)
        .map_err(|_| SdkError::CorruptData("possessions revision is out of range"))
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
