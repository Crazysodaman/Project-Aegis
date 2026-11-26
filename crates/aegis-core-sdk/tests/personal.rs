use std::fs;
use std::path::{Path, PathBuf};

use aegis_contracts::initialization::{CoreInitializationRequest, CORE_INIT_REQUEST_SCHEMA};
use aegis_contracts::personal::MAX_ELAPSED_OFFLINE_SECONDS;
use aegis_core_install::CoreInstaller;
use aegis_core_sdk::{
    Activity, ActivityStatus, CoreMemoryStore, CoreSdkContext, EmotionDelta, EmotionHistoryEntry,
    EmotionInfluence, OutfitDecisionRecord, PersonalStore, PersonalityProfile, Routine, SdkError,
    MAX_EMOTION_HISTORY_LIMIT, MAX_EXPERIENCE_CHARS, MAX_OPEN_THREAD_CHARS,
    PERSONAL_APPLICATION_ID, PERSONAL_DB_FILE, PERSONAL_SCHEMA,
};
use aegis_core_storage::{CoreDatabase, CoreStorageLayout};
use aegis_platform::PlatformService;
use aegis_storage::now_rfc3339;
use chrono::{Duration, Utc};
use rusqlite::{params, Connection};

const CORE_A: &str = "a9a0a604-a4e7-4a88-a985-3f071e2c8e7d";
const CORE_B: &str = "11111111-1111-4111-8111-111111111111";

struct Fixture {
    _temp: tempfile::TempDir,
    root: PathBuf,
}

fn request(core_uuid: &str, core_number: u8, display_name: &str) -> CoreInitializationRequest {
    CoreInitializationRequest {
        schema: CORE_INIT_REQUEST_SCHEMA.to_owned(),
        core_uuid: core_uuid.to_owned(),
        core_number,
        display_name: display_name.to_owned(),
        requested_at: now_rfc3339().expect("timestamp"),
    }
}

fn fixture() -> Fixture {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path().to_path_buf();
    let initialized = PlatformService::initialize(&root).expect("platform initialize");
    let installer = CoreInstaller::new(&root).expect("installer");
    installer
        .initialize(&request(CORE_A, 1, "Core A"), &initialized.owner_token)
        .expect("install A");
    installer
        .initialize(&request(CORE_B, 2, "Core B"), &initialized.owner_token)
        .expect("install B");
    Fixture { _temp: temp, root }
}

fn layout(root: &Path) -> CoreStorageLayout {
    CoreStorageLayout::new(root).expect("layout")
}

fn personal_path(root: &Path, core_uuid: &str) -> PathBuf {
    layout(root)
        .core_dir(core_uuid)
        .expect("core dir")
        .join(PERSONAL_DB_FILE)
}

#[allow(clippy::too_many_arguments)]
fn influence(
    source: &str,
    weight: f32,
    valence: f32,
    arousal: f32,
    joy: f32,
    calm: f32,
    curiosity: f32,
    tension: f32,
) -> EmotionInfluence {
    EmotionInfluence {
        source: source.to_owned(),
        weight,
        delta: EmotionDelta {
            valence,
            arousal,
            joy,
            calm,
            curiosity,
            tension,
        },
    }
}

fn activity(id: &str, title: &str, status: ActivityStatus) -> Activity {
    let now = now_rfc3339().expect("timestamp");
    Activity {
        id: id.to_owned(),
        title: title.to_owned(),
        status,
        started_at: now.clone(),
        updated_at: now,
        notes: String::new(),
    }
}

fn approx(actual: f32, expected: f32) {
    assert!(
        (actual - expected).abs() < 1e-4,
        "expected {expected}, found {actual}"
    );
}

#[test]
fn personal_store_binds_identity_and_opens_database() {
    let fixture = fixture();
    let snapshot =
        CoreDatabase::read_snapshot(&layout(&fixture.root), CORE_A).expect("core snapshot");
    let store = PersonalStore::open(&fixture.root, CORE_A).expect("personal store");

    assert_eq!(store.core_uuid(), CORE_A);
    assert_eq!(
        store.database_instance_uuid(),
        snapshot.metadata.database_instance_uuid
    );
    assert_eq!(
        store.database_path().parent(),
        Some(layout(&fixture.root).core_dir(CORE_A).unwrap().as_path())
    );
    assert_eq!(
        store
            .database_path()
            .file_name()
            .and_then(|name| name.to_str()),
        Some(PERSONAL_DB_FILE)
    );
    assert!(store.database_path().is_file());
    assert_ne!(store.database_path(), fixture.root.join("platform.db"));
    assert_ne!(store.database_path(), fixture.root.join("safe.db"));
    assert_ne!(
        store.database_path(),
        layout(&fixture.root)
            .core_dir(CORE_A)
            .unwrap()
            .join("core.db")
    );
    assert_ne!(
        store.database_path(),
        layout(&fixture.root)
            .core_dir(CORE_A)
            .unwrap()
            .join("memory.db")
    );

    let path = store.database_path().to_path_buf();
    drop(store);

    let connection = Connection::open(&path).expect("raw personal db");
    let application_id: i64 = connection
        .query_row("PRAGMA application_id", [], |row| row.get(0))
        .expect("application id");
    assert_eq!(application_id, PERSONAL_APPLICATION_ID);
    let journal_mode: String = connection
        .query_row("PRAGMA journal_mode", [], |row| row.get(0))
        .expect("journal mode");
    assert_eq!(journal_mode.to_lowercase(), "delete");
    let synchronous: i64 = connection
        .query_row("PRAGMA synchronous", [], |row| row.get(0))
        .expect("synchronous");
    assert_eq!(synchronous, 2);

    let ownership: (String, String, String) = connection
        .query_row(
            "SELECT core_uuid, database_instance_uuid, schema FROM personal_ownership",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .expect("ownership row");
    assert_eq!(ownership.0, CORE_A);
    assert_eq!(ownership.1, snapshot.metadata.database_instance_uuid);
    assert_eq!(ownership.2, PERSONAL_SCHEMA);
    let count: i64 = connection
        .query_row("SELECT COUNT(*) FROM personal_ownership", [], |row| {
            row.get(0)
        })
        .expect("ownership count");
    assert_eq!(count, 1);
}

#[test]
fn two_cores_keep_distinct_emotions_and_history() {
    let fixture = fixture();
    let store_a = PersonalStore::open(&fixture.root, CORE_A).expect("A store");
    let store_b = PersonalStore::open(&fixture.root, CORE_B).expect("B store");
    let now = Utc::now();

    store_a
        .apply_influences_at(
            &[influence("praise", 1.0, 0.6, 0.0, 0.8, 0.0, 0.0, 0.0)],
            now,
        )
        .expect("A influence");
    store_b
        .apply_influences_at(
            &[influence("stress", 1.0, -0.4, 0.6, 0.0, 0.0, 0.0, 0.7)],
            now,
        )
        .expect("B influence");

    let state_a = store_a.emotion_state().expect("A state");
    let state_b = store_b.emotion_state().expect("B state");
    assert!(state_a.dimensions.joy > 0.0);
    assert_eq!(state_b.dimensions.joy, 0.0);
    assert!(state_b.dimensions.tension > 0.0);
    assert_eq!(state_a.dimensions.tension, 0.0);
    assert_ne!(state_a.dimensions, state_b.dimensions);

    let history_a = store_a.emotion_history(10).expect("A history");
    let history_b = store_b.emotion_history(10).expect("B history");
    assert_eq!(history_a.len(), 1);
    assert_eq!(history_b.len(), 1);
    assert_eq!(history_a[0].source, "praise");
    assert_eq!(history_b[0].source, "stress");
    assert_ne!(history_a[0].state.dimensions, history_b[0].state.dimensions);

    store_a
        .record_experience("A-only experience")
        .expect("A exp");
    let thread = store_a.open_thread("A-only thread").expect("A thread");
    assert!(!thread.is_empty());

    let summary_b = store_b.continuity_summary().expect("B summary");
    assert!(summary_b.recent_experiences.is_empty());
    assert!(summary_b.open_threads.is_empty());
    assert_ne!(store_a.database_path(), store_b.database_path());
}

#[test]
fn personal_state_persists_across_reopen() {
    let fixture = fixture();
    let now = Utc::now();
    let (state, history_len, thread_id) = {
        let store = PersonalStore::open(&fixture.root, CORE_A).expect("store");
        let state = store
            .apply_influences_at(
                &[influence("calm morning", 0.8, 0.4, 0.1, 0.5, 0.6, 0.2, 0.0)],
                now,
            )
            .expect("influence");
        store
            .record_experience_at("read a chapter", now)
            .expect("experience");
        let thread_id = store.open_thread("finish the chapter").expect("thread");
        store
            .update_last_active_at(Some("session-7"), now)
            .expect("last active");
        (
            state,
            store.emotion_history(10).expect("history").len(),
            thread_id,
        )
    };

    let reopened = PersonalStore::open(&fixture.root, CORE_A).expect("reopen");
    assert_eq!(reopened.emotion_state().expect("state"), state);
    assert_eq!(
        reopened.emotion_history(10).expect("history").len(),
        history_len
    );

    let summary = reopened
        .continuity_summary_at(now + Duration::seconds(60))
        .expect("summary");
    assert_eq!(summary.last_session_id.as_deref(), Some("session-7"));
    assert_eq!(summary.elapsed_offline_seconds, 60);
    assert_eq!(
        summary.recent_experiences,
        vec!["read a chapter".to_owned()]
    );
    assert_eq!(summary.open_threads, vec!["finish the chapter".to_owned()]);
    assert!(reopened
        .list_open_threads(10)
        .expect("threads")
        .iter()
        .any(|thread| thread.id == thread_id));
}

#[test]
fn elapsed_offline_is_computed_and_bounded() {
    let fixture = fixture();
    let store = PersonalStore::open(&fixture.root, CORE_A).expect("store");
    let t0 = Utc::now();
    store
        .update_last_active_at(Some("session-1"), t0)
        .expect("last active");

    let summary = store
        .continuity_summary_at(t0 + Duration::seconds(3600))
        .expect("summary");
    assert_eq!(summary.elapsed_offline_seconds, 3600);
    assert_eq!(summary.last_session_id.as_deref(), Some("session-1"));

    let earlier = store
        .continuity_summary_at(t0 - Duration::seconds(600))
        .expect("earlier summary");
    assert_eq!(earlier.elapsed_offline_seconds, 0);

    let bounded = store
        .continuity_summary_at(t0 + Duration::days(365 * 100))
        .expect("bounded summary");
    assert_eq!(bounded.elapsed_offline_seconds, MAX_ELAPSED_OFFLINE_SECONDS);
}

#[test]
fn ownership_and_instance_mismatch_are_rejected() {
    let first = fixture();
    {
        let store = PersonalStore::open(&first.root, CORE_A).expect("A store");
        store
            .apply_influences(&[influence("warmth", 0.5, 0.2, 0.0, 0.2, 0.0, 0.0, 0.0)])
            .expect("influence");
    }
    let a_path = personal_path(&first.root, CORE_A);
    let b_path = personal_path(&first.root, CORE_B);
    assert!(!b_path.exists());
    fs::copy(&a_path, &b_path).expect("copy A personal over B");
    let copied = PersonalStore::open(&first.root, CORE_B).expect_err("copied ownership");
    assert!(matches!(copied, SdkError::OwnershipMismatch));

    let second = fixture();
    let wrong_instance =
        CoreDatabase::read_snapshot(&layout(&second.root), CORE_B).expect("B snapshot");
    let raw_path = personal_path(&second.root, CORE_A);
    let connection = Connection::open(&raw_path).expect("raw personal");
    connection
        .execute_batch(&format!(
            "PRAGMA application_id = {PERSONAL_APPLICATION_ID};"
        ))
        .expect("application id");
    connection
        .execute_batch(
            "CREATE TABLE personal_ownership (
                singleton INTEGER NOT NULL PRIMARY KEY CHECK (singleton = 1),
                core_uuid TEXT NOT NULL,
                database_instance_uuid TEXT NOT NULL,
                schema TEXT NOT NULL,
                created_at TEXT NOT NULL
             );",
        )
        .expect("ownership table");
    connection
        .execute(
            "INSERT INTO personal_ownership(
                singleton, core_uuid, database_instance_uuid, schema, created_at
             ) VALUES(1, ?1, ?2, ?3, ?4)",
            params![
                CORE_A,
                wrong_instance.metadata.database_instance_uuid,
                PERSONAL_SCHEMA,
                now_rfc3339().expect("timestamp")
            ],
        )
        .expect("ownership row");
    drop(connection);

    let mismatch =
        PersonalStore::open(&second.root, CORE_A).expect_err("instance mismatch must fail");
    assert!(matches!(mismatch, SdkError::OwnershipMismatch));
}

#[test]
fn emotion_decay_and_multiple_influences_are_deterministic() {
    let fixture = fixture();
    let store_a = PersonalStore::open(&fixture.root, CORE_A).expect("A store");
    let store_b = PersonalStore::open(&fixture.root, CORE_B).expect("B store");
    let t0 = Utc::now();
    let influences = vec![
        influence("praise", 0.5, 0.8, 0.0, 0.6, 0.0, 0.0, 0.0),
        influence("surprise", 1.0, 0.2, 0.4, 0.0, 0.0, 0.5, 0.0),
    ];

    let state_a = store_a
        .apply_influences_at(&influences, t0)
        .expect("A influence");
    let state_b = store_b
        .apply_influences_at(&influences, t0)
        .expect("B influence");

    approx(state_a.dimensions.valence, 0.6);
    approx(state_a.dimensions.joy, 0.3);
    approx(state_a.dimensions.arousal, 0.4);
    approx(state_a.dimensions.curiosity, 0.5);
    assert_eq!(state_a.dimensions, state_b.dimensions);
    approx(state_a.intensity, state_b.intensity);
    assert_eq!(state_a.revision, state_b.revision);
    assert_eq!(
        store_a.emotion_history(1).expect("A history")[0].source,
        "praise,surprise"
    );

    let decayed = store_a
        .apply_influences_at(&[], t0 + Duration::minutes(90))
        .expect("decay");
    approx(
        decayed.dimensions.valence,
        state_a.dimensions.valence * 0.25,
    );
    approx(decayed.dimensions.joy, state_a.dimensions.joy * 0.25);
    approx(
        decayed.dimensions.curiosity,
        state_a.dimensions.curiosity * 0.25,
    );
    approx(decayed.intensity, state_a.intensity * 0.25);
    assert_eq!(decayed.revision, state_a.revision + 1);
    assert_eq!(
        store_a.emotion_history(1).expect("history")[0].source,
        "decay"
    );
}

#[test]
fn emotion_history_and_append_only_tables_reject_mutation() {
    let fixture = fixture();
    let now = Utc::now();
    {
        let store = PersonalStore::open(&fixture.root, CORE_A).expect("store");
        store
            .apply_influences_at(
                &[influence("first", 1.0, 0.3, 0.0, 0.3, 0.0, 0.0, 0.0)],
                now,
            )
            .expect("influence one");
        store
            .apply_influences_at(
                &[influence("second", 1.0, 0.2, 0.0, 0.2, 0.0, 0.0, 0.0)],
                now,
            )
            .expect("influence two");
        store
            .record_experience_at("event", now)
            .expect("experience");
        store
            .record_outfit(&OutfitDecisionRecord {
                id: "outfit-1".to_owned(),
                decided_at: now.to_rfc3339(),
                asset_ids: vec!["asset-1".to_owned()],
                rationale: "simple".to_owned(),
            })
            .expect("outfit");
        assert_eq!(store.emotion_history(10).expect("history").len(), 2);
    }

    let connection = Connection::open(personal_path(&fixture.root, CORE_A)).expect("raw db");
    for statement in [
        "UPDATE emotion_history SET source = 'tampered'",
        "DELETE FROM emotion_history",
        "UPDATE personal_events SET detail = 'tampered'",
        "DELETE FROM personal_events",
        "UPDATE outfit_decisions SET rationale = 'tampered'",
        "DELETE FROM outfit_decisions",
        "UPDATE personal_ownership SET core_uuid = 'tampered'",
        "DELETE FROM personal_ownership",
    ] {
        assert!(
            connection.execute(statement, []).is_err(),
            "statement must be rejected: {statement}"
        );
    }
    let history: i64 = connection
        .query_row("SELECT COUNT(*) FROM emotion_history", [], |row| row.get(0))
        .expect("history count");
    assert_eq!(history, 2);
    let events: i64 = connection
        .query_row("SELECT COUNT(*) FROM personal_events", [], |row| row.get(0))
        .expect("event count");
    assert_eq!(events, 1);
    let outfits: i64 = connection
        .query_row("SELECT COUNT(*) FROM outfit_decisions", [], |row| {
            row.get(0)
        })
        .expect("outfit count");
    assert_eq!(outfits, 1);
    let ownership: i64 = connection
        .query_row("SELECT COUNT(*) FROM personal_ownership", [], |row| {
            row.get(0)
        })
        .expect("ownership count");
    assert_eq!(ownership, 1);

    let store = PersonalStore::open(&fixture.root, CORE_A).expect("reopen");
    let bounded = store.emotion_history(usize::MAX).expect("bounded history");
    assert_eq!(bounded.len(), 2);
    assert!(bounded.len() <= MAX_EMOTION_HISTORY_LIMIT);
}

#[test]
fn continuity_and_listings_are_bounded() {
    let fixture = fixture();
    let store = PersonalStore::open(&fixture.root, CORE_A).expect("store");
    let now = Utc::now();

    for index in 0..12 {
        store
            .record_experience_at(&format!("experience {index}"), now)
            .expect("experience");
    }
    for index in 0..10 {
        store
            .open_thread(&format!("thread {index}"))
            .expect("thread");
    }
    for index in 0..12 {
        store
            .upsert_activity(&activity(
                &format!("act-{index:02}"),
                &format!("Activity {index}"),
                ActivityStatus::Active,
            ))
            .expect("activity");
    }
    store
        .set_routine(&Routine {
            id: "routine-1".to_owned(),
            name: "Morning stretch".to_owned(),
            time_of_day: Some("morning".to_owned()),
            notes: String::new(),
        })
        .expect("routine");
    store
        .set_routine(&Routine {
            id: "routine-2".to_owned(),
            name: "Evening review".to_owned(),
            time_of_day: Some("evening".to_owned()),
            notes: "short".to_owned(),
        })
        .expect("routine two");

    let summary = store.continuity_summary_at(now).expect("summary");
    assert_eq!(summary.recent_experiences.len(), 8);
    assert_eq!(summary.open_threads.len(), 8);
    assert_eq!(summary.activities.len(), 8);
    assert_eq!(store.list_activities(100).expect("activities").len(), 12);
    assert_eq!(store.list_routines(10).expect("routines").len(), 2);
    assert_eq!(store.list_open_threads(100).expect("threads").len(), 10);

    let too_long_experience = "x".repeat(MAX_EXPERIENCE_CHARS + 1);
    assert!(store.record_experience(&too_long_experience).is_err());
    let too_long_thread = "x".repeat(MAX_OPEN_THREAD_CHARS + 1);
    assert!(store.open_thread(&too_long_thread).is_err());

    let held = store.open_thread("resolvable").expect("thread");
    store.resolve_thread(&held).expect("resolve");
    assert!(store.resolve_thread(&held).is_ok());
    assert!(store.resolve_thread("missing-thread").is_err());
}

#[test]
fn outfits_and_routines_round_trip() {
    let fixture = fixture();
    let store = PersonalStore::open(&fixture.root, CORE_A).expect("store");
    let now = now_rfc3339().expect("timestamp");

    store
        .record_outfit(&OutfitDecisionRecord {
            id: "outfit-a".to_owned(),
            decided_at: now.clone(),
            asset_ids: vec!["asset-1".to_owned(), "asset-2".to_owned()],
            rationale: "cool weather".to_owned(),
        })
        .expect("outfit a");
    store
        .record_outfit(&OutfitDecisionRecord {
            id: "outfit-b".to_owned(),
            decided_at: now,
            asset_ids: vec!["asset-3".to_owned()],
            rationale: "evening".to_owned(),
        })
        .expect("outfit b");

    let outfits = store.outfit_decisions(10).expect("outfits");
    assert_eq!(outfits.len(), 2);
    assert!(outfits.iter().any(|record| record.id == "outfit-a"));
    assert!(outfits
        .iter()
        .any(|record| record.asset_ids == vec!["asset-1".to_owned(), "asset-2".to_owned()]));
    assert!(store
        .record_outfit(&OutfitDecisionRecord {
            id: "outfit-a".to_owned(),
            decided_at: "2026-11-26T00:00:00Z".to_owned(),
            asset_ids: Vec::new(),
            rationale: String::new(),
        })
        .is_err());

    store
        .set_routine(&Routine {
            id: "routine-a".to_owned(),
            name: "Reading".to_owned(),
            time_of_day: Some("evening".to_owned()),
            notes: "one chapter".to_owned(),
        })
        .expect("routine");
    store
        .set_routine(&Routine {
            id: "routine-a".to_owned(),
            name: "Reading".to_owned(),
            time_of_day: Some("night".to_owned()),
            notes: "one chapter".to_owned(),
        })
        .expect("routine update");
    let routines = store.list_routines(10).expect("routines");
    assert_eq!(routines.len(), 1);
    assert_eq!(routines[0].time_of_day.as_deref(), Some("night"));
}

#[test]
fn personality_modulates_influence_weight() {
    let fixture = fixture();
    let memory_a = CoreMemoryStore::open(&fixture.root, CORE_A).expect("A memory");
    let mut profile = PersonalityProfile::neutral(CORE_A).expect("neutral");
    profile.traits.insert("sensitivity".to_owned(), 1.0);
    memory_a
        .store_personality(&profile)
        .expect("store personality");
    drop(memory_a);

    let store_a = PersonalStore::open(&fixture.root, CORE_A).expect("A personal");
    let store_b = PersonalStore::open(&fixture.root, CORE_B).expect("B personal");
    let now = Utc::now();
    let influences = [influence("praise", 0.4, 0.0, 0.0, 0.5, 0.0, 0.0, 0.0)];

    let state_a = store_a
        .apply_influences_at(&influences, now)
        .expect("A influence");
    let state_b = store_b
        .apply_influences_at(&influences, now)
        .expect("B influence");

    approx(state_a.dimensions.joy, 0.3);
    approx(state_b.dimensions.joy, 0.2);
    assert!(state_a.dimensions.joy > state_b.dimensions.joy);
    assert!(state_a.intensity > state_b.intensity);
}

#[test]
fn core_sdk_context_integrates_personal_store() {
    let fixture = fixture();
    let context = CoreSdkContext::open(&fixture.root, CORE_A).expect("context");
    let personal = context.personal().expect("personal");
    assert_eq!(personal.core_uuid(), CORE_A);
    assert_eq!(context.identity.core_uuid, CORE_A);

    let state = personal
        .apply_influences(&[influence("context", 1.0, 0.1, 0.0, 0.1, 0.0, 0.0, 0.0)])
        .expect("influence");
    assert!(state.dimensions.joy > 0.0);

    let reopened = CoreSdkContext::open(&fixture.root, CORE_A).expect("reopen");
    assert_eq!(
        reopened
            .personal()
            .expect("personal")
            .emotion_state()
            .expect("state")
            .dimensions,
        state.dimensions
    );
}

#[test]
fn history_entry_shape_is_stable() {
    let fixture = fixture();
    let store = PersonalStore::open(&fixture.root, CORE_A).expect("store");
    store
        .apply_influences_at(
            &[influence("single", 1.0, 0.1, 0.2, 0.3, 0.4, 0.5, 0.6)],
            Utc::now(),
        )
        .expect("influence");
    let history: Vec<EmotionHistoryEntry> = store.emotion_history(1).expect("history");
    assert_eq!(history.len(), 1);
    let entry = &history[0];
    assert_eq!(entry.state.core_uuid, CORE_A);
    assert_eq!(entry.state.revision, 2);
    assert_eq!(entry.sequence, 1);
    assert_eq!(entry.source, "single");
}
