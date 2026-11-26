use aegis_contracts::personal::{
    Activity, ActivityStatus, EmotionDimensions, EmotionState, EMOTION_STATE_SCHEMA,
};
use aegis_contracts::world::{
    FreshnessState, Provenance, Season, TimeOfDay, WeatherObservation, WorldSnapshot,
    WORLD_SNAPSHOT_SCHEMA,
};
use aegis_embodiment::animation::{project_avatar_state, MotionMode, StateSignal};
use aegis_embodiment::expression::{Expression, Gesture};
use aegis_embodiment::lipsync::{plan_lip_sync, LipSyncPlan};
use aegis_embodiment::sequence::{
    render_state_sequence, write_state_sequence, write_state_sequence_capped, SequenceOptions,
    MAX_SEQUENCE_TOTAL_BYTES,
};
use aegis_embodiment::{
    sha256_hex, write_sequence, AssetKind, AvatarScene, LayerRole, PlaybackMode, SceneLayer, Theme,
    BMP_MAGIC,
};

const CORE_A: &str = "00000000-0000-4000-8000-0000000000aa";
const CORE_B: &str = "00000000-0000-4000-8000-0000000000bb";

fn emotion(core: &str, dimensions: EmotionDimensions) -> EmotionState {
    EmotionState {
        schema: EMOTION_STATE_SCHEMA.to_owned(),
        core_uuid: core.to_owned(),
        dimensions,
        intensity: 0.8,
        updated_at: "2026-11-26T12:00:00Z".to_owned(),
        revision: 1,
    }
}

fn dimensions(
    valence: f32,
    arousal: f32,
    joy: f32,
    calm: f32,
    curiosity: f32,
    tension: f32,
) -> EmotionDimensions {
    EmotionDimensions {
        valence,
        arousal,
        joy,
        calm,
        curiosity,
        tension,
    }
}

fn activity(title: &str, status: ActivityStatus) -> Activity {
    Activity {
        id: "activity-1".to_owned(),
        title: title.to_owned(),
        status,
        started_at: "2026-11-26T12:00:00Z".to_owned(),
        updated_at: "2026-11-26T12:00:00Z".to_owned(),
        notes: String::new(),
    }
}

fn weather(condition: &str, temperature_celsius: f64) -> WeatherObservation {
    WeatherObservation {
        condition: condition.to_owned(),
        temperature_celsius: Some(temperature_celsius),
        feels_like_celsius: Some(temperature_celsius),
        humidity_percent: Some(50),
        wind_speed_kph: Some(5.0),
        observed_at: None,
    }
}

fn world(
    season: Season,
    time_of_day: TimeOfDay,
    weather: Option<WeatherObservation>,
) -> WorldSnapshot {
    WorldSnapshot {
        schema: WORLD_SNAPSHOT_SCHEMA.to_owned(),
        generated_at: "2026-11-26T12:00:00+00:00".to_owned(),
        timezone: "America/Chicago".to_owned(),
        utc_offset_seconds: -6 * 3600,
        local_date: "2026-11-26".to_owned(),
        local_time: "12:00:00".to_owned(),
        time_of_day,
        season,
        sun: None,
        weather,
        forecast: Vec::new(),
        calendar: Vec::new(),
        provenance: Provenance {
            source: "test".to_owned(),
            provider: None,
            retrieved_at: None,
            note: None,
        },
        freshness: FreshnessState::Fresh,
        stale_after_seconds: 900,
    }
}

fn scene(core: &str, asset_id: &str) -> AvatarScene {
    let layer = SceneLayer {
        role: LayerRole::Base,
        asset_id: asset_id.to_owned(),
        asset_kind: AssetKind::Avatar,
        media_type: "image/png".to_owned(),
        sha256: "synthetic-digest".to_owned(),
        visible: true,
        z_index: 0,
    };
    AvatarScene::new(core, Theme::default(), vec![layer], None, None).expect("synthetic scene")
}

fn declared_bmp_size(bytes: &[u8]) -> usize {
    u32::from_le_bytes([bytes[2], bytes[3], bytes[4], bytes[5]]) as usize
}

fn assert_valid_frame(frame: &aegis_embodiment::sequence::RenderedFrame) {
    assert!(frame.bmp.starts_with(&BMP_MAGIC));
    assert_eq!(declared_bmp_size(&frame.bmp), frame.bmp.len());
    assert_eq!(frame.sha256.len(), 64);
    assert_eq!(frame.sha256, sha256_hex(&frame.bmp));
}

#[test]
fn projection_is_deterministic_and_uses_supplied_signals() {
    let emotion = emotion(CORE_A, dimensions(0.8, 0.8, 0.9, 0.1, 0.4, 0.0));
    let activity = activity("reading", ActivityStatus::Active);
    let world = world(
        Season::Autumn,
        TimeOfDay::Afternoon,
        Some(weather("clear", 18.0)),
    );

    let first = project_avatar_state(Some(&emotion), Some(&activity), Some(&world));
    let second = project_avatar_state(Some(&emotion), Some(&activity), Some(&world));
    assert_eq!(first, second);
    first.validate().expect("valid state");

    assert_eq!(first.schema, "aegis.embodiment.avatar_state.v1");
    assert_eq!(first.core_uuid.as_deref(), Some(CORE_A));
    assert_eq!(first.expression, Expression::Happy);
    assert_eq!(first.gesture, Gesture::ThumbsUp);
    assert_eq!(first.motion, MotionMode::Active);
    assert_eq!(
        first.signals,
        vec![
            StateSignal::Emotion,
            StateSignal::Activity,
            StateSignal::World,
            StateSignal::Weather,
        ]
    );

    let clip = first.clip().expect("clip");
    assert_eq!(clip.name, first.clip_name);
    assert_eq!(clip.playback, PlaybackMode::Loop);
    assert_eq!(clip.duration_ms, 1_000);
    assert_ne!(
        clip.sample(0),
        clip.sample(clip.duration_ms / 2),
        "movement clip must vary"
    );
}

#[test]
fn missing_signals_degrade_to_neutral_without_invention() {
    let empty = project_avatar_state(None, None, None);
    empty.validate().expect("valid neutral state");
    assert_eq!(empty.expression, Expression::Neutral);
    assert_eq!(empty.gesture, Gesture::None);
    assert_eq!(empty.motion, MotionMode::Idle);
    assert_eq!(empty.energy, 40);
    assert_eq!(empty.clip_name, "idle-sway");
    assert!(empty.core_uuid.is_none());
    assert!(empty.signals.is_empty());

    let activity_only = project_avatar_state(
        None,
        Some(&activity("walking", ActivityStatus::Active)),
        None,
    );
    assert_eq!(activity_only.expression, Expression::Neutral);
    assert_eq!(activity_only.gesture, Gesture::None);
    assert_eq!(activity_only.motion, MotionMode::Active);
    assert_eq!(activity_only.signals, vec![StateSignal::Activity]);
    assert!(activity_only.core_uuid.is_none());

    let world_only = project_avatar_state(
        None,
        None,
        Some(&world(Season::Winter, TimeOfDay::Night, None)),
    );
    assert_eq!(world_only.expression, Expression::Neutral);
    assert_eq!(world_only.gesture, Gesture::None);
    assert_eq!(world_only.motion, MotionMode::Resting);
    assert_eq!(world_only.signals, vec![StateSignal::World]);
    assert!(world_only.core_uuid.is_none());
}

#[test]
fn two_cores_with_different_emotion_produce_different_frames() {
    let scene_a = scene(CORE_A, "synthetic-base");
    let scene_b = scene(CORE_B, "synthetic-base");
    let state_a = project_avatar_state(
        Some(&emotion(CORE_A, dimensions(0.9, 0.8, 0.95, 0.05, 0.3, 0.0))),
        None,
        None,
    );
    let state_b = project_avatar_state(
        Some(&emotion(CORE_B, dimensions(-0.8, 0.2, 0.0, 0.2, 0.2, 0.6))),
        None,
        None,
    );
    assert_ne!(state_a.expression, state_b.expression);

    let options = SequenceOptions::new(10, 1_000, 64, 80);
    let frames_a = render_state_sequence(&scene_a, &state_a, None, options).expect("frames a");
    let frames_b = render_state_sequence(&scene_b, &state_b, None, options).expect("frames b");
    let frames_a_again =
        render_state_sequence(&scene_a, &state_a, None, options).expect("frames a again");

    assert_eq!(frames_a, frames_a_again);
    assert_eq!(frames_a.len(), 10);
    assert_eq!(frames_b.len(), 10);
    assert_ne!(frames_a[0].sha256, frames_b[0].sha256);
    assert!(frames_a
        .iter()
        .zip(frames_b.iter())
        .all(|(left, right)| left.sha256 != right.sha256));

    for frame in frames_a.iter().chain(frames_b.iter()) {
        assert_valid_frame(frame);
    }
    let unique_a: std::collections::HashSet<_> =
        frames_a.iter().map(|frame| frame.sha256.as_str()).collect();
    assert!(unique_a.len() > 1, "movement must vary frames");
}

#[test]
fn lipsync_mouth_frames_change_rendered_pixels() {
    let scene = scene(CORE_A, "synthetic-base");
    let state = project_avatar_state(
        Some(&emotion(CORE_A, dimensions(0.6, 0.4, 0.6, 0.3, 0.4, 0.0))),
        None,
        None,
    );
    let options = SequenceOptions::new(10, 1_000, 64, 80);
    let plan = plan_lip_sync("hello there world", None, 10, 1_000).expect("plan");

    let with_plan =
        render_state_sequence(&scene, &state, Some(&plan), options).expect("lip-synced frames");
    let without_plan = render_state_sequence(&scene, &state, None, options).expect("plain frames");

    assert_eq!(with_plan.len(), plan.frame_count);
    assert_eq!(without_plan.len(), plan.frame_count);
    assert!(plan
        .mouth_frames
        .iter()
        .any(|shape| *shape != aegis_embodiment::lipsync::MouthShape::Closed));
    assert!(
        with_plan
            .iter()
            .zip(without_plan.iter())
            .any(|(left, right)| left.sha256 != right.sha256),
        "mouth shapes must paint different pixels"
    );
    for frame in &with_plan {
        assert_valid_frame(frame);
    }

    let wrong_fps = plan_lip_sync("hello there world", None, 12, 1_000).expect("wrong fps");
    assert!(render_state_sequence(&scene, &state, Some(&wrong_fps), options).is_err());

    let wrong_duration = plan_lip_sync("hello there world", None, 10, 900).expect("wrong duration");
    assert!(render_state_sequence(&scene, &state, Some(&wrong_duration), options).is_err());
}

#[test]
fn manifest_records_state_and_mouth_frames() {
    let scene = scene(CORE_A, "synthetic-base");
    let state = project_avatar_state(
        Some(&emotion(CORE_A, dimensions(0.7, 0.5, 0.7, 0.2, 0.5, 0.0))),
        None,
        None,
    );
    let options = SequenceOptions::new(10, 1_000, 64, 80);
    let plan = plan_lip_sync("two little words", None, 10, 1_000).expect("plan");
    let frames = render_state_sequence(&scene, &state, Some(&plan), options).expect("state frames");

    let temp = tempfile::tempdir().expect("tempdir");
    let out = temp.path().join("state-sequence");
    let manifest =
        write_state_sequence(&out, &frames, &state, Some(&plan), options).expect("write state");

    assert_eq!(manifest.schema, "aegis.embodiment.sequence_manifest.v1");
    assert_eq!(manifest.avatar_state.as_ref(), Some(&state));
    assert_eq!(
        manifest.mouth_frames.as_deref(),
        Some(plan.mouth_frames.as_slice())
    );
    assert_eq!(
        manifest.total_bytes,
        frames.iter().map(|frame| frame.bmp.len()).sum::<usize>()
    );
    assert_eq!(manifest.frame_count, frames.len());
    for entry in &manifest.frames {
        let on_disk = std::fs::read(out.join(&entry.file)).expect("frame file");
        assert_eq!(entry.sha256, sha256_hex(&on_disk));
    }
    let json = manifest.to_json().expect("manifest json");
    assert!(json.contains("avatar_state"));
    assert!(json.contains("mouth_frames"));

    let plain_out = temp.path().join("plain-sequence");
    let plain = write_sequence(
        &plain_out,
        &frames,
        options.fps,
        options.duration_ms,
        options.width,
        options.height,
    )
    .expect("plain write");
    assert!(plain.avatar_state.is_none());
    assert!(plain.mouth_frames.is_none());
    let plain_json = plain.to_json().expect("plain json");
    assert!(!plain_json.contains("avatar_state"));
    assert!(!plain_json.contains("mouth_frames"));

    let total: usize = frames.iter().map(|frame| frame.bmp.len()).sum();
    let too_small = temp.path().join("too-small");
    assert!(write_state_sequence_capped(
        &too_small,
        &frames,
        &state,
        Some(&plan),
        options,
        total - 1,
    )
    .is_err());
    assert!(!too_small.exists());
}

#[test]
fn state_core_mismatch_and_bounds_are_rejected() {
    let scene_a = scene(CORE_A, "asset-a");
    let scene_b = scene(CORE_B, "asset-b");
    let state_a = project_avatar_state(
        Some(&emotion(CORE_A, dimensions(0.5, 0.4, 0.5, 0.3, 0.2, 0.0))),
        None,
        None,
    );
    let state_b = project_avatar_state(
        Some(&emotion(CORE_B, dimensions(-0.4, 0.3, 0.0, 0.2, 0.2, 0.0))),
        None,
        None,
    );
    let options = SequenceOptions::new(8, 500, 48, 48);

    assert!(matches!(
        render_state_sequence(&scene_a, &state_b, None, options),
        Err(aegis_embodiment::EmbodimentError::OwnershipMismatch)
    ));

    let scene_a_before = scene_a.clone();
    let frames_a = render_state_sequence(&scene_a, &state_a, None, options).expect("frames a");
    let frames_b = render_state_sequence(&scene_b, &state_b, None, options).expect("frames b");
    assert_eq!(
        scene_a, scene_a_before,
        "rendering must not mutate the scene"
    );
    assert_ne!(frames_a[0].sha256, frames_b[0].sha256);
    assert!(scene_a
        .layers
        .iter()
        .all(|layer| layer.asset_id == "asset-a"));
    assert!(scene_b
        .layers
        .iter()
        .all(|layer| layer.asset_id == "asset-b"));

    let mut bad_energy = state_a.clone();
    bad_energy.energy = 101;
    assert!(matches!(
        render_state_sequence(&scene_a, &bad_energy, None, options),
        Err(aegis_embodiment::EmbodimentError::InvalidInput(_))
    ));

    let mut bad_schema = state_a.clone();
    bad_schema.schema = "aegis.embodiment.avatar_state.v0".to_owned();
    assert!(matches!(
        render_state_sequence(&scene_a, &bad_schema, None, options),
        Err(aegis_embodiment::EmbodimentError::CorruptData(_))
    ));

    for bad_options in [
        SequenceOptions::new(0, 500, 48, 48),
        SequenceOptions::new(8, 0, 48, 48),
        SequenceOptions::new(8, 500, 8, 8),
    ] {
        assert!(render_state_sequence(&scene_a, &state_a, None, bad_options).is_err());
    }
}

#[test]
fn bounded_state_sequence_scales() {
    let scene = scene(CORE_A, "synthetic-base");
    let state = project_avatar_state(
        Some(&emotion(CORE_A, dimensions(0.4, 0.3, 0.4, 0.4, 0.3, 0.0))),
        None,
        None,
    );
    let options = SequenceOptions::new(24, 2_000, 256, 256);
    let plan: LipSyncPlan = plan_lip_sync(
        "a much longer transcript for bounded rendering",
        None,
        24,
        2_000,
    )
    .expect("plan");
    let frames =
        render_state_sequence(&scene, &state, Some(&plan), options).expect("bounded frames");
    assert_eq!(frames.len(), 48);
    let total: usize = frames.iter().map(|frame| frame.bmp.len()).sum();
    assert!(total <= MAX_SEQUENCE_TOTAL_BYTES);

    let temp = tempfile::tempdir().expect("tempdir");
    let manifest = write_state_sequence(
        temp.path().join("bounded"),
        &frames,
        &state,
        Some(&plan),
        options,
    )
    .expect("bounded write");
    assert_eq!(manifest.frame_count, 48);
    assert_eq!(manifest.total_bytes, total);
}
