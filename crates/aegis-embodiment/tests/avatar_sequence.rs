use std::collections::HashSet;

use aegis_embodiment::{
    render_sequence, sha256_hex, write_sequence, ActiveMotion, AnimationClip, AssetKind,
    AvatarScene, Easing, Keyframe, LayerRole, PlaybackMode, SceneLayer, Theme, Transform,
    BMP_MAGIC,
};

const SYNTHETIC_CORE: &str = "00000000-0000-4000-8000-0000000000aa";

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

fn scene(expression: Option<ActiveMotion>) -> AvatarScene {
    AvatarScene::new(
        SYNTHETIC_CORE,
        Theme::default(),
        vec![
            layer(LayerRole::Base, "synthetic-base", 0),
            layer(LayerRole::Outfit, "synthetic-outfit", 10),
            layer(LayerRole::Hair, "synthetic-hair", 20),
            layer(LayerRole::Eyes, "synthetic-eyes", 30),
        ],
        expression,
        None,
    )
    .expect("synthetic scene")
}

fn movement_clip() -> AnimationClip {
    AnimationClip::new(
        "approved-style-sway",
        500,
        PlaybackMode::Loop,
        vec![
            Keyframe::new(
                0,
                Transform {
                    translate_x: -0.10,
                    ..Transform::identity()
                },
                Easing::EaseInOut,
            ),
            Keyframe::new(
                250,
                Transform {
                    translate_x: 0.10,
                    ..Transform::identity()
                },
                Easing::EaseInOut,
            ),
            Keyframe::new(
                500,
                Transform {
                    translate_x: -0.10,
                    ..Transform::identity()
                },
                Easing::EaseInOut,
            ),
        ],
    )
    .expect("movement clip")
}

fn declared_bmp_size(bytes: &[u8]) -> usize {
    u32::from_le_bytes([bytes[2], bytes[3], bytes[4], bytes[5]]) as usize
}

#[test]
fn live_avatar_sequence() {
    if std::env::var("AEGIS_AVATAR_LIVE").ok().as_deref() != Some("1") {
        eprintln!("AEGIS_AVATAR_LIVE not set to 1; skipping live avatar sequence test");
        return;
    }

    let clip = movement_clip();
    let neutral_scene = scene(None);
    let happy_scene = scene(Some(
        ActiveMotion::new("happy", "synthetic-expression").expect("expression"),
    ));

    let neutral =
        render_sequence(&neutral_scene, &clip, 128, 160, 500, 12).expect("neutral sequence");
    let happy = render_sequence(&happy_scene, &clip, 128, 160, 500, 12).expect("happy sequence");
    assert!(neutral.len() >= 3, "expected at least 3 frames");
    assert_eq!(neutral.len(), happy.len());

    for frame in neutral.iter().chain(happy.iter()) {
        assert!(frame.bmp.starts_with(&BMP_MAGIC));
        assert_eq!(declared_bmp_size(&frame.bmp), frame.bmp.len());
        assert_eq!(frame.sha256, sha256_hex(&frame.bmp));
    }

    let neutral_hashes: HashSet<&str> = neutral.iter().map(|frame| frame.sha256.as_str()).collect();
    let happy_hashes: HashSet<&str> = happy.iter().map(|frame| frame.sha256.as_str()).collect();
    assert!(neutral_hashes.len() > 1, "movement must change frames");
    assert!(happy_hashes.len() > 1, "movement must change frames");
    assert_ne!(neutral[0].sha256, happy[0].sha256, "expression must differ");

    let temp = tempfile::tempdir().expect("tempdir");
    let neutral_manifest = write_sequence(temp.path().join("neutral"), &neutral, 12, 500, 128, 160)
        .expect("write neutral");
    let happy_manifest =
        write_sequence(temp.path().join("happy"), &happy, 12, 500, 128, 160).expect("write happy");

    assert_eq!(neutral_manifest.frame_count, neutral.len());
    assert_eq!(happy_manifest.frame_count, happy.len());
    assert_eq!(
        neutral_manifest.total_bytes,
        neutral.iter().map(|frame| frame.bmp.len()).sum::<usize>()
    );
    assert!(neutral.len() >= 3);

    println!(
        "live avatar sequence: neutral_frames={} neutral_bytes={} happy_frames={} happy_bytes={}",
        neutral.len(),
        neutral_manifest.total_bytes,
        happy.len(),
        happy_manifest.total_bytes
    );
    for frame in neutral.iter().take(2) {
        println!(
            "  neutral frame {} time_ms={} bytes={} sha256={}",
            frame.index,
            frame.time_ms,
            frame.bmp.len(),
            frame.sha256
        );
    }
    for frame in happy.iter().take(2) {
        println!(
            "  happy frame {} time_ms={} bytes={} sha256={}",
            frame.index,
            frame.time_ms,
            frame.bmp.len(),
            frame.sha256
        );
    }
}
