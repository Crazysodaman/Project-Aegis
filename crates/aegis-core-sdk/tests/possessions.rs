use std::fs;
use std::path::{Path, PathBuf};

use aegis_contracts::initialization::{CoreInitializationRequest, CORE_INIT_REQUEST_SCHEMA};
use aegis_contracts::living::{
    DocumentFormat, ItemCategory, Visibility, MAX_DOCUMENT_BODY_CHARS, MAX_LIST_ITEMS,
    MAX_NAME_CHARS, MAX_TEXT_CHARS,
};
use aegis_core_install::CoreInstaller;
use aegis_core_sdk::{
    CoreSdkContext, InstanceUpdate, NewInstance, NewObject, NewOutfit, NewPossessionEvent,
    ObjectUpdate, PossessionsStore, SdkError, DEFAULT_ITEM_STATE, DEFAULT_OBJECT_STATE,
    DOCUMENTS_TABLE, ITEM_INSTANCES_TABLE, MAX_EVENT_DETAIL_BYTES, MAX_EVENT_LIST,
    MAX_POSSESSION_LIST, MAX_PROPERTIES_BYTES, OUTFITS_TABLE, POSSESSIONS_APPLICATION_ID,
    POSSESSIONS_DB_FILE, POSSESSIONS_OWNERSHIP_TABLE, POSSESSIONS_SCHEMA,
    POSSESSIONS_SCHEMA_REVISION, POSSESSIONS_SCHEMA_TABLE, POSSESSION_EVENTS_TABLE,
    TRANSFER_OFFER_ACTION, TRANSFER_OFFER_STATUS, WORLD_OBJECTS_TABLE,
};
use aegis_core_storage::{CoreDatabase, CoreStorageLayout};
use aegis_platform::PlatformService;
use aegis_storage::now_rfc3339;
use rusqlite::{params, Connection};
use serde_json::json;

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

fn possessions_path(root: &Path, core_uuid: &str) -> PathBuf {
    layout(root)
        .core_dir(core_uuid)
        .expect("core dir")
        .join(POSSESSIONS_DB_FILE)
}

fn table_count(connection: &Connection, name: &str) -> i64 {
    connection
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = ?1",
            [name],
            |row| row.get(0),
        )
        .expect("table count")
}

#[test]
fn possessions_store_binds_identity_and_creates_database() {
    let fixture = fixture();
    let snapshot =
        CoreDatabase::read_snapshot(&layout(&fixture.root), CORE_A).expect("core snapshot");
    let store = PossessionsStore::open(&fixture.root, CORE_A).expect("possessions store");

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
        Some(POSSESSIONS_DB_FILE)
    );
    assert!(store.database_path().is_file());
    let core_dir = layout(&fixture.root).core_dir(CORE_A).unwrap();
    assert_ne!(store.database_path(), core_dir.join("core.db"));
    assert_ne!(store.database_path(), core_dir.join("memory.db"));
    assert_ne!(store.database_path(), core_dir.join("personal.db"));
    assert_ne!(store.database_path(), fixture.root.join("platform.db"));
    assert_ne!(store.database_path(), fixture.root.join("safe.db"));

    let path = store.database_path().to_path_buf();
    drop(store);

    let connection = Connection::open(&path).expect("raw possessions db");
    let application_id: i64 = connection
        .query_row("PRAGMA application_id", [], |row| row.get(0))
        .expect("application id");
    assert_eq!(application_id, POSSESSIONS_APPLICATION_ID);
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
            "SELECT core_uuid, database_instance_uuid, schema FROM possessions_ownership",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .expect("ownership row");
    assert_eq!(ownership.0, CORE_A);
    assert_eq!(ownership.1, snapshot.metadata.database_instance_uuid);
    assert_eq!(ownership.2, POSSESSIONS_SCHEMA);
    let count: i64 = connection
        .query_row("SELECT COUNT(*) FROM possessions_ownership", [], |row| {
            row.get(0)
        })
        .expect("ownership count");
    assert_eq!(count, 1);

    let marker: (String, i64) = connection
        .query_row(
            "SELECT schema, revision FROM possessions_schema WHERE singleton = 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("schema marker");
    assert_eq!(marker.0, POSSESSIONS_SCHEMA);
    assert_eq!(
        marker.1,
        i64::try_from(POSSESSIONS_SCHEMA_REVISION).unwrap()
    );

    for table in [
        ITEM_INSTANCES_TABLE,
        OUTFITS_TABLE,
        DOCUMENTS_TABLE,
        WORLD_OBJECTS_TABLE,
        POSSESSION_EVENTS_TABLE,
        POSSESSIONS_OWNERSHIP_TABLE,
        POSSESSIONS_SCHEMA_TABLE,
    ] {
        assert_eq!(table_count(&connection, table), 1, "missing table {table}");
    }
}

#[test]
fn create_own_wear_separation_and_reopen_persistence() {
    let fixture = fixture();
    let item_id;
    let updated;
    {
        let store = PossessionsStore::open(&fixture.root, CORE_A).expect("store");
        let mut new = NewInstance::new("Linen dress", ItemCategory::Garment);
        new.description = "Hand-sewn summer dress".to_owned();
        new.properties = json!({"color": "ivory", "fabric": "linen"});
        let item = store.create_instance(&new).expect("create item");

        assert_eq!(item.owner_core_uuid(), CORE_A);
        assert_eq!(item.state(), DEFAULT_ITEM_STATE);
        assert_eq!(item.holder_core_uuid(), None);
        assert_eq!(item.visibility(), Visibility::Private);
        assert_eq!(item.name, "Linen dress");
        assert_eq!(item.description, "Hand-sewn summer dress");
        assert_eq!(
            item.properties,
            json!({"color": "ivory", "fabric": "linen"})
        );
        assert_eq!(item.instance.revision, 1);
        assert!(!item.instance.provenance.is_empty());
        assert_eq!(store.list_inventory(10).expect("inventory").len(), 1);

        let worn = store.set_state(item.instance_id(), "worn").expect("wear");
        assert_eq!(worn.state(), "worn");
        assert_eq!(worn.owner_core_uuid(), CORE_A);
        assert_eq!(worn.holder_core_uuid(), None);
        assert_eq!(worn.instance.revision, 2);

        let update = InstanceUpdate {
            name: "Linen day dress".to_owned(),
            description: "Altered hem".to_owned(),
            category: ItemCategory::Garment,
            visibility: Visibility::Shared,
            properties: json!({"color": "ivory", "fabric": "linen", "fitted": true}),
            condition_percent: 90,
            location: Some("atelier".to_owned()),
        };
        let item_id_value = item.instance_id().to_owned();
        updated = store
            .update_instance(&item_id_value, &update)
            .expect("update item");
        item_id = item_id_value;
        assert_eq!(updated.name, "Linen day dress");
        assert_eq!(updated.description, "Altered hem");
        assert_eq!(updated.state(), "worn");
        assert_eq!(updated.owner_core_uuid(), CORE_A);
        assert_eq!(updated.instance.condition_percent, 90);
        assert_eq!(updated.instance.location.as_deref(), Some("atelier"));
        assert_eq!(updated.instance.visibility, Visibility::Shared);
        assert_eq!(updated.instance.revision, 3);
    }

    let reopened = PossessionsStore::open(&fixture.root, CORE_A).expect("reopen");
    let reloaded = reopened
        .get_instance(&item_id)
        .expect("get item")
        .expect("item exists");
    assert_eq!(reloaded, updated);
    assert_eq!(reopened.list_inventory(10).expect("inventory").len(), 1);

    let connection = Connection::open(possessions_path(&fixture.root, CORE_A)).expect("raw db");
    let indexed: (String, String, String, String, i64) = connection
        .query_row(
            "SELECT core_uuid, state, category, visibility, condition_percent
             FROM item_instances WHERE instance_id = ?1",
            params![item_id],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                ))
            },
        )
        .expect("indexed item row");
    assert_eq!(indexed.0, CORE_A);
    assert_eq!(indexed.1, "worn");
    assert_eq!(indexed.2, "garment");
    assert_eq!(indexed.3, "shared");
    assert_eq!(indexed.4, 90);
}

#[test]
fn outfit_rejects_foreign_or_unknown_items_and_archives() {
    let fixture = fixture();
    let store = PossessionsStore::open(&fixture.root, CORE_A).expect("store");
    let item = store
        .create_instance(&NewInstance::new("Silk scarf", ItemCategory::Accessory))
        .expect("item");
    let outfit = store
        .create_outfit(&NewOutfit::new(
            "Day look",
            vec![item.instance_id().to_owned()],
        ))
        .expect("outfit");
    assert_eq!(outfit.core_uuid, CORE_A);
    assert_eq!(
        outfit.item_instance_ids,
        vec![item.instance_id().to_owned()]
    );
    assert_eq!(outfit.visibility, Visibility::Private);
    assert_eq!(store.list_outfits(10).expect("outfits").len(), 1);

    let updated = store
        .update_outfit(
            &outfit.outfit_id,
            "Evening look",
            &[item.instance_id().to_owned()],
            Visibility::Shared,
        )
        .expect("update outfit");
    assert_eq!(updated.name, "Evening look");
    assert_eq!(updated.revision, 2);
    assert_eq!(updated.visibility, Visibility::Shared);

    let unknown = store
        .create_outfit(&NewOutfit::new(
            "Unknown",
            vec!["22222222-2222-4222-8222-222222222222".to_owned()],
        ))
        .expect_err("unknown item must fail");
    assert!(matches!(unknown, SdkError::InvalidMemory(_)));
    drop(store);

    let connection = Connection::open(possessions_path(&fixture.root, CORE_A)).expect("raw db");
    let now = now_rfc3339().expect("timestamp");
    connection
        .execute(
            "INSERT INTO item_instances(
                instance_id, core_uuid, holder_core_uuid, definition_id, name, category,
                state, visibility, condition_percent, location, payload_json,
                created_at, updated_at, revision
             ) VALUES(?1, ?2, NULL, NULL, 'Foreign item', 'other', ?3, 'private',
                      100, NULL, '{}', ?4, ?4, 1)",
            params!["foreign-item", CORE_B, DEFAULT_ITEM_STATE, now],
        )
        .expect("foreign item row");
    drop(connection);

    let store = PossessionsStore::open(&fixture.root, CORE_A).expect("reopen");
    assert_eq!(
        store.list_outfits(10).expect("outfits after reopen").len(),
        1
    );
    let foreign = store
        .create_outfit(&NewOutfit::new("Foreign", vec!["foreign-item".to_owned()]))
        .expect_err("foreign item must fail");
    assert!(matches!(foreign, SdkError::ForeignCore));

    store.archive_outfit(&outfit.outfit_id).expect("archive");
    assert!(store.list_outfits(10).expect("outfits").is_empty());
    store
        .archive_outfit(&outfit.outfit_id)
        .expect("archive is idempotent");
    assert!(matches!(
        store.update_outfit(
            &outfit.outfit_id,
            "Again",
            &[item.instance_id().to_owned()],
            Visibility::Private
        ),
        Err(SdkError::NotFound)
    ));
    assert!(matches!(
        store.archive_outfit("33333333-3333-4333-8333-333333333333"),
        Err(SdkError::NotFound)
    ));
}

#[test]
fn document_body_persists_and_revision_increments() {
    let fixture = fixture();
    let body = "Chapter one.\nIt was a quiet morning.\n\tThe house slept.\n";
    let document_id;
    let updated;
    {
        let store = PossessionsStore::open(&fixture.root, CORE_A).expect("store");
        let document = store
            .create_document(
                "Morning notes",
                body,
                DocumentFormat::Markdown,
                Visibility::Private,
            )
            .expect("create document");
        document_id = document.document_id.clone();
        assert_eq!(document.body, body);
        assert_eq!(document.revision, 1);
        assert_eq!(document.owner_core_uuid, CORE_A);
        assert_eq!(document.visibility, Visibility::Private);
        assert_eq!(
            store
                .get_document(&document.document_id)
                .expect("get document")
                .expect("document exists"),
            document
        );

        let long_body = "A line of prose.\n".repeat(2_000);
        updated = store
            .update_document(
                &document.document_id,
                "Morning notes, revised",
                &long_body,
                DocumentFormat::PlainText,
                Visibility::Shared,
            )
            .expect("update document");
        assert_eq!(updated.body, long_body);
        assert_eq!(updated.revision, 2);
        assert_eq!(updated.format, DocumentFormat::PlainText);
        assert_eq!(updated.visibility, Visibility::Shared);
    }

    let reopened = PossessionsStore::open(&fixture.root, CORE_A).expect("reopen");
    assert_eq!(
        reopened
            .get_document(&document_id)
            .expect("get document")
            .expect("document exists"),
        updated
    );
    assert_eq!(reopened.list_documents(10).expect("documents").len(), 1);

    let too_long = "x".repeat(MAX_DOCUMENT_BODY_CHARS + 1);
    assert!(reopened
        .create_document(
            "Too long",
            &too_long,
            DocumentFormat::PlainText,
            Visibility::Private
        )
        .is_err());
    let too_long_title = "x".repeat(MAX_NAME_CHARS + 1);
    assert!(reopened
        .update_document(
            &document_id,
            &too_long_title,
            "body",
            DocumentFormat::Note,
            Visibility::Private
        )
        .is_err());
    assert!(matches!(
        reopened.update_document(
            "44444444-4444-4444-8444-444444444444",
            "Missing",
            "body",
            DocumentFormat::Note,
            Visibility::Private
        ),
        Err(SdkError::NotFound)
    ));
}

#[test]
fn world_object_state_round_trips_and_persists() {
    let fixture = fixture();
    let object_id;
    let updated;
    {
        let store = PossessionsStore::open(&fixture.root, CORE_A).expect("store");
        let mut new = NewObject::new("study");
        new.definition_id = Some("def-shelf".to_owned());
        new.state = "present".to_owned();
        new.affordances = vec!["read".to_owned(), "open".to_owned()];
        let object = store.create_object(&new).expect("create object");
        object_id = object.object_id.clone();
        assert_eq!(object.owner_core_uuid.as_deref(), Some(CORE_A));
        assert_eq!(object.holder_core_uuid.as_deref(), Some(CORE_A));
        assert_eq!(object.location, "study");
        assert_eq!(object.state, "present");
        assert_eq!(
            object.affordances,
            vec!["read".to_owned(), "open".to_owned()]
        );
        assert_eq!(object.visibility, Visibility::Private);
        assert_eq!(object.revision, 1);

        let change = ObjectUpdate {
            location: "garden".to_owned(),
            state: "moved".to_owned(),
            affordances: vec!["look".to_owned()],
            visibility: Visibility::Shared,
        };
        updated = store
            .update_object(&object.object_id, &change)
            .expect("update object");
        assert_eq!(updated.location, "garden");
        assert_eq!(updated.state, "moved");
        assert_eq!(updated.affordances, vec!["look".to_owned()]);
        assert_eq!(updated.revision, 2);
        assert_eq!(updated.visibility, Visibility::Shared);
    }

    let reopened = PossessionsStore::open(&fixture.root, CORE_A).expect("reopen");
    let objects = reopened.list_objects(10).expect("objects");
    assert_eq!(objects.len(), 1);
    assert_eq!(objects[0], updated);
    assert_eq!(objects[0].object_id, object_id);

    let mut empty_location = NewObject::new("");
    empty_location.state = DEFAULT_OBJECT_STATE.to_owned();
    assert!(reopened.create_object(&empty_location).is_err());
    let mut too_many = NewObject::new("room");
    too_many.affordances = vec!["look".to_owned(); MAX_LIST_ITEMS + 1];
    assert!(reopened.create_object(&too_many).is_err());
    let mut long_state = NewObject::new("room");
    long_state.state = "x".repeat(200);
    assert!(reopened.create_object(&long_state).is_err());
    assert!(matches!(
        reopened.update_object(
            "55555555-5555-4555-8555-555555555555",
            &ObjectUpdate {
                location: "nowhere".to_owned(),
                state: "missing".to_owned(),
                affordances: Vec::new(),
                visibility: Visibility::Private,
            }
        ),
        Err(SdkError::NotFound)
    ));
}

#[test]
fn possession_events_are_append_only_and_bounded() {
    let fixture = fixture();
    {
        let store = PossessionsStore::open(&fixture.root, CORE_A).expect("store");
        let first = store
            .record_event(&NewPossessionEvent {
                entity_kind: "note".to_owned(),
                entity_id: "event-1".to_owned(),
                action: "observed".to_owned(),
                detail: json!({"n": 1}),
                visibility: Visibility::Private,
            })
            .expect("event one");
        assert_eq!(first.sequence, 1);
        assert_eq!(first.core_uuid, CORE_A);
        assert_eq!(first.detail, json!({"n": 1}));

        let second = store
            .record_event(&NewPossessionEvent {
                entity_kind: "note".to_owned(),
                entity_id: "event-1".to_owned(),
                action: "observed_again".to_owned(),
                detail: json!({"n": 2}),
                visibility: Visibility::Shared,
            })
            .expect("event two");
        assert_eq!(second.sequence, 2);
        let events = store.list_events(10).expect("events");
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].sequence, 2);
        assert_eq!(events[1].sequence, 1);
        assert!(store.list_events(0).expect("zero limit").is_empty());
        assert!(store.list_events(usize::MAX).expect("bounded").len() <= MAX_EVENT_LIST);

        let too_long_action = store
            .record_event(&NewPossessionEvent {
                entity_kind: "note".to_owned(),
                entity_id: "event-1".to_owned(),
                action: "x".repeat(1_000),
                detail: json!({}),
                visibility: Visibility::Private,
            })
            .expect_err("action bound");
        assert!(matches!(too_long_action, SdkError::InvalidMemory(_)));
        let too_large_detail = store
            .record_event(&NewPossessionEvent {
                entity_kind: "note".to_owned(),
                entity_id: "event-1".to_owned(),
                action: "observed".to_owned(),
                detail: json!({"blob": "x".repeat(MAX_EVENT_DETAIL_BYTES)}),
                visibility: Visibility::Private,
            })
            .expect_err("detail bound");
        assert!(matches!(too_large_detail, SdkError::InvalidMemory(_)));
    }

    let connection = Connection::open(possessions_path(&fixture.root, CORE_A)).expect("raw db");
    for statement in [
        "UPDATE possession_events SET action = 'tampered'",
        "DELETE FROM possession_events",
        "UPDATE possessions_ownership SET core_uuid = 'tampered'",
        "DELETE FROM possessions_ownership",
    ] {
        assert!(
            connection.execute(statement, []).is_err(),
            "statement must be rejected: {statement}"
        );
    }
    let events: i64 = connection
        .query_row("SELECT COUNT(*) FROM possession_events", [], |row| {
            row.get(0)
        })
        .expect("event count");
    assert_eq!(events, 2);
    let ownership: i64 = connection
        .query_row("SELECT COUNT(*) FROM possessions_ownership", [], |row| {
            row.get(0)
        })
        .expect("ownership count");
    assert_eq!(ownership, 1);
}

#[test]
fn transfer_offer_records_status_without_delivery() {
    let fixture = fixture();
    let store_a = PossessionsStore::open(&fixture.root, CORE_A).expect("A store");
    let item = store_a
        .create_instance(&NewInstance::new("Brass key", ItemCategory::Tool))
        .expect("item");
    let offer = store_a
        .create_transfer_offer(item.instance_id(), CORE_B)
        .expect("offer");
    assert_eq!(offer.from_core_uuid, CORE_A);
    assert_eq!(offer.to_core_uuid, CORE_B);
    assert_eq!(offer.instance_id, item.instance_id());
    assert_eq!(offer.status, TRANSFER_OFFER_STATUS);
    assert!(!offer.offer_id.is_empty());

    let events = store_a.list_events(10).expect("events");
    let recorded = events
        .iter()
        .find(|event| event.action == TRANSFER_OFFER_ACTION)
        .expect("transfer offer event");
    assert_eq!(recorded.detail["status"], TRANSFER_OFFER_STATUS);
    assert_eq!(recorded.detail["to_core_uuid"], CORE_B);
    assert_eq!(
        recorded.detail["delivery"],
        "pending_phase5_transfer_subsystem"
    );

    let reloaded = store_a
        .get_instance(item.instance_id())
        .expect("get item")
        .expect("item exists");
    assert_eq!(reloaded.owner_core_uuid(), CORE_A);
    assert_eq!(reloaded.state(), DEFAULT_ITEM_STATE);

    let store_b = PossessionsStore::open(&fixture.root, CORE_B).expect("B store");
    assert!(store_b.list_inventory(10).expect("B inventory").is_empty());
    assert!(store_b
        .get_instance(item.instance_id())
        .expect("B get")
        .is_none());

    assert!(store_a
        .create_transfer_offer(item.instance_id(), CORE_A)
        .is_err());
    assert!(matches!(
        store_a.create_transfer_offer("66666666-6666-4666-8666-666666666666", CORE_B),
        Err(SdkError::NotFound)
    ));
}

#[test]
fn cross_core_database_copy_is_rejected() {
    let fixture = fixture();
    {
        let store = PossessionsStore::open(&fixture.root, CORE_A).expect("A store");
        store
            .create_instance(&NewInstance::new("Private notebook", ItemCategory::Book))
            .expect("item");
    }
    let a_path = possessions_path(&fixture.root, CORE_A);
    let b_path = possessions_path(&fixture.root, CORE_B);
    assert!(!b_path.exists());
    fs::copy(&a_path, &b_path).expect("copy A possessions over B");

    let copied = PossessionsStore::open(&fixture.root, CORE_B).expect_err("copied ownership");
    assert!(matches!(copied, SdkError::OwnershipMismatch));
    let reopened = PossessionsStore::open(&fixture.root, CORE_A).expect("A still opens");
    assert_eq!(reopened.list_inventory(10).expect("inventory").len(), 1);
}

#[test]
fn private_possessions_are_not_cross_core_readable() {
    let fixture = fixture();
    let store_a = PossessionsStore::open(&fixture.root, CORE_A).expect("A store");
    let item = store_a
        .create_instance(&NewInstance::new("Diary", ItemCategory::Book))
        .expect("item");
    let outfit = store_a
        .create_outfit(&NewOutfit::new(
            "Private look",
            vec![item.instance_id().to_owned()],
        ))
        .expect("outfit");
    let document = store_a
        .create_document(
            "Private letter",
            "For this Core only.",
            DocumentFormat::Letter,
            Visibility::Private,
        )
        .expect("document");
    let object = store_a
        .create_object(&NewObject::new("private room"))
        .expect("object");

    assert_eq!(item.visibility(), Visibility::Private);
    assert_eq!(outfit.visibility, Visibility::Private);
    assert_eq!(document.visibility, Visibility::Private);
    assert_eq!(object.visibility, Visibility::Private);

    let store_b = PossessionsStore::open(&fixture.root, CORE_B).expect("B store");
    assert!(store_b.list_inventory(10).expect("inventory").is_empty());
    assert!(store_b.list_outfits(10).expect("outfits").is_empty());
    assert!(store_b.list_documents(10).expect("documents").is_empty());
    assert!(store_b.list_objects(10).expect("objects").is_empty());
    assert!(store_b
        .get_instance(item.instance_id())
        .expect("get")
        .is_none());
    assert!(store_b
        .get_document(&document.document_id)
        .expect("get document")
        .is_none());
    assert!(matches!(
        store_b.set_state(item.instance_id(), "worn"),
        Err(SdkError::NotFound)
    ));
    assert!(matches!(
        store_b.update_instance(
            item.instance_id(),
            &InstanceUpdate {
                name: "Stolen".to_owned(),
                description: String::new(),
                category: ItemCategory::Book,
                visibility: Visibility::PublicCatalog,
                properties: json!({}),
                condition_percent: 50,
                location: None,
            }
        ),
        Err(SdkError::NotFound)
    ));
    assert!(matches!(
        store_b.update_document(
            &document.document_id,
            "Stolen",
            "Stolen",
            DocumentFormat::Note,
            Visibility::PublicCatalog
        ),
        Err(SdkError::NotFound)
    ));
    assert!(matches!(
        store_b.update_object(
            &object.object_id,
            &ObjectUpdate {
                location: "stolen".to_owned(),
                state: "stolen".to_owned(),
                affordances: Vec::new(),
                visibility: Visibility::PublicCatalog,
            }
        ),
        Err(SdkError::NotFound)
    ));
}

#[test]
fn input_bounds_and_limits_are_enforced() {
    let fixture = fixture();
    let store = PossessionsStore::open(&fixture.root, CORE_A).expect("store");

    let mut long_name = NewInstance::new("x".repeat(MAX_NAME_CHARS + 1), ItemCategory::Other);
    long_name.description = String::new();
    assert!(store.create_instance(&long_name).is_err());

    let mut long_description = NewInstance::new("Valid", ItemCategory::Other);
    long_description.description = "x".repeat(MAX_TEXT_CHARS + 1);
    assert!(store.create_instance(&long_description).is_err());

    let mut huge_properties = NewInstance::new("Valid", ItemCategory::Other);
    huge_properties.properties = json!({"blob": "x".repeat(MAX_PROPERTIES_BYTES)});
    assert!(store.create_instance(&huge_properties).is_err());

    let mut bad_definition = NewInstance::new("Valid", ItemCategory::Other);
    bad_definition.definition_id = Some("not a valid id".to_owned());
    assert!(store.create_instance(&bad_definition).is_err());

    let item = store
        .create_instance(&NewInstance::new("Valid", ItemCategory::Other))
        .expect("item");
    assert!(store
        .set_state(item.instance_id(), &"x".repeat(1_000))
        .is_err());

    assert!(store
        .create_outfit(&NewOutfit::new(
            "x".repeat(MAX_NAME_CHARS + 1),
            vec![item.instance_id().to_owned()]
        ))
        .is_err());
    assert!(store
        .create_outfit(&NewOutfit::new(
            "Duplicate",
            vec![item.instance_id().to_owned(), item.instance_id().to_owned()]
        ))
        .is_err());
    assert!(store
        .create_outfit(&NewOutfit::new(
            "Too many",
            vec!["item-0".to_owned(); MAX_LIST_ITEMS + 1]
        ))
        .is_err());

    for index in 0..3 {
        store
            .create_instance(&NewInstance::new(
                format!("Item {index}"),
                ItemCategory::Other,
            ))
            .expect("item");
    }
    assert!(store.list_inventory(0).expect("zero").is_empty());
    assert_eq!(store.list_inventory(2).expect("two").len(), 2);
    assert!(store.list_inventory(usize::MAX).expect("bounded").len() <= MAX_POSSESSION_LIST);

    assert!(matches!(
        PossessionsStore::open(&fixture.root, "not-a-core"),
        Err(SdkError::InvalidCoreUuid) | Err(SdkError::CoreStorage(_))
    ));
}

#[test]
fn core_sdk_context_exposes_possessions() {
    let fixture = fixture();
    let item_id;
    {
        let context = CoreSdkContext::open(&fixture.root, CORE_A).expect("context");
        assert_eq!(context.identity.core_uuid, CORE_A);
        let possessions = context.possessions().expect("possessions");
        assert_eq!(possessions.core_uuid(), CORE_A);
        let item = possessions
            .create_instance(&NewInstance::new("Context item", ItemCategory::Decoration))
            .expect("item");
        item_id = item.instance_id().to_owned();
    }

    let reopened = CoreSdkContext::open(&fixture.root, CORE_A).expect("reopen context");
    assert!(reopened
        .possessions()
        .expect("possessions")
        .get_instance(&item_id)
        .expect("get")
        .is_some());
}
