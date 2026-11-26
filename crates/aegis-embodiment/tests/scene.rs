use std::path::{Path, PathBuf};

use aegis_contracts::initialization::{CoreInitializationRequest, CORE_INIT_REQUEST_SCHEMA};
use aegis_core_install::CoreInstaller;
use aegis_embodiment::{
    ActiveMotion, AssetKind, AvatarScene, EmbodimentError, EmbodimentStore, LayerRole, NewAsset,
    SceneLayer, Theme, AEGIS_ACCENT, AEGIS_PRIMARY, AEGIS_TERTIARY, SCENE_SCHEMA,
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

fn custom_theme() -> Theme {
    Theme {
        primary: "#112233".to_owned(),
        accent: "#445566".to_owned(),
        tertiary: "#778899".to_owned(),
    }
}

fn layer(role: LayerRole, asset_id: &str, z_index: i32) -> SceneLayer {
    SceneLayer {
        role,
        asset_id: asset_id.to_owned(),
        asset_kind: AssetKind::Avatar,
        media_type: "image/png".to_owned(),
        sha256: "digest".to_owned(),
        visible: true,
        z_index,
    }
}

#[test]
fn compose_builds_layers_theme_and_active_motion() {
    let (_temp, root) = setup();
    let store = open_store(&root, CORE_A);
    let avatar = import(&store, AssetKind::Avatar, "base.png", b"avatar");
    let coat = import(&store, AssetKind::Wardrobe, "coat.png", b"coat");
    let boots = import(&store, AssetKind::Wardrobe, "boots.png", b"boots");
    let happy = import(&store, AssetKind::Expression, "happy.bin", b"happy");
    let wave = import(&store, AssetKind::Gesture, "wave.bin", b"wave");

    store.set_avatar(Some(&avatar)).expect("avatar");
    store
        .set_wardrobe(vec![coat.clone(), boots.clone()])
        .expect("wardrobe");
    store.set_expression("happy", &happy).expect("expression");
    store.set_gesture("wave", &wave).expect("gesture");
    store.set_theme(custom_theme()).expect("theme");

    let appearance = store.appearance().expect("appearance");
    let assets = store.list_assets(None, false).expect("assets");
    let scene =
        AvatarScene::compose(&appearance, &assets, Some("happy"), Some("wave")).expect("compose");

    scene.validate().expect("valid");
    assert_eq!(scene.schema, SCENE_SCHEMA);
    assert_eq!(scene.core_uuid, CORE_A);
    assert_eq!(scene.theme, custom_theme());
    assert_eq!(scene.layers.len(), 3);
    assert_eq!(scene.layers[0].role, LayerRole::Base);
    assert_eq!(scene.layers[0].asset_id, avatar);
    assert_eq!(scene.layers[0].z_index, LayerRole::Base.default_z());
    assert_eq!(scene.layers[1].role, LayerRole::Outfit);
    assert_eq!(scene.layers[1].asset_id, coat);
    assert_eq!(scene.layers[2].role, LayerRole::Outfit);
    assert_eq!(scene.layers[2].asset_id, boots);
    assert!(scene.layers[1].z_index < scene.layers[2].z_index);

    let expression = scene.expression.clone().expect("expression");
    assert_eq!(expression.name, "happy");
    assert_eq!(expression.asset_id, happy);
    let gesture = scene.gesture.clone().expect("gesture");
    assert_eq!(gesture.name, "wave");
    assert_eq!(gesture.asset_id, wave);

    let again = AvatarScene::compose(&appearance, &assets, Some("happy"), Some("wave"))
        .expect("compose again");
    assert_eq!(scene, again);
}

#[test]
fn default_appearance_composes_an_empty_scene() {
    let (_temp, root) = setup();
    let store = open_store(&root, CORE_A);
    let appearance = store.appearance().expect("appearance");
    let assets = store.list_assets(None, false).expect("assets");
    let scene = AvatarScene::compose(&appearance, &assets, None, None).expect("compose");
    assert!(scene.layers.is_empty());
    assert!(scene.expression.is_none());
    assert!(scene.gesture.is_none());
    assert_eq!(scene.theme.primary, AEGIS_PRIMARY);
    assert_eq!(scene.theme.accent, AEGIS_ACCENT);
    assert_eq!(scene.theme.tertiary, AEGIS_TERTIARY);
}

#[test]
fn compose_rejects_wrong_asset_kind() {
    let (_temp, root) = setup();
    let store = open_store(&root, CORE_A);
    let expression = import(&store, AssetKind::Expression, "happy.bin", b"happy");
    let mut appearance = store.appearance().expect("appearance");
    appearance.avatar_asset_id = Some(expression);
    let assets = store.list_assets(None, false).expect("assets");
    let error = AvatarScene::compose(&appearance, &assets, None, None).expect_err("wrong kind");
    assert!(matches!(error, EmbodimentError::WrongAssetKind(_)));

    let gesture = import(&store, AssetKind::Gesture, "wave.bin", b"wave");
    let mut appearance = store.appearance().expect("appearance");
    appearance
        .expressions
        .insert("bad".to_owned(), gesture.clone());
    let assets = store.list_assets(None, false).expect("assets");
    let error = AvatarScene::compose(&appearance, &assets, Some("bad"), None)
        .expect_err("wrong expression kind");
    assert!(matches!(error, EmbodimentError::WrongAssetKind(_)));

    appearance
        .expressions
        .insert("missing".to_owned(), "unknown-asset".to_owned());
    let error = AvatarScene::compose(&appearance, &assets, Some("missing"), None)
        .expect_err("missing expression asset");
    assert!(matches!(error, EmbodimentError::NotFound));
}

#[test]
fn compose_rejects_assets_owned_by_another_core() {
    let (_temp, root) = setup();
    let store_a = open_store(&root, CORE_A);
    let store_b = open_store(&root, CORE_B);
    let foreign = import(&store_b, AssetKind::Avatar, "b.png", b"b-avatar");
    let mut appearance = store_a.appearance().expect("appearance a");
    appearance.avatar_asset_id = Some(foreign.clone());

    let foreign_assets = store_b.list_assets(None, false).expect("b assets");
    let error = AvatarScene::compose(&appearance, &foreign_assets, None, None)
        .expect_err("foreign ownership");
    assert!(matches!(error, EmbodimentError::OwnershipMismatch));

    let own_assets = store_a.list_assets(None, false).expect("a assets");
    let error =
        AvatarScene::compose(&appearance, &own_assets, None, None).expect_err("unresolved asset");
    assert!(matches!(error, EmbodimentError::NotFound));
}

#[test]
fn scene_constructor_validates_structure() {
    let theme = Theme::default();
    let error =
        AvatarScene::new("", theme.clone(), Vec::new(), None, None).expect_err("empty core");
    assert!(matches!(error, EmbodimentError::InvalidInput(_)));

    let invalid_theme = Theme {
        primary: "purple".to_owned(),
        ..Theme::default()
    };
    let error =
        AvatarScene::new(CORE_A, invalid_theme, Vec::new(), None, None).expect_err("invalid theme");
    assert!(matches!(error, EmbodimentError::InvalidInput(_)));

    let duplicate = vec![
        layer(LayerRole::Base, "avatar-a", 0),
        layer(LayerRole::Base, "avatar-b", 0),
    ];
    let error =
        AvatarScene::new(CORE_A, theme.clone(), duplicate, None, None).expect_err("duplicate base");
    assert!(matches!(error, EmbodimentError::InvalidInput(_)));

    let motion = ActiveMotion::new("", "avatar-a").expect_err("empty name");
    assert!(matches!(motion, EmbodimentError::InvalidInput(_)));

    let scene = AvatarScene::new(
        CORE_A,
        theme,
        vec![
            layer(LayerRole::Overlay, "overlay", 100),
            layer(LayerRole::Base, "avatar", 0),
            layer(LayerRole::Eyes, "eyes", 30),
        ],
        Some(ActiveMotion::new("happy", "happy-asset").expect("motion")),
        None,
    )
    .expect("scene");
    assert_eq!(scene.layers[0].role, LayerRole::Base);
    assert_eq!(scene.layers[1].role, LayerRole::Eyes);
    assert_eq!(scene.layers[2].role, LayerRole::Overlay);
}

#[test]
fn two_cores_compose_isolated_scenes() {
    let (_temp, root) = setup();
    let store_a = open_store(&root, CORE_A);
    let store_b = open_store(&root, CORE_B);

    let avatar_a = import(&store_a, AssetKind::Avatar, "a.png", b"a-avatar");
    store_a.set_avatar(Some(&avatar_a)).expect("avatar a");
    store_a
        .set_theme(Theme {
            primary: "#010203".to_owned(),
            accent: "#040506".to_owned(),
            tertiary: "#070809".to_owned(),
        })
        .expect("theme a");

    let avatar_b = import(&store_b, AssetKind::Avatar, "b.png", b"b-avatar");
    let dress = import(&store_b, AssetKind::Wardrobe, "dress.png", b"dress");
    store_b.set_avatar(Some(&avatar_b)).expect("avatar b");
    store_b
        .set_wardrobe(vec![dress.clone()])
        .expect("wardrobe b");
    store_b.set_theme(custom_theme()).expect("theme b");

    let scene_a = AvatarScene::compose(
        &store_a.appearance().expect("a"),
        &store_a.list_assets(None, false).expect("a assets"),
        None,
        None,
    )
    .expect("scene a");
    let scene_b = AvatarScene::compose(
        &store_b.appearance().expect("b"),
        &store_b.list_assets(None, false).expect("b assets"),
        None,
        None,
    )
    .expect("scene b");

    assert_eq!(scene_a.core_uuid, CORE_A);
    assert_eq!(scene_b.core_uuid, CORE_B);
    assert_ne!(scene_a.theme, scene_b.theme);
    assert_eq!(scene_a.layers.len(), 1);
    assert_eq!(scene_b.layers.len(), 2);
    assert_eq!(scene_a.layers[0].asset_id, avatar_a);
    assert_eq!(scene_b.layers[0].asset_id, avatar_b);
    assert!(scene_b.layers.iter().any(|entry| entry.asset_id == dress));
    assert!(!scene_a.layers.iter().any(|entry| entry.asset_id == dress));

    let cross = AvatarScene::compose(
        &store_a.appearance().expect("a"),
        &store_b.list_assets(None, false).expect("b assets"),
        None,
        None,
    );
    assert!(cross.is_err());
}
