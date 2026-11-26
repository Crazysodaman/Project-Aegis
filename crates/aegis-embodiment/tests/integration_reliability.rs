use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Barrier};
use std::time::Instant;

use aegis_contracts::initialization::{CoreInitializationRequest, CORE_INIT_REQUEST_SCHEMA};
use aegis_core_install::CoreInstaller;
use aegis_embodiment::{
    AssetFilter, AssetKind, AuditOutcome, EmbodimentStore, NewAsset, MAX_ASSET_METADATA_BYTES,
    MAX_ASSET_NAME_CHARS,
};
use aegis_platform::PlatformService;
use aegis_storage::now_rfc3339;
use serde_json::json;

const CORE_A: &str = "a9a0a604-a4e7-4a88-a985-3f071e2c8e7d";
const CORE_B: &str = "11111111-1111-4111-8111-111111111111";
const CORE_C: &str = "22222222-2222-4222-8222-222222222222";
const CORE_D: &str = "33333333-3333-4333-8333-333333333333";
const CORES: [&str; 4] = [CORE_A, CORE_B, CORE_C, CORE_D];

struct Fixture {
    temp: tempfile::TempDir,
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
    Fixture { temp, root }
}

fn open_store(root: &Path, core: &str) -> EmbodimentStore {
    EmbodimentStore::open(root, core).expect("store")
}

fn import(store: &EmbodimentStore, name: &str, bytes: &[u8]) -> String {
    store
        .import_asset(NewAsset::new(
            AssetKind::Other,
            name,
            "application/octet-stream",
            bytes,
        ))
        .expect("import")
        .asset_id
}

fn marker_for(root_index: usize) -> String {
    format!("core{root_index}")
}

#[test]
fn repeated_open_reopen_and_audit_cycles_release_handles() {
    let fixture = fixture();
    let cycles = 12;
    let started = Instant::now();
    let mut expected = Vec::new();
    for cycle in 0..cycles {
        let store = open_store(&fixture.root, CORE_A);
        let name = format!("asset-{cycle}.bin");
        let bytes = format!("payload-{cycle}").into_bytes();
        let asset_id = import(&store, &name, &bytes);
        expected.push((asset_id, bytes.len()));
        let audit = store.verify_all().expect("audit");
        assert_eq!(audit.entries.len(), cycle + 1);
        assert!(audit
            .entries
            .iter()
            .all(|entry| entry.outcome == AuditOutcome::Verified));
    }
    let elapsed = started.elapsed();

    let reopened = open_store(&fixture.root, CORE_A);
    let assets = reopened
        .list_filtered(&AssetFilter::default())
        .expect("list");
    assert_eq!(assets.len(), cycles);
    assert!(assets.iter().all(|asset| asset.core_uuid == CORE_A));
    for (asset_id, length) in &expected {
        assert_eq!(
            reopened.read_asset(asset_id).expect("read asset").len(),
            *length
        );
    }
    drop(reopened);

    fs::remove_dir_all(&fixture.root).expect("no leaked store handles");
    drop(fixture.temp);
    eprintln!("{cycles} embodiment open/audit cycles in {elapsed:?}");
}

#[test]
fn concurrent_multi_core_asset_libraries_stay_isolated() {
    let fixture = fixture();
    let root = Arc::new(fixture.root.clone());
    let barrier = Arc::new(Barrier::new(CORES.len()));
    let assets_per_core = 5;
    let started = Instant::now();
    let mut handles = Vec::with_capacity(CORES.len());
    for (core_index, core_uuid) in CORES.iter().enumerate() {
        let root = Arc::clone(&root);
        let barrier = Arc::clone(&barrier);
        let core_uuid = *core_uuid;
        handles.push(std::thread::spawn(move || {
            let store = EmbodimentStore::open(root.as_ref(), core_uuid).expect("store");
            let marker = marker_for(core_index);
            barrier.wait();
            for index in 0..assets_per_core {
                let name = format!("{marker}-asset-{index}.bin");
                let bytes = format!("{marker}-{index}").into_bytes();
                import(&store, &name, &bytes);
            }
            store
                .verify_all()
                .expect("audit")
                .entries
                .iter()
                .all(|entry| entry.outcome == AuditOutcome::Verified)
        }));
    }
    for handle in handles {
        assert!(handle.join().expect("core thread"), "audit verified");
    }
    let elapsed = started.elapsed();

    for (core_index, core_uuid) in CORES.iter().enumerate() {
        let store = open_store(&fixture.root, core_uuid);
        let assets = store.list_filtered(&AssetFilter::default()).expect("list");
        assert_eq!(assets.len(), assets_per_core);
        assert!(assets.iter().all(|asset| asset.core_uuid == *core_uuid));
        let marker = marker_for(core_index);
        assert!(assets.iter().all(|asset| asset.name.starts_with(&marker)));
        let other = marker_for((core_index + 1) % CORES.len());
        let foreign = store
            .search(&AssetFilter {
                name_contains: Some(other),
                ..AssetFilter::default()
            })
            .expect("foreign search");
        assert!(foreign.is_empty(), "no cross-Core asset contamination");
    }
    eprintln!(
        "concurrent embodiment: {} cores x {assets_per_core} assets in {elapsed:?}",
        CORES.len()
    );
}

#[test]
fn large_but_bounded_assets_and_metadata_stay_within_contract() {
    let fixture = fixture();
    let store = open_store(&fixture.root, CORE_A);
    let bytes = vec![0x5a_u8; 1024 * 1024];
    let note = "n".repeat(60_000);
    let metadata = json!({ "note": note });
    let encoded = serde_json::to_vec(&metadata).expect("metadata");
    assert!(encoded.len() <= MAX_ASSET_METADATA_BYTES);

    let name = "a".repeat(MAX_ASSET_NAME_CHARS);
    let asset = store
        .import_asset(
            NewAsset::new(AssetKind::Other, &name, "application/octet-stream", &bytes)
                .with_metadata(&metadata),
        )
        .expect("large bounded asset");
    assert_eq!(asset.bytes, bytes.len() as u64);
    assert_eq!(asset.metadata["note"].as_str().expect("note").len(), 60_000);
    assert_eq!(
        store.read_asset(&asset.asset_id).expect("read").len(),
        bytes.len()
    );
    assert_eq!(
        store.asset(&asset.asset_id).expect("asset").metadata["note"]
            .as_str()
            .expect("note")
            .len(),
        60_000
    );

    assert!(store
        .import_asset(NewAsset::new(
            AssetKind::Other,
            &"a".repeat(MAX_ASSET_NAME_CHARS + 1),
            "application/octet-stream",
            &bytes,
        ))
        .is_err());
    let oversized = json!({ "note": "n".repeat(70_000) });
    assert!(store
        .import_asset(
            NewAsset::new(
                AssetKind::Other,
                "oversized.bin",
                "application/octet-stream",
                &bytes
            )
            .with_metadata(&oversized),
        )
        .is_err());
    assert!(store
        .import_asset(NewAsset::new(
            AssetKind::Other,
            "empty.bin",
            "application/octet-stream",
            &[],
        ))
        .is_err());
}
