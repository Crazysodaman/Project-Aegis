//! Additive coverage for the background-friendly refresh helper and the
//! many-Core distribution helper.

use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};

use aegis_contracts::world::{FreshnessState, SunTimes, WeatherForecast, WeatherObservation};
use aegis_platform::{
    PlatformService, UnavailableWeatherProvider, WeatherData, WeatherProvider, WeatherRequest,
    WeatherUnavailable, WorldService, DEFAULT_TIMEZONE,
};
use aegis_storage::{open_read_only, StoragePaths, PLATFORM_APPLICATION_ID};
use chrono::{DateTime, Duration, Utc};

struct CountingProvider {
    calls: AtomicUsize,
}

impl CountingProvider {
    fn new() -> Self {
        Self {
            calls: AtomicUsize::new(0),
        }
    }

    fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

impl WeatherProvider for CountingProvider {
    fn name(&self) -> &'static str {
        "counting_test"
    }

    fn observe(&self, request: &WeatherRequest) -> Result<WeatherData, WeatherUnavailable> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(WeatherData {
            observation: Some(WeatherObservation {
                condition: "clear".to_owned(),
                temperature_celsius: Some(18.0),
                feels_like_celsius: Some(17.0),
                humidity_percent: Some(45),
                wind_speed_kph: Some(4.0),
                observed_at: Some(request.now.to_rfc3339()),
            }),
            forecast: vec![WeatherForecast {
                at: request.now.format("%Y-%m-%d").to_string(),
                condition: "clear".to_owned(),
                high_celsius: Some(20.0),
                low_celsius: Some(11.0),
                precipitation_probability_percent: Some(5),
            }],
            sun: Some(SunTimes {
                sunrise: Some("07:00".to_owned()),
                sunset: Some("16:30".to_owned()),
            }),
        })
    }
}

struct FailingProvider;

impl WeatherProvider for FailingProvider {
    fn name(&self) -> &'static str {
        "failing_test"
    }

    fn observe(&self, _request: &WeatherRequest) -> Result<WeatherData, WeatherUnavailable> {
        Err(WeatherUnavailable::new("network is offline"))
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

fn event_count(root: &Path) -> i64 {
    let storage = paths(root);
    let platform = open_read_only(&storage.platform_db(), PLATFORM_APPLICATION_ID)
        .expect("platform read-only");
    platform
        .query_row("SELECT COUNT(*) FROM world_events", [], |row| row.get(0))
        .expect("event count")
}

#[test]
fn fresh_snapshot_is_reused_without_calling_the_provider() {
    let temp = tempfile::tempdir().expect("tempdir");
    let world = initialized_world(temp.path());
    let at = instant("2026-11-26T18:00:00Z");
    let provider = CountingProvider::new();

    let first = world
        .refresh_if_stale_at(&provider, at)
        .expect("initial refresh builds a snapshot");
    assert_eq!(provider.calls(), 1);
    assert_eq!(first.freshness, FreshnessState::Fresh);
    assert_eq!(first.generated_at, at.to_rfc3339());
    assert!(first.weather.is_some());

    let reused = world
        .refresh_if_stale_at(&provider, at + Duration::seconds(600))
        .expect("fresh snapshot is reused");
    assert_eq!(provider.calls(), 1, "fresh data must not hit the provider");
    assert_eq!(reused, first);
    assert_eq!(event_count(temp.path()), 1, "no extra history event");
}

#[test]
fn stale_snapshot_is_refreshed_exactly_once() {
    let temp = tempfile::tempdir().expect("tempdir");
    let world = initialized_world(temp.path());
    let at = instant("2026-11-26T18:00:00Z");
    let provider = CountingProvider::new();

    let first = world
        .refresh_if_stale_at(&provider, at)
        .expect("initial refresh");
    let later = at + Duration::seconds(901);
    let refreshed = world
        .refresh_if_stale_at(&provider, later)
        .expect("stale snapshot is refreshed");

    assert_eq!(provider.calls(), 2);
    assert_ne!(refreshed.generated_at, first.generated_at);
    assert_eq!(refreshed.generated_at, later.to_rfc3339());
    assert_eq!(refreshed.freshness, FreshnessState::Fresh);
    let weather = refreshed.weather.as_ref().expect("fresh weather");
    assert_eq!(weather.observed_at, Some(later.to_rfc3339()));
    assert_eq!(refreshed.local_time, "18:15:01");
    assert_eq!(event_count(temp.path()), 2);

    let between = world.latest_at(later).expect("latest").expect("stored");
    assert_eq!(between, refreshed);
}

#[test]
fn stale_refresh_with_failing_provider_stays_honest() {
    let temp = tempfile::tempdir().expect("tempdir");
    let world = initialized_world(temp.path());
    let at = instant("2026-11-26T18:00:00Z");

    world
        .refresh_if_stale_at(&CountingProvider::new(), at)
        .expect("initial weather snapshot");

    let failing_at = at + Duration::seconds(901);
    let snapshot = world
        .refresh_if_stale_at(&FailingProvider, failing_at)
        .expect("provider failure does not fail the refresh");

    snapshot.validate().expect("unavailable snapshot is valid");
    assert!(snapshot.weather.is_none(), "no weather may be invented");
    assert!(snapshot.forecast.is_empty(), "no forecast may be invented");
    assert!(snapshot.sun.is_none(), "no sun times may be invented");
    assert_eq!(snapshot.freshness, FreshnessState::Unavailable);
    assert_eq!(snapshot.provenance.source, "unavailable");
    assert_eq!(
        snapshot.provenance.provider.as_deref(),
        Some("failing_test")
    );
    assert!(snapshot
        .provenance
        .note
        .as_deref()
        .expect("reason is recorded")
        .contains("network is offline"));
    assert_eq!(snapshot.generated_at, failing_at.to_rfc3339());
    assert_eq!(snapshot.local_time, "18:15:01");
    assert_eq!(event_count(temp.path()), 2);
}

#[test]
fn unavailable_provider_from_empty_store_never_invents_weather() {
    let temp = tempfile::tempdir().expect("tempdir");
    let world = initialized_world(temp.path());
    let at = instant("2026-11-26T18:00:00Z");

    let snapshot = world
        .refresh_if_stale_at(&UnavailableWeatherProvider::new(), at)
        .expect("honest unavailable snapshot");
    snapshot.validate().expect("valid snapshot");
    assert!(snapshot.weather.is_none());
    assert!(snapshot.forecast.is_empty());
    assert!(snapshot.sun.is_none());
    assert_eq!(snapshot.freshness, FreshnessState::Unavailable);
    assert_eq!(snapshot.timezone, "UTC");
    assert_eq!(event_count(temp.path()), 1);
}

#[test]
fn distribution_helper_returns_the_same_snapshot_for_every_core() {
    let temp = tempfile::tempdir().expect("tempdir");
    let world = initialized_world(temp.path());
    let at = instant("2026-11-26T18:00:00Z");
    let provider = CountingProvider::new();

    let generated = world
        .snapshot_at(&provider, at)
        .expect("generated snapshot");
    assert_eq!(provider.calls(), 1);

    let distributed = world
        .snapshot_for_cores_at(&provider, 3, at + Duration::seconds(100))
        .expect("distributed snapshot");
    assert_eq!(distributed.len(), 3);
    assert!(distributed.iter().all(|snapshot| snapshot == &generated));
    assert_eq!(provider.calls(), 1, "fresh data is shared, not refetched");
    assert_eq!(event_count(temp.path()), 1);

    let later = at + Duration::seconds(901);
    let refreshed = world
        .snapshot_for_cores_at(&provider, 4, later)
        .expect("stale distribution refreshes once");
    assert_eq!(refreshed.len(), 4);
    assert!(refreshed
        .iter()
        .all(|snapshot| snapshot.generated_at == later.to_rfc3339()));
    assert!(refreshed.windows(2).all(|pair| pair[0] == pair[1]));
    assert_eq!(provider.calls(), 2, "one refresh for all Cores");
    assert_eq!(event_count(temp.path()), 2);
}

#[test]
fn default_timezone_is_utc_and_not_a_local_city() {
    assert_eq!(DEFAULT_TIMEZONE, "UTC");

    let temp = tempfile::tempdir().expect("tempdir");
    let world = initialized_world(temp.path());
    let config = world.config().expect("default config");
    assert_eq!(config.timezone, "UTC");
    assert_eq!(world.clock().expect("clock").timezone(), "UTC");
}
