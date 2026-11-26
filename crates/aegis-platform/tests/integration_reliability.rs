use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Barrier};
use std::time::Instant;

use aegis_contracts::world::{
    FreshnessState, SunTimes, WeatherForecast, WeatherObservation, MAX_FORECAST_ENTRIES,
    MAX_PROVENANCE_NOTE_CHARS,
};
use aegis_platform::{
    PlatformService, WeatherData, WeatherProvider, WeatherRequest, WeatherUnavailable, WorldService,
};
use aegis_storage::StoragePaths;
use chrono::{DateTime, Duration, Utc};

struct FixedProvider;

impl WeatherProvider for FixedProvider {
    fn name(&self) -> &'static str {
        "reliability_fixed"
    }

    fn observe(&self, request: &WeatherRequest) -> Result<WeatherData, WeatherUnavailable> {
        Ok(WeatherData {
            observation: Some(WeatherObservation {
                condition: "clear".to_owned(),
                temperature_celsius: Some(19.0),
                feels_like_celsius: Some(18.0),
                humidity_percent: Some(45),
                wind_speed_kph: Some(6.0),
                observed_at: Some(request.now.to_rfc3339()),
            }),
            forecast: vec![WeatherForecast {
                at: request.now.format("%Y-%m-%d").to_string(),
                condition: "clear".to_owned(),
                high_celsius: Some(22.0),
                low_celsius: Some(12.0),
                precipitation_probability_percent: Some(5),
            }],
            sun: Some(SunTimes {
                sunrise: Some("07:01".to_owned()),
                sunset: Some("16:41".to_owned()),
            }),
        })
    }
}

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
        "reliability_counting"
    }

    fn observe(&self, request: &WeatherRequest) -> Result<WeatherData, WeatherUnavailable> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        FixedProvider.observe(request)
    }
}

struct BarrierProvider {
    calls: AtomicUsize,
    barrier: Barrier,
}

impl BarrierProvider {
    fn new(parties: usize) -> Self {
        Self {
            calls: AtomicUsize::new(0),
            barrier: Barrier::new(parties),
        }
    }

    fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

impl WeatherProvider for BarrierProvider {
    fn name(&self) -> &'static str {
        "reliability_barrier"
    }

    fn observe(&self, request: &WeatherRequest) -> Result<WeatherData, WeatherUnavailable> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.barrier.wait();
        FixedProvider.observe(request)
    }
}

struct CountingFailingProvider {
    calls: AtomicUsize,
}

impl CountingFailingProvider {
    fn new() -> Self {
        Self {
            calls: AtomicUsize::new(0),
        }
    }

    fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

impl WeatherProvider for CountingFailingProvider {
    fn name(&self) -> &'static str {
        "reliability_failing"
    }

    fn observe(&self, _request: &WeatherRequest) -> Result<WeatherData, WeatherUnavailable> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Err(WeatherUnavailable::new("x".repeat(1_000)))
    }
}

struct BulkForecastProvider {
    entries: usize,
}

impl WeatherProvider for BulkForecastProvider {
    fn name(&self) -> &'static str {
        "reliability_bulk"
    }

    fn observe(&self, request: &WeatherRequest) -> Result<WeatherData, WeatherUnavailable> {
        let day = request.now.format("%Y-%m-%d").to_string();
        let forecast = (0..self.entries)
            .map(|index| WeatherForecast {
                at: format!("{day}+{index}"),
                condition: "clear".to_owned(),
                high_celsius: Some(20.0),
                low_celsius: Some(10.0),
                precipitation_probability_percent: Some(10),
            })
            .collect();
        Ok(WeatherData {
            observation: None,
            forecast,
            sun: None,
        })
    }
}

fn instant(value: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(value)
        .expect("fixed instant")
        .with_timezone(&Utc)
}

fn initialized_world(root: &Path) -> WorldService {
    PlatformService::initialize(root).expect("initialize platform");
    WorldService::start(StoragePaths::new(root).expect("storage paths")).expect("start world")
}

#[test]
fn concurrent_cores_share_one_identical_world_snapshot_without_contamination() {
    let temp = tempfile::tempdir().expect("tempdir");
    let world = initialized_world(temp.path());
    let at = instant("2026-11-26T18:00:00Z");
    let provider = Arc::new(CountingProvider::new());

    let generated = world
        .snapshot_at(provider.as_ref(), at)
        .expect("initial snapshot");
    assert_eq!(provider.calls(), 1);

    let cores = 8;
    let barrier = Arc::new(Barrier::new(cores));
    let started = Instant::now();
    let mut handles = Vec::with_capacity(cores);
    for _ in 0..cores {
        let world = world.clone();
        let provider = Arc::clone(&provider);
        let barrier = Arc::clone(&barrier);
        handles.push(std::thread::spawn(move || {
            barrier.wait();
            world
                .refresh_if_stale_at(provider.as_ref(), at + Duration::seconds(1))
                .expect("concurrent fresh read")
        }));
    }
    for handle in handles {
        let snapshot = handle.join().expect("core thread");
        assert_eq!(snapshot, generated);
        snapshot.validate().expect("valid shared snapshot");
    }
    let elapsed = started.elapsed();
    assert_eq!(
        provider.calls(),
        1,
        "a fresh snapshot must not be refetched per Core"
    );
    eprintln!("concurrent core world reads: {cores} readers in {elapsed:?}");
}

#[test]
fn concurrent_stale_refresh_documents_duplicate_provider_work() {
    let temp = tempfile::tempdir().expect("tempdir");
    let world = initialized_world(temp.path());
    let at = instant("2026-11-26T18:00:00Z");
    let cores = 4;
    let provider = Arc::new(BarrierProvider::new(cores));

    world
        .snapshot_at(&FixedProvider, at)
        .expect("initial snapshot");
    let later = at + Duration::seconds(901);

    let barrier = Arc::new(Barrier::new(cores));
    let mut handles = Vec::with_capacity(cores);
    for _ in 0..cores {
        let world = world.clone();
        let provider = Arc::clone(&provider);
        let barrier = Arc::clone(&barrier);
        handles.push(std::thread::spawn(move || {
            barrier.wait();
            world.refresh_if_stale_at(provider.as_ref(), later)
        }));
    }

    let mut successes = 0;
    for handle in handles {
        if let Ok(snapshot) = handle.join().expect("refresh thread") {
            snapshot.validate().expect("valid refresh");
            assert_eq!(snapshot.generated_at, later.to_rfc3339());
            successes += 1;
        }
    }
    assert!(
        successes >= 1,
        "at least one concurrent refresh must commit"
    );
    assert_eq!(
        provider.calls(),
        cores,
        "stale refresh is read-then-write without coalescing, so all racers fetch"
    );
    let shared = world
        .latest_at(later)
        .expect("latest")
        .expect("stored snapshot");
    shared.validate().expect("store remains valid after races");
}

#[test]
fn failing_weather_degrades_gracefully_but_re_polls_on_every_stale_refresh() {
    let temp = tempfile::tempdir().expect("tempdir");
    let world = initialized_world(temp.path());
    let at = instant("2026-11-26T18:00:00Z");
    let provider = CountingFailingProvider::new();

    let first = world
        .snapshot_at(&provider, at)
        .expect("failure must not fail snapshotting");
    first.validate().expect("unavailable snapshot is valid");
    assert!(first.weather.is_none());
    assert!(first.forecast.is_empty());
    assert!(first.sun.is_none());
    assert_eq!(first.freshness, FreshnessState::Unavailable);
    assert_eq!(first.provenance.source, "unavailable");
    let note = first.provenance.note.as_deref().expect("failure note");
    assert_eq!(note.chars().count(), MAX_PROVENANCE_NOTE_CHARS);
    assert_eq!(provider.calls(), 1);

    let rounds: i64 = 5;
    for round in 1..=rounds {
        let later = at + Duration::seconds(901 * round);
        let snapshot = world
            .refresh_if_stale_at(&provider, later)
            .expect("repeated failure stays graceful");
        snapshot.validate().expect("valid unavailable snapshot");
        assert!(snapshot.weather.is_none());
        assert_eq!(snapshot.freshness, FreshnessState::Unavailable);
        assert_eq!(snapshot.generated_at, later.to_rfc3339());
    }
    assert_eq!(
        provider.calls(),
        usize::try_from(rounds + 1).expect("round count"),
        "an unavailable snapshot is always stale, so each refresh re-polls the provider"
    );

    let last = at + Duration::seconds(901 * rounds);
    let shared = world
        .snapshot_for_core(&FixedProvider)
        .expect("shared read from the last unavailable snapshot");
    assert_eq!(shared.generated_at, last.to_rfc3339());
    assert_eq!(shared.freshness, FreshnessState::Unavailable);
    assert!(shared.weather.is_none());
    assert_eq!(world.clock().expect("clock still works").timezone(), "UTC");
}

#[test]
fn large_but_bounded_provider_inputs_are_truncated_by_the_snapshot_contract() {
    let temp = tempfile::tempdir().expect("tempdir");
    let world = initialized_world(temp.path());
    let at = instant("2026-11-26T18:00:00Z");
    let provider = BulkForecastProvider { entries: 200 };

    let snapshot = world
        .snapshot_at(&provider, at)
        .expect("bulk forecast snapshot");
    snapshot.validate().expect("truncated snapshot is valid");
    assert_eq!(snapshot.forecast.len(), MAX_FORECAST_ENTRIES);
    assert_eq!(snapshot.forecast[0].at, "2026-11-26+0");
    assert_eq!(snapshot.freshness, FreshnessState::Fresh);

    let stored = world
        .latest_at(at)
        .expect("latest")
        .expect("stored snapshot");
    assert_eq!(stored, snapshot);
    assert_eq!(stored.forecast.len(), MAX_FORECAST_ENTRIES);
}
