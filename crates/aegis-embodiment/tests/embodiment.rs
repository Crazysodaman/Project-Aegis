use std::fs;
use std::path::{Path, PathBuf};

use aegis_contracts::initialization::{CoreInitializationRequest, CORE_INIT_REQUEST_SCHEMA};
use aegis_core_install::CoreInstaller;
use aegis_embodiment::{
    AssetKind, EmbodimentError, EmbodimentStore, NewAsset, Theme, VoiceProfile, AEGIS_ACCENT,
    AEGIS_PRIMARY, AEGIS_TERTIARY,
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

fn import(store: &EmbodimentStore, kind: AssetKind, name: &str, bytes: &[u8]) -> String {
    store
        .import_asset(NewAsset::new(kind, name, "application/octet-stream", bytes))
        .expect("import")
        .asset_id
}

#[test]
fn assets_import_read_list_and_delete() {
    let (_temp, root) = setup();
    let store = open_store(&root, CORE_A);
    let id = import(&store, AssetKind::Avatar, "base.png", b"avatar-bytes");

    let asset = store.asset(&id).expect("asset");
    assert_eq!(asset.kind, AssetKind::Avatar);
    assert_eq!(asset.bytes, 12);
    assert!(!asset.deleted);
    assert_eq!(asset.revision, 1);
    assert_eq!(store.read_asset(&id).expect("read"), b"avatar-bytes");

    let listed = store.list_assets(None, false).expect("list");
    assert_eq!(listed.len(), 1);
    assert_eq!(
        store
            .list_assets(Some(AssetKind::Gesture), false)
            .expect("kind filter")
            .len(),
        0
    );

    let deleted = store.delete_asset(&id).expect("delete");
    assert!(deleted.deleted);
    assert_eq!(deleted.revision, 2);
    assert!(matches!(
        store.read_asset(&id),
        Err(EmbodimentError::Deleted)
    ));
    assert_eq!(store.list_assets(None, false).expect("list").len(), 0);
    assert_eq!(store.list_assets(None, true).expect("list").len(), 1);
}

#[test]
fn tampered_asset_content_is_detected() {
    let (_temp, root) = setup();
    let store = open_store(&root, CORE_A);
    let id = import(&store, AssetKind::Expression, "smile.bin", b"original");
    let relative = store.asset(&id).expect("asset").relative_path;
    let path = root.join("cores").join(CORE_A).join(relative);
    fs::write(&path, b"tampered").expect("tamper");
    assert!(matches!(
        store.read_asset(&id),
        Err(EmbodimentError::IntegrityMismatch)
    ));
}

#[test]
fn appearance_defaults_and_updates_persist() {
    let (_temp, root) = setup();
    let store = open_store(&root, CORE_A);
    let initial = store.appearance().expect("appearance");
    assert_eq!(initial.theme.primary, AEGIS_PRIMARY);
    assert_eq!(initial.theme.accent, AEGIS_ACCENT);
    assert_eq!(initial.theme.tertiary, AEGIS_TERTIARY);
    assert_eq!(initial.revision, 1);
    assert!(initial.avatar_asset_id.is_none());

    let avatar = import(&store, AssetKind::Avatar, "base.png", b"avatar");
    let updated = store.set_avatar(Some(&avatar)).expect("avatar");
    assert_eq!(updated.avatar_asset_id.as_deref(), Some(avatar.as_str()));
    assert!(updated.revision > initial.revision);

    let expression = import(&store, AssetKind::Expression, "happy.bin", b"happy");
    let gesture = import(&store, AssetKind::Gesture, "wave.bin", b"wave");
    let wardrobe = import(&store, AssetKind::Wardrobe, "coat.bin", b"coat");
    let voice_sample = import(&store, AssetKind::VoiceSample, "hello.wav", b"voice");

    store
        .set_expression("happy", &expression)
        .expect("expression");
    store.set_gesture("wave", &gesture).expect("gesture");
    store
        .set_wardrobe(vec![wardrobe.clone()])
        .expect("wardrobe");
    store
        .set_voice(Some(VoiceProfile {
            voice_id: "local-voice".to_owned(),
            display_name: "Local Voice".to_owned(),
            sample_asset_id: Some(voice_sample),
        }))
        .expect("voice");
    let theme = Theme {
        primary: "#112233".to_owned(),
        accent: "#445566".to_owned(),
        tertiary: "#778899".to_owned(),
    };
    store.set_theme(theme.clone()).expect("theme");

    let reopened = open_store(&root, CORE_A);
    let appearance = reopened.appearance().expect("reopen");
    assert_eq!(appearance.avatar_asset_id.as_deref(), Some(avatar.as_str()));
    assert_eq!(appearance.expressions.get("happy"), Some(&expression));
    assert_eq!(appearance.gestures.get("wave"), Some(&gesture));
    assert_eq!(appearance.wardrobe, vec![wardrobe]);
    assert_eq!(appearance.voice.expect("voice").voice_id, "local-voice");
    assert_eq!(appearance.theme, theme);

    let removed = reopened.remove_expression("happy").expect("remove");
    assert!(!removed.expressions.contains_key("happy"));
}

#[test]
fn appearance_rejects_wrong_kind_and_invalid_theme() {
    let (_temp, root) = setup();
    let store = open_store(&root, CORE_A);
    let avatar = import(&store, AssetKind::Avatar, "base.png", b"avatar");
    let error = store
        .set_expression("happy", &avatar)
        .expect_err("wrong kind");
    assert!(matches!(error, EmbodimentError::WrongAssetKind(_)));

    let before = store.appearance().expect("appearance");
    let error = store
        .set_theme(Theme {
            primary: "purple".to_owned(),
            ..Theme::default()
        })
        .expect_err("invalid hex");
    assert!(matches!(error, EmbodimentError::InvalidInput(_)));
    assert_eq!(store.appearance().expect("appearance"), before);
}

#[test]
fn cores_have_isolated_assets_and_appearance() {
    let (_temp, root) = setup();
    let store_a = open_store(&root, CORE_A);
    let store_b = open_store(&root, CORE_B);
    let avatar_a = import(&store_a, AssetKind::Avatar, "a.png", b"a-avatar");
    store_a.set_avatar(Some(&avatar_a)).expect("avatar a");

    assert_eq!(store_b.list_assets(None, true).expect("list b").len(), 0);
    assert!(store_b
        .appearance()
        .expect("appearance b")
        .avatar_asset_id
        .is_none());

    drop(store_b);
    let source = root.join("cores").join(CORE_A).join("assets.db");
    let target = root.join("cores").join(CORE_B).join("assets.db");
    fs::copy(&source, &target).expect("copy");
    let error = EmbodimentStore::open(&root, CORE_B).expect_err("must reject foreign DB");
    assert!(matches!(error, EmbodimentError::OwnershipMismatch));
}

#[test]
fn export_contains_assets_and_appearance() {
    let (_temp, root) = setup();
    let store = open_store(&root, CORE_A);
    let avatar = import(&store, AssetKind::Avatar, "base.png", b"avatar");
    store.set_avatar(Some(&avatar)).expect("avatar");
    let export = store.export().expect("export");
    assert_eq!(export.core_uuid, CORE_A);
    assert_eq!(export.assets.len(), 1);
    assert_eq!(
        export.appearance.avatar_asset_id.as_deref(),
        Some(avatar.as_str())
    );
}

#[test]
fn asset_names_and_kinds_are_validated() {
    let (_temp, root) = setup();
    let store = open_store(&root, CORE_A);
    assert!(store
        .import_asset(NewAsset::new(
            AssetKind::Other,
            "../escape",
            "application/octet-stream",
            b"x"
        ))
        .is_err());
    assert!(store
        .import_asset(NewAsset::new(
            AssetKind::Other,
            "ok",
            "application/octet-stream",
            b""
        ))
        .is_err());
    assert!(AssetKind::parse("not_a_kind").is_err());
}
