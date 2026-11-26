//! Versioned, authority-free shared world contracts.
//!
//! These types describe what every Core is allowed to know about the shared
//! world: the Platform clock, optional sun/weather/forecast data, optional
//! calendar entries, provenance, and freshness. They carry no authority, no
//! credentials, and no personal Core state.

use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const WORLD_SNAPSHOT_SCHEMA: &str = "aegis.world.snapshot.v1";

pub const MAX_FORECAST_ENTRIES: usize = 16;
pub const MAX_CALENDAR_EVENTS: usize = 64;
pub const MAX_TIMEZONE_CHARS: usize = 64;
pub const MAX_LOCAL_DATE_CHARS: usize = 32;
pub const MAX_LOCAL_TIME_CHARS: usize = 32;
pub const MAX_TIMESTAMP_CHARS: usize = 64;
pub const MAX_CONDITION_CHARS: usize = 120;
pub const MAX_EVENT_TITLE_CHARS: usize = 160;
pub const MAX_EVENT_LOCATION_CHARS: usize = 160;
pub const MAX_EVENT_NOTES_CHARS: usize = 512;
pub const MAX_PROVENANCE_SOURCE_CHARS: usize = 64;
pub const MAX_PROVENANCE_NOTE_CHARS: usize = 240;

/// IANA offsets never exceed +/-14h; +/-18h is a conservative contract bound.
pub const MIN_UTC_OFFSET_SECONDS: i32 = -18 * 3600;
pub const MAX_UTC_OFFSET_SECONDS: i32 = 18 * 3600;

pub const MIN_STALE_AFTER_SECONDS: u64 = 1;
pub const MAX_STALE_AFTER_SECONDS: u64 = 7 * 24 * 60 * 60;

const MIN_TEMPERATURE_CELSIUS: f64 = -100.0;
const MAX_TEMPERATURE_CELSIUS: f64 = 100.0;
const MAX_WIND_SPEED_KPH: f64 = 1_000.0;
const MAX_PERCENT: u8 = 100;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum WorldContractError {
    #[error("world snapshot schema is unsupported")]
    InvalidSchema,
    #[error("world timestamp is invalid")]
    InvalidTimestamp,
    #[error("world timezone is invalid")]
    InvalidTimezone,
    #[error("world timezone offset is out of range")]
    InvalidUtcOffset,
    #[error("world local date is invalid")]
    InvalidLocalDate,
    #[error("world local time is invalid")]
    InvalidLocalTime,
    #[error("world string field is invalid: {0}")]
    InvalidString(&'static str),
    #[error("world number field is invalid: {0}")]
    InvalidNumber(&'static str),
    #[error("world forecast exceeds the supported maximum of {0} entries")]
    TooManyForecast(usize),
    #[error("world calendar exceeds the supported maximum of {0} events")]
    TooManyCalendar(usize),
    #[error("world forecast entry {0} is invalid")]
    InvalidForecast(usize),
    #[error("world calendar event {0} is invalid")]
    InvalidCalendar(usize),
    #[error("world provenance is invalid")]
    InvalidProvenance,
    #[error("world stale-after duration is invalid")]
    InvalidStaleAfter,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TimeOfDay {
    Night,
    Dawn,
    Morning,
    Midday,
    Afternoon,
    Evening,
    Dusk,
}

impl TimeOfDay {
    /// Deterministic local-hour mapping used by the Platform clock.
    ///
    /// Night 22:00-04:59, Dawn 05:00-06:59, Morning 07:00-11:59,
    /// Midday 12:00-12:59, Afternoon 13:00-16:59, Evening 17:00-19:59,
    /// Dusk 20:00-21:59.
    pub const fn from_local_hour(hour: u8) -> Option<Self> {
        match hour {
            0..=4 => Some(Self::Night),
            5..=6 => Some(Self::Dawn),
            7..=11 => Some(Self::Morning),
            12 => Some(Self::Midday),
            13..=16 => Some(Self::Afternoon),
            17..=19 => Some(Self::Evening),
            20..=21 => Some(Self::Dusk),
            22..=23 => Some(Self::Night),
            _ => None,
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Night => "night",
            Self::Dawn => "dawn",
            Self::Morning => "morning",
            Self::Midday => "midday",
            Self::Afternoon => "afternoon",
            Self::Evening => "evening",
            Self::Dusk => "dusk",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Season {
    Spring,
    Summer,
    Autumn,
    Winter,
    Unknown,
}

impl Season {
    /// Meteorological season from a calendar month. The caller selects the
    /// hemisphere; the Platform derives it from configured latitude.
    pub const fn from_month(month: u32, northern_hemisphere: bool) -> Option<Self> {
        let northern = match month {
            3..=5 => Self::Spring,
            6..=8 => Self::Summer,
            9..=11 => Self::Autumn,
            12 | 1 | 2 => Self::Winter,
            _ => return None,
        };
        Some(if northern_hemisphere {
            northern
        } else {
            northern.opposite()
        })
    }

    pub const fn opposite(self) -> Self {
        match self {
            Self::Spring => Self::Autumn,
            Self::Summer => Self::Winter,
            Self::Autumn => Self::Spring,
            Self::Winter => Self::Summer,
            Self::Unknown => Self::Unknown,
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Spring => "spring",
            Self::Summer => "summer",
            Self::Autumn => "autumn",
            Self::Winter => "winter",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FreshnessState {
    Fresh,
    Stale,
    Unavailable,
}

impl FreshnessState {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Fresh => "fresh",
            Self::Stale => "stale",
            Self::Unavailable => "unavailable",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Provenance {
    pub source: String,
    pub provider: Option<String>,
    pub retrieved_at: Option<String>,
    pub note: Option<String>,
}

impl Provenance {
    pub fn validate(&self) -> Result<(), WorldContractError> {
        validate_bounded(
            &self.source,
            MAX_PROVENANCE_SOURCE_CHARS,
            "provenance.source",
        )?;
        validate_optional_bounded(
            &self.provider,
            MAX_PROVENANCE_SOURCE_CHARS,
            "provenance.provider",
        )?;
        validate_optional_bounded(
            &self.retrieved_at,
            MAX_TIMESTAMP_CHARS,
            "provenance.retrieved_at",
        )?;
        validate_optional_bounded(&self.note, MAX_PROVENANCE_NOTE_CHARS, "provenance.note")?;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SunTimes {
    pub sunrise: Option<String>,
    pub sunset: Option<String>,
}

impl SunTimes {
    pub fn validate(&self) -> Result<(), WorldContractError> {
        validate_optional_bounded(&self.sunrise, MAX_LOCAL_TIME_CHARS, "sun.sunrise")?;
        validate_optional_bounded(&self.sunset, MAX_LOCAL_TIME_CHARS, "sun.sunset")?;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WeatherObservation {
    pub condition: String,
    pub temperature_celsius: Option<f64>,
    pub feels_like_celsius: Option<f64>,
    pub humidity_percent: Option<u8>,
    pub wind_speed_kph: Option<f64>,
    pub observed_at: Option<String>,
}

impl WeatherObservation {
    pub fn validate(&self) -> Result<(), WorldContractError> {
        validate_bounded(&self.condition, MAX_CONDITION_CHARS, "weather.condition")?;
        validate_temperature(self.temperature_celsius, "weather.temperature_celsius")?;
        validate_temperature(self.feels_like_celsius, "weather.feels_like_celsius")?;
        validate_optional_percent(self.humidity_percent, "weather.humidity_percent")?;
        if let Some(speed) = self.wind_speed_kph {
            if !speed.is_finite() || !(0.0..=MAX_WIND_SPEED_KPH).contains(&speed) {
                return Err(WorldContractError::InvalidNumber("weather.wind_speed_kph"));
            }
        }
        validate_optional_bounded(
            &self.observed_at,
            MAX_TIMESTAMP_CHARS,
            "weather.observed_at",
        )?;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WeatherForecast {
    pub at: String,
    pub condition: String,
    pub high_celsius: Option<f64>,
    pub low_celsius: Option<f64>,
    pub precipitation_probability_percent: Option<u8>,
}

impl WeatherForecast {
    pub fn validate(&self) -> Result<(), WorldContractError> {
        validate_bounded(&self.at, MAX_TIMESTAMP_CHARS, "forecast.at")?;
        validate_bounded(&self.condition, MAX_CONDITION_CHARS, "forecast.condition")?;
        validate_temperature(self.high_celsius, "forecast.high_celsius")?;
        validate_temperature(self.low_celsius, "forecast.low_celsius")?;
        validate_optional_percent(
            self.precipitation_probability_percent,
            "forecast.precipitation_probability_percent",
        )?;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CalendarEvent {
    pub title: String,
    pub starts_at: String,
    pub ends_at: Option<String>,
    pub all_day: bool,
    pub location: Option<String>,
    pub notes: Option<String>,
}

impl CalendarEvent {
    pub fn validate(&self) -> Result<(), WorldContractError> {
        validate_bounded(&self.title, MAX_EVENT_TITLE_CHARS, "calendar.title")?;
        validate_bounded(&self.starts_at, MAX_TIMESTAMP_CHARS, "calendar.starts_at")?;
        validate_optional_bounded(&self.ends_at, MAX_TIMESTAMP_CHARS, "calendar.ends_at")?;
        validate_optional_bounded(
            &self.location,
            MAX_EVENT_LOCATION_CHARS,
            "calendar.location",
        )?;
        validate_optional_bounded(&self.notes, MAX_EVENT_NOTES_CHARS, "calendar.notes")?;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorldSnapshot {
    pub schema: String,
    pub generated_at: String,
    pub timezone: String,
    pub utc_offset_seconds: i32,
    pub local_date: String,
    pub local_time: String,
    pub time_of_day: TimeOfDay,
    pub season: Season,
    pub sun: Option<SunTimes>,
    pub weather: Option<WeatherObservation>,
    pub forecast: Vec<WeatherForecast>,
    pub calendar: Vec<CalendarEvent>,
    pub provenance: Provenance,
    pub freshness: FreshnessState,
    pub stale_after_seconds: u64,
}

impl WorldSnapshot {
    pub fn validate(&self) -> Result<(), WorldContractError> {
        if self.schema != WORLD_SNAPSHOT_SCHEMA {
            return Err(WorldContractError::InvalidSchema);
        }
        validate_bounded(&self.generated_at, MAX_TIMESTAMP_CHARS, "generated_at")
            .map_err(|_| WorldContractError::InvalidTimestamp)?;
        validate_bounded(&self.timezone, MAX_TIMEZONE_CHARS, "timezone")
            .map_err(|_| WorldContractError::InvalidTimezone)?;
        if !(MIN_UTC_OFFSET_SECONDS..=MAX_UTC_OFFSET_SECONDS).contains(&self.utc_offset_seconds) {
            return Err(WorldContractError::InvalidUtcOffset);
        }
        validate_bounded(&self.local_date, MAX_LOCAL_DATE_CHARS, "local_date")
            .map_err(|_| WorldContractError::InvalidLocalDate)?;
        validate_bounded(&self.local_time, MAX_LOCAL_TIME_CHARS, "local_time")
            .map_err(|_| WorldContractError::InvalidLocalTime)?;
        if let Some(sun) = &self.sun {
            sun.validate()?;
        }
        if let Some(weather) = &self.weather {
            weather.validate()?;
        }
        if self.forecast.len() > MAX_FORECAST_ENTRIES {
            return Err(WorldContractError::TooManyForecast(MAX_FORECAST_ENTRIES));
        }
        for (index, forecast) in self.forecast.iter().enumerate() {
            forecast
                .validate()
                .map_err(|_| WorldContractError::InvalidForecast(index))?;
        }
        if self.calendar.len() > MAX_CALENDAR_EVENTS {
            return Err(WorldContractError::TooManyCalendar(MAX_CALENDAR_EVENTS));
        }
        for (index, event) in self.calendar.iter().enumerate() {
            event
                .validate()
                .map_err(|_| WorldContractError::InvalidCalendar(index))?;
        }
        self.provenance
            .validate()
            .map_err(|_| WorldContractError::InvalidProvenance)?;
        if !(MIN_STALE_AFTER_SECONDS..=MAX_STALE_AFTER_SECONDS).contains(&self.stale_after_seconds)
        {
            return Err(WorldContractError::InvalidStaleAfter);
        }
        Ok(())
    }
}

fn validate_bounded(
    value: &str,
    maximum: usize,
    field: &'static str,
) -> Result<(), WorldContractError> {
    if value.trim().is_empty()
        || value.chars().count() > maximum
        || value.chars().any(char::is_control)
    {
        return Err(WorldContractError::InvalidString(field));
    }
    Ok(())
}

fn validate_optional_bounded(
    value: &Option<String>,
    maximum: usize,
    field: &'static str,
) -> Result<(), WorldContractError> {
    if let Some(value) = value {
        validate_bounded(value, maximum, field)?;
    }
    Ok(())
}

fn validate_temperature(value: Option<f64>, field: &'static str) -> Result<(), WorldContractError> {
    if let Some(value) = value {
        if !value.is_finite()
            || !(MIN_TEMPERATURE_CELSIUS..=MAX_TEMPERATURE_CELSIUS).contains(&value)
        {
            return Err(WorldContractError::InvalidNumber(field));
        }
    }
    Ok(())
}

fn validate_optional_percent(
    value: Option<u8>,
    field: &'static str,
) -> Result<(), WorldContractError> {
    if value.is_some_and(|percent| percent > MAX_PERCENT) {
        return Err(WorldContractError::InvalidNumber(field));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_snapshot() -> WorldSnapshot {
        WorldSnapshot {
            schema: WORLD_SNAPSHOT_SCHEMA.to_owned(),
            generated_at: "2026-11-26T12:00:00+00:00".to_owned(),
            timezone: "America/Chicago".to_owned(),
            utc_offset_seconds: -6 * 3600,
            local_date: "2026-11-26".to_owned(),
            local_time: "06:00:00".to_owned(),
            time_of_day: TimeOfDay::Dawn,
            season: Season::Autumn,
            sun: Some(SunTimes {
                sunrise: Some("06:45".to_owned()),
                sunset: Some("17:10".to_owned()),
            }),
            weather: Some(WeatherObservation {
                condition: "clear".to_owned(),
                temperature_celsius: Some(8.0),
                feels_like_celsius: Some(6.5),
                humidity_percent: Some(62),
                wind_speed_kph: Some(9.0),
                observed_at: Some("2026-11-26T11:55:00+00:00".to_owned()),
            }),
            forecast: vec![WeatherForecast {
                at: "2026-11-26".to_owned(),
                condition: "clear".to_owned(),
                high_celsius: Some(11.0),
                low_celsius: Some(2.0),
                precipitation_probability_percent: Some(10),
            }],
            calendar: vec![CalendarEvent {
                title: "Planning".to_owned(),
                starts_at: "2026-11-26T09:00:00-06:00".to_owned(),
                ends_at: None,
                all_day: false,
                location: None,
                notes: None,
            }],
            provenance: Provenance {
                source: "platform_clock".to_owned(),
                provider: Some("test".to_owned()),
                retrieved_at: Some("2026-11-26T12:00:00+00:00".to_owned()),
                note: None,
            },
            freshness: FreshnessState::Fresh,
            stale_after_seconds: 900,
        }
    }

    #[test]
    fn valid_snapshot_passes_validation() {
        valid_snapshot().validate().expect("valid snapshot");
    }

    #[test]
    fn time_of_day_is_deterministic() {
        assert_eq!(TimeOfDay::from_local_hour(0), Some(TimeOfDay::Night));
        assert_eq!(TimeOfDay::from_local_hour(5), Some(TimeOfDay::Dawn));
        assert_eq!(TimeOfDay::from_local_hour(9), Some(TimeOfDay::Morning));
        assert_eq!(TimeOfDay::from_local_hour(12), Some(TimeOfDay::Midday));
        assert_eq!(TimeOfDay::from_local_hour(15), Some(TimeOfDay::Afternoon));
        assert_eq!(TimeOfDay::from_local_hour(18), Some(TimeOfDay::Evening));
        assert_eq!(TimeOfDay::from_local_hour(21), Some(TimeOfDay::Dusk));
        assert_eq!(TimeOfDay::from_local_hour(23), Some(TimeOfDay::Night));
        assert_eq!(TimeOfDay::from_local_hour(24), None);
    }

    #[test]
    fn seasons_switch_by_hemisphere() {
        assert_eq!(Season::from_month(7, true), Some(Season::Summer));
        assert_eq!(Season::from_month(7, false), Some(Season::Winter));
        assert_eq!(Season::from_month(1, false), Some(Season::Summer));
        assert_eq!(Season::from_month(13, true), None);
    }

    #[test]
    fn forecast_and_calendar_are_bounded() {
        let mut snapshot = valid_snapshot();
        snapshot.forecast = (0..=MAX_FORECAST_ENTRIES)
            .map(|index| WeatherForecast {
                at: format!("2026-11-{index:02}"),
                condition: "clear".to_owned(),
                high_celsius: None,
                low_celsius: None,
                precipitation_probability_percent: None,
            })
            .collect();
        assert_eq!(
            snapshot.validate(),
            Err(WorldContractError::TooManyForecast(MAX_FORECAST_ENTRIES))
        );

        let mut snapshot = valid_snapshot();
        snapshot.calendar = (0..=MAX_CALENDAR_EVENTS)
            .map(|index| CalendarEvent {
                title: format!("Event {index}"),
                starts_at: "2026-11-26T09:00:00-06:00".to_owned(),
                ends_at: None,
                all_day: false,
                location: None,
                notes: None,
            })
            .collect();
        assert_eq!(
            snapshot.validate(),
            Err(WorldContractError::TooManyCalendar(MAX_CALENDAR_EVENTS))
        );
    }

    #[test]
    fn offset_timezone_and_bounds_are_validated() {
        let mut snapshot = valid_snapshot();
        snapshot.timezone.clear();
        assert_eq!(
            snapshot.validate(),
            Err(WorldContractError::InvalidTimezone)
        );

        let mut snapshot = valid_snapshot();
        snapshot.utc_offset_seconds = MAX_UTC_OFFSET_SECONDS + 1;
        assert_eq!(
            snapshot.validate(),
            Err(WorldContractError::InvalidUtcOffset)
        );

        let mut snapshot = valid_snapshot();
        snapshot.stale_after_seconds = 0;
        assert_eq!(
            snapshot.validate(),
            Err(WorldContractError::InvalidStaleAfter)
        );

        let mut snapshot = valid_snapshot();
        snapshot.forecast[0].condition = "x".repeat(MAX_CONDITION_CHARS + 1);
        assert_eq!(
            snapshot.validate(),
            Err(WorldContractError::InvalidForecast(0))
        );
    }
}
