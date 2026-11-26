//! Explicit important-experience consolidation and bounded session summaries.
//!
//! These tests use only public APIs and a fresh temporary root per test. They
//! assert that consolidation is explicit, bounded, idempotent by a stable
//! source key, and always stays inside the owning Core's memory database.

use std::path::{Path, PathBuf};

use aegis_contracts::initialization::{CoreInitializationRequest, CORE_INIT_REQUEST_SCHEMA};
use aegis_conversation::{
    ConsolidationOutcome, ConversationEngine, ConversationError, ImportantExperience, MessageRole,
    NewMessage, SessionState, CONSOLIDATION_SOURCE_PREFIX, MAX_CONSOLIDATION_BATCH,
    MAX_CONSOLIDATION_CONTENT_CHARS, SESSION_SUMMARY_SCHEMA,
};
use aegis_core_install::CoreInstaller;
use aegis_core_sdk::{CoreMemoryStore, MemoryKind, MemoryQuery};
use aegis_platform::PlatformService;
use aegis_storage::now_rfc3339;

const CORE_A: &str = "a9a0a604-a4e7-4a88-a985-3f071e2c8e7d";
const CORE_B: &str = "11111111-1111-4111-8111-111111111111";

fn setup() -> (tempfile::TempDir, PathBuf) {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path().join("aegis-root");
    let initialized = PlatformService::initialize(&root).expect("platform");
    let installer = CoreInstaller::new(&root).expect("installer");
    for (uuid, number, name) in [(CORE_A, 1, "Sofía Ada Lyra"), (CORE_B, 2, "Mack Fuller")] {
        installer
            .initialize(
                &CoreInitializationRequest {
                    schema: CORE_INIT_REQUEST_SCHEMA.to_owned(),
                    core_uuid: uuid.to_owned(),
                    core_number: number,
                    display_name: name.to_owned(),
                    requested_at: now_rfc3339().expect("time"),
                },
                &initialized.owner_token,
            )
            .expect("core");
    }
    (temp, root)
}

fn open_engine(root: &Path, core: &str) -> ConversationEngine {
    ConversationEngine::open(root, core).expect("engine")
}

fn episodes(engine: &ConversationEngine) -> Vec<aegis_core_sdk::MemoryEntry> {
    engine
        .sdk()
        .memory()
        .search(&MemoryQuery::new().with_kind(MemoryKind::Episode))
        .expect("episodes")
}

#[test]
fn consolidation_creates_one_memory_and_preserves_ownership() {
    let (_temp, root) = setup();
    let engine = open_engine(&root, CORE_A);
    let session = engine.create_session(Some("Important")).expect("session");
    let memory = engine.sdk().memory();

    let experiences = vec![ImportantExperience::new(
        "The owner entrusted a difficult story and asked for patience.",
    )];
    let outcome = engine
        .store()
        .consolidate_experiences(memory, &session.session_id, &experiences)
        .expect("consolidate");
    assert_eq!(
        outcome,
        ConsolidationOutcome {
            created: 1,
            skipped: 0,
            truncated: 0,
        }
    );

    let entries = episodes(&engine);
    assert_eq!(entries.len(), 1);
    let entry = &entries[0];
    assert_eq!(entry.core_uuid, CORE_A);
    assert_eq!(entry.kind, MemoryKind::Episode);
    assert_eq!(
        entry.content,
        "The owner entrusted a difficult story and asked for patience."
    );
    assert!(entry.source.starts_with(CONSOLIDATION_SOURCE_PREFIX));
    assert!(entry.source.contains(&session.session_id));
    assert_eq!(
        entry.provenance,
        format!("conversation.consolidation:{}", session.session_id)
    );
    assert!(entry.tags.contains(&"consolidation".to_owned()));
    assert!(entry
        .tags
        .contains(&format!("session:{}", session.session_id)));

    // A different Core's memory must not see the consolidated episode.
    let other = open_engine(&root, CORE_B);
    assert!(episodes(&other).is_empty());

    // Reopening the originating Core preserves the memory.
    let reopened = open_engine(&root, CORE_A);
    assert_eq!(episodes(&reopened).len(), 1);
}

#[test]
fn repeated_and_reordered_consolidation_is_idempotent() {
    let (_temp, root) = setup();
    let engine = open_engine(&root, CORE_A);
    let session = engine.create_session(None).expect("session");
    let memory = engine.sdk().memory();

    let first = ImportantExperience::new("The owner celebrated a small win.");
    let second = ImportantExperience::new("The owner asked to be reminded about rest.");
    let batch = vec![first.clone(), second.clone()];

    let initial = engine
        .store()
        .consolidate_experiences(memory, &session.session_id, &batch)
        .expect("initial");
    assert_eq!(initial.created, 2);
    assert_eq!(initial.skipped, 0);

    let repeated = engine
        .store()
        .consolidate_experiences(memory, &session.session_id, &batch)
        .expect("repeated");
    assert_eq!(
        repeated,
        ConsolidationOutcome {
            created: 0,
            skipped: 2,
            truncated: 0,
        }
    );

    // Reordering the batch must not defeat the stable source key.
    let reordered = vec![second, first];
    let outcome = engine
        .store()
        .consolidate_experiences(memory, &session.session_id, &reordered)
        .expect("reordered");
    assert_eq!(outcome.created, 0);
    assert_eq!(outcome.skipped, 2);

    // Identical entries inside one batch are also deduplicated.
    let duplicated = vec![
        ImportantExperience::new("A repeated experience summary."),
        ImportantExperience::new("A repeated experience summary."),
    ];
    let outcome = engine
        .store()
        .consolidate_experiences(memory, &session.session_id, &duplicated)
        .expect("duplicated");
    assert_eq!(outcome.created, 1);
    assert_eq!(outcome.skipped, 1);

    assert_eq!(episodes(&engine).len(), 3);
}

#[test]
fn archived_consolidation_still_counts_as_already_consolidated() {
    let (_temp, root) = setup();
    let engine = open_engine(&root, CORE_A);
    let session = engine.create_session(None).expect("session");
    let memory = engine.sdk().memory();
    let experience = ImportantExperience::new("The owner watched the first snow.");

    engine
        .store()
        .consolidate_experiences(
            memory,
            &session.session_id,
            std::slice::from_ref(&experience),
        )
        .expect("initial");
    let entry = episodes(&engine).remove(0);
    memory.archive(&entry.id).expect("archive");

    let outcome = engine
        .store()
        .consolidate_experiences(
            memory,
            &session.session_id,
            std::slice::from_ref(&experience),
        )
        .expect("repeat after archive");
    assert_eq!(outcome.created, 0);
    assert_eq!(outcome.skipped, 1);

    // The archived entry is still the single record for this source key.
    assert_eq!(
        memory
            .find_by_source(&entry.source)
            .expect("by source")
            .len(),
        1
    );
}

#[test]
fn consolidation_rejects_a_foreign_core_memory_store() {
    let (_temp, root) = setup();
    let engine_a = open_engine(&root, CORE_A);
    let session_a = engine_a.create_session(None).expect("session a");
    let memory_b = CoreMemoryStore::open(&root, CORE_B).expect("B memory");

    let error = engine_a
        .store()
        .consolidate_experiences(
            &memory_b,
            &session_a.session_id,
            &[ImportantExperience::new(
                "Should not cross Core boundaries.",
            )],
        )
        .expect_err("foreign store must be rejected");
    assert!(matches!(error, ConversationError::OwnershipMismatch));

    assert!(episodes(&engine_a).is_empty());
    let engine_b = open_engine(&root, CORE_B);
    assert!(episodes(&engine_b).is_empty());
}

#[test]
fn consolidation_is_bounded_and_skips_empty_entries() {
    let (_temp, root) = setup();
    let engine = open_engine(&root, CORE_A);
    let session = engine.create_session(None).expect("session");
    let memory = engine.sdk().memory();

    let oversized: Vec<ImportantExperience> = (0..=MAX_CONSOLIDATION_BATCH)
        .map(|index| ImportantExperience::new(format!("experience {index}")))
        .collect();
    let error = engine
        .store()
        .consolidate_experiences(memory, &session.session_id, &oversized)
        .expect_err("oversized batch must be rejected");
    assert!(matches!(error, ConversationError::InvalidInput(_)));
    assert!(episodes(&engine).is_empty());

    let empty = vec![
        ImportantExperience::new("   "),
        ImportantExperience::new("\n\t"),
        ImportantExperience::new(""),
    ];
    let outcome = engine
        .store()
        .consolidate_experiences(memory, &session.session_id, &empty)
        .expect("empty batch");
    assert_eq!(outcome, ConsolidationOutcome::default());
    assert!(episodes(&engine).is_empty());

    let long = "é".repeat(MAX_CONSOLIDATION_CONTENT_CHARS + 128);
    let outcome = engine
        .store()
        .consolidate_experiences(
            memory,
            &session.session_id,
            &[ImportantExperience::new(long)],
        )
        .expect("long experience");
    assert_eq!(
        outcome,
        ConsolidationOutcome {
            created: 1,
            skipped: 0,
            truncated: 1,
        }
    );
    let entries = episodes(&engine);
    assert_eq!(entries.len(), 1);
    assert_eq!(
        entries[0].content.chars().count(),
        MAX_CONSOLIDATION_CONTENT_CHARS
    );
    assert!(entries[0].content.chars().all(|character| character == 'é'));
}

#[test]
fn session_summary_is_metadata_only_and_bounded() {
    let (_temp, root) = setup();
    let engine = open_engine(&root, CORE_A);
    let session = engine.create_session(Some("Summary")).expect("session");
    let store = engine.store();
    store
        .append_message(
            &session.session_id,
            NewMessage::new(MessageRole::User, "user secret one"),
        )
        .expect("user");
    store
        .append_message(
            &session.session_id,
            NewMessage::new(MessageRole::Assistant, "assistant secret two"),
        )
        .expect("assistant");
    store
        .append_message(
            &session.session_id,
            NewMessage::new(MessageRole::Tool, "tool secret three"),
        )
        .expect("tool");

    let summary = store.session_summary(&session.session_id).expect("summary");
    assert_eq!(summary.schema, SESSION_SUMMARY_SCHEMA);
    assert_eq!(summary.core_uuid, CORE_A);
    assert_eq!(summary.session_id, session.session_id);
    assert_eq!(summary.state, SessionState::Active);
    assert_eq!(summary.message_count, 3);
    assert_eq!(summary.system_message_count, 0);
    assert_eq!(summary.user_message_count, 1);
    assert_eq!(summary.assistant_message_count, 1);
    assert_eq!(summary.tool_message_count, 1);
    assert!(summary.first_message_at.is_some());
    assert!(summary.last_message_at.is_some());
    assert!(summary.revision >= 1);

    // A summary must never carry transcript content.
    let encoded = serde_json::to_string(&summary).expect("serialize");
    for secret in [
        "user secret one",
        "assistant secret two",
        "tool secret three",
    ] {
        assert!(!encoded.contains(secret), "summary leaked {secret}");
    }

    // An empty session has zero counts and no message timestamps.
    let empty = engine.create_session(None).expect("empty session");
    let summary = store.session_summary(&empty.session_id).expect("summary");
    assert_eq!(summary.message_count, 0);
    assert_eq!(summary.user_message_count, 0);
    assert!(summary.first_message_at.is_none());
    assert!(summary.last_message_at.is_none());

    // Archived state is reflected without touching message history.
    store.archive_session(&session.session_id).expect("archive");
    let summary = store.session_summary(&session.session_id).expect("summary");
    assert_eq!(summary.state, SessionState::Archived);
    assert_eq!(summary.assistant_message_count, 1);
}

#[test]
fn session_summary_is_isolated_per_core() {
    let (_temp, root) = setup();
    let engine_a = open_engine(&root, CORE_A);
    let session_a = engine_a.create_session(None).expect("session a");
    let engine_b = open_engine(&root, CORE_B);

    let error = engine_b
        .store()
        .session_summary(&session_a.session_id)
        .expect_err("foreign session must not resolve");
    assert!(matches!(error, ConversationError::SessionNotFound(_)));
}
