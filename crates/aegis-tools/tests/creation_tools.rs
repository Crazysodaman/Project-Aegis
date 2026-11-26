//! Acceptance tests for Core-facing creation tools.
//!
//! These exercise the real `ToolExecutor` (registered tools, grants, STOP,
//! audit) against real per-Core SQLite storage, not direct store calls.

use serde_json::{json, Value};
use tempfile::TempDir;

use aegis_contracts::initialization::{CoreInitializationRequest, CORE_INIT_REQUEST_SCHEMA};
use aegis_contracts::living::{ItemCategory, ItemDefinition, Visibility, LIVING_CONTRACT_VERSION};
use aegis_core_install::CoreInstaller;
use aegis_platform::{CatalogStore, PlatformService};
use aegis_storage::{now_rfc3339, StoragePaths};
use aegis_tools::{ToolExecutor, ToolStatus};

const SOFIA: &str = "a9a0a604-a4e7-4a88-a985-3f071e2c8e7d";
const MACK: &str = "11111111-1111-4111-8111-111111111111";
const DEF_ID: &str = "def-summer-dress";

fn request(core_uuid: &str, number: u8, name: &str) -> CoreInitializationRequest {
    CoreInitializationRequest {
        schema: CORE_INIT_REQUEST_SCHEMA.to_owned(),
        core_uuid: core_uuid.to_owned(),
        core_number: number,
        display_name: name.to_owned(),
        requested_at: now_rfc3339().expect("timestamp"),
    }
}

fn setup() -> (TempDir, String) {
    let temp = tempfile::tempdir().expect("tempdir");
    let initialized = PlatformService::initialize(temp.path()).expect("platform");
    let installer = CoreInstaller::new(temp.path()).expect("installer");
    installer
        .initialize_approved(
            &request(SOFIA, 1, "Sofía Ada Lyra"),
            &initialized.owner_token,
        )
        .expect("install Sofía");
    let catalog = CatalogStore::new(StoragePaths::new(temp.path()).expect("paths"));
    catalog.initialize().expect("catalog init");
    let definition = ItemDefinition {
        schema: LIVING_CONTRACT_VERSION.to_owned(),
        definition_id: DEF_ID.to_owned(),
        creator_core_uuid: Some(SOFIA.to_owned()),
        name: "Summer Dress".to_owned(),
        description: "A light summer dress".to_owned(),
        category: ItemCategory::Garment,
        visibility: Visibility::PublicCatalog,
        tags: vec!["summer".to_owned(), "light".to_owned()],
        sizes: vec!["s".to_owned(), "m".to_owned(), "l".to_owned()],
        materials: vec!["linen".to_owned()],
        properties: json!({"color": "teal"}),
        asset_id: None,
        created_at: now_rfc3339().expect("time"),
        updated_at: now_rfc3339().expect("time"),
        revision: 1,
    };
    catalog
        .create(&initialized.owner_token, definition)
        .expect("catalog definition");
    (temp, initialized.owner_token)
}

fn exec(executor: &ToolExecutor, tool: &str, arguments: Value) -> Value {
    let result = executor.execute(tool, &arguments, None).expect("execute");
    assert_eq!(
        result.status,
        ToolStatus::Executed,
        "{tool} failed: {:?}",
        result.detail
    );
    result.output
}

#[test]
fn core_creation_workflow_persists_across_restart() {
    let (temp, _token) = setup();
    let executor = ToolExecutor::new(temp.path(), SOFIA).expect("executor");

    // 1. Core-authored clothing item.
    let item = exec(
        &executor,
        "item.create",
        json!({"name":"Blue dress","category":"garment","description":"A test dress"}),
    );
    let item_id = item["instance"]["instance_id"]
        .as_str()
        .expect("item id")
        .to_owned();
    assert_eq!(item["instance"]["owner_core_uuid"].as_str(), Some(SOFIA));

    // 2. Shared catalog search + add to personal inventory.
    let found = exec(&executor, "catalog.search", json!({"text":"summer"}));
    assert!(found
        .as_array()
        .expect("array")
        .iter()
        .any(|entry| { entry["definition_id"].as_str() == Some(DEF_ID) }));
    let catalog_item = exec(
        &executor,
        "catalog.add_to_inventory",
        json!({"definition_id": DEF_ID}),
    );
    let catalog_item_id = catalog_item["instance"]["instance_id"]
        .as_str()
        .expect("catalog item id")
        .to_owned();

    // 3. Outfit from both owned instances.
    let outfit = exec(
        &executor,
        "outfit.create",
        json!({"name":"Date night","item_instance_ids":[item_id, catalog_item_id]}),
    );
    let outfit_id = outfit["outfit_id"].as_str().expect("outfit id").to_owned();
    exec(
        &executor,
        "outfit.rename",
        json!({"outfit_id": outfit_id, "name":"Evening"}),
    );

    // 4. Journal with real persisted multiline content.
    let journal = exec(
        &executor,
        "document.create",
        json!({"title":"Journal","format":"markdown","body":"Day 1: started my journal.\nDay 2: wrote more."}),
    );
    let document_id = journal["document_id"]
        .as_str()
        .expect("document id")
        .to_owned();
    exec(
        &executor,
        "document.write",
        json!({"document_id": document_id, "body":"Day 1: started.\nDay 2: continued.\nDay 3: done."}),
    );

    // 5. Personal object with real state transitions.
    let object = exec(
        &executor,
        "object.create",
        json!({"location":"desk","state":"closed","affordances":["open","read"]}),
    );
    let object_id = object["object_id"].as_str().expect("object id").to_owned();
    exec(
        &executor,
        "object.update",
        json!({"object_id": object_id, "state":"open", "location":"reading desk"}),
    );

    // 6. Simulate Core restart: new executor, same persistent root.
    drop(executor);
    let restarted = ToolExecutor::new(temp.path(), SOFIA).expect("executor after restart");

    let inventory = exec(&restarted, "item.list", json!({}));
    let inventory = inventory.as_array().expect("inventory");
    assert!(inventory.len() >= 2);
    assert!(inventory
        .iter()
        .any(|item| item["instance"]["instance_id"].as_str() == Some(item_id.as_str())));

    let read = exec(
        &restarted,
        "document.read",
        json!({"document_id": document_id}),
    );
    let body = read["body"].as_str().expect("body");
    assert!(body.contains("Day 3: done."));

    let object = exec(&restarted, "object.get", json!({"object_id": object_id}));
    assert_eq!(object["state"].as_str(), Some("open"));
    assert_eq!(object["location"].as_str(), Some("reading desk"));

    let outfits = exec(&restarted, "outfit.list", json!({}));
    let outfits = outfits.as_array().expect("outfits");
    assert!(outfits
        .iter()
        .any(|outfit| outfit["name"].as_str() == Some("Evening")));
}

#[test]
fn another_core_cannot_read_or_modify_private_items() {
    let (temp, token) = setup();
    let installer = CoreInstaller::new(temp.path()).expect("installer");
    installer
        .initialize(&request(MACK, 2, "Mack Fuller"), &token)
        .expect("install Mack");

    let sofia = ToolExecutor::new(temp.path(), SOFIA).expect("sofia executor");
    let item = exec(
        &sofia,
        "item.create",
        json!({"name":"Private coat","category":"garment"}),
    );
    let item_id = item["instance"]["instance_id"]
        .as_str()
        .expect("item id")
        .to_owned();

    let mack = ToolExecutor::new(temp.path(), MACK).expect("mack executor");
    let read = mack
        .execute("item.get", &json!({"instance_id": item_id}), None)
        .expect("execute");
    assert_ne!(read.status, ToolStatus::Executed);
    assert_eq!(read.output, Value::Null);

    let modify = mack
        .execute(
            "item.modify",
            &json!({"instance_id": item_id, "name":"Stolen coat"}),
            None,
        )
        .expect("execute");
    assert_ne!(modify.status, ToolStatus::Executed);

    // The owner's item remains unchanged.
    let current = exec(&sofia, "item.get", json!({"instance_id": item_id}));
    assert_eq!(current["name"].as_str(), Some("Private coat"));
}
