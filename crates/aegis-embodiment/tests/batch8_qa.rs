//! Batch 8 / Milestone 7 independent internal QA for deterministic embodiment
//! presentation: wardrobe selection, expressions, voice, and ownership safety.

use std::path::{Path, PathBuf};

use aegis_contracts::initialization::{CoreInitializationRequest, CORE_INIT_REQUEST_SCHEMA};
use aegis_contracts::personal::{EmotionDimensions, EmotionState, EMOTION_STATE_SCHEMA};
use aegis_contracts::world::{
    FreshnessState, Provenance, Season, TimeOfDay, WeatherObservation, WorldSnapshot,
    WORLD_SNAPSHOT_SCHEMA,
};
use aegis_core_install::CoreInstaller;
use aegis_embodiment::{
    expression_for_emotion, render_presentation_frame, select_outfit, voice_projection, AssetKind,
    EmbodimentError, EmbodimentStore, NewAsset, PresentationContext,
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

fn all_asset_ids(store: &EmbodimentStore) -> Vec<String> {
    store
        .list_assets(None, true)
        .expect("assets")
        .into_iter()
        .map(|asset| asset.asset_id)
        .collect()
}

#[test]
fn same_world_different_wardrobe_and_emotion_change_presentation() {
    let (_temp, root) = setup();
    let store_a = open_store(&root, CORE_A);
    let store_b = open_store(&root, CORE_B);

    let coat = import_tagged(
        &store_a,
        AssetKind::Wardrobe,
        "winter coat",
        b"a-coat",
        &["warm", "waterproof", "winter"],
    );
    let _dress = import_tagged(
        &store_a,
        AssetKind::Wardrobe,
        "sun dress",
        b"a-dress",
        &["light", "summer"],
    );
    let happy_asset = import_tagged(&store_a, AssetKind::Expression, "happy.bin", b"happy", &[]);
    let sad_asset = import_tagged(&store_a, AssetKind::Expression, "sad.bin", b"sad", &[]);
    store_a
        .set_expression("happy", &happy_asset)
        .expect("happy");
    store_a.set_expression("sad", &sad_asset).expect("sad");

    let suit = import_tagged(
        &store_b,
        AssetKind::Wardrobe,
        "business suit",
        b"b-suit",
        &["warm", "waterproof", "formal"],
    );
    let _hoodie = import_tagged(
        &store_b,
        AssetKind::Wardrobe,
        "cozy hoodie",
        b"b-hoodie",
        &["warm", "waterproof", "cozy"],
    );
    let sad_b = import_tagged(&store_b, AssetKind::Expression, "sad.bin", b"sad-b", &[]);
    store_b.set_expression("sad", &sad_b).expect("sad b");

    let cold = weather("rain", 0.0);
    let shared_world = world(Season::Autumn, TimeOfDay::Evening, Some(cold));
    let emotion_a = emotion(CORE_A, dimensions(0.8, 0.5, 0.9, 0.3, 0.2, 0.0));
    let emotion_b = emotion(CORE_B, dimensions(-0.8, 0.3, 0.0, 0.1, 0.1, 0.2));

    let ctx_a = PresentationContext::new()
        .with_world(&shared_world)
        .with_emotion(&emotion_a);
    let ctx_b = PresentationContext::new()
        .with_world(&shared_world)
        .with_emotion(&emotion_b);

    let decision_a = select_outfit(&store_a, CORE_A, &ctx_a, 2).expect("decision a");
    let decision_b = select_outfit(&store_b, CORE_B, &ctx_b, 2).expect("decision b");
    assert_eq!(decision_a.asset_ids, vec![coat]);
    assert_eq!(decision_b.asset_ids.first(), Some(&suit));
    assert_ne!(decision_a.asset_ids, decision_b.asset_ids);

    // No cross-Core leakage in either direction.
    let assets_a = all_asset_ids(&store_a);
    let assets_b = all_asset_ids(&store_b);
    for asset_id in &decision_a.asset_ids {
        assert!(assets_a.contains(asset_id));
        assert!(!assets_b.contains(asset_id));
    }
    for asset_id in &decision_b.asset_ids {
        assert!(assets_b.contains(asset_id));
        assert!(!assets_a.contains(asset_id));
    }

    let appearance_a = store_a.appearance().expect("appearance a");
    let appearance_b = store_b.appearance().expect("appearance b");
    assert_eq!(
        expression_for_emotion(&appearance_a, Some(&emotion_a)).as_deref(),
        Some("happy")
    );
    assert_eq!(
        expression_for_emotion(&appearance_b, Some(&emotion_b)).as_deref(),
        Some("sad")
    );

    let voice_a = voice_projection(Some(&emotion_a));
    let voice_b = voice_projection(Some(&emotion_b));
    assert_ne!(voice_a.style_label, voice_b.style_label);
    assert_eq!(voice_a.style_label, "bright");
    assert_eq!(voice_b.style_label, "subdued");

    let frame_a =
        render_presentation_frame(&store_a, CORE_A, Some(&shared_world), Some(&emotion_a))
            .expect("frame a");
    assert_eq!(frame_a.core_uuid, CORE_A);
    assert_eq!(frame_a.expression.as_deref(), Some("happy"));
    assert_eq!(frame_a.season, Some(Season::Autumn));
    assert_eq!(frame_a.time_of_day, Some(TimeOfDay::Evening));
}

#[test]
fn weather_safety_identity_and_cross_core_constraints_are_enforced() {
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
    assert_eq!(decision.considered, 3, "non-wardrobe assets are excluded");
    assert!(decision
        .constraints
        .iter()
        .any(|value| value == "cold-safety"));
    assert!(decision
        .constraints
        .iter()
        .any(|value| value == "precipitation-safety"));
    assert!(decision
        .rejected
        .iter()
        .any(|entry| entry.asset_id == coat && entry.reason == "deleted"));
    assert!(decision
        .rejected
        .iter()
        .any(|entry| entry.asset_id == light && entry.reason == "cold-safety"));
    assert!(!decision
        .rejected
        .iter()
        .any(|entry| entry.asset_id == _avatar));

    let _summer = import_tagged(
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

    assert!(matches!(
        select_outfit(&store_a, CORE_B, &ctx, 5),
        Err(EmbodimentError::OwnershipMismatch)
    ));
    assert!(matches!(
        render_presentation_frame(&store_a, CORE_B, Some(&cold), None),
        Err(EmbodimentError::OwnershipMismatch)
    ));
}

#[test]
fn voice_is_bounded_for_nonfinite_emotion_and_expressions_are_never_invented() {
    let (_temp, root) = setup();
    let store = open_store(&root, CORE_A);

    let pathological = emotion(
        CORE_A,
        dimensions(
            f32::NAN,
            f32::INFINITY,
            -1.0,
            5.0,
            f32::NEG_INFINITY,
            f32::NAN,
        ),
    );
    let first = voice_projection(Some(&pathological));
    let second = voice_projection(Some(&pathological));
    assert_eq!(first, second);
    assert!(first.rate_percent >= aegis_embodiment::MIN_VOICE_RATE_PERCENT);
    assert!(first.rate_percent <= aegis_embodiment::MAX_VOICE_RATE_PERCENT);
    assert!(first.volume_percent >= aegis_embodiment::MIN_VOICE_VOLUME_PERCENT);
    assert!(first.volume_percent <= aegis_embodiment::MAX_VOICE_VOLUME_PERCENT);
    assert!(!first.style_label.is_empty());

    let strong = emotion(CORE_A, dimensions(1.0, 1.0, 1.0, 1.0, 1.0, 1.0));
    let appearance = store.appearance().expect("appearance");
    assert!(appearance.expressions.is_empty());
    assert!(
        expression_for_emotion(&appearance, Some(&strong)).is_none(),
        "an absent expression name must never be invented"
    );
}
