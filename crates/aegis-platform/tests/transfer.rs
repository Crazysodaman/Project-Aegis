use std::path::{Path, PathBuf};

use aegis_contracts::core::CORE_IDENTITY_SCHEMA;
use aegis_contracts::initialization::FRESH_INITIALIZATION_MODE;
use aegis_contracts::living::{ItemCategory, TransferStatus, TransferType, Visibility};
use aegis_core_sdk::{NewInstance, PossessionItem, PossessionsStore, TRANSFER_IN_ACTION};
use aegis_platform::{PlatformService, TransferError, TransferStore};
use aegis_storage::{now_rfc3339, open_initialized, StoragePaths, PLATFORM_APPLICATION_ID};
use rusqlite::{params, Connection};
use serde_json::json;

const CORE_ONE: &str = "a9a0a604-a4e7-4a88-a985-3f071e2c8e7d";
const CORE_TWO: &str = "11111111-1111-4111-8111-111111111111";
const CORE_ONE_INSTANCE: &str = "22222222-2222-4222-8222-222222222222";
const CORE_TWO_INSTANCE: &str = "33333333-3333-4333-8333-333333333333";
const OTHER_TRANSFER: &str = "44444444-4444-4444-8444-444444444444";
const FOREIGN_INSTANCE: &str = "55555555-5555-4555-8555-555555555555";
const CORE_APPLICATION_ID: i64 = 0x4145_4743;

struct Fixture {
    _temp: tempfile::TempDir,
    root: PathBuf,
    owner_token: String,
}

fn fixture() -> Fixture {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path().to_path_buf();
    let initialized = PlatformService::initialize(&root).expect("platform initialize");
    write_core(&root, CORE_ONE, 1, CORE_ONE_INSTANCE);
    write_core(&root, CORE_TWO, 2, CORE_TWO_INSTANCE);
    Fixture {
        _temp: temp,
        root,
        owner_token: initialized.owner_token,
    }
}

fn write_core(root: &Path, core_uuid: &str, core_number: u8, database_instance: &str) {
    let core_dir = root.join("cores").join(core_uuid);
    std::fs::create_dir_all(&core_dir).expect("core dir");
    let connection = open_initialized(&core_dir.join("core.db"), CORE_APPLICATION_ID, "core")
        .expect("core database");
    connection
        .execute_batch(
            "CREATE TABLE IF NOT EXISTS core_identity (
                singleton INTEGER NOT NULL PRIMARY KEY CHECK (singleton = 1),
                schema TEXT NOT NULL,
                core_uuid TEXT NOT NULL,
                database_instance_uuid TEXT NOT NULL UNIQUE,
                core_number INTEGER NOT NULL CHECK (core_number BETWEEN 1 AND 255),
                display_name TEXT NOT NULL,
                created_at TEXT NOT NULL,
                identity_revision INTEGER NOT NULL CHECK (identity_revision >= 1),
                initialization_mode TEXT NOT NULL,
                schema_revision INTEGER NOT NULL CHECK (schema_revision >= 1)
            );",
        )
        .expect("identity schema");
    connection
        .execute(
            "INSERT INTO core_identity(
                singleton, schema, core_uuid, database_instance_uuid, core_number,
                display_name, created_at, identity_revision, initialization_mode,
                schema_revision
             ) VALUES(1, ?1, ?2, ?3, ?4, ?5, ?6, 1, ?7, 1)",
            params![
                CORE_IDENTITY_SCHEMA,
                core_uuid,
                database_instance,
                core_number,
                format!("Core {core_number}"),
                now_rfc3339().expect("timestamp"),
                FRESH_INITIALIZATION_MODE,
            ],
        )
        .expect("identity row");
}

fn transfer_store(root: &Path) -> TransferStore {
    let store = TransferStore::new(StoragePaths::new(root).expect("storage paths"));
    store.initialize().expect("initialize transfers");
    store
}

fn possessions(root: &Path, core_uuid: &str) -> PossessionsStore {
    PossessionsStore::open(root, core_uuid).expect("possessions store")
}

fn authored(store: &PossessionsStore, name: &str) -> PossessionItem {
    let mut new = NewInstance::new(name, ItemCategory::Tool);
    new.description = format!("{name} description");
    new.visibility = Visibility::Shared;
    new.properties = json!({"test": true});
    store.create_instance(&new).expect("item")
}

#[test]
fn offer_and_accept_move_item_between_real_cores() {
    let fixture = fixture();
    let transfers = transfer_store(&fixture.root);
    let sender = possessions(&fixture.root, CORE_ONE);
    let recipient = possessions(&fixture.root, CORE_TWO);
    let item = authored(&sender, "Brass key");

    let offered = transfers
        .offer(
            &fixture.owner_token,
            CORE_ONE,
            CORE_TWO,
            item.instance_id(),
            TransferType::Gift,
            "a gift",
        )
        .expect("offer");
    assert_eq!(offered.status, TransferStatus::Pending);
    assert_eq!(offered.from_core_uuid, CORE_ONE);
    assert_eq!(offered.to_core_uuid, CORE_TWO);
    assert_eq!(offered.note, "a gift");
    assert_eq!(
        transfers.get(&offered.transfer_id).expect("get"),
        Some(offered.clone())
    );
    assert_eq!(transfers.list(10).expect("list").len(), 1);

    let accepted = transfers
        .accept(&fixture.owner_token, &offered.transfer_id)
        .expect("accept");
    assert_eq!(accepted.status, TransferStatus::Accepted);
    let delivered_id = accepted
        .delivered_instance_id
        .clone()
        .expect("delivered instance");
    assert_ne!(delivered_id, item.instance_id());

    assert!(sender
        .get_instance(item.instance_id())
        .expect("sender get")
        .is_none());
    assert!(sender.list_inventory(10).expect("inventory").is_empty());

    let delivered = recipient
        .get_instance(&delivered_id)
        .expect("recipient get")
        .expect("delivered item");
    assert_eq!(delivered.owner_core_uuid(), CORE_TWO);
    assert_eq!(delivered.name, item.name);
    assert!(delivered
        .instance
        .provenance
        .iter()
        .any(|entry| { entry.action == TRANSFER_IN_ACTION && entry.note == offered.transfer_id }));

    let events = transfers.events(&offered.transfer_id, 10).expect("events");
    let kinds: Vec<&str> = events
        .iter()
        .map(|event| event.event_type.as_str())
        .collect();
    assert_eq!(kinds, ["offered", "accepted"]);
    assert_eq!(
        events.last().expect("accepted event").status,
        TransferStatus::Accepted
    );
}

#[test]
fn offer_rejects_foreign_missing_same_and_non_canonical_cores() {
    let fixture = fixture();
    let transfers = transfer_store(&fixture.root);
    let sender = possessions(&fixture.root, CORE_ONE);
    let recipient = possessions(&fixture.root, CORE_TWO);
    let own = authored(&sender, "Owned");
    let foreign = authored(&recipient, "Foreign");

    // A different Core's item is simply absent from the sender's database.
    assert!(matches!(
        transfers.offer(
            &fixture.owner_token,
            CORE_ONE,
            CORE_TWO,
            foreign.instance_id(),
            TransferType::Gift,
            "",
        ),
        Err(TransferError::MissingItem)
    ));
    assert!(matches!(
        transfers.offer(
            &fixture.owner_token,
            CORE_ONE,
            CORE_TWO,
            OTHER_TRANSFER,
            TransferType::Gift,
            "",
        ),
        Err(TransferError::MissingItem)
    ));

    // A row planted in the sender's database but owned by another Core is
    // rejected by the ownership proof in `export_for_transfer`.
    let sender_db = fixture
        .root
        .join("cores")
        .join(CORE_ONE)
        .join("possessions.db");
    let connection = Connection::open(sender_db).expect("sender possessions db");
    connection
        .execute(
            "INSERT INTO item_instances(
                instance_id, core_uuid, holder_core_uuid, definition_id, name, category,
                state, visibility, condition_percent, location, payload_json,
                created_at, updated_at, revision
             ) VALUES(?1, ?2, NULL, NULL, 'Foreign row', 'tool', 'stored',
                      'private', 100, NULL, '{}', ?3, ?3, 1)",
            params![
                FOREIGN_INSTANCE,
                CORE_TWO,
                now_rfc3339().expect("timestamp")
            ],
        )
        .expect("foreign row");
    drop(connection);
    assert!(matches!(
        transfers.offer(
            &fixture.owner_token,
            CORE_ONE,
            CORE_TWO,
            FOREIGN_INSTANCE,
            TransferType::Gift,
            "",
        ),
        Err(TransferError::ForeignItem)
    ));
    assert!(matches!(
        transfers.offer(
            &fixture.owner_token,
            CORE_ONE,
            CORE_ONE,
            own.instance_id(),
            TransferType::Gift,
            "",
        ),
        Err(TransferError::SameCore)
    ));
    assert!(matches!(
        transfers.offer(
            &fixture.owner_token,
            CORE_ONE,
            "not-a-canonical-uuid",
            own.instance_id(),
            TransferType::Gift,
            "",
        ),
        Err(TransferError::InvalidCoreUuid)
    ));
    assert!(transfers.list(10).expect("list").is_empty());
}

#[test]
fn accept_fails_after_sender_loses_the_item() {
    let fixture = fixture();
    let transfers = transfer_store(&fixture.root);
    let sender = possessions(&fixture.root, CORE_ONE);
    let recipient = possessions(&fixture.root, CORE_TWO);
    let item = authored(&sender, "Vanishing item");

    let offered = transfers
        .offer(
            &fixture.owner_token,
            CORE_ONE,
            CORE_TWO,
            item.instance_id(),
            TransferType::Loan,
            "",
        )
        .expect("offer");
    sender
        .remove_transferred_item(item.instance_id(), OTHER_TRANSFER)
        .expect("remove source");

    assert!(matches!(
        transfers.accept(&fixture.owner_token, &offered.transfer_id),
        Err(TransferError::MissingItem)
    ));
    let failed = transfers
        .get(&offered.transfer_id)
        .expect("get")
        .expect("record");
    assert_eq!(failed.status, TransferStatus::Failed);
    assert!(recipient.list_inventory(10).expect("inventory").is_empty());

    let events = transfers.events(&offered.transfer_id, 10).expect("events");
    let last = events.last().expect("failed event");
    assert_eq!(last.event_type, "failed");
    assert_eq!(last.detail["reason"], "item_missing");
}

#[test]
fn double_accept_is_rejected_without_duplicate_delivery() {
    let fixture = fixture();
    let transfers = transfer_store(&fixture.root);
    let sender = possessions(&fixture.root, CORE_ONE);
    let recipient = possessions(&fixture.root, CORE_TWO);
    let item = authored(&sender, "Single delivery");

    let offered = transfers
        .offer(
            &fixture.owner_token,
            CORE_ONE,
            CORE_TWO,
            item.instance_id(),
            TransferType::Trade,
            "",
        )
        .expect("offer");
    transfers
        .accept(&fixture.owner_token, &offered.transfer_id)
        .expect("first accept");

    assert!(matches!(
        transfers.accept(&fixture.owner_token, &offered.transfer_id),
        Err(TransferError::NotPending(status)) if status == "accepted"
    ));
    assert_eq!(recipient.list_inventory(10).expect("inventory").len(), 1);
}

#[test]
fn reject_and_cancel_are_terminal_without_delivery() {
    let fixture = fixture();
    let transfers = transfer_store(&fixture.root);
    let sender = possessions(&fixture.root, CORE_ONE);
    let recipient = possessions(&fixture.root, CORE_TWO);
    let rejected_item = authored(&sender, "Rejected");
    let cancelled_item = authored(&sender, "Cancelled");

    let rejected = transfers
        .offer(
            &fixture.owner_token,
            CORE_ONE,
            CORE_TWO,
            rejected_item.instance_id(),
            TransferType::Gift,
            "",
        )
        .expect("offer reject");
    let rejected = transfers
        .reject(&fixture.owner_token, &rejected.transfer_id)
        .expect("reject");
    assert_eq!(rejected.status, TransferStatus::Rejected);
    assert!(sender
        .get_instance(rejected_item.instance_id())
        .expect("sender get")
        .is_some());

    let cancelled = transfers
        .offer(
            &fixture.owner_token,
            CORE_ONE,
            CORE_TWO,
            cancelled_item.instance_id(),
            TransferType::Swap,
            "",
        )
        .expect("offer cancel");
    let cancelled = transfers
        .cancel(&fixture.owner_token, &cancelled.transfer_id)
        .expect("cancel");
    assert_eq!(cancelled.status, TransferStatus::Cancelled);

    assert!(recipient.list_inventory(10).expect("inventory").is_empty());
    assert!(matches!(
        transfers.reject(&fixture.owner_token, &rejected.transfer_id),
        Err(TransferError::NotPending(status)) if status == "rejected"
    ));
    let reject_events = transfers
        .events(&rejected.transfer_id, 10)
        .expect("reject events");
    let reject_kinds: Vec<&str> = reject_events
        .iter()
        .map(|event| event.event_type.as_str())
        .collect();
    assert_eq!(reject_kinds, ["offered", "rejected"]);
    let cancel_events = transfers
        .events(&cancelled.transfer_id, 10)
        .expect("cancel events");
    assert_eq!(cancel_events.len(), 2);
    assert_eq!(
        cancel_events.last().expect("cancel event").event_type,
        "cancelled"
    );
}

#[test]
fn every_mutation_requires_owner_proof() {
    let fixture = fixture();
    let transfers = transfer_store(&fixture.root);
    let sender = possessions(&fixture.root, CORE_ONE);
    let item = authored(&sender, "Guarded");
    let other = authored(&sender, "Guarded too");

    let offered = transfers
        .offer(
            &fixture.owner_token,
            CORE_ONE,
            CORE_TWO,
            item.instance_id(),
            TransferType::Gift,
            "",
        )
        .expect("offer");

    assert!(matches!(
        transfers.offer(
            "wrong-token",
            CORE_ONE,
            CORE_TWO,
            other.instance_id(),
            TransferType::Gift,
            "",
        ),
        Err(TransferError::Unauthorized)
    ));
    assert_eq!(transfers.list(10).expect("list").len(), 1);
    assert!(matches!(
        transfers.accept("wrong-token", &offered.transfer_id),
        Err(TransferError::Unauthorized)
    ));
    assert!(matches!(
        transfers.reject("wrong-token", &offered.transfer_id),
        Err(TransferError::Unauthorized)
    ));
    assert!(matches!(
        transfers.cancel("wrong-token", &offered.transfer_id),
        Err(TransferError::Unauthorized)
    ));
    assert_eq!(
        transfers
            .get(&offered.transfer_id)
            .expect("get")
            .expect("record")
            .status,
        TransferStatus::Pending
    );
}

#[test]
fn transfer_only_moves_the_named_item() {
    let fixture = fixture();
    let transfers = transfer_store(&fixture.root);
    let sender = possessions(&fixture.root, CORE_ONE);
    let recipient = possessions(&fixture.root, CORE_TWO);
    let moved = authored(&sender, "Moved");
    let kept = authored(&sender, "Kept");

    let offered = transfers
        .offer(
            &fixture.owner_token,
            CORE_ONE,
            CORE_TWO,
            moved.instance_id(),
            TransferType::Gift,
            "",
        )
        .expect("offer");
    transfers
        .accept(&fixture.owner_token, &offered.transfer_id)
        .expect("accept");

    let sender_items = sender.list_inventory(10).expect("sender inventory");
    assert_eq!(sender_items.len(), 1);
    assert_eq!(sender_items[0].instance_id(), kept.instance_id());
    let recipient_items = recipient.list_inventory(10).expect("recipient inventory");
    assert_eq!(recipient_items.len(), 1);
    assert_eq!(recipient_items[0].name, moved.name);
}

#[test]
fn transfer_event_ledger_is_append_only() {
    let fixture = fixture();
    let transfers = transfer_store(&fixture.root);
    let sender = possessions(&fixture.root, CORE_ONE);
    let item = authored(&sender, "Ledger");
    let offered = transfers
        .offer(
            &fixture.owner_token,
            CORE_ONE,
            CORE_TWO,
            item.instance_id(),
            TransferType::Gift,
            "",
        )
        .expect("offer");

    let paths = StoragePaths::new(&fixture.root).expect("paths");
    let connection = open_initialized(&paths.platform_db(), PLATFORM_APPLICATION_ID, "platform")
        .expect("platform db");

    let update = connection.execute(
        "UPDATE item_transfer_events SET event_type = 'tampered' WHERE transfer_id = ?1",
        [offered.transfer_id.as_str()],
    );
    assert!(update.is_err());
    let delete = connection.execute(
        "DELETE FROM item_transfer_events WHERE transfer_id = ?1",
        [offered.transfer_id.as_str()],
    );
    assert!(delete.is_err());
    assert_eq!(
        transfers
            .events(&offered.transfer_id, 10)
            .expect("events")
            .len(),
        1
    );
}

#[test]
fn transfers_persist_across_store_reopen() {
    let fixture = fixture();
    let sender = possessions(&fixture.root, CORE_ONE);
    let recipient = possessions(&fixture.root, CORE_TWO);
    let item = authored(&sender, "Persistent");

    let offered = {
        let transfers = transfer_store(&fixture.root);
        transfers
            .offer(
                &fixture.owner_token,
                CORE_ONE,
                CORE_TWO,
                item.instance_id(),
                TransferType::Return,
                "persisted",
            )
            .expect("offer")
    };

    {
        let transfers = transfer_store(&fixture.root);
        let pending = transfers
            .get(&offered.transfer_id)
            .expect("get")
            .expect("pending record");
        assert_eq!(pending.status, TransferStatus::Pending);
        let accepted = transfers
            .accept(&fixture.owner_token, &offered.transfer_id)
            .expect("accept after reopen");
        assert_eq!(accepted.status, TransferStatus::Accepted);
    }

    let transfers = transfer_store(&fixture.root);
    let persisted = transfers
        .get(&offered.transfer_id)
        .expect("get")
        .expect("accepted record");
    assert_eq!(persisted.status, TransferStatus::Accepted);
    assert!(persisted.delivered_instance_id.is_some());
    assert_eq!(
        transfers
            .events(&offered.transfer_id, 10)
            .expect("events")
            .len(),
        2
    );
    assert_eq!(recipient.list_inventory(10).expect("inventory").len(), 1);
    assert!(sender.list_inventory(10).expect("inventory").is_empty());
}
