use std::path::Path;

use aegis_contracts::living::{ItemCategory, ItemDefinition, Visibility, LIVING_CONTRACT_VERSION};
use aegis_platform::{
    CatalogActor, CatalogError, CatalogFilter, CatalogStore, PlatformService, CATALOG_MAX_LIMIT,
};
use aegis_storage::{
    open_initialized, open_read_only, StoragePaths, PLATFORM_APPLICATION_ID, SAFE_APPLICATION_ID,
};
use serde_json::json;

const CREATOR: &str = "a9a0a604-a4e7-4a88-a985-3f071e2c8e7d";
const OTHER_CREATOR: &str = "b9a0a604-a4e7-4a88-a985-3f071e2c8e7d";

fn definition(
    definition_id: &str,
    name: &str,
    category: ItemCategory,
    tags: &[&str],
    creator_core_uuid: Option<&str>,
) -> ItemDefinition {
    ItemDefinition {
        schema: LIVING_CONTRACT_VERSION.to_owned(),
        definition_id: definition_id.to_owned(),
        creator_core_uuid: creator_core_uuid.map(str::to_owned),
        name: name.to_owned(),
        description: format!("{name} description"),
        category,
        visibility: Visibility::Private,
        tags: tags.iter().map(|tag| (*tag).to_owned()).collect(),
        sizes: vec!["m".to_owned()],
        materials: vec!["linen".to_owned()],
        properties: json!({"color": "blue"}),
        asset_id: None,
        created_at: "2026-01-01T00:00:00Z".to_owned(),
        updated_at: "2026-01-01T00:00:00Z".to_owned(),
        revision: 1,
    }
}

fn paths(root: &Path) -> StoragePaths {
    StoragePaths::new(root).expect("storage paths")
}

fn initialized_catalog(root: &Path) -> (CatalogStore, String) {
    let initialized = PlatformService::initialize(root).expect("initialize platform");
    let catalog = CatalogStore::new(paths(root));
    catalog.initialize().expect("initialize catalog");
    (catalog, initialized.owner_token)
}

fn ids(definitions: &[ItemDefinition]) -> Vec<&str> {
    definitions
        .iter()
        .map(|definition| definition.definition_id.as_str())
        .collect()
}

fn sorted_ids(definitions: &[ItemDefinition]) -> Vec<String> {
    let mut ids: Vec<String> = definitions
        .iter()
        .map(|definition| definition.definition_id.clone())
        .collect();
    ids.sort();
    ids
}

fn assert_deterministic_order(definitions: &[ItemDefinition]) {
    for pair in definitions.windows(2) {
        let first = &pair[0];
        let second = &pair[1];
        assert!(
            first.updated_at > second.updated_at
                || (first.updated_at == second.updated_at
                    && first.definition_id < second.definition_id),
            "{} must sort before {}",
            first.definition_id,
            second.definition_id
        );
    }
}

fn event_rows(root: &Path, definition_id: &str) -> Vec<(String, String, i64)> {
    let connection = open_read_only(&paths(root).platform_db(), PLATFORM_APPLICATION_ID)
        .expect("platform read-only");
    let mut statement = connection
        .prepare(
            "SELECT event_type, actor, revision FROM catalog_events
             WHERE definition_id = ?1 ORDER BY sequence ASC",
        )
        .expect("prepare events");
    statement
        .query_map([definition_id], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?))
        })
        .expect("query events")
        .map(|row| row.expect("event row"))
        .collect()
}

#[test]
fn create_get_list_and_search_by_category_tag_and_creator() {
    let temp = tempfile::tempdir().expect("tempdir");
    let (catalog, owner_token) = initialized_catalog(temp.path());

    let dress = definition(
        "def-dress",
        "Summer dress",
        ItemCategory::Garment,
        &["summer", "linen"],
        Some(CREATOR),
    );
    let boots = definition(
        "def-boots",
        "Leather boots",
        ItemCategory::Footwear,
        &["winter", "leather"],
        Some(OTHER_CREATOR),
    );
    let book = definition(
        "def-book",
        "Field notes",
        ItemCategory::Book,
        &["summer", "notes"],
        None,
    );

    let stored = catalog
        .create(&owner_token, dress.clone())
        .expect("create dress");
    assert_eq!(stored.definition_id, dress.definition_id);
    assert_eq!(stored.revision, 1);
    assert!(!stored.created_at.is_empty());
    assert!(!stored.updated_at.is_empty());
    catalog.create(&owner_token, boots).expect("create boots");
    catalog.create(&owner_token, book).expect("create book");

    let fetched = catalog
        .get("def-dress")
        .expect("get dress")
        .expect("dress present");
    assert_eq!(fetched, stored);
    assert!(catalog.get("def-missing").expect("missing").is_none());

    let all = catalog.list(&CatalogFilter::default()).expect("list all");
    assert_eq!(ids(&all).len(), 3);
    assert_deterministic_order(&all);

    let garments = catalog
        .list(&CatalogFilter {
            category: Some(ItemCategory::Garment),
            ..CatalogFilter::default()
        })
        .expect("list garments");
    assert_eq!(sorted_ids(&garments), vec!["def-dress"]);

    let summer = catalog
        .list(&CatalogFilter {
            tag: Some("summer".to_owned()),
            ..CatalogFilter::default()
        })
        .expect("list summer");
    assert_eq!(sorted_ids(&summer), vec!["def-book", "def-dress"]);

    let by_creator = catalog
        .list(&CatalogFilter {
            creator_core_uuid: Some(CREATOR.to_owned()),
            ..CatalogFilter::default()
        })
        .expect("list creator");
    assert_eq!(sorted_ids(&by_creator), vec!["def-dress"]);

    let by_visibility = catalog
        .list(&CatalogFilter {
            visibility: Some(Visibility::Private),
            ..CatalogFilter::default()
        })
        .expect("list visibility");
    assert_eq!(by_visibility.len(), 3);

    let found_summer = catalog.search("summer").expect("search summer");
    assert_eq!(sorted_ids(&found_summer), vec!["def-book", "def-dress"]);
    let found_leather = catalog.search("leather").expect("search leather");
    assert_eq!(sorted_ids(&found_leather), vec!["def-boots"]);
    let found_category = catalog.search("footwear").expect("search category");
    assert_eq!(sorted_ids(&found_category), vec!["def-boots"]);
    let found_creator = catalog.search(CREATOR).expect("search creator");
    assert_eq!(sorted_ids(&found_creator), vec!["def-dress"]);
    assert!(matches!(
        catalog.search("   "),
        Err(CatalogError::InvalidSearch)
    ));
}

#[test]
fn list_limits_are_bounded_and_ordered() {
    let temp = tempfile::tempdir().expect("tempdir");
    let (catalog, owner_token) = initialized_catalog(temp.path());

    for index in 0..3 {
        let item = definition(
            &format!("def-{index}"),
            &format!("Item {index}"),
            ItemCategory::Other,
            &["batch"],
            None,
        );
        catalog.create(&owner_token, item).expect("create item");
    }

    let limited = catalog
        .list(&CatalogFilter {
            limit: Some(2),
            ..CatalogFilter::default()
        })
        .expect("bounded list");
    assert_eq!(limited.len(), 2);
    assert_deterministic_order(&limited);

    assert!(matches!(
        catalog.list(&CatalogFilter {
            limit: Some(0),
            ..CatalogFilter::default()
        }),
        Err(CatalogError::InvalidFilter("limit"))
    ));
    assert!(matches!(
        catalog.list(&CatalogFilter {
            limit: Some(CATALOG_MAX_LIMIT + 1),
            ..CatalogFilter::default()
        }),
        Err(CatalogError::InvalidFilter("limit"))
    ));
}

#[test]
fn duplicate_definition_ids_are_rejected() {
    let temp = tempfile::tempdir().expect("tempdir");
    let (catalog, owner_token) = initialized_catalog(temp.path());
    let item = definition(
        "def-duplicate",
        "First name",
        ItemCategory::Accessory,
        &["unique"],
        None,
    );

    catalog
        .create(&owner_token, item.clone())
        .expect("first create");
    let error = catalog
        .create(&owner_token, item)
        .expect_err("duplicate must fail");
    assert!(matches!(error, CatalogError::Duplicate(id) if id == "def-duplicate"));
    assert_eq!(
        catalog.list(&CatalogFilter::default()).expect("list").len(),
        1
    );
}

#[test]
fn invalid_definitions_and_creators_are_rejected() {
    let temp = tempfile::tempdir().expect("tempdir");
    let (catalog, owner_token) = initialized_catalog(temp.path());

    let blank_name = definition("def-blank", " ", ItemCategory::Garment, &["valid"], None);
    assert!(matches!(
        catalog.create(&owner_token, blank_name),
        Err(CatalogError::InvalidDefinition)
    ));

    let mut bad_creator = definition(
        "def-bad-creator",
        "Bad creator",
        ItemCategory::Garment,
        &["valid"],
        None,
    );
    bad_creator.creator_core_uuid = Some("not-a-uuid".to_owned());
    assert!(matches!(
        catalog.create(&owner_token, bad_creator),
        Err(CatalogError::InvalidCreator)
    ));

    let mut uppercase_creator = definition(
        "def-uppercase-creator",
        "Uppercase creator",
        ItemCategory::Garment,
        &["valid"],
        None,
    );
    uppercase_creator.creator_core_uuid = Some(CREATOR.to_uppercase());
    assert!(matches!(
        catalog.create(&owner_token, uppercase_creator),
        Err(CatalogError::InvalidCreator)
    ));

    assert!(catalog
        .list(&CatalogFilter::default())
        .expect("list")
        .is_empty());
}

#[test]
fn catalog_survives_reopen_with_events() {
    let temp = tempfile::tempdir().expect("tempdir");
    let stored = {
        let (catalog, owner_token) = initialized_catalog(temp.path());
        catalog
            .create(
                &owner_token,
                definition(
                    "def-persistent",
                    "Persistent item",
                    ItemCategory::Jewelry,
                    &["keepsake"],
                    Some(CREATOR),
                ),
            )
            .expect("create persistent")
    };

    let reopened = CatalogStore::new(paths(temp.path()));
    reopened.initialize().expect("reinitialize catalog");
    let fetched = reopened
        .get("def-persistent")
        .expect("get after reopen")
        .expect("present after reopen");
    assert_eq!(fetched, stored);
    assert_eq!(
        event_rows(temp.path(), "def-persistent"),
        vec![("catalog.created".to_owned(), "owner".to_owned(), 1)]
    );
}

#[test]
fn events_are_append_only_and_archive_preserves_history() {
    let temp = tempfile::tempdir().expect("tempdir");
    let (catalog, owner_token) = initialized_catalog(temp.path());
    let created = catalog
        .create(
            &owner_token,
            definition(
                "def-history",
                "History item",
                ItemCategory::Tool,
                &["history"],
                Some(CREATOR),
            ),
        )
        .expect("create history");

    let mut replacement = created.clone();
    replacement.name = "Renamed history item".to_owned();
    replacement.revision = 99;
    let updated = catalog
        .update_own(&owner_token, "def-history", replacement)
        .expect("update history");
    assert_eq!(updated.revision, 2);
    assert_eq!(updated.created_at, created.created_at);
    assert_eq!(updated.name, "Renamed history item");

    assert!(catalog
        .archive(&owner_token, "def-history")
        .expect("archive history"));
    assert!(!catalog
        .archive(&owner_token, "def-history")
        .expect("archive is idempotent"));
    assert!(catalog.get("def-history").expect("get archived").is_none());
    assert!(!catalog
        .delete(&owner_token, "def-history")
        .expect("delete alias is idempotent"));

    let visible = catalog
        .list(&CatalogFilter::default())
        .expect("default list");
    assert!(visible.is_empty());
    let archived = catalog
        .list(&CatalogFilter {
            include_archived: true,
            ..CatalogFilter::default()
        })
        .expect("archived list");
    assert_eq!(ids(&archived), vec!["def-history"]);

    assert_eq!(
        event_rows(temp.path(), "def-history"),
        vec![
            ("catalog.created".to_owned(), "owner".to_owned(), 1),
            ("catalog.updated".to_owned(), "owner".to_owned(), 2),
            ("catalog.archived".to_owned(), "owner".to_owned(), 2),
        ]
    );

    let writable = open_initialized(
        &paths(temp.path()).platform_db(),
        PLATFORM_APPLICATION_ID,
        "platform",
    )
    .expect("platform writable");
    assert!(writable
        .execute(
            "UPDATE catalog_events SET actor = 'tampered' WHERE definition_id = 'def-history'",
            [],
        )
        .is_err());
    assert!(writable
        .execute(
            "DELETE FROM catalog_events WHERE definition_id = 'def-history'",
            [],
        )
        .is_err());
    assert_eq!(event_rows(temp.path(), "def-history").len(), 3);
}

#[test]
fn owner_proof_is_required_for_mutations_including_visibility() {
    let temp = tempfile::tempdir().expect("tempdir");
    let (catalog, owner_token) = initialized_catalog(temp.path());
    let created = catalog
        .create(
            &owner_token,
            definition(
                "def-gated",
                "Gated item",
                ItemCategory::Artwork,
                &["gated"],
                None,
            ),
        )
        .expect("create gated");

    assert!(matches!(
        catalog.create(
            "wrong-token",
            definition(
                "def-denied",
                "Denied item",
                ItemCategory::Other,
                &["denied"],
                None
            )
        ),
        Err(CatalogError::Unauthorized)
    ));

    let mut public = created.clone();
    public.visibility = Visibility::PublicCatalog;
    assert!(matches!(
        catalog.update_own("wrong-token", "def-gated", public.clone()),
        Err(CatalogError::Unauthorized)
    ));
    assert!(matches!(
        catalog.archive("wrong-token", "def-gated"),
        Err(CatalogError::Unauthorized)
    ));

    let published = catalog
        .update_own(&owner_token, "def-gated", public)
        .expect("owner can publish");
    assert_eq!(published.visibility, Visibility::PublicCatalog);
    assert_eq!(published.revision, 2);
}

#[test]
fn explicit_catalog_actor_with_owner_token_is_recorded() {
    let temp = tempfile::tempdir().expect("tempdir");
    let (catalog, owner_token) = initialized_catalog(temp.path());
    let actor = CatalogActor::new("core:assistant", &owner_token).expect("catalog actor");

    let created = catalog
        .create_as(
            &actor,
            definition(
                "def-actor",
                "Actor item",
                ItemCategory::Container,
                &["actor"],
                Some(OTHER_CREATOR),
            ),
        )
        .expect("create as actor");
    let mut replacement = created.clone();
    replacement.description = "Updated by actor".to_owned();
    catalog
        .update_own_as(&actor, "def-actor", replacement)
        .expect("update as actor");
    catalog
        .archive_as(&actor, "def-actor")
        .expect("archive as actor");

    assert_eq!(
        event_rows(temp.path(), "def-actor"),
        vec![
            ("catalog.created".to_owned(), "core:assistant".to_owned(), 1),
            ("catalog.updated".to_owned(), "core:assistant".to_owned(), 2),
            (
                "catalog.archived".to_owned(),
                "core:assistant".to_owned(),
                2
            ),
        ]
    );
    assert!(CatalogActor::new(" ", &owner_token).is_err());
}

#[test]
fn catalogs_do_not_cross_platform_boundaries() {
    let first = tempfile::tempdir().expect("first tempdir");
    let second = tempfile::tempdir().expect("second tempdir");
    let (catalog_a, token_a) = initialized_catalog(first.path());
    let (catalog_b, _token_b) = initialized_catalog(second.path());

    catalog_a
        .create(
            &token_a,
            definition(
                "def-platform-a",
                "Platform A item",
                ItemCategory::Decoration,
                &["alpha"],
                None,
            ),
        )
        .expect("create on first platform");

    assert!(catalog_b
        .get("def-platform-a")
        .expect("second get")
        .is_none());
    assert!(catalog_b
        .list(&CatalogFilter::default())
        .expect("second list")
        .is_empty());

    let first_paths = paths(first.path());
    let platform = open_read_only(&first_paths.platform_db(), PLATFORM_APPLICATION_ID)
        .expect("first platform read-only");
    let definitions: i64 = platform
        .query_row("SELECT COUNT(*) FROM catalog_definitions", [], |row| {
            row.get(0)
        })
        .expect("definition count");
    assert_eq!(definitions, 1);

    let safe =
        open_read_only(&first_paths.safe_db(), SAFE_APPLICATION_ID).expect("first safe read-only");
    let catalog_tables: i64 = safe
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master
             WHERE type = 'table' AND name LIKE 'catalog_%'",
            [],
            |row| row.get(0),
        )
        .expect("safe catalog tables");
    assert_eq!(catalog_tables, 0);
}
