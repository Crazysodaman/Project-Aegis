use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Barrier};
use std::time::Instant;

use aegis_contracts::initialization::{CoreInitializationRequest, CORE_INIT_REQUEST_SCHEMA};
use aegis_core_install::CoreInstaller;
use aegis_core_sdk::{
    CoreSdkContext, EmotionDelta, EmotionInfluence, MemoryKind, MemoryQuery, NewMemory,
    PersonalStore, MAX_CONTENT_BYTES, MAX_EXPERIENCE_CHARS, MAX_OPEN_THREADS,
    MAX_OPEN_THREAD_CHARS, MAX_RECENT_EXPERIENCES, MAX_SESSION_ID_CHARS,
};
use aegis_platform::PlatformService;
use aegis_storage::now_rfc3339;
use chrono::{Duration, Utc};
use rusqlite::Connection;

const CORE_A: &str = "a9a0a604-a4e7-4a88-a985-3f071e2c8e7d";
const CORE_B: &str = "11111111-1111-4111-8111-111111111111";
const CORE_C: &str = "22222222-2222-4222-8222-222222222222";
const CORE_D: &str = "33333333-3333-4333-8333-333333333333";
const CORES: [&str; 4] = [CORE_A, CORE_B, CORE_C, CORE_D];

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
    for (index, (core_uuid, number)) in CORES.iter().zip([1_u8, 2, 3, 6]).enumerate() {
        installer
            .initialize(
                &request(core_uuid, number, &format!("Core {index}")),
                &initialized.owner_token,
            )
            .expect("install core");
    }
    Fixture { _temp: temp, root }
}

fn influence(source: &str) -> EmotionInfluence {
    EmotionInfluence {
        source: source.to_owned(),
        weight: 0.3,
        delta: EmotionDelta {
            valence: 0.1,
            arousal: 0.05,
            joy: 0.2,
            calm: 0.1,
            curiosity: 0.15,
            tension: 0.0,
        },
    }
}

fn personal_db(root: &Path, core_uuid: &str) -> PathBuf {
    root.join("cores").join(core_uuid).join("personal.db")
}

fn assert_single_owner(path: &Path, core_uuid: &str, table: &str, expected_rows: i64) {
    let connection = Connection::open(path).expect("raw personal db");
    let owners: i64 = connection
        .query_row(
            &format!("SELECT COUNT(DISTINCT core_uuid) FROM {table}"),
            [],
            |row| row.get(0),
        )
        .expect("distinct owners");
    assert_eq!(owners, 1, "{table} must contain one owner");
    let owner: String = connection
        .query_row(
            &format!("SELECT DISTINCT core_uuid FROM {table} LIMIT 1"),
            [],
            |row| row.get(0),
        )
        .expect("owner");
    assert_eq!(owner, core_uuid);
    let rows: i64 = connection
        .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
            row.get(0)
        })
        .expect("row count");
    assert_eq!(rows, expected_rows, "{table} row count");
}

#[test]
fn concurrent_multi_core_personal_access_is_isolated() {
    let fixture = fixture();
    let root = Arc::new(fixture.root.clone());
    let barrier = Arc::new(Barrier::new(CORES.len()));
    let writes = 6;
    let started = Instant::now();
    let mut handles = Vec::with_capacity(CORES.len());
    for core_uuid in CORES {
        let root = Arc::clone(&root);
        let barrier = Arc::clone(&barrier);
        handles.push(std::thread::spawn(move || {
            let store = PersonalStore::open(root.as_ref(), core_uuid).expect("personal store");
            let now = Utc::now();
            barrier.wait();
            for index in 0..writes {
                let state = store
                    .apply_influences_at(
                        &[influence(&format!("{core_uuid}-influence-{index}"))],
                        now + Duration::seconds(index + 1),
                    )
                    .expect("influence");
                assert_eq!(state.core_uuid, core_uuid);
                store
                    .record_experience_at(
                        &format!("{core_uuid} experience {index}"),
                        now + Duration::seconds(index + 1),
                    )
                    .expect("experience");
                let thread = store
                    .open_thread(&format!("{core_uuid} thread {index}"))
                    .expect("thread");
                assert!(!thread.is_empty());
            }
            store
                .emotion_history(100)
                .expect("history")
                .into_iter()
                .all(|entry| entry.state.core_uuid == core_uuid)
        }));
    }
    for handle in handles {
        assert!(handle.join().expect("core thread"), "history ownership");
    }
    let elapsed = started.elapsed();

    for core_uuid in CORES {
        let path = personal_db(&fixture.root, core_uuid);
        assert_single_owner(&path, core_uuid, "personal_events", writes);
        assert_single_owner(&path, core_uuid, "open_threads", writes);
        assert_single_owner(&path, core_uuid, "emotion_history", writes);

        let store = PersonalStore::open(&fixture.root, core_uuid).expect("reopen");
        let summary = store.continuity_summary().expect("summary");
        assert!(summary
            .recent_experiences
            .iter()
            .all(|item| item.starts_with(core_uuid)));
        assert!(summary
            .open_threads
            .iter()
            .all(|item| item.starts_with(core_uuid)));
    }
    eprintln!(
        "concurrent personal access: {} cores x {writes} writes in {elapsed:?}",
        CORES.len()
    );
}

#[test]
fn repeated_open_reopen_cycles_do_not_leak_or_corrupt_stores() {
    let fixture = fixture();
    let cycles = 25;
    let now = Utc::now();
    let started = Instant::now();
    for cycle in 0..cycles {
        let context = CoreSdkContext::open(&fixture.root, CORE_A).expect("context");
        let personal = context.personal().expect("personal");
        let before = personal.emotion_state().expect("emotion");
        let after = personal
            .apply_influences_at(
                &[influence(&format!("cycle-{cycle}"))],
                now + Duration::seconds(cycle as i64 + 1),
            )
            .expect("influence");
        assert_eq!(after.revision, before.revision + 1);
        personal
            .record_experience_at(
                &format!("cycle {cycle}"),
                now + Duration::seconds(cycle as i64 + 1),
            )
            .expect("experience");
        context
            .memory()
            .create(NewMemory::new(
                MemoryKind::Fact,
                format!("cycle {cycle} fact"),
            ))
            .expect("memory");
    }
    let elapsed = started.elapsed();

    let reopened = CoreSdkContext::open(&fixture.root, CORE_A).expect("reopen");
    let personal = reopened.personal().expect("personal");
    let history = personal.emotion_history(100).expect("history");
    assert_eq!(history.len(), cycles);
    assert!(
        personal.emotion_state().expect("emotion").revision
            > u64::try_from(cycles).expect("cycle count")
    );
    let memories = reopened
        .memory()
        .search(&MemoryQuery::new().with_limit(100))
        .expect("search");
    assert_eq!(memories.len(), cycles);
    drop(reopened);

    fs::remove_dir_all(&fixture.root).expect("no leaked store handles");
    eprintln!("{cycles} open/reopen cycles in {elapsed:?}");
}

#[test]
fn large_but_bounded_inputs_are_accepted_and_summaries_stay_bounded() {
    let fixture = fixture();
    let context = CoreSdkContext::open(&fixture.root, CORE_A).expect("context");
    let personal = context.personal().expect("personal");

    let content = "x".repeat(MAX_CONTENT_BYTES);
    let entry = context
        .memory()
        .create(NewMemory::new(MemoryKind::Fact, content.clone()))
        .expect("max content");
    assert_eq!(entry.content.len(), MAX_CONTENT_BYTES);
    assert!(context
        .memory()
        .create(NewMemory::new(
            MemoryKind::Fact,
            "x".repeat(MAX_CONTENT_BYTES + 1)
        ))
        .is_err());

    let experience = "e".repeat(MAX_EXPERIENCE_CHARS);
    personal
        .record_experience(&experience)
        .expect("max experience");
    assert!(personal
        .record_experience(&"e".repeat(MAX_EXPERIENCE_CHARS + 1))
        .is_err());

    let thread = "t".repeat(MAX_OPEN_THREAD_CHARS);
    personal.open_thread(&thread).expect("max thread");
    assert!(personal
        .open_thread(&"t".repeat(MAX_OPEN_THREAD_CHARS + 1))
        .is_err());

    let session = "s".repeat(MAX_SESSION_ID_CHARS);
    personal
        .update_last_active(Some(&session))
        .expect("max session id");
    assert!(personal
        .update_last_active(Some(&"s".repeat(MAX_SESSION_ID_CHARS + 1)))
        .is_err());

    for index in 0..MAX_RECENT_EXPERIENCES + 4 {
        personal
            .record_experience(&format!("experience-{index}"))
            .expect("experience");
    }
    for index in 0..MAX_OPEN_THREADS + 4 {
        personal
            .open_thread(&format!("thread-{index}"))
            .expect("thread");
    }

    let summary = personal.continuity_summary().expect("summary");
    assert!(summary.recent_experiences.len() <= MAX_RECENT_EXPERIENCES);
    assert!(summary.open_threads.len() <= MAX_OPEN_THREADS);
    assert!(summary
        .recent_experiences
        .iter()
        .all(|item| item.chars().count() <= MAX_EXPERIENCE_CHARS));
    assert!(summary
        .open_threads
        .iter()
        .all(|item| item.chars().count() <= MAX_OPEN_THREAD_CHARS));
    assert_eq!(summary.last_session_id.as_deref(), Some(session.as_str()));
}
