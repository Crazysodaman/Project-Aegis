//! Batch 8 / Milestone 7 independent internal QA for shared world snapshots,
//! per-Core personal state, ownership rejection, and continuity.
//!
//! Only public APIs are used. Every database lives in a fresh temporary root.

use std::fs;
use std::path::{Path, PathBuf};

use aegis_contracts::initialization::{CoreInitializationRequest, CORE_INIT_REQUEST_SCHEMA};
use aegis_contracts::world::{SunTimes, WeatherForecast, WeatherObservation};
use aegis_core_install::CoreInstaller;
use aegis_core_sdk::{
    Activity, ActivityStatus, CoreSdkContext, EmotionDelta, EmotionInfluence, PersonalStore,
    SdkError,
};
use aegis_core_storage::{CoreDatabase, CoreStorageLayout};
use aegis_platform::{PlatformService, WeatherData, WeatherProvider, WeatherRequest, WorldService};
use aegis_storage::{now_rfc3339, StoragePaths};
use chrono::{DateTime, Duration, Utc};

const CORE_A: &str = "a9a0a604-a4e7-4a88-a985-3f071e2c8e7d";
const CORE_B: &str = "11111111-1111-4111-8111-111111111111";

struct Fixture {
    _temp: tempfile::TempDir,
    root: PathBuf,
}

struct FixedWeatherProvider;

impl WeatherProvider for FixedWeatherProvider {
    fn name(&self) -> &'static str {
        "fixed_batch8_qa"
    }

    fn observe(
        &self,
        request: &WeatherRequest,
    ) -> Result<WeatherData, aegis_platform::WeatherUnavailable> {
        Ok(WeatherData {
            observation: Some(WeatherObservation {
                condition: "clear".to_owned(),
                temperature_celsius: Some(18.0),
                feels_like_celsius: Some(17.5),
                humidity_percent: Some(45),
                wind_speed_kph: Some(4.0),
                observed_at: Some(request.now.to_rfc3339()),
            }),
            forecast: vec![WeatherForecast {
                at: request.now.format("%Y-%m-%d").to_string(),
                condition: "clear".to_owned(),
                high_celsius: Some(20.0),
                low_celsius: Some(10.0),
                precipitation_probability_percent: Some(10),
            }],
            sun: Some(SunTimes {
                sunrise: Some("06:40".to_owned()),
                sunset: Some("17:05".to_owned()),
            }),
        })
    }
}

fn request(core_uuid: &str, core_number: u8, display_name: &str) -> CoreInitializationRequest {
    CoreInitializationRequest {
        schema: CORE_INIT_REQUEST_SCHEMA.to_owned(),
        core_uuid: core_uuid.to_owned(),
        core_number,
        display_name: display_name.to_owned(),
        requested_at: now_rfc3339().expect("timestamp"),
    }
}

fn fixture() -> Fixture {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path().to_path_buf();
    let initialized = PlatformService::initialize(&root).expect("platform initialize");
    let installer = CoreInstaller::new(&root).expect("installer");
    installer
        .initialize(&request(CORE_A, 1, "Core A"), &initialized.owner_token)
        .expect("install A");
    installer
        .initialize(&request(CORE_B, 2, "Core B"), &initialized.owner_token)
        .expect("install B");
    Fixture { _temp: temp, root }
}

fn layout(root: &Path) -> CoreStorageLayout {
    CoreStorageLayout::new(root).expect("layout")
}

fn paths(root: &Path) -> StoragePaths {
    StoragePaths::new(root).expect("storage paths")
}

fn core_dir(root: &Path, core_uuid: &str) -> PathBuf {
    layout(root).core_dir(core_uuid).expect("core dir")
}

fn instant(value: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(value)
        .expect("fixed instant")
        .with_timezone(&Utc)
}

#[allow(clippy::too_many_arguments)]
fn influence(
    source: &str,
    weight: f32,
    valence: f32,
    arousal: f32,
    joy: f32,
    calm: f32,
    curiosity: f32,
    tension: f32,
) -> EmotionInfluence {
    EmotionInfluence {
        source: source.to_owned(),
        weight,
        delta: EmotionDelta {
            valence,
            arousal,
            joy,
            calm,
            curiosity,
            tension,
        },
    }
}

fn activity(id: &str, title: &str, at: &str) -> Activity {
    Activity {
        id: id.to_owned(),
        title: title.to_owned(),
        status: ActivityStatus::Active,
        started_at: at.to_owned(),
        updated_at: at.to_owned(),
        notes: String::new(),
    }
}

#[test]
fn cores_share_one_world_snapshot_but_keep_distinct_emotions_across_reopen() {
    let fixture = fixture();
    let at = instant("2026-11-26T18:00:00Z");

    let snapshot = {
        let world = WorldService::start(paths(&fixture.root)).expect("world");
        let snapshot = world
            .snapshot_at(&FixedWeatherProvider, at)
            .expect("generated snapshot");

        let first_core_view = world
            .snapshot_for_core(&FixedWeatherProvider)
            .expect("first core view");
        let second_core_view = world
            .snapshot_for_core(&FixedWeatherProvider)
            .expect("second core view");
        assert_eq!(
            first_core_view, second_core_view,
            "every Core must receive the same stored world snapshot"
        );
        assert_eq!(first_core_view.generated_at, snapshot.generated_at);
        assert_eq!(first_core_view.weather, snapshot.weather);
        assert_eq!(first_core_view.provenance, snapshot.provenance);

        assert_eq!(
            world.latest_at(at).expect("latest").expect("stored"),
            snapshot,
            "the deterministic view must equal the generated snapshot"
        );
        snapshot
    };

    let state_a = {
        let store = PersonalStore::open(&fixture.root, CORE_A).expect("A personal");
        store
            .apply_influences_at(
                &[influence("praise", 1.0, 0.6, 0.0, 0.8, 0.0, 0.0, 0.0)],
                at,
            )
            .expect("A influence")
    };
    let state_b = {
        let store = PersonalStore::open(&fixture.root, CORE_B).expect("B personal");
        store
            .apply_influences_at(
                &[influence("stress", 1.0, -0.4, 0.6, 0.0, 0.0, 0.0, 0.7)],
                at,
            )
            .expect("B influence")
    };
    assert_ne!(
        state_a.dimensions, state_b.dimensions,
        "the same instant must allow different personal emotion states"
    );
    assert!(state_a.dimensions.joy > 0.0);
    assert_eq!(state_b.dimensions.joy, 0.0);
    assert!(state_b.dimensions.tension > 0.0);
    assert_eq!(state_a.dimensions.tension, 0.0);

    // Reopen world and both personal stores: nothing is lost or cross-wired.
    let reopened_world = WorldService::start(paths(&fixture.root)).expect("reopen world");
    assert_eq!(
        reopened_world
            .latest_at(at)
            .expect("latest")
            .expect("persisted"),
        snapshot
    );

    let reopened_a = PersonalStore::open(&fixture.root, CORE_A).expect("reopen A");
    let reopened_b = PersonalStore::open(&fixture.root, CORE_B).expect("reopen B");
    assert_eq!(
        reopened_a.emotion_state().expect("A state").dimensions,
        state_a.dimensions
    );
    assert_eq!(
        reopened_b.emotion_state().expect("B state").dimensions,
        state_b.dimensions
    );
    assert_ne!(
        reopened_a.emotion_state().expect("A state").dimensions,
        reopened_b.emotion_state().expect("B state").dimensions
    );
}

#[test]
fn copied_personal_database_is_rejected_for_another_core() {
    let fixture = fixture();
    {
        let store = PersonalStore::open(&fixture.root, CORE_A).expect("A personal");
        store
            .apply_influences_at(
                &[influence("warmth", 0.5, 0.2, 0.0, 0.2, 0.0, 0.0, 0.0)],
                Utc::now(),
            )
            .expect("influence");
    }

    let a_path = core_dir(&fixture.root, CORE_A).join("personal.db");
    let b_path = core_dir(&fixture.root, CORE_B).join("personal.db");
    assert!(a_path.is_file());
    assert!(!b_path.exists());
    fs::copy(&a_path, &b_path).expect("copy A personal database over B");

    let error = PersonalStore::open(&fixture.root, CORE_B).expect_err("B must reject A's db");
    assert!(matches!(error, SdkError::OwnershipMismatch));

    // Core A's own database is untouched and still opens normally.
    let original = PersonalStore::open(&fixture.root, CORE_A).expect("A still opens");
    assert_eq!(original.core_uuid(), CORE_A);
}

#[test]
fn continuity_elapsed_offline_is_exact_across_reopen_and_bounded() {
    let fixture = fixture();
    let t_past = instant("2026-01-01T00:00:00Z");
    let t_future = instant("2026-01-03T00:00:00Z");

    {
        let store = PersonalStore::open(&fixture.root, CORE_A).expect("personal");
        store
            .update_last_active_at(Some("session-42"), t_past)
            .expect("last active");
        for index in 0..10 {
            store
                .open_thread(&format!("thread {index}"))
                .expect("thread");
        }
        for index in 0..12 {
            store
                .upsert_activity(&activity(
                    &format!("act-{index:02}"),
                    &format!("Activity {index}"),
                    &t_past.to_rfc3339(),
                ))
                .expect("activity");
        }
        for index in 0..10 {
            store
                .record_experience_at(&format!("experience {index}"), t_past)
                .expect("experience");
        }
    }

    let reopened = PersonalStore::open(&fixture.root, CORE_A).expect("reopen");
    let summary = reopened
        .continuity_summary_at(t_future)
        .expect("continuity summary");
    assert_eq!(summary.last_active_at, t_past.to_rfc3339());
    assert_eq!(summary.last_session_id.as_deref(), Some("session-42"));
    assert_eq!(summary.elapsed_offline_seconds, 2 * 24 * 60 * 60);
    assert_eq!(summary.recent_experiences.len(), 8);
    assert_eq!(summary.open_threads.len(), 8);
    assert_eq!(summary.activities.len(), 8);

    let before_last_active = reopened
        .continuity_summary_at(t_past - Duration::hours(1))
        .expect("earlier summary");
    assert_eq!(before_last_active.elapsed_offline_seconds, 0);
}

#[test]
fn core_identity_is_unchanged_across_sdk_reopen_and_world_start() {
    let fixture = fixture();
    let layout = layout(&fixture.root);
    let before = CoreDatabase::read_snapshot(&layout, CORE_A).expect("identity before");

    let context = CoreSdkContext::open(&fixture.root, CORE_A).expect("sdk context");
    assert_eq!(context.identity.core_uuid, CORE_A);

    WorldService::start(paths(&fixture.root)).expect("world start");
    let after = CoreDatabase::read_snapshot(&layout, CORE_A).expect("identity after");

    assert_eq!(before.identity.core_uuid, after.identity.core_uuid);
    assert_eq!(before.identity.core_number, after.identity.core_number);
    assert_eq!(before.identity.display_name, after.identity.display_name);
    assert_eq!(
        before.metadata.database_instance_uuid,
        after.metadata.database_instance_uuid
    );
    assert_eq!(after.identity.core_uuid, CORE_A);
    assert_eq!(after.identity.core_number, 1);
    assert_eq!(after.identity.display_name, "Core A");
}
