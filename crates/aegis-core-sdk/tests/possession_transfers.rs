use std::path::{Path, PathBuf};

use aegis_contracts::initialization::{CoreInitializationRequest, CORE_INIT_REQUEST_SCHEMA};
use aegis_contracts::living::{ItemCategory, Visibility};
use aegis_core_install::CoreInstaller;
use aegis_core_sdk::{
    NewInstance, PossessionsStore, SdkError, POSSESSIONS_DB_FILE, TRANSFER_IN_ACTION,
    TRANSFER_OUT_ACTION, TRANSFER_ROLLBACK_ACTION,
};
use aegis_core_storage::CoreStorageLayout;
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

fn authored_item(store: &PossessionsStore) -> aegis_core_sdk::PossessionItem {
    let mut new = NewInstance::new("Brass key", ItemCategory::Tool);
    new.definition_id = Some("definition-brass-key".to_owned());
    new.description = "A small engraved key".to_owned();
    new.visibility = Visibility::Shared;
    new.properties = json!({"metal": "brass", "engraving": "A"});
    store.create_instance(&new).expect("item")
}

#[test]
fn export_import_remove_round_trip_preserves_provenance() {
    let fixture = fixture();
    let store_a = PossessionsStore::open(&fixture.root, CORE_A).expect("A store");
    let store_b = PossessionsStore::open(&fixture.root, CORE_B).expect("B store");
    let source = authored_item(&store_a);

    let exported = store_a
        .export_for_transfer(source.instance_id())
        .expect("export");
    assert_eq!(exported, source);
    assert_eq!(exported.owner_core_uuid(), CORE_A);
    assert!(!exported.instance.provenance.is_empty());

    let transfer_id = "transfer-round-trip";
    let imported = store_b
        .import_transferred_item(&exported, CORE_A, transfer_id)
        .expect("import");
    assert_ne!(imported.instance_id(), exported.instance_id());
    assert_eq!(imported.owner_core_uuid(), CORE_B);
    assert_eq!(imported.holder_core_uuid(), None);
    assert_eq!(imported.state(), "stored");
    assert_eq!(imported.name, exported.name);
    assert_eq!(imported.description, exported.description);
    assert_eq!(imported.category, exported.category);
    assert_eq!(imported.properties, exported.properties);
    assert_eq!(
        imported.instance.definition_id,
        exported.instance.definition_id
    );
    let transfer_entry = imported
        .instance
        .provenance
        .iter()
        .find(|entry| entry.action == TRANSFER_IN_ACTION)
        .expect("transferred_in provenance");
    assert_eq!(transfer_entry.note, transfer_id);
    assert_eq!(transfer_entry.actor, CORE_A);
    assert!(imported
        .instance
        .provenance
        .iter()
        .any(|entry| entry.action == "created"));
    assert_eq!(
        store_b
            .get_instance(imported.instance_id())
            .expect("get imported")
            .expect("imported exists"),
        imported
    );
    assert_eq!(store_b.list_inventory(10).expect("inventory").len(), 1);
    assert!(store_b
        .list_events(10)
        .expect("B events")
        .iter()
        .any(|event| event.action == TRANSFER_IN_ACTION
            && event.detail["transfer_id"] == transfer_id));

    store_a
        .remove_transferred_item(exported.instance_id(), transfer_id)
        .expect("remove source");
    assert!(store_a
        .get_instance(exported.instance_id())
        .expect("get source")
        .is_none());
    let source_events = store_a.list_events(10).expect("A events");
    assert!(source_events
        .iter()
        .any(|event| event.action == TRANSFER_OUT_ACTION
            && event.detail["transfer_id"] == transfer_id));

    store_b
        .remove_transferred_item(imported.instance_id(), transfer_id)
        .expect("remove imported");
    assert!(store_b
        .get_instance(imported.instance_id())
        .expect("get imported")
        .is_none());
    assert!(store_b.list_inventory(10).expect("inventory").is_empty());
    assert!(store_b
        .list_events(10)
        .expect("B events")
        .iter()
        .any(|event| event.action == TRANSFER_OUT_ACTION
            && event.detail["transfer_id"] == transfer_id));
}

#[test]
fn import_rejects_mismatched_source_and_invalid_transfer() {
    let fixture = fixture();
    let store_a = PossessionsStore::open(&fixture.root, CORE_A).expect("A store");
    let store_b = PossessionsStore::open(&fixture.root, CORE_B).expect("B store");
    let source = authored_item(&store_a);
    let exported = store_a
        .export_for_transfer(source.instance_id())
        .expect("export");

    assert!(matches!(
        store_b.import_transferred_item(&exported, CORE_B, "transfer-mismatch"),
        Err(SdkError::OwnershipMismatch)
    ));
    assert!(matches!(
        store_b.import_transferred_item(&exported, CORE_A, "not a valid id"),
        Err(SdkError::InvalidMemory(_))
    ));

    let own = authored_item(&store_b);
    assert!(matches!(
        store_b.import_transferred_item(&own, CORE_B, "transfer-self"),
        Err(SdkError::InvalidMemory(_))
    ));
    assert!(matches!(
        store_a.export_for_transfer("66666666-6666-4666-8666-666666666666"),
        Err(SdkError::NotFound)
    ));
}

#[test]
fn foreign_rows_are_rejected_by_all_transfer_operations() {
    let fixture = fixture();
    let store_a = PossessionsStore::open(&fixture.root, CORE_A).expect("A store");
    let source = authored_item(&store_a);
    let exported = store_a
        .export_for_transfer(source.instance_id())
        .expect("export");
    drop(store_a);
    drop(PossessionsStore::open(&fixture.root, CORE_B).expect("initialize B"));

    let payload = serde_json::to_string(&exported).expect("payload");
    let connection = Connection::open(possessions_path(&fixture.root, CORE_B)).expect("raw B db");
    connection
        .execute(
            "INSERT INTO item_instances(
                instance_id, core_uuid, holder_core_uuid, definition_id, name, category,
                state, visibility, condition_percent, location, payload_json,
                created_at, updated_at, revision
             ) VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
            params![
                exported.instance.instance_id,
                CORE_A,
                exported.instance.holder_core_uuid,
                exported.instance.definition_id,
                exported.name,
                exported.category.as_str(),
                exported.instance.state,
                exported.instance.visibility.as_str(),
                i64::from(exported.instance.condition_percent),
                exported.instance.location,
                payload,
                exported.instance.created_at,
                exported.updated_at,
                i64::try_from(exported.instance.revision).expect("revision"),
            ],
        )
        .expect("foreign row");
    drop(connection);

    let store_b = PossessionsStore::open(&fixture.root, CORE_B).expect("B store");
    assert!(matches!(
        store_b.export_for_transfer(exported.instance_id()),
        Err(SdkError::OwnershipMismatch)
    ));
    assert!(matches!(
        store_b.remove_transferred_item(exported.instance_id(), "transfer-foreign"),
        Err(SdkError::OwnershipMismatch)
    ));
    assert!(matches!(
        store_b.rollback_imported_item(exported.instance_id(), "transfer-foreign"),
        Err(SdkError::OwnershipMismatch)
    ));
    assert!(matches!(
        store_b.remove_transferred_item("77777777-7777-4777-8777-777777777777", "transfer-none"),
        Err(SdkError::NotFound)
    ));
}

#[test]
fn rollback_removes_imported_item_and_records_event() {
    let fixture = fixture();
    let store_a = PossessionsStore::open(&fixture.root, CORE_A).expect("A store");
    let store_b = PossessionsStore::open(&fixture.root, CORE_B).expect("B store");
    let source = authored_item(&store_a);
    let exported = store_a
        .export_for_transfer(source.instance_id())
        .expect("export");
    let transfer_id = "transfer-rollback";
    let imported = store_b
        .import_transferred_item(&exported, CORE_A, transfer_id)
        .expect("import");

    assert!(matches!(
        store_b.rollback_imported_item(imported.instance_id(), "different-transfer"),
        Err(SdkError::InvalidMemory(_))
    ));
    store_b
        .rollback_imported_item(imported.instance_id(), transfer_id)
        .expect("rollback");
    assert!(store_b
        .get_instance(imported.instance_id())
        .expect("get")
        .is_none());
    assert!(store_b.list_inventory(10).expect("inventory").is_empty());

    let events = store_b.list_events(10).expect("events");
    let rolled_back = events
        .iter()
        .find(|event| event.action == TRANSFER_ROLLBACK_ACTION)
        .expect("rollback event");
    assert_eq!(rolled_back.detail["transfer_id"], transfer_id);
    assert!(matches!(
        store_b.rollback_imported_item(imported.instance_id(), transfer_id),
        Err(SdkError::NotFound)
    ));
}
