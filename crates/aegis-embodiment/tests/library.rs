use std::fs;
use std::path::{Path, PathBuf};

use aegis_contracts::initialization::{CoreInitializationRequest, CORE_INIT_REQUEST_SCHEMA};
use aegis_core_install::CoreInstaller;
use aegis_embodiment::{
    AssetAudit, AssetFilter, AssetKind, AssetState, AuditOutcome, EmbodimentStore, NewAsset,
    ASSET_MANIFEST_SCHEMA,
};
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

fn open_store(root: &Path, core: &str) -> EmbodimentStore {
    EmbodimentStore::open(root, core).expect("store")
}

fn core_assets_dir(root: &Path, core: &str) -> PathBuf {
    root.join("cores").join(core)
}

fn import_tagged(
    store: &EmbodimentStore,
    kind: AssetKind,
    name: &str,
    media_type: &str,
    bytes: &[u8],
    tags: Vec<String>,
) -> String {
    store
        .import_asset(NewAsset::new(kind, name, media_type, bytes).with_tags(tags))
        .expect("import")
        .asset_id
}

fn outcome_for(audit: &AssetAudit, asset_id: &str) -> AuditOutcome {
    audit
        .entries
        .iter()
        .find(|entry| entry.asset_id == asset_id)
        .expect("audit entry")
        .outcome
}

fn ids(assets: &[aegis_embodiment::Asset]) -> Vec<String> {
    assets.iter().map(|asset| asset.asset_id.clone()).collect()
}

#[test]
fn search_filters_by_kind_name_tags_and_media_type() {
    let (_temp, root) = setup();
    let store = open_store(&root, CORE_A);
    let hero = import_tagged(
        &store,
        AssetKind::Avatar,
        "Hero Portrait.png",
        "image/png",
        b"hero-bytes",
        vec!["character".to_owned(), "Hero".to_owned()],
    );
    let alt = import_tagged(
        &store,
        AssetKind::Avatar,
        "hero_alt.png",
        "IMAGE/PNG",
        b"alt-bytes",
        vec!["hero".to_owned()],
    );
    let smile = import_tagged(
        &store,
        AssetKind::Expression,
        "smile.bin",
        "application/octet-stream",
        b"smile-bytes",
        vec!["face".to_owned()],
    );
    let _voice = import_tagged(
        &store,
        AssetKind::VoiceSample,
        "hello.wav",
        "audio/wav",
        b"hello-bytes",
        vec!["voice".to_owned(), "hero".to_owned()],
    );

    let avatars = store
        .list_filtered(&AssetFilter {
            kind: Some(AssetKind::Avatar),
            ..AssetFilter::default()
        })
        .expect("kind filter");
    assert_eq!(avatars.len(), 2);

    let by_name = store
        .search(&AssetFilter {
            name_contains: Some("HERO".to_owned()),
            ..AssetFilter::default()
        })
        .expect("name filter");
    assert_eq!(by_name.len(), 2);
    let name_ids = ids(&by_name);
    assert!(name_ids.contains(&hero));
    assert!(name_ids.contains(&alt));
    assert!(!name_ids.contains(&smile));

    let hero_tag = store
        .search(&AssetFilter {
            tags: vec!["hErO".to_owned()],
            ..AssetFilter::default()
        })
        .expect("tag filter");
    assert_eq!(hero_tag.len(), 3);

    let hero_character = store
        .search(&AssetFilter {
            tags: vec!["hero".to_owned(), "character".to_owned()],
            ..AssetFilter::default()
        })
        .expect("multi tag filter");
    assert_eq!(hero_character.len(), 1);
    assert_eq!(hero_character[0].asset_id, hero);

    let png = store
        .search(&AssetFilter {
            media_type: Some("image/png".to_owned()),
            ..AssetFilter::default()
        })
        .expect("media filter");
    assert_eq!(png.len(), 2);

    let combined = store
        .search(&AssetFilter {
            kind: Some(AssetKind::Avatar),
            name_contains: Some("hero".to_owned()),
            tags: vec!["hero".to_owned()],
            media_type: Some("image/png".to_owned()),
            include_deleted: false,
            limit: None,
        })
        .expect("combined filter");
    assert_eq!(combined.len(), 2);

    let limited = store
        .search(&AssetFilter {
            tags: vec!["hero".to_owned()],
            limit: Some(1),
            ..AssetFilter::default()
        })
        .expect("limited");
    assert_eq!(limited.len(), 1);

    let zero = store
        .search(&AssetFilter {
            tags: vec!["hero".to_owned()],
            limit: Some(0),
            ..AssetFilter::default()
        })
        .expect("zero limit");
    assert!(zero.is_empty());

    let first = store
        .list_filtered(&AssetFilter {
            include_deleted: true,
            ..AssetFilter::default()
        })
        .expect("deterministic a");
    let second = store
        .list_filtered(&AssetFilter {
            include_deleted: true,
            ..AssetFilter::default()
        })
        .expect("deterministic b");
    assert_eq!(ids(&first), ids(&second));
    assert_eq!(first.len(), 4);

    store.delete_asset(&smile).expect("delete");
    let live = store
        .search(&AssetFilter {
            include_deleted: false,
            ..AssetFilter::default()
        })
        .expect("live only");
    assert_eq!(live.len(), 3);
    let with_deleted = store
        .search(&AssetFilter {
            include_deleted: true,
            ..AssetFilter::default()
        })
        .expect("with deleted");
    assert_eq!(with_deleted.len(), 4);
}

#[test]
fn audit_reports_corrupt_and_missing_without_mutating() {
    let (_temp, root) = setup();
    let store = open_store(&root, CORE_A);
    let good = import_tagged(
        &store,
        AssetKind::Avatar,
        "good.bin",
        "application/octet-stream",
        b"good-data",
        Vec::new(),
    );
    let corrupt = import_tagged(
        &store,
        AssetKind::Expression,
        "corrupt.bin",
        "application/octet-stream",
        b"corrupt-data",
        Vec::new(),
    );
    let missing = import_tagged(
        &store,
        AssetKind::Gesture,
        "missing.bin",
        "application/octet-stream",
        b"missing-data",
        Vec::new(),
    );

    let corrupt_path = core_assets_dir(&root, CORE_A)
        .join(&store.asset(&corrupt).expect("corrupt asset").relative_path);
    fs::write(&corrupt_path, b"tampered").expect("tamper");
    let missing_path = core_assets_dir(&root, CORE_A)
        .join(&store.asset(&missing).expect("missing asset").relative_path);
    fs::remove_file(&missing_path).expect("remove missing");

    let audit = store.audit().expect("audit");
    assert_eq!(audit.core_uuid, CORE_A);
    assert_eq!(audit.entries.len(), 3);
    assert_eq!(audit.verified, 1);
    assert_eq!(audit.missing, 1);
    assert_eq!(audit.corrupt, 1);
    assert!(!audit.is_clean());
    assert_eq!(outcome_for(&audit, &good), AuditOutcome::Verified);
    assert_eq!(outcome_for(&audit, &corrupt), AuditOutcome::Corrupt);
    assert_eq!(outcome_for(&audit, &missing), AuditOutcome::Missing);
    for entry in &audit.entries {
        assert_eq!(entry.state, AssetState::Live);
    }

    let alias = store.verify_all().expect("verify_all");
    assert_eq!(alias.verified, 1);
    assert_eq!(alias.missing, 1);
    assert_eq!(alias.corrupt, 1);

    assert!(!store.asset(&corrupt).expect("corrupt record").deleted);
    assert!(!store.asset(&missing).expect("missing record").deleted);
    assert_eq!(store.list_assets(None, true).expect("records").len(), 3);
    assert!(corrupt_path.exists());
    assert!(!missing_path.exists());
    assert_eq!(store.audit().expect("audit again").entries.len(), 3);
}

#[test]
fn recovery_manifest_lists_live_and_deleted_assets() {
    let (_temp, root) = setup();
    let store = open_store(&root, CORE_A);
    let live = import_tagged(
        &store,
        AssetKind::Avatar,
        "live.png",
        "image/png",
        b"live-bytes",
        vec!["hero".to_owned()],
    );
    let deleted = import_tagged(
        &store,
        AssetKind::Expression,
        "gone.bin",
        "application/octet-stream",
        b"gone-bytes",
        Vec::new(),
    );
    store.delete_asset(&deleted).expect("delete");

    let manifest = store.recovery_manifest().expect("manifest");
    assert_eq!(manifest.schema, ASSET_MANIFEST_SCHEMA);
    assert_eq!(manifest.core_uuid, CORE_A);
    assert_eq!(manifest.entries.len(), 2);

    let live_entry = manifest
        .entries
        .iter()
        .find(|entry| entry.asset_id == live)
        .expect("live entry");
    let live_asset = store.asset(&live).expect("live asset");
    assert_eq!(live_entry.kind, live_asset.kind);
    assert_eq!(live_entry.name, live_asset.name);
    assert_eq!(live_entry.media_type, live_asset.media_type);
    assert_eq!(live_entry.sha256, live_asset.sha256);
    assert_eq!(live_entry.sha256.len(), 64);
    assert_eq!(live_entry.bytes, live_asset.bytes);
    assert_eq!(live_entry.state, AssetState::Live);
    assert_eq!(live_entry.relative_path, live_asset.relative_path);
    assert!(live_entry.relative_path.starts_with("assets/"));
    assert!(!live_entry.relative_path.starts_with("cores"));

    let deleted_entry = manifest
        .entries
        .iter()
        .find(|entry| entry.asset_id == deleted)
        .expect("deleted entry");
    assert_eq!(deleted_entry.state, AssetState::Deleted);

    let second = store.recovery_manifest().expect("manifest again");
    let first_ids: Vec<_> = manifest
        .entries
        .iter()
        .map(|entry| entry.asset_id.clone())
        .collect();
    let second_ids: Vec<_> = second
        .entries
        .iter()
        .map(|entry| entry.asset_id.clone())
        .collect();
    assert_eq!(first_ids, second_ids);

    let json = serde_json::to_string(&manifest).expect("manifest json");
    assert!(json.contains(ASSET_MANIFEST_SCHEMA));
}

#[test]
fn search_audit_and_manifest_are_core_scoped() {
    let (_temp, root) = setup();
    let store_a = open_store(&root, CORE_A);
    let store_b = open_store(&root, CORE_B);
    import_tagged(
        &store_a,
        AssetKind::Avatar,
        "only-a.png",
        "image/png",
        b"a-bytes",
        vec!["hero".to_owned()],
    );

    assert_eq!(
        store_b
            .list_filtered(&AssetFilter::default())
            .expect("b search")
            .len(),
        0
    );
    let audit_b = store_b.audit().expect("b audit");
    assert!(audit_b.entries.is_empty());
    assert!(audit_b.is_clean());
    assert!(store_b
        .recovery_manifest()
        .expect("b manifest")
        .entries
        .is_empty());
    assert_eq!(
        store_a
            .list_filtered(&AssetFilter::default())
            .expect("a search")
            .len(),
        1
    );
}
