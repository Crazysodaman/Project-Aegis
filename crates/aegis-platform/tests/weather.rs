use aegis_platform::{
    parse_open_meteo, wmo_condition, OpenMeteoProvider, WeatherProvider, WeatherRequest,
};
use chrono::{DateTime, Utc};

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

fn chicago_request() -> WeatherRequest {
    WeatherRequest {
        timezone: "America/Chicago".to_owned(),
        latitude: Some(41.88),
        longitude: Some(-87.63),
        now: DateTime::parse_from_rfc3339("2026-11-26T19:00:00Z")
            .expect("fixed instant")
            .with_timezone(&Utc),
    }
}

#[test]
fn open_meteo_fixture_parses_into_contract_data() {
    let data = parse_open_meteo(FIXTURE, &chicago_request()).expect("fixture parses");
    let observation = data.observation.as_ref().expect("observation");
    assert_eq!(observation.condition, "slight rain");
    assert_eq!(observation.temperature_celsius, Some(6.0));
    assert_eq!(observation.humidity_percent, Some(88));
    assert_eq!(data.forecast.len(), 3);
    assert_eq!(data.forecast[1].condition, "slight rain");
    let sun = data.sun.as_ref().expect("sun times");
    assert_eq!(sun.sunrise.as_deref(), Some("2026-11-26T06:53"));
}

#[test]
fn open_meteo_rejects_malformed_partial_and_missing_location() {
    let error = parse_open_meteo("{}", &chicago_request()).expect_err("empty payload");
    assert!(error.reason.contains("timezone"), "{error}");

    let error = parse_open_meteo("not json", &chicago_request()).expect_err("bad json");
    assert!(error.reason.contains("JSON"), "{error}");

    let provider = OpenMeteoProvider::new();
    let mut request = chicago_request();
    request.latitude = None;
    let error = provider.observe(&request).expect_err("missing location");
    assert_eq!(error.reason, "location not configured");
}

#[test]
fn wmo_mapping_covers_common_conditions() {
    assert_eq!(wmo_condition(1), "mainly clear");
    assert_eq!(wmo_condition(3), "overcast");
    assert_eq!(wmo_condition(45), "fog");
    assert_eq!(wmo_condition(82), "violent rain showers");
    assert_eq!(wmo_condition(86), "heavy snow showers");
    assert_eq!(wmo_condition(999), "unknown conditions");
}

#[test]
fn live_open_meteo_observation_when_enabled() {
    if std::env::var("AEGIS_WEATHER_LIVE").as_deref() != Ok("1") {
        eprintln!("skipped live Open-Meteo check: set AEGIS_WEATHER_LIVE=1 to run it");
        return;
    }

    let provider = OpenMeteoProvider::new();
    let request = WeatherRequest {
        timezone: "America/Chicago".to_owned(),
        latitude: Some(41.88),
        longitude: Some(-87.63),
        now: Utc::now(),
    };

    match provider.observe(&request) {
        Ok(data) => {
            let observation = data.observation.as_ref().expect("live observation");
            println!(
                "live weather source={} condition={} temperature_c={:?} feels_like_c={:?} wind_kph={:?} humidity_pct={:?} observed_at={:?} forecast_days={} sunrise={:?} sunset={:?}",
                provider.name(),
                observation.condition,
                observation.temperature_celsius,
                observation.feels_like_celsius,
                observation.wind_speed_kph,
                observation.humidity_percent,
                observation.observed_at,
                data.forecast.len(),
                data.sun.as_ref().and_then(|sun| sun.sunrise.as_deref()),
                data.sun.as_ref().and_then(|sun| sun.sunset.as_deref()),
            );
        }
        Err(error) => {
            eprintln!("skipped live Open-Meteo check: network/provider failure: {error}");
        }
    }
}
