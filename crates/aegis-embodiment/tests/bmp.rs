use std::path::{Path, PathBuf};

use aegis_contracts::initialization::{CoreInitializationRequest, CORE_INIT_REQUEST_SCHEMA};
use aegis_core_install::CoreInstaller;
use aegis_embodiment::{
    fill_rect, render_bmp, render_bmp_at, sha256_hex, write_bmp, ActiveMotion, AnimationClip,
    AnimationPlayer, AssetKind, AvatarScene, Easing, EmbodimentError, EmbodimentStore, Keyframe,
    LayerRole, NewAsset, PlaybackMode, Rect, SceneLayer, Theme, Transform, BMP_HEADER_BYTES,
};
use aegis_platform::PlatformService;
use aegis_storage::now_rfc3339;

const SYNTHETIC_CORE: &str = "00000000-0000-4000-8000-0000000000aa";

fn theme(primary: &str, accent: &str, tertiary: &str) -> Theme {
    Theme {
        primary: primary.to_owned(),
        accent: accent.to_owned(),
        tertiary: tertiary.to_owned(),
    }
}

fn layer(role: LayerRole, asset_id: &str, z_index: i32) -> SceneLayer {
    SceneLayer {
        role,
        asset_id: asset_id.to_owned(),
        asset_kind: AssetKind::Avatar,
        media_type: "image/png".to_owned(),
        sha256: "synthetic-digest".to_owned(),
        visible: true,
        z_index,
    }
}

fn base_scene(theme: Theme) -> AvatarScene {
    AvatarScene::new(
        SYNTHETIC_CORE,
        theme,
        vec![layer(LayerRole::Base, "synthetic-base", 0)],
        None,
        None,
    )
    .expect("base scene")
}

fn full_scene(theme: Theme, with_motion: bool) -> AvatarScene {
    let expression = with_motion.then(|| {
        ActiveMotion::new("synthetic-expression", "synthetic-expression-asset").expect("expression")
    });
    let gesture = with_motion.then(|| {
        ActiveMotion::new("synthetic-gesture", "synthetic-gesture-asset").expect("gesture")
    });
    AvatarScene::new(
        SYNTHETIC_CORE,
        theme,
        vec![
            layer(LayerRole::Base, "synthetic-base", 0),
            layer(LayerRole::Outfit, "synthetic-outfit", 10),
            layer(LayerRole::Hair, "synthetic-hair", 20),
            layer(LayerRole::Eyes, "synthetic-eyes", 30),
            layer(LayerRole::Overlay, "synthetic-overlay", 100),
        ],
        expression,
        gesture,
    )
    .expect("full scene")
}

fn read_u32(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes([
        bytes[offset],
        bytes[offset + 1],
        bytes[offset + 2],
        bytes[offset + 3],
    ])
}

fn read_u16(bytes: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes([bytes[offset], bytes[offset + 1]])
}

fn read_i32(bytes: &[u8], offset: usize) -> i32 {
    i32::from_le_bytes([
        bytes[offset],
        bytes[offset + 1],
        bytes[offset + 2],
        bytes[offset + 3],
    ])
}

fn setup() -> (tempfile::TempDir, PathBuf) {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path().join("aegis-root");
    let initialized = PlatformService::initialize(&root).expect("platform");
    let installer = CoreInstaller::new(&root).expect("installer");
    for (uuid, number, name) in [
        ("22222222-2222-4222-8222-222222222222", 1, "Render Alpha"),
        ("33333333-3333-4333-8333-333333333333", 2, "Render Beta"),
    ] {
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
fn bmp_header_and_size_are_valid() {
    let scene = full_scene(theme("#9400D3", "#00C2FF", "#39FF14"), false);
    let bytes = render_bmp(&scene, 64, 48).expect("render");

    assert!(bytes.starts_with(b"BM"));
    assert_eq!(read_u32(&bytes, 2) as usize, bytes.len());
    assert_eq!(read_u32(&bytes, 10) as usize, BMP_HEADER_BYTES);
    assert_eq!(read_u32(&bytes, 14), 40);
    assert_eq!(read_i32(&bytes, 18), 64);
    assert_eq!(read_i32(&bytes, 22), 48);
    assert_eq!(read_u16(&bytes, 26), 1);
    assert_eq!(read_u16(&bytes, 28), 24);
    assert_eq!(read_u32(&bytes, 30), 0);

    let row_bytes = 64usize * 3;
    let stride = row_bytes.div_ceil(4) * 4;
    let pixel_bytes = stride * 48;
    assert_eq!(read_u32(&bytes, 34) as usize, pixel_bytes);
    assert_eq!(bytes.len(), BMP_HEADER_BYTES + pixel_bytes);
}

#[test]
fn bmp_rows_are_stored_bottom_up() {
    let scene = base_scene(theme("#102030", "#405060", "#708090"));
    let bytes = render_bmp(&scene, 16, 16).expect("render");

    let stride = 16 * 3;
    let data = BMP_HEADER_BYTES;
    let first_row_center = data + 8 * 3;
    let last_row_center = data + 15 * stride + 8 * 3;

    assert_eq!(
        &bytes[first_row_center..first_row_center + 3],
        &[48, 32, 16]
    );
    assert_eq!(&bytes[last_row_center..last_row_center + 3], &[7, 5, 2]);
    assert_eq!(&bytes[data..data + 3], &[7, 5, 2]);
}

#[test]
fn bmp_pads_rows_to_four_byte_stride() {
    let scene = base_scene(theme("#102030", "#405060", "#708090"));
    let bytes = render_bmp(&scene, 17, 16).expect("render");

    let row_bytes = 17 * 3;
    let stride = 52;
    let pixel_bytes = stride * 16;
    assert_eq!(stride - row_bytes, 1);
    assert_eq!(read_u32(&bytes, 2) as usize, bytes.len());
    assert_eq!(read_u32(&bytes, 34) as usize, pixel_bytes);
    assert_eq!(bytes.len(), BMP_HEADER_BYTES + pixel_bytes);

    for row in 0..16 {
        let padding_offset = BMP_HEADER_BYTES + row * stride + row_bytes;
        assert_eq!(bytes[padding_offset], 0);
    }
}

#[test]
fn bmp_rendering_is_deterministic() {
    let scene = full_scene(theme("#9400D3", "#00C2FF", "#39FF14"), true);
    let first = render_bmp(&scene, 96, 128).expect("first");
    let second = render_bmp(&scene, 96, 128).expect("second");
    assert_eq!(first, second);
}

#[test]
fn different_themes_produce_different_pixels() {
    let scene_a = full_scene(theme("#101010", "#202020", "#303030"), false);
    let scene_b = full_scene(theme("#a0b0c0", "#d0e0f0", "#f0a0b0"), false);
    let bytes_a = render_bmp(&scene_a, 64, 64).expect("a");
    let bytes_b = render_bmp(&scene_b, 64, 64).expect("b");

    assert_eq!(bytes_a.len(), bytes_b.len());
    assert_ne!(bytes_a, bytes_b);
    assert_ne!(&bytes_a[BMP_HEADER_BYTES..], &bytes_b[BMP_HEADER_BYTES..]);
}

#[test]
fn expression_and_gesture_overlays_change_pixels() {
    let theme = theme("#9400D3", "#00C2FF", "#39FF14");
    let plain = full_scene(theme.clone(), false);
    let animated = full_scene(theme, true);
    assert_ne!(
        render_bmp(&plain, 128, 160).expect("plain"),
        render_bmp(&animated, 128, 160).expect("animated")
    );
}

#[test]
fn animation_frames_produce_different_pixels() {
    let scene = full_scene(theme("#9400D3", "#00C2FF", "#39FF14"), true);
    let clip = AnimationClip::new(
        "walk",
        1000,
        PlaybackMode::Loop,
        vec![
            Keyframe::new(
                0,
                Transform {
                    translate_x: 0.0,
                    ..Transform::identity()
                },
                Easing::Linear,
            ),
            Keyframe::new(
                1000,
                Transform {
                    translate_x: 0.25,
                    ..Transform::identity()
                },
                Easing::Linear,
            ),
        ],
    )
    .expect("clip");
    let mut player = AnimationPlayer::new(clip);

    let start = player.seek(0);
    let middle = player.seek(500);
    assert_eq!(start.time_ms, 0);
    assert_eq!(middle.time_ms, 500);

    let frame_start = render_bmp_at(&scene, 128, 160, Some(&start)).expect("start frame");
    let frame_middle = render_bmp_at(&scene, 128, 160, Some(&middle)).expect("middle frame");
    assert_eq!(frame_start.len(), frame_middle.len());
    assert_ne!(frame_start, frame_middle);
}

#[test]
fn fill_rect_clips_to_canvas_edges() {
    let width = 12;
    let height = 12;
    let background = [9u8, 9, 9];
    let foreground = [255u8, 0, 0];
    let mut pixels = vec![9u8; width * height * 3];

    fill_rect(
        &mut pixels,
        width as u32,
        height as u32,
        Rect::new(-4, -4, 8, 8),
        foreground,
    )
    .expect("top-left clip");
    fill_rect(
        &mut pixels,
        width as u32,
        height as u32,
        Rect::new(8, 8, 8, 8),
        foreground,
    )
    .expect("bottom-right clip");
    fill_rect(
        &mut pixels,
        width as u32,
        height as u32,
        Rect::new(-10, -10, 5, 5),
        foreground,
    )
    .expect("outside clip");
    fill_rect(
        &mut pixels,
        width as u32,
        height as u32,
        Rect::new(20, 20, 5, 5),
        foreground,
    )
    .expect("far outside clip");

    for y in 0..height {
        for x in 0..width {
            let offset = (y * width + x) * 3;
            let painted = (x < 4 && y < 4) || (x >= 8 && y >= 8);
            let expected = if painted { foreground } else { background };
            assert_eq!(&pixels[offset..offset + 3], &expected, "pixel ({x},{y})");
        }
    }
}

#[test]
fn render_rejects_invalid_dimensions_and_empty_scenes() {
    let scene = base_scene(theme("#102030", "#405060", "#708090"));
    for (width, height) in [(0, 16), (15, 16), (2049, 16), (16, 0), (16, 2049)] {
        let error = render_bmp(&scene, width, height).expect_err("dimensions");
        assert!(matches!(error, EmbodimentError::InvalidInput(_)));
    }

    let empty = AvatarScene::new(
        SYNTHETIC_CORE,
        theme("#102030", "#405060", "#708090"),
        Vec::new(),
        None,
        None,
    )
    .expect("empty scene");
    let error = render_bmp(&empty, 32, 32).expect_err("empty layers");
    assert!(matches!(error, EmbodimentError::InvalidInput(_)));
}

#[test]
fn per_core_themes_render_differently_without_contamination() {
    let (_temp, root) = setup();
    let core_a = "22222222-2222-4222-8222-222222222222";
    let core_b = "33333333-3333-4333-8333-333333333333";
    let store_a = open_store(&root, core_a);
    let store_b = open_store(&root, core_b);

    let avatar_a = import(&store_a, AssetKind::Avatar, "a.png", b"a-avatar");
    store_a.set_avatar(Some(&avatar_a)).expect("avatar a");
    store_a
        .set_theme(theme("#101010", "#202020", "#303030"))
        .expect("theme a");

    let avatar_b = import(&store_b, AssetKind::Avatar, "b.png", b"b-avatar");
    let dress = import(&store_b, AssetKind::Wardrobe, "dress.png", b"dress");
    store_b.set_avatar(Some(&avatar_b)).expect("avatar b");
    store_b
        .set_wardrobe(vec![dress.clone()])
        .expect("wardrobe b");
    store_b
        .set_theme(theme("#a0b0c0", "#d0e0f0", "#f0a0b0"))
        .expect("theme b");

    let scene_a = AvatarScene::compose(
        &store_a.appearance().expect("appearance a"),
        &store_a.list_assets(None, false).expect("assets a"),
        None,
        None,
    )
    .expect("scene a");
    let scene_b = AvatarScene::compose(
        &store_b.appearance().expect("appearance b"),
        &store_b.list_assets(None, false).expect("assets b"),
        None,
        None,
    )
    .expect("scene b");

    assert_eq!(scene_a.core_uuid, core_a);
    assert_eq!(scene_b.core_uuid, core_b);
    assert_ne!(scene_a.theme, scene_b.theme);
    assert!(scene_a.layers.iter().all(|layer| layer.asset_id != dress));
    assert!(scene_b.layers.iter().any(|layer| layer.asset_id == dress));

    let bytes_a = render_bmp(&scene_a, 64, 64).expect("render a");
    let bytes_b = render_bmp(&scene_b, 64, 64).expect("render b");
    assert_ne!(bytes_a, bytes_b);

    let mut foreign_appearance = store_a.appearance().expect("appearance a");
    foreign_appearance.avatar_asset_id = Some(avatar_b.clone());
    let foreign = AvatarScene::compose(
        &foreign_appearance,
        &store_b.list_assets(None, false).expect("assets b"),
        None,
        None,
    )
    .expect_err("foreign assets");
    assert!(matches!(foreign, EmbodimentError::OwnershipMismatch));

    let missing = AvatarScene::compose(
        &store_a.appearance().expect("appearance a"),
        &store_b.list_assets(None, false).expect("assets b"),
        None,
        None,
    )
    .expect_err("unresolved asset");
    assert!(matches!(missing, EmbodimentError::NotFound));

    let expression = import(&store_a, AssetKind::Expression, "smile.bin", b"smile");
    let mut wrong_kind = store_a.appearance().expect("appearance a");
    wrong_kind.avatar_asset_id = Some(expression);
    let wrong_kind = AvatarScene::compose(
        &wrong_kind,
        &store_a.list_assets(None, false).expect("assets a"),
        None,
        None,
    )
    .expect_err("wrong kind");
    assert!(matches!(wrong_kind, EmbodimentError::WrongAssetKind(_)));
}

#[test]
fn write_bmp_validates_and_writes_file() {
    let scene = base_scene(theme("#102030", "#405060", "#708090"));
    let bytes = render_bmp(&scene, 32, 32).expect("render");
    let temp = tempfile::tempdir().expect("tempdir");
    let path = temp.path().join("avatar.bmp");
    write_bmp(&path, &bytes).expect("write");

    let on_disk = std::fs::read(&path).expect("read");
    assert_eq!(on_disk, bytes);

    let truncated = &bytes[..BMP_HEADER_BYTES - 1];
    assert!(matches!(
        write_bmp(&path, truncated).expect_err("truncated"),
        EmbodimentError::InvalidInput(_)
    ));

    let mut corrupt = bytes.clone();
    corrupt[2] = corrupt[2].wrapping_add(1);
    assert!(matches!(
        write_bmp(&path, &corrupt).expect_err("size mismatch"),
        EmbodimentError::CorruptData(_)
    ));
}

#[test]
fn live_avatar_writes_bmp_file() {
    if std::env::var("AEGIS_AVATAR_LIVE").ok().as_deref() != Some("1") {
        eprintln!("AEGIS_AVATAR_LIVE not set to 1; skipping live avatar BMP test");
        return;
    }

    let scene = full_scene(Theme::default(), true);
    let bytes = render_bmp(&scene, 128, 160).expect("render");
    let temp = tempfile::tempdir().expect("tempdir");
    let path = temp.path().join("synthetic-avatar.bmp");
    write_bmp(&path, &bytes).expect("write");

    let on_disk = std::fs::read(&path).expect("read");
    assert!(!on_disk.is_empty());
    assert!(on_disk.starts_with(b"BM"));
    assert_eq!(read_u32(&on_disk, 2) as usize, on_disk.len());
    assert_eq!(on_disk, bytes);

    let sha = sha256_hex(&on_disk);
    assert_eq!(sha.len(), 64);
    println!(
        "live avatar BMP: path={} bytes={} sha256={}",
        path.display(),
        on_disk.len(),
        sha
    );
}
