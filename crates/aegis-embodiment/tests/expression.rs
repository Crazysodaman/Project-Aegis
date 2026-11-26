use aegis_embodiment::{
    expression_params, gesture_params, render_expression_bmp, AssetKind, AvatarScene,
    EmbodimentError, Expression, Gesture, LayerRole, SceneLayer, Theme, BMP_MAGIC,
};

const SYNTHETIC_CORE: &str = "00000000-0000-4000-8000-0000000000aa";

fn scene() -> AvatarScene {
    let layer = SceneLayer {
        role: LayerRole::Base,
        asset_id: "synthetic-base".to_owned(),
        asset_kind: AssetKind::Avatar,
        media_type: "image/png".to_owned(),
        sha256: "synthetic-digest".to_owned(),
        visible: true,
        z_index: 0,
    };
    AvatarScene::new(SYNTHETIC_CORE, Theme::default(), vec![layer], None, None).expect("scene")
}

#[test]
fn expression_mapping_is_pure_and_distinct() {
    for expression in Expression::ALL {
        let params = expression_params(expression);
        assert_eq!(params.expression, expression);
        assert_eq!(expression_params(expression), params);
        assert!(params.mouth_open <= 100);
        assert!(params.eye_open <= 100);
        assert!((-100..=100).contains(&params.brow_tilt));
        assert_eq!(params, expression_params(expression));
    }

    let neutral = expression_params(Expression::Neutral);
    let happy = expression_params(Expression::Happy);
    assert_ne!(neutral.overlay, happy.overlay);
    assert!(happy.mouth_open > neutral.mouth_open);
    assert_eq!(Expression::Happy.as_str(), "happy");

    let overlays: std::collections::HashSet<[u8; 3]> = Expression::ALL
        .iter()
        .map(|expression| expression_params(*expression).overlay)
        .collect();
    assert_eq!(overlays.len(), Expression::ALL.len());
}

#[test]
fn gesture_mapping_is_pure_and_distinct() {
    for gesture in Gesture::ALL {
        let params = gesture_params(gesture);
        assert_eq!(params.gesture, gesture);
        assert_eq!(gesture_params(gesture), params);
        assert_eq!(params, gesture_params(gesture));
        assert!(params.scale_percent > 0);
    }
    assert_eq!(gesture_params(Gesture::None).overlay_weight, 0);
    assert_ne!(
        gesture_params(Gesture::Wave).offset_x,
        gesture_params(Gesture::Nod).offset_x
    );
    assert_eq!(Gesture::ThumbsUp.as_str(), "thumbs_up");
}

#[test]
fn rendering_an_expression_changes_pixels_deterministically() {
    let scene = scene();
    let neutral_first = render_expression_bmp(&scene, Expression::Neutral, 96, 128).expect("n1");
    let neutral_second = render_expression_bmp(&scene, Expression::Neutral, 96, 128).expect("n2");
    let happy = render_expression_bmp(&scene, Expression::Happy, 96, 128).expect("happy");
    let sad = render_expression_bmp(&scene, Expression::Sad, 96, 128).expect("sad");

    assert_eq!(neutral_first, neutral_second);
    assert_ne!(neutral_first, happy);
    assert_ne!(happy, sad);

    for bytes in [&neutral_first, &happy, &sad] {
        assert!(bytes.starts_with(&BMP_MAGIC));
        let declared = u32::from_le_bytes([bytes[2], bytes[3], bytes[4], bytes[5]]) as usize;
        assert_eq!(declared, bytes.len());
    }
}

#[test]
fn expression_render_propagates_invalid_dimensions() {
    assert!(matches!(
        render_expression_bmp(&scene(), Expression::Happy, 8, 8).expect_err("dimensions"),
        EmbodimentError::InvalidInput(_)
    ));
}
