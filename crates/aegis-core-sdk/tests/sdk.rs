use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use aegis_contracts::initialization::{CoreInitializationRequest, CORE_INIT_REQUEST_SCHEMA};
use aegis_core_install::CoreInstaller;
use aegis_core_sdk::{
    CapabilityManifest, CoreCapability, CoreMemoryStore, CoreSdkContext, ImportMode, MemoryKind,
    MemoryQuery, NewMemory, PersonalityProfile, SdkError, CORE_SDK_CONTRACT,
    CORE_SDK_CONTRACT_SCHEMA, CORE_SDK_NAME, MAX_CONTENT_BYTES, MEMORY_APPLICATION_ID,
    MEMORY_CONTRACT_SCHEMA, MEMORY_DB_FILE,
};
use aegis_core_storage::CoreStorageLayout;
use aegis_platform::PlatformService;
use aegis_storage::now_rfc3339;
use rusqlite::Connection;
use serde_json::json;

const CORE_A: &str = "a9a0a604-a4e7-4a88-a985-3f071e2c8e7d";
const CORE_B: &str = "11111111-1111-4111-8111-111111111111";
const UNKNOWN_CORE: &str = "33333333-3333-4333-8333-333333333333";

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

fn memory_path(root: &Path, core_uuid: &str) -> PathBuf {
    layout(root)
        .core_dir(core_uuid)
        .expect("core dir")
        .join(MEMORY_DB_FILE)
}

#[test]
fn capability_manifest_validates_and_round_trips() {
    let manifest = CapabilityManifest::full(CORE_A).expect("manifest");
    manifest.validate().expect("valid manifest");
    assert_eq!(CoreCapability::all().len(), 23);
    assert!(manifest.has(CoreCapability::Memory));
    assert!(manifest.has(CoreCapability::LifecycleEvents));
    assert!(manifest.has(CoreCapability::ToolModel));
    assert_eq!(manifest.sdk_name, CORE_SDK_NAME);
    assert_eq!(
        CoreCapability::parse("growth_history"),
        Some(CoreCapability::GrowthHistory)
    );
    assert!(CapabilityManifest::required_capabilities().contains(&CoreCapability::Identity));

    let encoded = serde_json::to_string(&manifest).expect("encode");
    let decoded: CapabilityManifest = serde_json::from_str(&encoded).expect("decode");
    assert_eq!(decoded, manifest);

    let mut bad_schema = manifest.clone();
    bad_schema.schema = "aegis.core.capabilities.bad".to_owned();
    assert!(bad_schema.validate().is_err());

    let mut duplicate = manifest.clone();
    duplicate.capabilities.push(CoreCapability::Memory);
    assert!(duplicate.validate().is_err());

    let mut bad_uuid = manifest.clone();
    bad_uuid.core_uuid = "not-a-uuid".to_owned();
    assert!(bad_uuid.validate().is_err());

    let contract = &*CORE_SDK_CONTRACT;
    assert_eq!(contract.schema, CORE_SDK_CONTRACT_SCHEMA);
    assert_eq!(contract.memory_schema, MEMORY_CONTRACT_SCHEMA);
    assert_eq!(contract.capabilities.len(), 23);
}

#[test]
fn personality_round_trip_increments_revision() {
    let fixture = fixture();
    let store = CoreMemoryStore::open(&fixture.root, CORE_A).expect("store");
    assert!(store.load_personality().expect("load").is_none());

    let mut profile = PersonalityProfile::neutral(CORE_A).expect("neutral");
    profile.traits.insert("curiosity".to_owned(), 0.75);
    profile.tone = "warm and precise".to_owned();
    profile.values = vec!["honesty".to_owned()];

    let first = store.store_personality(&profile).expect("store first");
    assert_eq!(first.revision, 1);
    assert_eq!(first.core_uuid, CORE_A);
    let loaded = store
        .load_personality()
        .expect("load")
        .expect("profile present");
    assert_eq!(loaded, first);

    profile.tone = "calm".to_owned();
    let second = store.save_personality(&profile).expect("store second");
    assert_eq!(second.revision, 2);
    assert_eq!(
        store.get_personality().expect("get").expect("present"),
        second
    );
    let growth = store.personality_growth().expect("growth");
    assert_eq!(growth.len(), 2);
    assert_eq!(growth[0].revision, 1);
    assert_eq!(growth[1].revision, 2);
    assert_eq!(growth[1].tone, "calm");
}

#[test]
fn personality_rejects_permission_like_trait_keys() {
    let fixture = fixture();
    let store = CoreMemoryStore::open(&fixture.root, CORE_A).expect("store");
    for key in ["permission", "Authorization", "ROLE", "admin", "privilege"] {
        let mut profile = PersonalityProfile::neutral(CORE_A).expect("neutral");
        profile.traits.insert(key.to_owned(), 0.1);
        assert!(
            profile.validate().is_err(),
            "trait key {key} must be rejected"
        );
        assert!(
            store.store_personality(&profile).is_err(),
            "store must reject trait key {key}"
        );
    }

    let mut out_of_range = PersonalityProfile::neutral(CORE_A).expect("neutral");
    out_of_range.traits.insert("warmth".to_owned(), 1.5);
    assert!(out_of_range.validate().is_err());

    let mut non_finite = PersonalityProfile::neutral(CORE_A).expect("neutral");
    non_finite.traits.insert("warmth".to_owned(), f64::NAN);
    assert!(non_finite.validate().is_err());
}

#[test]
fn memory_create_get_and_search_by_text_kind_and_tag() {
    let fixture = fixture();
    let store = CoreMemoryStore::open(&fixture.root, CORE_A).expect("store");

    let fact = store
        .create(
            NewMemory::new(MemoryKind::Fact, "The sky is blue")
                .with_source("observation")
                .with_tags(vec!["Color".to_owned(), "nature".to_owned()]),
        )
        .expect("create fact");
    let preference = store
        .create(NewMemory::new(
            MemoryKind::Preference,
            "Likes quiet mornings",
        ))
        .expect("create preference");

    assert_eq!(store.get(&fact.id).expect("get"), Some(fact.clone()));
    assert!(store
        .get("00000000-0000-4000-8000-000000000000")
        .expect("get")
        .is_none());

    let by_text = store
        .search(&MemoryQuery::new().with_text("SKY"))
        .expect("search text");
    assert_eq!(by_text.len(), 1);
    assert_eq!(by_text[0].id, fact.id);

    let by_tag = store
        .search(&MemoryQuery::new().with_text("NATURE"))
        .expect("search tag");
    assert_eq!(by_tag.len(), 1);
    assert_eq!(by_tag[0].id, fact.id);

    let facts = store
        .search(&MemoryQuery::new().with_kind(MemoryKind::Fact))
        .expect("search kind");
    assert_eq!(facts.len(), 1);
    assert_eq!(facts[0].id, fact.id);

    let preferences = store
        .search(&MemoryQuery::new().with_kind(MemoryKind::Preference))
        .expect("search preference kind");
    assert_eq!(preferences.len(), 1);
    assert_eq!(preferences[0].id, preference.id);

    let limited = store
        .search(&MemoryQuery::new().with_limit(1))
        .expect("search limited");
    assert_eq!(limited.len(), 1);

    let offset = store
        .search(&MemoryQuery::new().with_limit(1).with_offset(1))
        .expect("search offset");
    assert_eq!(offset.len(), 1);
    assert_ne!(offset[0].id, limited[0].id);

    let missing = store
        .search(&MemoryQuery::new().with_text("absent"))
        .expect("search missing");
    assert!(missing.is_empty());
}

#[test]
fn memory_mutations_are_recorded_in_the_event_log() {
    let fixture = fixture();
    let store = CoreMemoryStore::open(&fixture.root, CORE_A).expect("store");
    let entry = store
        .create(NewMemory::new(MemoryKind::Episode, "First draft"))
        .expect("create");

    let updated = store
        .update_content(&entry.id, "Second draft")
        .expect("update content");
    assert_eq!(updated.content, "Second draft");
    assert_eq!(updated.revision, entry.revision + 1);

    let corrected = store
        .correct(&entry.id, "Corrected draft", "user", "fix typo")
        .expect("correct");
    assert_eq!(corrected.content, "Corrected draft");
    assert_eq!(corrected.source, "user");

    let tagged = store
        .set_tags(&entry.id, &["alpha".to_owned(), "beta".to_owned()])
        .expect("set tags");
    assert_eq!(tagged.tags, vec!["alpha".to_owned(), "beta".to_owned()]);

    let archived = store.archive(&entry.id).expect("archive");
    assert!(archived.archived);
    assert!(store
        .search(&MemoryQuery::new())
        .expect("default search")
        .is_empty());
    assert_eq!(
        store
            .search(&MemoryQuery::new().including_archived())
            .expect("archived search")
            .len(),
        1
    );

    let restored = store.restore_archived(&entry.id).expect("restore");
    assert!(!restored.archived);

    let deleted = store.soft_delete(&entry.id).expect("soft delete");
    assert!(deleted.deleted);
    assert!(store
        .search(&MemoryQuery::new())
        .expect("default search after delete")
        .is_empty());
    assert_eq!(
        store
            .search(&MemoryQuery::new().including_deleted())
            .expect("deleted search")
            .len(),
        1
    );

    store.purge(&entry.id).expect("purge");
    assert!(store.get(&entry.id).expect("get after purge").is_none());
    assert!(store
        .search(&MemoryQuery::new().including_deleted().including_archived())
        .expect("search after purge")
        .is_empty());

    let events = store
        .list_events(Some(&entry.id))
        .expect("events for entry");
    let actions: Vec<&str> = events.iter().map(|event| event.action.as_str()).collect();
    assert_eq!(
        actions,
        vec![
            "created",
            "content_updated",
            "corrected",
            "tags_set",
            "archived",
            "restored",
            "soft_deleted",
            "purged",
        ]
    );
    assert_eq!(events.len(), 8);
    assert_eq!(store.list_events(None).expect("all events").len(), 8);
}

#[test]
fn export_import_round_trip_supports_all_modes() {
    let fixture = fixture();
    let store = CoreMemoryStore::open(&fixture.root, CORE_A).expect("store");
    let first = store
        .create(NewMemory::new(MemoryKind::Fact, "One fact"))
        .expect("first");
    let second = store
        .create(NewMemory::new(MemoryKind::Goal, "One goal"))
        .expect("second");
    let export = store.export().expect("export");
    assert_eq!(export.core_uuid, CORE_A);
    assert_eq!(export.entries.len(), 2);
    assert!(!export.events.is_empty());

    let reject = store
        .import(&export, ImportMode::RejectExisting)
        .expect_err("reject existing");
    assert!(matches!(reject, SdkError::ImportConflict(_)));

    let skipped = store
        .import(&export, ImportMode::SkipExisting)
        .expect("skip existing");
    assert_eq!(skipped.skipped, 2);
    assert_eq!(skipped.inserted, 0);
    assert_eq!(skipped.replaced, 0);

    let replaced = store
        .import(&export, ImportMode::ReplaceExisting)
        .expect("replace existing");
    assert_eq!(replaced.replaced, 2);
    assert_eq!(replaced.inserted, 0);

    store.purge(&first.id).expect("purge first");
    store.purge(&second.id).expect("purge second");

    let inserted = store
        .import(&export, ImportMode::RejectExisting)
        .expect("insert fresh");
    assert_eq!(inserted.inserted, 2);
    assert_eq!(inserted.skipped, 0);
    let entries = store
        .search(&MemoryQuery::new())
        .expect("search after import");
    assert_eq!(entries.len(), 2);
    assert!(entries.iter().any(|entry| entry.id == first.id));
    assert!(entries.iter().any(|entry| entry.id == second.id));

    let other = CoreMemoryStore::open(&fixture.root, CORE_B).expect("other store");
    assert!(matches!(
        other
            .import(&export, ImportMode::RejectExisting)
            .expect_err("foreign core import"),
        SdkError::ForeignCore
    ));
}

#[test]
fn import_rejects_bad_schema_and_oversized_content() {
    let fixture = fixture();
    let store = CoreMemoryStore::open(&fixture.root, CORE_A).expect("store");
    store
        .create(NewMemory::new(MemoryKind::Fact, "exported fact"))
        .expect("create");
    let export = store.export().expect("export");

    let mut bad_schema = export.clone();
    bad_schema.schema = "aegis.core.memory.export.bad".to_owned();
    assert!(matches!(
        store
            .import(&bad_schema, ImportMode::RejectExisting)
            .expect_err("bad schema"),
        SdkError::UnsupportedImportSchema
    ));

    let mut foreign = export.clone();
    foreign.core_uuid = CORE_B.to_owned();
    assert!(matches!(
        store
            .import(&foreign, ImportMode::RejectExisting)
            .expect_err("foreign export"),
        SdkError::ForeignCore
    ));

    let mut oversized = export.clone();
    if let Some(entry) = oversized.entries.first_mut() {
        entry.content = "x".repeat(1024 * 1024 + 1);
    }
    assert!(matches!(
        store
            .import(&oversized, ImportMode::RejectExisting)
            .expect_err("oversized import"),
        SdkError::InvalidImport(_)
    ));
}

#[test]
fn cross_core_entries_are_isolated() {
    let fixture = fixture();
    let store_a = CoreMemoryStore::open(&fixture.root, CORE_A).expect("A store");
    let store_b = CoreMemoryStore::open(&fixture.root, CORE_B).expect("B store");

    let entry = store_a
        .create(NewMemory::new(MemoryKind::Fact, "Only for A"))
        .expect("create A");

    assert!(store_b.get(&entry.id).expect("B get").is_none());
    assert!(store_b
        .search(&MemoryQuery::new().with_text("Only for A"))
        .expect("B search")
        .is_empty());
    assert!(store_b.export().expect("B export").entries.is_empty());
    assert!(store_b.list_events(None).expect("B events").is_empty());

    let b_path = store_b.database_path().to_path_buf();
    drop(store_b);
    drop(store_a);

    let a_path = memory_path(&fixture.root, CORE_A);
    fs::copy(&a_path, &b_path).expect("copy A memory over B");
    let error = CoreMemoryStore::open(&fixture.root, CORE_B).expect_err("foreign ownership");
    assert!(matches!(error, SdkError::OwnershipMismatch));
}

#[test]
fn triggers_block_event_and_growth_mutation() {
    let fixture = fixture();
    let memory = memory_path(&fixture.root, CORE_A);
    {
        let store = CoreMemoryStore::open(&fixture.root, CORE_A).expect("store");
        store
            .create(NewMemory::new(MemoryKind::Fact, "event source"))
            .expect("create");
        let profile = PersonalityProfile::neutral(CORE_A).expect("neutral");
        store
            .store_personality(&profile)
            .expect("store personality");
    }

    let connection = Connection::open(&memory).expect("raw memory db");
    let update_event = connection.execute("UPDATE memory_events SET action = 'tampered'", []);
    assert!(
        update_event.is_err(),
        "memory_events UPDATE must be blocked"
    );
    let delete_event = connection.execute("DELETE FROM memory_events", []);
    assert!(
        delete_event.is_err(),
        "memory_events DELETE must be blocked"
    );
    let update_growth = connection.execute("UPDATE personality_growth SET revision = 99", []);
    assert!(
        update_growth.is_err(),
        "personality_growth UPDATE must be blocked"
    );
    let delete_growth = connection.execute("DELETE FROM personality_growth", []);
    assert!(
        delete_growth.is_err(),
        "personality_growth DELETE must be blocked"
    );
    let update_ownership = connection.execute(
        "UPDATE core_memory_ownership SET core_uuid = 'tampered'",
        [],
    );
    assert!(
        update_ownership.is_err(),
        "core_memory_ownership UPDATE must be blocked"
    );
    let delete_ownership = connection.execute("DELETE FROM core_memory_ownership", []);
    assert!(
        delete_ownership.is_err(),
        "core_memory_ownership DELETE must be blocked"
    );

    let events: i64 = connection
        .query_row("SELECT COUNT(*) FROM memory_events", [], |row| row.get(0))
        .expect("count events");
    assert_eq!(events, 1);
    let growth: i64 = connection
        .query_row("SELECT COUNT(*) FROM personality_growth", [], |row| {
            row.get(0)
        })
        .expect("count growth");
    assert_eq!(growth, 1);
}

#[test]
fn oversized_content_is_rejected_on_create_update_and_correct() {
    let fixture = fixture();
    let store = CoreMemoryStore::open(&fixture.root, CORE_A).expect("store");
    let oversized = "a".repeat(MAX_CONTENT_BYTES + 1);
    assert!(matches!(
        store
            .create(NewMemory::new(MemoryKind::Fact, oversized.clone()))
            .expect_err("oversized create"),
        SdkError::InvalidMemory(_)
    ));

    let entry = store
        .create(NewMemory::new(MemoryKind::Fact, "small"))
        .expect("create");
    assert!(store.update_content(&entry.id, &oversized).is_err());
    assert!(store
        .correct(&entry.id, &oversized, "source", "reason")
        .is_err());
}

#[test]
fn unknown_and_invalid_core_ids_are_rejected() {
    let fixture = fixture();
    assert!(CoreMemoryStore::open(&fixture.root, UNKNOWN_CORE).is_err());
    assert!(CoreMemoryStore::open(&fixture.root, "not-a-uuid").is_err());
}

#[test]
fn memory_paths_stay_inside_each_core_and_away_from_platform_and_safe() {
    let fixture = fixture();
    let store_a = CoreMemoryStore::open(&fixture.root, CORE_A).expect("A");
    let store_b = CoreMemoryStore::open(&fixture.root, CORE_B).expect("B");
    let layout = layout(&fixture.root);

    assert_ne!(store_a.database_path(), store_b.database_path());
    assert_eq!(
        store_a
            .database_path()
            .file_name()
            .and_then(|name| name.to_str()),
        Some(MEMORY_DB_FILE)
    );
    assert_eq!(
        store_a.database_path().parent(),
        Some(layout.core_dir(CORE_A).expect("A dir").as_path())
    );
    assert_eq!(
        store_b.database_path().parent(),
        Some(layout.core_dir(CORE_B).expect("B dir").as_path())
    );
    assert!(store_a.database_path().is_file());
    assert!(store_b.database_path().is_file());
    assert_ne!(store_a.database_path(), fixture.root.join("platform.db"));
    assert_ne!(store_a.database_path(), fixture.root.join("safe.db"));
    assert_ne!(store_b.database_path(), fixture.root.join("platform.db"));
    assert_ne!(store_b.database_path(), fixture.root.join("safe.db"));
    assert!(store_a
        .database_path()
        .starts_with(layout.cores_root().join(CORE_A)));

    let connection = Connection::open(store_a.database_path()).expect("open memory");
    let application_id: i64 = connection
        .query_row("PRAGMA application_id", [], |row| row.get(0))
        .expect("application id");
    assert_eq!(application_id, MEMORY_APPLICATION_ID);
    let journal_mode: String = connection
        .query_row("PRAGMA journal_mode", [], |row| row.get(0))
        .expect("journal mode");
    assert_eq!(journal_mode.to_lowercase(), "delete");
    let synchronous: i64 = connection
        .query_row("PRAGMA synchronous", [], |row| row.get(0))
        .expect("synchronous");
    assert_eq!(synchronous, 2);
    drop(connection);
}

#[test]
fn sdk_context_combines_identity_capabilities_memory_and_personality() {
    let fixture = fixture();
    let mut context = CoreSdkContext::open(&fixture.root, CORE_A).expect("context");
    assert_eq!(context.identity.core_uuid, CORE_A);
    context.capabilities.validate().expect("capabilities");
    assert!(context.personality().is_none());

    let entry = context
        .memory()
        .create(NewMemory::new(MemoryKind::Reflection, "Context reflection"))
        .expect("create");
    assert_eq!(
        context
            .memory()
            .get(&entry.id)
            .expect("get")
            .expect("present")
            .id,
        entry.id
    );

    let profile = PersonalityProfile::neutral(CORE_A).expect("neutral");
    let stored = context
        .memory()
        .store_personality(&profile)
        .expect("store personality");
    let refreshed = context
        .refresh_personality()
        .expect("refresh")
        .expect("personality");
    assert_eq!(refreshed.revision, stored.revision);

    let reopened = CoreSdkContext::open(&fixture.root, CORE_A).expect("reopen");
    assert_eq!(reopened.personality().expect("personality").revision, 1);
}

#[test]
fn memory_kind_serializes_to_stable_strings() {
    let encoded = serde_json::to_value(MemoryKind::Fact).expect("encode");
    assert_eq!(encoded, json!("fact"));
    let custom = MemoryKind::Other("weather_log".to_owned());
    let encoded = serde_json::to_value(&custom).expect("encode custom");
    assert_eq!(encoded, json!("weather_log"));
    let decoded: MemoryKind = serde_json::from_value(json!("weather_log")).expect("decode");
    assert_eq!(decoded, custom);
    assert!(MemoryKind::parse("bad kind!").is_err());

    let mut traits = BTreeMap::new();
    traits.insert("warmth".to_owned(), 0.5);
    let profile = PersonalityProfile {
        schema: "aegis.core.personality.v1".to_owned(),
        core_uuid: CORE_A.to_owned(),
        traits,
        tone: "calm".to_owned(),
        values: Vec::new(),
        growth_policy: "append_only".to_owned(),
        updated_at: now_rfc3339().expect("timestamp"),
        revision: 1,
    };
    profile.validate().expect("valid profile");
}
