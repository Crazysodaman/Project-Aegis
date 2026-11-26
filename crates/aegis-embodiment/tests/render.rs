use std::path::{Path, PathBuf};

use aegis_contracts::initialization::{CoreInitializationRequest, CORE_INIT_REQUEST_SCHEMA};
use aegis_core_install::CoreInstaller;
use aegis_embodiment::{
    AnimationClip, AnimationPlayer, AssetKind, AvatarRenderer, AvatarScene, Easing,
    EmbodimentStore, HeadlessSceneRenderer, Keyframe, NewAsset, PlaybackMode, SceneRenderer, Theme,
    Transform, HEADLESS_RENDERER, SCENE_DESCRIPTION_SCHEMA, SCENE_SCHEMA,
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

fn fade_player() -> AnimationPlayer {
    let clip = AnimationClip::new(
        "fade",
        1000,
        PlaybackMode::Loop,
        vec![
            Keyframe::new(
                0,
                Transform {
                    opacity: 0.0,
                    ..Transform::identity()
                },
                Easing::Linear,
            ),
            Keyframe::new(
                1000,
                Transform {
                    opacity: 1.0,
                    ..Transform::identity()
                },
                Easing::Linear,
            ),
        ],
    )
    .expect("clip");
    AnimationPlayer::new(clip)
}

fn compose_for(store: &EmbodimentStore) -> AvatarScene {
    AvatarScene::compose(
        &store.appearance().expect("appearance"),
        &store.list_assets(None, false).expect("assets"),
        None,
        None,
    )
    .expect("scene")
}

#[test]
fn headless_render_is_deterministic_and_explicitly_non_pixel() {
    let (_temp, root) = setup();
    let store = open_store(&root, CORE_A);
    let avatar = import(&store, AssetKind::Avatar, "base.png", b"avatar");
    let coat = import(&store, AssetKind::Wardrobe, "coat.png", b"coat");
    store.set_avatar(Some(&avatar)).expect("avatar");
    store.set_wardrobe(vec![coat.clone()]).expect("wardrobe");
    store
        .set_theme(Theme {
            primary: "#112233".to_owned(),
            accent: "#445566".to_owned(),
            tertiary: "#778899".to_owned(),
        })
        .expect("theme");

    let scene = compose_for(&store);
    let mut player = fade_player();
    let state = player.advance(250);

    let renderer = HeadlessSceneRenderer::new();
    let first = renderer.render(&scene, Some(&state)).expect("render first");
    let second = renderer
        .render(&scene, Some(&state))
        .expect("render second");
    assert_eq!(first, second);

    let first_json = first.to_json().expect("json");
    let second_json = second.to_json().expect("json");
    assert_eq!(first_json, second_json);
    assert!(first_json.contains(SCENE_DESCRIPTION_SCHEMA));
    assert!(first_json.contains(SCENE_SCHEMA));
    assert!(!first_json.contains("NaN"));
    assert!(!first_json.contains("inf"));

    assert_eq!(first.schema, SCENE_DESCRIPTION_SCHEMA);
    assert_eq!(first.renderer, HEADLESS_RENDERER);
    assert_eq!(first.renderer, renderer.name());
    assert_eq!(first.core_uuid, CORE_A);
    assert_eq!(first.layers.len(), 2);
    assert_eq!(first.layers[0].asset_id, avatar);
    assert_eq!(first.layers[1].asset_id, coat);
    let animation = first.animation.as_ref().expect("animation");
    assert_eq!(animation.clip_name, "fade");
    assert_eq!(animation.time_ms, 250);
    assert!(!animation.finished);
    assert!(!first.rasterized);
    assert!(!first.gpu_used);
    assert!(!first.windowed);

    let as_avatar: &dyn AvatarRenderer = &renderer;
    let via_trait = as_avatar
        .render(&scene, Some(&state))
        .expect("trait render");
    assert_eq!(via_trait, first);
}

#[test]
fn headless_render_without_animation_has_no_pixels_and_empty_layers() {
    let (_temp, root) = setup();
    let store = open_store(&root, CORE_A);
    let scene = compose_for(&store);
    let renderer = HeadlessSceneRenderer::new();
    let description = renderer.render(&scene, None).expect("render");
    assert!(description.layers.is_empty());
    assert!(description.animation.is_none());
    assert!(description.expression.is_none());
    assert!(description.gesture.is_none());
    assert!(!description.rasterized);
    assert!(!description.gpu_used);
    assert!(!description.windowed);
    let json = description.to_json_pretty().expect("pretty json");
    assert_eq!(
        json,
        renderer
            .render(&scene, None)
            .expect("again")
            .to_json_pretty()
            .expect("pretty again")
    );
}

#[test]
fn render_preserves_per_core_boundaries() {
    let (_temp, root) = setup();
    let store_a = open_store(&root, CORE_A);
    let store_b = open_store(&root, CORE_B);

    let avatar_a = import(&store_a, AssetKind::Avatar, "a.png", b"a-avatar");
    store_a.set_avatar(Some(&avatar_a)).expect("avatar a");
    store_a
        .set_theme(Theme {
            primary: "#010101".to_owned(),
            accent: "#020202".to_owned(),
            tertiary: "#030303".to_owned(),
        })
        .expect("theme a");

    let avatar_b = import(&store_b, AssetKind::Avatar, "b.png", b"b-avatar");
    let dress = import(&store_b, AssetKind::Wardrobe, "dress.png", b"dress");
    store_b.set_avatar(Some(&avatar_b)).expect("avatar b");
    store_b
        .set_wardrobe(vec![dress.clone()])
        .expect("wardrobe b");
    store_b
        .set_theme(Theme {
            primary: "#aaaaaa".to_owned(),
            accent: "#bbbbbb".to_owned(),
            tertiary: "#cccccc".to_owned(),
        })
        .expect("theme b");

    let renderer = HeadlessSceneRenderer::new();
    let description_a = renderer
        .render(&compose_for(&store_a), None)
        .expect("render a");
    let description_b = renderer
        .render(&compose_for(&store_b), None)
        .expect("render b");

    assert_eq!(description_a.core_uuid, CORE_A);
    assert_eq!(description_b.core_uuid, CORE_B);
    assert_ne!(description_a.theme, description_b.theme);
    assert_eq!(description_a.layers.len(), 1);
    assert_eq!(description_b.layers.len(), 2);
    assert_eq!(description_a.layers[0].asset_id, avatar_a);
    assert_eq!(description_b.layers[0].asset_id, avatar_b);
    assert!(!description_a
        .layers
        .iter()
        .any(|layer| layer.asset_id == dress));
    assert_ne!(
        description_a.to_json().expect("a json"),
        description_b.to_json().expect("b json")
    );
}
