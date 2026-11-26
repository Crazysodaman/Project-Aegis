//! Platform-owned shared world service: clock, optional weather, and snapshot store.
//!
//! The Platform is the single authority for shared time. Cores never open
//! `platform.db`; they receive a bounded [`WorldSnapshot`] from this service.

use std::str::FromStr;

use aegis_contracts::world::{
    CalendarEvent, FreshnessState, Provenance, Season, SunTimes, TimeOfDay, WeatherForecast,
    WeatherObservation, WorldSnapshot, MAX_FORECAST_ENTRIES, MAX_PROVENANCE_NOTE_CHARS,
    MAX_PROVENANCE_SOURCE_CHARS, MAX_STALE_AFTER_SECONDS, MAX_TIMEZONE_CHARS,
    MIN_STALE_AFTER_SECONDS, WORLD_SNAPSHOT_SCHEMA,
};
use aegis_safe::SafeStore;
use aegis_storage::{now_rfc3339, open_initialized, StoragePaths, PLATFORM_APPLICATION_ID};
use chrono::{DateTime, Datelike, Offset, Timelike, Utc};
use chrono_tz::Tz;
use rusqlite::{params, Connection, OptionalExtension};

pub const CONFIG_TIMEZONE: &str = "world.timezone";
pub const CONFIG_LATITUDE: &str = "world.latitude";
pub const CONFIG_LONGITUDE: &str = "world.longitude";
pub const CONFIG_STALE_AFTER_SECONDS: &str = "world.stale_after_seconds";

pub const DEFAULT_TIMEZONE: &str = "UTC";
pub const DEFAULT_STALE_AFTER_SECONDS: u64 = 900;

#[derive(Debug, thiserror::Error)]
pub enum WorldError {
    #[error("world storage failed: {0}")]
    Storage(#[from] aegis_storage::StorageError),
    #[error("world SQLite operation failed: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("world JSON encoding failed: {0}")]
    Json(#[from] serde_json::Error),
    #[error("world contract validation failed: {0}")]
    Contract(#[from] aegis_contracts::world::WorldContractError),
    #[error("world Safe operation failed: {0}")]
    Safe(#[from] aegis_safe::SafeError),
    #[error("world owner authorization failed")]
    Unauthorized,
    #[error("world timezone is invalid: {0}")]
    InvalidTimezone(String),
    #[error("world location is invalid")]
    InvalidLocation,
    #[error("world configuration is invalid: {0}")]
    InvalidConfig(&'static str),
    #[error("world clock could not be computed: {0}")]
    Clock(&'static str),
}

#[derive(Debug, Clone, PartialEq)]
pub struct WorldConfig {
    pub timezone: String,
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
    pub stale_after_seconds: u64,
}

impl Default for WorldConfig {
    fn default() -> Self {
        Self {
            timezone: DEFAULT_TIMEZONE.to_owned(),
            latitude: None,
            longitude: None,
            stale_after_seconds: DEFAULT_STALE_AFTER_SECONDS,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorldClockReading {
    pub generated_at: String,
    pub timezone: String,
    pub utc_offset_seconds: i32,
    pub local_date: String,
    pub local_time: String,
    pub time_of_day: TimeOfDay,
    pub season: Season,
}

#[derive(Debug, Clone)]
pub struct WorldClock {
    timezone: String,
    latitude: Option<f64>,
    longitude: Option<f64>,
}

impl WorldClock {
    pub fn new(
        timezone: impl Into<String>,
        latitude: Option<f64>,
        longitude: Option<f64>,
    ) -> Result<Self, WorldError> {
        let timezone = timezone.into();
        validate_timezone(&timezone)?;
        validate_location(latitude, longitude)?;
        Ok(Self {
            timezone,
            latitude,
            longitude,
        })
    }

    pub fn timezone(&self) -> &str {
        &self.timezone
    }

    pub fn latitude(&self) -> Option<f64> {
        self.latitude
    }

    pub fn longitude(&self) -> Option<f64> {
        self.longitude
    }

    pub fn now(&self) -> Result<WorldClockReading, WorldError> {
        self.clock_at(Utc::now())
    }

    /// Pure clock computation at a fixed instant; this is the DST test surface.
    pub fn clock_at(&self, instant: DateTime<Utc>) -> Result<WorldClockReading, WorldError> {
        let timezone = Tz::from_str(&self.timezone)
            .map_err(|_| WorldError::InvalidTimezone(self.timezone.clone()))?;
        let local = instant.with_timezone(&timezone);
        let utc_offset_seconds = local.offset().fix().local_minus_utc();
        let hour = u8::try_from(local.hour()).map_err(|_| WorldError::Clock("local hour"))?;
        let time_of_day =
            TimeOfDay::from_local_hour(hour).ok_or(WorldError::Clock("time of day"))?;
        let northern_hemisphere = self.latitude.is_none_or(|latitude| latitude >= 0.0);
        let season = Season::from_month(local.month(), northern_hemisphere)
            .ok_or(WorldError::Clock("season"))?;
        Ok(WorldClockReading {
            generated_at: instant.to_rfc3339(),
            timezone: self.timezone.clone(),
            utc_offset_seconds,
            local_date: local.format("%Y-%m-%d").to_string(),
            local_time: local.format("%H:%M:%S").to_string(),
            time_of_day,
            season,
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct WeatherRequest {
    pub timezone: String,
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
    pub now: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct WeatherData {
    pub observation: Option<WeatherObservation>,
    pub forecast: Vec<WeatherForecast>,
    pub sun: Option<SunTimes>,
}

impl WeatherData {
    pub fn is_empty(&self) -> bool {
        self.observation.is_none() && self.forecast.is_empty() && self.sun.is_none()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WeatherUnavailable {
    pub reason: String,
}

impl WeatherUnavailable {
    pub fn new(reason: impl Into<String>) -> Self {
        Self {
            reason: reason.into(),
        }
    }
}

impl std::fmt::Display for WeatherUnavailable {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}", self.reason)
    }
}

impl std::error::Error for WeatherUnavailable {}

pub trait WeatherProvider {
    fn name(&self) -> &'static str;
    fn observe(&self, request: &WeatherRequest) -> Result<WeatherData, WeatherUnavailable>;
}

/// Honest default: no network, no invented weather, no secrets.
#[derive(Debug, Clone)]
pub struct UnavailableWeatherProvider {
    reason: String,
}

impl UnavailableWeatherProvider {
    pub fn new() -> Self {
        Self {
            reason: "no weather provider is configured".to_owned(),
        }
    }

    pub fn with_reason(reason: impl Into<String>) -> Self {
        Self {
            reason: reason.into(),
        }
    }
}

impl Default for UnavailableWeatherProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl WeatherProvider for UnavailableWeatherProvider {
    fn name(&self) -> &'static str {
        "unavailable"
    }

    fn observe(&self, _request: &WeatherRequest) -> Result<WeatherData, WeatherUnavailable> {
        Err(WeatherUnavailable::new(self.reason.clone()))
    }
}

#[derive(Debug, Clone)]
pub struct WorldStore {
    paths: StoragePaths,
}

impl WorldStore {
    pub fn new(paths: StoragePaths) -> Self {
        Self { paths }
    }

    pub fn initialize(&self) -> Result<(), WorldError> {
        let connection = self.connection()?;
        connection.execute_batch(
            "CREATE TABLE IF NOT EXISTS world_context (
                singleton INTEGER PRIMARY KEY CHECK(singleton = 1),
                snapshot_json TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS world_events (
                sequence INTEGER PRIMARY KEY AUTOINCREMENT,
                event_type TEXT NOT NULL,
                detail_json TEXT NOT NULL,
                created_at TEXT NOT NULL
            );",
        )?;
        Ok(())
    }

    /// Atomically replace the current snapshot and append one history event.
    pub fn store(&self, snapshot: &WorldSnapshot) -> Result<(), WorldError> {
        snapshot.validate()?;
        let encoded = serde_json::to_string(snapshot)?;
        let mut connection = self.connection()?;
        let transaction = connection.transaction()?;
        transaction.execute(
            "INSERT INTO world_context(singleton, snapshot_json, updated_at)
             VALUES(1, ?1, ?2)
             ON CONFLICT(singleton) DO UPDATE SET
                snapshot_json = excluded.snapshot_json,
                updated_at = excluded.updated_at",
            params![encoded, snapshot.generated_at],
        )?;
        transaction.execute(
            "INSERT INTO world_events(event_type, detail_json, created_at)
             VALUES('world.snapshot', ?1, ?2)",
            params![encoded, snapshot.generated_at],
        )?;
        transaction.commit()?;
        Ok(())
    }

    pub fn latest(&self) -> Result<Option<WorldSnapshot>, WorldError> {
        self.latest_at(Utc::now())
    }

    /// Read the shared snapshot and recompute freshness relative to `now`.
    pub fn latest_at(&self, now: DateTime<Utc>) -> Result<Option<WorldSnapshot>, WorldError> {
        let connection = self.connection()?;
        let row = connection
            .query_row(
                "SELECT snapshot_json FROM world_context WHERE singleton = 1",
                [],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        match row {
            Some(encoded) => {
                let mut snapshot: WorldSnapshot = serde_json::from_str(&encoded)?;
                snapshot.freshness = freshness_for(&snapshot, now);
                Ok(Some(snapshot))
            }
            None => Ok(None),
        }
    }

    fn connection(&self) -> Result<Connection, WorldError> {
        Ok(open_initialized(
            &self.paths.platform_db(),
            PLATFORM_APPLICATION_ID,
            "platform",
        )?)
    }
}

#[derive(Debug, Clone)]
pub struct WorldService {
    paths: StoragePaths,
    store: WorldStore,
    safe: SafeStore,
}

impl WorldService {
    pub fn start(paths: StoragePaths) -> Result<Self, WorldError> {
        let store = WorldStore::new(paths.clone());
        store.initialize()?;
        Ok(Self {
            safe: SafeStore::new(paths.clone()),
            paths,
            store,
        })
    }

    pub fn paths(&self) -> &StoragePaths {
        &self.paths
    }

    pub fn config(&self) -> Result<WorldConfig, WorldError> {
        let connection = open_initialized(
            &self.paths.platform_db(),
            PLATFORM_APPLICATION_ID,
            "platform",
        )?;
        let timezone = read_config_value(&connection, CONFIG_TIMEZONE)?
            .unwrap_or_else(|| DEFAULT_TIMEZONE.to_owned());
        validate_timezone(&timezone)?;
        let latitude = parse_optional_f64(
            read_config_value(&connection, CONFIG_LATITUDE)?,
            CONFIG_LATITUDE,
        )?;
        let longitude = parse_optional_f64(
            read_config_value(&connection, CONFIG_LONGITUDE)?,
            CONFIG_LONGITUDE,
        )?;
        validate_location(latitude, longitude)?;
        let stale_after_seconds = match read_config_value(&connection, CONFIG_STALE_AFTER_SECONDS)?
        {
            Some(raw) => raw
                .parse::<u64>()
                .map_err(|_| WorldError::InvalidConfig(CONFIG_STALE_AFTER_SECONDS))?,
            None => DEFAULT_STALE_AFTER_SECONDS,
        };
        if !(MIN_STALE_AFTER_SECONDS..=MAX_STALE_AFTER_SECONDS).contains(&stale_after_seconds) {
            return Err(WorldError::InvalidConfig(CONFIG_STALE_AFTER_SECONDS));
        }
        Ok(WorldConfig {
            timezone,
            latitude,
            longitude,
            stale_after_seconds,
        })
    }

    pub fn clock(&self) -> Result<WorldClock, WorldError> {
        let config = self.config()?;
        WorldClock::new(config.timezone, config.latitude, config.longitude)
    }

    /// Owner-gated: only a valid Safe owner proof may change the timezone.
    pub fn set_timezone(&self, owner_token: &str, timezone: &str) -> Result<(), WorldError> {
        self.require_owner(owner_token)?;
        let timezone = timezone.trim();
        validate_timezone(timezone)?;
        let connection = open_initialized(
            &self.paths.platform_db(),
            PLATFORM_APPLICATION_ID,
            "platform",
        )?;
        write_config_value(&connection, CONFIG_TIMEZONE, timezone)?;
        Ok(())
    }

    /// Owner-gated: longitude and latitude must be supplied together.
    pub fn set_location(
        &self,
        owner_token: &str,
        latitude: f64,
        longitude: f64,
    ) -> Result<(), WorldError> {
        self.require_owner(owner_token)?;
        validate_location(Some(latitude), Some(longitude))?;
        let mut connection = open_initialized(
            &self.paths.platform_db(),
            PLATFORM_APPLICATION_ID,
            "platform",
        )?;
        let transaction = connection.transaction()?;
        write_config_value(&transaction, CONFIG_LATITUDE, &latitude.to_string())?;
        write_config_value(&transaction, CONFIG_LONGITUDE, &longitude.to_string())?;
        transaction.commit()?;
        Ok(())
    }

    pub fn snapshot(&self, provider: &dyn WeatherProvider) -> Result<WorldSnapshot, WorldError> {
        self.snapshot_at(provider, Utc::now())
    }

    pub fn snapshot_at(
        &self,
        provider: &dyn WeatherProvider,
        instant: DateTime<Utc>,
    ) -> Result<WorldSnapshot, WorldError> {
        let config = self.config()?;
        let clock = WorldClock::new(config.timezone.clone(), config.latitude, config.longitude)?;
        let reading = clock.clock_at(instant)?;
        let request = WeatherRequest {
            timezone: config.timezone.clone(),
            latitude: config.latitude,
            longitude: config.longitude,
            now: instant,
        };
        let outcome = match provider.observe(&request) {
            Ok(mut data) => {
                data.forecast.truncate(MAX_FORECAST_ENTRIES);
                WeatherOutcome::Available {
                    provider: provider.name().to_owned(),
                    data,
                }
            }
            Err(unavailable) => WeatherOutcome::Unavailable {
                provider: provider.name().to_owned(),
                reason: unavailable.reason,
            },
        };
        let candidate = build_snapshot(&config, &reading, outcome);
        let snapshot = match candidate.validate() {
            Ok(()) => candidate,
            Err(error) => {
                if snapshot_has_provider_data(&candidate) {
                    let fallback = build_snapshot(
                        &config,
                        &reading,
                        WeatherOutcome::Unavailable {
                            provider: provider.name().to_owned(),
                            reason: format!("provider data rejected: {error}"),
                        },
                    );
                    fallback.validate()?;
                    fallback
                } else {
                    return Err(error.into());
                }
            }
        };
        self.store.store(&snapshot)?;
        Ok(snapshot)
    }

    /// Shared read for Cores: return the current stored snapshot so every Core
    /// sees the same world data. Build one only when none exists yet.
    pub fn snapshot_for_core(
        &self,
        provider: &dyn WeatherProvider,
    ) -> Result<WorldSnapshot, WorldError> {
        match self.store.latest()? {
            Some(snapshot) => Ok(snapshot),
            None => self.snapshot(provider),
        }
    }

    /// Background-friendly refresh: reuse the stored snapshot while it is fresh
    /// and refresh it at most once when it is missing, stale, or unavailable.
    ///
    /// This is additive over [`WorldService::snapshot_for_core`]: it never
    /// spawns a thread and performs no scheduling. The caller owns the trigger
    /// (timer, request, or event loop) and injects the [`WeatherProvider`], so
    /// network/secret policy stays with the caller.
    pub fn refresh_if_stale(
        &self,
        provider: &dyn WeatherProvider,
    ) -> Result<WorldSnapshot, WorldError> {
        self.refresh_if_stale_at(provider, Utc::now())
    }

    /// Deterministic variant of [`WorldService::refresh_if_stale`] for callers
    /// that supply their own clock instant (tests and schedulers).
    pub fn refresh_if_stale_at(
        &self,
        provider: &dyn WeatherProvider,
        instant: DateTime<Utc>,
    ) -> Result<WorldSnapshot, WorldError> {
        match self.store.latest_at(instant)? {
            Some(snapshot) if snapshot.freshness == FreshnessState::Fresh => Ok(snapshot),
            _ => self.snapshot_at(provider, instant),
        }
    }

    /// Distribution helper: hand every Core one *identical* snapshot.
    ///
    /// The snapshot is produced at most once (reusing a fresh stored value or
    /// refreshing when missing/stale/unavailable), then cloned for `core_count`
    /// Cores. This guarantees no Core observes a different world view and needs
    /// no threads.
    pub fn snapshot_for_cores(
        &self,
        provider: &dyn WeatherProvider,
        core_count: usize,
    ) -> Result<Vec<WorldSnapshot>, WorldError> {
        self.snapshot_for_cores_at(provider, core_count, Utc::now())
    }

    /// Deterministic variant of [`WorldService::snapshot_for_cores`].
    pub fn snapshot_for_cores_at(
        &self,
        provider: &dyn WeatherProvider,
        core_count: usize,
        instant: DateTime<Utc>,
    ) -> Result<Vec<WorldSnapshot>, WorldError> {
        let snapshot = self.refresh_if_stale_at(provider, instant)?;
        Ok(vec![snapshot; core_count])
    }

    pub fn shared_snapshot(&self) -> Result<Option<WorldSnapshot>, WorldError> {
        self.store.latest()
    }

    pub fn latest(&self) -> Result<Option<WorldSnapshot>, WorldError> {
        self.store.latest()
    }

    pub fn latest_at(&self, now: DateTime<Utc>) -> Result<Option<WorldSnapshot>, WorldError> {
        self.store.latest_at(now)
    }

    fn require_owner(&self, owner_token: &str) -> Result<(), WorldError> {
        match self.safe.verify_owner(owner_token) {
            Ok(true) => Ok(()),
            Ok(false) => Err(WorldError::Unauthorized),
            Err(error) => Err(WorldError::Safe(error)),
        }
    }
}

enum WeatherOutcome {
    Available { provider: String, data: WeatherData },
    Unavailable { provider: String, reason: String },
}

fn build_snapshot(
    config: &WorldConfig,
    reading: &WorldClockReading,
    outcome: WeatherOutcome,
) -> WorldSnapshot {
    let (sun, weather, forecast, provenance, freshness) = match outcome {
        WeatherOutcome::Available { provider, data } => {
            let provider = bounded_provider(&provider);
            let has_data = !data.is_empty();
            let note = if has_data {
                None
            } else {
                Some("provider returned no weather data".to_owned())
            };
            (
                data.sun,
                data.observation,
                data.forecast,
                Provenance {
                    source: provider.clone(),
                    provider: Some(provider),
                    retrieved_at: Some(reading.generated_at.clone()),
                    note,
                },
                if has_data {
                    FreshnessState::Fresh
                } else {
                    FreshnessState::Unavailable
                },
            )
        }
        WeatherOutcome::Unavailable { provider, reason } => {
            let note = bounded_note(&reason);
            (
                None,
                None,
                Vec::new(),
                Provenance {
                    source: "unavailable".to_owned(),
                    provider: Some(bounded_provider(&provider)),
                    retrieved_at: Some(reading.generated_at.clone()),
                    note: Some(note),
                },
                FreshnessState::Unavailable,
            )
        }
    };
    WorldSnapshot {
        schema: WORLD_SNAPSHOT_SCHEMA.to_owned(),
        generated_at: reading.generated_at.clone(),
        timezone: reading.timezone.clone(),
        utc_offset_seconds: reading.utc_offset_seconds,
        local_date: reading.local_date.clone(),
        local_time: reading.local_time.clone(),
        time_of_day: reading.time_of_day,
        season: reading.season,
        sun,
        weather,
        forecast,
        calendar: Vec::<CalendarEvent>::new(),
        provenance,
        freshness,
        stale_after_seconds: config.stale_after_seconds,
    }
}

fn snapshot_has_provider_data(snapshot: &WorldSnapshot) -> bool {
    snapshot.weather.is_some() || !snapshot.forecast.is_empty() || snapshot.sun.is_some()
}

fn freshness_for(snapshot: &WorldSnapshot, now: DateTime<Utc>) -> FreshnessState {
    if snapshot.weather.is_none() && snapshot.forecast.is_empty() && snapshot.sun.is_none() {
        return FreshnessState::Unavailable;
    }
    let generated = match DateTime::parse_from_rfc3339(&snapshot.generated_at) {
        Ok(value) => value.with_timezone(&Utc),
        Err(_) => return FreshnessState::Unavailable,
    };
    let age = now.signed_duration_since(generated).num_seconds().max(0);
    let age_seconds = u64::try_from(age).unwrap_or(u64::MAX);
    if age_seconds <= snapshot.stale_after_seconds {
        FreshnessState::Fresh
    } else {
        FreshnessState::Stale
    }
}

fn validate_timezone(timezone: &str) -> Result<(), WorldError> {
    if timezone.trim().is_empty()
        || timezone.chars().count() > MAX_TIMEZONE_CHARS
        || timezone.chars().any(char::is_control)
    {
        return Err(WorldError::InvalidTimezone(timezone.to_owned()));
    }
    Tz::from_str(timezone).map_err(|_| WorldError::InvalidTimezone(timezone.to_owned()))?;
    Ok(())
}

fn validate_location(latitude: Option<f64>, longitude: Option<f64>) -> Result<(), WorldError> {
    match (latitude, longitude) {
        (None, None) => Ok(()),
        (Some(latitude), Some(longitude))
            if latitude.is_finite()
                && longitude.is_finite()
                && (-90.0..=90.0).contains(&latitude)
                && (-180.0..=180.0).contains(&longitude) =>
        {
            Ok(())
        }
        _ => Err(WorldError::InvalidLocation),
    }
}

fn parse_optional_f64(
    value: Option<String>,
    field: &'static str,
) -> Result<Option<f64>, WorldError> {
    let Some(raw) = value else {
        return Ok(None);
    };
    let parsed = raw
        .trim()
        .parse::<f64>()
        .map_err(|_| WorldError::InvalidConfig(field))?;
    if !parsed.is_finite() {
        return Err(WorldError::InvalidConfig(field));
    }
    Ok(Some(parsed))
}

fn read_config_value(connection: &Connection, key: &str) -> Result<Option<String>, WorldError> {
    Ok(connection
        .query_row(
            "SELECT config_value FROM platform_config WHERE config_key = ?1",
            [key],
            |row| row.get(0),
        )
        .optional()?)
}

fn write_config_value(connection: &Connection, key: &str, value: &str) -> Result<(), WorldError> {
    connection.execute(
        "INSERT INTO platform_config(config_key, config_value, updated_at)
         VALUES(?1, ?2, ?3)
         ON CONFLICT(config_key) DO UPDATE SET
            config_value = excluded.config_value,
            updated_at = excluded.updated_at",
        params![key, value, now_rfc3339()?],
    )?;
    Ok(())
}

fn bounded_provider(provider: &str) -> String {
    let provider = sanitize_text(provider, MAX_PROVENANCE_SOURCE_CHARS);
    if provider.is_empty() {
        "unknown".to_owned()
    } else {
        provider
    }
}

fn bounded_note(note: &str) -> String {
    let note = sanitize_text(note, MAX_PROVENANCE_NOTE_CHARS);
    if note.is_empty() {
        "weather provider unavailable".to_owned()
    } else {
        note
    }
}

fn sanitize_text(value: &str, maximum: usize) -> String {
    value
        .chars()
        .map(|character| {
            if character.is_control() {
                ' '
            } else {
                character
            }
        })
        .take(maximum)
        .collect::<String>()
        .trim()
        .to_owned()
}
