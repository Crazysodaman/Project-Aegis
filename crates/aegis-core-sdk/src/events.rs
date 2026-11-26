//! Deterministic, idempotent event → experience → emotion pipeline.
//!
//! An [`InteractionEvent`] carries a stable `event_id` (for example the
//! conversation session plus assistant message id) and an explicit
//! [`EmotionInfluence`] with its own weight and per-dimension deltas. The
//! pipeline records one bounded experience and applies exactly that influence.
//!
//! Applying the same `event_id` twice is a no-op: a small ledger table inside
//! the Core's own `personal.db` claims each event before any state is written.
//! Because the ledger lives beside the personal state, idempotency survives a
//! process restart, a leave/re-open of the store, and a backup/restore round
//! trip. Every Core has its own `personal.db`, so identical event ids on
//! different Cores remain independent.
//!
//! The pipeline never reads the world snapshot, weather, clock-of-day, or
//! message text. Deltas come only from the event, so time and weather can never
//! dictate emotion. Nothing here grants authorization.

use std::path::Path;
use std::time::Duration;

use chrono::{DateTime, Utc};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

use crate::{
    chars_within, EmotionDelta, EmotionInfluence, EmotionState, PersonalStore, SdkError,
    MAX_EXPERIENCE_CHARS,
};

/// Table that records already-applied interaction events in `personal.db`.
pub const EVENT_LEDGER_TABLE: &str = "applied_interaction_events";

/// Maximum character length of a stable event identifier.
pub const MAX_EVENT_ID_CHARS: usize = 256;

/// Influence source used by the built-in deterministic interaction event.
pub const INTERACTION_INFLUENCE_SOURCE: &str = "interaction";

/// Fixed weight of the built-in interaction influence.
///
/// This is deliberately small and explicit; it is not derived from sentiment,
/// message length, weather, or time of day.
pub const INTERACTION_INFLUENCE_WEIGHT: f32 = 0.2;

/// Fixed per-dimension delta of the built-in interaction influence.
///
/// A completed exchange is modelled as a small, positive, calming and
/// curiosity-oriented engagement. It is a documented constant, applied only
/// when the engine explicitly creates an interaction event.
pub const INTERACTION_DELTA: EmotionDelta = EmotionDelta {
    valence: 0.05,
    arousal: 0.0,
    joy: 0.02,
    calm: 0.03,
    curiosity: 0.06,
    tension: 0.0,
};

const LEDGER_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS applied_interaction_events (
    event_id TEXT NOT NULL PRIMARY KEY,
    core_uuid TEXT NOT NULL,
    applied_at TEXT NOT NULL,
    experience TEXT NOT NULL,
    emotion_revision INTEGER
);
CREATE INDEX IF NOT EXISTS applied_interaction_events_core_idx
    ON applied_interaction_events(core_uuid);
"#;

/// One explicit event that can be applied to a Core's personal state.
///
/// `event_id` is the idempotency key. `experience` is the bounded human-readable
/// summary recorded in the Core's experience log. `influence` supplies the
/// explicit weight and deltas; no value is inferred from content or context.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InteractionEvent {
    pub event_id: String,
    pub experience: String,
    pub influence: EmotionInfluence,
}

impl InteractionEvent {
    /// Build an event from explicit parts.
    pub fn new(
        event_id: impl Into<String>,
        experience: impl Into<String>,
        influence: EmotionInfluence,
    ) -> Self {
        Self {
            event_id: event_id.into(),
            experience: experience.into(),
            influence,
        }
    }

    /// Build an event using the fixed, documented interaction influence.
    pub fn interaction(event_id: impl Into<String>, experience: impl Into<String>) -> Self {
        Self::new(event_id, experience, interaction_influence())
    }

    /// Build the deterministic interaction event for one assistant message.
    ///
    /// The event id is `interaction:<session_id>:<message_id>`, so replaying the
    /// same assistant message can never duplicate its experience or influence.
    pub fn for_message(session_id: &str, message_id: u64) -> Self {
        let event_id = format!("interaction:{session_id}:{message_id}");
        let experience =
            format!("Conversation turn in session {session_id} (assistant message {message_id})");
        Self::interaction(event_id, experience)
    }

    /// The bounded experience summary that will be recorded.
    pub fn experience_summary(&self) -> String {
        self.experience.trim().to_owned()
    }

    /// Validate the identifier, bounded experience, and explicit influence.
    pub fn validate(&self) -> Result<(), SdkError> {
        if self.event_id.is_empty()
            || self.event_id != self.event_id.trim()
            || !chars_within(&self.event_id, MAX_EVENT_ID_CHARS)
            || self.event_id.chars().any(char::is_control)
        {
            return Err(SdkError::InvalidMemory("event id is invalid"));
        }
        let experience = self.experience.trim();
        if experience.is_empty()
            || !chars_within(experience, MAX_EXPERIENCE_CHARS)
            || experience.chars().any(char::is_control)
        {
            return Err(SdkError::InvalidMemory("event experience is invalid"));
        }
        self.influence
            .validate()
            .map_err(|_| SdkError::InvalidMemory("event influence is invalid"))?;
        Ok(())
    }
}

/// Result of applying (or skipping) one interaction event.
#[derive(Debug, Clone, PartialEq)]
pub struct EventApplication {
    pub event_id: String,
    pub core_uuid: String,
    /// `false` when the event id had already been applied to this Core.
    pub applied: bool,
    /// The bounded experience summary carried by the event.
    pub experience: String,
    /// The Core's emotion state after the call (unchanged when skipped).
    pub emotion: EmotionState,
}

/// Build the fixed, documented interaction influence.
pub fn interaction_influence() -> EmotionInfluence {
    EmotionInfluence {
        source: INTERACTION_INFLUENCE_SOURCE.to_owned(),
        weight: INTERACTION_INFLUENCE_WEIGHT,
        delta: INTERACTION_DELTA,
    }
}

/// Apply one interaction event against the current wall clock.
pub fn apply_interaction_event(
    store: &PersonalStore,
    event: &InteractionEvent,
) -> Result<EventApplication, SdkError> {
    apply_interaction_event_at(store, event, Utc::now())
}

/// Deterministic variant of [`apply_interaction_event`].
///
/// The event is claimed in the Core's ledger before any experience or emotion is
/// written. A repeated event id therefore returns `applied = false` and leaves
/// both the experience log and the emotion state untouched.
pub fn apply_interaction_event_at(
    store: &PersonalStore,
    event: &InteractionEvent,
    now: DateTime<Utc>,
) -> Result<EventApplication, SdkError> {
    event.validate()?;
    let experience = event.experience_summary();
    let ledger = EventLedger::open(store.database_path())?;

    if !ledger.claim(store.core_uuid(), &event.event_id, &experience, now)? {
        return Ok(EventApplication {
            event_id: event.event_id.clone(),
            core_uuid: store.core_uuid().to_owned(),
            applied: false,
            experience,
            emotion: store.emotion_state()?,
        });
    }

    let result = (|| -> Result<EmotionState, SdkError> {
        store.record_experience_at(&experience, now)?;
        store.apply_influences_at(std::slice::from_ref(&event.influence), now)
    })();

    match result {
        Ok(emotion) => {
            ledger.record_revision(store.core_uuid(), &event.event_id, emotion.revision)?;
            Ok(EventApplication {
                event_id: event.event_id.clone(),
                core_uuid: store.core_uuid().to_owned(),
                applied: true,
                experience,
                emotion,
            })
        }
        Err(error) => {
            let _ = ledger.release(store.core_uuid(), &event.event_id);
            Err(error)
        }
    }
}

struct EventLedger {
    connection: Connection,
}

impl EventLedger {
    fn open(personal_path: &Path) -> Result<Self, SdkError> {
        let connection = Connection::open(personal_path)?;
        connection.busy_timeout(Duration::from_secs(5))?;
        connection.execute_batch(LEDGER_SQL)?;
        Ok(Self { connection })
    }

    /// Claim an event id. Returns `true` when this call claimed it and `false`
    /// when the same id was already applied to this Core.
    fn claim(
        &self,
        core_uuid: &str,
        event_id: &str,
        experience: &str,
        now: DateTime<Utc>,
    ) -> Result<bool, SdkError> {
        let changed = self.connection.execute(
            "INSERT OR IGNORE INTO applied_interaction_events(
                event_id, core_uuid, applied_at, experience
             ) VALUES(?1, ?2, ?3, ?4)",
            params![event_id, core_uuid, now.to_rfc3339(), experience],
        )?;
        if changed == 1 {
            return Ok(true);
        }
        let existing: Option<String> = self
            .connection
            .query_row(
                "SELECT core_uuid FROM applied_interaction_events WHERE event_id = ?1",
                [event_id],
                |row| row.get(0),
            )
            .optional()?;
        match existing {
            Some(stored) if stored.as_str() == core_uuid => Ok(false),
            Some(_) => Err(SdkError::OwnershipMismatch),
            None => Err(SdkError::CorruptData("event ledger claim is missing")),
        }
    }

    fn record_revision(
        &self,
        core_uuid: &str,
        event_id: &str,
        revision: u64,
    ) -> Result<(), SdkError> {
        let revision = i64::try_from(revision)
            .map_err(|_| SdkError::InvalidMemory("event emotion revision is out of range"))?;
        self.connection.execute(
            "UPDATE applied_interaction_events SET emotion_revision = ?3
             WHERE event_id = ?1 AND core_uuid = ?2",
            params![event_id, core_uuid, revision],
        )?;
        Ok(())
    }

    fn release(&self, core_uuid: &str, event_id: &str) -> Result<(), SdkError> {
        self.connection.execute(
            "DELETE FROM applied_interaction_events WHERE event_id = ?1 AND core_uuid = ?2",
            params![event_id, core_uuid],
        )?;
        Ok(())
    }
}
