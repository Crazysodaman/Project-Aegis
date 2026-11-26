use aegis_embodiment::{
    AnimationClip, AnimationPlayer, Easing, EmbodimentError, Keyframe, PlaybackMode, Transform,
    ANIMATION_SCHEMA, MAX_KEYFRAMES,
};

fn approx(actual: f32, expected: f32) {
    assert!(
        (actual - expected).abs() < 1e-5,
        "expected {expected}, got {actual}"
    );
}

fn opacity(value: f32) -> Transform {
    Transform {
        opacity: value,
        ..Transform::identity()
    }
}

fn fade_clip(playback: PlaybackMode) -> AnimationClip {
    AnimationClip::new(
        "fade",
        1000,
        playback,
        vec![
            Keyframe::new(0, opacity(0.0), Easing::Linear),
            Keyframe::new(1000, opacity(1.0), Easing::Linear),
        ],
    )
    .expect("clip")
}

#[test]
fn once_clamps_and_reports_finished() {
    let mut player = AnimationPlayer::new(fade_clip(PlaybackMode::Once));
    let start = player.state();
    assert_eq!(start.time_ms, 0);
    assert_eq!(start.total_elapsed_ms, 0);
    assert!(!start.finished);
    approx(start.transform.opacity, 0.0);

    let quarter = player.advance(250);
    assert_eq!(quarter.time_ms, 250);
    assert_eq!(quarter.total_elapsed_ms, 250);
    assert!(!quarter.finished);
    approx(quarter.transform.opacity, 0.25);

    let end = player.advance(1000);
    assert_eq!(end.time_ms, 1000);
    assert_eq!(end.total_elapsed_ms, 1250);
    assert!(end.finished);
    approx(end.transform.opacity, 1.0);

    let clamped = player.advance(500);
    assert_eq!(clamped.time_ms, 1000);
    assert_eq!(clamped.total_elapsed_ms, 1750);
    assert!(clamped.finished);
    approx(clamped.transform.opacity, 1.0);
    assert_eq!(clamped.clip_name, "fade");
}

#[test]
fn loop_wraps_time_and_never_finishes() {
    let clip = AnimationClip::new(
        "spin",
        1000,
        PlaybackMode::Loop,
        vec![
            Keyframe::new(
                0,
                Transform {
                    rotation_deg: 0.0,
                    ..Transform::identity()
                },
                Easing::Linear,
            ),
            Keyframe::new(
                1000,
                Transform {
                    rotation_deg: 360.0,
                    ..Transform::identity()
                },
                Easing::Linear,
            ),
        ],
    )
    .expect("clip");
    let mut player = AnimationPlayer::new(clip);

    let first = player.advance(1200);
    assert_eq!(first.time_ms, 200);
    assert_eq!(first.total_elapsed_ms, 1200);
    assert!(!first.finished);
    approx(first.transform.rotation_deg, 72.0);

    let middle = player.advance(1300);
    assert_eq!(middle.time_ms, 500);
    assert_eq!(middle.total_elapsed_ms, 2500);
    assert!(!middle.finished);
    approx(middle.transform.rotation_deg, 180.0);

    let wrapped = player.advance(500);
    assert_eq!(wrapped.time_ms, 0);
    assert_eq!(wrapped.total_elapsed_ms, 3000);
    assert!(!wrapped.finished);
    approx(wrapped.transform.rotation_deg, 0.0);
}

#[test]
fn interpolation_honors_easing() {
    let clip = |easing| {
        AnimationClip::new(
            "eased",
            1000,
            PlaybackMode::Once,
            vec![
                Keyframe::new(0, opacity(0.0), easing),
                Keyframe::new(1000, opacity(1.0), easing),
            ],
        )
        .expect("clip")
    };

    approx(clip(Easing::Linear).sample(500).opacity, 0.5);
    approx(clip(Easing::EaseIn).sample(500).opacity, 0.25);
    approx(clip(Easing::EaseOut).sample(500).opacity, 0.75);
    approx(clip(Easing::EaseInOut).sample(500).opacity, 0.5);
    approx(clip(Easing::EaseInOut).sample(250).opacity, 0.125);
    approx(clip(Easing::Step).sample(999).opacity, 0.0);
    approx(clip(Easing::Step).sample(1000).opacity, 1.0);

    approx(Easing::Linear.apply(-1.0), 0.0);
    approx(Easing::Linear.apply(2.0), 1.0);
    approx(Easing::EaseIn.apply(0.5), 0.25);
}

#[test]
fn sampling_clamps_before_first_and_after_last_keyframe() {
    let clip = AnimationClip::new(
        "late",
        300,
        PlaybackMode::Once,
        vec![
            Keyframe::new(100, opacity(0.2), Easing::Linear),
            Keyframe::new(200, opacity(0.8), Easing::Linear),
        ],
    )
    .expect("clip");

    approx(clip.sample(0).opacity, 0.2);
    approx(clip.sample(100).opacity, 0.2);
    approx(clip.sample(150).opacity, 0.5);
    approx(clip.sample(250).opacity, 0.8);

    let mut player = AnimationPlayer::new(clip);
    let state = player.seek(0);
    assert_eq!(state.time_ms, 0);
    approx(state.transform.opacity, 0.2);
}

#[test]
fn reset_and_seek_recompute_state() {
    let mut player = AnimationPlayer::new(fade_clip(PlaybackMode::Once));
    let finished = player.advance(2000);
    assert!(finished.finished);
    assert_eq!(finished.time_ms, 1000);

    player.reset();
    let reset = player.state();
    assert_eq!(reset.total_elapsed_ms, 0);
    assert_eq!(reset.time_ms, 0);
    assert!(!reset.finished);
    approx(reset.transform.opacity, 0.0);

    let sought = player.seek(500);
    assert_eq!(sought.total_elapsed_ms, 500);
    assert_eq!(sought.time_ms, 500);
    approx(sought.transform.opacity, 0.5);
}

#[test]
fn invalid_clips_are_rejected() {
    let assert_invalid = |result: Result<AnimationClip, EmbodimentError>| {
        assert!(matches!(result, Err(EmbodimentError::InvalidInput(_))));
    };

    assert_invalid(AnimationClip::new(
        "",
        1000,
        PlaybackMode::Once,
        vec![Keyframe::new(0, Transform::identity(), Easing::Linear)],
    ));
    assert_invalid(AnimationClip::new(
        "zero",
        0,
        PlaybackMode::Once,
        vec![Keyframe::new(0, Transform::identity(), Easing::Linear)],
    ));
    assert_invalid(AnimationClip::new(
        "empty",
        1000,
        PlaybackMode::Once,
        Vec::new(),
    ));
    assert_invalid(AnimationClip::new(
        "duplicate",
        1000,
        PlaybackMode::Once,
        vec![
            Keyframe::new(0, Transform::identity(), Easing::Linear),
            Keyframe::new(0, Transform::identity(), Easing::Linear),
        ],
    ));
    assert_invalid(AnimationClip::new(
        "late",
        1000,
        PlaybackMode::Once,
        vec![
            Keyframe::new(0, Transform::identity(), Easing::Linear),
            Keyframe::new(1001, Transform::identity(), Easing::Linear),
        ],
    ));
    assert_invalid(AnimationClip::new(
        "nan",
        1000,
        PlaybackMode::Once,
        vec![Keyframe::new(
            0,
            Transform {
                opacity: f32::NAN,
                ..Transform::identity()
            },
            Easing::Linear,
        )],
    ));
    assert_invalid(AnimationClip::new(
        "many",
        1000,
        PlaybackMode::Once,
        (0..=MAX_KEYFRAMES)
            .map(|index| Keyframe::new(index as u64, Transform::identity(), Easing::Linear))
            .collect(),
    ));

    let clip = fade_clip(PlaybackMode::Once);
    assert_eq!(clip.schema, ANIMATION_SCHEMA);
    assert_eq!(clip.duration_ms, 1000);
    clip.validate().expect("valid");
}
