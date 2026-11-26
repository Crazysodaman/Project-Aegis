//! Per-Core personal state: emotions, continuity, activities, routines, and
//! outfit decisions.
//!
//! Personal state lives in a dedicated `<core>/personal.db` that is bound to
//! both the Core UUID and the Core database instance UUID. The emotion engine
//! is pure and deterministic: identical state, influences, personality, and
//! timestamps always produce identical dimensions. Nothing here grants
//! authorization.

use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use aegis_contracts::core::validate_core_id;
use aegis_contracts::personal::PersonalContractError;
pub use aegis_contracts::personal::{
    Activity, ActivityStatus, ContinuitySummary, EmotionDelta, EmotionDimensions, EmotionInfluence,
    EmotionState, ExpressedEmotion, OutfitDecisionRecord, RelationshipEmotionEvent, Routine,
    CONTINUITY_SUMMARY_SCHEMA, EMOTION_STATE_SCHEMA, EXPRESSED_EMOTION_GAIN,
    EXPRESSED_EMOTION_SCHEMA, MAX_ELAPSED_OFFLINE_SECONDS, MAX_EXPERIENCE_CHARS,
    MAX_INFLUENCE_SOURCE_CHARS, MAX_OPEN_THREADS, MAX_OPEN_THREAD_CHARS, MAX_RECENT_EXPERIENCES,
    MAX_RELATIONSHIP_EVENT_TYPE_CHARS, MAX_RELATIONSHIP_TRUST, MAX_SESSION_ID_CHARS,
    PERSONAL_SCHEMA,
};
use aegis_core_storage::{CoreDatabase, CoreStorageLayout};
use aegis_storage::open_initialized;
use chrono::{DateTime, Utc};
use rusqlite::{params, Connection, OpenFlags, OptionalExtension, Transaction};
use uuid::Uuid;

use crate::personality::PersonalityProfile;
use crate::{
    canonical_core_id, chars_within, now_rfc3339, SdkError, MEMORY_DB_FILE, PERSONALITY_TABLE,
};

/// File name of the per-Core personal database.
pub const PERSONAL_DB_FILE: &str = "personal.db";
/// SQLite application marker for a per-Core personal database ("AEGL").
pub const PERSONAL_APPLICATION_ID: i64 = 0x4145_474C;
/// Component name registered in the shared `aegis_schema` table.
pub const PERSONAL_COMPONENT: &str = "core_personal";
/// Ownership table for the personal database.
pub const PERSONAL_OWNERSHIP_TABLE: &str = "personal_ownership";
/// Schema marker table for the personal database.
pub const PERSONAL_SCHEMA_TABLE: &str = "personal_schema";
/// Current personal database schema revision.
pub const PERSONAL_SCHEMA_REVISION: u64 = 1;

/// Table holding the single emotion state row.
pub const EMOTION_STATE_TABLE: &str = "emotion_state";
/// Append-only emotion history table.
pub const EMOTION_HISTORY_TABLE: &str = "emotion_history";
/// Continuity table.
pub const CONTINUITY_TABLE: &str = "continuity";
/// Activities table.
pub const ACTIVITIES_TABLE: &str = "activities";
/// Routines table.
pub const ROUTINES_TABLE: &str = "routines";
/// Append-only outfit decision table.
pub const OUTFIT_DECISIONS_TABLE: &str = "outfit_decisions";
/// Append-only personal event table.
pub const PERSONAL_EVENTS_TABLE: &str = "personal_events";
/// Open/resolved thread table.
pub const OPEN_THREADS_TABLE: &str = "open_threads";

/// Exponential emotion decay half-life in seconds (45 minutes).
///
/// Every dimension decays toward zero by a factor of `0.5^(elapsed / 2700)`.
pub const EMOTION_DECAY_HALF_LIFE_SECONDS: u64 = 45 * 60;
/// Maximum number of influences accepted by one emotion update.
pub const MAX_EMOTION_INFLUENCES: usize = 32;
/// Maximum number of history rows returned by one query.
pub const MAX_EMOTION_HISTORY_LIMIT: usize = 1000;
/// Maximum number of activities returned by one listing.
pub const MAX_ACTIVITY_LIST: usize = 64;
/// Maximum number of routines returned by one listing.
pub const MAX_ROUTINE_LIST: usize = 64;
/// Maximum number of outfit decisions returned by one listing.
pub const MAX_OUTFIT_LIST: usize = 64;
/// Maximum number of open threads returned by one listing.
pub const MAX_OPEN_THREAD_LIST: usize = 64;

const MODULATION_TRAITS: [&str; 3] = ["sensitivity", "empathy", "openness"];

const SCHEMA_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS personal_ownership (
    singleton INTEGER NOT NULL PRIMARY KEY CHECK (singleton = 1),
    core_uuid TEXT NOT NULL,
    database_instance_uuid TEXT NOT NULL,
    schema TEXT NOT NULL,
    created_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS personal_schema (
    singleton INTEGER NOT NULL PRIMARY KEY CHECK (singleton = 1),
    schema TEXT NOT NULL,
    revision INTEGER NOT NULL CHECK (revision >= 1),
    created_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS emotion_state (
    core_uuid TEXT NOT NULL PRIMARY KEY,
    schema TEXT NOT NULL,
    valence REAL NOT NULL,
    arousal REAL NOT NULL,
    joy REAL NOT NULL,
    calm REAL NOT NULL,
    curiosity REAL NOT NULL,
    tension REAL NOT NULL,
    intensity REAL NOT NULL,
    updated_at TEXT NOT NULL,
    revision INTEGER NOT NULL CHECK (revision >= 1)
);
CREATE TABLE IF NOT EXISTS emotion_history (
    sequence INTEGER NOT NULL PRIMARY KEY AUTOINCREMENT,
    core_uuid TEXT NOT NULL,
    schema TEXT NOT NULL,
    revision INTEGER NOT NULL CHECK (revision >= 1),
    valence REAL NOT NULL,
    arousal REAL NOT NULL,
    joy REAL NOT NULL,
    calm REAL NOT NULL,
    curiosity REAL NOT NULL,
    tension REAL NOT NULL,
    intensity REAL NOT NULL,
    source TEXT NOT NULL,
    created_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS emotion_history_core_idx
    ON emotion_history(core_uuid, sequence);
CREATE TABLE IF NOT EXISTS continuity (
    core_uuid TEXT NOT NULL PRIMARY KEY,
    last_active_at TEXT NOT NULL,
    last_session_id TEXT,
    updated_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS activities (
    id TEXT NOT NULL PRIMARY KEY,
    core_uuid TEXT NOT NULL,
    title TEXT NOT NULL,
    status TEXT NOT NULL,
    started_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    notes TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS activities_core_updated_idx
    ON activities(core_uuid, updated_at);
CREATE TABLE IF NOT EXISTS routines (
    id TEXT NOT NULL PRIMARY KEY,
    core_uuid TEXT NOT NULL,
    name TEXT NOT NULL,
    time_of_day TEXT,
    notes TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS routines_core_idx ON routines(core_uuid);
CREATE TABLE IF NOT EXISTS outfit_decisions (
    id TEXT NOT NULL PRIMARY KEY,
    core_uuid TEXT NOT NULL,
    decided_at TEXT NOT NULL,
    asset_ids_json TEXT NOT NULL,
    rationale TEXT NOT NULL,
    created_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS outfit_decisions_core_idx
    ON outfit_decisions(core_uuid, created_at);
CREATE TABLE IF NOT EXISTS personal_events (
    sequence INTEGER NOT NULL PRIMARY KEY AUTOINCREMENT,
    core_uuid TEXT NOT NULL,
    kind TEXT NOT NULL,
    detail TEXT NOT NULL,
    session_id TEXT,
    created_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS personal_events_core_kind_idx
    ON personal_events(core_uuid, kind, sequence);
CREATE TABLE IF NOT EXISTS open_threads (
    id TEXT NOT NULL PRIMARY KEY,
    core_uuid TEXT NOT NULL,
    summary TEXT NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('open', 'resolved')),
    opened_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS open_threads_core_status_idx
    ON open_threads(core_uuid, status);
CREATE TRIGGER IF NOT EXISTS personal_ownership_no_update
BEFORE UPDATE ON personal_ownership
BEGIN
    SELECT RAISE(ABORT, 'personal_ownership is immutable');
END;
CREATE TRIGGER IF NOT EXISTS personal_ownership_no_delete
BEFORE DELETE ON personal_ownership
BEGIN
    SELECT RAISE(ABORT, 'personal_ownership is immutable');
END;
CREATE TRIGGER IF NOT EXISTS emotion_history_no_update
BEFORE UPDATE ON emotion_history
BEGIN
    SELECT RAISE(ABORT, 'emotion_history is append-only');
END;
CREATE TRIGGER IF NOT EXISTS emotion_history_no_delete
BEFORE DELETE ON emotion_history
BEGIN
    SELECT RAISE(ABORT, 'emotion_history is append-only');
END;
CREATE TRIGGER IF NOT EXISTS outfit_decisions_no_update
BEFORE UPDATE ON outfit_decisions
BEGIN
    SELECT RAISE(ABORT, 'outfit_decisions is append-only');
END;
CREATE TRIGGER IF NOT EXISTS outfit_decisions_no_delete
BEFORE DELETE ON outfit_decisions
BEGIN
    SELECT RAISE(ABORT, 'outfit_decisions is append-only');
END;
CREATE TRIGGER IF NOT EXISTS personal_events_no_update
BEFORE UPDATE ON personal_events
BEGIN
    SELECT RAISE(ABORT, 'personal_events is append-only');
END;
CREATE TRIGGER IF NOT EXISTS personal_events_no_delete
BEFORE DELETE ON personal_events
BEGIN
    SELECT RAISE(ABORT, 'personal_events is append-only');
END;
"#;

/// One open thread record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenThread {
    pub id: String,
    pub summary: String,
    pub status: String,
    pub opened_at: String,
    pub updated_at: String,
}

/// One append-only emotion history record.
#[derive(Debug, Clone, PartialEq)]
pub struct EmotionHistoryEntry {
    pub schema: String,
    pub sequence: u64,
    pub source: String,
    pub state: EmotionState,
}

/// Writable per-Core personal store.
///
/// Owns a `rusqlite::Connection` bound to `<core>/personal.db`. It never opens
/// another Core's personal database and never writes to `core.db` or
/// `memory.db`.
pub struct PersonalStore {
    core_uuid: String,
    database_instance_uuid: String,
    core_dir: PathBuf,
    database_path: PathBuf,
    connection: Connection,
}

impl std::fmt::Debug for PersonalStore {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PersonalStore")
            .field("core_uuid", &self.core_uuid)
            .field("database_instance_uuid", &self.database_instance_uuid)
            .field("database_path", &self.database_path)
            .finish_non_exhaustive()
    }
}

impl PersonalStore {
    /// Open (or initialize) `<core>/personal.db` for an existing Core.
    ///
    /// The Core database is read only; it supplies the immutable Core UUID and
    /// database instance UUID. An existing personal database must carry exactly
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
        let personal_path = core_dir.join(PERSONAL_DB_FILE);
        let existed = match fs::symlink_metadata(&personal_path) {
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
            open_initialized(&personal_path, PERSONAL_APPLICATION_ID, PERSONAL_COMPONENT)?;

        let canonical_dir = fs::canonicalize(&core_dir)?;
        let canonical_db = fs::canonicalize(&personal_path)?;
        if canonical_db.parent() != Some(canonical_dir.as_path())
            || canonical_db.file_name() != Some(OsStr::new(PERSONAL_DB_FILE))
        {
            return Err(SdkError::ForeignPath);
        }

        let has_ownership = table_exists(&connection, PERSONAL_OWNERSHIP_TABLE)?;
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
                "INSERT INTO personal_ownership(
                    singleton, core_uuid, database_instance_uuid, schema, created_at
                 ) VALUES(1, ?1, ?2, ?3, ?4)",
                params![
                    core_uuid,
                    database_instance_uuid,
                    PERSONAL_SCHEMA,
                    now_rfc3339()?
                ],
            )?;
        }

        seed_state(&connection, &core_uuid)?;

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

    /// Canonical path of `personal.db`.
    pub fn database_path(&self) -> &Path {
        &self.database_path
    }

    /// Current emotion state for this Core.
    pub fn emotion_state(&self) -> Result<EmotionState, SdkError> {
        read_emotion_state(&self.connection, &self.core_uuid)
    }

    /// Bounded, deterministic outward projection of the persisted internal
    /// emotion state. The internal state is not modified.
    pub fn expressed_emotion(&self) -> Result<ExpressedEmotion, SdkError> {
        self.expressed_emotion_with_gain(EXPRESSED_EMOTION_GAIN)
    }

    /// Projection variant with an explicit attenuation gain (0 ..= 1).
    pub fn expressed_emotion_with_gain(&self, gain: f32) -> Result<ExpressedEmotion, SdkError> {
        let state = self.emotion_state()?;
        state
            .expressed_with_gain(gain)
            .map_err(|_| SdkError::InvalidMemory("expression gain is invalid"))
    }

    /// Apply one explicit relationship event as an emotion influence.
    ///
    /// Trust comes only from the event itself; this method never reads the
    /// relationship store and never invents a trust value.
    pub fn apply_relationship_event(
        &self,
        event: &RelationshipEmotionEvent,
    ) -> Result<EmotionState, SdkError> {
        self.apply_relationship_event_at(event, Utc::now())
    }

    /// Deterministic variant of [`PersonalStore::apply_relationship_event`].
    pub fn apply_relationship_event_at(
        &self,
        event: &RelationshipEmotionEvent,
        now: DateTime<Utc>,
    ) -> Result<EmotionState, SdkError> {
        let influence = EmotionInfluence::from_relationship_event(event).map_err(contract_error)?;
        self.apply_influences_at(&[influence], now)
    }

    /// Apply weighted influences and return the new deterministic state.
    ///
    /// The current dimensions are first exponentially decayed from the stored
    /// `updated_at` to the current wall clock, then each influence is applied
    /// with its weight (modulated by bounded personality traits when present).
    pub fn apply_influences(
        &self,
        influences: &[EmotionInfluence],
    ) -> Result<EmotionState, SdkError> {
        self.apply_influences_at(influences, Utc::now())
    }

    /// Deterministic variant of [`PersonalStore::apply_influences`].
    pub fn apply_influences_at(
        &self,
        influences: &[EmotionInfluence],
        now: DateTime<Utc>,
    ) -> Result<EmotionState, SdkError> {
        if influences.len() > MAX_EMOTION_INFLUENCES {
            return Err(SdkError::InvalidMemory("too many emotion influences"));
        }
        for influence in influences {
            influence.validate().map_err(contract_error)?;
        }

        let current = self.emotion_state()?;
        let elapsed = elapsed_seconds_between(&current.updated_at, now)?;
        let mut dimensions = decay_dimensions(current.dimensions, elapsed);
        let weight_factor = self.personality_weight_factor()?;

        for influence in influences {
            let weight = (influence.weight * weight_factor).clamp(0.0, 1.0);
            dimensions.valence += influence.delta.valence * weight;
            dimensions.arousal += influence.delta.arousal * weight;
            dimensions.joy += influence.delta.joy * weight;
            dimensions.calm += influence.delta.calm * weight;
            dimensions.curiosity += influence.delta.curiosity * weight;
            dimensions.tension += influence.delta.tension * weight;
        }
        let dimensions = dimensions.clamped();
        let intensity = recompute_intensity(&dimensions);
        let updated_at = now.to_rfc3339();
        let state = EmotionState {
            schema: EMOTION_STATE_SCHEMA.to_owned(),
            core_uuid: self.core_uuid.clone(),
            dimensions,
            intensity,
            updated_at,
            revision: current.revision.saturating_add(1),
        };
        state.validate().map_err(contract_error)?;

        let source = if influences.is_empty() {
            "decay".to_owned()
        } else {
            influences
                .iter()
                .map(|influence| influence.source.as_str())
                .collect::<Vec<_>>()
                .join(",")
        };
        let transaction = self.connection.unchecked_transaction()?;
        write_emotion_state(&transaction, &state)?;
        append_emotion_history(&transaction, &state, &truncate_chars(&source, 512))?;
        transaction.commit()?;
        Ok(state)
    }

    /// Append-only emotion history, newest first, capped at `limit`.
    pub fn emotion_history(&self, limit: usize) -> Result<Vec<EmotionHistoryEntry>, SdkError> {
        let limit = limit.min(MAX_EMOTION_HISTORY_LIMIT);
        if limit == 0 {
            return Ok(Vec::new());
        }
        let limit = i64::try_from(limit)
            .map_err(|_| SdkError::InvalidMemory("history limit is out of range"))?;
        let mut statement = self.connection.prepare(
            "SELECT sequence, schema, core_uuid, revision, valence, arousal, joy, calm,
                    curiosity, tension, intensity, source, created_at
             FROM emotion_history WHERE core_uuid = ?1
             ORDER BY sequence DESC LIMIT ?2",
        )?;
        let mut rows = statement.query(params![self.core_uuid, limit])?;
        let mut history = Vec::new();
        while let Some(row) = rows.next()? {
            history.push(read_emotion_history(row, &self.core_uuid)?);
        }
        Ok(history)
    }

    /// Update last-active time and optional session identifier.
    pub fn update_last_active(&self, session_id: Option<&str>) -> Result<(), SdkError> {
        self.update_last_active_at(session_id, Utc::now())
    }

    /// Deterministic variant of [`PersonalStore::update_last_active`].
    pub fn update_last_active_at(
        &self,
        session_id: Option<&str>,
        now: DateTime<Utc>,
    ) -> Result<(), SdkError> {
        if let Some(session_id) = session_id {
            validate_bounded_text(session_id, MAX_SESSION_ID_CHARS, "session id is invalid")?;
        }
        let timestamp = now.to_rfc3339();
        let changed = self.connection.execute(
            "UPDATE continuity
             SET last_active_at = ?2, last_session_id = ?3, updated_at = ?2
             WHERE core_uuid = ?1",
            params![self.core_uuid, timestamp, session_id],
        )?;
        if changed != 1 {
            return Err(SdkError::CorruptData("continuity row is missing"));
        }
        Ok(())
    }

    /// Record a bounded recent experience in the append-only event log.
    pub fn record_experience(&self, summary: &str) -> Result<(), SdkError> {
        self.record_experience_at(summary, Utc::now())
    }

    /// Deterministic variant of [`PersonalStore::record_experience`].
    pub fn record_experience_at(&self, summary: &str, now: DateTime<Utc>) -> Result<(), SdkError> {
        validate_bounded_text(summary, MAX_EXPERIENCE_CHARS, "experience is invalid")?;
        self.connection.execute(
            "INSERT INTO personal_events(core_uuid, kind, detail, session_id, created_at)
             VALUES(?1, 'experience', ?2, NULL, ?3)",
            params![self.core_uuid, summary, now.to_rfc3339()],
        )?;
        Ok(())
    }

    /// Open a new continuity thread and return its identifier.
    pub fn open_thread(&self, summary: &str) -> Result<String, SdkError> {
        validate_bounded_text(summary, MAX_OPEN_THREAD_CHARS, "thread summary is invalid")?;
        let id = Uuid::new_v4().to_string();
        let now = now_rfc3339()?;
        self.connection.execute(
            "INSERT INTO open_threads(id, core_uuid, summary, status, opened_at, updated_at)
             VALUES(?1, ?2, ?3, 'open', ?4, ?4)",
            params![id, self.core_uuid, summary, now],
        )?;
        Ok(id)
    }

    /// Mark a thread resolved. Resolving an already-resolved thread is a no-op.
    pub fn resolve_thread(&self, id: &str) -> Result<(), SdkError> {
        let now = now_rfc3339()?;
        let changed = self.connection.execute(
            "UPDATE open_threads SET status = 'resolved', updated_at = ?3
             WHERE core_uuid = ?1 AND id = ?2 AND status = 'open'",
            params![self.core_uuid, id, now],
        )?;
        if changed != 0 {
            return Ok(());
        }
        let exists: Option<String> = self
            .connection
            .query_row(
                "SELECT status FROM open_threads WHERE core_uuid = ?1 AND id = ?2",
                params![self.core_uuid, id],
                |row| row.get(0),
            )
            .optional()?;
        match exists {
            Some(_) => Ok(()),
            None => Err(SdkError::NotFound),
        }
    }

    /// Open threads, newest first, capped at `limit`.
    pub fn list_open_threads(&self, limit: usize) -> Result<Vec<OpenThread>, SdkError> {
        let limit = limit.min(MAX_OPEN_THREAD_LIST);
        if limit == 0 {
            return Ok(Vec::new());
        }
        let limit = i64::try_from(limit)
            .map_err(|_| SdkError::InvalidMemory("thread limit is out of range"))?;
        let mut statement = self.connection.prepare(
            "SELECT id, summary, status, opened_at, updated_at FROM open_threads
             WHERE core_uuid = ?1 AND status = 'open'
             ORDER BY opened_at DESC, id ASC LIMIT ?2",
        )?;
        let mut rows = statement.query(params![self.core_uuid, limit])?;
        let mut threads = Vec::new();
        while let Some(row) = rows.next()? {
            threads.push(OpenThread {
                id: row.get(0)?,
                summary: row.get(1)?,
                status: row.get(2)?,
                opened_at: row.get(3)?,
                updated_at: row.get(4)?,
            });
        }
        Ok(threads)
    }

    /// Bounded continuity summary computed against the current clock.
    pub fn continuity_summary(&self) -> Result<ContinuitySummary, SdkError> {
        self.continuity_summary_at(Utc::now())
    }

    /// Deterministic variant of [`PersonalStore::continuity_summary`].
    pub fn continuity_summary_at(&self, now: DateTime<Utc>) -> Result<ContinuitySummary, SdkError> {
        let (last_active_at, last_session_id) = self.continuity_row()?;
        let elapsed =
            elapsed_seconds_between(&last_active_at, now)?.min(MAX_ELAPSED_OFFLINE_SECONDS);
        let recent_experiences = self.recent_experiences(MAX_RECENT_EXPERIENCES)?;
        let open_threads = self
            .list_open_threads(MAX_OPEN_THREADS)?
            .into_iter()
            .map(|thread| thread.summary)
            .collect();
        let activities = self.list_activities(8)?;
        let summary = ContinuitySummary {
            schema: CONTINUITY_SUMMARY_SCHEMA.to_owned(),
            core_uuid: self.core_uuid.clone(),
            last_active_at,
            last_session_id,
            elapsed_offline_seconds: elapsed,
            recent_experiences,
            open_threads,
            activities,
            updated_at: now.to_rfc3339(),
        };
        summary
            .validate()
            .map_err(|_| SdkError::CorruptData("continuity summary is invalid"))?;
        Ok(summary)
    }

    /// Insert or update an activity.
    pub fn upsert_activity(&self, activity: &Activity) -> Result<Activity, SdkError> {
        activity.validate().map_err(contract_error)?;
        let existing: Option<String> = self
            .connection
            .query_row(
                "SELECT core_uuid FROM activities WHERE id = ?1",
                params![activity.id],
                |row| row.get(0),
            )
            .optional()?;
        if let Some(core_uuid) = existing {
            if core_uuid != self.core_uuid {
                return Err(SdkError::OwnershipMismatch);
            }
        }
        self.connection.execute(
            "INSERT INTO activities(id, core_uuid, title, status, started_at, updated_at, notes)
             VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(id) DO UPDATE SET
                 title = excluded.title,
                 status = excluded.status,
                 started_at = excluded.started_at,
                 updated_at = excluded.updated_at,
                 notes = excluded.notes",
            params![
                activity.id,
                self.core_uuid,
                activity.title,
                activity.status.as_str(),
                activity.started_at,
                activity.updated_at,
                activity.notes
            ],
        )?;
        Ok(activity.clone())
    }

    /// List activities, most recently updated first, capped at `limit`.
    pub fn list_activities(&self, limit: usize) -> Result<Vec<Activity>, SdkError> {
        let limit = limit.min(MAX_ACTIVITY_LIST);
        if limit == 0 {
            return Ok(Vec::new());
        }
        let limit = i64::try_from(limit)
            .map_err(|_| SdkError::InvalidMemory("activity limit is out of range"))?;
        let mut statement = self.connection.prepare(
            "SELECT id, title, status, started_at, updated_at, notes FROM activities
             WHERE core_uuid = ?1 ORDER BY updated_at DESC, id ASC LIMIT ?2",
        )?;
        let mut rows = statement.query(params![self.core_uuid, limit])?;
        let mut activities = Vec::new();
        while let Some(row) = rows.next()? {
            activities.push(read_activity(row)?);
        }
        Ok(activities)
    }

    /// Insert or update a routine.
    pub fn set_routine(&self, routine: &Routine) -> Result<Routine, SdkError> {
        routine.validate().map_err(contract_error)?;
        let existing: Option<String> = self
            .connection
            .query_row(
                "SELECT core_uuid FROM routines WHERE id = ?1",
                params![routine.id],
                |row| row.get(0),
            )
            .optional()?;
        if let Some(core_uuid) = existing {
            if core_uuid != self.core_uuid {
                return Err(SdkError::OwnershipMismatch);
            }
        }
        let now = now_rfc3339()?;
        self.connection.execute(
            "INSERT INTO routines(id, core_uuid, name, time_of_day, notes, updated_at)
             VALUES(?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(id) DO UPDATE SET
                 name = excluded.name,
                 time_of_day = excluded.time_of_day,
                 notes = excluded.notes,
                 updated_at = excluded.updated_at",
            params![
                routine.id,
                self.core_uuid,
                routine.name,
                routine.time_of_day,
                routine.notes,
                now
            ],
        )?;
        Ok(routine.clone())
    }

    /// List routines, newest first, capped at `limit`.
    pub fn list_routines(&self, limit: usize) -> Result<Vec<Routine>, SdkError> {
        let limit = limit.min(MAX_ROUTINE_LIST);
        if limit == 0 {
            return Ok(Vec::new());
        }
        let limit = i64::try_from(limit)
            .map_err(|_| SdkError::InvalidMemory("routine limit is out of range"))?;
        let mut statement = self.connection.prepare(
            "SELECT id, name, time_of_day, notes FROM routines
             WHERE core_uuid = ?1 ORDER BY updated_at DESC, id ASC LIMIT ?2",
        )?;
        let mut rows = statement.query(params![self.core_uuid, limit])?;
        let mut routines = Vec::new();
        while let Some(row) = rows.next()? {
            routines.push(Routine {
                id: row.get(0)?,
                name: row.get(1)?,
                time_of_day: row.get(2)?,
                notes: row.get(3)?,
            });
        }
        Ok(routines)
    }

    /// Append one outfit decision. Records are immutable once written.
    pub fn record_outfit(&self, record: &OutfitDecisionRecord) -> Result<(), SdkError> {
        record.validate().map_err(contract_error)?;
        let asset_ids_json = serde_json::to_string(&record.asset_ids)?;
        self.connection.execute(
            "INSERT INTO outfit_decisions(id, core_uuid, decided_at, asset_ids_json, rationale, created_at)
             VALUES(?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                record.id,
                self.core_uuid,
                record.decided_at,
                asset_ids_json,
                record.rationale,
                now_rfc3339()?
            ],
        )?;
        Ok(())
    }

    /// Outfit decisions, newest first, capped at `limit`.
    pub fn outfit_decisions(&self, limit: usize) -> Result<Vec<OutfitDecisionRecord>, SdkError> {
        let limit = limit.min(MAX_OUTFIT_LIST);
        if limit == 0 {
            return Ok(Vec::new());
        }
        let limit = i64::try_from(limit)
            .map_err(|_| SdkError::InvalidMemory("outfit limit is out of range"))?;
        let mut statement = self.connection.prepare(
            "SELECT id, decided_at, asset_ids_json, rationale FROM outfit_decisions
             WHERE core_uuid = ?1 ORDER BY created_at DESC, id ASC LIMIT ?2",
        )?;
        let mut rows = statement.query(params![self.core_uuid, limit])?;
        let mut decisions = Vec::new();
        while let Some(row) = rows.next()? {
            let id: String = row.get(0)?;
            let decided_at: String = row.get(1)?;
            let asset_ids_json: String = row.get(2)?;
            let rationale: String = row.get(3)?;
            let asset_ids: Vec<String> = serde_json::from_str(&asset_ids_json)?;
            let record = OutfitDecisionRecord {
                id,
                decided_at,
                asset_ids,
                rationale,
            };
            record
                .validate()
                .map_err(|_| SdkError::CorruptData("outfit decision is invalid"))?;
            decisions.push(record);
        }
        Ok(decisions)
    }

    fn continuity_row(&self) -> Result<(String, Option<String>), SdkError> {
        self.connection
            .query_row(
                "SELECT last_active_at, last_session_id FROM continuity WHERE core_uuid = ?1",
                params![self.core_uuid],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?
            .ok_or(SdkError::CorruptData("continuity row is missing"))
    }

    fn recent_experiences(&self, limit: usize) -> Result<Vec<String>, SdkError> {
        let limit = i64::try_from(limit.min(MAX_RECENT_EXPERIENCES))
            .map_err(|_| SdkError::InvalidMemory("experience limit is out of range"))?;
        let mut statement = self.connection.prepare(
            "SELECT detail FROM personal_events
             WHERE core_uuid = ?1 AND kind = 'experience'
             ORDER BY sequence DESC LIMIT ?2",
        )?;
        let rows = statement.query_map(params![self.core_uuid, limit], |row| {
            row.get::<_, String>(0)
        })?;
        let mut experiences = Vec::new();
        for row in rows {
            experiences.push(row?);
        }
        Ok(experiences)
    }

    fn personality_weight_factor(&self) -> Result<f32, SdkError> {
        let memory_path = self.core_dir.join(MEMORY_DB_FILE);
        if !memory_path.is_file() {
            return Ok(1.0);
        }
        let connection =
            Connection::open_with_flags(&memory_path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        if !table_exists(&connection, "core_memory_ownership")?
            || !table_exists(&connection, PERSONALITY_TABLE)?
        {
            return Ok(1.0);
        }
        let owner: Option<String> = connection
            .query_row(
                "SELECT core_uuid FROM core_memory_ownership WHERE singleton = 1",
                [],
                |row| row.get(0),
            )
            .optional()?;
        let Some(owner) = owner else {
            return Ok(1.0);
        };
        if canonical_core_id(&owner).ok().as_deref() != Some(self.core_uuid.as_str()) {
            return Ok(1.0);
        }
        let json: Option<String> = connection
            .query_row(
                "SELECT profile_json FROM core_personality WHERE core_uuid = ?1",
                params![self.core_uuid],
                |row| row.get(0),
            )
            .optional()?;
        let Some(json) = json else {
            return Ok(1.0);
        };
        let profile: PersonalityProfile = serde_json::from_str(&json)?;
        if canonical_core_id(&profile.core_uuid).ok().as_deref() != Some(self.core_uuid.as_str()) {
            return Ok(1.0);
        }
        profile
            .validate()
            .map_err(|_| SdkError::CorruptData("stored personality is invalid"))?;
        Ok(personality_weight_factor_from(&profile.traits))
    }
}

fn initialize_schema(connection: &Connection) -> Result<(), SdkError> {
    connection.execute_batch(SCHEMA_SQL)?;
    Ok(())
}

fn ensure_schema_marker(connection: &Connection) -> Result<(), SdkError> {
    let existing: Option<(String, i64)> = connection
        .query_row(
            "SELECT schema, revision FROM personal_schema WHERE singleton = 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    match existing {
        None => {
            let revision = i64::try_from(PERSONAL_SCHEMA_REVISION)
                .map_err(|_| SdkError::CorruptData("personal schema revision is invalid"))?;
            connection.execute(
                "INSERT INTO personal_schema(singleton, schema, revision, created_at)
                 VALUES(1, ?1, ?2, ?3)",
                params![PERSONAL_SCHEMA, revision, now_rfc3339()?],
            )?;
        }
        Some((schema, revision)) => {
            let revision = u64::try_from(revision)
                .map_err(|_| SdkError::CorruptData("personal schema revision is invalid"))?;
            if schema != PERSONAL_SCHEMA || revision > PERSONAL_SCHEMA_REVISION {
                return Err(SdkError::CorruptData("personal schema is unsupported"));
            }
        }
    }
    Ok(())
}

fn seed_state(connection: &Connection, core_uuid: &str) -> Result<(), SdkError> {
    let now = now_rfc3339()?;
    let neutral = EmotionDimensions::neutral();
    connection.execute(
        "INSERT OR IGNORE INTO emotion_state(
            core_uuid, schema, valence, arousal, joy, calm, curiosity, tension,
            intensity, updated_at, revision
         ) VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, 1)",
        params![
            core_uuid,
            EMOTION_STATE_SCHEMA,
            f64::from(neutral.valence),
            f64::from(neutral.arousal),
            f64::from(neutral.joy),
            f64::from(neutral.calm),
            f64::from(neutral.curiosity),
            f64::from(neutral.tension),
            0.0_f64,
            now
        ],
    )?;
    connection.execute(
        "INSERT OR IGNORE INTO continuity(core_uuid, last_active_at, last_session_id, updated_at)
         VALUES(?1, ?2, NULL, ?2)",
        params![core_uuid, now],
    )?;
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

struct RawEmotion {
    schema: String,
    core_uuid: String,
    valence: f64,
    arousal: f64,
    joy: f64,
    calm: f64,
    curiosity: f64,
    tension: f64,
    intensity: f64,
    updated_at: String,
    revision: i64,
}

impl RawEmotion {
    fn into_state(self) -> Result<EmotionState, SdkError> {
        let revision = u64::try_from(self.revision)
            .map_err(|_| SdkError::CorruptData("emotion revision is invalid"))?;
        let state = EmotionState {
            schema: self.schema,
            core_uuid: self.core_uuid,
            dimensions: EmotionDimensions {
                valence: raw_f32(self.valence),
                arousal: raw_f32(self.arousal),
                joy: raw_f32(self.joy),
                calm: raw_f32(self.calm),
                curiosity: raw_f32(self.curiosity),
                tension: raw_f32(self.tension),
            },
            intensity: raw_f32(self.intensity),
            updated_at: self.updated_at,
            revision,
        };
        state
            .validate()
            .map_err(|_| SdkError::CorruptData("stored emotion state is invalid"))?;
        Ok(state)
    }
}

fn read_raw_emotion(row: &rusqlite::Row<'_>) -> rusqlite::Result<RawEmotion> {
    Ok(RawEmotion {
        schema: row.get(0)?,
        core_uuid: row.get(1)?,
        valence: row.get(2)?,
        arousal: row.get(3)?,
        joy: row.get(4)?,
        calm: row.get(5)?,
        curiosity: row.get(6)?,
        tension: row.get(7)?,
        intensity: row.get(8)?,
        updated_at: row.get(9)?,
        revision: row.get(10)?,
    })
}

fn read_emotion_state(
    connection: &Connection,
    expected_core: &str,
) -> Result<EmotionState, SdkError> {
    let raw = connection
        .query_row(
            "SELECT schema, core_uuid, valence, arousal, joy, calm, curiosity, tension,
                    intensity, updated_at, revision
             FROM emotion_state WHERE core_uuid = ?1",
            params![expected_core],
            read_raw_emotion,
        )
        .optional()?
        .ok_or(SdkError::CorruptData("emotion state row is missing"))?;
    let state = raw.into_state()?;
    if state.core_uuid != expected_core {
        return Err(SdkError::OwnershipMismatch);
    }
    Ok(state)
}

fn write_emotion_state(
    transaction: &Transaction<'_>,
    state: &EmotionState,
) -> Result<(), SdkError> {
    let revision = i64::try_from(state.revision)
        .map_err(|_| SdkError::InvalidMemory("emotion revision is out of range"))?;
    let changed = transaction.execute(
        "UPDATE emotion_state SET
            schema = ?2, valence = ?3, arousal = ?4, joy = ?5, calm = ?6,
            curiosity = ?7, tension = ?8, intensity = ?9, updated_at = ?10, revision = ?11
         WHERE core_uuid = ?1",
        params![
            state.core_uuid,
            state.schema,
            f64::from(state.dimensions.valence),
            f64::from(state.dimensions.arousal),
            f64::from(state.dimensions.joy),
            f64::from(state.dimensions.calm),
            f64::from(state.dimensions.curiosity),
            f64::from(state.dimensions.tension),
            f64::from(state.intensity),
            state.updated_at,
            revision
        ],
    )?;
    if changed != 1 {
        return Err(SdkError::CorruptData("emotion state row is missing"));
    }
    Ok(())
}

fn append_emotion_history(
    transaction: &Transaction<'_>,
    state: &EmotionState,
    source: &str,
) -> Result<(), SdkError> {
    let revision = i64::try_from(state.revision)
        .map_err(|_| SdkError::InvalidMemory("emotion revision is out of range"))?;
    transaction.execute(
        "INSERT INTO emotion_history(
            core_uuid, schema, revision, valence, arousal, joy, calm, curiosity,
            tension, intensity, source, created_at
         ) VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
        params![
            state.core_uuid,
            state.schema,
            revision,
            f64::from(state.dimensions.valence),
            f64::from(state.dimensions.arousal),
            f64::from(state.dimensions.joy),
            f64::from(state.dimensions.calm),
            f64::from(state.dimensions.curiosity),
            f64::from(state.dimensions.tension),
            f64::from(state.intensity),
            source,
            state.updated_at
        ],
    )?;
    Ok(())
}

fn read_emotion_history(
    row: &rusqlite::Row<'_>,
    expected_core: &str,
) -> Result<EmotionHistoryEntry, SdkError> {
    let sequence: i64 = row.get(0)?;
    let sequence = u64::try_from(sequence)
        .map_err(|_| SdkError::CorruptData("emotion history sequence is invalid"))?;
    let schema: String = row.get(1)?;
    let core_uuid: String = row.get(2)?;
    let revision: i64 = row.get(3)?;
    let revision = u64::try_from(revision)
        .map_err(|_| SdkError::CorruptData("emotion history revision is invalid"))?;
    let created_at: String = row.get(12)?;
    let source: String = row.get(11)?;
    let state = EmotionState {
        schema,
        core_uuid,
        dimensions: EmotionDimensions {
            valence: raw_f32(row.get(4)?),
            arousal: raw_f32(row.get(5)?),
            joy: raw_f32(row.get(6)?),
            calm: raw_f32(row.get(7)?),
            curiosity: raw_f32(row.get(8)?),
            tension: raw_f32(row.get(9)?),
        },
        intensity: raw_f32(row.get(10)?),
        updated_at: created_at,
        revision,
    };
    state
        .validate()
        .map_err(|_| SdkError::CorruptData("stored emotion history is invalid"))?;
    if state.core_uuid != expected_core {
        return Err(SdkError::OwnershipMismatch);
    }
    Ok(EmotionHistoryEntry {
        schema: EMOTION_STATE_SCHEMA.to_owned(),
        sequence,
        source,
        state,
    })
}

fn read_activity(row: &rusqlite::Row<'_>) -> Result<Activity, SdkError> {
    let status: String = row.get(2)?;
    let status = ActivityStatus::parse(&status)
        .ok_or(SdkError::CorruptData("activity status is invalid"))?;
    let activity = Activity {
        id: row.get(0)?,
        title: row.get(1)?,
        status,
        started_at: row.get(3)?,
        updated_at: row.get(4)?,
        notes: row.get(5)?,
    };
    activity
        .validate()
        .map_err(|_| SdkError::CorruptData("stored activity is invalid"))?;
    Ok(activity)
}

fn decay_dimensions(dimensions: EmotionDimensions, elapsed_seconds: u64) -> EmotionDimensions {
    if elapsed_seconds == 0 {
        return dimensions;
    }
    let factor =
        0.5_f64.powf(elapsed_seconds as f64 / EMOTION_DECAY_HALF_LIFE_SECONDS as f64) as f32;
    EmotionDimensions {
        valence: dimensions.valence * factor,
        arousal: dimensions.arousal * factor,
        joy: dimensions.joy * factor,
        calm: dimensions.calm * factor,
        curiosity: dimensions.curiosity * factor,
        tension: dimensions.tension * factor,
    }
}

fn recompute_intensity(dimensions: &EmotionDimensions) -> f32 {
    let valence = dimensions.valence.abs();
    let activation = (dimensions.arousal
        + dimensions.joy
        + dimensions.calm
        + dimensions.curiosity
        + dimensions.tension)
        / 5.0;
    (valence * 0.4 + activation * 0.6).clamp(0.0, 1.0)
}

fn personality_weight_factor_from(traits: &BTreeMap<String, f64>) -> f32 {
    let mut sum = 0.0_f64;
    let mut count = 0_u32;
    for name in MODULATION_TRAITS {
        if let Some(value) = traits.get(name) {
            if value.is_finite() {
                sum += value.clamp(-1.0, 1.0);
                count += 1;
            }
        }
    }
    if count == 0 {
        return 1.0;
    }
    let average = sum / f64::from(count);
    (1.0 + 0.5 * average).clamp(0.5, 1.5) as f32
}

fn elapsed_seconds_between(earlier: &str, later: DateTime<Utc>) -> Result<u64, SdkError> {
    let earlier = DateTime::parse_from_rfc3339(earlier)
        .map_err(|_| SdkError::CorruptData("stored timestamp is invalid"))?
        .with_timezone(&Utc);
    let seconds = (later - earlier).num_seconds();
    Ok(seconds.max(0) as u64)
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

fn validate_bounded_text(
    value: &str,
    maximum: usize,
    message: &'static str,
) -> Result<(), SdkError> {
    if value.trim().is_empty()
        || !chars_within(value, maximum)
        || value.chars().any(char::is_control)
    {
        return Err(SdkError::InvalidMemory(message));
    }
    Ok(())
}

fn contract_error(_error: PersonalContractError) -> SdkError {
    SdkError::InvalidMemory("personal state failed validation")
}
