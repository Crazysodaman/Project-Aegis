use std::collections::HashSet;

use aegis_embodiment::{
    frame_time_ms, render_sequence, sequence_frame_count, sha256_hex, write_sequence,
    write_sequence_capped, ActiveMotion, AnimationClip, AssetKind, AvatarScene, Easing,
    EmbodimentError, Keyframe, LayerRole, PlaybackMode, SceneLayer, Theme, Transform, BMP_MAGIC,
    MAX_SEQUENCE_DURATION_MS, SEQUENCE_MANIFEST_FILE, SEQUENCE_MANIFEST_SCHEMA,
};

const SYNTHETIC_CORE: &str = "00000000-0000-4000-8000-0000000000aa";

fn theme() -> Theme {
    Theme {
        primary: "#9400D3".to_owned(),
        accent: "#00C2FF".to_owned(),
        tertiary: "#39FF14".to_owned(),
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

fn scene() -> AvatarScene {
    AvatarScene::new(
        SYNTHETIC_CORE,
        theme(),
        vec![
            layer(LayerRole::Base, "synthetic-base", 0),
            layer(LayerRole::Outfit, "synthetic-outfit", 10),
            layer(LayerRole::Hair, "synthetic-hair", 20),
            layer(LayerRole::Eyes, "synthetic-eyes", 30),
        ],
        None,
        None,
    )
    .expect("scene")
}

fn expression_scene() -> AvatarScene {
    AvatarScene::new(
        SYNTHETIC_CORE,
        theme(),
        vec![layer(LayerRole::Base, "synthetic-base", 0)],
        Some(ActiveMotion::new("happy", "synthetic-expression").expect("expression")),
        None,
    )
    .expect("expression scene")
}

fn movement_clip() -> AnimationClip {
    AnimationClip::new(
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
    .expect("movement clip")
}

fn static_clip() -> AnimationClip {
    AnimationClip::new(
        "still",
        1000,
        PlaybackMode::Once,
        vec![
            Keyframe::new(0, Transform::identity(), Easing::Linear),
            Keyframe::new(1000, Transform::identity(), Easing::Linear),
        ],
    )
    .expect("static clip")
}

fn declared_bmp_size(bytes: &[u8]) -> usize {
    u32::from_le_bytes([bytes[2], bytes[3], bytes[4], bytes[5]]) as usize
}

#[test]
fn frame_count_and_times_are_deterministic() {
    assert_eq!(sequence_frame_count(1000, 10).expect("10fps"), 10);
    assert_eq!(sequence_frame_count(500, 12).expect("12fps"), 6);
    assert_eq!(sequence_frame_count(1000, 24).expect("24fps"), 24);
    assert_eq!(sequence_frame_count(100, 30).expect("ceil"), 3);
    assert_eq!(frame_time_ms(0, 10), 0);
    assert_eq!(frame_time_ms(1, 10), 100);
    assert_eq!(frame_time_ms(10, 12), 833);
}

#[test]
fn invalid_timing_bounds_are_rejected() {
    for (duration, fps) in [
        (0, 10),
        (MAX_SEQUENCE_DURATION_MS + 1, 10),
        (1000, 0),
        (1000, 121),
    ] {
        assert!(matches!(
            sequence_frame_count(duration, fps),
            Err(EmbodimentError::InvalidInput(_))
        ));
    }
    assert!(matches!(
        sequence_frame_count(MAX_SEQUENCE_DURATION_MS, 120),
        Err(EmbodimentError::InvalidInput(_))
    ));
}

#[test]
fn movement_clip_produces_differing_frames() {
    let frames = render_sequence(&scene(), &movement_clip(), 96, 128, 1000, 10).expect("sequence");
    assert_eq!(frames.len(), 10);
    assert_eq!(frames.first().expect("first").time_ms, 0);
    assert_eq!(frames.last().expect("last").time_ms, 900);
    assert_ne!(frames[0].bmp, frames[9].bmp);

    let unique: HashSet<&str> = frames.iter().map(|frame| frame.sha256.as_str()).collect();
    assert!(unique.len() > 1);
}

#[test]
fn static_clip_produces_identical_frames() {
    let frames = render_sequence(&scene(), &static_clip(), 96, 128, 1000, 10).expect("sequence");
    assert_eq!(frames.len(), 10);
    let first = &frames[0];
    for (index, frame) in frames.iter().enumerate() {
        assert_eq!(frame.index, index);
        assert_eq!(frame.bmp, first.bmp);
        assert_eq!(frame.sha256, first.sha256);
    }
}

#[test]
fn each_frame_digest_matches_its_bmp() {
    let frames =
        render_sequence(&expression_scene(), &movement_clip(), 64, 80, 400, 8).expect("sequence");
    assert_eq!(frames.len(), 4);
    for frame in &frames {
        assert!(frame.bmp.starts_with(&BMP_MAGIC));
        assert_eq!(declared_bmp_size(&frame.bmp), frame.bmp.len());
        assert_eq!(frame.sha256.len(), 64);
        assert_eq!(frame.sha256, sha256_hex(&frame.bmp));
    }
}

#[test]
fn writer_persists_frames_and_manifest() {
    let frames = render_sequence(&scene(), &static_clip(), 64, 64, 500, 10).expect("sequence");
    assert_eq!(frames.len(), 5);
    let temp = tempfile::tempdir().expect("tempdir");
    let out = temp.path().join("sequence");
    let manifest = write_sequence(&out, &frames, 10, 500, 64, 64).expect("write");

    assert_eq!(manifest.schema, SEQUENCE_MANIFEST_SCHEMA);
    assert_eq!(manifest.frame_count, 5);
    assert_eq!(manifest.frames.len(), 5);
    assert_eq!(manifest.fps, 10);
    assert_eq!(manifest.duration_ms, 500);
    assert_eq!(manifest.width, 64);
    assert_eq!(manifest.height, 64);

    let total: usize = frames.iter().map(|frame| frame.bmp.len()).sum();
    assert_eq!(manifest.total_bytes, total);
    assert!(out.join(SEQUENCE_MANIFEST_FILE).is_file());

    for (frame, entry) in frames.iter().zip(manifest.frames.iter()) {
        assert_eq!(entry.index, frame.index);
        assert_eq!(entry.time_ms, frame.time_ms);
        assert_eq!(entry.bytes, frame.bmp.len());
        assert_eq!(entry.sha256, frame.sha256);
        let on_disk = std::fs::read(out.join(&entry.file)).expect("frame file");
        assert_eq!(on_disk, frame.bmp);
    }

    let manifest_on_disk = std::fs::read(out.join(SEQUENCE_MANIFEST_FILE)).expect("manifest");
    assert_eq!(
        manifest_on_disk,
        manifest.to_json_pretty().expect("pretty").into_bytes()
    );
}

#[test]
fn writer_rejects_oversize_sequence_before_writing() {
    let frames = render_sequence(&scene(), &static_clip(), 32, 32, 200, 10).expect("sequence");
    let total: usize = frames.iter().map(|frame| frame.bmp.len()).sum();
    let temp = tempfile::tempdir().expect("tempdir");
    let out = temp.path().join("too-big");
    let error =
        write_sequence_capped(&out, &frames, 10, 200, 32, 32, total - 1).expect_err("oversize");
    assert!(matches!(error, EmbodimentError::InvalidInput(_)));
    assert!(!out.exists());
}

#[test]
fn writer_rejects_tampered_frames() {
    let frames = render_sequence(&scene(), &static_clip(), 32, 32, 200, 10).expect("sequence");
    let temp = tempfile::tempdir().expect("tempdir");

    let mut bad_digest = frames.clone();
    bad_digest[0].sha256 = "0".repeat(64);
    assert!(matches!(
        write_sequence(temp.path().join("digest"), &bad_digest, 10, 200, 32, 32)
            .expect_err("digest"),
        EmbodimentError::CorruptData(_)
    ));

    let mut bad_index = frames.clone();
    bad_index[1].index = 7;
    assert!(matches!(
        write_sequence(temp.path().join("index"), &bad_index, 10, 200, 32, 32).expect_err("index"),
        EmbodimentError::CorruptData(_)
    ));
}

#[test]
fn writer_rejects_mismatched_frame_count() {
    let frames = render_sequence(&scene(), &static_clip(), 32, 32, 500, 10).expect("sequence");
    assert_eq!(frames.len(), 5);
    let temp = tempfile::tempdir().expect("tempdir");
    assert!(matches!(
        write_sequence(temp.path(), &frames[..2], 10, 500, 32, 32).expect_err("count"),
        EmbodimentError::InvalidInput(_)
    ));
}
