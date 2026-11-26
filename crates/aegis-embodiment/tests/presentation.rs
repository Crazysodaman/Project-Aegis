use std::path::{Path, PathBuf};

use aegis_contracts::initialization::{CoreInitializationRequest, CORE_INIT_REQUEST_SCHEMA};
use aegis_contracts::personal::{
    Activity, ActivityStatus, EmotionDimensions, EmotionState, EMOTION_STATE_SCHEMA, MAX_ID_CHARS,
    MAX_OUTFIT_ASSET_IDS, MAX_OUTFIT_RATIONALE_CHARS, MAX_TIMESTAMP_CHARS,
};
use aegis_contracts::world::{
    FreshnessState, Provenance, Season, TimeOfDay, WeatherObservation, WorldSnapshot,
    WORLD_SNAPSHOT_SCHEMA,
};
use aegis_core_install::CoreInstaller;
use aegis_embodiment::{
    expression_for_emotion, render_presentation_frame, select_outfit, voice_projection, AssetKind,
    EmbodimentError, EmbodimentStore, NewAsset, PresentationContext, BMP_MAGIC,
    MAX_OUTFIT_SELECTION, MAX_VOICE_RATE_PERCENT, MAX_VOICE_VOLUME_PERCENT, MIN_VOICE_RATE_PERCENT,
    MIN_VOICE_VOLUME_PERCENT, OUTFIT_DECISION_SCHEMA, PRESENTATION_FRAME_HEIGHT,
    PRESENTATION_FRAME_SCHEMA, PRESENTATION_FRAME_WIDTH,
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

fn import_tagged(
    store: &EmbodimentStore,
    kind: AssetKind,
    name: &str,
    bytes: &[u8],
    tags: &[&str],
) -> String {
    let tags = tags.iter().map(|tag| (*tag).to_owned()).collect::<Vec<_>>();
    store
        .import_asset(NewAsset::new(kind, name, "application/octet-stream", bytes).with_tags(tags))
        .expect("import")
        .asset_id
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

fn emotion(core: &str, dimensions: EmotionDimensions, intensity: f32) -> EmotionState {
    EmotionState {
        schema: EMOTION_STATE_SCHEMA.to_owned(),
        core_uuid: core.to_owned(),
        dimensions,
        intensity,
        updated_at: "2026-11-26T12:00:00Z".to_owned(),
        revision: 1,
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

fn all_asset_ids(store: &EmbodimentStore) -> Vec<String> {
    store
        .list_assets(None, true)
        .expect("assets")
        .into_iter()
        .map(|asset| asset.asset_id)
        .collect()
}

#[test]
fn two_cores_make_different_outfit_decisions() {
    let (_temp, root) = setup();
    let store_a = open_store(&root, CORE_A);
    let store_b = open_store(&root, CORE_B);

    let winter_coat = import_tagged(
        &store_a,
        AssetKind::Wardrobe,
        "winter coat",
        b"a-coat",
        &["warm", "waterproof", "winter"],
    );
    let _sun_dress = import_tagged(
        &store_a,
        AssetKind::Wardrobe,
        "sun dress",
        b"a-dress",
        &["light", "summer"],
    );

    let business_suit = import_tagged(
        &store_b,
        AssetKind::Wardrobe,
        "business suit",
        b"b-suit",
        &["warm", "waterproof", "formal", "autumn"],
    );
    let _cozy_hoodie = import_tagged(
        &store_b,
        AssetKind::Wardrobe,
        "cozy hoodie",
        b"b-hoodie",
        &["warm", "waterproof", "cozy", "casual"],
    );

    let cold = weather("rain", 0.0);
    let world = world(Season::Autumn, TimeOfDay::Evening, Some(cold));
    let emotion = emotion(CORE_A, dimensions(0.6, 0.5, 0.7, 0.4, 0.3, 0.1), 0.8);
    let ctx = PresentationContext::new()
        .with_world(&world)
        .with_emotion(&emotion);

    let decision_a = select_outfit(&store_a, CORE_A, &ctx, 2).expect("decision a");
    let decision_b = select_outfit(&store_b, CORE_B, &ctx, 2).expect("decision b");

    assert_eq!(decision_a.schema, OUTFIT_DECISION_SCHEMA);
    assert_eq!(decision_a.core_uuid, CORE_A);
    assert_eq!(decision_b.core_uuid, CORE_B);
    assert!(decision_a.available && decision_b.available);
    assert_eq!(decision_a.asset_ids, vec![winter_coat]);
    assert_eq!(decision_b.asset_ids.first(), Some(&business_suit));
    assert!(!decision_a
        .asset_ids
        .iter()
        .any(|asset_id| decision_b.asset_ids.contains(asset_id)));

    assert!(matches!(
        select_outfit(&store_a, CORE_B, &ctx, 2),
        Err(EmbodimentError::OwnershipMismatch)
    ));
}

#[test]
fn weather_season_and_time_change_the_selection() {
    let (_temp, root) = setup();
    let store = open_store(&root, CORE_A);

    let shorts = import_tagged(
        &store,
        AssetKind::Wardrobe,
        "summer shorts",
        b"shorts",
        &["summer", "light", "cool", "casual"],
    );
    let coat = import_tagged(
        &store,
        AssetKind::Wardrobe,
        "winter coat",
        b"coat",
        &["winter", "warm", "waterproof", "coat", "formal"],
    );
    let _rain_jacket = import_tagged(
        &store,
        AssetKind::Wardrobe,
        "rain jacket",
        b"jacket",
        &["rain", "waterproof", "jacket", "warm"],
    );

    let cold_world = world(
        Season::Winter,
        TimeOfDay::Evening,
        Some(weather("rain", -2.0)),
    );
    let cold_ctx = PresentationContext::new().with_world(&cold_world);
    let cold = select_outfit(&store, CORE_A, &cold_ctx, 3).expect("cold decision");

    let hot_world = world(
        Season::Summer,
        TimeOfDay::Midday,
        Some(weather("clear", 33.0)),
    );
    let hot_ctx = PresentationContext::new().with_world(&hot_world);
    let hot = select_outfit(&store, CORE_A, &hot_ctx, 3).expect("hot decision");

    assert_eq!(cold.asset_ids.first(), Some(&coat));
    assert_eq!(hot.asset_ids.first(), Some(&shorts));
    assert_ne!(cold.asset_ids, hot.asset_ids);
    assert!(!hot.asset_ids.contains(&coat));
    assert!(cold.constraints.iter().any(|c| c == "cold-safety"));
    assert!(cold.constraints.iter().any(|c| c == "precipitation-safety"));
    assert!(hot.constraints.iter().any(|c| c == "heat-safety"));
}

#[test]
fn missing_world_and_emotion_are_honest() {
    let (_temp, root) = setup();
    let store_a = open_store(&root, CORE_A);
    let store_b = open_store(&root, CORE_B);

    let plain = import_tagged(
        &store_a,
        AssetKind::Wardrobe,
        "plain shirt",
        b"shirt",
        &["casual"],
    );

    let ctx = PresentationContext::new();
    let decision = select_outfit(&store_a, CORE_A, &ctx, 2).expect("decision");
    assert!(decision.available);
    assert_eq!(decision.asset_ids, vec![plain]);
    assert!(decision.rationale.contains("default"));
    assert!(decision.rejected.is_empty());

    let appearance = store_a.appearance().expect("appearance");
    assert!(expression_for_emotion(&appearance, None).is_none());
    let projection = voice_projection(None);
    assert_eq!(projection.rate_percent, 100);
    assert_eq!(projection.volume_percent, 100);
    assert_eq!(projection.style_label, "neutral");

    let empty = select_outfit(&store_b, CORE_B, &ctx, 2).expect("empty decision");
    assert!(!empty.available);
    assert!(empty.asset_ids.is_empty());
    assert!(empty.rationale.contains("no live wardrobe"));
}

#[test]
fn selections_never_cross_core_boundaries() {
    let (_temp, root) = setup();
    let store_a = open_store(&root, CORE_A);
    let store_b = open_store(&root, CORE_B);

    let shirt = import_tagged(
        &store_a,
        AssetKind::Wardrobe,
        "a neutral shirt",
        b"a-shirt",
        &["casual"],
    );
    let suit = import_tagged(
        &store_b,
        AssetKind::Wardrobe,
        "b formal suit",
        b"b-suit",
        &["formal"],
    );

    let ctx = PresentationContext::new();
    let decision = select_outfit(&store_a, CORE_A, &ctx, 8).expect("decision");
    assert_eq!(decision.asset_ids, vec![shirt.clone()]);
    assert!(!decision.asset_ids.contains(&suit));
    for asset_id in &decision.asset_ids {
        assert!(all_asset_ids(&store_a).contains(asset_id));
        assert!(!all_asset_ids(&store_b).contains(asset_id));
    }

    assert!(matches!(
        select_outfit(&store_a, CORE_B, &ctx, 8),
        Err(EmbodimentError::OwnershipMismatch)
    ));
    assert!(matches!(
        render_presentation_frame(&store_a, CORE_B, None, None),
        Err(EmbodimentError::OwnershipMismatch)
    ));
}

#[test]
fn hard_safety_and_identity_constraints_are_enforced() {
    let (_temp, root) = setup();
    let store_a = open_store(&root, CORE_A);
    let store_b = open_store(&root, CORE_B);

    let _avatar = import_tagged(&store_a, AssetKind::Avatar, "face", b"avatar", &[]);
    let light = import_tagged(
        &store_a,
        AssetKind::Wardrobe,
        "light tee",
        b"tee",
        &["light", "summer"],
    );
    let coat = import_tagged(
        &store_a,
        AssetKind::Wardrobe,
        "warm coat",
        b"coat",
        &["warm", "coat"],
    );
    let shell = import_tagged(
        &store_a,
        AssetKind::Wardrobe,
        "waterproof shell",
        b"shell",
        &["waterproof", "warm"],
    );
    store_a.delete_asset(&coat).expect("delete coat");

    let cold = world(
        Season::Winter,
        TimeOfDay::Morning,
        Some(weather("rain", 1.0)),
    );
    let ctx = PresentationContext::new().with_world(&cold);
    let decision = select_outfit(&store_a, CORE_A, &ctx, 5).expect("decision");

    assert!(decision.available);
    assert_eq!(decision.asset_ids, vec![shell]);
    assert_eq!(decision.considered, 3);

    let deleted = decision
        .rejected
        .iter()
        .find(|entry| entry.asset_id == coat)
        .expect("deleted rejection");
    assert_eq!(deleted.reason, "deleted");
    let unsafe_light = decision
        .rejected
        .iter()
        .find(|entry| entry.asset_id == light)
        .expect("light rejection");
    assert_eq!(unsafe_light.reason, "cold-safety");
    assert!(!decision
        .rejected
        .iter()
        .any(|entry| entry.asset_id == _avatar));

    let _light_b = import_tagged(
        &store_b,
        AssetKind::Wardrobe,
        "summer top",
        b"top",
        &["light", "summer"],
    );
    let unavailable = select_outfit(&store_b, CORE_B, &ctx, 5).expect("unavailable");
    assert!(!unavailable.available);
    assert!(unavailable.asset_ids.is_empty());
    assert!(unavailable
        .rejected
        .iter()
        .any(|entry| entry.reason == "cold-safety"));
    assert!(unavailable.rationale.contains("hard constraints"));
}

#[test]
fn required_identity_tags_and_weather_safety_beat_preference() {
    let (_temp, root) = setup();
    let store = open_store(&root, CORE_A);

    let preferred_light = import_tagged(
        &store,
        AssetKind::Wardrobe,
        "favorite summer tee",
        b"tee",
        &["preferred", "favorite", "light", "summer"],
    );
    let preferred_coat = import_tagged(
        &store,
        AssetKind::Wardrobe,
        "preferred warm coat",
        b"coat",
        &[
            "preferred",
            "favorite",
            "warm",
            "waterproof",
            "winter",
            "coat",
        ],
    );
    let armor = import_tagged(
        &store,
        AssetKind::Wardrobe,
        "approved ceremonial armor",
        b"armor",
        &["armor", "warm", "waterproof", "winter"],
    );

    let cold = world(
        Season::Winter,
        TimeOfDay::Evening,
        Some(weather("rain", -3.0)),
    );
    let required = vec!["armor".to_owned()];
    let preferences = vec!["preferred".to_owned(), "favorite".to_owned()];
    let ctx = PresentationContext::new()
        .with_world(&cold)
        .with_preference_tags(&preferences)
        .with_required_tags(&required);
    let decision = select_outfit(&store, CORE_A, &ctx, 8).expect("decision");

    assert!(decision.available);
    assert_eq!(decision.asset_ids, vec![armor]);
    let light_rejection = decision
        .rejected
        .iter()
        .find(|entry| entry.asset_id == preferred_light)
        .expect("light rejection");
    assert_eq!(light_rejection.reason, "cold-safety");
    let coat_rejection = decision
        .rejected
        .iter()
        .find(|entry| entry.asset_id == preferred_coat)
        .expect("coat rejection");
    assert!(coat_rejection.reason.starts_with("required-tag"));
    assert!(decision.constraints.iter().any(|c| c == "required-tags"));
    assert!(decision.constraints.iter().any(|c| c == "armor-identity"));
    assert!(decision.rationale.contains("constraints"));
}

#[test]
fn empty_inventory_is_honest_and_never_invents_assets() {
    let (_temp, root) = setup();
    let store = open_store(&root, CORE_B);
    let cold = world(
        Season::Winter,
        TimeOfDay::Night,
        Some(weather("snow", -10.0)),
    );
    let required = vec!["armor".to_owned()];
    let ctx = PresentationContext::new()
        .with_world(&cold)
        .with_required_tags(&required);
    let decision = select_outfit(&store, CORE_B, &ctx, 8).expect("decision");

    assert!(!decision.available);
    assert!(decision.asset_ids.is_empty());
    assert_eq!(decision.considered, 0);
    assert!(decision.rejected.is_empty());
    assert!(decision.rationale.contains("no live wardrobe"));
    assert!(decision.rationale.chars().count() <= MAX_OUTFIT_RATIONALE_CHARS);
}

#[test]
fn outfit_scoring_is_deterministic_across_repeated_calls() {
    let (_temp, root) = setup();
    let store = open_store(&root, CORE_A);

    for (name, tags) in [
        (
            "warm formal coat",
            &["identity", "warm", "waterproof", "winter", "formal"][..],
        ),
        (
            "preferred wool coat",
            &["identity", "warm", "waterproof", "winter", "preferred"],
        ),
        (
            "rain shell",
            &["identity", "warm", "waterproof", "winter", "casual"],
        ),
        (
            "dinner jacket",
            &["identity", "warm", "waterproof", "winter", "formal"],
        ),
    ] {
        import_tagged(&store, AssetKind::Wardrobe, name, name.as_bytes(), tags);
    }

    let cold = world(
        Season::Winter,
        TimeOfDay::Evening,
        Some(weather("rain", -1.0)),
    );
    let emotion = emotion(CORE_A, dimensions(0.5, 0.6, 0.7, 0.3, 0.4, 0.1), 0.8);
    let active = activity("formal dinner", ActivityStatus::Active);
    let required = vec!["identity".to_owned()];
    let personality = vec!["formal".to_owned()];
    let preferences = vec!["preferred".to_owned()];
    let ctx = PresentationContext::new()
        .with_world(&cold)
        .with_emotion(&emotion)
        .with_activity(&active)
        .with_personality_tags(&personality)
        .with_preference_tags(&preferences)
        .with_required_tags(&required);

    let first = select_outfit(&store, CORE_A, &ctx, 3).expect("first");
    assert!(first.available);
    for _ in 0..5 {
        let next = select_outfit(&store, CORE_A, &ctx, 3).expect("next");
        assert_eq!(next, first);
    }
}

#[test]
fn outfit_decisions_convert_to_bounded_personal_records() {
    let (_temp, root) = setup();
    let store = open_store(&root, CORE_A);
    let coat = import_tagged(
        &store,
        AssetKind::Wardrobe,
        "warm coat",
        b"coat",
        &["warm", "waterproof", "winter"],
    );
    let cold = world(
        Season::Winter,
        TimeOfDay::Morning,
        Some(weather("snow", -4.0)),
    );
    let ctx = PresentationContext::new().with_world(&cold);
    let decision = select_outfit(&store, CORE_A, &ctx, 8).expect("decision");
    assert_eq!(decision.asset_ids, vec![coat]);
    assert!(decision.rationale.chars().count() <= MAX_OUTFIT_RATIONALE_CHARS);

    let record = decision.to_personal_record("outfit-1", "2026-11-26T12:00:00Z");
    assert_eq!(record.id, "outfit-1");
    assert_eq!(record.decided_at, "2026-11-26T12:00:00Z");
    assert_eq!(record.asset_ids, decision.asset_ids);
    assert_eq!(record.rationale, decision.rationale);
    record.validate().expect("valid personal record");

    let mut oversized = decision;
    oversized.rationale = "r".repeat(MAX_OUTFIT_RATIONALE_CHARS * 3);
    oversized.asset_ids = (0..(MAX_OUTFIT_ASSET_IDS + 5))
        .map(|index| format!("asset-{index}"))
        .collect();
    let bounded = oversized.to_personal_record(
        "i".repeat(MAX_ID_CHARS * 2),
        "t".repeat(MAX_TIMESTAMP_CHARS * 2),
    );
    assert_eq!(bounded.id.chars().count(), MAX_ID_CHARS);
    assert_eq!(bounded.decided_at.chars().count(), MAX_TIMESTAMP_CHARS);
    assert_eq!(
        bounded.rationale.chars().count(),
        MAX_OUTFIT_RATIONALE_CHARS
    );
    assert_eq!(bounded.asset_ids.len(), MAX_OUTFIT_ASSET_IDS);
    bounded.validate().expect("bounded record valid");
}

#[test]
fn voice_projection_is_bounded_and_deterministic() {
    let neutral = emotion(CORE_A, EmotionDimensions::neutral(), 0.0);
    let joyful = emotion(CORE_A, dimensions(0.9, 0.8, 1.0, 0.0, 0.4, 0.0), 0.9);
    let calm = emotion(CORE_A, dimensions(0.5, 0.1, 0.2, 1.0, 0.2, 0.0), 0.7);
    let tense = emotion(CORE_A, dimensions(-0.2, 0.8, 0.0, 0.0, 0.1, 0.9), 0.9);
    let subdued = emotion(CORE_A, dimensions(-0.8, 0.2, 0.0, 0.1, 0.1, 0.1), 0.6);

    for state in [&neutral, &joyful, &calm, &tense, &subdued] {
        let first = voice_projection(Some(state));
        let second = voice_projection(Some(state));
        assert_eq!(first, second);
        assert!((MIN_VOICE_RATE_PERCENT..=MAX_VOICE_RATE_PERCENT).contains(&first.rate_percent));
        assert!(
            (MIN_VOICE_VOLUME_PERCENT..=MAX_VOICE_VOLUME_PERCENT).contains(&first.volume_percent)
        );
        assert!(!first.style_label.is_empty());
    }

    let neutral_projection = voice_projection(Some(&neutral));
    let joyful_projection = voice_projection(Some(&joyful));
    let calm_projection = voice_projection(Some(&calm));
    let tense_projection = voice_projection(Some(&tense));
    let subdued_projection = voice_projection(Some(&subdued));

    assert!(joyful_projection.rate_percent > neutral_projection.rate_percent);
    assert!(calm_projection.rate_percent < neutral_projection.rate_percent);
    assert_eq!(joyful_projection.style_label, "bright");
    assert_eq!(calm_projection.style_label, "calm");
    assert_eq!(tense_projection.style_label, "tense");
    assert_eq!(subdued_projection.style_label, "subdued");
    assert_eq!(neutral_projection.style_label, "neutral");
    assert_eq!(voice_projection(None), neutral_projection);
}

#[test]
fn expression_selection_uses_existing_names_only() {
    let (_temp, root) = setup();
    let store_a = open_store(&root, CORE_A);
    let store_b = open_store(&root, CORE_B);

    let joy_asset = import_tagged(&store_a, AssetKind::Expression, "joy.bin", b"joy", &[]);
    let sad_asset = import_tagged(&store_a, AssetKind::Expression, "sad.bin", b"sad", &[]);
    store_a.set_expression("Joyful", &joy_asset).expect("joy");
    store_a.set_expression("sad", &sad_asset).expect("sad");
    let appearance_a = store_a.appearance().expect("appearance a");

    let happy = emotion(CORE_A, dimensions(0.9, 0.5, 0.9, 0.2, 0.3, 0.0), 0.9);
    let down = emotion(CORE_A, dimensions(-0.8, 0.1, 0.0, 0.2, 0.1, 0.1), 0.6);

    let joyful = expression_for_emotion(&appearance_a, Some(&happy)).expect("joyful");
    assert_eq!(joyful, "Joyful");
    assert!(appearance_a.expressions.contains_key(&joyful));

    let sad = expression_for_emotion(&appearance_a, Some(&down)).expect("sad");
    assert_eq!(sad, "sad");
    assert!(appearance_a.expressions.contains_key(&sad));

    let sad_only = import_tagged(&store_b, AssetKind::Expression, "sad.bin", b"sad-b", &[]);
    store_b.set_expression("sad", &sad_only).expect("sad b");
    let appearance_b = store_b.appearance().expect("appearance b");
    assert!(expression_for_emotion(&appearance_b, Some(&happy)).is_none());
}

#[test]
fn rendered_presentation_frame_is_deterministic_and_valid() {
    let (_temp, root) = setup();
    let store_a = open_store(&root, CORE_A);
    let store_b = open_store(&root, CORE_B);

    let avatar = import_tagged(&store_a, AssetKind::Avatar, "portrait", b"avatar", &[]);
    let coat = import_tagged(
        &store_a,
        AssetKind::Wardrobe,
        "winter coat",
        b"coat",
        &["warm", "winter"],
    );
    let happy = import_tagged(&store_a, AssetKind::Expression, "happy.bin", b"happy", &[]);
    store_a.set_avatar(Some(&avatar)).expect("avatar");
    store_a.set_wardrobe(vec![coat.clone()]).expect("wardrobe");
    store_a.set_expression("happy", &happy).expect("expression");

    let autumn = world(Season::Autumn, TimeOfDay::Midday, None);
    let emotion = emotion(CORE_A, dimensions(0.8, 0.4, 0.8, 0.3, 0.2, 0.0), 0.7);

    let frame_a = render_presentation_frame(&store_a, CORE_A, Some(&autumn), Some(&emotion))
        .expect("frame a");
    let frame_b = render_presentation_frame(&store_a, CORE_A, Some(&autumn), Some(&emotion))
        .expect("frame b");

    assert_eq!(frame_a, frame_b);
    assert_eq!(frame_a.schema, PRESENTATION_FRAME_SCHEMA);
    assert_eq!(frame_a.core_uuid, CORE_A);
    assert_eq!(frame_a.expression.as_deref(), Some("happy"));
    assert_eq!(frame_a.width, PRESENTATION_FRAME_WIDTH);
    assert_eq!(frame_a.height, PRESENTATION_FRAME_HEIGHT);
    assert_eq!(frame_a.season, Some(Season::Autumn));
    assert_eq!(frame_a.time_of_day, Some(TimeOfDay::Midday));
    assert!(frame_a
        .scene
        .layers
        .iter()
        .any(|layer| layer.asset_id == coat));
    let bmp = frame_a.bmp.as_ref().expect("bmp");
    assert_eq!(&bmp[0..2], &BMP_MAGIC);

    let empty_a = render_presentation_frame(&store_b, CORE_B, None, None).expect("empty frame");
    assert!(empty_a.scene.layers.is_empty());
    assert!(empty_a.bmp.is_none());
    assert!(empty_a.expression.is_none());
    assert_eq!(empty_a.season, None);
}

#[test]
fn outfit_limit_is_honored_and_bounded() {
    let (_temp, root) = setup();
    let store = open_store(&root, CORE_A);
    for index in 0..(MAX_OUTFIT_SELECTION + 2) {
        import_tagged(
            &store,
            AssetKind::Wardrobe,
            &format!("casual item {index}"),
            format!("item-{index}").as_bytes(),
            &["casual"],
        );
    }
    let active = activity("casual work", ActivityStatus::Active);
    let ctx = PresentationContext::new().with_activity(&active);
    let limited = select_outfit(&store, CORE_A, &ctx, 3).expect("limited");
    assert_eq!(limited.asset_ids.len(), 3);
    let capped = select_outfit(&store, CORE_A, &ctx, usize::MAX).expect("capped");
    assert_eq!(capped.asset_ids.len(), MAX_OUTFIT_SELECTION);
    let none = select_outfit(&store, CORE_A, &ctx, 0).expect("none");
    assert!(none.available);
    assert!(none.asset_ids.is_empty());

    let first = select_outfit(&store, CORE_A, &ctx, 3).expect("first");
    let second = select_outfit(&store, CORE_A, &ctx, 3).expect("second");
    assert_eq!(first, second);
}
