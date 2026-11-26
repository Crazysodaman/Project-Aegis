//! Live, environment-gated evidence for state-driven avatar sequences.
//!
//! This test only runs when `AEGIS_AVATAR_LIVE=1`. It renders two Cores with
//! different emotion through the schematic CPU BMP pipeline, writes manifests,
//! and prints per-frame counts, byte totals, and SHA-256 digests.

use aegis_contracts::personal::{EmotionDimensions, EmotionState, EMOTION_STATE_SCHEMA};
use aegis_embodiment::animation::project_avatar_state;
use aegis_embodiment::lipsync::plan_lip_sync;
use aegis_embodiment::sequence::{render_state_sequence, write_state_sequence, SequenceOptions};
use aegis_embodiment::{AssetKind, AvatarScene, LayerRole, SceneLayer, Theme};

const CORE_A: &str = "00000000-0000-4000-8000-0000000000aa";
const CORE_B: &str = "00000000-0000-4000-8000-0000000000bb";

fn emotion(
    core: &str,
    valence: f32,
    arousal: f32,
    joy: f32,
    calm: f32,
    curiosity: f32,
    tension: f32,
) -> EmotionState {
    EmotionState {
        schema: EMOTION_STATE_SCHEMA.to_owned(),
        core_uuid: core.to_owned(),
        dimensions: EmotionDimensions {
            valence,
            arousal,
            joy,
            calm,
            curiosity,
            tension,
        },
        intensity: 0.8,
        updated_at: "2026-11-26T12:00:00Z".to_owned(),
        revision: 1,
    }
}

fn scene(core: &str) -> AvatarScene {
    let layer = SceneLayer {
        role: LayerRole::Base,
        asset_id: "synthetic-base".to_owned(),
        asset_kind: AssetKind::Avatar,
        media_type: "image/png".to_owned(),
        sha256: "synthetic-digest".to_owned(),
        visible: true,
        z_index: 0,
    };
    AvatarScene::new(core, Theme::default(), vec![layer], None, None).expect("synthetic scene")
}

#[test]
fn live_state_sequence() {
    if std::env::var("AEGIS_AVATAR_LIVE").ok().as_deref() != Some("1") {
        eprintln!("AEGIS_AVATAR_LIVE not set to 1; skipping live state sequence test");
        return;
    }

    let options = SequenceOptions::new(12, 1_500, 128, 160);
    let plan = plan_lip_sync(
        "hello from the aegis avatar state pipeline",
        None,
        options.fps,
        options.duration_ms,
    )
    .expect("lip-sync plan");

    let cases = [
        (
            CORE_A,
            project_avatar_state(
                Some(&emotion(CORE_A, 0.9, 0.8, 0.95, 0.05, 0.3, 0.0)),
                None,
                None,
            ),
        ),
        (
            CORE_B,
            project_avatar_state(
                Some(&emotion(CORE_B, -0.8, 0.2, 0.0, 0.2, 0.2, 0.7)),
                None,
                None,
            ),
        ),
    ];

    let temp = tempfile::tempdir().expect("tempdir");
    let mut first_hashes = Vec::new();
    for (core, state) in &cases {
        let scene = scene(core);
        let frames = render_state_sequence(&scene, state, Some(&plan), options)
            .expect("state sequence frames");
        let manifest =
            write_state_sequence(temp.path().join(core), &frames, state, Some(&plan), options)
                .expect("state sequence write");

        assert!(frames.len() > 1);
        let unique: std::collections::HashSet<_> =
            frames.iter().map(|frame| frame.sha256.as_str()).collect();
        assert!(unique.len() > 1, "movement must vary frames");
        first_hashes.push(frames[0].sha256.clone());

        println!(
            "live avatar state: core={} expression={} gesture={} motion={} frames={} bytes={}",
            core,
            state.expression.as_str(),
            state.gesture.as_str(),
            state.motion.as_str(),
            manifest.frame_count,
            manifest.total_bytes
        );
        println!(
            "  manifest={} state_schema={} mouth_frames={} unique_hashes={}",
            temp.path().join(core).join("manifest.json").display(),
            manifest
                .avatar_state
                .as_ref()
                .map(|state| state.schema.as_str())
                .unwrap_or("none"),
            manifest.mouth_frames.as_ref().map_or(0, Vec::len),
            unique.len()
        );
        for &index in &[0usize, frames.len() / 2, frames.len() - 1] {
            let frame = &frames[index];
            println!(
                "  frame={} time_ms={} bytes={} sha256={}",
                frame.index,
                frame.time_ms,
                frame.bmp.len(),
                frame.sha256
            );
        }
    }

    assert_ne!(
        first_hashes[0], first_hashes[1],
        "different Cores/states must produce different frames"
    );
}
