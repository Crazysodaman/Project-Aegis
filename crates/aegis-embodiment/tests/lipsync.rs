use aegis_embodiment::{
    plan_lip_sync, EmbodimentError, LipSyncPlan, MouthShape, WordTiming, LIPSYNC_SCHEMA,
    MAX_LIPSYNC_TRANSCRIPT_CHARS, MAX_LIPSYNC_WORDS,
};

#[test]
fn even_distribution_is_deterministic_and_bounded() {
    let plan = plan_lip_sync("one two three", None, 10, 900).expect("plan");
    let again = plan_lip_sync("one two three", None, 10, 900).expect("again");
    assert_eq!(plan, again);
    assert_eq!(
        plan.to_json().expect("json"),
        again.to_json().expect("json")
    );

    assert_eq!(plan.schema, LIPSYNC_SCHEMA);
    assert_eq!(plan.fps, 10);
    assert_eq!(plan.duration_ms, 900);
    assert_eq!(plan.frame_count, 9);
    assert_eq!(plan.mouth_frames.len(), plan.frame_count);
    assert_eq!(plan.cues.len(), 3);

    let spans: Vec<(u64, u64)> = plan
        .cues
        .iter()
        .map(|cue| (cue.start_ms, cue.end_ms))
        .collect();
    assert_eq!(spans, vec![(0, 300), (300, 600), (600, 900)]);

    for cue in &plan.cues {
        assert!(cue.frame_index < plan.frame_count);
        assert!(cue.start_ms < cue.end_ms);
    }
    assert_eq!(
        plan.cues
            .iter()
            .map(|cue| cue.frame_index)
            .collect::<Vec<_>>(),
        vec![1, 4, 7]
    );
    assert!(plan
        .mouth_frames
        .iter()
        .any(|shape| *shape != MouthShape::Closed));
}

#[test]
fn explicit_timings_are_respected() {
    let timing = [WordTiming::new(0, 200), WordTiming::new(400, 900)];
    let plan = plan_lip_sync("hello world", Some(&timing), 10, 1000).expect("plan");
    assert_eq!(plan.frame_count, 10);
    assert_eq!(plan.cues[0].start_ms, 0);
    assert_eq!(plan.cues[0].end_ms, 200);
    assert_eq!(plan.cues[0].frame_index, 1);
    assert_eq!(plan.cues[1].start_ms, 400);
    assert_eq!(plan.cues[1].end_ms, 900);
    assert_eq!(plan.cues[1].frame_index, 6);

    assert_ne!(plan.mouth_frames[0], MouthShape::Closed);
    assert_eq!(plan.mouth_frames[2], MouthShape::Closed);
    assert_eq!(plan.mouth_frames[3], MouthShape::Closed);
    assert_ne!(plan.mouth_frames[4], MouthShape::Closed);
    assert_ne!(plan.mouth_frames[8], MouthShape::Closed);
    assert_eq!(plan.mouth_frames[9], MouthShape::Closed);
}

#[test]
fn invalid_transcripts_and_timings_are_rejected() {
    for transcript in ["", "   ", "\n\t"] {
        assert!(matches!(
            plan_lip_sync(transcript, None, 10, 1000),
            Err(EmbodimentError::InvalidInput(_))
        ));
    }

    let too_long = "a".repeat(MAX_LIPSYNC_TRANSCRIPT_CHARS + 1);
    assert!(matches!(
        plan_lip_sync(&too_long, None, 10, 1000),
        Err(EmbodimentError::InvalidInput(_))
    ));

    let many_words = vec!["word"; MAX_LIPSYNC_WORDS + 1].join(" ");
    assert!(matches!(
        plan_lip_sync(&many_words, None, 10, 1_000_000),
        Err(EmbodimentError::InvalidInput(_))
    ));

    assert!(matches!(
        plan_lip_sync("a b", None, 0, 1000),
        Err(EmbodimentError::InvalidInput(_))
    ));
    assert!(matches!(
        plan_lip_sync("a b", None, 10, 0),
        Err(EmbodimentError::InvalidInput(_))
    ));
    assert!(matches!(
        plan_lip_sync("a b", None, 10, 1),
        Err(EmbodimentError::InvalidInput(_))
    ));
}

#[test]
fn invalid_word_timings_are_rejected() {
    let cases = [
        vec![WordTiming::new(0, 500)],
        vec![WordTiming::new(500, 500), WordTiming::new(600, 700)],
        vec![WordTiming::new(0, 500), WordTiming::new(400, 900)],
        vec![WordTiming::new(0, 500), WordTiming::new(600, 1100)],
    ];
    for timings in cases {
        assert!(matches!(
            plan_lip_sync("one two", Some(&timings), 10, 1000),
            Err(EmbodimentError::InvalidInput(_))
        ));
    }
}

#[test]
fn mouth_frames_never_exceed_bounds() {
    let timings = [
        WordTiming::new(0, 200),
        WordTiming::new(400, 600),
        WordTiming::new(800, 1000),
    ];
    let plan: LipSyncPlan =
        plan_lip_sync("two little words", Some(&timings), 24, 1000).expect("plan");
    assert_eq!(plan.frame_count, 24);
    assert_eq!(plan.mouth_frames.len(), 24);
    let mut see_closed = false;
    let mut see_open = false;
    for shape in &plan.mouth_frames {
        match shape {
            MouthShape::Closed => see_closed = true,
            _ => see_open = true,
        }
    }
    assert!(see_closed && see_open);
    for cue in &plan.cues {
        assert!(cue.frame_index < plan.frame_count);
    }
}
