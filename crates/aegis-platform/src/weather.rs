//! Open-Meteo weather provider.
//!
//! The provider shells out to `curl.exe` with an argument vector (never a shell
//! string), parses the Open-Meteo JSON strictly, and keeps a small bounded
//! in-memory cache. A failed request is reported as [`WeatherUnavailable`] and
//! never breaks the Platform snapshot pipeline.

use std::collections::HashMap;
use std::process::Command;
use std::sync::{Mutex, MutexGuard};

use aegis_contracts::world::{
    SunTimes, WeatherForecast, WeatherObservation, MAX_FORECAST_ENTRIES, MAX_TIMEZONE_CHARS,
};
use chrono::{DateTime, FixedOffset, NaiveDateTime, TimeZone, Utc};
use serde_json::{Map, Value};

use crate::world::{WeatherData, WeatherProvider, WeatherRequest, WeatherUnavailable};

pub const OPEN_METEO_ENDPOINT: &str = "https://api.open-meteo.com/v1/forecast";
pub const OPEN_METEO_CACHE_TTL_SECONDS: u64 = 900;
pub const OPEN_METEO_PROVIDER_NAME: &str = "open-meteo";

const CURL_TIMEOUT_SECONDS: u64 = 15;
const MAX_RESPONSE_BYTES: usize = 64 * 1024;
const MAX_CACHE_ENTRIES: usize = 16;
const MAX_REASON_CHARS: usize = 200;
const MAX_UTC_OFFSET_SECONDS: i64 = 18 * 3600;

const CURRENT_FIELDS: &str = "temperature_2m,apparent_temperature,relative_humidity_2m,precipitation,precipitation_probability,weather_code,wind_speed_10m,is_day";
const DAILY_FIELDS: &str = "temperature_2m_min,temperature_2m_max,precipitation_probability_max,weather_code,sunrise,sunset";

#[derive(Debug)]
struct CacheEntry {
    data: WeatherData,
    fetched_at: DateTime<Utc>,
}

/// Live Open-Meteo provider with a bounded TTL cache.
#[derive(Debug)]
pub struct OpenMeteoProvider {
    cache: Mutex<HashMap<String, CacheEntry>>,
    cache_ttl_seconds: u64,
    cache_capacity: usize,
}

impl OpenMeteoProvider {
    pub fn new() -> Self {
        Self::with_cache_ttl(OPEN_METEO_CACHE_TTL_SECONDS)
    }

    /// Cache hits are never older than [`OPEN_METEO_CACHE_TTL_SECONDS`].
    pub fn with_cache_ttl(cache_ttl_seconds: u64) -> Self {
        Self {
            cache: Mutex::new(HashMap::new()),
            cache_ttl_seconds: cache_ttl_seconds.min(OPEN_METEO_CACHE_TTL_SECONDS),
            cache_capacity: MAX_CACHE_ENTRIES,
        }
    }

    fn cached_data(&self, key: &str, now: DateTime<Utc>) -> Option<WeatherData> {
        let mut entries = self.lock_cache();
        let entry = entries.get(key)?;
        let ttl = i64::try_from(self.cache_ttl_seconds).unwrap_or(i64::MAX);
        let age_seconds = now.signed_duration_since(entry.fetched_at).num_seconds();
        if age_seconds <= ttl {
            Some(entry.data.clone())
        } else {
            entries.remove(key);
            None
        }
    }

    fn cache_data(&self, key: String, data: WeatherData, fetched_at: DateTime<Utc>) {
        let mut entries = self.lock_cache();
        if entries.len() >= self.cache_capacity && !entries.contains_key(&key) {
            let oldest = entries
                .iter()
                .min_by_key(|(_, entry)| entry.fetched_at)
                .map(|(oldest_key, _)| oldest_key.clone());
            if let Some(oldest) = oldest {
                entries.remove(&oldest);
            }
        }
        entries.insert(key, CacheEntry { data, fetched_at });
    }

    fn lock_cache(&self) -> MutexGuard<'_, HashMap<String, CacheEntry>> {
        self.cache
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn fetch(
        &self,
        timezone: &str,
        latitude: f64,
        longitude: f64,
    ) -> Result<String, WeatherUnavailable> {
        let output = Command::new("curl.exe")
            .arg("-sS")
            .arg("--fail")
            .arg("-m")
            .arg(CURL_TIMEOUT_SECONDS.to_string())
            .arg("--max-filesize")
            .arg(MAX_RESPONSE_BYTES.to_string())
            .arg("-H")
            .arg("Accept: application/json")
            .arg("-G")
            .arg(OPEN_METEO_ENDPOINT)
            .arg("--data-urlencode")
            .arg(format!("latitude={latitude}"))
            .arg("--data-urlencode")
            .arg(format!("longitude={longitude}"))
            .arg("--data-urlencode")
            .arg(format!("current={CURRENT_FIELDS}"))
            .arg("--data-urlencode")
            .arg(format!("daily={DAILY_FIELDS}"))
            .arg("--data-urlencode")
            .arg("forecast_days=3")
            .arg("--data-urlencode")
            .arg(format!("timezone={timezone}"))
            .output()
            .map_err(|error| reason(format!("weather transport failed: {error}")))?;

        if !output.status.success() {
            let code = output
                .status
                .code()
                .map_or_else(|| "unknown".to_owned(), |code| code.to_string());
            let detail = String::from_utf8_lossy(&output.stderr);
            let detail = bounded_reason(&detail);
            let suffix = if detail.is_empty() {
                String::new()
            } else {
                format!(": {detail}")
            };
            return Err(reason(format!(
                "weather provider request failed with exit code {code}{suffix}"
            )));
        }

        if output.stdout.len() > MAX_RESPONSE_BYTES {
            return Err(reason("weather provider response exceeded the size limit"));
        }

        String::from_utf8(output.stdout)
            .map_err(|_| reason("weather provider response was not valid UTF-8"))
    }
}

impl Default for OpenMeteoProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl WeatherProvider for OpenMeteoProvider {
    fn name(&self) -> &'static str {
        OPEN_METEO_PROVIDER_NAME
    }

    fn observe(&self, request: &WeatherRequest) -> Result<WeatherData, WeatherUnavailable> {
        let (latitude, longitude) = match (request.latitude, request.longitude) {
            (Some(latitude), Some(longitude)) => (latitude, longitude),
            _ => return Err(WeatherUnavailable::new("location not configured")),
        };
        validate_location(latitude, longitude)?;
        validate_timezone(&request.timezone)?;

        let key = cache_key(latitude, longitude, &request.timezone);
        if let Some(data) = self.cached_data(&key, request.now) {
            return Ok(data);
        }

        let body = self.fetch(&request.timezone, latitude, longitude)?;
        let data = parse_open_meteo(&body, request)?;
        self.cache_data(key, data.clone(), request.now);
        Ok(data)
    }
}

/// Parse an Open-Meteo current + daily payload into contract types.
pub fn parse_open_meteo(
    json: &str,
    request: &WeatherRequest,
) -> Result<WeatherData, WeatherUnavailable> {
    validate_timezone(&request.timezone)?;

    let root: Value =
        serde_json::from_str(json).map_err(|_| reason("weather response was not valid JSON"))?;
    let root = root
        .as_object()
        .ok_or_else(|| reason("weather response was not a JSON object"))?;

    let response_timezone = string_field(root, "timezone")?;
    if response_timezone != request.timezone {
        return Err(reason(
            "weather response timezone did not match the request",
        ));
    }

    let utc_offset_seconds = integer_field(root, "utc_offset_seconds")?;
    if !(-MAX_UTC_OFFSET_SECONDS..=MAX_UTC_OFFSET_SECONDS).contains(&utc_offset_seconds) {
        return Err(reason("weather response UTC offset was out of range"));
    }

    let observation = parse_observation(root, utc_offset_seconds)?;
    let (forecast, sun) = parse_daily(root)?;

    observation
        .validate()
        .map_err(|error| reason(format!("weather observation failed validation: {error}")))?;
    for (index, entry) in forecast.iter().enumerate() {
        entry.validate().map_err(|error| {
            reason(format!(
                "weather forecast entry {index} failed validation: {error}"
            ))
        })?;
    }
    if let Some(sun) = &sun {
        sun.validate()
            .map_err(|error| reason(format!("weather sun times failed validation: {error}")))?;
    }

    Ok(WeatherData {
        observation: Some(observation),
        forecast,
        sun,
    })
}

fn parse_observation(
    root: &Map<String, Value>,
    utc_offset_seconds: i64,
) -> Result<WeatherObservation, WeatherUnavailable> {
    let current = object_field(root, "current")?;

    let current_time = string_field(current, "time")?;
    let observed_at = local_timestamp(current_time, utc_offset_seconds)?;
    let temperature_celsius = number_field(current, "temperature_2m")?;
    let feels_like_celsius = number_field(current, "apparent_temperature")?;
    let humidity_percent = percent_field(current, "relative_humidity_2m")?;
    let wind_speed_kph = number_field(current, "wind_speed_10m")?;
    let weather_code = integer_field(current, "weather_code")?;
    let precipitation = number_field(current, "precipitation")?;
    if precipitation < 0.0 {
        return Err(reason("weather response precipitation was negative"));
    }
    percent_field(current, "precipitation_probability")?;
    let is_day = integer_field(current, "is_day")?;
    if !(0..=1).contains(&is_day) {
        return Err(reason("weather response is_day was invalid"));
    }

    Ok(WeatherObservation {
        condition: wmo_condition(weather_code).to_owned(),
        temperature_celsius: Some(temperature_celsius),
        feels_like_celsius: Some(feels_like_celsius),
        humidity_percent: Some(humidity_percent),
        wind_speed_kph: Some(wind_speed_kph),
        observed_at: Some(observed_at),
    })
}

fn parse_daily(
    root: &Map<String, Value>,
) -> Result<(Vec<WeatherForecast>, Option<SunTimes>), WeatherUnavailable> {
    let daily = object_field(root, "daily")?;
    let dates = array_field(daily, "time")?;
    let lows = array_field(daily, "temperature_2m_min")?;
    let highs = array_field(daily, "temperature_2m_max")?;
    let probabilities = array_field(daily, "precipitation_probability_max")?;
    let codes = array_field(daily, "weather_code")?;
    let sunrises = array_field(daily, "sunrise")?;
    let sunsets = array_field(daily, "sunset")?;

    let length = dates.len();
    if length == 0 || length > MAX_FORECAST_ENTRIES {
        return Err(reason("weather response forecast length was invalid"));
    }
    for array in [lows, highs, probabilities, codes, sunrises, sunsets] {
        if array.len() != length {
            return Err(reason(
                "weather response forecast arrays had mismatched lengths",
            ));
        }
    }

    let mut forecast = Vec::with_capacity(length);
    for index in 0..length {
        let at = array_string(dates, "time", index)?.to_owned();
        let low_celsius = array_number(lows, "temperature_2m_min", index)?;
        let high_celsius = array_number(highs, "temperature_2m_max", index)?;
        let precipitation_probability_percent =
            array_percent(probabilities, "precipitation_probability_max", index)?;
        let code = array_integer(codes, "weather_code", index)?;
        forecast.push(WeatherForecast {
            at,
            condition: wmo_condition(code).to_owned(),
            high_celsius: Some(high_celsius),
            low_celsius: Some(low_celsius),
            precipitation_probability_percent: Some(precipitation_probability_percent),
        });
    }

    let sunrise = array_optional_string(sunrises, "sunrise", 0)?.map(str::to_owned);
    let sunset = array_optional_string(sunsets, "sunset", 0)?.map(str::to_owned);
    let sun = if sunrise.is_some() || sunset.is_some() {
        Some(SunTimes { sunrise, sunset })
    } else {
        None
    };

    Ok((forecast, sun))
}

/// Map a WMO weather interpretation code to a bounded human condition.
pub fn wmo_condition(code: i64) -> &'static str {
    match code {
        0 => "clear sky",
        1 => "mainly clear",
        2 => "partly cloudy",
        3 => "overcast",
        45 => "fog",
        48 => "depositing rime fog",
        51 => "light drizzle",
        53 => "moderate drizzle",
        55 => "dense drizzle",
        56 => "light freezing drizzle",
        57 => "dense freezing drizzle",
        61 => "slight rain",
        63 => "moderate rain",
        65 => "heavy rain",
        66 => "light freezing rain",
        67 => "heavy freezing rain",
        71 => "slight snow",
        73 => "moderate snow",
        75 => "heavy snow",
        77 => "snow grains",
        80 => "slight rain showers",
        81 => "moderate rain showers",
        82 => "violent rain showers",
        85 => "slight snow showers",
        86 => "heavy snow showers",
        95 => "thunderstorm",
        96 => "thunderstorm with slight hail",
        99 => "thunderstorm with heavy hail",
        _ => "unknown conditions",
    }
}

fn validate_location(latitude: f64, longitude: f64) -> Result<(), WeatherUnavailable> {
    if latitude.is_finite()
        && longitude.is_finite()
        && (-90.0..=90.0).contains(&latitude)
        && (-180.0..=180.0).contains(&longitude)
    {
        Ok(())
    } else {
        Err(reason("location is invalid"))
    }
}

fn validate_timezone(timezone: &str) -> Result<(), WeatherUnavailable> {
    if timezone.trim().is_empty()
        || timezone.chars().count() > MAX_TIMEZONE_CHARS
        || timezone.chars().any(char::is_control)
    {
        Err(reason("timezone is invalid"))
    } else {
        Ok(())
    }
}

fn cache_key(latitude: f64, longitude: f64, timezone: &str) -> String {
    format!(
        "{:016x}:{:016x}:{timezone}",
        latitude.to_bits(),
        longitude.to_bits()
    )
}

fn local_timestamp(raw: &str, utc_offset_seconds: i64) -> Result<String, WeatherUnavailable> {
    if let Ok(parsed) = DateTime::parse_from_rfc3339(raw) {
        return Ok(parsed.to_rfc3339());
    }
    let offset = i32::try_from(utc_offset_seconds)
        .ok()
        .and_then(FixedOffset::east_opt)
        .ok_or_else(|| reason("weather response UTC offset was invalid"))?;
    for format in ["%Y-%m-%dT%H:%M:%S", "%Y-%m-%dT%H:%M"] {
        if let Ok(naive) = NaiveDateTime::parse_from_str(raw, format) {
            let parsed = offset
                .from_local_datetime(&naive)
                .single()
                .ok_or_else(|| reason("weather response observation time was ambiguous"))?;
            return Ok(parsed.to_rfc3339());
        }
    }
    Err(reason("weather response observation time was invalid"))
}

fn object_field<'a>(
    object: &'a Map<String, Value>,
    key: &str,
) -> Result<&'a Map<String, Value>, WeatherUnavailable> {
    object
        .get(key)
        .and_then(Value::as_object)
        .ok_or_else(|| invalid_field(key))
}

fn string_field<'a>(
    object: &'a Map<String, Value>,
    key: &str,
) -> Result<&'a str, WeatherUnavailable> {
    object
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| invalid_field(key))
}

fn number_field(object: &Map<String, Value>, key: &str) -> Result<f64, WeatherUnavailable> {
    object
        .get(key)
        .and_then(Value::as_f64)
        .filter(|value| value.is_finite())
        .ok_or_else(|| invalid_field(key))
}

fn integer_field(object: &Map<String, Value>, key: &str) -> Result<i64, WeatherUnavailable> {
    object
        .get(key)
        .and_then(Value::as_i64)
        .ok_or_else(|| invalid_field(key))
}

fn percent_field(object: &Map<String, Value>, key: &str) -> Result<u8, WeatherUnavailable> {
    percent_from_i64(integer_field(object, key)?).ok_or_else(|| invalid_field(key))
}

fn percent_from_i64(value: i64) -> Option<u8> {
    u8::try_from(value).ok().filter(|value| *value <= 100)
}

fn array_field<'a>(
    object: &'a Map<String, Value>,
    key: &str,
) -> Result<&'a Vec<Value>, WeatherUnavailable> {
    object
        .get(key)
        .and_then(Value::as_array)
        .ok_or_else(|| invalid_field(key))
}

fn array_string<'a>(
    array: &'a [Value],
    key: &str,
    index: usize,
) -> Result<&'a str, WeatherUnavailable> {
    array
        .get(index)
        .and_then(Value::as_str)
        .ok_or_else(|| invalid_field(key))
}

fn array_optional_string<'a>(
    array: &'a [Value],
    key: &str,
    index: usize,
) -> Result<Option<&'a str>, WeatherUnavailable> {
    let value = array.get(index).ok_or_else(|| invalid_field(key))?;
    if value.is_null() {
        return Ok(None);
    }
    value.as_str().map(Some).ok_or_else(|| invalid_field(key))
}

fn array_number(array: &[Value], key: &str, index: usize) -> Result<f64, WeatherUnavailable> {
    array
        .get(index)
        .and_then(Value::as_f64)
        .filter(|value| value.is_finite())
        .ok_or_else(|| invalid_field(key))
}

fn array_integer(array: &[Value], key: &str, index: usize) -> Result<i64, WeatherUnavailable> {
    array
        .get(index)
        .and_then(Value::as_i64)
        .ok_or_else(|| invalid_field(key))
}

fn array_percent(array: &[Value], key: &str, index: usize) -> Result<u8, WeatherUnavailable> {
    percent_from_i64(array_integer(array, key, index)?).ok_or_else(|| invalid_field(key))
}

fn invalid_field(field: &str) -> WeatherUnavailable {
    reason(format!(
        "weather response field was missing or invalid: {field}"
    ))
}

fn bounded_reason(value: &str) -> String {
    value
        .chars()
        .map(|character| {
            if character.is_control() {
                ' '
            } else {
                character
            }
        })
        .take(MAX_REASON_CHARS)
        .collect::<String>()
        .trim()
        .to_owned()
}

fn reason(message: impl Into<String>) -> WeatherUnavailable {
    let message = bounded_reason(&message.into());
    if message.is_empty() {
        WeatherUnavailable::new("weather provider unavailable")
    } else {
        WeatherUnavailable::new(message)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &str = r#"{
        "latitude": 41.879498,
        "longitude": -87.64974,
        "utc_offset_seconds": -21600,
        "timezone": "America/Chicago",
        "current": {
            "time": "2026-11-26T13:00",
            "temperature_2m": 6.0,
            "apparent_temperature": 0.4,
            "relative_humidity_2m": 88,
            "precipitation": 0.4,
            "precipitation_probability": 42,
            "weather_code": 61,
            "wind_speed_10m": 29.3,
            "is_day": 1
        },
        "daily": {
            "time": ["2026-11-26", "2026-11-27", "2026-11-28"],
            "temperature_2m_min": [3.8, -0.2, -0.1],
            "temperature_2m_max": [6.4, 6.1, 9.9],
            "precipitation_probability_max": [89, 87, 1],
            "weather_code": [63, 61, 51],
            "sunrise": ["2026-11-26T06:53", "2026-11-27T06:54", "2026-11-28T06:55"],
            "sunset": ["2026-11-26T16:22", "2026-11-27T16:21", "2026-11-28T16:21"]
        }
    }"#;

    fn request(latitude: Option<f64>, longitude: Option<f64>) -> WeatherRequest {
        WeatherRequest {
            timezone: "America/Chicago".to_owned(),
            latitude,
            longitude,
            now: DateTime::parse_from_rfc3339("2026-11-26T19:00:00Z")
                .expect("fixed instant")
                .with_timezone(&Utc),
        }
    }

    #[test]
    fn fixture_parses_into_contract_types() {
        let data =
            parse_open_meteo(FIXTURE, &request(Some(41.88), Some(-87.63))).expect("fixture parses");
        let observation = data.observation.expect("observation");
        assert_eq!(observation.condition, "slight rain");
        assert_eq!(observation.temperature_celsius, Some(6.0));
        assert_eq!(observation.feels_like_celsius, Some(0.4));
        assert_eq!(observation.humidity_percent, Some(88));
        assert_eq!(observation.wind_speed_kph, Some(29.3));
        assert_eq!(
            observation.observed_at.as_deref(),
            Some("2026-11-26T13:00:00-06:00")
        );

        assert_eq!(data.forecast.len(), 3);
        assert_eq!(data.forecast[0].at, "2026-11-26");
        assert_eq!(data.forecast[0].condition, "moderate rain");
        assert_eq!(data.forecast[0].high_celsius, Some(6.4));
        assert_eq!(data.forecast[0].low_celsius, Some(3.8));
        assert_eq!(data.forecast[0].precipitation_probability_percent, Some(89));

        let sun = data.sun.expect("sun times");
        assert_eq!(sun.sunrise.as_deref(), Some("2026-11-26T06:53"));
        assert_eq!(sun.sunset.as_deref(), Some("2026-11-26T16:22"));
    }

    #[test]
    fn malformed_and_partial_payloads_are_typed_errors() {
        let error = parse_open_meteo("{not json", &request(Some(41.88), Some(-87.63)))
            .expect_err("malformed json");
        assert!(error.reason.contains("not valid JSON"), "{error}");

        let error =
            parse_open_meteo("[]", &request(Some(41.88), Some(-87.63))).expect_err("not an object");
        assert!(error.reason.contains("JSON object"), "{error}");

        let missing_current = FIXTURE.replace("\"current\"", "\"current_removed\"");
        let error = parse_open_meteo(&missing_current, &request(Some(41.88), Some(-87.63)))
            .expect_err("missing current");
        assert!(error.reason.contains("current"), "{error}");

        let missing_temperature = FIXTURE.replace("\"temperature_2m\": 6.0,", "");
        let error = parse_open_meteo(&missing_temperature, &request(Some(41.88), Some(-87.63)))
            .expect_err("missing temperature");
        assert!(error.reason.contains("temperature_2m"), "{error}");

        let wrong_timezone = FIXTURE.replace("America/Chicago", "Europe/Paris");
        let error = parse_open_meteo(&wrong_timezone, &request(Some(41.88), Some(-87.63)))
            .expect_err("timezone mismatch");
        assert!(error.reason.contains("timezone"), "{error}");
    }

    #[test]
    fn wmo_codes_map_to_human_conditions() {
        assert_eq!(wmo_condition(0), "clear sky");
        assert_eq!(wmo_condition(2), "partly cloudy");
        assert_eq!(wmo_condition(61), "slight rain");
        assert_eq!(wmo_condition(71), "slight snow");
        assert_eq!(wmo_condition(95), "thunderstorm");
        assert_eq!(wmo_condition(99), "thunderstorm with heavy hail");
        assert_eq!(wmo_condition(1234), "unknown conditions");
    }

    #[test]
    fn missing_location_fails_before_any_network_call() {
        let provider = OpenMeteoProvider::new();
        let error = provider
            .observe(&request(None, None))
            .expect_err("missing location");
        assert_eq!(error.reason, "location not configured");

        let error = provider
            .observe(&request(Some(120.0), Some(-87.63)))
            .expect_err("invalid latitude");
        assert_eq!(error.reason, "location is invalid");
    }

    #[test]
    fn cached_data_is_bounded_expires_and_keeps_observed_at() {
        let provider = OpenMeteoProvider::with_cache_ttl(10_000);
        assert_eq!(provider.cache_ttl_seconds, OPEN_METEO_CACHE_TTL_SECONDS);
        let first = request(Some(41.88), Some(-87.63));
        let data = parse_open_meteo(FIXTURE, &first).expect("fixture parses");
        let key = cache_key(41.88, -87.63, &first.timezone);

        provider.cache_data(key.clone(), data.clone(), first.now);
        let cached = provider
            .cached_data(&key, first.now)
            .expect("fresh cache hit");
        assert_eq!(
            cached.observation.expect("observation").observed_at,
            data.observation.expect("observation").observed_at
        );

        let stale_instant = first.now + chrono::Duration::seconds(901);
        assert!(provider.cached_data(&key, stale_instant).is_none());
    }

    #[test]
    fn cache_evicts_the_oldest_entry_when_full() {
        let provider = OpenMeteoProvider::with_cache_ttl(900);
        let base = request(Some(41.88), Some(-87.63)).now;
        let data = WeatherData::default();
        for index in 0..MAX_CACHE_ENTRIES {
            provider.cache_data(
                format!("key-{index}"),
                data.clone(),
                base + chrono::Duration::seconds(i64::try_from(index).expect("index")),
            );
        }
        provider.cache_data(
            "key-new".to_owned(),
            data,
            base + chrono::Duration::seconds(100),
        );
        let entries = provider.lock_cache();
        assert_eq!(entries.len(), MAX_CACHE_ENTRIES);
        assert!(!entries.contains_key("key-0"));
        assert!(entries.contains_key("key-new"));
    }
}
