use std::path::Path;

use aegis_contracts::world::{
    FreshnessState, Season, SunTimes, TimeOfDay, WeatherForecast, WeatherObservation, WorldSnapshot,
};
use aegis_platform::{
    PlatformService, UnavailableWeatherProvider, WeatherData, WeatherProvider, WeatherRequest,
    WeatherUnavailable, WorldClock, WorldError, WorldService,
};
use aegis_storage::{open_read_only, StoragePaths, PLATFORM_APPLICATION_ID};
use chrono::{DateTime, Duration, Utc};

struct FixedWeatherProvider;

impl WeatherProvider for FixedWeatherProvider {
    fn name(&self) -> &'static str {
        "fixed_test"
    }

    fn observe(&self, request: &WeatherRequest) -> Result<WeatherData, WeatherUnavailable> {
        Ok(WeatherData {
            observation: Some(WeatherObservation {
                condition: "clear".to_owned(),
                temperature_celsius: Some(21.0),
                feels_like_celsius: Some(20.5),
                humidity_percent: Some(40),
                wind_speed_kph: Some(5.5),
                observed_at: Some(request.now.to_rfc3339()),
            }),
            forecast: vec![WeatherForecast {
                at: request.now.format("%Y-%m-%d").to_string(),
                condition: "clear".to_owned(),
                high_celsius: Some(24.0),
                low_celsius: Some(14.0),
                precipitation_probability_percent: Some(5),
            }],
            sun: Some(SunTimes {
                sunrise: Some("06:45".to_owned()),
                sunset: Some("17:10".to_owned()),
            }),
        })
    }
}

struct InvalidWeatherProvider;

impl WeatherProvider for InvalidWeatherProvider {
    fn name(&self) -> &'static str {
        "invalid_test"
    }

    fn observe(&self, _request: &WeatherRequest) -> Result<WeatherData, WeatherUnavailable> {
        Ok(WeatherData {
            observation: Some(WeatherObservation {
                condition: "storm".to_owned(),
                temperature_celsius: Some(15.0),
                feels_like_celsius: None,
                humidity_percent: Some(200),
                wind_speed_kph: None,
                observed_at: None,
            }),
            forecast: Vec::new(),
            sun: None,
        })
    }
}

fn instant(value: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(value)
        .expect("fixed instant")
        .with_timezone(&Utc)
}

fn paths(root: &Path) -> StoragePaths {
    StoragePaths::new(root).expect("storage paths")
}

fn initialized_world(root: &Path) -> (WorldService, String) {
    let initialized = PlatformService::initialize(root).expect("initialize platform");
    let world = WorldService::start(paths(root)).expect("start world service");
    (world, initialized.owner_token)
}

#[test]
fn chicago_dst_spring_forward_and_fall_back_are_correct() {
    let clock =
        WorldClock::new("America/Chicago", Some(41.8781), Some(-87.6298)).expect("chicago clock");

    let before_spring = clock
        .clock_at(instant("2026-03-08T07:59:59Z"))
        .expect("before spring");
    let after_spring = clock
        .clock_at(instant("2026-03-08T08:00:00Z"))
        .expect("after spring");
    assert_eq!(before_spring.utc_offset_seconds, -6 * 3600);
    assert_eq!(before_spring.local_time, "01:59:59");
    assert_eq!(before_spring.time_of_day, TimeOfDay::Night);
    assert_eq!(after_spring.utc_offset_seconds, -5 * 3600);
    assert_eq!(after_spring.local_time, "03:00:00");
    assert_eq!(after_spring.local_date, "2026-03-08");

    let before_fall = clock
        .clock_at(instant("2026-11-01T06:59:59Z"))
        .expect("before fall");
    let after_fall = clock
        .clock_at(instant("2026-11-01T07:00:00Z"))
        .expect("after fall");
    assert_eq!(before_fall.utc_offset_seconds, -5 * 3600);
    assert_eq!(before_fall.local_time, "01:59:59");
    assert_eq!(after_fall.utc_offset_seconds, -6 * 3600);
    assert_eq!(after_fall.local_time, "01:00:00");
}

#[test]
fn time_of_day_and_season_follow_local_clock_and_hemisphere() {
    let utc = WorldClock::new("UTC", None, None).expect("utc clock");
    let midsummer = utc
        .clock_at(instant("2026-07-15T12:00:00Z"))
        .expect("midsummer");
    assert_eq!(midsummer.local_time, "12:00:00");
    assert_eq!(midsummer.time_of_day, TimeOfDay::Midday);
    assert_eq!(midsummer.season, Season::Summer);

    let sydney =
        WorldClock::new("Australia/Sydney", Some(-33.8688), Some(151.2093)).expect("sydney clock");
    let southern_winter = sydney
        .clock_at(instant("2026-07-15T02:00:00Z"))
        .expect("southern winter");
    assert_eq!(southern_winter.local_time, "12:00:00");
    assert_eq!(southern_winter.time_of_day, TimeOfDay::Midday);
    assert_eq!(southern_winter.season, Season::Winter);
}

#[test]
fn config_defaults_and_owner_gated_timezone_and_location() {
    let temp = tempfile::tempdir().expect("tempdir");
    let (world, owner_token) = initialized_world(temp.path());

    let config = world.config().expect("default config");
    assert_eq!(config.timezone, "UTC");
    assert_eq!(config.stale_after_seconds, 900);
    assert_eq!(config.latitude, None);
    assert_eq!(config.longitude, None);

    assert!(matches!(
        world.set_timezone("wrong-token", "Europe/Paris"),
        Err(WorldError::Unauthorized)
    ));
    assert!(matches!(
        world.set_timezone(&owner_token, "Not/AZone"),
        Err(WorldError::InvalidTimezone(_))
    ));

    world
        .set_timezone(&owner_token, "Europe/Paris")
        .expect("set timezone");
    world
        .set_location(&owner_token, 48.8566, 2.3522)
        .expect("set location");

    let config = world.config().expect("updated config");
    assert_eq!(config.timezone, "Europe/Paris");
    assert!((config.latitude.expect("latitude") - 48.8566).abs() < 1e-9);
    assert!((config.longitude.expect("longitude") - 2.3522).abs() < 1e-9);

    assert!(matches!(
        world.set_location(&owner_token, 91.0, 2.3522),
        Err(WorldError::InvalidLocation)
    ));
}

#[test]
fn snapshot_is_valid_stored_and_recomputes_freshness() {
    let temp = tempfile::tempdir().expect("tempdir");
    let (world, _) = initialized_world(temp.path());
    let at = instant("2026-11-26T18:00:00Z");

    let snapshot = world
        .snapshot_at(&FixedWeatherProvider, at)
        .expect("world snapshot");
    snapshot.validate().expect("valid snapshot");
    assert_eq!(snapshot.timezone, "UTC");
    assert_eq!(snapshot.time_of_day, TimeOfDay::Evening);
    assert_eq!(snapshot.season, Season::Autumn);
    assert_eq!(snapshot.freshness, FreshnessState::Fresh);
    assert_eq!(snapshot.provenance.source, "fixed_test");
    assert_eq!(snapshot.stale_after_seconds, 900);
    assert_eq!(snapshot.forecast.len(), 1);
    assert!(snapshot.calendar.is_empty());
    let weather = snapshot.weather.as_ref().expect("weather");
    assert_eq!(weather.condition, "clear");
    assert_eq!(weather.humidity_percent, Some(40));

    let latest = world
        .latest_at(at)
        .expect("latest")
        .expect("stored snapshot");
    assert_eq!(latest, snapshot);

    let stale = world
        .latest_at(at + Duration::seconds(901))
        .expect("later")
        .expect("stored snapshot");
    assert_eq!(stale.freshness, FreshnessState::Stale);
    assert_eq!(stale.weather, snapshot.weather);

    let storage = paths(temp.path());
    let platform = open_read_only(&storage.platform_db(), PLATFORM_APPLICATION_ID)
        .expect("platform read-only");
    let contexts: i64 = platform
        .query_row("SELECT COUNT(*) FROM world_context", [], |row| row.get(0))
        .expect("context count");
    assert_eq!(contexts, 1);
    let events: i64 = platform
        .query_row("SELECT COUNT(*) FROM world_events", [], |row| row.get(0))
        .expect("event count");
    assert_eq!(events, 1);
}

#[test]
fn snapshot_store_survives_reopen() {
    let temp = tempfile::tempdir().expect("tempdir");
    let at = instant("2026-11-26T18:00:00Z");
    let snapshot = {
        let (world, _) = initialized_world(temp.path());
        world
            .snapshot_at(&FixedWeatherProvider, at)
            .expect("world snapshot")
    };

    let reopened = WorldService::start(paths(temp.path())).expect("reopen world");
    let latest = reopened
        .latest_at(at)
        .expect("latest")
        .expect("persisted snapshot");
    assert_eq!(latest, snapshot);
    assert_eq!(latest.provenance.source, "fixed_test");
}

#[test]
fn unavailable_provider_is_honest_and_does_not_break_snapshotting() {
    let temp = tempfile::tempdir().expect("tempdir");
    let (world, _) = initialized_world(temp.path());
    let at = instant("2026-11-26T18:00:00Z");

    let snapshot = world
        .snapshot_at(&UnavailableWeatherProvider::new(), at)
        .expect("snapshot without weather");
    snapshot.validate().expect("valid snapshot");
    assert!(snapshot.weather.is_none());
    assert!(snapshot.forecast.is_empty());
    assert!(snapshot.sun.is_none());
    assert_eq!(snapshot.freshness, FreshnessState::Unavailable);
    assert_eq!(snapshot.provenance.source, "unavailable");
    assert!(snapshot
        .provenance
        .note
        .as_deref()
        .expect("reason")
        .contains("no weather provider"));
    assert_eq!(snapshot.local_time, "18:00:00");
    assert!(world.latest_at(at).expect("latest").is_some());
}

#[test]
fn malformed_provider_data_falls_back_to_unavailable() {
    let temp = tempfile::tempdir().expect("tempdir");
    let (world, _) = initialized_world(temp.path());
    let at = instant("2026-11-26T18:00:00Z");

    let snapshot = world
        .snapshot_at(&InvalidWeatherProvider, at)
        .expect("snapshot rejects provider data without failing");
    snapshot.validate().expect("valid snapshot");
    assert!(snapshot.weather.is_none());
    assert_eq!(snapshot.freshness, FreshnessState::Unavailable);
    assert!(snapshot
        .provenance
        .note
        .as_deref()
        .expect("reason")
        .contains("provider data rejected"));
}

#[test]
fn multiple_cores_receive_the_same_shared_snapshot() {
    let temp = tempfile::tempdir().expect("tempdir");
    let (world, _) = initialized_world(temp.path());

    let generated = world
        .snapshot(&FixedWeatherProvider)
        .expect("generated snapshot");
    let first_reader = world
        .snapshot_for_core(&FixedWeatherProvider)
        .expect("first core");
    let second_reader = world
        .snapshot_for_core(&UnavailableWeatherProvider::new())
        .expect("second core");
    assert_eq!(first_reader, generated);
    assert_eq!(second_reader, generated);

    let shared = world
        .shared_snapshot()
        .expect("shared snapshot")
        .expect("snapshot exists");
    assert_eq!(shared, generated);

    let storage = paths(temp.path());
    let platform = open_read_only(&storage.platform_db(), PLATFORM_APPLICATION_ID)
        .expect("platform read-only");
    let events: i64 = platform
        .query_row("SELECT COUNT(*) FROM world_events", [], |row| row.get(0))
        .expect("event count");
    assert_eq!(events, 1);
}

#[test]
fn world_snapshot_contract_rejects_unbounded_values() {
    let temp = tempfile::tempdir().expect("tempdir");
    let (world, _) = initialized_world(temp.path());
    let at = instant("2026-11-26T18:00:00Z");
    let mut snapshot: WorldSnapshot = world
        .snapshot_at(&FixedWeatherProvider, at)
        .expect("world snapshot");

    snapshot.timezone.clear();
    assert!(snapshot.validate().is_err());
}
