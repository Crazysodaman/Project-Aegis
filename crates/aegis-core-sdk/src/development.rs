//! Per-Core personality development: evidence, preferences, reflections, and
//! owner-approved personality proposals.
//!
//! Development state is additive inside the existing `<core>/personal.db`. The
//! database remains bound to both the Core UUID and the Core database instance
//! UUID through the immutable `personal_ownership` row, so a copied database
//! from another Core (or another Core database instance) is rejected.
//!
//! This module never changes Core identity or the Constitution. Applying a
//! proposal only merges expressive personality values and traits into the
//! existing append-only personality growth history in `memory.db`, and only
//! after the Safe owner proof is verified.

use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use aegis_contracts::core::validate_core_id;
use aegis_contracts::living::{
    PersonalityProposal, PersonalityProposalStatus, Preference, Reflection,
    LIVING_CONTRACT_VERSION, MAX_ID_CHARS, MAX_REFLECTION_SUMMARY_CHARS,
};
use aegis_safe::SafeStore;
use aegis_storage::{open_initialized, StoragePaths};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use uuid::Uuid;

use crate::memory::CoreMemoryStore;
use crate::personal::{
    PersonalStore, MAX_EXPERIENCE_CHARS, PERSONAL_APPLICATION_ID, PERSONAL_COMPONENT,
    PERSONAL_SCHEMA,
};
use crate::personality::{PersonalityProfile, MAX_TRAITS, MAX_VALUES};
use crate::{canonical_core_id, chars_within, now_rfc3339, SdkError};

/// Schema identifier for the development subsystem.
pub const DEVELOPMENT_SCHEMA: &str = "aegis.core.development.v1";
/// Schema identifier for one development event.
pub const DEVELOPMENT_EVENT_SCHEMA: &str = "aegis.core.development.event.v1";
/// Table holding current preferences.
pub const DEVELOPMENT_PREFERENCES_TABLE: &str = "development_preferences";
/// Append-only evidence table.
pub const DEVELOPMENT_EVIDENCE_TABLE: &str = "development_evidence";
/// Table holding stored reflections.
pub const DEVELOPMENT_REFLECTIONS_TABLE: &str = "development_reflections";
/// Table holding personality proposals.
pub const DEVELOPMENT_PROPOSALS_TABLE: &str = "development_proposals";
/// Append-only development event table.
pub const DEVELOPMENT_EVENTS_TABLE: &str = "development_events";

/// Maximum number of preferences, reflections, or proposals returned by one
/// listing.
pub const MAX_DEVELOPMENT_LIST: usize = 256;
/// Maximum number of development events returned by one listing.
pub const MAX_DEVELOPMENT_EVENT_LIST: usize = 500;
/// Maximum serialized size of a development event detail payload.
pub const MAX_DEVELOPMENT_EVENT_DETAIL_BYTES: usize = 16 * 1024;

/// Maximum character length of an evidence source.
pub const MAX_DEVELOPMENT_SOURCE_CHARS: usize = 128;
/// Number of strongest preferences summarized by one reflection.
pub const MAX_REFLECTION_PREFERENCES: usize = 5;
/// Base exponential-moving-average weight for preference evidence.
pub const PREFERENCE_BASE_ALPHA: f32 = 0.2;
/// Additional exponential-moving-average weight scaled by evidence weight.
pub const PREFERENCE_WEIGHT_ALPHA: f32 = 0.6;

const SCHEMA_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS development_preferences (
    core_uuid TEXT NOT NULL,
    preference_key TEXT NOT NULL,
    value REAL NOT NULL CHECK (value >= -1.0 AND value <= 1.0),
    confidence REAL NOT NULL CHECK (confidence >= 0.0 AND confidence <= 1.0),
    evidence_count INTEGER NOT NULL CHECK (evidence_count >= 0),
    updated_at TEXT NOT NULL,
    revision INTEGER NOT NULL CHECK (revision >= 1),
    PRIMARY KEY (core_uuid, preference_key)
);
CREATE INDEX IF NOT EXISTS development_preferences_core_idx
    ON development_preferences(core_uuid, updated_at);
CREATE TABLE IF NOT EXISTS development_evidence (
    sequence INTEGER NOT NULL PRIMARY KEY AUTOINCREMENT,
    core_uuid TEXT NOT NULL,
    preference_key TEXT,
    summary TEXT NOT NULL,
    valence REAL NOT NULL CHECK (valence >= -1.0 AND valence <= 1.0),
    weight REAL NOT NULL CHECK (weight >= 0.0 AND weight <= 1.0),
    source TEXT NOT NULL,
    created_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS development_evidence_core_idx
    ON development_evidence(core_uuid, sequence);
CREATE TABLE IF NOT EXISTS development_reflections (
    reflection_id TEXT NOT NULL PRIMARY KEY,
    core_uuid TEXT NOT NULL,
    summary TEXT NOT NULL,
    created_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS development_reflections_core_idx
    ON development_reflections(core_uuid, created_at);
CREATE TABLE IF NOT EXISTS development_proposals (
    proposal_id TEXT NOT NULL PRIMARY KEY,
    core_uuid TEXT NOT NULL,
    status TEXT NOT NULL,
    payload_json TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    revision INTEGER NOT NULL CHECK (revision >= 1)
);
CREATE INDEX IF NOT EXISTS development_proposals_core_idx
    ON development_proposals(core_uuid, status, updated_at);
CREATE TABLE IF NOT EXISTS development_events (
    sequence INTEGER NOT NULL PRIMARY KEY AUTOINCREMENT,
    core_uuid TEXT NOT NULL,
    entity_kind TEXT NOT NULL,
    entity_id TEXT NOT NULL,
    action TEXT NOT NULL,
    detail_json TEXT NOT NULL,
    created_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS development_events_core_idx
    ON development_events(core_uuid, sequence);
CREATE TRIGGER IF NOT EXISTS development_events_no_update
BEFORE UPDATE ON development_events
BEGIN
    SELECT RAISE(ABORT, 'development_events is append-only');
END;
CREATE TRIGGER IF NOT EXISTS development_events_no_delete
BEFORE DELETE ON development_events
BEGIN
    SELECT RAISE(ABORT, 'development_events is append-only');
END;
CREATE TRIGGER IF NOT EXISTS development_evidence_no_update
BEFORE UPDATE ON development_evidence
BEGIN
    SELECT RAISE(ABORT, 'development_evidence is append-only');
END;
CREATE TRIGGER IF NOT EXISTS development_evidence_no_delete
BEFORE DELETE ON development_evidence
BEGIN
    SELECT RAISE(ABORT, 'development_evidence is append-only');
END;
"#;

/// One append-only development event.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DevelopmentEvent {
    /// Event schema identifier.
    pub schema: String,
    /// Monotonic sequence within this Core's personal database.
    pub sequence: u64,
    /// Owning Core.
    pub core_uuid: String,
    /// Entity kind (`experience`, `preference`, `reflection`, `proposal`,
    /// `personality`).
    pub entity_kind: String,
    /// Entity identifier.
    pub entity_id: String,
    /// Action name.
    pub action: String,
    /// JSON detail payload.
    pub detail: Value,
    /// Creation timestamp.
    pub created_at: String,
}

/// Writable per-Core development store backed by `<core>/personal.db`.
///
/// The store holds a connection to the Core's own personal database plus a
/// [`PersonalStore`] used to record real experiences. It never opens another
/// Core's database and never writes to `core.db`, `memory.db` (except the
/// append-only personality history during apply/revert), or any Platform or
/// Safe database.
pub struct DevelopmentStore {
    core_uuid: String,
    database_instance_uuid: String,
    root: PathBuf,
    core_dir: PathBuf,
    database_path: PathBuf,
    connection: Connection,
    personal: PersonalStore,
}

impl std::fmt::Debug for DevelopmentStore {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("DevelopmentStore")
            .field("core_uuid", &self.core_uuid)
            .field("database_instance_uuid", &self.database_instance_uuid)
            .field("database_path", &self.database_path)
            .finish_non_exhaustive()
    }
}

impl DevelopmentStore {
    /// Open (or initialize) the development tables in `<core>/personal.db`.
    ///
    /// The Core must already exist. Ownership is verified against the immutable
    /// `personal_ownership` row (Core UUID and database instance UUID) before
    /// any development table is touched.
    pub fn open(root: impl AsRef<Path>, core_uuid: &str) -> Result<Self, SdkError> {
        let root = StoragePaths::new(root.as_ref())?.root().to_path_buf();
        let core_uuid = canonical_core_id(core_uuid)?;
        let personal = PersonalStore::open(&root, &core_uuid)?;
        let database_instance_uuid = personal.database_instance_uuid().to_owned();
        let core_dir = personal.core_dir().to_path_buf();
        let database_path = personal.database_path().to_path_buf();

        let connection =
            open_initialized(&database_path, PERSONAL_APPLICATION_ID, PERSONAL_COMPONENT)?;
        verify_personal_ownership(&connection, &core_uuid, &database_instance_uuid)?;
        initialize_schema(&connection)?;

        Ok(Self {
            core_uuid,
            database_instance_uuid,
            root,
            core_dir,
            database_path,
            connection,
            personal,
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

    /// Canonical path of the underlying `personal.db`.
    pub fn database_path(&self) -> &Path {
        &self.database_path
    }

    /// Record a bounded experience as evidence and as a real
    /// [`PersonalStore`] experience.
    pub fn record_experience(
        &self,
        summary: &str,
        valence: f32,
        weight: f32,
        source: &str,
    ) -> Result<(), SdkError> {
        validate_experience(summary)?;
        validate_source(source)?;
        validate_valence(valence)?;
        validate_weight(weight)?;

        self.personal.record_experience(summary)?;

        let now = now_rfc3339()?;
        let transaction = self.connection.unchecked_transaction()?;
        insert_evidence(
            &transaction,
            &self.core_uuid,
            &EvidenceInput {
                key: None,
                summary,
                valence,
                weight,
                source,
                created_at: &now,
            },
        )?;
        insert_event(
            &transaction,
            &self.core_uuid,
            "experience",
            "",
            "recorded",
            &json!({
                "summary": summary,
                "valence": valence,
                "weight": weight,
                "source": source,
            }),
            &now,
        )?;
        transaction.commit()?;
        Ok(())
    }

    /// Record preference evidence and update the preference with an
    /// exponential moving average, confidence growth, and an incremented
    /// evidence count.
    pub fn record_preference_evidence(
        &self,
        key: &str,
        valence: f32,
        weight: f32,
        source: &str,
    ) -> Result<Preference, SdkError> {
        if !valid_id(key) {
            return Err(SdkError::InvalidMemory("preference key is invalid"));
        }
        validate_source(source)?;
        validate_valence(valence)?;
        validate_weight(weight)?;

        let now = now_rfc3339()?;
        let transaction = self.connection.unchecked_transaction()?;
        let existing = read_preference(&transaction, &self.core_uuid, key)?;
        let preference = match existing {
            Some(previous) => {
                let alpha = (PREFERENCE_BASE_ALPHA
                    + PREFERENCE_WEIGHT_ALPHA * weight.clamp(0.0, 1.0))
                .clamp(0.05, 0.8);
                let value = (previous.value * (1.0 - alpha) + valence * alpha).clamp(-1.0, 1.0);
                let confidence = (previous.confidence + (1.0 - previous.confidence) * alpha * 0.5)
                    .clamp(0.0, 1.0);
                Preference {
                    schema: LIVING_CONTRACT_VERSION.to_owned(),
                    core_uuid: self.core_uuid.clone(),
                    key: key.to_owned(),
                    value,
                    confidence,
                    evidence_count: previous.evidence_count.saturating_add(1),
                    updated_at: now.clone(),
                    revision: previous.revision.saturating_add(1),
                }
            }
            None => Preference {
                schema: LIVING_CONTRACT_VERSION.to_owned(),
                core_uuid: self.core_uuid.clone(),
                key: key.to_owned(),
                value: valence.clamp(-1.0, 1.0),
                confidence: weight.clamp(0.0, 1.0),
                evidence_count: 1,
                updated_at: now.clone(),
                revision: 1,
            },
        };
        preference
            .validate()
            .map_err(|_| SdkError::InvalidMemory("preference failed validation"))?;

        upsert_preference(&transaction, &preference)?;
        let summary = format!("Preference evidence for {key}");
        insert_evidence(
            &transaction,
            &self.core_uuid,
            &EvidenceInput {
                key: Some(key),
                summary: &summary,
                valence,
                weight,
                source,
                created_at: &now,
            },
        )?;
        insert_event(
            &transaction,
            &self.core_uuid,
            "preference",
            key,
            "evidence_recorded",
            &json!({
                "key": key,
                "valence": valence,
                "weight": weight,
                "value": preference.value,
                "confidence": preference.confidence,
                "evidence_count": preference.evidence_count,
            }),
            &now,
        )?;
        transaction.commit()?;
        Ok(preference)
    }

    /// Fetch one preference owned by this Core.
    pub fn get_preference(&self, key: &str) -> Result<Option<Preference>, SdkError> {
        if !valid_id(key) {
            return Ok(None);
        }
        read_preference(&self.connection, &self.core_uuid, key)
    }

    /// Preferences owned by this Core, most recently updated first.
    pub fn list_preferences(&self, limit: usize) -> Result<Vec<Preference>, SdkError> {
        let Some(limit) = limit_value(limit, MAX_DEVELOPMENT_LIST)? else {
            return Ok(Vec::new());
        };
        let mut statement = self.connection.prepare(
            "SELECT preference_key, value, confidence, evidence_count, updated_at, revision
             FROM development_preferences
             WHERE core_uuid = ?1
             ORDER BY updated_at DESC, preference_key ASC LIMIT ?2",
        )?;
        let mut rows = statement.query(params![self.core_uuid, limit])?;
        let mut preferences = Vec::new();
        while let Some(row) = rows.next()? {
            let key: String = row.get(0)?;
            let value: f64 = row.get(1)?;
            let confidence: f64 = row.get(2)?;
            let evidence_count: i64 = row.get(3)?;
            let updated_at: String = row.get(4)?;
            let revision: i64 = row.get(5)?;
            let preference = Preference {
                schema: LIVING_CONTRACT_VERSION.to_owned(),
                core_uuid: self.core_uuid.clone(),
                key,
                value: raw_f32(value),
                confidence: raw_f32(confidence),
                evidence_count: u64::try_from(evidence_count).map_err(|_| {
                    SdkError::CorruptData("stored preference evidence count is invalid")
                })?,
                updated_at,
                revision: u64::try_from(revision)
                    .map_err(|_| SdkError::CorruptData("stored preference revision is invalid"))?,
            };
            preference
                .validate()
                .map_err(|_| SdkError::CorruptData("stored preference is invalid"))?;
            preferences.push(preference);
        }
        Ok(preferences)
    }

    /// Build and store a bounded reflection from the strongest persisted
    /// preferences.
    pub fn reflect(&self) -> Result<Reflection, SdkError> {
        let mut preferences = self.list_preferences(MAX_DEVELOPMENT_LIST)?;
        preferences.sort_by(|left, right| {
            strength(right)
                .partial_cmp(&strength(left))
                .unwrap_or(Ordering::Equal)
                .then_with(|| left.key.cmp(&right.key))
        });

        let summary = if preferences.is_empty() {
            "No established preferences yet.".to_owned()
        } else {
            let parts = preferences
                .iter()
                .take(MAX_REFLECTION_PREFERENCES)
                .map(|preference| {
                    format!(
                        "{} ({:+.2}, confidence {:.2})",
                        preference.key, preference.value, preference.confidence
                    )
                })
                .collect::<Vec<_>>();
            format!(
                "Reflection from {} preference(s): {}.",
                preferences.len(),
                parts.join("; ")
            )
        };
        let summary = truncate_chars(&summary, MAX_REFLECTION_SUMMARY_CHARS);

        let now = now_rfc3339()?;
        let reflection = Reflection {
            schema: LIVING_CONTRACT_VERSION.to_owned(),
            reflection_id: Uuid::new_v4().to_string(),
            core_uuid: self.core_uuid.clone(),
            summary,
            created_at: now.clone(),
        };
        reflection
            .validate()
            .map_err(|_| SdkError::InvalidMemory("reflection failed validation"))?;

        let transaction = self.connection.unchecked_transaction()?;
        transaction.execute(
            "INSERT INTO development_reflections(reflection_id, core_uuid, summary, created_at)
             VALUES(?1, ?2, ?3, ?4)",
            params![
                reflection.reflection_id,
                self.core_uuid,
                reflection.summary,
                reflection.created_at
            ],
        )?;
        insert_event(
            &transaction,
            &self.core_uuid,
            "reflection",
            &reflection.reflection_id,
            "created",
            &json!({
                "reflection_id": reflection.reflection_id,
                "preference_count": preferences.len(),
            }),
            &now,
        )?;
        transaction.commit()?;
        Ok(reflection)
    }

    /// Validate and store a proposed personality change. Nothing is applied
    /// until the owner approves it.
    pub fn propose_personality_change(
        &self,
        rationale: &str,
        values: Vec<String>,
        traits: BTreeMap<String, f32>,
    ) -> Result<PersonalityProposal, SdkError> {
        let now = now_rfc3339()?;
        let proposal = PersonalityProposal {
            schema: LIVING_CONTRACT_VERSION.to_owned(),
            proposal_id: Uuid::new_v4().to_string(),
            core_uuid: self.core_uuid.clone(),
            rationale: rationale.to_owned(),
            proposed_values: values,
            proposed_traits: traits,
            status: PersonalityProposalStatus::Proposed,
            created_at: now.clone(),
            updated_at: now,
            revision: 1,
        };
        proposal
            .validate()
            .map_err(|_| SdkError::InvalidMemory("personality proposal failed validation"))?;
        validate_proposal_traits(&proposal)?;

        let payload_json = serde_json::to_string(&proposal)?;
        let revision = revision_value(proposal.revision)?;
        let transaction = self.connection.unchecked_transaction()?;
        transaction.execute(
            "INSERT INTO development_proposals(
                proposal_id, core_uuid, status, payload_json, created_at, updated_at, revision
             ) VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                proposal.proposal_id,
                self.core_uuid,
                proposal.status.as_str(),
                payload_json,
                proposal.created_at,
                proposal.updated_at,
                revision
            ],
        )?;
        insert_event(
            &transaction,
            &self.core_uuid,
            "proposal",
            &proposal.proposal_id,
            "proposed",
            &json!({
                "proposal_id": proposal.proposal_id,
                "value_count": proposal.proposed_values.len(),
                "trait_count": proposal.proposed_traits.len(),
            }),
            &proposal.created_at,
        )?;
        transaction.commit()?;
        Ok(proposal)
    }

    /// Fetch one personality proposal owned by this Core.
    pub fn get_proposal(&self, proposal_id: &str) -> Result<Option<PersonalityProposal>, SdkError> {
        if !valid_id(proposal_id) {
            return Ok(None);
        }
        let payload: Option<String> = self
            .connection
            .query_row(
                "SELECT payload_json FROM development_proposals
                 WHERE core_uuid = ?1 AND proposal_id = ?2",
                params![self.core_uuid, proposal_id],
                |row| row.get(0),
            )
            .optional()?;
        payload
            .map(|payload| decode_proposal(&payload, &self.core_uuid))
            .transpose()
    }

    /// Personality proposals owned by this Core, most recently updated first.
    pub fn list_proposals(&self, limit: usize) -> Result<Vec<PersonalityProposal>, SdkError> {
        let Some(limit) = limit_value(limit, MAX_DEVELOPMENT_LIST)? else {
            return Ok(Vec::new());
        };
        let mut statement = self.connection.prepare(
            "SELECT payload_json FROM development_proposals
             WHERE core_uuid = ?1
             ORDER BY updated_at DESC, proposal_id ASC LIMIT ?2",
        )?;
        let rows = statement.query_map(params![self.core_uuid, limit], |row| {
            row.get::<_, String>(0)
        })?;
        let mut proposals = Vec::new();
        for row in rows {
            proposals.push(decode_proposal(&row?, &self.core_uuid)?);
        }
        Ok(proposals)
    }

    /// Reject a proposed personality change. Requires the Safe owner proof.
    pub fn reject_proposal(
        &self,
        owner_token: &str,
        proposal_id: &str,
    ) -> Result<PersonalityProposal, SdkError> {
        self.verify_owner(owner_token)?;
        let proposal = self.require_proposal(proposal_id)?;
        if proposal.status != PersonalityProposalStatus::Proposed {
            return Err(SdkError::InvalidMemory("proposal is not proposed"));
        }
        self.update_proposal_status(proposal, PersonalityProposalStatus::Rejected, "rejected")
    }

    /// Apply a proposed personality change after verifying the Safe owner
    /// proof.
    ///
    /// The current personality profile is loaded from `memory.db`, proposed
    /// values and traits are merged within contract bounds, and the result is
    /// stored through the append-only personality growth history. Applying a
    /// proposal never touches Core identity or the Constitution.
    pub fn apply_proposal(
        &self,
        owner_token: &str,
        proposal_id: &str,
    ) -> Result<PersonalityProfile, SdkError> {
        self.verify_owner(owner_token)?;
        let proposal = self.require_proposal(proposal_id)?;
        if proposal.status != PersonalityProposalStatus::Proposed {
            return Err(SdkError::InvalidMemory("proposal is not proposed"));
        }
        validate_proposal_traits(&proposal)?;

        let memory = CoreMemoryStore::open(&self.root, &self.core_uuid)?;
        let mut profile = match memory.load_personality()? {
            Some(profile) => profile,
            None => {
                let neutral = PersonalityProfile::neutral(&self.core_uuid)?;
                memory.store_personality(&neutral)?;
                neutral
            }
        };

        for value in &proposal.proposed_values {
            if profile.values.iter().any(|existing| existing == value) {
                continue;
            }
            if profile.values.len() >= MAX_VALUES {
                return Err(SdkError::InvalidMemory("too many personality values"));
            }
            profile.values.push(value.clone());
        }
        for (name, value) in &proposal.proposed_traits {
            if !value.is_finite() || !(-1.0..=1.0).contains(value) {
                return Err(SdkError::InvalidMemory(
                    "proposed trait value is out of range",
                ));
            }
            if !profile.traits.contains_key(name) && profile.traits.len() >= MAX_TRAITS {
                return Err(SdkError::InvalidMemory("too many personality traits"));
            }
            profile.traits.insert(name.clone(), f64::from(*value));
        }
        profile
            .validate()
            .map_err(|_| SdkError::InvalidMemory("merged personality failed validation"))?;
        let stored = memory.store_personality(&profile)?;
        self.update_proposal_status(proposal, PersonalityProposalStatus::Applied, "applied")?;
        Ok(stored)
    }

    /// Restore the previous personality growth revision by appending it as a
    /// new revision. Requires the Safe owner proof.
    pub fn revert_personality(&self, owner_token: &str) -> Result<PersonalityProfile, SdkError> {
        self.verify_owner(owner_token)?;
        let memory = CoreMemoryStore::open(&self.root, &self.core_uuid)?;
        let growth = memory.personality_growth()?;
        if growth.len() < 2 {
            return Err(SdkError::InvalidMemory(
                "no previous personality revision exists",
            ));
        }
        let previous = growth[growth.len() - 2].clone();
        let restored = memory.store_personality(&previous)?;

        let now = now_rfc3339()?;
        let transaction = self.connection.unchecked_transaction()?;
        insert_event(
            &transaction,
            &self.core_uuid,
            "personality",
            "",
            "reverted",
            &json!({
                "previous_revision": previous.revision,
                "restored_revision": restored.revision,
            }),
            &now,
        )?;
        transaction.commit()?;
        Ok(restored)
    }

    /// Development events, newest first, capped at `limit`.
    pub fn list_events(&self, limit: usize) -> Result<Vec<DevelopmentEvent>, SdkError> {
        let Some(limit) = limit_value(limit, MAX_DEVELOPMENT_EVENT_LIST)? else {
            return Ok(Vec::new());
        };
        let mut statement = self.connection.prepare(
            "SELECT sequence, entity_kind, entity_id, action, detail_json, created_at
             FROM development_events
             WHERE core_uuid = ?1
             ORDER BY sequence DESC LIMIT ?2",
        )?;
        let mut rows = statement.query(params![self.core_uuid, limit])?;
        let mut events = Vec::new();
        while let Some(row) = rows.next()? {
            let sequence: i64 = row.get(0)?;
            let detail_json: String = row.get(4)?;
            let detail: Value = serde_json::from_str(&detail_json)
                .map_err(|_| SdkError::CorruptData("stored development event detail is invalid"))?;
            events.push(DevelopmentEvent {
                schema: DEVELOPMENT_EVENT_SCHEMA.to_owned(),
                sequence: u64::try_from(sequence).map_err(|_| {
                    SdkError::CorruptData("stored development event sequence is invalid")
                })?,
                core_uuid: self.core_uuid.clone(),
                entity_kind: row.get(1)?,
                entity_id: row.get(2)?,
                action: row.get(3)?,
                detail,
                created_at: row.get(5)?,
            });
        }
        Ok(events)
    }

    fn require_proposal(&self, proposal_id: &str) -> Result<PersonalityProposal, SdkError> {
        self.get_proposal(proposal_id)?.ok_or(SdkError::NotFound)
    }

    fn update_proposal_status(
        &self,
        mut proposal: PersonalityProposal,
        status: PersonalityProposalStatus,
        action: &str,
    ) -> Result<PersonalityProposal, SdkError> {
        proposal.status = status;
        proposal.updated_at = now_rfc3339()?;
        proposal.revision = proposal.revision.saturating_add(1);
        proposal
            .validate()
            .map_err(|_| SdkError::InvalidMemory("personality proposal failed validation"))?;
        let payload_json = serde_json::to_string(&proposal)?;
        let revision = revision_value(proposal.revision)?;

        let transaction = self.connection.unchecked_transaction()?;
        let changed = transaction.execute(
            "UPDATE development_proposals
             SET status = ?3, payload_json = ?4, updated_at = ?5, revision = ?6
             WHERE core_uuid = ?1 AND proposal_id = ?2",
            params![
                self.core_uuid,
                proposal.proposal_id,
                status.as_str(),
                payload_json,
                proposal.updated_at,
                revision
            ],
        )?;
        if changed != 1 {
            return Err(SdkError::NotFound);
        }
        insert_event(
            &transaction,
            &self.core_uuid,
            "proposal",
            &proposal.proposal_id,
            action,
            &json!({
                "proposal_id": proposal.proposal_id,
                "status": status.as_str(),
                "revision": proposal.revision,
            }),
            &proposal.updated_at,
        )?;
        transaction.commit()?;
        Ok(proposal)
    }

    fn verify_owner(&self, owner_token: &str) -> Result<(), SdkError> {
        let paths = StoragePaths::new(&self.root)?;
        let safe = SafeStore::new(paths);
        if safe.verify_owner(owner_token)? {
            Ok(())
        } else {
            Err(SdkError::Unauthorized)
        }
    }
}

fn initialize_schema(connection: &Connection) -> Result<(), SdkError> {
    connection.execute_batch(SCHEMA_SQL)?;
    Ok(())
}

fn verify_personal_ownership(
    connection: &Connection,
    expected_core: &str,
    expected_instance: &str,
) -> Result<(), SdkError> {
    let count: i64 =
        connection.query_row("SELECT COUNT(*) FROM personal_ownership", [], |row| {
            row.get(0)
        })?;
    if count != 1 {
        return Err(SdkError::OwnershipMissing);
    }
    let row: Option<(String, String, String)> = connection
        .query_row(
            "SELECT core_uuid, database_instance_uuid, schema
             FROM personal_ownership WHERE singleton = 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()?;
    let Some((stored_core, stored_instance, schema)) = row else {
        return Err(SdkError::OwnershipMissing);
    };
    if schema != PERSONAL_SCHEMA {
        return Err(SdkError::CorruptData("personal schema is unsupported"));
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

fn validate_proposal_traits(proposal: &PersonalityProposal) -> Result<(), SdkError> {
    let mut profile = PersonalityProfile::neutral(&proposal.core_uuid)?;
    profile.values = proposal.proposed_values.clone();
    profile.traits = proposal
        .proposed_traits
        .iter()
        .map(|(name, value)| (name.clone(), f64::from(*value)))
        .collect();
    profile.validate().map_err(|_| {
        SdkError::InvalidMemory("personality proposal contains invalid traits or values")
    })?;
    Ok(())
}

fn read_preference(
    connection: &Connection,
    expected_core: &str,
    key: &str,
) -> Result<Option<Preference>, SdkError> {
    let row: Option<(String, f64, f64, i64, String, i64)> = connection
        .query_row(
            "SELECT preference_key, value, confidence, evidence_count, updated_at, revision
             FROM development_preferences
             WHERE core_uuid = ?1 AND preference_key = ?2",
            params![expected_core, key],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                ))
            },
        )
        .optional()?;
    row.map(
        |(stored_key, value, confidence, evidence_count, updated_at, revision)| {
            let preference = Preference {
                schema: LIVING_CONTRACT_VERSION.to_owned(),
                core_uuid: expected_core.to_owned(),
                key: stored_key,
                value: raw_f32(value),
                confidence: raw_f32(confidence),
                evidence_count: u64::try_from(evidence_count).map_err(|_| {
                    SdkError::CorruptData("stored preference evidence count is invalid")
                })?,
                updated_at,
                revision: u64::try_from(revision)
                    .map_err(|_| SdkError::CorruptData("stored preference revision is invalid"))?,
            };
            preference
                .validate()
                .map_err(|_| SdkError::CorruptData("stored preference is invalid"))?;
            Ok(preference)
        },
    )
    .transpose()
}

fn upsert_preference(connection: &Connection, preference: &Preference) -> Result<(), SdkError> {
    let evidence_count = i64::try_from(preference.evidence_count)
        .map_err(|_| SdkError::InvalidMemory("preference evidence count is out of range"))?;
    let revision = revision_value(preference.revision)?;
    connection.execute(
        "INSERT INTO development_preferences(
            core_uuid, preference_key, value, confidence, evidence_count, updated_at, revision
         ) VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7)
         ON CONFLICT(core_uuid, preference_key) DO UPDATE SET
             value = excluded.value,
             confidence = excluded.confidence,
             evidence_count = excluded.evidence_count,
             updated_at = excluded.updated_at,
             revision = excluded.revision",
        params![
            preference.core_uuid,
            preference.key,
            f64::from(preference.value),
            f64::from(preference.confidence),
            evidence_count,
            preference.updated_at,
            revision
        ],
    )?;
    Ok(())
}

struct EvidenceInput<'a> {
    key: Option<&'a str>,
    summary: &'a str,
    valence: f32,
    weight: f32,
    source: &'a str,
    created_at: &'a str,
}

fn insert_evidence(
    connection: &Connection,
    core_uuid: &str,
    evidence: &EvidenceInput<'_>,
) -> Result<(), SdkError> {
    connection.execute(
        "INSERT INTO development_evidence(
            core_uuid, preference_key, summary, valence, weight, source, created_at
         ) VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            core_uuid,
            evidence.key,
            evidence.summary,
            f64::from(evidence.valence),
            f64::from(evidence.weight),
            evidence.source,
            evidence.created_at
        ],
    )?;
    Ok(())
}

fn insert_event(
    connection: &Connection,
    core_uuid: &str,
    entity_kind: &str,
    entity_id: &str,
    action: &str,
    detail: &Value,
    created_at: &str,
) -> Result<(), SdkError> {
    let detail_json = serde_json::to_string(detail)?;
    if detail_json.len() > MAX_DEVELOPMENT_EVENT_DETAIL_BYTES {
        return Err(SdkError::InvalidMemory(
            "development event detail is too large",
        ));
    }
    connection.execute(
        "INSERT INTO development_events(
            core_uuid, entity_kind, entity_id, action, detail_json, created_at
         ) VALUES(?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            core_uuid,
            entity_kind,
            entity_id,
            action,
            detail_json,
            created_at
        ],
    )?;
    Ok(())
}

fn decode_proposal(payload: &str, expected_core: &str) -> Result<PersonalityProposal, SdkError> {
    let proposal: PersonalityProposal = serde_json::from_str(payload)
        .map_err(|_| SdkError::CorruptData("stored personality proposal is invalid"))?;
    let stored_core =
        canonical_core_id(&proposal.core_uuid).map_err(|_| SdkError::OwnershipMismatch)?;
    if stored_core != expected_core {
        return Err(SdkError::OwnershipMismatch);
    }
    proposal
        .validate()
        .map_err(|_| SdkError::CorruptData("stored personality proposal is invalid"))?;
    Ok(proposal)
}

fn validate_experience(summary: &str) -> Result<(), SdkError> {
    if summary.trim().is_empty()
        || !chars_within(summary, MAX_EXPERIENCE_CHARS)
        || summary.chars().any(char::is_control)
    {
        return Err(SdkError::InvalidMemory("experience summary is invalid"));
    }
    Ok(())
}

fn validate_source(source: &str) -> Result<(), SdkError> {
    if source.trim().is_empty()
        || !chars_within(source, MAX_DEVELOPMENT_SOURCE_CHARS)
        || source.chars().any(char::is_control)
    {
        return Err(SdkError::InvalidMemory("evidence source is invalid"));
    }
    Ok(())
}

fn validate_valence(valence: f32) -> Result<(), SdkError> {
    if !valence.is_finite() || !(-1.0..=1.0).contains(&valence) {
        return Err(SdkError::InvalidMemory("evidence valence is out of range"));
    }
    Ok(())
}

fn validate_weight(weight: f32) -> Result<(), SdkError> {
    if !weight.is_finite() || !(0.0..=1.0).contains(&weight) {
        return Err(SdkError::InvalidMemory("evidence weight is out of range"));
    }
    Ok(())
}

fn strength(preference: &Preference) -> f32 {
    preference.confidence * preference.value.abs()
}

fn valid_id(value: &str) -> bool {
    !value.is_empty()
        && chars_within(value, MAX_ID_CHARS)
        && value.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.' | ':')
        })
}

fn revision_value(revision: u64) -> Result<i64, SdkError> {
    i64::try_from(revision)
        .map_err(|_| SdkError::CorruptData("development revision is out of range"))
}

fn limit_value(limit: usize, cap: usize) -> Result<Option<i64>, SdkError> {
    let limit = limit.min(cap);
    if limit == 0 {
        return Ok(None);
    }
    i64::try_from(limit)
        .map(Some)
        .map_err(|_| SdkError::InvalidMemory("limit is out of range"))
}

fn raw_f32(value: f64) -> f32 {
    value as f32
}

fn truncate_chars(value: &str, max_chars: usize) -> String {
    if value.chars().count() <= max_chars {
        return value.to_owned();
    }
    value.chars().take(max_chars).collect()
}
