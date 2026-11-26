//! Universal Aegis Core SDK: versioned capabilities, personality, and per-Core memory.
//!
//! The SDK is deliberately authority-free. Capabilities, personality traits, and
//! memory content are descriptive data only; none of them are authorization
//! inputs. Memory is stored in a separate `memory.db` inside the owning Core
//! directory and is bound to that Core by an immutable ownership row. Platform
//! and Safe databases are never opened from this crate.

#![forbid(unsafe_code)]

pub mod capability;
pub mod events;
pub mod memory;
pub mod personal;
pub mod personality;
pub mod possessions;

pub use capability::{
    CapabilityManifest, CoreCapability, ALL_CAPABILITIES, CAPABILITY_MANIFEST_SCHEMA,
    REQUIRED_CAPABILITIES,
};
pub use events::{
    apply_interaction_event, apply_interaction_event_at, interaction_influence, EventApplication,
    InteractionEvent, EVENT_LEDGER_TABLE, INTERACTION_DELTA, INTERACTION_INFLUENCE_SOURCE,
    INTERACTION_INFLUENCE_WEIGHT, MAX_EVENT_ID_CHARS,
};
pub use memory::{
    CoreMemoryStore, ImportMode, ImportOutcome, MemoryEntry, MemoryEvent, MemoryExport, MemoryKind,
    MemoryQuery, NewMemory, MAX_CONTENT_BYTES, MAX_EVENT_DETAIL_CHARS, MAX_IMPORT_CONTENT_BYTES,
    MAX_IMPORT_ENTRIES, MAX_IMPORT_EVENTS, MAX_KIND_CHARS, MAX_PROVENANCE_CHARS, MAX_QUERY_LIMIT,
    MAX_QUERY_TEXT_CHARS, MAX_SOURCE_CHARS, MAX_TAGS, MAX_TAG_CHARS, MEMORY_ENTRIES_TABLE,
    MEMORY_EVENTS_TABLE, MEMORY_EVENT_SCHEMA, MEMORY_EXPORT_SCHEMA, MEMORY_OWNERSHIP_TABLE,
    PERSONALITY_GROWTH_TABLE, PERSONALITY_TABLE,
};
pub use personal::{
    Activity, ActivityStatus, ContinuitySummary, EmotionDelta, EmotionDimensions,
    EmotionHistoryEntry, EmotionInfluence, EmotionState, OpenThread, OutfitDecisionRecord,
    PersonalStore, Routine, ACTIVITIES_TABLE, CONTINUITY_SUMMARY_SCHEMA, CONTINUITY_TABLE,
    EMOTION_DECAY_HALF_LIFE_SECONDS, EMOTION_HISTORY_TABLE, EMOTION_STATE_SCHEMA,
    EMOTION_STATE_TABLE, MAX_ACTIVITY_LIST, MAX_ELAPSED_OFFLINE_SECONDS, MAX_EMOTION_HISTORY_LIMIT,
    MAX_EMOTION_INFLUENCES, MAX_EXPERIENCE_CHARS, MAX_OPEN_THREADS, MAX_OPEN_THREAD_CHARS,
    MAX_OPEN_THREAD_LIST, MAX_OUTFIT_LIST, MAX_RECENT_EXPERIENCES, MAX_ROUTINE_LIST,
    MAX_SESSION_ID_CHARS, OPEN_THREADS_TABLE, OUTFIT_DECISIONS_TABLE, PERSONAL_APPLICATION_ID,
    PERSONAL_COMPONENT, PERSONAL_DB_FILE, PERSONAL_EVENTS_TABLE, PERSONAL_OWNERSHIP_TABLE,
    PERSONAL_SCHEMA, PERSONAL_SCHEMA_REVISION, PERSONAL_SCHEMA_TABLE, ROUTINES_TABLE,
};
pub use personality::{
    PersonalityProfile, MAX_GROWTH_POLICY_CHARS, MAX_TONE_CHARS, MAX_TRAITS, MAX_TRAIT_NAME_CHARS,
    MAX_VALUES, MAX_VALUE_CHARS, PERSONALITY_SCHEMA,
};
pub use possessions::{
    InstanceUpdate, NewInstance, NewObject, NewOutfit, NewPossessionEvent, ObjectUpdate,
    PossessionEvent, PossessionItem, PossessionsStore, TransferOffer, DEFAULT_ITEM_STATE,
    DEFAULT_OBJECT_STATE, DOCUMENTS_TABLE, ITEM_INSTANCES_TABLE, MAX_ACTION_CHARS,
    MAX_ENTITY_KIND_CHARS, MAX_EVENT_DETAIL_BYTES, MAX_EVENT_LIST, MAX_POSSESSION_LIST,
    MAX_PROPERTIES_BYTES, OUTFITS_TABLE, POSSESSIONS_APPLICATION_ID, POSSESSIONS_COMPONENT,
    POSSESSIONS_DB_FILE, POSSESSIONS_OWNERSHIP_TABLE, POSSESSIONS_SCHEMA,
    POSSESSIONS_SCHEMA_REVISION, POSSESSIONS_SCHEMA_TABLE, POSSESSION_EVENTS_TABLE,
    POSSESSION_EVENT_SCHEMA, TRANSFER_DELIVERY_PENDING, TRANSFER_OFFER_ACTION,
    TRANSFER_OFFER_STATUS, WORLD_OBJECTS_TABLE,
};

use std::path::Path;
use std::sync::LazyLock;

use aegis_contracts::core::CoreIdentity;
use aegis_core_storage::{CoreDatabase, CoreStorageLayout};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Stable SDK name used in descriptors and manifests.
pub const CORE_SDK_NAME: &str = "aegis-core-sdk";
/// SDK contract version.
pub const CORE_SDK_VERSION: &str = "0.1.0";
/// Schema identifier for the memory contract.
pub const MEMORY_CONTRACT_SCHEMA: &str = "aegis.core.memory.v1";
/// File name of the per-Core memory database.
pub const MEMORY_DB_FILE: &str = "memory.db";
/// SQLite application marker for a per-Core memory database ("AEGM").
pub const MEMORY_APPLICATION_ID: i64 = 0x4145_474D;
/// Component name registered in the shared `aegis_schema` table.
pub const MEMORY_COMPONENT: &str = "core_memory";
/// Schema identifier for the top-level SDK contract descriptor.
pub const CORE_SDK_CONTRACT_SCHEMA: &str = "aegis.core.sdk.v1";

/// Errors returned by the Core SDK.
#[derive(Debug, Error)]
pub enum SdkError {
    #[error("Core UUID is invalid")]
    InvalidCoreUuid,
    #[error("Core storage failed: {0}")]
    CoreStorage(#[from] aegis_core_storage::CoreStorageError),
    #[error("storage failed: {0}")]
    Storage(#[from] aegis_storage::StorageError),
    #[error("SQLite operation failed: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("serialization failed: {0}")]
    Json(#[from] serde_json::Error),
    #[error("memory database belongs to a different Core")]
    OwnershipMismatch,
    #[error("memory database ownership record is missing")]
    OwnershipMissing,
    #[error("memory database is outside its Core directory")]
    ForeignPath,
    #[error("memory database schema is unsupported")]
    UnsupportedMemorySchema,
    #[error("memory entry was not found")]
    NotFound,
    #[error("memory entry is deleted")]
    Deleted,
    #[error("memory entry is not archived")]
    NotArchived,
    #[error("invalid capability manifest")]
    InvalidCapabilityManifest,
    #[error("invalid personality profile: {0}")]
    InvalidPersonality(&'static str),
    #[error("personality profile schema is unsupported")]
    UnsupportedPersonalitySchema,
    #[error("invalid memory entry: {0}")]
    InvalidMemory(&'static str),
    #[error("invalid memory kind")]
    InvalidMemoryKind,
    #[error("invalid memory query: {0}")]
    InvalidQuery(&'static str),
    #[error("invalid import: {0}")]
    InvalidImport(&'static str),
    #[error("import schema is unsupported")]
    UnsupportedImportSchema,
    #[error("import belongs to a different Core")]
    ForeignCore,
    #[error("import conflicts with an existing memory entry: {0}")]
    ImportConflict(String),
    #[error("memory database is corrupt: {0}")]
    CorruptData(&'static str),
}

pub(crate) fn canonical_core_id(value: &str) -> Result<String, SdkError> {
    let parsed =
        aegis_contracts::core::validate_core_id(value).map_err(|_| SdkError::InvalidCoreUuid)?;
    Ok(parsed.hyphenated().to_string())
}

pub(crate) fn now_rfc3339() -> Result<String, SdkError> {
    Ok(aegis_storage::now_rfc3339()?)
}

pub(crate) fn chars_within(value: &str, max_chars: usize) -> bool {
    value.chars().count() <= max_chars
}

/// Versioned description of what the SDK surface provides.
///
/// The descriptor is metadata only. It never grants permissions or
/// authorization.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CoreSdkContract {
    pub schema: String,
    pub sdk_name: String,
    pub sdk_version: String,
    pub memory_schema: String,
    pub capability_schema: String,
    pub personality_schema: String,
    pub memory_export_schema: String,
    pub capabilities: Vec<CoreCapability>,
}

/// The process-wide, read-only SDK contract descriptor.
pub static CORE_SDK_CONTRACT: LazyLock<CoreSdkContract> = LazyLock::new(|| CoreSdkContract {
    schema: CORE_SDK_CONTRACT_SCHEMA.to_owned(),
    sdk_name: CORE_SDK_NAME.to_owned(),
    sdk_version: CORE_SDK_VERSION.to_owned(),
    memory_schema: MEMORY_CONTRACT_SCHEMA.to_owned(),
    capability_schema: CAPABILITY_MANIFEST_SCHEMA.to_owned(),
    personality_schema: PERSONALITY_SCHEMA.to_owned(),
    memory_export_schema: MEMORY_EXPORT_SCHEMA.to_owned(),
    capabilities: CoreCapability::all().to_vec(),
});

/// Return a fresh copy of the SDK contract descriptor.
pub fn core_sdk_contract() -> CoreSdkContract {
    CORE_SDK_CONTRACT.clone()
}

/// Combined SDK context for one Core.
#[derive(Debug)]
pub struct CoreSdkContext {
    pub identity: CoreIdentity,
    pub capabilities: CapabilityManifest,
    pub memory: CoreMemoryStore,
    pub personal: PersonalStore,
    pub possessions: PossessionsStore,
    pub personality: Option<PersonalityProfile>,
}

impl CoreSdkContext {
    /// Open the SDK surface for an existing Core.
    ///
    /// The Core must already exist (its `core.db` is verified read-only); this
    /// function only creates or opens the Core's own `memory.db`.
    pub fn open(root: impl AsRef<Path>, core_uuid: &str) -> Result<Self, SdkError> {
        let root = root.as_ref();
        let core_uuid = canonical_core_id(core_uuid)?;
        let layout = CoreStorageLayout::new(root)?;
        let snapshot = CoreDatabase::read_snapshot(&layout, &core_uuid)?;
        if snapshot.identity.core_uuid != core_uuid {
            return Err(SdkError::OwnershipMismatch);
        }
        let memory = CoreMemoryStore::open(root, &core_uuid)?;
        let personal = PersonalStore::open(root, &core_uuid)?;
        let possessions = PossessionsStore::open(root, &core_uuid)?;
        let capabilities = CapabilityManifest::full(&core_uuid)?;
        let personality = memory.load_personality()?;
        Ok(Self {
            identity: snapshot.identity,
            capabilities,
            memory,
            personal,
            possessions,
            personality,
        })
    }

    /// Borrow the immutable Core identity.
    pub fn identity(&self) -> &CoreIdentity {
        &self.identity
    }

    /// Borrow the capability manifest.
    pub fn capabilities(&self) -> &CapabilityManifest {
        &self.capabilities
    }

    /// Borrow the memory store.
    pub fn memory(&self) -> &CoreMemoryStore {
        &self.memory
    }

    /// Borrow the personal-state store bound to the same Core identity.
    pub fn personal(&self) -> Result<&PersonalStore, SdkError> {
        Ok(&self.personal)
    }

    /// Borrow the possessions store bound to the same Core identity.
    pub fn possessions(&self) -> Result<&PossessionsStore, SdkError> {
        Ok(&self.possessions)
    }

    /// Borrow the most recently loaded personality profile, if any.
    pub fn personality(&self) -> Option<&PersonalityProfile> {
        self.personality.as_ref()
    }

    /// Reload personality from the memory database.
    pub fn refresh_personality(&mut self) -> Result<Option<&PersonalityProfile>, SdkError> {
        self.personality = self.memory.load_personality()?;
        Ok(self.personality.as_ref())
    }
}
