//! Batch 8 / Milestone 7 independent internal QA for the Platform world layer.
//!
//! These tests are adversarial checks for weather-failure honesty, freshness
//! recomputation, and IANA/DST clock correctness. They only use the public
//! `aegis-platform` surface and never touch live `var/` state.

use std::path::Path;

use aegis_contracts::world::{
    FreshnessState, Season, SunTimes, TimeOfDay, WeatherForecast, WeatherObservation,
};
use aegis_platform::{
    PlatformService, WeatherData, WeatherProvider, WeatherRequest, WeatherUnavailable, WorldClock,
    WorldService,
};
use aegis_storage::StoragePaths;
use chrono::{DateTime, Duration, Utc};

struct FailingWeatherProvider;

impl WeatherProvider for FailingWeatherProvider {
    fn name(&self) -> &'static str {
        "failing_test_provider"
    }

    fn observe(&self, _request: &WeatherRequest) -> Result<WeatherData, WeatherUnavailable> {
        Err(WeatherUnavailable::new("network is offline"))
    }
}

struct FixedWeatherProvider;

impl WeatherProvider for FixedWeatherProvider {
    fn name(&self) -> &'static str {
        "fixed_test_provider"
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

fn instant(value: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(value)
        .expect("fixed instant")
        .with_timezone(&Utc)
}

fn paths(root: &Path) -> StoragePaths {
    StoragePaths::new(root).expect("storage paths")
}

fn initialized_world(root: &Path) -> WorldService {
    PlatformService::initialize(root).expect("initialize platform");
    WorldService::start(paths(root)).expect("start world service")
}

#[test]
fn failing_weather_provider_is_honest_and_platform_still_works() {
    let temp = tempfile::tempdir().expect("tempdir");
    let world = initialized_world(temp.path());
    let at = instant("2026-11-26T18:00:00Z");

    let snapshot = world
        .snapshot_at(&FailingWeatherProvider, at)
        .expect("snapshot survives provider failure");
    snapshot.validate().expect("unavailable snapshot is valid");

    assert!(snapshot.weather.is_none(), "no weather may be invented");
    assert!(snapshot.forecast.is_empty(), "no forecast may be invented");
    assert!(snapshot.sun.is_none(), "no sun times may be invented");
    assert_eq!(snapshot.freshness, FreshnessState::Unavailable);
    assert_eq!(snapshot.provenance.source, "unavailable");
    assert_eq!(
        snapshot.provenance.provider.as_deref(),
        Some("failing_test_provider")
    );
    assert!(snapshot
        .provenance
        .note
        .as_deref()
        .expect("failure reason is recorded")
        .contains("network is offline"));

    // The clock is still authoritative even when weather is not.
    assert_eq!(snapshot.timezone, "UTC");
    assert_eq!(snapshot.local_time, "18:00:00");
    assert_eq!(snapshot.local_date, "2026-11-26");
    assert_eq!(snapshot.time_of_day, TimeOfDay::Evening);
    assert_eq!(snapshot.season, Season::Autumn);
    assert_eq!(snapshot.utc_offset_seconds, 0);
    assert_eq!(snapshot.generated_at, at.to_rfc3339());

    // The Platform still starts and reports status after a provider failure.
    let service = PlatformService::start(temp.path()).expect("platform restart");
    assert!(service.status().expect("status").zero_core);
    assert_eq!(world.clock().expect("world clock").timezone(), "UTC");

    // An unavailable snapshot stays unavailable no matter how old it is.
    let old = world
        .latest_at(at + Duration::days(30))
        .expect("latest")
        .expect("stored snapshot");
    assert_eq!(old.freshness, FreshnessState::Unavailable);
    assert!(old.weather.is_none());
}

#[test]
fn weather_freshness_is_recomputed_from_snapshot_age() {
    let temp = tempfile::tempdir().expect("tempdir");
    let world = initialized_world(temp.path());
    let at = instant("2026-11-26T18:00:00Z");

    let snapshot = world
        .snapshot_at(&FixedWeatherProvider, at)
        .expect("snapshot with weather");
    assert_eq!(snapshot.freshness, FreshnessState::Fresh);
    let weather = snapshot.weather.clone();

    let exactly_threshold = world
        .latest_at(at + Duration::seconds(900))
        .expect("latest")
        .expect("stored");
    assert_eq!(exactly_threshold.freshness, FreshnessState::Fresh);
    assert_eq!(exactly_threshold.weather, weather);

    let just_stale = world
        .latest_at(at + Duration::seconds(901))
        .expect("latest")
        .expect("stored");
    assert_eq!(just_stale.freshness, FreshnessState::Stale);
    assert_eq!(just_stale.weather, weather);

    let very_stale = world
        .latest_at(at + Duration::days(7))
        .expect("latest")
        .expect("stored");
    assert_eq!(very_stale.freshness, FreshnessState::Stale);
    assert_eq!(very_stale.weather, weather);
}

#[test]
fn chicago_spring_forward_dst_offsets_are_correct() {
    let clock =
        WorldClock::new("America/Chicago", Some(41.8781), Some(-87.6298)).expect("chicago clock");

    let before = clock
        .clock_at(instant("2026-03-08T07:59:59Z"))
        .expect("before spring forward");
    assert_eq!(
        before.utc_offset_seconds,
        -6 * 3600,
        "CST before transition"
    );
    assert_eq!(before.local_time, "01:59:59");
    assert_eq!(before.local_date, "2026-03-08");
    assert_eq!(before.time_of_day, TimeOfDay::Night);
    assert_eq!(before.season, Season::Spring);

    let after = clock
        .clock_at(instant("2026-03-08T08:00:00Z"))
        .expect("after spring forward");
    assert_eq!(after.utc_offset_seconds, -5 * 3600, "CDT after transition");
    assert_eq!(after.local_time, "03:00:00", "02:00 is skipped");
    assert_eq!(after.local_date, "2026-03-08");
    assert_eq!(after.time_of_day, TimeOfDay::Night);
}

#[test]
fn chicago_fall_back_dst_offsets_and_repeated_hour_are_correct() {
    let clock =
        WorldClock::new("America/Chicago", Some(41.8781), Some(-87.6298)).expect("chicago clock");

    let before = clock
        .clock_at(instant("2026-11-01T06:59:59Z"))
        .expect("before fall back");
    assert_eq!(
        before.utc_offset_seconds,
        -5 * 3600,
        "CDT before transition"
    );
    assert_eq!(before.local_time, "01:59:59");
    assert_eq!(before.local_date, "2026-11-01");
    assert_eq!(before.season, Season::Autumn);

    let after = clock
        .clock_at(instant("2026-11-01T07:00:00Z"))
        .expect("after fall back");
    assert_eq!(after.utc_offset_seconds, -6 * 3600, "CST after transition");
    assert_eq!(after.local_time, "01:00:00", "01:00 repeats");
    assert_eq!(after.local_date, "2026-11-01");
    assert_eq!(after.season, Season::Autumn);
}
